use anyhow::{Context, Result};
use std::path::Path;

use super::auth::{self, clone_url};
use super::paths::skill_path;
use crate::config::Registry;

/// Run a git command. Returns error with stderr on failure.
pub(super) fn git_command(args: &[&str], dir: Option<&Path>) -> Result<()> {
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
pub(super) fn git_output(args: &[&str], dir: &Path) -> Result<String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .context("Failed to start git")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "git {} failed: {}",
            args.first().unwrap_or(&""),
            stderr.trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Run a git command with transient authentication via GIT_ASKPASS.
/// Token is resolved at runtime and never persisted in .git/config.
fn git_command_auth(args: &[&str], dir: Option<&Path>, reg: &Registry) -> Result<()> {
    let mut cmd = std::process::Command::new("git");

    // Inject credentials via GIT_ASKPASS (transient, per-command)
    if let Ok(Some(token)) = auth::resolve_token(reg) {
        cmd.env("GIT_ASKPASS", "/bin/sh");
        cmd.env("GIT_TERMINAL_PROMPT", "0");
        cmd.env("RUNE_GIT_TOKEN", &token);
        cmd.env(
            "GIT_ASKPASS",
            auth::create_askpass_path()?.to_string_lossy().to_string(),
        );
    }

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

/// Configure local git identity in a registry clone.
/// Uses git_email/git_name from config, or auto-detects from glab/gh.
fn configure_identity(repo_dir: &Path, reg: &Registry) -> Result<()> {
    let email = reg.git_email.as_deref().map(|s| s.to_string()).or_else(|| {
        if reg.url.contains("gitlab.com") || reg.url.contains("gitlab.") {
            auth::cli_token("glab", &["api", "user", "--jq", ".email"])
        } else if reg.url.contains("github.com") || reg.url.contains("github.") {
            auth::cli_token("gh", &["api", "user", "--jq", ".email"])
        } else {
            None
        }
    });

    let name = reg.git_name.as_deref().map(|s| s.to_string()).or_else(|| {
        if reg.url.contains("gitlab.com") || reg.url.contains("gitlab.") {
            auth::cli_token("glab", &["api", "user", "--jq", ".name"])
        } else if reg.url.contains("github.com") || reg.url.contains("github.") {
            auth::cli_token("gh", &["api", "user", "--jq", ".name"])
        } else {
            None
        }
    });

    if let Some(email) = &email {
        git_command(&["config", "user.email", email], Some(repo_dir))?;
    }
    if let Some(name) = &name {
        git_command(&["config", "user.name", name], Some(repo_dir))?;
    }

    Ok(())
}

pub(super) fn clone(reg: &Registry, dest: &Path, branch: &str) -> Result<()> {
    let url = clone_url(reg)?;
    let dest_str = dest.to_string_lossy();
    git_command_auth(
        &[
            "clone",
            "--quiet",
            "--depth",
            "1",
            "--branch",
            branch,
            "--single-branch",
            "--",
            &url,
            &dest_str,
        ],
        None,
        reg,
    )?;

    // Scrub auth from persisted remote URL
    git_command(&["remote", "set-url", "origin", &reg.url], Some(dest))?;

    configure_identity(dest, reg)?;

    Ok(())
}

pub(super) fn pull(repo_dir: &Path, reg: &Registry) -> Result<()> {
    let url = clone_url(reg)?;
    git_command(&["remote", "set-url", "origin", &url], Some(repo_dir))?;

    git_command_auth(
        &[
            "fetch",
            "--quiet",
            "--depth",
            "1",
            "origin",
            "--",
            &reg.branch,
        ],
        Some(repo_dir),
        reg,
    )?;

    git_command(&["remote", "set-url", "origin", &reg.url], Some(repo_dir))?;

    let target = format!("origin/{}", reg.branch);
    git_command(&["reset", "--quiet", "--hard", &target], Some(repo_dir))
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
/// Uses transient auth for push -- no PAT in remote URL.
pub fn commit_and_push(
    repo_dir: &Path,
    skill_name: &str,
    reg: &Registry,
    message: Option<&str>,
) -> Result<()> {
    let skill_rel = skill_path(repo_dir, reg, skill_name);
    let rel = skill_rel.strip_prefix(repo_dir).unwrap_or(&skill_rel);
    let rel_str = rel.to_string_lossy();
    git_command(&["add", "-A", "--", &rel_str], Some(repo_dir))?;

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

    git_command(&["commit", "--quiet", "-m", msg], Some(repo_dir))?;

    let url = clone_url(reg)?;
    git_command(&["remote", "set-url", "origin", &url], Some(repo_dir))?;

    let result = git_command_auth(
        &["push", "--quiet", "origin", "--", &reg.branch],
        Some(repo_dir),
        reg,
    );

    // Always scrub auth from remote URL, even on push failure
    let _ = git_command(&["remote", "set-url", "origin", &reg.url], Some(repo_dir));

    result
}
