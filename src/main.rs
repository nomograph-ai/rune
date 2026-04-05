mod commands;
mod config;
mod manifest;
mod registry;
mod setup;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "rune",
    version,
    about = "Skill registry manager for AI coding agents",
    long_about = "rune syncs markdown skill files from git-based registries into .claude/skills/.\n\n\
        Skills are inscribed knowledge -- reusable instructions that teach AI agents \
        how to perform specific workflows. rune keeps them current across projects.\n\n\
        Example:\n  \
        rune setup                    # one-time: create config, install hook\n  \
        rune init                     # per-project: create manifest\n  \
        rune add tidy --from public   # add a skill from a registry\n  \
        rune sync                     # pull latest from registries\n  \
        rune check                    # show drift between local and registries\n  \
        rune push tidy                # push local changes back to registry\n  \
        rune ls                       # list skills and their status"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Project directory (defaults to current directory)
    #[arg(long, global = true)]
    project: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// One-time global setup: create config, install Claude Code hook
    Setup,

    /// Initialize rune in the current project (creates .claude/rune.toml)
    Init,

    /// Add a skill from a registry to this project
    ///
    /// Example: rune add tidy --from public
    Add {
        /// Skill name (without .md extension)
        skill: String,

        /// Registry to pull from
        #[arg(long)]
        from: String,
    },

    /// Check for drift between local skills and registries
    ///
    /// Shows which skills have changed locally or in the registry.
    /// When called with --file, checks only the specified skill.
    Check {
        /// Check a specific file (used by hook)
        #[arg(long)]
        file: Option<String>,
    },

    /// Sync all skills from registries (pull updates)
    ///
    /// Pulls the latest version of each skill from its registry.
    /// Only overwrites files that have actually changed.
    Sync,

    /// Push a local skill change back to its registry
    ///
    /// Commits and pushes the local version of a skill to the
    /// registry it came from. Other projects will get this change
    /// on their next `rune sync`.
    Push {
        /// Skill name to push
        skill: String,
    },

    /// List all skills and their sync status
    Ls,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let project_dir = cli.project.unwrap_or_else(|| {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    });

    match cli.command {
        Commands::Setup => setup::setup(),
        Commands::Init => setup::init(&project_dir),
        Commands::Add { skill, from } => commands::add(&project_dir, &skill, &from),
        Commands::Check { file } => {
            let results = commands::check(&project_dir, file.as_deref())?;
            for (name, reg, status) in &results {
                println!("  {name:<24} {status:<30} registry: {reg}");
            }
            // Exit non-zero if any drift (for hook usage)
            if results
                .iter()
                .any(|(_, _, s)| !matches!(s, commands::SkillStatus::Current))
            {
                std::process::exit(1);
            }
            Ok(())
        }
        Commands::Sync => {
            let count = commands::sync(&project_dir)?;
            eprintln!("Synced {count} skills.");
            Ok(())
        }
        Commands::Push { skill } => commands::push(&project_dir, &skill),
        Commands::Ls => commands::ls(&project_dir),
    }
}
