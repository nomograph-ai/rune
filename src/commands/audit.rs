use anyhow::Result;
use std::path::Path;

use crate::color;
use crate::config::Config;
use crate::pedigree::{self, Pedigree};
use crate::registry;

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
            let ped = Pedigree::from_skill_or_warn(&skill_path);

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
