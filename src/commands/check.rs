use anyhow::Result;
use std::path::Path;

use super::{resolve_registry, DriftDirection, SkillStatus};
use crate::config::Config;
use crate::lockfile::Lockfile;
use crate::manifest::Manifest;
use crate::registry;

/// Check a single skill against its registry.
/// Uses lockfile for accurate drift direction instead of unreliable mtime.
pub(crate) fn check_skill(
    skill_name: &str,
    entry: &crate::manifest::SkillEntry,
    config: &Config,
    project_dir: &Path,
    lockfile: &Lockfile,
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
                // Use lockfile for drift direction
                let direction = if let Some(locked) = lockfile.skills.get(skill_name) {
                    let local_changed = local_hash.as_deref() != Some(locked.hash.as_str());
                    let reg_changed = reg_hash.as_deref() != Some(locked.hash.as_str());
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

    Ok((skill_name.to_string(), reg.name.clone(), status))
}

/// Check all skills in the project manifest.
pub fn check(project_dir: &Path, file_filter: Option<&str>) -> Result<Vec<(String, String, SkillStatus)>> {
    let config = Config::load()?;
    let manifest = Manifest::load(project_dir)?;
    let lockfile = Lockfile::load(project_dir).unwrap_or_default();

    let mut results = Vec::new();

    for (skill_name, entry) in &manifest.skills {
        if let Err(e) = registry::validate_skill_name(skill_name) {
            eprintln!("  {skill_name}: {e}");
            continue;
        }
        if let Some(filter) = file_filter {
            let filter_stem = Path::new(filter)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if filter_stem != *skill_name {
                continue;
            }
        }

        match check_skill(skill_name, entry, &config, project_dir, &lockfile) {
            Ok(result) => results.push(result),
            Err(e) => eprintln!("  {skill_name}: error: {e}"),
        }
    }

    Ok(results)
}
