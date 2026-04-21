use anyhow::{Context, Result};
use std::path::Path;

use super::resolve_registry_typed;
use crate::color;
use crate::config::Config;
use crate::lockfile::{LockedSkill, Lockfile};
use crate::manifest::{ALL_TYPES, ArtifactType, Manifest, SkillEntry};
use crate::pedigree;
use crate::registry;

/// Add an item from a registry to the project manifest and sync it.
pub fn add(
    project_dir: &Path,
    name: &str,
    registry_name: Option<&str>,
    artifact_type: ArtifactType,
) -> Result<()> {
    registry::validate_name(name)?;
    let config = Config::load()?;

    // Resolve the registry
    let reg = if let Some(rname) = registry_name {
        config
            .registry(rname)
            .with_context(|| format!("Unknown registry: {rname}"))?
    } else {
        // Ensure all registries are pulled, then resolve
        for r in &config.registry {
            let _ = registry::ensure_registry(r);
        }
        let cache_dir = Config::cache_dir()?;
        config
            .resolve_artifact(name, &cache_dir, artifact_type)
            .with_context(|| format!("{name}: not found in any registry"))?
    };

    let repo_dir = registry::ensure_registry(reg)?;
    let reg_path = registry::artifact_path(&repo_dir, reg, name, artifact_type);
    if !reg_path.exists() {
        anyhow::bail!("{name} not found in registry {}", reg.name);
    }

    // Update manifest
    let mut manifest = Manifest::try_load(project_dir).unwrap_or_default();
    let entry = if registry_name.is_some() {
        SkillEntry {
            registry: Some(reg.name.clone()),
            version: None,
        }
    } else {
        SkillEntry {
            registry: None,
            version: None,
        }
    };
    manifest
        .section_mut(artifact_type)
        .insert(name.to_string(), entry);
    manifest.save(project_dir)?;

    // Copy the item
    let artifact_dir = manifest.artifact_dir(project_dir, artifact_type);
    std::fs::create_dir_all(&artifact_dir)?;

    let is_dir = artifact_type.is_directory_type() && registry::is_directory_skill(&reg_path);
    let local_path = if is_dir {
        artifact_dir.join(name)
    } else {
        artifact_dir.join(format!("{name}.md"))
    };

    registry::copy_skill(&reg_path, &local_path)?;

    // Update lockfile
    let mut lockfile = Lockfile::load(project_dir).unwrap_or_default();
    let hash = registry::skill_hash(&local_path).unwrap_or_default();
    let item_rel = if artifact_type == ArtifactType::Skill {
        registry::skill_path_relative(reg, name)
    } else {
        registry::artifact_path_relative(reg, name, artifact_type)
    };
    let registry_commit = registry::skill_commit(&repo_dir, &item_rel);
    lockfile.section_mut(artifact_type).insert(
        name.to_string(),
        LockedSkill {
            registry: reg.name.clone(),
            hash,
            registry_commit,
            synced_at: pedigree::today(),
        },
    );
    lockfile.save(project_dir)?;

    let kind = if is_dir { "dir" } else { "file" };
    eprintln!(
        "Added {} {} from {} ({kind})",
        artifact_type.singular(),
        name,
        reg.name
    );
    Ok(())
}

/// Push a local item change back to its registry. All git operations via CLI.
pub fn push(
    project_dir: &Path,
    name: &str,
    message: Option<&str>,
    artifact_type: Option<ArtifactType>,
) -> Result<()> {
    registry::validate_name(name)?;
    let config = Config::load()?;
    let manifest = Manifest::load(project_dir)?;

    // Auto-detect type if not specified
    let at = match artifact_type {
        Some(t) => t,
        None => manifest
            .find_type(name)
            .with_context(|| format!("{name} not in manifest"))?,
    };

    let entry = manifest
        .section(at)
        .get(name)
        .with_context(|| format!("{name} not in {} section", at.section()))?;

    let reg = resolve_registry_typed(name, entry, &config, at)?;

    if reg.readonly {
        anyhow::bail!("Registry {} is read-only. Cannot push {}.", reg.name, name);
    }

    let repo_dir = registry::ensure_registry(reg)?;

    let artifact_dir = manifest.artifact_dir(project_dir, at);
    let local_dir = artifact_dir.join(name);
    let local_file = artifact_dir.join(format!("{name}.md"));

    let local_path = if local_dir.exists() {
        &local_dir
    } else if local_file.exists() {
        &local_file
    } else {
        anyhow::bail!("{name} not found locally");
    };

    // Pass local format hint so new items get the right path in the registry
    let local_is_dir = Some(local_path.is_dir());
    let reg_path = if at == ArtifactType::Skill {
        registry::skill_path_with_hint(&repo_dir, reg, name, local_is_dir)
    } else {
        registry::artifact_path_with_hint(&repo_dir, reg, name, at, local_is_dir)
    };

    if registry::is_dry_run() {
        eprintln!(
            "  {}: {} to {}",
            name,
            color::yellow("would push"),
            color::cyan(&reg.name)
        );
        return Ok(());
    }

    // Copy local -> registry
    if reg_path.exists() && reg_path.is_dir() {
        std::fs::remove_dir_all(&reg_path)?;
    }
    registry::copy_skill(local_path, &reg_path)?;

    // Commit and push via git CLI
    registry::commit_and_push(&repo_dir, name, reg, message)?;

    eprintln!("Pushed {} to {}", name, color::cyan(&reg.name));
    Ok(())
}

/// Remove an item from the project manifest and optionally delete local files.
pub fn remove(project_dir: &Path, name: &str, artifact_type: Option<ArtifactType>) -> Result<()> {
    registry::validate_name(name)?;
    let mut manifest = Manifest::load(project_dir)?;

    // Auto-detect type if not specified
    let at = match artifact_type {
        Some(t) => t,
        None => manifest
            .find_type(name)
            .with_context(|| format!("{name} not in manifest"))?,
    };

    if manifest.section_mut(at).remove(name).is_none() {
        anyhow::bail!("{name} not in {} section", at.section());
    }
    manifest.save(project_dir)?;

    // Remove local files
    let artifact_dir = manifest.artifact_dir(project_dir, at);
    let dir_path = artifact_dir.join(name);
    let file_path = artifact_dir.join(format!("{name}.md"));

    if dir_path.exists() && dir_path.is_dir() {
        std::fs::remove_dir_all(&dir_path)?;
        eprintln!("Removed {name} (directory)");
    } else if file_path.exists() {
        std::fs::remove_file(&file_path)?;
        eprintln!("Removed {name} (file)");
    } else {
        eprintln!("Removed {name} from manifest (no local files found)");
    }

    // Clean lockfile entry
    let mut lockfile = Lockfile::load(project_dir).unwrap_or_default();
    if lockfile.section_mut(at).remove(name).is_some() {
        lockfile.save(project_dir)?;
    }

    Ok(())
}

/// Add one or more items of the given type, or all items of that type from a registry.
pub fn add_many(
    project_dir: &Path,
    names: &[String],
    registry_name: Option<&str>,
    all: bool,
    artifact_type: ArtifactType,
) -> Result<()> {
    if all {
        let reg_name = registry_name.expect("--all requires --from (enforced by clap)");
        let config = Config::load()?;
        let reg = config
            .registry(reg_name)
            .with_context(|| format!("Unknown registry: {reg_name}"))?;
        let repo_dir = registry::ensure_registry(reg)?;
        let items = registry::list_artifacts(&repo_dir, reg, artifact_type)?;
        for name in &items {
            if let Err(e) = add(project_dir, name, Some(reg_name), artifact_type) {
                eprintln!("  {name}: {e}");
            }
        }
        return Ok(());
    }
    for name in names {
        add(project_dir, name, registry_name, artifact_type)?;
    }
    Ok(())
}

/// Remove manifest entries whose registry is not configured on this machine.
/// Operates across all artifact types (skills, agents, rules).
pub fn prune(project_dir: &Path) -> Result<()> {
    let config = Config::load()?;
    let mut manifest = Manifest::load(project_dir)?;
    let configured: std::collections::HashSet<String> =
        config.registry.iter().map(|r| r.name.clone()).collect();

    let mut total = 0usize;
    for at in ALL_TYPES {
        let stale: Vec<String> = manifest
            .section(at)
            .iter()
            .filter_map(|(name, entry)| {
                entry.registry.as_ref().and_then(|reg| {
                    if !configured.contains(reg) {
                        Some(name.clone())
                    } else {
                        None
                    }
                })
            })
            .collect();
        for name in &stale {
            let reg = manifest.section(at)[name]
                .registry
                .as_deref()
                .unwrap_or("?");
            eprintln!(
                "  {} {name} ({}) (registry: {reg})",
                color::red("pruned"),
                at.singular()
            );
            manifest.section_mut(at).remove(name);
        }
        total += stale.len();
    }

    if total == 0 {
        eprintln!("No stale entries found.");
        return Ok(());
    }

    manifest.save(project_dir)?;
    eprintln!(
        "Pruned {total} entr{}. Run `rune sync` to update.",
        if total == 1 { "y" } else { "ies" }
    );
    Ok(())
}
