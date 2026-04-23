//! Registry operations: validate names, cache/lock management, fetch
//! (git or archive), filesystem helpers, path resolution, worktree
//! materialization, and commit/push.
//!
//! Each concern lives in its own submodule. The public API is unchanged
//! from when this was one 1197-line file — callers still import from
//! `crate::registry::X`.

use anyhow::Result;
use std::path::PathBuf;

use crate::config::{Config, Registry, SourceKind};

pub mod archive;
mod auth;
mod cache;
pub mod fs;
mod git;
mod materialize;
pub mod paths;
mod validate;

// ── Public API ──────────────────────────────────────────────────────

pub use cache::{is_dry_run, is_offline, parse_cache_metadata_name, set_dry_run, set_offline};
pub use fs::{copy_skill, is_directory_skill, skill_hash};
pub use git::{commit_and_push, skill_commit};
pub use materialize::{materialize_artifact, resolved_commit};
pub use paths::{
    artifact_path, artifact_path_relative, artifact_path_with_hint, list_artifacts, list_skills,
    skill_path, skill_path_relative, skill_path_with_hint,
};
pub use validate::{validate_name, validate_skill_name};

// ── Ensure registry ─────────────────────────────────────────────────

/// Ensure a registry is cloned/downloaded and up to date. Returns the local path.
/// Pulls at most once per invocation. Respects --offline flag.
/// Uses file locking to prevent concurrent corruption.
pub fn ensure_registry(reg: &Registry) -> Result<PathBuf> {
    let cache_dir = Config::cache_dir()?;
    let repo_dir = cache_dir.join(reg.fs_name());

    if is_offline() {
        if repo_dir.exists() {
            return Ok(repo_dir);
        }
        anyhow::bail!(
            "Registry {} not cached and --offline is set. Run without --offline first.",
            reg.name
        );
    }

    if cache::already_pulled(&reg.name) {
        return Ok(repo_dir);
    }

    // Acquire lock before any git/download operations
    let _lock = cache::lock_registry(reg)?;

    if reg.source == SourceKind::Archive {
        archive::ensure_archive_registry(reg, &repo_dir)?;
    } else if repo_dir.exists() {
        match git::pull(&repo_dir, reg) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("  warning: failed to update {}: {e}", reg.name);
                eprintln!("  using cached version");
            }
        }
    } else {
        git::clone(reg, &repo_dir, &reg.branch)?;
    }

    cache::mark_pulled(&reg.name);
    Ok(repo_dir)
}
