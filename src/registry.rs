use anyhow::{Context, Result};
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

// ── Ensure registry ─────────────────────────────────────────────────

/// Ensure a registry is cloned and up to date. Returns the local path.
/// Pulls at most once per invocation. Respects --offline flag.
pub fn ensure_registry(reg: &Registry) -> Result<PathBuf> {
    let cache_dir = Config::cache_dir()?;
    let repo_dir = cache_dir.join(&reg.name);

    if repo_dir.exists() {
        if !is_offline() && !already_pulled(&reg.name) {
            match pull(&repo_dir, &reg.branch) {
                Ok(()) => mark_pulled(&reg.name),
                Err(e) => {
                    eprintln!("  warning: failed to update {}: {e}", reg.name);
                    eprintln!("  using cached version");
                }
            }
        }
    } else if is_offline() {
        anyhow::bail!(
            "Registry {} not cached and --offline is set. Run without --offline first.",
            reg.name
        );
    } else {
        clone(&reg.url, &repo_dir, &reg.branch)?;
        mark_pulled(&reg.name);
    }

    Ok(repo_dir)
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

fn clone(url: &str, dest: &Path, branch: &str) -> Result<()> {
    let dest_str = dest.to_string_lossy();
    git_command(
        &[
            "clone", "--quiet", "--depth", "1", "--branch", branch,
            "--single-branch", "--", url, &dest_str,
        ],
        None,
    )
}

fn pull(repo_dir: &Path, branch: &str) -> Result<()> {
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
pub fn commit_and_push(repo_dir: &Path, skill_name: &str, branch: &str) -> Result<()> {
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

    git_command(
        &["commit", "--quiet", "-m", &format!("update {skill_name}\n\nPushed by rune")],
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
