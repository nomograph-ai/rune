# rune

Skill registry manager for AI coding agents. Syncs markdown skill files
from git-based registries into `.claude/skills/`. Tracks drift via
content-hash lockfile and pedigree frontmatter.

## Build

```bash
cargo build --release       # binary at target/release/rune
cargo test --lib            # unit tests (11 in pedigree.rs)
cargo clippy -- -D warnings # lint (also enforced at crate level)
```

CI uses `nomograph/pipeline/rust-cli@v2.4.6` with `cargo-deny` for
advisories and license audit. `audit_allow_failure: false`.

## Architecture

Lib + binary crate. `lib.rs` re-exports all modules for testing.

| Module | Lines | Responsibility |
|--------|-------|---------------|
| `main.rs` | ~255 | CLI (clap derive), command dispatch |
| `commands.rs` | ~1453 | All command implementations |
| `registry.rs` | ~700 | Git/archive registry operations, hashing, file locking |
| `pedigree.rs` | ~380 | YAML frontmatter parsing/writing, URL slug extraction |
| `manifest.rs` | ~100 | Per-project rune.toml: skill declarations |
| `config.rs` | ~130 | Global ~/.config/rune/config.toml: registry definitions |
| `setup.rs` | ~200 | One-time setup: config creation, Claude Code hook install |
| `lockfile.rs` | ~60 | Per-project rune.lock: content hashes for drift detection |
| `color.rs` | ~60 | Terminal color support (ANSI, respects TTY) |

## Key types

- `Manifest` -- per-project `.claude/rune.toml`. Declares which skills
  this project uses, optionally pinned to a registry.
- `Lockfile` -- per-project `.claude/rune.lock`. Records content hash,
  registry commit, and sync timestamp per skill.
- `Pedigree` -- YAML frontmatter metadata tracking origin, import date,
  upstream commit, and modification status.
- `Config` -- global `~/.config/rune/config.toml`. Registry definitions
  with URL, branch, auth, and source mode (git/archive).
- `Registry` -- a named registry: URL, branch, token_env, readonly flag,
  git identity overrides.
- `SkillStatus` -- drift check result: Current, Drifted (with direction),
  Missing, Unregistered, RegistryMissing.

## Conventions

- All errors use `anyhow::Result` with `.context()` or `bail!()`.
  Errors should be prescriptive: "Run `rune setup` to create one."
- `#![deny(warnings, clippy::all)]` enforced in both lib.rs and main.rs.
- Skill names are validated against path traversal via `validate_skill_name()`.
- Registry pulls use file locking (`fs2`) for concurrent safety.
- Registries are cached in `~/.cache/rune/registries/`.
- Lockfile header says "Do not edit manually" (derived output).
- Shell completions derived from clap via `clap_complete`.
- Global state (offline, dry_run) via `AtomicBool` statics in registry.rs.

## Testing

Existing tests: 11 in `pedigree.rs` (frontmatter parsing, URL slugs,
date formatting). Run with `cargo test --lib`.

Pure functions suitable for additional tests: `validate_skill_name`,
`Manifest` serde roundtrip, `Lockfile` serde roundtrip,
`SkillStatus::colored`/`Display`, `Config` deserialization,
`parse_skill_ref`.
