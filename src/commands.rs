use anyhow::{Context, Result};
use std::path::Path;

use crate::config::Config;
use crate::manifest::{Manifest, SkillEntry};
use crate::registry;

/// Status of a skill relative to its registry.
#[derive(Debug)]
pub enum SkillStatus {
    Current,
    Drifted { direction: DriftDirection },
    Missing,         // in manifest but not on disk
    #[allow(dead_code)]
    Unregistered,    // on disk but not in manifest
    RegistryMissing, // in manifest but skill not found in any registry
}

#[derive(Debug)]
pub enum DriftDirection {
    LocalNewer,
    RegistryNewer,
    Diverged,
}

impl std::fmt::Display for SkillStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Current => write!(f, "CURRENT"),
            Self::Drifted { direction } => {
                let dir = match direction {
                    DriftDirection::LocalNewer => "local is newer",
                    DriftDirection::RegistryNewer => "registry is newer",
                    DriftDirection::Diverged => "diverged",
                };
                write!(f, "DRIFTED  {dir}")
            }
            Self::Missing => write!(f, "MISSING"),
            Self::Unregistered => write!(f, "UNREGISTERED"),
            Self::RegistryMissing => write!(f, "REGISTRY MISSING"),
        }
    }
}

/// Resolve which registry to use for a skill.
/// If pinned in manifest, use that. Otherwise resolve by priority.
fn resolve_registry<'a>(
    skill_name: &str,
    entry: &SkillEntry,
    config: &'a Config,
) -> Result<&'a crate::config::Registry> {
    if let Some(ref pinned) = entry.registry {
        config
            .registry(pinned)
            .with_context(|| format!("Unknown registry: {pinned}"))
    } else {
        let cache_dir = Config::cache_dir()?;
        // Ensure all registries are cloned so we can search them
        for reg in &config.registry {
            let _ = registry::ensure_registry(reg);
        }
        config
            .resolve_skill(skill_name, &cache_dir)
            .with_context(|| format!("{skill_name}: not found in any registry"))
    }
}

/// Check a single skill against its registry.
fn check_skill(
    skill_name: &str,
    entry: &SkillEntry,
    config: &Config,
    project_dir: &Path,
) -> Result<(String, String, SkillStatus)> {
    let reg = resolve_registry(skill_name, entry, config)?;
    let repo_dir = registry::ensure_registry(reg)?;
    let reg_path = registry::skill_path(&repo_dir, reg, skill_name);

    let local_path = if registry::is_directory_skill(&reg_path) {
        Manifest::skills_dir(project_dir).join(skill_name)
    } else {
        Manifest::skills_dir(project_dir).join(format!("{skill_name}.md"))
    };

    let status = match (local_path.exists(), reg_path.exists()) {
        (false, false) => SkillStatus::RegistryMissing,
        (false, true) => SkillStatus::Missing,
        (true, false) => SkillStatus::RegistryMissing,
        (true, true) => {
            let local_hash = registry::skill_hash(&local_path);
            let reg_hash = registry::skill_hash(&reg_path);
            if local_hash == reg_hash {
                SkillStatus::Current
            } else {
                let local_mtime = std::fs::metadata(&local_path)
                    .and_then(|m| m.modified())
                    .ok();
                let reg_mtime = std::fs::metadata(&reg_path)
                    .and_then(|m| m.modified())
                    .ok();

                let direction = match (local_mtime, reg_mtime) {
                    (Some(l), Some(r)) if l > r => DriftDirection::LocalNewer,
                    (Some(l), Some(r)) if r > l => DriftDirection::RegistryNewer,
                    _ => DriftDirection::Diverged,
                };
                SkillStatus::Drifted { direction }
            }
        }
    };

    Ok((skill_name.to_string(), reg.name.clone(), status))
}

/// Check all skills in the project manifest.
pub fn check(project_dir: &Path, file_filter: Option<&str>) -> Result<Vec<(String, String, SkillStatus)>> {
    let config = Config::load()?;
    let manifest = Manifest::load(project_dir)?;

    let mut results = Vec::new();

    for (skill_name, entry) in &manifest.skills {
        if let Some(filter) = file_filter {
            let filter_stem = Path::new(filter)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if filter_stem != *skill_name {
                continue;
            }
        }

        match check_skill(skill_name, entry, &config, project_dir) {
            Ok(result) => results.push(result),
            Err(e) => eprintln!("  {skill_name}: error: {e}"),
        }
    }

    Ok(results)
}

/// Sync all skills from registries to the project.
pub fn sync(project_dir: &Path) -> Result<u32> {
    let config = Config::load()?;
    let manifest = Manifest::load(project_dir)?;
    let skills_dir = Manifest::skills_dir(project_dir);
    std::fs::create_dir_all(&skills_dir)?;

    let mut count = 0;

    for (skill_name, entry) in &manifest.skills {
        let reg = match resolve_registry(skill_name, entry, &config) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("  {skill_name}: {e}");
                continue;
            }
        };

        let repo_dir = registry::ensure_registry(reg)?;
        let reg_path = registry::skill_path(&repo_dir, reg, skill_name);

        if !reg_path.exists() {
            eprintln!("  {skill_name}: not found in registry {}", reg.name);
            continue;
        }

        let is_dir = registry::is_directory_skill(&reg_path);
        let local_path = if is_dir {
            skills_dir.join(skill_name)
        } else {
            skills_dir.join(format!("{skill_name}.md"))
        };

        let reg_hash = registry::skill_hash(&reg_path);
        let local_hash = if local_path.exists() {
            registry::skill_hash(&local_path)
        } else {
            None
        };

        if reg_hash != local_hash {
            if is_dir && local_path.exists() {
                std::fs::remove_dir_all(&local_path)?;
            }
            registry::copy_skill(&reg_path, &local_path)
                .with_context(|| format!("Failed to sync {skill_name}"))?;
            eprintln!("  {skill_name}: synced from {}", reg.name);
            count += 1;
        } else {
            eprintln!("  {skill_name}: current");
        }
    }

    Ok(count)
}

/// Add a skill from a registry to the project manifest and sync it.
pub fn add(project_dir: &Path, skill_name: &str, registry_name: Option<&str>) -> Result<()> {
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
    let kind = if is_dir { "dir" } else { "file" };
    eprintln!("Added {skill_name} from {} ({kind})", reg.name);
    Ok(())
}

/// Push a local skill change back to its registry.
pub fn push(project_dir: &Path, skill_name: &str) -> Result<()> {
    let config = Config::load()?;
    let manifest = Manifest::load(project_dir)?;

    let entry = manifest
        .skills
        .get(skill_name)
        .with_context(|| format!("{skill_name} not in manifest"))?;

    let reg = resolve_registry(skill_name, entry, &config)?;

    if reg.readonly {
        anyhow::bail!(
            "Registry {} is read-only. Cannot push {}.",
            reg.name,
            skill_name
        );
    }

    let repo_dir = registry::ensure_registry(reg)?;
    let reg_path = registry::skill_path(&repo_dir, reg, skill_name);

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

    // Copy local -> registry
    if reg_path.exists() && reg_path.is_dir() {
        std::fs::remove_dir_all(&reg_path)?;
    }
    registry::copy_skill(local_path, &reg_path)?;

    // Git add + commit + push
    let repo = git2::Repository::open(&repo_dir)
        .context("Failed to open registry repo")?;

    let mut index = repo.index()?;
    // Add all files in the skill (handles both file and directory)
    if local_path.is_dir() {
        let files = crate::registry::collect_files_public(&reg_path);
        for file in files {
            let relative = file.strip_prefix(&repo_dir).unwrap_or(&file);
            index.add_path(relative)?;
        }
    } else {
        let relative = reg_path.strip_prefix(&repo_dir).unwrap_or(&reg_path);
        index.add_path(relative)?;
    }
    index.write()?;

    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let head = repo.head()?.peel_to_commit()?;

    let sig = repo
        .signature()
        .unwrap_or_else(|_| git2::Signature::now("rune", "rune@localhost").unwrap());

    let message = format!(
        "update {skill_name}\n\nPushed by rune from {}",
        project_dir.display()
    );
    repo.commit(Some("HEAD"), &sig, &sig, &message, &tree, &[&head])?;

    // Push via git CLI
    let status = std::process::Command::new("git")
        .args(["push", "--quiet", "origin", &reg.branch])
        .current_dir(&repo_dir)
        .status()
        .context("Failed to run git push")?;

    if !status.success() {
        anyhow::bail!("git push failed for registry {}", reg.name);
    }

    eprintln!("Pushed {skill_name} to {}", reg.name);
    Ok(())
}

/// List all skills and their status.
pub fn ls(project_dir: &Path) -> Result<()> {
    let config = Config::load()?;
    let manifest = Manifest::load(project_dir)?;

    if manifest.skills.is_empty() {
        eprintln!("No skills in manifest. Run `rune add <skill>`.");
        return Ok(());
    }

    for (skill_name, entry) in &manifest.skills {
        match check_skill(skill_name, entry, &config, project_dir) {
            Ok((name, reg, status)) => {
                println!("  {name:<24} {status:<30} registry: {reg}");
            }
            Err(e) => {
                println!("  {skill_name:<24} ERROR: {e}");
            }
        }
    }

    Ok(())
}

/// List all available skills in a specific registry.
pub fn ls_registry(registry_name: &str) -> Result<()> {
    let config = Config::load()?;
    let reg = config
        .registry(registry_name)
        .with_context(|| format!("Unknown registry: {registry_name}"))?;

    let repo_dir = registry::ensure_registry(reg)?;
    let skills = registry::list_skills(&repo_dir, reg)?;

    if skills.is_empty() {
        eprintln!("No skills in registry {registry_name}");
    } else {
        let ro = if reg.readonly { " (read-only)" } else { "" };
        eprintln!("{registry_name}{ro}:");
        for skill in &skills {
            let path = registry::skill_path(&repo_dir, reg, skill);
            let kind = if path.is_dir() { "dir " } else { "file" };
            println!("  {skill:<24} {kind}");
        }
    }

    Ok(())
}
