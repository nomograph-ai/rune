use anyhow::{Context, Result};
use fs2::FileExt;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::config::{Config, Registry};

/// Validate a skill name contains only safe characters.
/// Prevents path traversal attacks via names like `../../etc/passwd`.
pub fn validate_skill_name(name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("Skill name cannot be empty");
    }
    if name != name.trim() || name.contains(char::is_whitespace) {
        anyhow::bail!("Invalid skill name: {name:?} (must not contain whitespace)");
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") || name.contains('\0') {
        anyhow::bail!("Invalid skill name: {name} (must not contain /, \\, .., or null bytes)");
    }
    if name.starts_with('.') || name.starts_with('-') {
        anyhow::bail!("Invalid skill name: {name} (must not start with . or -)");
    }
    Ok(())
}

// ── Registry pull cache ─────────────────────────────────────────────

static PULLED_REGISTRIES: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn mark_pulled(name: &str) {
    let mut guard = PULLED_REGISTRIES.lock().unwrap();
    let set = guard.get_or_insert_with(HashSet::new);
    set.insert(name.to_string());
}

fn already_pulled(name: &str) -> bool {
    let guard = PULLED_REGISTRIES.lock().unwrap();
    guard.as_ref().map(|s| s.contains(name)).unwrap_or(false)
}

/// Global offline flag. Set by --offline CLI flag.
static OFFLINE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_offline(offline: bool) {
    OFFLINE.store(offline, std::sync::atomic::Ordering::Relaxed);
}

pub fn is_offline() -> bool {
    OFFLINE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Global dry-run flag. Set by --dry-run CLI flag.
static DRY_RUN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_dry_run(dry_run: bool) {
    DRY_RUN.store(dry_run, std::sync::atomic::Ordering::Relaxed);
}

pub fn is_dry_run() -> bool {
    DRY_RUN.load(std::sync::atomic::Ordering::Relaxed)
}

// ── File locking ────────────────────────────────────────────────────

/// RAII guard that holds a file lock and releases it on drop.
struct LockGuard(std::fs::File);

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// Acquire an exclusive file lock on a registry's cache directory.
/// Returns a guard that releases the lock on drop.
fn lock_registry(reg: &Registry) -> Result<LockGuard> {
    let cache_dir = Config::cache_dir()?;
    std::fs::create_dir_all(&cache_dir)?;
    let lock_path = cache_dir.join(format!(".{}.lock", reg.name));
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("Failed to create lock file for {}", reg.name))?;

    lock_file
        .try_lock_exclusive()
        .or_else(|_| {
            eprintln!("  waiting for lock on {}...", reg.name);
            lock_file.lock_exclusive()
        })
        .with_context(|| format!("Failed to lock registry {}", reg.name))?;

    Ok(LockGuard(lock_file))
}

// ── Ensure registry ─────────────────────────────────────────────────

/// Ensure a registry is cloned/downloaded and up to date. Returns the local path.
/// Pulls at most once per invocation. Respects --offline flag.
/// Uses file locking to prevent concurrent corruption.
pub fn ensure_registry(reg: &Registry) -> Result<PathBuf> {
    let cache_dir = Config::cache_dir()?;
    let repo_dir = cache_dir.join(&reg.name);

    if is_offline() {
        if repo_dir.exists() {
            return Ok(repo_dir);
        }
        anyhow::bail!(
            "Registry {} not cached and --offline is set. Run without --offline first.",
            reg.name
        );
    }

    if already_pulled(&reg.name) {
        return Ok(repo_dir);
    }

    // Acquire lock before any git/download operations
    let _lock = lock_registry(reg)?;

    if reg.source == "archive" {
        ensure_archive_registry(reg, &repo_dir)?;
    } else if repo_dir.exists() {
        match pull(&repo_dir, reg) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("  warning: failed to update {}: {e}", reg.name);
                eprintln!("  using cached version");
            }
        }
    } else {
        clone(reg, &repo_dir, &reg.branch)?;
    }

    mark_pulled(&reg.name);
    Ok(repo_dir)
}

/// Download and extract an archive registry (GitHub/GitLab tarball).
/// Uses etag caching to skip redundant downloads.
fn ensure_archive_registry(reg: &Registry, dest: &Path) -> Result<()> {
    let archive_url = resolve_archive_url(&reg.url, &reg.branch)?;

    let cache_dir = dest.parent().context("Invalid cache path")?;
    std::fs::create_dir_all(cache_dir)?;

    let etag_path = cache_dir.join(format!(".{}.etag", reg.name));

    // Try conditional download with etag
    let tmp_tar = cache_dir.join(format!(".{}-archive.tar.gz", reg.name));
    let mut curl_args = vec![
        "-fsSL",
        "--proto",
        "=https",
        "--max-redirs",
        "5",
        "--max-time",
        "60",
    ];

    // Add auth header if token available
    let auth_header;
    if let Ok(Some(token)) = resolve_token(reg) {
        auth_header = format!("PRIVATE-TOKEN: {token}");
        curl_args.push("-H");
        curl_args.push(&auth_header);
    }

    // If we have a cached etag and the dest exists, use conditional request
    let old_etag = if dest.exists() {
        std::fs::read_to_string(&etag_path).ok()
    } else {
        None
    };

    let header_path = cache_dir.join(format!(".{}-headers.txt", reg.name));

    // Always dump response headers to capture etag
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
        // curl exit code 22 = HTTP error (includes 304 with -f)
        // Check headers for 304
        if let Ok(headers) = std::fs::read_to_string(&header_path)
            && (headers.contains("304") || headers.contains("Not Modified"))
        {
            let _ = std::fs::remove_file(&tmp_tar);
            let _ = std::fs::remove_file(&header_path);
            return Ok(());
        }
        let _ = std::fs::remove_file(&tmp_tar);
        let _ = std::fs::remove_file(&header_path);

        // If we have a cached version, warn and continue
        if dest.exists() {
            eprintln!("  warning: failed to refresh archive for {}: {}", reg.name, stderr.trim());
            eprintln!("  using cached version");
            return Ok(());
        }
        anyhow::bail!("Failed to download archive for {}", reg.name);
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

    // Check if downloaded content matches what we have (content hash)
    if dest.exists()
        && let (Ok(new_bytes), Some(old_hash)) = (std::fs::read(&tmp_tar), archive_content_hash(dest))
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
    let tmp_extract = cache_dir.join(format!(".{}-extract", reg.name));
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
        return Ok(format!("{url}/-/archive/{branch}/{project}-{branch}.tar.gz"));
    }

    anyhow::bail!(
        "Cannot determine archive URL for {url}. Use source = \"git\" instead."
    )
}

// ── Git operations ──────────────────────────────────────────────────

/// Run a git command. Returns error with stderr on failure.
fn git_command(args: &[&str], dir: Option<&Path>) -> Result<()> {
    let mut cmd = std::process::Command::new("git");
    cmd.args(args);
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    let output = cmd.output().context("Failed to start git")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "git {} failed: {}",
            args.first().unwrap_or(&""),
            stderr.trim()
        );
    }
    Ok(())
}

/// Run a git command and capture stdout.
fn git_output(args: &[&str], dir: &Path) -> Result<String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .context("Failed to start git")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git {} failed: {}", args.first().unwrap_or(&""), stderr.trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Resolve the authenticated URL for a registry.
///
/// Token resolution order:
/// 1. `token_env` -- explicit env var in config (highest priority)
/// 2. `glab auth token` -- if URL contains gitlab.com and glab is installed
/// 3. `gh auth token` -- if URL contains github.com and gh is installed
/// 4. No auth -- use URL as-is (public repos, or system credential helpers)
fn authenticated_url(reg: &Registry) -> Result<String> {
    let token = resolve_token(reg)?;

    let Some(token) = token else {
        return Ok(reg.url.clone());
    };

    // Inject oauth2:token into HTTPS URL
    if let Some(rest) = reg.url.strip_prefix("https://") {
        Ok(format!("https://oauth2:{token}@{rest}"))
    } else {
        // Non-HTTPS URL with a token -- warn but proceed without injection
        Ok(reg.url.clone())
    }
}

/// Resolve a PAT for a registry.
fn resolve_token(reg: &Registry) -> Result<Option<String>> {
    // 1. Explicit env var takes priority
    if let Some(ref env_var) = reg.token_env {
        return match std::env::var(env_var) {
            Ok(t) if !t.is_empty() => Ok(Some(t)),
            Ok(_) => anyhow::bail!("${env_var} is set but empty (registry {})", reg.name),
            Err(_) => anyhow::bail!(
                "Registry {} requires token from ${env_var} but the variable is not set",
                reg.name
            ),
        };
    }

    // 2. Auto-detect from glab/gh CLI based on URL host
    if (reg.url.contains("gitlab.com") || reg.url.contains("gitlab."))
        && let Some(token) = cli_token("glab", &["auth", "token"])
    {
        return Ok(Some(token));
    }

    if (reg.url.contains("github.com") || reg.url.contains("github."))
        && let Some(token) = cli_token("gh", &["auth", "token"])
    {
        return Ok(Some(token));
    }

    // 3. No auth -- rely on system credential helpers or public access
    Ok(None)
}

/// Try to get a token from a CLI tool. Returns None on any failure.
fn cli_token(cmd: &str, args: &[&str]) -> Option<String> {
    std::process::Command::new(cmd)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|t| !t.is_empty())
}

fn clone(reg: &Registry, dest: &Path, branch: &str) -> Result<()> {
    let url = authenticated_url(reg)?;
    let dest_str = dest.to_string_lossy();
    git_command(
        &[
            "clone", "--quiet", "--depth", "1", "--branch", branch,
            "--single-branch", "--", &url, &dest_str,
        ],
        None,
    )
}

fn pull(repo_dir: &Path, reg: &Registry) -> Result<()> {
    // If token auth, update the remote URL in case it changed
    if reg.token_env.is_some() {
        let url = authenticated_url(reg)?;
        git_command(
            &["remote", "set-url", "origin", &url],
            Some(repo_dir),
        )?;
    }

    git_command(
        &["fetch", "--quiet", "--depth", "1", "origin", "--", &reg.branch],
        Some(repo_dir),
    )?;
    let target = format!("origin/{}", reg.branch);
    git_command(
        &["reset", "--quiet", "--hard", &target],
        Some(repo_dir),
    )
}

/// Get the last commit hash that modified a specific path in a repo.
/// This tracks skill-specific changes, not repo-wide HEAD.
pub fn skill_commit(repo_dir: &Path, skill_path_relative: &str) -> Option<String> {
    git_output(
        &["log", "-1", "--format=%h", "--", skill_path_relative],
        repo_dir,
    )
    .ok()
    .filter(|s| !s.is_empty())
}

/// Commit and push changes to a registry via git CLI.
pub fn commit_and_push(repo_dir: &Path, skill_name: &str, branch: &str, message: Option<&str>) -> Result<()> {
    git_command(&["add", "-A", "--", skill_name], Some(repo_dir))?;

    // Check if there are staged changes
    let status = std::process::Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .current_dir(repo_dir)
        .status()
        .context("Failed to check git diff")?;

    if status.success() {
        anyhow::bail!("No changes to push for {skill_name}");
    }

    let default_msg = format!("update {skill_name}\n\nPushed by rune");
    let msg = message.unwrap_or(&default_msg);

    git_command(
        &["commit", "--quiet", "-m", msg],
        Some(repo_dir),
    )?;
    git_command(
        &["push", "--quiet", "origin", "--", branch],
        Some(repo_dir),
    )
}

// ── Path operations ─────────────────────────────────────────────────

/// Get the path to a skill in a registry. Uses symlink_metadata
/// to avoid following symlinks. Verifies path stays within repo.
pub fn skill_path(repo_dir: &Path, reg: &Registry, skill_name: &str) -> PathBuf {
    let base = match &reg.path {
        Some(p) => {
            let resolved = repo_dir.join(p);
            if !resolved.starts_with(repo_dir) {
                return repo_dir.join(format!("{skill_name}.md"));
            }
            resolved
        }
        None => repo_dir.to_path_buf(),
    };

    let dir_path = base.join(skill_name);
    let is_real_dir = dir_path
        .symlink_metadata()
        .map(|m| m.file_type().is_dir())
        .unwrap_or(false);
    if is_real_dir {
        return dir_path;
    }

    base.join(format!("{skill_name}.md"))
}

/// The relative path of a skill within a registry (for git log queries).
pub fn skill_path_relative(reg: &Registry, skill_name: &str) -> String {
    match &reg.path {
        Some(p) => format!("{p}/{skill_name}"),
        None => skill_name.to_string(),
    }
}

pub fn is_directory_skill(path: &Path) -> bool {
    path.symlink_metadata()
        .map(|m| m.file_type().is_dir())
        .unwrap_or(false)
}

// ── Copy operations ─────────────────────────────────────────────────

/// Copy a skill from registry to local. Rejects symlinks.
pub fn copy_skill(src: &Path, dest: &Path) -> Result<()> {
    if src.symlink_metadata()?.file_type().is_symlink() {
        anyhow::bail!("Refusing to copy symlink: {}", src.display());
    }
    if src.is_dir() {
        copy_dir_recursive(src, dest)?;
    } else {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(src, dest)?;
    }
    Ok(())
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        let name = entry.file_name();

        if ft.is_symlink() || name.to_string_lossy().starts_with('.') {
            continue;
        }

        let src_path = entry.path();
        let dest_path = dest.join(&name);

        if ft.is_dir() {
            copy_dir_recursive(&src_path, &dest_path)?;
        } else {
            std::fs::copy(&src_path, &dest_path)?;
        }
    }
    Ok(())
}

// ── Hash operations ─────────────────────────────────────────────────

/// Hash all files in a skill for drift detection. Rejects symlinks.
pub fn skill_hash(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};

    if path.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false) {
        return None;
    }

    if path.is_dir() {
        let mut hasher = Sha256::new();
        let mut files = collect_files(path);
        files.sort();
        for file in files {
            let relative = file.strip_prefix(path).unwrap_or(&file);
            hasher.update(relative.to_string_lossy().as_bytes());
            match std::fs::read(&file) {
                Ok(content) => hasher.update(&content),
                Err(e) => {
                    eprintln!("  warning: cannot read {}: {e}", file.display());
                    return None;
                }
            }
        }
        Some(hex::encode(hasher.finalize()))
    } else if path.is_file() {
        let content = std::fs::read(path).ok()?;
        let mut hasher = Sha256::new();
        hasher.update(&content);
        Some(hex::encode(hasher.finalize()))
    } else {
        None
    }
}

/// Collect all files recursively. Skips symlinks and dotfiles.
/// Public for integration tests.
#[allow(dead_code)]
pub fn collect_files_public(dir: &Path) -> Vec<PathBuf> {
    collect_files(dir)
}

fn collect_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return files,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if ft.is_symlink() {
            continue;
        }
        let path = entry.path();
        if ft.is_dir() {
            files.extend(collect_files(&path));
        } else {
            files.push(path);
        }
    }
    files
}

// ── List operations ─────────────────────────────────────────────────

/// List all available skills in a registry.
pub fn list_skills(repo_dir: &Path, reg: &Registry) -> Result<Vec<String>> {
    let base = match &reg.path {
        Some(p) => repo_dir.join(p),
        None => repo_dir.to_path_buf(),
    };

    if !base.exists() {
        return Ok(vec![]);
    }

    let mut skills = Vec::new();
    for entry in std::fs::read_dir(&base)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        if ft.is_symlink() {
            continue;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }

        if ft.is_dir() && path.join("SKILL.md").exists() {
            skills.push(name);
        } else if ft.is_file()
            && path.extension().map(|e| e == "md").unwrap_or(false)
            && let Some(stem) = path.file_stem()
        {
            skills.push(stem.to_string_lossy().to_string());
        }
    }
    skills.sort();
    skills.dedup();
    Ok(skills)
}
