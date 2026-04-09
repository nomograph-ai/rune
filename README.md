![hero](hero.svg)

# rune

[![pipeline](https://gitlab.com/nomograph/rune/badges/main/pipeline.svg)](https://gitlab.com/nomograph/rune/-/pipelines)
[![license](https://img.shields.io/badge/license-MIT-green)](LICENSE)
[![built with GitLab](https://img.shields.io/badge/built_with-GitLab-FC6D26?logo=gitlab)](https://gitlab.com/nomograph/rune)

Skill registry manager for AI coding agents. Syncs markdown skill files
from git-based registries into `.claude/skills/`.

## What this is

Skills are inscribed knowledge -- reusable instructions that teach AI
agents how to perform specific workflows. rune keeps them current across
projects via git-based registries.

- **Bidirectional sync** -- pull updates from registries, push changes back
- **Multi-registry** -- public and private registries with separate auth
- **Upstream tracking** -- import from third-party registries, track drift
- **Lockfile** -- reproducible syncs with content-hash drift detection
- **LLM-native** -- self-installs a Claude Code hook that detects skill drift

## Install

Install via [mise](https://mise.jdx.dev). See the
[latest release](https://gitlab.com/nomograph/rune/-/releases) for
the mise config block with current version and URLs.

## Quick start

```bash
rune setup                  # one-time: create config, install Claude Code hook
rune init                   # per-project: create .claude/rune.toml manifest
rune add tidy --from runes  # add a skill from a registry
rune sync                   # pull latest from registries
rune check                  # show drift between local and registries
rune push tidy              # push local changes back to registry
rune status                 # combined summary view
rune audit                  # check for content regressions
```

## Registries

Registries are git repos containing skill directories. Configure them
in `~/.config/rune/config.toml`:

```toml
# Public registry -- anyone can clone
[[registry]]
name = "runes"
url = "https://gitlab.com/nomograph/runes.git"

# Private registry -- requires authentication
[[registry]]
name = "private"
url = "https://gitlab.com/work-namespace/private-skills.git"
token_env = "RUNE_TOKEN_PRIVATE"
```

### Authentication

Registries on different GitLab/GitHub namespaces often need different
credentials. rune resolves tokens per-registry in this order:

1. **`token_env`** -- an env var name in the registry config. rune reads
   the variable at runtime and injects the token into the HTTPS URL.
   No secrets in config files.

2. **`glab auth token`** -- for gitlab.com URLs, rune tries the token
   from the `glab` CLI if installed and authenticated.

3. **`gh auth token`** -- for github.com URLs, rune tries the token
   from the `gh` CLI if installed and authenticated.

4. **No auth** -- falls through to system git credential helpers or
   public access.

#### Setting up a private registry

Create a fine-grained personal access token scoped to the registry
project with `Code: read` (and `write` if you'll `rune push`).

Set the token in your shell environment:

```bash
# In ~/.config/env/secrets.zsh or equivalent
export RUNE_TOKEN_PRIVATE="glpat-xxxxxxxxxxxx"
```

The naming convention is `RUNE_TOKEN_{REGISTRY_NAME}` in uppercase.

#### Multiple GitLab identities

If you have personal and work GitLab accounts, `glab auth token`
returns whichever account is active in glab. This may not match the
namespace your registry lives in. Use `token_env` to be explicit:

```toml
# Personal namespace -- glab auto-detect works
[[registry]]
name = "public-skills"
url = "https://gitlab.com/personal/skills.git"

# Work namespace -- needs explicit token
[[registry]]
name = "work-skills"
url = "https://gitlab.com/work-org/team/skills.git"
token_env = "RUNE_TOKEN_WORK"
```

## Per-project manifest

Each project declares which skills it needs in `.claude/rune.toml`:

```toml
[skills]
tidy = "runes"            # pinned to specific registry
research = {}             # resolved by registry priority
voice = "private"         # from private registry
```

After `rune sync`, a lockfile (`.claude/rune.lock`) records exactly
what was installed -- content hash, registry commit, and sync date.
This enables accurate drift detection:

- **Local newer** -- you edited the skill since last sync
- **Registry newer** -- upstream changed since last sync
- **Diverged** -- both changed

## Upstream imports

Browse and import skills from third-party registries:

```bash
rune browse k-dense                 # list available skills
rune import scanpy@k-dense          # import into your own registry
rune upstream                       # check for upstream updates
rune diff scanpy                    # compare local vs upstream
rune update scanpy                  # pull upstream changes
```

Imported skills carry pedigree metadata tracking origin, upstream
commit, and whether you've modified them locally.

## Drift detection

rune installs a Claude Code PostToolUse hook that fires when a skill
file is modified. The hook runs `rune check` and surfaces drift to
Claude as context, prompting you to push or revert.

If you use [muxr](https://gitlab.com/nomograph/muxr) for session
management, add `rune sync` as a `pre_create` hook so skills are
pulled before each session starts.

## Commands

| Command | Description |
|---------|-------------|
| `rune setup` | One-time: create config, install Claude Code hook |
| `rune init` | Per-project: create .claude/rune.toml manifest |
| `rune add <skill>` | Add a skill from a registry |
| `rune remove <skill>` | Remove a skill from the project |
| `rune sync [--force]` | Pull latest from registries (--force overwrites local edits) |
| `rune check` | Show drift between local and registries |
| `rune push <skill> [-m msg]` | Push local changes back to registry |
| `rune ls` | List skills and their status |
| `rune status` | Combined summary: registries + project + upstream |
| `rune audit` | Check for content regressions across registries |
| `rune browse <registry>` | List available skills in a registry |
| `rune import <skill>@<registry>` | Import from upstream into your registry |
| `rune upstream` | Check imported skills for upstream updates |
| `rune diff <skill>` | Diff imported skill against upstream |
| `rune update <skill>` | Pull upstream changes for an imported skill |
| `rune clean` | Remove stale cache entries |
| `rune doctor` | Diagnose configuration and registry health |
| `rune completions <shell>` | Generate shell completions (zsh, bash, fish) |

### Global flags

| Flag | Description |
|------|-------------|
| `--offline` | Use cached registries, no network |
| `--dry-run` | Show what would change without mutating |
| `--project <dir>` | Target a different project directory |

---
Built in the Den by Tanuki and Andrew Dunn, April 2026.
