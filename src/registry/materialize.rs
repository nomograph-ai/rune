use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::ensure_registry;
use super::paths::artifact_path;
use crate::config::{Config, Registry, SourceKind};
use crate::manifest::ArtifactType;

/// Materialize an artifact at a specific git ref.
///
/// Common case (`version = None`): returns the path inside the registry's
/// primary checkout at the configured branch.
///
/// Versioned case: creates (or reuses) a cached git worktree of the
/// registry at the given ref and returns the artifact path inside it.
pub fn materialize_artifact(
    reg: &Registry,
    name: &str,
    at: ArtifactType,
    version: Option<&str>,
) -> Result<PathBuf> {
    let repo_dir = ensure_registry(reg)?;
    let default_path = artifact_path(&repo_dir, reg, name, at);

    let Some(ver) = version else {
        return Ok(default_path);
    };

    if reg.source == SourceKind::Archive {
        anyhow::bail!(
            "Version pin `@{ver}` on {name} is not supported for archive-type \
             registry {reg_name}. Fix: change `source = \"git\"` for {reg_name} \
             in rune config.toml, or remove the `@{ver}` suffix from {name} in \
             rune.toml.",
            reg_name = reg.name
        );
    }

    let wt = worktree_at_ref(reg, &repo_dir, ver)?;
    Ok(artifact_path(&wt, reg, name, at))
}

/// Resolved commit hash for an artifact's effective ref.
pub fn resolved_commit(reg: &Registry, version: Option<&str>) -> Result<String> {
    let repo_dir = ensure_registry(reg)?;
    let reference = version.unwrap_or(&reg.branch);
    git_rev_parse(&repo_dir, reference)
}

fn worktree_at_ref(reg: &Registry, repo_dir: &Path, ref_spec: &str) -> Result<PathBuf> {
    let cache_dir = Config::cache_dir()?;
    let worktrees_base = cache_dir.join("worktrees");
    let key = format!("{}--{}", reg.fs_name(), sanitize_ref(ref_spec));
    let dest = worktrees_base.join(&key);

    if dest.exists() {
        let _ = Command::new("git")
            .args(["-C", dest.to_str().unwrap(), "fetch", "--quiet", "origin"])
            .status();
        return Ok(dest);
    }

    std::fs::create_dir_all(&worktrees_base).with_context(|| {
        format!(
            "Failed to create worktree cache directory {}",
            worktrees_base.display()
        )
    })?;

    let mkwt = |r: &str| -> std::process::Output {
        Command::new("git")
            .args([
                "-C",
                repo_dir.to_str().unwrap(),
                "worktree",
                "add",
                "--detach",
                dest.to_str().unwrap(),
                r,
            ])
            .output()
            .expect("git worktree add: failed to spawn")
    };

    let first = mkwt(ref_spec);
    if first.status.success() {
        return Ok(dest);
    }
    let origin_ref = format!("origin/{ref_spec}");
    let second = mkwt(&origin_ref);
    if second.status.success() {
        return Ok(dest);
    }

    let _ = std::fs::remove_dir_all(&dest);
    anyhow::bail!(
        "Failed to materialize ref `{ref_spec}` for registry {}: {}",
        reg.name,
        String::from_utf8_lossy(&second.stderr).trim(),
    );
}

fn git_rev_parse(repo_dir: &Path, ref_spec: &str) -> Result<String> {
    let run = |r: &str| -> Result<String> {
        let out = Command::new("git")
            .args(["-C", repo_dir.to_str().unwrap(), "rev-parse", r])
            .output()
            .with_context(|| format!("git rev-parse {r}"))?;
        if !out.status.success() {
            anyhow::bail!("git rev-parse failed for {r}");
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    run(ref_spec).or_else(|_| run(&format!("origin/{ref_spec}")))
}

fn sanitize_ref(ref_spec: &str) -> String {
    ref_spec
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect()
}
