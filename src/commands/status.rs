use anyhow::Result;
use std::path::Path;

use super::SkillStatus;
use super::check::check_item;
use crate::color;
use crate::config::Config;
use crate::lockfile::Lockfile;
use crate::manifest::{ALL_TYPES, Manifest};
use crate::pedigree::{self, Pedigree};
use crate::registry;

/// Combined status view: registries + project items + upstream updates.
pub fn status(project_dir: &Path) -> Result<()> {
    let config = Config::load()?;

    // Registries
    eprintln!("{}", color::bold("registries"));
    for reg in &config.registry {
        let cache_dir = Config::cache_dir()?;
        let repo_dir = cache_dir.join(reg.fs_name());
        let ro = if reg.readonly {
            color::dim(" (ro)")
        } else {
            String::new()
        };
        let src = color::dim(&format!(" [{}]", reg.source));

        if repo_dir.exists() {
            let skills =
                registry::list_artifacts(&repo_dir, reg, crate::manifest::ArtifactType::Skill)
                    .unwrap_or_default();
            eprintln!(
                "  {}{ro}{src}: {} skills",
                color::cyan(&reg.name),
                skills.len()
            );
        } else {
            eprintln!(
                "  {}{ro}{src}: {}",
                color::cyan(&reg.name),
                color::dim("not cached")
            );
        }
    }

    // Project items
    let manifest = match Manifest::try_load(project_dir)? {
        Some(m) => m,
        None => {
            eprintln!("\n{}", color::dim("No rune.toml in this project."));
            return Ok(());
        }
    };

    let lockfile = Lockfile::load(project_dir)?;
    let mut current = 0u32;
    let mut drifted = 0u32;
    let mut missing = 0u32;

    let total = manifest.total_count();
    eprintln!("\n{} ({} items)", color::bold("project"), total);

    for at in ALL_TYPES {
        let section = manifest.section(at);
        if section.is_empty() {
            continue;
        }
        for (name, entry) in section {
            if let Err(e) = registry::validate_name(name) {
                eprintln!("  {name}: {e}");
                continue;
            }
            match check_item(name, entry, &config, project_dir, &lockfile, at) {
                Ok((item_name, reg, status)) => {
                    match &status {
                        SkillStatus::Current => current += 1,
                        SkillStatus::Drifted { .. } => drifted += 1,
                        _ => missing += 1,
                    }
                    println!(
                        "  {item_name:<24} {:<30} {}",
                        status.colored(),
                        color::dim(&reg)
                    );
                }
                Err(e) => {
                    missing += 1;
                    println!("  {name:<24} {}", color::red(&format!("ERROR: {e}")));
                }
            }
        }
    }

    let summary = format!(
        "{} current, {} drifted, {} missing",
        current, drifted, missing
    );
    eprintln!(
        "  {}",
        if drifted > 0 || missing > 0 {
            color::yellow(&summary)
        } else {
            color::green(&summary)
        }
    );

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
        let skills = registry::list_artifacts(&repo_dir, reg, crate::manifest::ArtifactType::Skill)
            .unwrap_or_default();
        for skill_name in &skills {
            let skill_path = registry::skill_path(&repo_dir, reg, skill_name);
            let ped = Pedigree::from_skill_or_warn(&skill_path);
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
                let skill_rel = registry::skill_path_relative(&source_dir, source_reg, skill_name);
                let upstream_commit = registry::skill_commit(&source_dir, &skill_rel)
                    .unwrap_or_else(|| "unknown".to_string());
                if upstream_commit != imported_commit {
                    updates.push((skill_name.clone(), origin.to_string()));
                }
            }
        }
    }

    if !updates.is_empty() {
        eprintln!(
            "\n{}",
            color::yellow(&format!("{} upstream update(s) available", updates.len()))
        );
        for (name, origin) in &updates {
            eprintln!("  {} from {}", name, color::dim(origin));
        }
        eprintln!("  run: rune upstream");
    }

    Ok(())
}
