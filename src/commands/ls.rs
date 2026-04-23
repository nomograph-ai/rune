use anyhow::{Context, Result};
use std::path::Path;

use super::check::check_item;
use crate::color;
use crate::config::Config;
use crate::lockfile::Lockfile;
use crate::manifest::{ALL_TYPES, Manifest};
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
