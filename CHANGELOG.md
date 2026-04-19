# Changelog

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
