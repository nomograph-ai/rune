# rune

Registry manager for AI coding agent skills, agents, and rules. Syncs
markdown files from git-based registries into `.claude/skills/`,
`.claude/agents/`, and `.claude/rules/`. Tracks drift via content-hash
lockfile and pedigree frontmatter.

## Build

```bash
cargo build --release       # binary at target/release/rune
cargo test                  # 22 unit + 40 integration tests
cargo clippy -- -D warnings # lint (also enforced at crate level)
```

CI uses `nomograph/pipeline/rust-cli@v2.4.6` with `cargo-deny` for
advisories and license audit. `audit_allow_failure: false`.

## Architecture

Lib + binary crate. `lib.rs` re-exports all modules for testing.

| Module | Responsibility |
|--------|---------------|
| `main.rs` | CLI (clap derive), command dispatch, `--type` flag parsing |
| `commands/mod.rs` | SkillStatus enum, resolve_registry helpers |
| `commands/check.rs` | Drift checking for all three types |
| `commands/sync.rs` | Sync engine, AGENTS.md generation, agent symlink |
| `commands/crud.rs` | add, push, remove (type-aware) |
| `commands/info.rs` | ls, status, doctor, clean, audit |
| `commands/upstream.rs` | browse, import, upstream, diff, update (skill-only) |
| `registry.rs` | Git/archive registry operations, hashing, file locking |
| `pedigree.rs` | YAML frontmatter parsing/writing, URL slug extraction |
| `manifest.rs` | ArtifactType enum, per-project rune.toml, path overrides |
| `config.rs` | Global config, registry definitions, artifact resolution |
| `setup.rs` | One-time setup: config creation, Claude Code hook install |
| `lockfile.rs` | Per-project rune.lock: content hashes for drift detection |
| `color.rs` | Terminal color support (ANSI, respects TTY) |

## Key types

- `ArtifactType` -- enum: Skill, Agent, Rule. Each has section name,
  default directory, and display name. Agents use `.claude/agents/`
  (Anthropic's directory name; their docs say "subagents").
- `Manifest` -- per-project `.claude/rune.toml`. Sections: `[skills]`,
  `[agents]`, `[rules]`, optional `[paths]` for overrides.
- `Lockfile` -- per-project `.claude/rune.lock`. Matching sections.
- `Pedigree` -- YAML frontmatter tracking origin, import date,
  upstream commit, modification status. Skill-only concept.
- `Config` -- global `~/.config/rune/config.toml`. Registry definitions.
- `SkillStatus` -- drift check result: Current, Drifted, Missing, etc.

## Conventions

- All errors use `anyhow::Result` with `.context()` or `bail!()`.
- `#![deny(warnings, clippy::all)]` enforced in both lib.rs and main.rs.
- Names validated against path traversal via `validate_name()`.
- Registry pulls use file locking (`fs2`) for concurrent safety.
- Registries cached in `~/.cache/rune/registries/`.
- import/upstream/diff/update are skill-only (use pedigree metadata).
- Global state (offline, dry_run) via `AtomicBool` statics in registry.rs.
- Types kept as `SkillEntry` and `LockedSkill` for serde backward compat.

## Testing

62 tests: 11 unit (pedigree), 11 duplicated in bin, 40 integration.

Integration tests cover: manifest/lockfile roundtrips for all types,
typed and legacy registry layouts, artifact path resolution, find_type
collision warnings, path overrides, and all original skill-focused tests.
