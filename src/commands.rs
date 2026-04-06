use anyhow::{Context, Result};
use std::path::Path;

use crate::config::Config;
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
    let kind = if is_dir { "dir" } else { "file" };
    eprintln!("Added {skill_name} from {} ({kind})", reg.name);
    Ok(())
}

/// Push a local skill change back to its registry.
pub fn push(project_dir: &Path, skill_name: &str) -> Result<()> {
    registry::validate_skill_name(skill_name)?;
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
        .or_else(|_| git2::Signature::now("rune", "rune@localhost"))
        .context("Failed to create git signature")?;

    let message = format!("update {skill_name}\n\nPushed by rune");
    repo.commit(Some("HEAD"), &sig, &sig, &message, &tree, &[&head])?;

    // Push via git CLI
    let status = std::process::Command::new("git")
        .args(["push", "--quiet", "origin", "--"])
        .arg(&reg.branch)
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
        if let Err(e) = registry::validate_skill_name(skill_name) {
            eprintln!("  {skill_name}: {e}");
            continue;
        }
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

    let ro = if reg.readonly { " (read-only)" } else { "" };
    eprintln!("{registry_name}{ro}: {} skills\n", skills.len());

    for skill in &skills {
        let path = registry::skill_path(&repo_dir, reg, skill);
        let pedigree = Pedigree::from_skill(&path).unwrap_or_default();
        let desc = pedigree
            .description
            .unwrap_or_else(|| "-".to_string());
        // Truncate description for display (safe for multi-byte UTF-8)
        let desc_short = if desc.chars().count() > 70 {
            let truncated: String = desc.chars().take(67).collect();
            format!("{truncated}...")
        } else {
            desc
        };
        println!("  {skill:<24} {desc_short}");
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

    // Get upstream commit hash
    let upstream_commit = pedigree::repo_head_short(&source_dir)
        .unwrap_or_else(|| "unknown".to_string());

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
    let cache_dir = Config::cache_dir()?;
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

            // Check upstream HEAD
            let source_dir = cache_dir.join(&source_reg.name);
            if !source_dir.exists() {
                let _ = registry::ensure_registry(source_reg);
            }

            let upstream_commit = pedigree::repo_head_short(&source_dir)
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

    eprintln!("rune: {} upstream update(s) available\n", updates.len());
    eprintln!("  {:<20} {:<30} {:<10} {:<10} STATUS", "SKILL", "ORIGIN", "LOCAL", "UPSTREAM");

    for (name, origin, local, upstream, modified) in &updates {
        let status = if *modified { "MODIFIED" } else { "UPDATED" };
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

    let upstream_commit = pedigree::repo_head_short(&source_dir)
        .unwrap_or_else(|| "unknown".to_string());

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
        "  updated {skill_name} from {} (commit {upstream_commit})",
        source_reg.name
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
