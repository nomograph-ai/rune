use anyhow::{Context, Result};
use std::path::Path;

use super::check::check_skill;
use super::{resolve_registry, SkillStatus};
use crate::color;
use crate::config::Config;
use crate::lockfile::{Lockfile, LockedSkill};
use crate::manifest::Manifest;
use crate::pedigree::{self, Pedigree};
use crate::registry;

/// Sync all skills from registries to the project.
/// Writes a lockfile recording exactly what was installed.
/// Detects locally modified imported skills and updates pedigree.
pub fn sync(project_dir: &Path, force: bool) -> Result<u32> {
    let config = Config::load()?;
    let manifest = Manifest::load(project_dir)?;
    let mut lockfile = Lockfile::load(project_dir).unwrap_or_default();
    let skills_dir = Manifest::skills_dir(project_dir);
    let dry_run = registry::is_dry_run();

    if !dry_run {
        std::fs::create_dir_all(&skills_dir)?;
    }

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
            eprintln!("  {}: not found in registry {}",
                skill_name, color::cyan(&reg.name));
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

        // Detect local modifications via lockfile
        let locally_modified = if let Some(locked) = lockfile.skills.get(skill_name) {
            local_hash.as_deref() != Some(locked.hash.as_str()) && local_path.exists()
        } else {
            false
        };

        if reg_hash != local_hash {
            if locally_modified && !force {
                // Local was changed since last sync -- don't overwrite
                eprintln!("  {}: {} (run `rune sync --force` or `rune push {}` first)",
                    skill_name,
                    color::yellow("skipped -- locally modified"),
                    skill_name);

                // Mark pedigree as modified for imported skills
                if !dry_run {
                    let ped = Pedigree::from_skill(&local_path).unwrap_or_default();
                    if ped.has_origin() && ped.modified != Some(true) {
                        let updated_ped = Pedigree {
                            modified: Some(true),
                            ..ped
                        };
                        let _ = updated_ped.write_to_skill(&local_path);
                    }
                }
                continue;
            }

            if dry_run {
                eprintln!("  {}: {} from {}",
                    skill_name,
                    color::yellow("would sync"),
                    color::cyan(&reg.name));
            } else {
                if is_dir && local_path.exists() {
                    std::fs::remove_dir_all(&local_path)?;
                }
                registry::copy_skill(&reg_path, &local_path)
                    .with_context(|| format!("Failed to sync {skill_name}"))?;
                eprintln!("  {}: synced from {}",
                    skill_name, color::cyan(&reg.name));
            }
            count += 1;
        } else {
            eprintln!("  {}: {}", skill_name, color::green("current"));
        }

        // Record in lockfile (even for current skills, to keep lockfile complete)
        if !dry_run {
            let hash = registry::skill_hash(&local_path)
                .unwrap_or_default();
            let skill_rel = registry::skill_path_relative(reg, skill_name);
            let registry_commit = registry::skill_commit(&repo_dir, &skill_rel);
            lockfile.skills.insert(skill_name.to_string(), LockedSkill {
                registry: reg.name.clone(),
                hash,
                registry_commit,
                synced_at: pedigree::today(),
            });
        }
    }

    // Remove lockfile entries for skills no longer in manifest
    if !dry_run {
        lockfile.skills.retain(|name, _| manifest.skills.contains_key(name));
        lockfile.save(project_dir)?;

        // Multi-agent support: .agent/skills/ symlink + AGENTS.md
        ensure_agent_symlink(project_dir)?;
        generate_agents_md(project_dir, &manifest)?;

        // Bundled enforcement: verify state after sync.
        // Re-load lockfile (just saved) and check all skills.
        let lockfile = Lockfile::load(project_dir).unwrap_or_default();
        let mut drifted = 0u32;
        for (skill_name, entry) in &manifest.skills {
            if let Ok((name, _, status)) =
                check_skill(skill_name, entry, &config, project_dir, &lockfile)
                && !matches!(status, SkillStatus::Current)
            {
                eprintln!(
                    "  {}: {} after sync",
                    name,
                    color::yellow("still drifted")
                );
                drifted += 1;
            }
        }
        if drifted > 0 {
            eprintln!(
                "\n{}",
                color::yellow(&format!(
                    "{drifted} skill(s) still drifted after sync. Run `rune check` for details."
                ))
            );
        }
    }

    Ok(count)
}

/// Ensure .agent/skills/ exists as a symlink to .claude/skills/.
/// This gives non-Claude agents (Cursor, Windsurf, Copilot, Aider)
/// access to the same skills via the agentskills.io convention.
fn ensure_agent_symlink(project_dir: &Path) -> Result<()> {
    let agent_skills = project_dir.join(".agent").join("skills");
    let claude_skills = project_dir.join(".claude").join("skills");

    if !claude_skills.exists() {
        return Ok(());
    }

    // If .agent/skills already exists and points to the right place, done
    if agent_skills.symlink_metadata().is_ok() {
        if agent_skills.read_link().ok().as_deref() == Some(&claude_skills) {
            return Ok(());
        }
        // Wrong target -- remove and recreate
        if agent_skills.is_dir() && !agent_skills.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false) {
            // It's a real directory, not a symlink -- leave it alone
            return Ok(());
        }
        let _ = std::fs::remove_file(&agent_skills);
    }

    std::fs::create_dir_all(project_dir.join(".agent"))?;

    #[cfg(unix)]
    std::os::unix::fs::symlink(&claude_skills, &agent_skills)
        .with_context(|| "Failed to create .agent/skills symlink")?;

    Ok(())
}

/// Generate AGENTS.md at the project root with skill metadata.
/// This is the agentskills.io interop format -- parsed by Cursor,
/// Windsurf, Copilot, Aider, and other agent-enabled editors.
fn generate_agents_md(project_dir: &Path, manifest: &Manifest) -> Result<()> {
    let agents_path = project_dir.join("AGENTS.md");

    let mut skills_xml = String::new();
    for skill_name in manifest.skills.keys() {
        if registry::validate_skill_name(skill_name).is_err() {
            continue;
        }
        let skills_dir = Manifest::skills_dir(project_dir);
        let skill_path = skills_dir.join(skill_name);
        let skill_file = if skill_path.is_dir() {
            skill_path.join("SKILL.md")
        } else {
            skills_dir.join(format!("{skill_name}.md"))
        };

        let description = if skill_file.exists() {
            let ped = Pedigree::from_skill(&skill_path).unwrap_or_default();
            ped.description.unwrap_or_else(|| format!("{skill_name} skill"))
        } else {
            format!("{skill_name} skill")
        };

        skills_xml.push_str(&format!(
            "\n<skill>\n<name>{skill_name}</name>\n<description>{description}</description>\n<location>project</location>\n</skill>\n"
        ));
    }

    let content = format!(
        r#"# AGENTS

<!-- Generated by rune sync. Do not edit manually. -->

<skills_system priority="1">

## Available Skills

When users ask you to perform tasks, check if any of the available skills
below can help complete the task more effectively. Skills provide specialized
capabilities and domain knowledge.

How to use skills:
- Skills are loaded from `.claude/skills/` (or `.agent/skills/`)
- Each skill directory contains a `SKILL.md` with detailed instructions
- Skills are managed by [rune](https://gitlab.com/nomograph/rune)

<available_skills>
{skills_xml}
</available_skills>

</skills_system>
"#
    );

    std::fs::write(&agents_path, content)
        .with_context(|| "Failed to write AGENTS.md")?;

    Ok(())
}
