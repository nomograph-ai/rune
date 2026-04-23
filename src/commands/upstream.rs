use anyhow::{Context, Result};

use crate::color;
use crate::config::Config;
use crate::manifest::{ALL_TYPES, ArtifactType};
use crate::pedigree::{self, Pedigree};
use crate::registry;

/// Parse a skill@registry reference.
fn parse_skill_ref(skill_ref: &str) -> Result<(&str, &str)> {
    skill_ref
        .split_once('@')
        .with_context(|| format!("Expected skill@registry format, got: {skill_ref}"))
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

/// Browse available items in a registry. Shows all types when filter is None.
pub fn browse(registry_name: &str, type_filter: Option<ArtifactType>) -> Result<()> {
    let config = Config::load()?;
    let reg = config
        .registry(registry_name)
        .with_context(|| format!("Unknown registry: {registry_name}"))?;

    let repo_dir = registry::ensure_registry(reg)?;
    let ro = if reg.readonly {
        color::dim(" (read-only)")
    } else {
        String::new()
    };

    let types = match type_filter {
        Some(t) => vec![t],
        None => ALL_TYPES.to_vec(),
    };

    let mut total = 0;
    for at in &types {
        let items = registry::list_artifacts(&repo_dir, reg, *at)?;
        if items.is_empty() {
            continue;
        }

        total += items.len();
        eprintln!(
            "{}{ro} -- {} ({} items)\n",
            color::cyan(registry_name),
            at.section(),
            items.len()
        );

        for item in &items {
            let path = registry::artifact_path(&repo_dir, reg, item, *at);
            let pedigree = Pedigree::from_skill_or_warn(&path);
            let desc = pedigree.description.unwrap_or_else(|| "-".to_string());
            let desc_short = if desc.chars().count() > 70 {
                let truncated: String = desc.chars().take(67).collect();
                format!("{truncated}...")
            } else {
                desc
            };
            println!("  {item:<24} {}", color::dim(&desc_short));
        }
        eprintln!();
    }

    if total == 0 {
        eprintln!("No items in registry {registry_name}");
    }

    Ok(())
}

/// Import a skill from an upstream registry into your own registry.
pub fn import(skill_ref: &str, target_name: Option<&str>) -> Result<()> {
    let config = Config::load()?;
    let (skill_name, source_name) = parse_skill_ref(skill_ref)?;
    registry::validate_name(skill_name)?;

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
    let upstream_commit =
        registry::skill_commit(&source_dir, &skill_rel).unwrap_or_else(|| "unknown".to_string());

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
        let skills =
            registry::list_artifacts(&repo_dir, reg, crate::manifest::ArtifactType::Skill)?;

        for skill_name in &skills {
            let skill_path = registry::skill_path(&repo_dir, reg, skill_name);
            let ped = Pedigree::from_skill_or_warn(&skill_path);

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

    eprintln!(
        "{}\n",
        color::yellow(&format!(
            "rune: {} upstream update(s) available",
            updates.len()
        ))
    );
    eprintln!(
        "  {:<20} {:<30} {:<10} {:<10} STATUS",
        "SKILL", "ORIGIN", "LOCAL", "UPSTREAM"
    );

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
    registry::validate_name(skill_name)?;
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
        anyhow::bail!(
            "{skill_name} no longer exists in upstream {}",
            source_reg.name
        );
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
                "--label",
                &format!("{skill_name} (imported)"),
                "--label",
                &format!("{skill_name} (upstream)"),
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
                "--label",
                &format!("{skill_name} (imported)"),
                "--label",
                &format!("{skill_name} (upstream)"),
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
    registry::validate_name(skill_name)?;
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
        anyhow::bail!(
            "{skill_name} no longer exists in upstream {}",
            source_reg.name
        );
    }

    let skill_rel = registry::skill_path_relative(source_reg, skill_name);
    let upstream_commit =
        registry::skill_commit(&source_dir, &skill_rel).unwrap_or_else(|| "unknown".to_string());

    if registry::is_dry_run() {
        eprintln!(
            "  {}: {} from {} (commit {upstream_commit})",
            skill_name,
            color::yellow("would update"),
            color::cyan(&source_reg.name)
        );
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
        skill_name,
        color::cyan(&source_reg.name)
    );
    eprintln!("  push: rune push {skill_name}");

    Ok(())
}
