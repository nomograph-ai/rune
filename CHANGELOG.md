# Changelog

## v0.10.0 (2026-04-23)

Discipline pass. Eight commits that tighten rune's LLM-correctness
signal (Bundled Enforcement, Prescriptive Failure, Vacuity Detection)
without restructuring the codebase. v0.11.0 will do the structural
splits; shipping discipline first gives the splits a stronger test
suite to land under.

### Fixed

- `Config::resolve_artifact` now uses `reg.fs_name()` for the cache
  path lookup. Previously, registries whose display name contained `/`
  (e.g. `andunn/arcana`) silently returned `None` from resolve because
  the cache was stored at `andunn--arcana/` but the lookup joined on
  the raw name. v0.8.1 fixed this everywhere else; this site was missed.
  Regression test included.

### Changed

- `Registry.source` is now an enum `SourceKind { Git, Archive }` instead
  of a free-form string. Existing `source = "git"` and `source = "archive"`
  TOML values continue to parse (serde lowercases on (de)serialize).
  Typos are now compile errors, not silent misroutes.

- Parsers return `Result` instead of swallowing errors into defaults:
  - `registry::skill_hash(path) -> Result<String>` (was `Option<String>`).
    Symlink / IO failure / not-a-regular-path become hard errors instead
    of empty-string hashes. This closes a drift-detection hole where a
    corrupt skill file would hash to `""` and compare equal to another
    corrupt file (or a lockfile default).
  - `Manifest::try_load -> Result<Option<Self>>` (was `Option<Self>`).
    Missing file is still `Ok(None)`; malformed TOML now errors with a
    path-qualified message instead of masquerading as a pre-init project.
  - `Lockfile::load` callers switched from `.unwrap_or_default()` to
    `?`. Missing lockfile still defaults cleanly; malformed lockfile
    surfaces instead of silently erasing itself on next write.

- Error messages rewritten with next-action hints (Prescriptive Failure):
  `rune add <unknown>` now names the `rune browse` command; `rune push`
  of a skill not in the manifest names the `rune add` command with the
  right flags; `validate_name` explains the allowed character class
  (`[a-zA-Z0-9_-]+`) instead of enumerating disallowed characters.

- `SkillStatus::hint(name)` adds a dim `→ run: rune push X` /
  `→ run: rune sync` line under each non-Current line in `rune check`
  output. Users no longer need to remember which command applies to
  which drift direction.

### Added

- **Integrity check on pinned skills.** When a skill is pinned to
  `@v1.2.0` (tag, branch, or commit) and the lockfile already has a
  recorded `registry_commit` for it, the next `rune sync` resolves
  the pin again and compares against the lockfile. If the resolved
  SHA has moved (force-pushed tag, rewritten history), sync bails
  with a prescriptive error naming both SHAs and the two recovery
  paths. Fixes the npm-integrity class of threat for pinned entries.

- **File size CI gate.** Any `.rs` file over 500 lines fails the
  `file-size-gate` job in the test stage. Existing offenders (four
  files totaling ~3400 lines) are waived in `.file-size-waiver`
  pending the v0.11 structural split. New files that regress get
  blocked at CI.

- **Hook script extracted** to `resources/hook.sh` and baked in via
  `include_str!`. Enables shellcheck lint coverage in CI. No runtime
  change — `rune setup` still writes a self-contained script.

- **Shellcheck CI gate** covering `resources/hook.sh` (bash) and
  `scripts/check-file-sizes.sh` (POSIX sh).

- `registry::parse_cache_metadata_name(&str) -> &str` extracted from
  an inline closure in `rune clean`. The prior test duplicated that
  closure, so it verified test code not production code. The new test
  calls the same function users do, with added coverage for fs_name
  (`--`-substituted) registry names and negative cases.

## v0.9.0 (2026-04-21)

### Added

- Skill versioning via `@version` suffix in `rune.toml`. Any git ref is
  valid — tag, branch, or commit hash. Supports both shorthand and table
  forms:

  ```toml
  voice = "andunn/arcana"                # track main (unchanged)
  voice = "andunn/arcana@v1.2.0"         # pin to tag
  voice = "andunn/arcana@abc1234"        # pin to commit
  voice = { registry = "andunn/arcana", version = "v1.2.0" }
  ```

  For pinned skills, rune materializes a cached git worktree at the
  requested ref under `<cache_dir>/worktrees/<registry>--<ref>/` and
  reads the skill from there. One worktree per (registry, ref) pair
  covers all skills from that registry pinned to the same version.
  The lockfile records the resolved commit SHA.

  Only git-type registries support versioning. Archive-type registries
  error clearly when a pinned version is requested.

### Changed

- `SkillEntry` serde: serializes with version as `"registry@version"`
  shorthand when both are set, `"registry"` when unversioned. Round-trip
  preserves both forms.

## v0.8.1 (2026-04-21)

### Fixed

- Registry names containing `/` (e.g. `andunn/arcana`, `andrewdunndev/arcana`)
  now work. Previously the name was used unsanitized in filesystem paths,
  causing lock-file creation and cache-directory construction to fail with
  "No such file or directory." Added `Registry::fs_name()` that replaces
  `/` with `--` for filesystem operations; display name remains unchanged.
  Path-shaped registry names are the recommended convention going forward
  because they are self-describing in project `rune.toml` files without
  cross-referencing the global config.

## v0.8.0 (2026-04-18)

### Added

- Multi-type support: rune now manages skills, agents, and rules, with
  matching `.claude/skills/`, `.claude/agents/`, and `.claude/rules/`
  directories. New `ArtifactType` drives typed sections in manifest and
  lockfile, and typed subdirectories in registries (with legacy
  skill-at-root fallback).
- `-t` / `--type` flag on `add`, `remove`, `push`, `browse` to select
  the item type. `add` defaults to `skill`; `remove` and `push`
  auto-detect from the manifest when omitted.
- Per-project `[paths]` override in `.claude/rune.toml` for targeting
  other agent tools (Cursor, Windsurf, Pi, OpenCode).
- `AGENTS.md` generator emits a separate `<agent>` section distinct
  from `<skill>` tags (agentskills.io interop for non-Claude tools).

### Changed

- Manifest uses `[agents]` to match the `.claude/agents/` filesystem
  directory. Anthropic docs call these "subagents"; we match the
  directory name.
- `commands/` merged into `skills/` in Claude Code v2.1.3 (Jan 2026),
  so rune does not manage a separate `commands` type.

### Notes

- `import`, `upstream`, `diff`, and `update` remain skill-only (they
  use pedigree metadata, a skill-specific concept).
- 62 tests (22 unit + 40 integration) cover multi-type roundtrips,
  path overrides, legacy fallback, and find_type collision warnings.

## v0.7.0 (2026-04-10)

### Added

- `rune add` now accepts multiple skill names in one invocation:
  `rune add skill-a skill-b skill-c --from nomograph`
- `rune add --all --from <registry>` adds every skill a registry exposes.
- `rune prune` removes manifest entries whose registry is not configured on
  the current machine. Fixes permanently-broken manifests after migration.
- `rune doctor` now checks manifest entries for unconfigured registries and
  suggests `rune prune` when stale entries are found.
