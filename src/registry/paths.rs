use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::config::Registry;
use crate::manifest::ArtifactType;

/// Get the path to a skill in a registry. Uses symlink_metadata
/// to avoid following symlinks. Verifies path stays within repo.
pub fn skill_path(repo_dir: &Path, reg: &Registry, skill_name: &str) -> PathBuf {
    skill_path_with_hint(repo_dir, reg, skill_name, None)
}

/// Get the path to a skill in a registry with a local format hint.
///
/// Probes typed-subdirectory layout (`skills/<name>`) first, falls back
/// to legacy flat layout (`<name>` at registry root). Mirrors
/// `artifact_path_with_hint` so writable registries using either layout
/// are found by the upstream-pedigree scanner.
pub fn skill_path_with_hint(
    repo_dir: &Path,
    reg: &Registry,
    skill_name: &str,
    local_is_dir: Option<bool>,
) -> PathBuf {
    artifact_path_with_hint(repo_dir, reg, skill_name, ArtifactType::Skill, local_is_dir)
}

/// The relative path of a skill within a registry (for git log queries).
///
/// Probes the registry's on-disk layout: typed-subdir (`<reg>/skills/<name>`)
/// if `skills/` exists, else legacy flat (`<reg>/<name>`). Without filesystem
/// probing, git log queries against typed-subdir registries would return
/// no history.
pub fn skill_path_relative(repo_dir: &Path, reg: &Registry, skill_name: &str) -> String {
    let typed = match &reg.path {
        Some(p) => repo_dir.join(p).join(ArtifactType::Skill.section()),
        None => repo_dir.join(ArtifactType::Skill.section()),
    };
    let use_typed = typed.is_dir();

    match (&reg.path, use_typed) {
        (Some(p), true) => format!("{p}/{}/{skill_name}", ArtifactType::Skill.section()),
        (Some(p), false) => format!("{p}/{skill_name}"),
        (None, true) => format!("{}/{skill_name}", ArtifactType::Skill.section()),
        (None, false) => skill_name.to_string(),
    }
}

// ── Artifact-aware path operations ─────────────────────────────────

/// Get the base directory for a type within a registry.
/// Tries the typed subdirectory first (e.g. `skills/`, `agents/`, `rules/`).
/// Falls back to the registry root for skills in legacy registries.
pub fn artifact_base_in_registry(
    repo_dir: &Path,
    reg: &Registry,
    artifact_type: ArtifactType,
) -> PathBuf {
    let base = match &reg.path {
        Some(p) => {
            let resolved = repo_dir.join(p);
            if !resolved.starts_with(repo_dir) {
                repo_dir.to_path_buf()
            } else {
                resolved
            }
        }
        None => repo_dir.to_path_buf(),
    };

    // Try typed subdirectory first (e.g. skills/, agents/, rules/)
    let typed_dir = base.join(artifact_type.section());
    if typed_dir.is_dir() {
        return typed_dir;
    }

    // For skills, fall back to root (legacy registries store skills at root)
    if artifact_type == ArtifactType::Skill {
        return base;
    }

    typed_dir
}

/// Get the path to an item in a registry. Handles both typed subdirectory
/// and legacy flat layouts.
pub fn artifact_path(
    repo_dir: &Path,
    reg: &Registry,
    name: &str,
    artifact_type: ArtifactType,
) -> PathBuf {
    artifact_path_with_hint(repo_dir, reg, name, artifact_type, None)
}

/// Get the path to an item in a registry with a local format hint.
pub fn artifact_path_with_hint(
    repo_dir: &Path,
    reg: &Registry,
    name: &str,
    artifact_type: ArtifactType,
    local_is_dir: Option<bool>,
) -> PathBuf {
    let base = artifact_base_in_registry(repo_dir, reg, artifact_type);

    if artifact_type.is_directory_type() {
        let dir_path = base.join(name);
        let is_real_dir = dir_path
            .symlink_metadata()
            .map(|m| m.file_type().is_dir())
            .unwrap_or(false);

        if is_real_dir || local_is_dir == Some(true) {
            return dir_path;
        }
    }

    base.join(format!("{name}.md"))
}

/// The relative path of an item within a registry (for git log queries).
pub fn artifact_path_relative(reg: &Registry, name: &str, artifact_type: ArtifactType) -> String {
    let section = artifact_type.section();
    match &reg.path {
        Some(p) => format!("{p}/{section}/{name}"),
        None => format!("{section}/{name}"),
    }
}

/// List all items of a given type in a registry.
pub fn list_artifacts(
    repo_dir: &Path,
    reg: &Registry,
    artifact_type: ArtifactType,
) -> Result<Vec<String>> {
    let base = artifact_base_in_registry(repo_dir, reg, artifact_type);

    if !base.exists() {
        return Ok(vec![]);
    }

    let mut items = Vec::new();
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

        // Skip subdirectories that are type names (skills/, agents/, rules/)
        // to avoid listing them as items in legacy flat registries
        if ft.is_dir() && matches!(name.as_str(), "skills" | "agents" | "rules") {
            continue;
        }

        if artifact_type.is_directory_type() {
            // Skills: directory with SKILL.md or .md file
            if ft.is_dir() && path.join("SKILL.md").exists() {
                items.push(name);
            } else if ft.is_file()
                && path.extension().map(|e| e == "md").unwrap_or(false)
                && let Some(stem) = path.file_stem()
            {
                items.push(stem.to_string_lossy().to_string());
            }
        } else {
            // Agents and rules: .md files only
            if ft.is_file()
                && path.extension().map(|e| e == "md").unwrap_or(false)
                && let Some(stem) = path.file_stem()
            {
                items.push(stem.to_string_lossy().to_string());
            }
        }
    }
    items.sort();
    items.dedup();
    Ok(items)
}
