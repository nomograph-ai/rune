use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::config::{Config, Registry};

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
    let status = std::process::Command::new("git")
        .args(["clone", "--quiet", "--branch", branch, "--single-branch", url])
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
    let status = std::process::Command::new("git")
        .args(["pull", "--quiet", "--ff-only", "origin", branch])
        .current_dir(repo_dir)
        .status()
        .context("Failed to run git pull")?;

    if !status.success() {
        anyhow::bail!("git pull failed in {}", repo_dir.display());
    }
    Ok(())
}

/// Get the path to a skill file within a registry.
pub fn skill_path(repo_dir: &Path, reg: &Registry, skill_name: &str) -> PathBuf {
    let base = match &reg.path {
        Some(p) => repo_dir.join(p),
        None => repo_dir.to_path_buf(),
    };
    base.join(format!("{skill_name}.md"))
}

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
        let path = entry.path();
        if path.extension().map(|e| e == "md").unwrap_or(false) {
            if let Some(stem) = path.file_stem() {
                skills.push(stem.to_string_lossy().to_string());
            }
        }
    }
    skills.sort();
    Ok(skills)
}
