use anyhow::{Context, Result};
use std::path::Path;

use super::resolve_registry;
use crate::color;
use crate::config::Config;
use crate::lockfile::{Lockfile, LockedSkill};
use crate::manifest::{Manifest, SkillEntry};
use crate::pedigree;
use crate::registry;

/// Add a skill from a registry to the project manifest and sync it.
pub fn add(project_dir: &Path, skill_name: &str, registry_name: Option<&str>) -> Result<()> {
    registry::validate_skill_name(skill_name)?;
    let config = Config::load()?;

    // Resolve the registry
    let reg = if let Some(name) = registry_name {
        config
            .registry(name)
            .with_context(|| format!("Unknown registry: {name}"))?
    } else {
        // Ensure all registries are pulled, then resolve
        for r in &config.registry {
            let _ = registry::ensure_registry(r);
        }
        let cache_dir = Config::cache_dir()?;
        config
            .resolve_skill(skill_name, &cache_dir)
            .with_context(|| format!("{skill_name}: not found in any registry"))?
    };

    let repo_dir = registry::ensure_registry(reg)?;
    let reg_path = registry::skill_path(&repo_dir, reg, skill_name);
    if !reg_path.exists() {
        anyhow::bail!("{skill_name} not found in registry {}", reg.name);
    }

    // Update manifest
    let mut manifest = Manifest::try_load(project_dir).unwrap_or_default();
    let entry = if registry_name.is_some() {
        SkillEntry {
            registry: Some(reg.name.clone()),
        }
    } else {
        SkillEntry { registry: None }
    };
    manifest.skills.insert(skill_name.to_string(), entry);
    manifest.save(project_dir)?;

    // Copy the skill
    let skills_dir = Manifest::skills_dir(project_dir);
    std::fs::create_dir_all(&skills_dir)?;

    let is_dir = registry::is_directory_skill(&reg_path);
    let local_path = if is_dir {
        skills_dir.join(skill_name)
    } else {
        skills_dir.join(format!("{skill_name}.md"))
    };

    registry::copy_skill(&reg_path, &local_path)?;

    // Update lockfile
    let mut lockfile = Lockfile::load(project_dir).unwrap_or_default();
    let hash = registry::skill_hash(&local_path).unwrap_or_default();
    let skill_rel = registry::skill_path_relative(reg, skill_name);
    let registry_commit = registry::skill_commit(&repo_dir, &skill_rel);
    lockfile.skills.insert(skill_name.to_string(), LockedSkill {
        registry: reg.name.clone(),
        hash,
        registry_commit,
        synced_at: pedigree::today(),
    });
    lockfile.save(project_dir)?;

    let kind = if is_dir { "dir" } else { "file" };
    eprintln!("Added {skill_name} from {} ({kind})", reg.name);
    Ok(())
}

/// Push a local skill change back to its registry. All git operations via CLI.
pub fn push(project_dir: &Path, skill_name: &str, message: Option<&str>) -> Result<()> {
    registry::validate_skill_name(skill_name)?;
    let config = Config::load()?;
    let manifest = Manifest::load(project_dir)?;

    let entry = manifest
        .skills
        .get(skill_name)
        .with_context(|| format!("{skill_name} not in manifest"))?;

    let reg = resolve_registry(skill_name, entry, &config)?;

    if reg.readonly {
        anyhow::bail!("Registry {} is read-only. Cannot push {}.", reg.name, skill_name);
    }

    let repo_dir = registry::ensure_registry(reg)?;

    let skills_dir = Manifest::skills_dir(project_dir);
    let local_dir = skills_dir.join(skill_name);
    let local_file = skills_dir.join(format!("{skill_name}.md"));

    let local_path = if local_dir.exists() {
        &local_dir
    } else if local_file.exists() {
        &local_file
    } else {
        anyhow::bail!("{skill_name} not found locally");
    };

    // Pass local format hint so new skills get the right path in the registry
    let local_is_dir = Some(local_path.is_dir());
    let reg_path = registry::skill_path_with_hint(&repo_dir, reg, skill_name, local_is_dir);

    if registry::is_dry_run() {
        eprintln!("  {}: {} to {}",
            skill_name,
            color::yellow("would push"),
            color::cyan(&reg.name));
        return Ok(());
    }

    // Copy local -> registry
    if reg_path.exists() && reg_path.is_dir() {
        std::fs::remove_dir_all(&reg_path)?;
    }
    registry::copy_skill(local_path, &reg_path)?;

    // Commit and push via git CLI
    registry::commit_and_push(&repo_dir, skill_name, reg, message)?;

    eprintln!("Pushed {} to {}", skill_name, color::cyan(&reg.name));
    Ok(())
}

/// Remove a skill from the project manifest and optionally delete local files.
pub fn remove(project_dir: &Path, skill_name: &str) -> Result<()> {
    registry::validate_skill_name(skill_name)?;
    let mut manifest = Manifest::load(project_dir)?;

    if manifest.skills.remove(skill_name).is_none() {
        anyhow::bail!("{skill_name} not in manifest");
    }
    manifest.save(project_dir)?;

    // Remove local skill files
    let skills_dir = Manifest::skills_dir(project_dir);
    let dir_path = skills_dir.join(skill_name);
    let file_path = skills_dir.join(format!("{skill_name}.md"));

    if dir_path.exists() {
        std::fs::remove_dir_all(&dir_path)?;
        eprintln!("Removed {skill_name} (directory)");
    } else if file_path.exists() {
        std::fs::remove_file(&file_path)?;
        eprintln!("Removed {skill_name} (file)");
    } else {
        eprintln!("Removed {skill_name} from manifest (no local files found)");
    }

    // Clean lockfile entry
    let mut lockfile = Lockfile::load(project_dir).unwrap_or_default();
    if lockfile.skills.remove(skill_name).is_some() {
        lockfile.save(project_dir)?;
    }

    Ok(())
}
