use anyhow::{Context, Result};
use std::path::Path;

use super::{SkillStatus, check::check_skill};
use crate::color;
use crate::config::Config;
use crate::lockfile::Lockfile;
use crate::manifest::Manifest;
use crate::pedigree::{self, Pedigree};
use crate::registry;

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
