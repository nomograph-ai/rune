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

/// Clone a registry repo.
fn clone(url: &str, dest: &Path, branch: &str) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    let mut builder = git2::build::RepoBuilder::new();
    builder.branch(branch);

    // Try HTTPS with credential helper, fall back to SSH
    let mut callbacks = git2::RemoteCallbacks::new();
    callbacks.credentials(|_url, username, allowed| {
        if allowed.contains(git2::CredentialType::SSH_KEY) {
            git2::Cred::ssh_key_from_agent(username.unwrap_or("git"))
        } else {
            git2::Cred::default()
        }
    });

    let mut fetch_opts = git2::FetchOptions::new();
    fetch_opts.remote_callbacks(callbacks);
    builder.fetch_options(fetch_opts);

    builder
        .clone(url, dest)
        .with_context(|| format!("Failed to clone {url}"))?;
    Ok(())
}

/// Pull latest changes for a registry.
fn pull(repo_dir: &Path, branch: &str) -> Result<()> {
    let repo = git2::Repository::open(repo_dir)
        .context("Failed to open cached registry")?;

    let mut remote = repo.find_remote("origin")
        .context("No origin remote")?;

    let mut callbacks = git2::RemoteCallbacks::new();
    callbacks.credentials(|_url, username, allowed| {
        if allowed.contains(git2::CredentialType::SSH_KEY) {
            git2::Cred::ssh_key_from_agent(username.unwrap_or("git"))
        } else {
            git2::Cred::default()
        }
    });

    let mut fetch_opts = git2::FetchOptions::new();
    fetch_opts.remote_callbacks(callbacks);

    remote
        .fetch(&[branch], Some(&mut fetch_opts), None)
        .with_context(|| format!("Failed to fetch from origin"))?;

    // Fast-forward to origin/<branch>
    let fetch_head = repo
        .find_reference(&format!("refs/remotes/origin/{branch}"))
        .context("Could not find remote branch")?;
    let target = fetch_head
        .target()
        .context("Remote branch has no target")?;
    let commit = repo.find_commit(target)?;

    let branch_ref = format!("refs/heads/{branch}");
    repo.reference(&branch_ref, target, true, "rune pull")?;
    repo.set_head(&branch_ref)?;
    repo.checkout_head(Some(
        git2::build::CheckoutBuilder::new().force(),
    ))?;

    drop(remote);
    drop(commit);

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
