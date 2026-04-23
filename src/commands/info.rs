use anyhow::{Context, Result};
use std::path::Path;

use super::{SkillStatus, check::check_item};
use crate::color;
use crate::config::Config;
use crate::lockfile::Lockfile;
use crate::manifest::{ALL_TYPES, Manifest};
use crate::pedigree::{self, Pedigree};
use crate::registry;

/// List all items and their status.
pub fn ls(project_dir: &Path) -> Result<()> {
    let config = Config::load()?;
    let manifest = Manifest::load(project_dir)?;
    let lockfile = Lockfile::load(project_dir)?;

    if manifest.total_count() == 0 {
        eprintln!("No items in manifest. Run `rune add <name>`.");
        return Ok(());
    }

    for at in ALL_TYPES {
        let section = manifest.section(at);
        if section.is_empty() {
            continue;
        }

        eprintln!("{}", color::bold(at.section()));
        for (name, entry) in section {
            if let Err(e) = registry::validate_name(name) {
                eprintln!("  {name}: {e}");
                continue;
            }
            match check_item(name, entry, &config, project_dir, &lockfile, at) {
                Ok((item_name, reg, status)) => {
                    println!(
                        "  {item_name:<24} {:<30} registry: {}",
                        status.colored(),
                        color::cyan(&reg)
                    );
                }
                Err(e) => {
                    println!("  {name:<24} {}", color::red(&format!("ERROR: {e}")));
                }
            }
        }
    }

    Ok(())
}

/// List all available items in a specific registry.
pub fn ls_registry(registry_name: &str) -> Result<()> {
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

    let mut total = 0;
    for at in ALL_TYPES {
        let items = registry::list_artifacts(&repo_dir, reg, at)?;
        if items.is_empty() {
            continue;
        }
        total += items.len();

        eprintln!("{}{ro} -- {}:", color::cyan(registry_name), at.section());
        for item in &items {
            if at.is_directory_type() {
                let path = registry::artifact_path(&repo_dir, reg, item, at);
                let kind = if path.is_dir() { "dir " } else { "file" };
                println!("  {item:<24} {}", color::dim(kind));
            } else {
                println!("  {item:<24} {}", color::dim("file"));
            }
        }
    }

    if total == 0 {
        eprintln!("No items in registry {}", color::cyan(registry_name));
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
        eprintln!(
            "  config: {} {}",
            config_path.display(),
            color::red("MISSING")
        );
        eprintln!("  run: rune setup");
        return Ok(());
    }

    let config = Config::load()?;

    // Validate registries
    let mut names = std::collections::HashSet::new();
    for reg in &config.registry {
        if !names.insert(&reg.name) {
            eprintln!(
                "  registry {}: {}",
                color::cyan(&reg.name),
                color::red("DUPLICATE NAME")
            );
            continue;
        }
        if reg.url.is_empty() {
            eprintln!(
                "  registry {}: {}",
                color::cyan(&reg.name),
                color::red("EMPTY URL")
            );
            continue;
        }

        let cache_dir = Config::cache_dir()?;
        let repo_dir = cache_dir.join(reg.fs_name());
        let ro = if reg.readonly {
            color::dim(" (readonly)")
        } else {
            String::new()
        };
        let src = color::dim(&format!(" [{}]", reg.source));
        let auth = if reg.token_env.is_some() {
            color::dim(&format!(
                " (${} auth)",
                reg.token_env.as_deref().unwrap_or("?")
            ))
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
            let skills =
                registry::list_artifacts(&repo_dir, reg, crate::manifest::ArtifactType::Skill)
                    .unwrap_or_default();
            eprintln!(
                "  registry {}{ro}{src}{auth}{identity}: {} skills {}",
                color::cyan(&reg.name),
                skills.len(),
                color::green("ok")
            );
        } else {
            eprintln!(
                "  registry {}{ro}{src}{auth}{identity}: {}",
                color::cyan(&reg.name),
                color::dim("not cached")
            );
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
        let lf = Lockfile::load(project_dir)?;
        eprintln!(
            "  lockfile: {} entries locked {}",
            lf.total_count(),
            color::green("ok")
        );
    } else {
        eprintln!(
            "  lockfile: {}",
            color::dim("none (run rune sync to create)")
        );
    }

    // Manifest health: check for entries referencing unconfigured registries
    let configured: std::collections::HashSet<String> =
        config.registry.iter().map(|r| r.name.clone()).collect();
    if let Some(manifest) = Manifest::try_load(project_dir)? {
        let mut stale: Vec<(String, String, &'static str)> = Vec::new();
        for at in ALL_TYPES {
            for (name, entry) in manifest.section(at) {
                if let Some(reg) = entry.registry.as_deref()
                    && !configured.contains(reg)
                {
                    stale.push((name.clone(), reg.to_string(), at.singular()));
                }
            }
        }
        if stale.is_empty() {
            eprintln!(
                "  manifest: {} entr{} {}",
                manifest.total_count(),
                if manifest.total_count() == 1 {
                    "y"
                } else {
                    "ies"
                },
                color::green("ok")
            );
        } else {
            eprintln!(
                "  manifest: {} entr{} reference unconfigured registr{}:",
                stale.len(),
                if stale.len() == 1 { "y" } else { "ies" },
                if stale.len() == 1 { "y" } else { "ies" }
            );
            for (name, reg, kind) in &stale {
                eprintln!(
                    "    {} ({kind}) (registry: {})",
                    color::yellow(name),
                    color::cyan(reg)
                );
            }
            eprintln!("  run: rune prune  (remove stale entries)");
        }
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

    let configured: std::collections::HashSet<String> =
        config.registry.iter().map(|r| r.name.clone()).collect();

    let mut removed = 0;
    for entry in std::fs::read_dir(&cache_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip lock/etag/header files -- they'll be cleaned with their registry
        if name.starts_with('.') {
            // Check if it's a stale metadata file for a removed registry
            let base = registry::parse_cache_metadata_name(&name);
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
                eprintln!(
                    "  {} {} (not in config)",
                    color::yellow("would remove"),
                    name
                );
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
        eprintln!(
            "\n{} item(s) would be removed. Run without --dry-run to delete.",
            removed
        );
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
        let skills =
            registry::list_artifacts(&repo_dir, reg, crate::manifest::ArtifactType::Skill)?;

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
                            eprintln!(
                                "  {:<24} {:>4} lines  {} (upstream: {} lines, {pct:+}%)",
                                skill_name,
                                local_lines,
                                color::red("REGRESSED"),
                                upstream_lines
                            );
                            issues += 1;
                        } else if pct > 50 {
                            eprintln!(
                                "  {:<24} {:>4} lines  {} (upstream: {} lines, {pct:+}%)",
                                skill_name,
                                local_lines,
                                color::green("EXTENDED"),
                                upstream_lines
                            );
                        } else {
                            let modified = if ped.modified == Some(true) {
                                color::yellow(" (modified)")
                            } else {
                                String::new()
                            };
                            eprintln!(
                                "  {:<24} {:>4} lines  from {}{modified}",
                                skill_name,
                                local_lines,
                                color::dim(origin)
                            );
                        }
                    } else {
                        eprintln!(
                            "  {:<24} {:>4} lines  {} (not in upstream)",
                            skill_name,
                            local_lines,
                            color::yellow("REMOVED UPSTREAM")
                        );
                    }
                }
            } else {
                eprintln!(
                    "  {:<24} {:>4} lines  from {} {}",
                    skill_name,
                    local_lines,
                    origin,
                    color::dim("(registry not configured)")
                );
            }
        }
        eprintln!();
    }

    if issues > 0 {
        eprintln!(
            "{}",
            color::red(&format!(
                "{issues} skill(s) may have lost content. Review with `rune diff <skill>`."
            ))
        );
        std::process::exit(1);
    } else {
        eprintln!("All skills look healthy.");
    }

    Ok(())
}

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

/// Count lines in a skill (all .md + .sh + .txt + .toml files).
fn count_lines(path: &Path) -> usize {
    if path.is_dir() {
        let mut total = 0;
        for entry in registry::fs::collect_files(path) {
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
