use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

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

/// Ensure a registry is cloned and up to date. Returns the local path.
pub fn ensure_registry(reg: &Registry) -> Result<PathBuf> {
    let cache_dir = Config::cache_dir()?;
    let repo_dir = cache_dir.join(&reg.name);

    if repo_dir.exists() {
        pull(&repo_dir, &reg.branch)?;
    } else {
        clone(&reg.url, &repo_dir, &reg.branch)?;
    }

    Ok(repo_dir)
}

/// Timeout for git network operations (seconds). Reserved for future use.
#[allow(dead_code)]
const GIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Run a git command with a timeout. Returns an error if the command
/// times out or exits non-zero.
fn git_command(args: &[&str], dir: Option<&Path>) -> Result<()> {
    let mut cmd = std::process::Command::new("git");
    cmd.args(args);
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    let child = cmd.spawn().context("Failed to start git")?;
    let output = child.wait_with_output().context("Failed to wait for git")?;
    // Note: std::process doesn't have native timeout.
    // We rely on git's own timeout mechanisms.
    // For true timeout enforcement, we'd need tokio or a signal handler.
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git {} failed: {}", args.first().unwrap_or(&""), stderr.trim());
    }
    Ok(())
}

/// Clone a registry repo via git CLI (respects system credential helpers).
fn clone(url: &str, dest: &Path, branch: &str) -> Result<()> {
    // --depth 1 prevents DoS via large repositories
    // -- prevents URL/branch from being interpreted as flags
    let dest_str = dest.to_string_lossy();
    git_command(
        &["clone", "--quiet", "--depth", "1", "--branch", branch, "--single-branch", "--", url, &dest_str],
        None,
    )
}

/// Pull latest changes for a registry via git CLI.
fn pull(repo_dir: &Path, branch: &str) -> Result<()> {
    // fetch + reset for reliability (handles failed pushes that left local commits)
    git_command(
        &["fetch", "--quiet", "--depth", "1", "origin", "--", branch],
        Some(repo_dir),
    )?;
    let target = format!("origin/{branch}");
    git_command(
        &["reset", "--quiet", "--hard", &target],
        Some(repo_dir),
    )
}

/// Get the path to a skill in a registry.
/// Skills can be either a file (`skill-name.md`) or a directory
/// (`skill-name/` with SKILL.md inside). Returns the path that exists,
/// preferring directory format.
///
/// Validates that the resolved path stays within the repo directory
/// to prevent path traversal attacks.
pub fn skill_path(repo_dir: &Path, reg: &Registry, skill_name: &str) -> PathBuf {
    let base = match &reg.path {
        Some(p) => {
            let resolved = repo_dir.join(p);
            // Verify path stays within repo directory
            if !resolved.starts_with(repo_dir) {
                return repo_dir.join(format!("{skill_name}.md")); // safe fallback
            }
            resolved
        }
        None => repo_dir.to_path_buf(),
    };

    // Directory skill: skill-name/SKILL.md
    // Use symlink_metadata to avoid following symlinks
    let dir_path = base.join(skill_name);
    let is_real_dir = dir_path
        .symlink_metadata()
        .map(|m| m.file_type().is_dir())
        .unwrap_or(false);
    if is_real_dir {
        return dir_path;
    }

    // Flat file: skill-name.md
    base.join(format!("{skill_name}.md"))
}

/// Check if a skill is a directory skill (vs a flat file).
pub fn is_directory_skill(path: &Path) -> bool {
    path.is_dir()
}

/// Copy a skill from registry to local, handling both file and directory skills.
/// Rejects symlinks at the top level.
pub fn copy_skill(src: &Path, dest: &Path) -> Result<()> {
    // Reject symlinks at the source
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

/// Recursively copy a directory. Skips symlinks and dotfiles (.git, etc).
fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip symlinks (prevent reading arbitrary files)
        if file_type.is_symlink() {
            continue;
        }

        // Skip dotfiles/dotdirs (.git, .gitignore, etc)
        if name_str.starts_with('.') {
            continue;
        }

        let src_path = entry.path();
        let dest_path = dest.join(&name);

        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dest_path)?;
        } else {
            std::fs::copy(&src_path, &dest_path)?;
        }
    }
    Ok(())
}

/// Hash all files in a skill (file or directory) for drift detection.
/// Returns None only if the path doesn't exist. Errors on read failures.
pub fn skill_hash(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};

    // Reject symlinks at the top level
    if path.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false) {
        eprintln!("  warning: skipping symlink {}", path.display());
        return None;
    }

    if path.is_dir() {
        let mut hasher = Sha256::new();
        let mut files = collect_files(path);
        files.sort(); // deterministic order
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
        // Skip symlink files
        if path.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false) {
            eprintln!("  warning: skipping symlink {}", path.display());
            return None;
        }
        let content = std::fs::read(path).ok()?;
        let mut hasher = Sha256::new();
        hasher.update(&content);
        Some(hex::encode(hasher.finalize()))
    } else {
        None
    }
}

/// Collect all files in a directory recursively (public for push).
pub fn collect_files_public(dir: &Path) -> Vec<PathBuf> {
    collect_files(dir)
}

/// Collect all files in a directory recursively. Skips symlinks and dotfiles.
fn collect_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
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
    }
    files
}

/// List all available skills in a registry.
/// Detects both flat files (name.md) and directory skills (name/).
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
        // Skip symlinks entirely
        if ft.is_symlink() {
            continue;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        // Skip dotfiles
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
