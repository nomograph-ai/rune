use anyhow::{Context, Result};
use std::path::Path;

use crate::config::Config;
use crate::manifest::Manifest;

/// One-time global setup: create config, install Claude Code hook, install skill file.
pub fn setup() -> Result<()> {
    setup_config()?;
    setup_hook()?;
    eprintln!("rune setup complete.");
    eprintln!("  config:  {}", Config::path()?.display());
    eprintln!("  hook:    installed in ~/.claude/settings.json");
    eprintln!();
    eprintln!(
        "Next: edit {} to add registries, then `rune init` in a project.",
        Config::path()?.display()
    );
    Ok(())
}

/// Create default config if it doesn't exist.
fn setup_config() -> Result<()> {
    let path = Config::path()?;
    if path.exists() {
        eprintln!("Config already exists at {}", path.display());
        return Ok(());
    }

    let config = Config { registry: vec![] };
    config.save()?;
    eprintln!("Created {}", path.display());
    Ok(())
}

/// Install the PostToolUse hook into Claude Code settings.
fn setup_hook() -> Result<()> {
    // Install hook script
    let config_dir = Config::config_dir()?;
    let hook_path = config_dir.join("hook.sh");
    std::fs::create_dir_all(&config_dir)?;
    std::fs::write(&hook_path, HOOK_SCRIPT)?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755))?;
    }

    // Read and update Claude settings.json
    let home = dirs::home_dir().context("No home directory")?;
    let settings_path = home.join(".claude").join("settings.json");

    let mut settings: serde_json::Value = if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path)
            .with_context(|| format!("Failed to read {}", settings_path.display()))?;
        serde_json::from_str(&content).with_context(|| {
            format!(
                "Failed to parse {}. Fix the JSON manually or delete the file and re-run setup.",
                settings_path.display()
            )
        })?
    } else {
        std::fs::create_dir_all(settings_path.parent().unwrap())?;
        serde_json::json!({})
    };

    // Check if hook already installed
    if let Some(hooks) = settings.get("hooks")
        && let Some(post) = hooks.get("PostToolUse")
    {
        let already = post
            .as_array()
            .map(|arr| {
                arr.iter().any(|group| {
                    group
                        .get("hooks")
                        .and_then(|h| h.as_array())
                        .map(|hooks| {
                            hooks.iter().any(|h| {
                                h.get("command")
                                    .and_then(|c| c.as_str())
                                    .map(|c| c.contains("rune"))
                                    .unwrap_or(false)
                            })
                        })
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);

        if already {
            eprintln!("Hook already installed in settings.json");
            return Ok(());
        }
    }

    // Backup existing settings before modifying
    if settings_path.exists() {
        let backup = settings_path.with_extension("json.bak");
        std::fs::copy(&settings_path, &backup).with_context(|| "Failed to backup settings.json")?;
    }

    // Build the hook entry
    let hook_command = hook_path.to_string_lossy().to_string();
    let hook_entry = serde_json::json!({
        "matcher": "Edit|Write",
        "hooks": [{
            "type": "command",
            "command": hook_command,
            "timeout": 30
        }]
    });

    // Insert into settings
    let hooks = settings
        .as_object_mut()
        .context("settings.json root is not a JSON object")?
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));

    let post_tool_use = hooks
        .as_object_mut()
        .context("settings.json hooks field is not a JSON object")?
        .entry("PostToolUse")
        .or_insert_with(|| serde_json::json!([]));

    post_tool_use
        .as_array_mut()
        .context("settings.json PostToolUse field is not a JSON array")?
        .push(hook_entry);

    // Write back (with trailing newline for POSIX compliance)
    let content = serde_json::to_string_pretty(&settings)?;
    std::fs::write(&settings_path, format!("{content}\n"))
        .with_context(|| format!("Failed to write {}", settings_path.display()))?;
    eprintln!("Installed hook in {}", settings_path.display());

    Ok(())
}

/// Per-project init: create rune.toml manifest.
pub fn init(project_dir: &Path) -> Result<()> {
    let path = Manifest::path(project_dir);
    if path.exists() {
        eprintln!("Manifest already exists at {}", path.display());
        return Ok(());
    }

    let manifest = Manifest::default();
    manifest.save(project_dir)?;
    eprintln!("Created {}", path.display());
    eprintln!("Add items with: rune add <name> --from <registry> [-t skill|agent|rule]");
    Ok(())
}

/// The hook script installed to ~/.config/rune/hook.sh.
///
/// Lives in resources/hook.sh so it can be shellcheck'd in CI and read
/// as a real file during development. include_str! bakes it into the
/// binary at compile time, so `rune setup` still writes a self-contained
/// hook without needing the source tree.
const HOOK_SCRIPT: &str = include_str!("../resources/hook.sh");
