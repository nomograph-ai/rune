use anyhow::{Context, Result};
use std::path::Path;

use super::auth;
use super::fs::collect_files;
use crate::config::Registry;

/// Download and extract an archive registry (GitHub/GitLab tarball).
/// Uses etag caching to skip redundant downloads.
pub(super) fn ensure_archive_registry(reg: &Registry, dest: &Path) -> Result<()> {
    let archive_url = resolve_archive_url(&reg.url, &reg.branch)?;

    let cache_dir = dest.parent().context("Invalid cache path")?;
    std::fs::create_dir_all(cache_dir)?;

    let etag_path = cache_dir.join(format!(".{}.etag", reg.fs_name()));

    let tmp_tar = cache_dir.join(format!(".{}-archive.tar.gz", reg.fs_name()));
    let mut curl_args = vec![
        "-fsSL",
        "--proto",
        "=https",
        "--max-redirs",
        "5",
        "--max-time",
        "60",
    ];

    let auth_hdr;
    if let Ok(Some(header)) = auth::curl_auth_header(reg) {
        auth_hdr = header;
        curl_args.push("-H");
        curl_args.push(&auth_hdr);
    }

    let old_etag = if dest.exists() {
        std::fs::read_to_string(&etag_path).ok()
    } else {
        None
    };

    let header_path = cache_dir.join(format!(".{}-headers.txt", reg.fs_name()));
    let header_path_str = header_path.to_string_lossy().to_string();
    curl_args.extend_from_slice(&["-D", &header_path_str]);

    let if_none_match;
    if let Some(ref etag) = old_etag {
        let etag = etag.trim();
        if !etag.is_empty() {
            if_none_match = format!("If-None-Match: {etag}");
            curl_args.push("-H");
            curl_args.push(&if_none_match);
        }
    }

    curl_args.extend_from_slice(&["-o"]);
    let tmp_tar_str = tmp_tar.to_string_lossy().to_string();
    curl_args.push(&tmp_tar_str);
    curl_args.push(&archive_url);

    let output = std::process::Command::new("curl")
        .args(&curl_args)
        .output()
        .context("Failed to run curl")?;

    // Check for 304 Not Modified
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if let Ok(headers) = std::fs::read_to_string(&header_path)
            && (headers.contains("304") || headers.contains("Not Modified"))
        {
            let _ = std::fs::remove_file(&tmp_tar);
            let _ = std::fs::remove_file(&header_path);
            return Ok(());
        }
        let _ = std::fs::remove_file(&tmp_tar);
        let _ = std::fs::remove_file(&header_path);

        if dest.exists() {
            eprintln!(
                "  warning: failed to refresh archive for {}: {}",
                reg.name,
                stderr.trim()
            );
            eprintln!("  using cached version");
            return Ok(());
        }
        anyhow::bail!(
            "Failed to download archive for {}. Run `rune doctor` to check auth and network.",
            reg.name
        );
    }

    // Parse etag from response headers
    if let Ok(headers) = std::fs::read_to_string(&header_path) {
        for line in headers.lines() {
            let lower = line.to_lowercase();
            if lower.starts_with("etag:") {
                let etag = line[5..].trim();
                let _ = std::fs::write(&etag_path, etag);
                break;
            }
        }
    }
    let _ = std::fs::remove_file(&header_path);

    // If tmp_tar wasn't produced but curl returned success, we got a 304
    // Not Modified response with an empty body.
    if !tmp_tar.exists() {
        return Ok(());
    }

    // Check if downloaded content matches what we have (content hash)
    if dest.exists()
        && let (Ok(new_bytes), Some(old_hash)) =
            (std::fs::read(&tmp_tar), archive_content_hash(dest))
    {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&new_bytes);
        let new_hash = hex::encode(hasher.finalize());
        if new_hash == old_hash {
            let _ = std::fs::remove_file(&tmp_tar);
            return Ok(());
        }
    }

    // Extract -- GitHub/GitLab archives have a top-level directory
    let tmp_extract = cache_dir.join(format!(".{}-extract", reg.fs_name()));
    let _ = std::fs::remove_dir_all(&tmp_extract);
    std::fs::create_dir_all(&tmp_extract)?;

    let status = std::process::Command::new("tar")
        .args(["xzf"])
        .arg(&tmp_tar)
        .args(["--strip-components=1", "-C"])
        .arg(&tmp_extract)
        .status()
        .context("Failed to extract archive")?;

    let _ = std::fs::remove_file(&tmp_tar);

    if !status.success() {
        let _ = std::fs::remove_dir_all(&tmp_extract);
        anyhow::bail!("Failed to extract archive for {}", reg.name);
    }

    // Atomic swap: remove old, rename new
    if dest.exists() {
        std::fs::remove_dir_all(dest)?;
    }
    std::fs::rename(&tmp_extract, dest)?;

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

/// Resolve a git URL to an archive download URL.
fn resolve_archive_url(url: &str, branch: &str) -> Result<String> {
    let url = url.trim_end_matches(".git");

    // GitHub: https://github.com/owner/repo → /archive/refs/heads/branch.tar.gz
    if url.contains("github.com") {
        return Ok(format!("{url}/archive/refs/heads/{branch}.tar.gz"));
    }

    // GitLab: https://gitlab.com/group/project → /-/archive/branch/project-branch.tar.gz
    if url.contains("gitlab.com") {
        let project = url.rsplit('/').next().unwrap_or("repo");
        return Ok(format!(
            "{url}/-/archive/{branch}/{project}-{branch}.tar.gz"
        ));
    }

    anyhow::bail!("Cannot determine archive URL for {url}. Use source = \"git\" instead.")
}
