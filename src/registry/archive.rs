use anyhow::{Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};

use super::auth;
use super::fs::collect_files;
use crate::config::Registry;

/// Outcome of an archive fetch. Three unambiguous variants replace the
/// prior implementation's three-overlapping-signals (curl exit + headers
/// substring + file existence) for detecting 304 Not Modified.
enum ArchiveResponse {
    /// 200 OK with the tarball body and optional ETag for caching.
    Fresh { body: Vec<u8>, etag: Option<String> },
    /// 304 Not Modified — caller's cached `dest` is authoritative.
    NotModified,
    /// 4xx/5xx/network failure, but caller has a cached `dest` to fall
    /// back on. Contains the user-facing error string for a warning line.
    StaleOk(String),
}

/// Download and extract an archive registry (GitHub/GitLab tarball).
/// Uses ETag caching to skip redundant downloads. Replaces the
/// curl-and-tar shell-out with ureq + tar + flate2 so status codes are
/// first-class and 304 vs 200 vs error is unambiguous.
pub fn ensure_archive_registry(reg: &Registry, dest: &Path) -> Result<()> {
    let archive_url = resolve_archive_url(reg)?;
    let cache_dir = dest.parent().context("Invalid cache path")?;
    std::fs::create_dir_all(cache_dir)?;
    let etag_path = cache_dir.join(format!(".{}.etag", reg.fs_name()));

    let cached_etag = if dest.exists() {
        std::fs::read_to_string(&etag_path)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    } else {
        None
    };

    let response = fetch_archive(&archive_url, reg, cached_etag.as_deref(), dest.exists())?;

    match response {
        ArchiveResponse::NotModified => Ok(()),
        ArchiveResponse::StaleOk(err) => {
            eprintln!(
                "  warning: failed to refresh archive for {}: {err}",
                reg.name
            );
            eprintln!("  using cached version");
            Ok(())
        }
        ArchiveResponse::Fresh { body, etag } => {
            // Content-hash short-circuit: server may ignore If-None-Match
            // and return 200 with an identical tarball. Compare against the
            // materialized cache rather than re-extracting for no reason.
            if dest.exists()
                && let Some(old_hash) = archive_content_hash(dest)
            {
                use sha2::{Digest, Sha256};
                let mut hasher = Sha256::new();
                hasher.update(&body);
                let new_hash = hex::encode(hasher.finalize());
                if new_hash == old_hash {
                    if let Some(tag) = etag.as_deref() {
                        let _ = std::fs::write(&etag_path, tag);
                    }
                    return Ok(());
                }
            }

            extract_into(dest, &body, cache_dir, &reg.fs_name())?;

            // Write ETag only after successful extract — avoids a torn state
            // where we've recorded a version we didn't materialize.
            if let Some(tag) = etag.as_deref() {
                let _ = std::fs::write(&etag_path, tag);
            }
            Ok(())
        }
    }
}

/// Perform the HTTP fetch. Returns NotModified on 304, Fresh on 200,
/// StaleOk on recoverable failures when `has_cache` is true, and Err
/// otherwise.
fn fetch_archive(
    url: &str,
    reg: &Registry,
    cached_etag: Option<&str>,
    has_cache: bool,
) -> Result<ArchiveResponse> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(60))
        .build();

    let mut req = agent.get(url);
    if let Ok(Some(header)) = auth::curl_auth_header(reg)
        && let Some((name, value)) = header.split_once(':')
    {
        req = req.set(name.trim(), value.trim());
    }
    if let Some(tag) = cached_etag
        && !tag.is_empty()
    {
        req = req.set("If-None-Match", tag);
    }

    match req.call() {
        Ok(resp) => {
            let status = resp.status();
            if status == 304 {
                return Ok(ArchiveResponse::NotModified);
            }
            if !(200..300).contains(&status) {
                let msg = format!("HTTP {status}");
                if has_cache {
                    return Ok(ArchiveResponse::StaleOk(msg));
                }
                anyhow::bail!(
                    "Failed to download archive for {} ({msg}). Run `rune doctor` \
                     to check auth and network.",
                    reg.name
                );
            }
            let etag = resp.header("etag").map(|s| s.to_string());
            let mut body = Vec::new();
            resp.into_reader()
                .read_to_end(&mut body)
                .context("reading archive body")?;
            Ok(ArchiveResponse::Fresh { body, etag })
        }
        Err(ureq::Error::Status(304, _)) => Ok(ArchiveResponse::NotModified),
        Err(ureq::Error::Status(code, resp)) => {
            let msg = format!("HTTP {code} {}", resp.status_text());
            if has_cache {
                Ok(ArchiveResponse::StaleOk(msg))
            } else {
                anyhow::bail!(
                    "Failed to download archive for {} ({msg}). Run `rune doctor` \
                     to check auth and network.",
                    reg.name
                )
            }
        }
        Err(ureq::Error::Transport(t)) => {
            let msg = format!("transport error: {t}");
            if has_cache {
                Ok(ArchiveResponse::StaleOk(msg))
            } else {
                anyhow::bail!(
                    "Failed to download archive for {} ({msg}). Run `rune doctor` \
                     to check auth and network.",
                    reg.name
                )
            }
        }
    }
}

/// Extract a gzipped tarball into a fresh directory at `dest`, stripping
/// the GitHub/GitLab top-level archive directory (like tar
/// --strip-components=1). Atomic swap: extracts into a sibling temp dir,
/// then renames onto dest after fully materializing so readers never see
/// a partial tree.
fn extract_into(dest: &Path, body: &[u8], cache_dir: &Path, fs_name: &str) -> Result<()> {
    let tmp_extract = cache_dir.join(format!(".{fs_name}-extract"));
    let _ = std::fs::remove_dir_all(&tmp_extract);
    std::fs::create_dir_all(&tmp_extract)?;

    let gz = flate2::read::GzDecoder::new(body);
    let mut archive = tar::Archive::new(gz);
    archive.set_overwrite(true);
    archive.set_preserve_permissions(false);

    for entry in archive.entries().context("reading tar entries")? {
        let mut entry = entry.context("reading tar entry")?;
        let (stripped, display_path): (PathBuf, PathBuf) = {
            let path = entry.path().context("reading tar entry path")?;
            let display = path.to_path_buf();
            let stripped: PathBuf = path.components().skip(1).collect();
            (stripped, display)
        };

        // Strip the leading top-level directory (strip-components=1).
        // If an entry has no second path segment, skip it (the top-level
        // directory itself produces no extracted content).
        if stripped.as_os_str().is_empty() {
            continue;
        }

        // Reject path traversal: stripped must not contain `..` or be absolute.
        if stripped.is_absolute() || stripped.components().any(|c| c.as_os_str() == "..") {
            anyhow::bail!(
                "tar entry escapes extraction dir: {}",
                display_path.display()
            );
        }

        let out = tmp_extract.join(&stripped);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        entry.unpack(&out).with_context(|| {
            format!("extracting {} -> {}", display_path.display(), out.display())
        })?;
    }

    // Two-phase atomic swap: rename old dest out of the way FIRST, then
    // rename extracted into place, then remove the old. If anything fails
    // mid-way, dest is always either the old or the new tree, never absent.
    let backup = cache_dir.join(format!(".{fs_name}-old"));
    let _ = std::fs::remove_dir_all(&backup);
    if dest.exists() {
        std::fs::rename(dest, &backup)
            .with_context(|| format!("rotating {} -> {}", dest.display(), backup.display()))?;
    }
    if let Err(e) = std::fs::rename(&tmp_extract, dest) {
        // Recover: put old back if we had one.
        if backup.exists() {
            let _ = std::fs::rename(&backup, dest);
        }
        return Err(e).with_context(|| format!("promoting extracted tree to {}", dest.display()));
    }
    let _ = std::fs::remove_dir_all(&backup);
    Ok(())
}

/// Hash the content of an archive cache directory for change detection.
fn archive_content_hash(dir: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    let mut files = collect_files(dir);
    files.sort();
    for file in &files {
        let relative = file.strip_prefix(dir).unwrap_or(file);
        hasher.update(relative.to_string_lossy().as_bytes());
        if let Ok(content) = std::fs::read(file) {
            hasher.update(&content);
        }
    }
    Some(hex::encode(hasher.finalize()))
}

/// Resolve a registry to its archive download URL.
///
/// GitHub / GitLab paths are derived from the Registry.url. For tests or
/// non-standard hosts, an explicit URL can be provided via the
/// `RUNE_ARCHIVE_URL_<FS_NAME>` env var — a deliberately narrow test hook
/// that doesn't need a new config field.
fn resolve_archive_url(reg: &Registry) -> Result<String> {
    let env_var = format!(
        "RUNE_ARCHIVE_URL_{}",
        reg.fs_name().to_uppercase().replace('-', "_")
    );
    if let Ok(override_url) = std::env::var(&env_var)
        && !override_url.is_empty()
    {
        return Ok(override_url);
    }

    let url = reg.url.trim_end_matches(".git");
    let branch = &reg.branch;

    if url.contains("github.com") {
        return Ok(format!("{url}/archive/refs/heads/{branch}.tar.gz"));
    }

    if url.contains("gitlab.com") {
        let project = url.rsplit('/').next().unwrap_or("repo");
        return Ok(format!(
            "{url}/-/archive/{branch}/{project}-{branch}.tar.gz"
        ));
    }

    anyhow::bail!(
        "Cannot determine archive URL for {url}. Set source = \"git\" in config.toml, \
         or set {env_var} for a test override."
    )
}
