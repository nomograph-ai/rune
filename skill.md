---
name: rune
description: Skill registry manager. Keeps .claude/skills/ synced with central registries. Run check after modifying skill files, sync at session start.
---

# rune -- skill registry

Manages skill files across projects via git-based registries.
Skills are markdown files in `.claude/skills/` that teach agents
how to perform workflows. rune keeps them current.

## When to invoke

- **After modifying any file in `.claude/skills/`**: run `rune check`
  to detect drift. If drifted, ask the user if they want to push.
- **At session start**: muxr pre_create hook runs `rune sync` automatically.
  You do not need to run sync yourself.
- **When the user asks about skills**: run `rune ls` to show status.
- **When adding a skill**: `rune add <name> --from <registry>`.

## Commands

```
rune check                    # show drift for all skills
rune check --file PATH        # check a specific file (used by hook)
rune sync                     # pull updates from all registries
rune push <skill>             # push local changes back to registry
rune add <skill> --from <reg> # add a skill from a registry
rune ls                       # list skills and sync status
rune init                     # create .claude/rune.toml for this project
rune setup                    # one-time: create config + install hook
```

## After modifying a skill file

The PostToolUse hook fires automatically and shows drift. When you
see drift output, present it to the user:

"I updated <skill>.md. It now differs from the <registry> registry.
Want me to push this change upstream so other projects get it?"

Wait for explicit approval before running `rune push <skill>`.
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

## Configuration

- `~/.config/rune/config.toml` -- registries (name, url, branch)
- `.claude/rune.toml` -- per-project manifest (skill -> registry map)
