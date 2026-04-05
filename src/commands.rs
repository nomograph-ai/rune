use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::Path;

use crate::config::Config;
use crate::manifest::Manifest;
use crate::registry;

/// Status of a skill relative to its registry.
#[derive(Debug)]
pub enum SkillStatus {
    Current,
    Drifted { direction: DriftDirection },
    Missing,         // in manifest but not on disk
    #[allow(dead_code)]
    Unregistered,    // on disk but not in manifest
    RegistryMissing, // in manifest but not in registry
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

fn file_hash(path: &Path) -> Option<String> {
    let content = std::fs::read(path).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&content);
    Some(hex::encode(hasher.finalize()))
}

/// Check a single skill file against its registry.
fn check_skill(
    skill_name: &str,
    registry_name: &str,
    config: &Config,
    project_dir: &Path,
) -> Result<(String, String, SkillStatus)> {
    let reg = config
        .registry(registry_name)
        .with_context(|| format!("Unknown registry: {registry_name}"))?;

    let repo_dir = registry::ensure_registry(reg)?;
    let reg_path = registry::skill_path(&repo_dir, reg, skill_name);
    let local_path = Manifest::skills_dir(project_dir).join(format!("{skill_name}.md"));

    let status = match (local_path.exists(), reg_path.exists()) {
        (false, false) => SkillStatus::RegistryMissing,
        (false, true) => SkillStatus::Missing,
        (true, false) => SkillStatus::RegistryMissing,
        (true, true) => {
            let local_hash = file_hash(&local_path);
            let reg_hash = file_hash(&reg_path);
            if local_hash == reg_hash {
                SkillStatus::Current
            } else {
                // Compare modification times for direction hint
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

    Ok((skill_name.to_string(), registry_name.to_string(), status))
}

/// Check all skills in the project manifest.
pub fn check(project_dir: &Path, file_filter: Option<&str>) -> Result<Vec<(String, String, SkillStatus)>> {
    let config = Config::load()?;
    let manifest = Manifest::load(project_dir)?;

    let mut results = Vec::new();

    for (skill_name, registry_name) in &manifest.skills {
        // If filtering by file, only check the matching skill
        if let Some(filter) = file_filter {
            let filter_stem = Path::new(filter)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if filter_stem != *skill_name {
                continue;
            }
        }

        match check_skill(skill_name, registry_name, &config, project_dir) {
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

    for (skill_name, registry_name) in &manifest.skills {
        let reg = match config.registry(registry_name) {
            Some(r) => r,
            None => {
                eprintln!("  {skill_name}: unknown registry {registry_name}, skipping");
                continue;
            }
        };

        let repo_dir = registry::ensure_registry(reg)?;
        let reg_path = registry::skill_path(&repo_dir, reg, skill_name);
        let local_path = skills_dir.join(format!("{skill_name}.md"));

        if !reg_path.exists() {
            eprintln!("  {skill_name}: not found in registry {registry_name}");
            continue;
        }

        // Only copy if different
        let reg_hash = file_hash(&reg_path);
        let local_hash = if local_path.exists() {
            file_hash(&local_path)
        } else {
            None
        };

        if reg_hash != local_hash {
            std::fs::copy(&reg_path, &local_path)
                .with_context(|| format!("Failed to copy {skill_name}"))?;
            eprintln!("  {skill_name}: synced from {registry_name}");
            count += 1;
        } else {
            eprintln!("  {skill_name}: current");
        }
    }

    Ok(count)
}

/// Add a skill from a registry to the project manifest and sync it.
pub fn add(project_dir: &Path, skill_name: &str, registry_name: &str) -> Result<()> {
    let config = Config::load()?;
    let reg = config
        .registry(registry_name)
        .with_context(|| format!("Unknown registry: {registry_name}"))?;

    // Verify skill exists in registry
    let repo_dir = registry::ensure_registry(reg)?;
    let reg_path = registry::skill_path(&repo_dir, reg, skill_name);
    if !reg_path.exists() {
        anyhow::bail!("{skill_name} not found in registry {registry_name}");
    }

    // Update manifest
    let mut manifest = Manifest::try_load(project_dir).unwrap_or_default();
    manifest
        .skills
        .insert(skill_name.to_string(), registry_name.to_string());
    manifest.save(project_dir)?;

    // Copy the skill file
    let skills_dir = Manifest::skills_dir(project_dir);
    std::fs::create_dir_all(&skills_dir)?;
    let local_path = skills_dir.join(format!("{skill_name}.md"));
    std::fs::copy(&reg_path, &local_path)?;

    eprintln!("Added {skill_name} from {registry_name}");
    Ok(())
}

/// Push a local skill change back to its registry.
pub fn push(project_dir: &Path, skill_name: &str) -> Result<()> {
    let config = Config::load()?;
    let manifest = Manifest::load(project_dir)?;

    let registry_name = manifest
        .skills
        .get(skill_name)
        .with_context(|| format!("{skill_name} not in manifest"))?;

    let reg = config
        .registry(registry_name)
        .with_context(|| format!("Unknown registry: {registry_name}"))?;

    let repo_dir = registry::ensure_registry(reg)?;
    let reg_path = registry::skill_path(&repo_dir, reg, skill_name);
    let local_path = Manifest::skills_dir(project_dir).join(format!("{skill_name}.md"));

    if !local_path.exists() {
        anyhow::bail!("{skill_name}.md not found locally");
    }

    // Copy local -> registry
    if let Some(parent) = reg_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(&local_path, &reg_path)
        .context("Failed to copy skill to registry")?;

    // Git add + commit + push
    let repo = git2::Repository::open(&repo_dir)
        .context("Failed to open registry repo")?;

    let mut index = repo.index()?;
    let relative = reg_path
        .strip_prefix(&repo_dir)
        .unwrap_or(&reg_path);
    index.add_path(relative)?;
    index.write()?;

    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let head = repo.head()?.peel_to_commit()?;

    let sig = repo.signature()
        .unwrap_or_else(|_| git2::Signature::now("rune", "rune@localhost").unwrap());

    let message = format!("update {skill_name}\n\nPushed by rune from {}", project_dir.display());
    repo.commit(Some("HEAD"), &sig, &sig, &message, &tree, &[&head])?;

    // Push to origin via git CLI (handles both local and remote repos)
    let status = std::process::Command::new("git")
        .args(["push", "origin", &reg.branch])
        .current_dir(&repo_dir)
        .status()
        .context("Failed to run git push")?;

    if !status.success() {
        anyhow::bail!("git push failed for registry {registry_name}");
    }

    eprintln!("Pushed {skill_name} to {registry_name}");
    Ok(())
}

/// List all skills and their status.
pub fn ls(project_dir: &Path) -> Result<()> {
    let config = Config::load()?;
    let manifest = Manifest::load(project_dir)?;

    if manifest.skills.is_empty() {
        eprintln!("No skills in manifest. Run `rune add <skill> --from <registry>`.");
        return Ok(());
    }

    for (skill_name, registry_name) in &manifest.skills {
        match check_skill(skill_name, registry_name, &config, project_dir) {
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
