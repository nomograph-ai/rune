use anyhow::Result;
use std::path::Path;

use crate::color;
use crate::config::Config;
use crate::lockfile::Lockfile;
use crate::manifest::{ALL_TYPES, Manifest};
use crate::registry;

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
