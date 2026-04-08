use anyhow::{Context, Result};
use std::path::Path;

use crate::color;
use crate::config::Config;
use crate::lockfile::{Lockfile, LockedSkill};
use crate::manifest::{Manifest, SkillEntry};
use crate::pedigree::{self, Pedigree};
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

impl SkillStatus {
    /// Colored string representation.
    pub fn colored(&self) -> String {
        match self {
            Self::Current => color::green("CURRENT"),
            Self::Drifted { direction } => {
                let dir = match direction {
                    DriftDirection::LocalNewer => "local is newer",
                    DriftDirection::RegistryNewer => "registry is newer",
                    DriftDirection::Diverged => "diverged",
                };
                color::yellow(&format!("DRIFTED  {dir}"))
            }
            Self::Missing => color::red("MISSING"),
            Self::Unregistered => color::yellow("UNREGISTERED"),
            Self::RegistryMissing => color::red("REGISTRY MISSING"),
        }
    }
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
/// Uses lockfile for accurate drift direction instead of unreliable mtime.
fn check_skill(
    skill_name: &str,
    entry: &SkillEntry,
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

/// Sync all skills from registries to the project.
/// Writes a lockfile recording exactly what was installed.
/// Detects locally modified imported skills and updates pedigree.
pub fn sync(project_dir: &Path, force: bool) -> Result<u32> {
    let config = Config::load()?;
    let manifest = Manifest::load(project_dir)?;
    let mut lockfile = Lockfile::load(project_dir).unwrap_or_default();
    let skills_dir = Manifest::skills_dir(project_dir);
    let dry_run = registry::is_dry_run();

    if !dry_run {
        std::fs::create_dir_all(&skills_dir)?;
    }

    let mut count = 0;

    for (skill_name, entry) in &manifest.skills {
        if let Err(e) = registry::validate_skill_name(skill_name) {
            eprintln!("  {skill_name}: {e}");
            continue;
        }
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
            eprintln!("  {}: not found in registry {}",
                skill_name, color::cyan(&reg.name));
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

        // Detect local modifications via lockfile
        let locally_modified = if let Some(locked) = lockfile.skills.get(skill_name) {
            local_hash.as_deref() != Some(locked.hash.as_str()) && local_path.exists()
        } else {
            false
        };

        if reg_hash != local_hash {
            if locally_modified && !force {
                // Local was changed since last sync -- don't overwrite
                eprintln!("  {}: {} (run `rune sync --force` or `rune push {}` first)",
                    skill_name,
                    color::yellow("skipped -- locally modified"),
                    skill_name);

                // Mark pedigree as modified for imported skills
                if !dry_run {
                    let ped = Pedigree::from_skill(&local_path).unwrap_or_default();
                    if ped.has_origin() && ped.modified != Some(true) {
                        let updated_ped = Pedigree {
                            modified: Some(true),
                            ..ped
                        };
                        let _ = updated_ped.write_to_skill(&local_path);
                    }
                }
                continue;
            }

            if dry_run {
                eprintln!("  {}: {} from {}",
                    skill_name,
                    color::yellow("would sync"),
                    color::cyan(&reg.name));
            } else {
                if is_dir && local_path.exists() {
                    std::fs::remove_dir_all(&local_path)?;
                }
                registry::copy_skill(&reg_path, &local_path)
                    .with_context(|| format!("Failed to sync {skill_name}"))?;
                eprintln!("  {}: synced from {}",
                    skill_name, color::cyan(&reg.name));
            }
            count += 1;
        } else {
            eprintln!("  {}: {}", skill_name, color::green("current"));
        }

        // Record in lockfile (even for current skills, to keep lockfile complete)
        if !dry_run {
            let hash = registry::skill_hash(&local_path)
                .unwrap_or_default();
            let skill_rel = registry::skill_path_relative(reg, skill_name);
            let registry_commit = registry::skill_commit(&repo_dir, &skill_rel);
            lockfile.skills.insert(skill_name.to_string(), LockedSkill {
                registry: reg.name.clone(),
                hash,
                registry_commit,
                synced_at: pedigree::today(),
            });
        }
    }

    // Remove lockfile entries for skills no longer in manifest
    if !dry_run {
        lockfile.skills.retain(|name, _| manifest.skills.contains_key(name));
        lockfile.save(project_dir)?;
    }

    Ok(count)
}

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

/// List all skills and their status.
pub fn ls(project_dir: &Path) -> Result<()> {
    let config = Config::load()?;
    let manifest = Manifest::load(project_dir)?;
    let lockfile = Lockfile::load(project_dir).unwrap_or_default();

    if manifest.skills.is_empty() {
        eprintln!("No skills in manifest. Run `rune add <skill>`.");
        return Ok(());
    }

    for (skill_name, entry) in &manifest.skills {
        if let Err(e) = registry::validate_skill_name(skill_name) {
            eprintln!("  {skill_name}: {e}");
            continue;
        }
        match check_skill(skill_name, entry, &config, project_dir, &lockfile) {
            Ok((name, reg, status)) => {
                println!("  {name:<24} {:<30} registry: {}",
                    status.colored(), color::cyan(&reg));
            }
            Err(e) => {
                println!("  {skill_name:<24} {}", color::red(&format!("ERROR: {e}")));
            }
        }
    }

    Ok(())
}

/// Parse a skill@registry reference.
fn parse_skill_ref(skill_ref: &str) -> Result<(&str, &str)> {
    skill_ref
        .split_once('@')
        .with_context(|| format!("Expected skill@registry format, got: {skill_ref}"))
}

/// Browse available skills in an upstream registry with descriptions.
pub fn browse(registry_name: &str) -> Result<()> {
    let config = Config::load()?;
    let reg = config
        .registry(registry_name)
        .with_context(|| format!("Unknown registry: {registry_name}"))?;

    let repo_dir = registry::ensure_registry(reg)?;
    let skills = registry::list_skills(&repo_dir, reg)?;

    if skills.is_empty() {
        eprintln!("No skills in registry {registry_name}");
        return Ok(());
    }

    let ro = if reg.readonly { color::dim(" (read-only)") } else { String::new() };
    eprintln!("{}{ro}: {} skills\n", color::cyan(registry_name), skills.len());

    for skill in &skills {
        let path = registry::skill_path(&repo_dir, reg, skill);
        let pedigree = Pedigree::from_skill(&path).unwrap_or_default();
        let desc = pedigree
            .description
            .unwrap_or_else(|| "-".to_string());
        let desc_short = if desc.chars().count() > 70 {
            let truncated: String = desc.chars().take(67).collect();
            format!("{truncated}...")
        } else {
            desc
        };
        println!("  {skill:<24} {}", color::dim(&desc_short));
    }

    Ok(())
}

/// Import a skill from an upstream registry into your own registry.
pub fn import(skill_ref: &str, target_name: Option<&str>) -> Result<()> {
    let config = Config::load()?;
    let (skill_name, source_name) = parse_skill_ref(skill_ref)?;
    registry::validate_skill_name(skill_name)?;

    // Resolve source registry
    let source_reg = config
        .registry(source_name)
        .with_context(|| format!("Unknown registry: {source_name}"))?;

    // Resolve target registry (first writable, or specified)
    let target_reg = if let Some(name) = target_name {
        config
            .registry(name)
            .with_context(|| format!("Unknown registry: {name}"))?
    } else {
        config
            .registry
            .iter()
            .find(|r| !r.readonly)
            .with_context(|| "No writable registry found. Specify --to <registry>.")?
    };

    if target_reg.readonly {
        anyhow::bail!("Target registry {} is read-only", target_reg.name);
    }

    // Ensure both registries are current
    let source_dir = registry::ensure_registry(source_reg)?;
    let target_dir = registry::ensure_registry(target_reg)?;

    // Find the skill in source
    let source_path = registry::skill_path(&source_dir, source_reg, skill_name);
    if !source_path.exists() {
        anyhow::bail!(
            "{skill_name} not found in registry {source_name}. Run `rune browse {source_name}` to see available skills."
        );
    }

    // Check if already exists in target
    let target_path = registry::skill_path(&target_dir, target_reg, skill_name);
    if target_path.exists() {
        anyhow::bail!(
            "{skill_name} already exists in {target}. Use `rune update {skill_name}` to pull upstream changes.",
            target = target_reg.name
        );
    }

    // Determine the origin path (how the skill is stored in the upstream repo)
    let origin_path = match &source_reg.path {
        Some(p) => format!("{p}/{skill_name}"),
        None => skill_name.to_string(),
    };

    // Get the skill-specific commit hash from the upstream registry
    let skill_rel = registry::skill_path_relative(source_reg, skill_name);
    let upstream_commit = registry::skill_commit(&source_dir, &skill_rel)
        .unwrap_or_else(|| "unknown".to_string());

    // Reject symlink sources
    if source_path.symlink_metadata()?.file_type().is_symlink() {
        anyhow::bail!("Refusing to import symlink: {}", source_path.display());
    }

    // Copy to target as a directory skill
    let target_skill_dir = if source_path.is_dir() {
        target_dir.join(skill_name)
    } else {
        // Convert flat file to directory format on import
        let dir = target_dir.join(skill_name);
        std::fs::create_dir_all(&dir)?;
        let dest = dir.join("SKILL.md");
        std::fs::copy(&source_path, &dest)?;
        eprintln!("  converted flat file to directory format");
        dir
    };

    if source_path.is_dir() {
        registry::copy_skill(&source_path, &target_skill_dir)?;
    }

    // Write pedigree into the imported skill
    let ped = Pedigree {
        origin: Some(pedigree::url_to_slug(&source_reg.url)),
        origin_path: Some(origin_path),
        imported: Some(pedigree::today()),
        upstream_commit: Some(upstream_commit.clone()),
        modified: Some(false),
        ..Default::default()
    };
    ped.write_to_skill(&target_skill_dir)?;

    eprintln!(
        "  imported {skill_name} from {source_name} → {target}",
        target = target_reg.name
    );
    eprintln!("  pedigree: origin={source_name}, commit={upstream_commit}");
    eprintln!();
    eprintln!("  review: {}", target_skill_dir.display());
    eprintln!("  push:   rune push {skill_name}");

    Ok(())
}

/// Check imported skills for upstream updates.
pub fn upstream(quiet: bool) -> Result<()> {
    let config = Config::load()?;
    let mut updates = Vec::new();

    // Scan all writable registries for skills with pedigree
    for reg in &config.registry {
        if reg.readonly {
            continue;
        }

        let repo_dir = registry::ensure_registry(reg)?;
        let skills = registry::list_skills(&repo_dir, reg)?;

        for skill_name in &skills {
            let skill_path = registry::skill_path(&repo_dir, reg, skill_name);
            let ped = Pedigree::from_skill(&skill_path).unwrap_or_default();

            if !ped.has_origin() {
                continue; // Not imported, skip
            }

            let origin = ped.origin.as_deref().unwrap_or("unknown");
            let imported_commit = ped.upstream_commit.as_deref().unwrap_or("unknown");

            // Find the source registry by matching origin against registry URLs
            let source_reg = config.registry.iter().find(|r| {
                let slug = pedigree::url_to_slug(&r.url);
                origin.contains(&slug) || origin == r.name
            });

            let source_reg = match source_reg {
                Some(r) => r,
                None => {
                    if !quiet {
                        eprintln!("  {skill_name}: origin {origin} not in config, skipping");
                    }
                    continue;
                }
            };

            // Check the specific skill's last commit in the upstream registry
            let source_dir = registry::ensure_registry(source_reg)?;
            let skill_rel = registry::skill_path_relative(source_reg, skill_name);
            let upstream_commit = registry::skill_commit(&source_dir, &skill_rel)
                .unwrap_or_else(|| "unknown".to_string());

            if upstream_commit != imported_commit {
                updates.push((
                    skill_name.clone(),
                    origin.to_string(),
                    imported_commit.to_string(),
                    upstream_commit,
                    ped.modified.unwrap_or(false),
                ));
            }
        }
    }

    if updates.is_empty() {
        if !quiet {
            eprintln!("All imported skills are current with upstream.");
        }
        return Ok(());
    }

    eprintln!("{}\n",
        color::yellow(&format!("rune: {} upstream update(s) available", updates.len())));
    eprintln!("  {:<20} {:<30} {:<10} {:<10} STATUS", "SKILL", "ORIGIN", "LOCAL", "UPSTREAM");

    for (name, origin, local, upstream, modified) in &updates {
        let status = if *modified {
            color::red("MODIFIED")
        } else {
            color::yellow("UPDATED")
        };
        eprintln!("  {name:<20} {origin:<30} {local:<10} {upstream:<10} {status}");
    }

    eprintln!();
    eprintln!("Run `rune diff <skill>` to review changes.");
    eprintln!("Run `rune update <skill>` to pull updates.");

    Ok(())
}

/// Show diff between imported skill and upstream version.
pub fn diff(skill_name: &str) -> Result<()> {
    registry::validate_skill_name(skill_name)?;
    let config = Config::load()?;

    // Find the skill in a writable registry
    let (reg, repo_dir) = find_imported_skill(&config, skill_name)?;
    let skill_path = registry::skill_path(&repo_dir, reg, skill_name);
    let ped = Pedigree::from_skill(&skill_path)?;

    if !ped.has_origin() {
        anyhow::bail!("{skill_name} was not imported from upstream (no pedigree)");
    }

    let origin = ped.origin.as_deref().unwrap_or("unknown");
    let ped_origin_path = ped.origin_path.as_deref().unwrap_or(skill_name);

    // Find source registry
    let source_reg = config
        .registry
        .iter()
        .find(|r| {
            let slug = pedigree::url_to_slug(&r.url);
            origin.contains(&slug) || origin == r.name
        })
        .with_context(|| format!("Source registry for {origin} not in config"))?;

    let source_dir = registry::ensure_registry(source_reg)?;
    let source_path = registry::skill_path(&source_dir, source_reg, skill_name);

    if !source_path.exists() {
        anyhow::bail!("{skill_name} no longer exists in upstream {}", source_reg.name);
    }

    eprintln!("origin: {origin}");
    eprintln!("origin_path: {ped_origin_path}");
    eprintln!(
        "imported: {} (commit {})",
        ped.imported.as_deref().unwrap_or("unknown"),
        ped.upstream_commit.as_deref().unwrap_or("unknown")
    );
    eprintln!();

    // Diff the full directory tree if both are directories
    if skill_path.is_dir() && source_path.is_dir() {
        let status = std::process::Command::new("diff")
            .args([
                "-ru",
                "--label", &format!("{skill_name} (imported)"),
                "--label", &format!("{skill_name} (upstream)"),
            ])
            .arg(&skill_path)
            .arg(&source_path)
            .status()
            .context("Failed to run diff")?;

        if status.success() {
            eprintln!("No differences.");
        }
    } else {
        // Fall back to single-file diff
        let local_file = if skill_path.is_dir() {
            skill_path.join("SKILL.md")
        } else {
            skill_path.clone()
        };
        let upstream_file = if source_path.is_dir() {
            source_path.join("SKILL.md")
        } else {
            source_path.clone()
        };

        let status = std::process::Command::new("diff")
            .args([
                "-u",
                "--label", &format!("{skill_name} (imported)"),
                "--label", &format!("{skill_name} (upstream)"),
            ])
            .arg(&local_file)
            .arg(&upstream_file)
            .status()
            .context("Failed to run diff")?;

        if status.success() {
            eprintln!("No differences.");
        }
    }

    Ok(())
}

/// Pull upstream changes for an imported skill.
pub fn update(skill_name: &str, force: bool) -> Result<()> {
    registry::validate_skill_name(skill_name)?;
    let config = Config::load()?;

    let (reg, repo_dir) = find_imported_skill(&config, skill_name)?;
    let skill_path = registry::skill_path(&repo_dir, reg, skill_name);
    let ped = Pedigree::from_skill(&skill_path)?;

    if !ped.has_origin() {
        anyhow::bail!("{skill_name} was not imported from upstream (no pedigree)");
    }

    if ped.modified == Some(true) && !force {
        eprintln!("WARNING: {skill_name} was modified locally since import.");
        eprintln!("Local changes will be overwritten.");
        eprintln!();
        eprintln!("Options:");
        eprintln!("  rune diff {skill_name}            # see what upstream changed");
        eprintln!("  rune update {skill_name} --force   # overwrite local changes");
        eprintln!("  (cancel)                          # keep local version");
        anyhow::bail!("Use --force to overwrite local modifications");
    }

    let origin = ped.origin.as_deref().unwrap_or("unknown");

    // Find source registry
    let source_reg = config
        .registry
        .iter()
        .find(|r| {
            let slug = pedigree::url_to_slug(&r.url);
            origin.contains(&slug) || origin == r.name
        })
        .with_context(|| format!("Source registry for {origin} not in config"))?;

    let source_dir = registry::ensure_registry(source_reg)?;
    let source_path = registry::skill_path(&source_dir, source_reg, skill_name);

    if !source_path.exists() {
        anyhow::bail!("{skill_name} no longer exists in upstream {}", source_reg.name);
    }

    let skill_rel = registry::skill_path_relative(source_reg, skill_name);
    let upstream_commit = registry::skill_commit(&source_dir, &skill_rel)
        .unwrap_or_else(|| "unknown".to_string());

    if registry::is_dry_run() {
        eprintln!("  {}: {} from {} (commit {upstream_commit})",
            skill_name,
            color::yellow("would update"),
            color::cyan(&source_reg.name));
        return Ok(());
    }

    // Remove old and copy new
    if skill_path.exists() && skill_path.is_dir() {
        std::fs::remove_dir_all(&skill_path)?;
    }
    registry::copy_skill(&source_path, &skill_path)?;

    // Update pedigree
    let new_ped = Pedigree {
        imported: Some(pedigree::today()),
        upstream_commit: Some(upstream_commit.clone()),
        modified: Some(false),
        origin: ped.origin,
        origin_path: ped.origin_path,
        ..Default::default()
    };
    new_ped.write_to_skill(&skill_path)?;

    eprintln!(
        "  updated {} from {} (commit {upstream_commit})",
        skill_name, color::cyan(&source_reg.name)
    );
    eprintln!("  push: rune push {skill_name}");

    Ok(())
}

/// Find an imported skill in a writable registry.
fn find_imported_skill<'a>(
    config: &'a Config,
    skill_name: &str,
) -> Result<(&'a crate::config::Registry, std::path::PathBuf)> {
    for reg in &config.registry {
        if reg.readonly {
            continue;
        }
        let repo_dir = registry::ensure_registry(reg)?;
        let path = registry::skill_path(&repo_dir, reg, skill_name);
        if path.exists() {
            return Ok((reg, repo_dir));
        }
    }
    anyhow::bail!("{skill_name} not found in any writable registry")
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
        eprintln!("No skills in registry {}", color::cyan(registry_name));
    } else {
        let ro = if reg.readonly { color::dim(" (read-only)") } else { String::new() };
        eprintln!("{}{ro}:", color::cyan(registry_name));
        for skill in &skills {
            let path = registry::skill_path(&repo_dir, reg, skill);
            let kind = if path.is_dir() { "dir " } else { "file" };
            println!("  {skill:<24} {}", color::dim(kind));
        }
    }

    Ok(())
}

/// Diagnose configuration and registry health.
pub fn doctor(project_dir: &Path) -> Result<()> {
    eprintln!("{}\n", color::bold("rune doctor"));

    // Config
    let config_path = Config::path()?;
    if config_path.exists() {
        eprintln!("  config: {} {}", config_path.display(), color::green("ok"));
    } else {
        eprintln!("  config: {} {}", config_path.display(), color::red("MISSING"));
        eprintln!("  run: rune setup");
        return Ok(());
    }

    let config = Config::load()?;

    // Validate registries
    let mut names = std::collections::HashSet::new();
    for reg in &config.registry {
        if !names.insert(&reg.name) {
            eprintln!("  registry {}: {}", color::cyan(&reg.name), color::red("DUPLICATE NAME"));
            continue;
        }
        if reg.url.is_empty() {
            eprintln!("  registry {}: {}", color::cyan(&reg.name), color::red("EMPTY URL"));
            continue;
        }

        let cache_dir = Config::cache_dir()?;
        let repo_dir = cache_dir.join(&reg.name);
        let ro = if reg.readonly { color::dim(" (readonly)") } else { String::new() };
        let src = color::dim(&format!(" [{}]", reg.source));
        let auth = if reg.token_env.is_some() {
            color::dim(&format!(" (${} auth)", reg.token_env.as_deref().unwrap_or("?")))
        } else if reg.url.contains("gitlab.com") || reg.url.contains("github.com") {
            color::dim(" (cli auth)")
        } else {
            String::new()
        };
        let identity = match (&reg.git_email, &reg.git_name) {
            (Some(e), _) => color::dim(&format!(" <{e}>")),
            _ => String::new(),
        };

        if repo_dir.exists() {
            let skills = registry::list_skills(&repo_dir, reg).unwrap_or_default();
            eprintln!("  registry {}{ro}{src}{auth}{identity}: {} skills {}",
                color::cyan(&reg.name), skills.len(), color::green("ok"));
        } else {
            eprintln!("  registry {}{ro}{src}{auth}{identity}: {}",
                color::cyan(&reg.name), color::dim("not cached"));
        }
    }

    // Hook
    let hook_path = Config::config_dir()?.join("hook.sh");
    if hook_path.exists() {
        eprintln!("  hook: {} {}", hook_path.display(), color::green("ok"));
    } else {
        eprintln!("  hook: {}", color::yellow("not installed"));
        eprintln!("  run: rune setup");
    }

    // Mode
    if registry::is_offline() {
        eprintln!("  mode: {}", color::yellow("offline"));
    } else {
        eprintln!("  mode: online");
    }
    if registry::is_dry_run() {
        eprintln!("  mode: {}", color::yellow("dry-run"));
    }

    // Lockfile
    let lockfile_path = Lockfile::path(project_dir);
    if lockfile_path.exists() {
        let lf = Lockfile::load(project_dir).unwrap_or_default();
        eprintln!("  lockfile: {} skills locked {}", lf.skills.len(), color::green("ok"));
    } else {
        eprintln!("  lockfile: {}", color::dim("none (run rune sync to create)"));
    }

    eprintln!();
    Ok(())
}

/// Remove stale registry caches that don't match any configured registry.
pub fn clean() -> Result<()> {
    let config = Config::load()?;
    let cache_dir = Config::cache_dir()?;
    let dry_run = registry::is_dry_run();

    if !cache_dir.exists() {
        eprintln!("Cache directory does not exist. Nothing to clean.");
        return Ok(());
    }

    let configured: std::collections::HashSet<String> = config
        .registry
        .iter()
        .map(|r| r.name.clone())
        .collect();

    let mut removed = 0;
    for entry in std::fs::read_dir(&cache_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip lock/etag/header files -- they'll be cleaned with their registry
        if name.starts_with('.') {
            // Check if it's a stale metadata file for a removed registry
            let base = name.trim_start_matches('.')
                .trim_end_matches(".lock")
                .trim_end_matches(".etag")
                .trim_end_matches("-headers.txt")
                .trim_end_matches("-archive.tar.gz")
                .trim_end_matches("-extract");
            if !configured.contains(base) {
                if dry_run {
                    eprintln!("  {} {}", color::yellow("would remove"), name);
                } else {
                    let path = entry.path();
                    if path.is_dir() {
                        std::fs::remove_dir_all(&path)?;
                    } else {
                        std::fs::remove_file(&path)?;
                    }
                    eprintln!("  {} {}", color::red("removed"), name);
                }
                removed += 1;
            }
            continue;
        }

        if !configured.contains(&name) {
            if dry_run {
                eprintln!("  {} {} (not in config)", color::yellow("would remove"), name);
            } else {
                let path = entry.path();
                if path.is_dir() {
                    std::fs::remove_dir_all(&path)?;
                } else {
                    std::fs::remove_file(&path)?;
                }
                eprintln!("  {} {} (not in config)", color::red("removed"), name);
            }
            removed += 1;
        }
    }

    if removed == 0 {
        eprintln!("Cache is clean. Nothing to remove.");
    } else if dry_run {
        eprintln!("\n{} item(s) would be removed. Run without --dry-run to delete.", removed);
    } else {
        eprintln!("\nRemoved {} stale cache item(s).", removed);
    }

    Ok(())
}

/// Audit skill content: compare each skill in writable registries against
/// its upstream source (if imported). Flags size regressions that may
/// indicate content was lost during migration.
pub fn audit() -> Result<()> {
    let config = Config::load()?;
    let mut issues = 0u32;

    for reg in &config.registry {
        if reg.readonly {
            continue;
        }

        let repo_dir = registry::ensure_registry(reg)?;
        let skills = registry::list_skills(&repo_dir, reg)?;

        eprintln!("{} ({} skills)\n", color::bold(&reg.name), skills.len());

        for skill_name in &skills {
            let skill_path = registry::skill_path(&repo_dir, reg, skill_name);
            let local_lines = count_lines(&skill_path);
            let ped = Pedigree::from_skill(&skill_path).unwrap_or_default();

            if !ped.has_origin() {
                // Not imported -- just report size
                eprintln!("  {:<24} {:>4} lines", skill_name, local_lines);
                continue;
            }

            let origin = ped.origin.as_deref().unwrap_or("unknown");

            // Find source registry and compare
            let source_reg = config.registry.iter().find(|r| {
                let slug = pedigree::url_to_slug(&r.url);
                origin.contains(&slug) || origin == r.name
            });

            if let Some(source_reg) = source_reg {
                if let Ok(source_dir) = registry::ensure_registry(source_reg) {
                    let source_path = registry::skill_path(&source_dir, source_reg, skill_name);
                    if source_path.exists() {
                        let upstream_lines = count_lines(&source_path);
                        let delta = local_lines as i64 - upstream_lines as i64;
                        let pct = if upstream_lines > 0 {
                            (delta * 100) / upstream_lines as i64
                        } else {
                            0
                        };

                        if pct < -20 {
                            eprintln!("  {:<24} {:>4} lines  {} (upstream: {} lines, {pct:+}%)",
                                skill_name, local_lines,
                                color::red("REGRESSED"),
                                upstream_lines);
                            issues += 1;
                        } else if pct > 50 {
                            eprintln!("  {:<24} {:>4} lines  {} (upstream: {} lines, {pct:+}%)",
                                skill_name, local_lines,
                                color::green("EXTENDED"),
                                upstream_lines);
                        } else {
                            let modified = if ped.modified == Some(true) {
                                color::yellow(" (modified)")
                            } else {
                                String::new()
                            };
                            eprintln!("  {:<24} {:>4} lines  from {}{modified}",
                                skill_name, local_lines,
                                color::dim(origin));
                        }
                    } else {
                        eprintln!("  {:<24} {:>4} lines  {} (not in upstream)",
                            skill_name, local_lines,
                            color::yellow("REMOVED UPSTREAM"));
                    }
                }
            } else {
                eprintln!("  {:<24} {:>4} lines  from {} {}",
                    skill_name, local_lines,
                    origin,
                    color::dim("(registry not configured)"));
            }
        }
        eprintln!();
    }

    if issues > 0 {
        eprintln!("{}", color::red(
            &format!("{issues} skill(s) may have lost content. Review with `rune diff <skill>`.")));
        std::process::exit(1);
    } else {
        eprintln!("All skills look healthy.");
    }

    Ok(())
}

/// Count lines in a skill (all .md + .sh + .txt files).
fn count_lines(path: &Path) -> usize {
    if path.is_dir() {
        let mut total = 0;
        for entry in walkdir_simple(path) {
            let ext = entry.extension().and_then(|e| e.to_str()).unwrap_or("");
            if matches!(ext, "md" | "sh" | "txt" | "toml") {
                total += std::fs::read_to_string(&entry)
                    .map(|s| s.lines().count())
                    .unwrap_or(0);
            }
        }
        total
    } else {
        std::fs::read_to_string(path)
            .map(|s| s.lines().count())
            .unwrap_or(0)
    }
}

/// Simple recursive directory walk, skipping dotfiles and symlinks.
fn walkdir_simple(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return files,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if ft.is_symlink() {
            continue;
        }
        let path = entry.path();
        if ft.is_dir() {
            files.extend(walkdir_simple(&path));
        } else {
            files.push(path);
        }
    }
    files
}

/// Combined status view: registries + project skills + upstream updates.
pub fn status(project_dir: &Path) -> Result<()> {
    let config = Config::load()?;

    // Registries
    eprintln!("{}", color::bold("registries"));
    for reg in &config.registry {
        let cache_dir = Config::cache_dir()?;
        let repo_dir = cache_dir.join(&reg.name);
        let ro = if reg.readonly { color::dim(" (ro)") } else { String::new() };
        let src = color::dim(&format!(" [{}]", reg.source));

        if repo_dir.exists() {
            let skills = registry::list_skills(&repo_dir, reg).unwrap_or_default();
            eprintln!("  {}{ro}{src}: {} skills",
                color::cyan(&reg.name), skills.len());
        } else {
            eprintln!("  {}{ro}{src}: {}",
                color::cyan(&reg.name), color::dim("not cached"));
        }
    }

    // Project skills
    let manifest = match Manifest::try_load(project_dir) {
        Some(m) => m,
        None => {
            eprintln!("\n{}", color::dim("No rune.toml in this project."));
            return Ok(());
        }
    };

    let lockfile = Lockfile::load(project_dir).unwrap_or_default();
    let mut current = 0u32;
    let mut drifted = 0u32;
    let mut missing = 0u32;

    eprintln!("\n{} ({} skills)", color::bold("project"), manifest.skills.len());
    for (skill_name, entry) in &manifest.skills {
        if let Err(e) = registry::validate_skill_name(skill_name) {
            eprintln!("  {skill_name}: {e}");
            continue;
        }
        match check_skill(skill_name, entry, &config, project_dir, &lockfile) {
            Ok((name, reg, status)) => {
                match &status {
                    SkillStatus::Current => current += 1,
                    SkillStatus::Drifted { .. } => drifted += 1,
                    _ => missing += 1,
                }
                println!("  {name:<24} {:<30} {}", status.colored(), color::dim(&reg));
            }
            Err(e) => {
                missing += 1;
                println!("  {skill_name:<24} {}", color::red(&format!("ERROR: {e}")));
            }
        }
    }

    let summary = format!("{} current, {} drifted, {} missing",
        current, drifted, missing);
    eprintln!("  {}", if drifted > 0 || missing > 0 {
        color::yellow(&summary)
    } else {
        color::green(&summary)
    });

    // Upstream updates (scan writable registries for imported skills)
    let mut updates = Vec::new();
    for reg in &config.registry {
        if reg.readonly {
            continue;
        }
        let repo_dir = match registry::ensure_registry(reg) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let skills = registry::list_skills(&repo_dir, reg).unwrap_or_default();
        for skill_name in &skills {
            let skill_path = registry::skill_path(&repo_dir, reg, skill_name);
            let ped = Pedigree::from_skill(&skill_path).unwrap_or_default();
            if !ped.has_origin() {
                continue;
            }
            let imported_commit = ped.upstream_commit.as_deref().unwrap_or("unknown");
            let origin = ped.origin.as_deref().unwrap_or("unknown");
            let source_reg = config.registry.iter().find(|r| {
                let slug = pedigree::url_to_slug(&r.url);
                origin.contains(&slug) || origin == r.name
            });
            if let Some(source_reg) = source_reg
                && let Ok(source_dir) = registry::ensure_registry(source_reg)
            {
                let skill_rel = registry::skill_path_relative(source_reg, skill_name);
                let upstream_commit = registry::skill_commit(&source_dir, &skill_rel)
                    .unwrap_or_else(|| "unknown".to_string());
                if upstream_commit != imported_commit {
                    updates.push((skill_name.clone(), origin.to_string()));
                }
            }
        }
    }

    if !updates.is_empty() {
        eprintln!("\n{}", color::yellow(
            &format!("{} upstream update(s) available", updates.len())));
        for (name, origin) in &updates {
            eprintln!("  {} from {}", name, color::dim(origin));
        }
        eprintln!("  run: rune upstream");
    }

    Ok(())
}
