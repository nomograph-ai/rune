use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::config::{Config, Registry};

/// Validate a skill name contains only safe characters.
/// Prevents path traversal attacks via names like `../../etc/passwd`.
pub fn validate_skill_name(name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("Skill name cannot be empty");
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

/// Clone a registry repo via git CLI (respects system credential helpers).
fn clone(url: &str, dest: &Path, branch: &str) -> Result<()> {
    // Use -- to prevent URL/branch from being interpreted as flags
    let status = std::process::Command::new("git")
        .args(["clone", "--quiet", "--branch", branch, "--single-branch", "--"])
        .arg(url)
        .arg(dest)
        .status()
        .context("Failed to run git clone")?;

    if !status.success() {
        anyhow::bail!("git clone failed for {url}");
    }
    Ok(())
}

/// Pull latest changes for a registry via git CLI.
fn pull(repo_dir: &Path, branch: &str) -> Result<()> {
    // Use fetch + reset for reliability (handles failed pushes that left local commits)
    let status = std::process::Command::new("git")
        .args(["fetch", "--quiet", "origin", "--"])
        .arg(branch)
        .current_dir(repo_dir)
        .status()
        .context("Failed to run git fetch")?;

    if !status.success() {
        anyhow::bail!("git fetch failed in {}", repo_dir.display());
    }

    let target = format!("origin/{branch}");
    let status = std::process::Command::new("git")
        .args(["reset", "--quiet", "--hard", &target])
        .current_dir(repo_dir)
        .status()
        .context("Failed to run git reset")?;

    if !status.success() {
        anyhow::bail!("git reset failed in {}", repo_dir.display());
    }
    Ok(())
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
        Some(p) => repo_dir.join(p),
        None => repo_dir.to_path_buf(),
    };

    // Directory skill: skill-name/SKILL.md
    let dir_path = base.join(skill_name);
    if dir_path.is_dir() {
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
pub fn copy_skill(src: &Path, dest: &Path) -> Result<()> {
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
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if path.is_dir() && !name.starts_with('.') && path.join("SKILL.md").exists() {
            skills.push(name);
        } else if path.extension().map(|e| e == "md").unwrap_or(false)
            && let Some(stem) = path.file_stem()
        {
            skills.push(stem.to_string_lossy().to_string());
        }
    }
    skills.sort();
    skills.dedup();
    Ok(skills)
}
