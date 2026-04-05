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
    eprintln!("Next: edit {} to add registries, then `rune init` in a project.", Config::path()?.display());
    Ok(())
}

/// Create default config if it doesn't exist.
fn setup_config() -> Result<()> {
    let path = Config::path()?;
    if path.exists() {
        eprintln!("Config already exists at {}", path.display());
        return Ok(());
    }

    let config = Config {
        registry: vec![],
    };
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
        let content = std::fs::read_to_string(&settings_path)?;
        serde_json::from_str(&content)?
    } else {
        serde_json::json!({})
    };

    // Check if hook already installed
    let hook_command = hook_path.to_string_lossy().to_string();
    if let Some(hooks) = settings.get("hooks")
        && let Some(post) = hooks.get("PostToolUse")
    {
        let already = post.as_array().map(|arr| {
            arr.iter().any(|group| {
                group.get("hooks").and_then(|h| h.as_array()).map(|hooks| {
                    hooks.iter().any(|h| {
                        h.get("command")
                            .and_then(|c| c.as_str())
                            .map(|c| c.contains("rune"))
                            .unwrap_or(false)
                    })
                }).unwrap_or(false)
            })
        }).unwrap_or(false);

        if already {
            eprintln!("Hook already installed in settings.json");
            return Ok(());
        }
    }

    // Build the hook entry
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
        .context("settings.json is not an object")?
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));

    let post_tool_use = hooks
        .as_object_mut()
        .context("hooks is not an object")?
        .entry("PostToolUse")
        .or_insert_with(|| serde_json::json!([]));

    post_tool_use
        .as_array_mut()
        .context("PostToolUse is not an array")?
        .push(hook_entry);

    // Write back
    let content = serde_json::to_string_pretty(&settings)?;
    std::fs::write(&settings_path, content)?;
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
    eprintln!("Add skills with: rune add <skill> --from <registry>");
    Ok(())
}

/// The hook script installed to ~/.config/rune/hook.sh.
/// Reads Claude's PostToolUse JSON from stdin, checks if the file
/// is a skill file, and runs rune check if so.
const HOOK_SCRIPT: &str = r#"#!/bin/bash
set -e

INPUT=$(cat)
FILE_PATH=$(echo "$INPUT" | jq -r '.tool_input.file_path // empty')

# Only act on .claude/skills/*.md files
if [[ "$FILE_PATH" != *".claude/skills/"* ]] || [[ "$FILE_PATH" != *.md ]]; then
    exit 0
fi

# Find project root (walk up to find .claude/rune.toml)
DIR=$(dirname "$FILE_PATH")
while [[ "$DIR" != "/" ]]; do
    if [[ -f "$DIR/.claude/rune.toml" ]] || [[ -f "$DIR/rune.toml" ]]; then
        break
    fi
    # Go up from .claude/skills/ to project root
    DIR=$(dirname "$DIR")
done

if [[ ! -f "$DIR/.claude/rune.toml" ]]; then
    exit 0
fi

# Run rune check on the specific file
OUTPUT=$(rune check --file "$FILE_PATH" --project "$DIR" 2>&1) || true

if [[ -n "$OUTPUT" ]] && [[ "$OUTPUT" == *"DRIFTED"* ]]; then
    # Surface to Claude via additionalContext
    ESCAPED=$(echo "$OUTPUT" | jq -Rs .)
    printf '{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":%s}}' "$ESCAPED"
fi

exit 0
"#;
