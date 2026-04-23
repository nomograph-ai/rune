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
use manifest::ArtifactType;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "rune",
    version,
    about = "Registry manager for AI coding agent skills, agents, and rules",
    long_about = "rune syncs markdown files from git-based registries into .claude/skills/, \
        .claude/agents/, and .claude/rules/.\n\n\
        Skills are inscribed knowledge -- reusable instructions that teach AI agents \
        how to perform specific workflows. Agents are subagent definitions. Rules are \
        conditional instructions. rune keeps them current across projects.\n\n\
        Example:\n  \
        rune setup                           # one-time: create config, install hook\n  \
        rune init                            # per-project: create manifest\n  \
        rune add tidy --from runes           # add a skill from a registry\n  \
        rune add researcher -t agent         # add an agent\n  \
        rune add no-emdash -t rule           # add a rule\n  \
        rune sync                            # pull latest from registries\n  \
        rune check                           # show drift between local and registries\n  \
        rune push tidy                       # push local changes back to registry\n  \
        rune browse runes                    # discover available items\n  \
        rune import scanpy@k-dense           # import from upstream\n  \
        rune status                          # combined summary view\n  \
        rune ls                              # list all items and their status"
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

    /// Add one or more items from a registry to this project
    Add {
        /// Item name(s) to add
        #[arg(required_unless_present = "all")]
        names: Vec<String>,
        /// Registry to add from
        #[arg(long)]
        from: Option<String>,
        /// Add all items of the given type from the specified registry (requires --from)
        #[arg(long, requires = "from")]
        all: bool,
        /// Item type: skill (default), agent, or rule
        #[arg(short = 't', long = "type", default_value = "skill")]
        item_type: String,
    },

    /// Remove manifest entries whose registry is not configured
    Prune,

    /// Remove a skill, agent, or rule from this project
    Remove {
        /// Item name to remove
        name: String,
        /// Item type (auto-detected from manifest if omitted)
        #[arg(short = 't', long = "type")]
        item_type: Option<String>,
    },

    /// Check for drift between local items and registries
    Check {
        /// Check a specific file (used by hook)
        #[arg(long)]
        file: Option<String>,
    },

    /// Sync all skills, agents, and rules from registries (pull updates)
    Sync {
        /// Overwrite locally modified items
        #[arg(long)]
        force: bool,
    },

    /// Push a local change back to its registry
    Push {
        /// Item name to push
        name: String,
        /// Custom commit message
        #[arg(long, short)]
        message: Option<String>,
        /// Item type (auto-detected from manifest if omitted)
        #[arg(short = 't', long = "type")]
        item_type: Option<String>,
    },

    /// List items and their sync status, or browse a registry
    Ls {
        /// List available items in a specific registry
        #[arg(long)]
        registry: Option<String>,
    },

    /// Browse available items in an upstream registry
    Browse {
        /// Registry name (e.g., runes, anthropic)
        registry: String,
        /// Filter by type: skill, agent, or rule
        #[arg(short = 't', long = "type")]
        item_type: Option<String>,
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

    /// Combined status: registries, project items, upstream updates
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

/// Parse a --type flag string into an ArtifactType.
fn parse_type_flag(s: &str) -> Result<ArtifactType> {
    ArtifactType::parse(s)
        .ok_or_else(|| anyhow::anyhow!("Unknown type: {s}. Use skill, agent, or rule."))
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

    let project_dir = cli
        .project
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    match cli.command {
        Commands::Setup => setup::setup(),
        Commands::Init => setup::init(&project_dir),
        Commands::Add {
            names,
            from,
            all,
            item_type,
        } => {
            let at = parse_type_flag(&item_type)?;
            commands::add_many(&project_dir, &names, from.as_deref(), all, at)
        }
        Commands::Prune => commands::prune(&project_dir),
        Commands::Remove { name, item_type } => {
            let at = item_type.as_deref().map(parse_type_flag).transpose()?;
            commands::remove(&project_dir, &name, at)
        }
        Commands::Check { file } => {
            let results = commands::check(&project_dir, file.as_deref())?;
            for (name, reg, status) in &results {
                println!(
                    "  {name:<24} {:<30} registry: {}",
                    status.colored(),
                    color::cyan(reg)
                );
                if let Some(hint) = status.hint(name) {
                    println!("  {:<24} {}", "", color::dim(&hint));
                }
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
                eprintln!("Would sync {count} item(s). (dry run)");
            } else {
                eprintln!("Synced {count} item(s).");
            }
            Ok(())
        }
        Commands::Push {
            name,
            message,
            item_type,
        } => {
            let at = item_type.as_deref().map(parse_type_flag).transpose()?;
            commands::push(&project_dir, &name, message.as_deref(), at)
        }
        Commands::Ls { registry } => {
            if let Some(reg_name) = registry {
                commands::ls_registry(&reg_name)
            } else {
                commands::ls(&project_dir)
            }
        }
        Commands::Browse {
            registry,
            item_type,
        } => {
            let at = item_type.as_deref().map(parse_type_flag).transpose()?;
            commands::browse(&registry, at)
        }
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
            clap_complete::generate(shell, &mut Cli::command(), "rune", &mut std::io::stdout());
            Ok(())
        }
    }
}
