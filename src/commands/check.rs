use anyhow::Result;
use std::path::Path;

use super::{DriftDirection, SkillStatus, resolve_registry_typed};
use crate::config::Config;
use crate::lockfile::Lockfile;
use crate::manifest::{ALL_TYPES, ArtifactType, Manifest};
use crate::registry;

/// Check a single item of any type against its registry.
/// Uses lockfile for accurate drift direction instead of unreliable mtime.
pub(crate) fn check_item(
    name: &str,
    entry: &crate::manifest::SkillEntry,
    config: &Config,
    project_dir: &Path,
    lockfile: &Lockfile,
    artifact_type: ArtifactType,
) -> Result<(String, String, SkillStatus)> {
    let reg = resolve_registry_typed(name, entry, config, artifact_type)?;
    let repo_dir = registry::ensure_registry(reg)?;
    let reg_path = registry::artifact_path(&repo_dir, reg, name, artifact_type);

    let artifact_dir = Manifest::load(project_dir)
        .map(|m| m.artifact_dir(project_dir, artifact_type))
        .unwrap_or_else(|_| project_dir.join(artifact_type.default_dir()));

    let local_path = if artifact_type.is_directory_type() && registry::is_directory_skill(&reg_path)
    {
        artifact_dir.join(name)
    } else {
        artifact_dir.join(format!("{name}.md"))
    };

    let status = match (local_path.exists(), reg_path.exists()) {
        (false, false) => SkillStatus::RegistryMissing,
        (false, true) => SkillStatus::Missing,
        (true, false) => SkillStatus::RegistryMissing,
        (true, true) => {
            let local_hash = registry::skill_hash(&local_path)?;
            let reg_hash = registry::skill_hash(&reg_path)?;
            if local_hash == reg_hash {
                SkillStatus::Current
            } else {
                // Use lockfile for drift direction
                let direction = if let Some(locked) = lockfile.section(artifact_type).get(name) {
                    let local_changed = local_hash != locked.hash;
                    let reg_changed = reg_hash != locked.hash;
                    match (local_changed, reg_changed) {
                        (true, false) => DriftDirection::LocalNewer,
                        (false, true) => DriftDirection::RegistryNewer,
                        _ => DriftDirection::Diverged,
                    }
                } else {
                    DriftDirection::Diverged
                };
                SkillStatus::Drifted { direction }
            }
        }
    };

    Ok((name.to_string(), reg.name.clone(), status))
}

/// Check all items in the project manifest.
pub fn check(
    project_dir: &Path,
    file_filter: Option<&str>,
) -> Result<Vec<(String, String, SkillStatus)>> {
    let config = Config::load()?;
    let manifest = Manifest::load(project_dir)?;
    let lockfile = Lockfile::load(project_dir)?;

    let mut results = Vec::new();

    for at in ALL_TYPES {
        for (name, entry) in manifest.section(at) {
            if let Err(e) = registry::validate_name(name) {
                eprintln!("  {name}: {e}");
                continue;
            }
            if let Some(filter) = file_filter {
                let filter_stem = Path::new(filter)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                if filter_stem != *name {
                    continue;
                }
            }

            match check_item(name, entry, &config, project_dir, &lockfile, at) {
                Ok(result) => results.push(result),
                Err(e) => eprintln!("  {name}: error: {e}"),
            }
        }
    }

    Ok(results)
}
