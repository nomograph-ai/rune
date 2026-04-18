---
name: rune
description: Registry manager for skills, agents, and rules. Keeps .claude/skills/, .claude/agents/, and .claude/rules/ synced with central registries. Run check after modifying files, sync at session start.
---

# rune -- registry manager

Manages skills, agents, and rules across projects via git-based registries.
Skills are markdown files in `.claude/skills/` that teach agents
how to perform workflows. Agents are subagent definitions in
`.claude/agents/`. Rules are conditional instructions in `.claude/rules/`.
rune keeps them current.

## When to invoke

- **After modifying any file in `.claude/skills/`, `.claude/agents/`, or
  `.claude/rules/`**: run `rune check` to detect drift. If drifted, ask
  the user if they want to push.
- **At session start**: muxr pre_create hook runs `rune sync` automatically.
  You do not need to run sync yourself.
- **When the user asks about skills**: run `rune ls` to show status.
- **When adding an item**: `rune add <name> --from <registry> [-t type]`.

## Commands

```
rune check                    # show drift for all items
rune check --file PATH        # check a specific file (used by hook)
rune sync                     # pull updates from all registries
rune push <name>              # push local changes back to registry
rune add <name> --from <reg>  # add a skill from a registry
rune add <name> -t agent      # add an agent
rune add <name> -t rule       # add a rule
rune ls                       # list items and sync status
rune init                     # create .claude/rune.toml for this project
rune setup                    # one-time: create config + install hook
```

## After modifying a skill file

The PostToolUse hook fires automatically and shows drift. When you
see drift output, present it to the user:

"I updated <name>.md. It now differs from the <registry> registry.
Want me to push this change upstream so other projects get it?"

Wait for explicit approval before running `rune push <name>`.
This is an external write -- the same approval gate as git push.

## Output format

```
  tidy                     CURRENT                        registry: public
  voice                    DRIFTED  local is newer        registry: private
  mirror                   MISSING                        registry: public
```

- **CURRENT**: local matches registry
- **DRIFTED**: local and registry differ (direction shown)
- **MISSING**: in manifest but not on disk (run sync)
- **REGISTRY MISSING**: in manifest but not in registry

## Organizing an existing .claude/ directory with rune

If the user has skills, agents, or rules that are not yet managed by
rune, help them migrate:

1. Check if rune is set up: look for `.claude/rune.toml`. If missing,
   run `rune init`.
2. For each file in `.claude/skills/`, `.claude/agents/`, `.claude/rules/`:
   - If it came from a registry, add it to the manifest with
     `rune add <name> --from <registry> [-t type]`.
   - If it was written locally and should be shared, suggest creating
     a registry (a git repo with `skills/`, `agents/`, `rules/` dirs)
     and using `rune push <name>` after adding it to the manifest.
3. Files in `.claude/commands/` should be moved to `.claude/skills/`.
   Claude Code merged commands into skills in v2.1.3. Move the file
   to a skill directory: `mkdir -p .claude/skills/<name>` then
   `mv .claude/commands/<name>.md .claude/skills/<name>/SKILL.md`.
4. Run `rune sync` to verify everything resolves.

## Configuration

- `~/.config/rune/config.toml` -- registries (name, url, branch)
- `.claude/rune.toml` -- per-project manifest (skills, agents, rules)
