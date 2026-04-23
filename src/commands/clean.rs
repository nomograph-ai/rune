use anyhow::Result;

use crate::color;
use crate::config::Config;
use crate::registry;

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
