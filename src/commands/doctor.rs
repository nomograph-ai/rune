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

    // Manifest health: entries referencing unconfigured registries.
    // Uses config.registry(...) so aliases resolve — a manifest entry
    // that matches a registry alias is considered configured.
    if let Some(manifest) = Manifest::try_load(project_dir)? {
        let mut stale: Vec<(String, String, &'static str)> = Vec::new();
        for at in ALL_TYPES {
            for (name, entry) in manifest.section(at) {
                if let Some(reg) = entry.registry.as_deref()
                    && config.registry(reg).is_none()
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
            eprintln!(
                "  fix: add `aliases = [\"<old-name>\"]` to the renamed registry in \
                 config.toml, edit the manifest entries, or run `rune prune`."
            );
        }
    }

    // Lockfile drift health: entries whose `registry` field doesn't
    // resolve in the current config (even via alias). This catches
    // registry renames that happened between syncs — the lock carries
    // the old canonical name, the config is under a new name, no
    // alias bridges them yet. Proactive surface; commands that use
    // this lock would fail at runtime otherwise.
    if lockfile_path.exists()
        && let Ok(lf) = Lockfile::load(project_dir)
    {
        let mut drifted: Vec<(String, String, &'static str)> = Vec::new();
        for at in ALL_TYPES {
            for (name, locked) in lf.section(at) {
                if config.registry(&locked.registry).is_none() {
                    drifted.push((name.clone(), locked.registry.clone(), at.singular()));
                }
            }
        }
        if drifted.is_empty() {
            // quiet: covered by the lockfile ok line above
        } else {
            eprintln!(
                "  {}",
                color::yellow(&format!(
                    "lockfile: {} entr{} reference unresolvable registr{}:",
                    drifted.len(),
                    if drifted.len() == 1 { "y" } else { "ies" },
                    if drifted.len() == 1 { "y" } else { "ies" }
                ))
            );
            for (name, reg, kind) in &drifted {
                eprintln!(
                    "    {} ({kind}) (lock registry: {})",
                    color::yellow(name),
                    color::cyan(reg)
                );
            }
            eprintln!(
                "  fix: add `aliases = [\"{}\"]` to the corresponding registry in \
                 config.toml (backward-compatible), or re-sync to refresh the lock.",
                drifted
                    .first()
                    .map(|(_, r, _)| r.as_str())
                    .unwrap_or("<old-name>")
            );
        }
    }

    eprintln!();
    Ok(())
}
