![hero](hero.svg)

# rune

[![pipeline](https://gitlab.com/nomograph/rune/badges/main/pipeline.svg)](https://gitlab.com/nomograph/rune/-/pipelines)
[![license](https://img.shields.io/badge/license-MIT-green)](LICENSE)
![built with GitLab](https://img.shields.io/badge/built_with-GitLab-FC6D26?logo=gitlab)

Skill registry manager for AI coding agents. Syncs markdown skill files
from git-based registries into `.claude/skills/`.

## What this is

Skills are inscribed knowledge -- reusable instructions that teach AI
agents how to perform specific workflows. rune keeps them current across
projects via git-based registries.

- **Bidirectional sync** -- pull updates from registries, push changes back
- **Multi-registry** -- public and private registries side by side
- **LLM-native** -- self-installs a Claude Code hook that detects skill drift
- **agentskills.io compatible** -- works with Claude Code, Cursor, Copilot, Gemini CLI

## How it works

```bash
cargo install rune          # install
rune setup                  # one-time: create config, install Claude Code hook
rune init                   # per-project: create .claude/rune.toml manifest
rune add tidy --from runes  # add a skill from a registry
rune sync                   # pull latest from registries
rune check                  # show drift between local and registries
rune push tidy              # push local changes back to registry
rune ls                     # list skills and their status
```

## Config

Global registries in `~/.config/rune/config.toml`:

```toml
[[registry]]
name = "runes"
url = "https://gitlab.com/nomograph/runes.git"

[[registry]]
name = "arcana"
url = "https://gitlab.com/nomograph/arcana.git"
```

Per-project manifest in `.claude/rune.toml`:

```toml
[skills]
tidy = "runes"
research = "runes"
voice = "arcana"
```

## How drift detection works

rune installs a Claude Code PostToolUse hook that fires whenever a
skill file is modified. The hook runs `rune check`, and if drift is
detected, surfaces it to Claude as context. Claude then asks you
whether to push the change upstream. No LLM memory required -- the
hook is deterministic.

When muxr creates a session, a pre_create hook runs `rune sync` so
skills are always current before Claude launches.

---
Built in the Den by Tanuki and Andrew Dunn, April 2026.
