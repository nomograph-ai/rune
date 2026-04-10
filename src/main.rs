#![deny(warnings, clippy::all)]

mod color;
mod commands;
mod config;
mod lockfile;
mod manifest;
mod pedigree;
mod registry;
mod setup;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
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
        rune add tidy --from runes    # add a skill from a registry\n  \
        rune sync                     # pull latest from registries\n  \
        rune check                    # show drift between local and registries\n  \
        rune push tidy                # push local changes back to registry\n  \
        rune browse k-dense           # discover upstream skills\n  \
        rune import scanpy@k-dense    # import from upstream\n  \
        rune status                   # combined summary view\n  \
        rune ls                       # list skills and their status"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Project directory (defaults to current directory)
    #[arg(long, global = true)]
    project: Option<PathBuf>,

    /// Use cached registries without pulling (no network)
    #[arg(long, global = true)]
    offline: bool,

    /// Show what would be done without making changes
    #[arg(long, global = true)]
    dry_run: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// One-time global setup: create config, install Claude Code hook
    Setup,

    /// Initialize rune in the current project (creates .claude/rune.toml)
    Init,

    /// Add one or more skills from a registry to this project
    Add {
        /// Skill name(s) to add
        #[arg(required_unless_present = "all")]
        skills: Vec<String>,
        /// Registry to add from
        #[arg(long)]
        from: Option<String>,
        /// Add all skills from the specified registry (requires --from)
        #[arg(long, requires = "from")]
        all: bool,
    },

    /// Remove manifest entries whose registry is not configured
    Prune,

    /// Remove a skill from this project
    Remove {
        /// Skill name to remove
        skill: String,
    },

    /// Check for drift between local skills and registries
    Check {
        /// Check a specific file (used by hook)
        #[arg(long)]
        file: Option<String>,
    },

    /// Sync all skills from registries (pull updates)
    Sync {
        /// Overwrite locally modified skills
        #[arg(long)]
        force: bool,
    },

    /// Push a local skill change back to its registry
    Push {
        /// Skill name to push
        skill: String,
        /// Custom commit message
        #[arg(long, short)]
        message: Option<String>,
    },

    /// List skills and their sync status, or browse a registry
    Ls {
        /// List available skills in a specific registry
        #[arg(long)]
        registry: Option<String>,
    },

    /// Browse available skills in an upstream registry
    Browse {
        /// Registry name (e.g., k-dense, anthropic)
        registry: String,
    },

    /// Import a skill from an upstream registry into your own
    Import {
        /// Skill name with registry: skill@registry
        skill_ref: String,
        /// Target registry to import into
        #[arg(long)]
        to: Option<String>,
    },

    /// Check imported skills for upstream updates
    Upstream {
        /// Suppress output if no updates (for hooks)
        #[arg(long)]
        quiet: bool,
    },

    /// Show diff between imported skill and upstream version
    Diff {
        /// Skill name
        skill: String,
    },

    /// Pull upstream changes for an imported skill
    Update {
        /// Skill name
        skill: String,
        /// Overwrite local modifications
        #[arg(long)]
        force: bool,
    },

    /// Combined status: registries, project skills, upstream updates
    Status,

    /// Audit skill content: compare sizes against upstream, flag regressions
    Audit,

    /// Remove stale registry caches not in config
    Clean,

    /// Diagnose configuration and registry health
    Doctor,

    /// Generate shell completions
    Completions {
        /// Shell (zsh, bash, fish)
        shell: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    color::init();

    if cli.offline {
        registry::set_offline(true);
    }
    if cli.dry_run {
        registry::set_dry_run(true);
    }

    let project_dir = cli.project.unwrap_or_else(|| {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    });

    match cli.command {
        Commands::Setup => setup::setup(),
        Commands::Init => setup::init(&project_dir),
        Commands::Add { skills, from, all } => commands::add_many(&project_dir, &skills, from.as_deref(), all),
        Commands::Prune => commands::prune(&project_dir),
        Commands::Remove { skill } => commands::remove(&project_dir, &skill),
        Commands::Check { file } => {
            let results = commands::check(&project_dir, file.as_deref())?;
            for (name, reg, status) in &results {
                println!("  {name:<24} {:<30} registry: {}",
                    status.colored(), color::cyan(reg));
            }
            if results
                .iter()
                .any(|(_, _, s)| !matches!(s, commands::SkillStatus::Current))
            {
                std::process::exit(1);
            }
            Ok(())
        }
        Commands::Sync { force } => {
            let count = commands::sync(&project_dir, force)?;
            if registry::is_dry_run() {
                eprintln!("Would sync {count} skill(s). (dry run)");
            } else {
                eprintln!("Synced {count} skill(s).");
            }
            Ok(())
        }
        Commands::Push { skill, message } => {
            commands::push(&project_dir, &skill, message.as_deref())
        }
        Commands::Ls { registry } => {
            if let Some(reg_name) = registry {
                commands::ls_registry(&reg_name)
            } else {
                commands::ls(&project_dir)
            }
        }
        Commands::Browse { registry } => commands::browse(&registry),
        Commands::Import { skill_ref, to } => commands::import(&skill_ref, to.as_deref()),
        Commands::Upstream { quiet } => commands::upstream(quiet),
        Commands::Diff { skill } => commands::diff(&skill),
        Commands::Update { skill, force } => commands::update(&skill, force),
        Commands::Status => commands::status(&project_dir),
        Commands::Audit => commands::audit(),
        Commands::Clean => commands::clean(),
        Commands::Doctor => commands::doctor(&project_dir),
        Commands::Completions { shell } => {
            let shell = match shell.as_str() {
                "zsh" => clap_complete::Shell::Zsh,
                "bash" => clap_complete::Shell::Bash,
                "fish" => clap_complete::Shell::Fish,
                other => anyhow::bail!("Unknown shell: {other}. Use zsh, bash, or fish."),
            };
            clap_complete::generate(
                shell,
                &mut Cli::command(),
                "rune",
                &mut std::io::stdout(),
            );
            Ok(())
        }
    }
}
