# Changelog

## v0.12.0 (2026-04-23)

Stabilization pass. Five commits cleaning up structural debt that
survived v0.10 and v0.11. Pure quality; no behavior change for
end-users.

### Changed

- **Dead code removed.** Six items flagged `#[allow(dead_code)]` or
  otherwise unused were deleted:
  - `SkillStatus::Unregistered` — never constructed
  - `validate_skill_name` — alias for `validate_name`; call sites
    migrated
  - `Config::resolve_skill` — dead wrapper around `resolve_artifact`
  - `Manifest::skills_dir` — dead helper
  - `Manifest::all_items` — dead iterator
  - `registry::list_skills` — duplicate of `list_artifacts(Skill)`;
    5 call sites migrated

- **`SkillStatus::colored` and `Display` deduplicated.** Both methods
  match-armed their way through the same labels. Extracted a private
  `label()` method; colored wraps it in one color pass, Display writes
  it directly. Half the boilerplate.

- **Directory walker consolidation.** Three near-identical recursive
  walkers (`collect_files` in registry/fs, `collect_files_public`
  test wrapper, `walkdir_simple` in info.rs) merged into one public
  `registry::fs::collect_files`. ~50 lines deleted.

- **`Pedigree::from_skill_or_warn` surfaces bulk-scan errors.** Seven
  call sites across info/sync/upstream used
  `Pedigree::from_skill(path).unwrap_or_default()` to tolerate bad
  frontmatter. The silent-default swallowed IO errors and broke
  `rune update` eligibility for malformed SKILL.md files. The new
  helper logs `warning: could not read pedigree at X: <err>` to
  stderr and returns default. Bulk-scan semantics preserved,
  failures now visible.

- **`commands/info.rs` (636 lines) split into per-command files.**
  Five unrelated commands (ls/ls_registry, doctor, clean, audit,
  status) each got their own file. Imports narrowed per-command.
  Public API unchanged.

### Added

- **Hook script derivation test.** `resources/hook.sh` hardcodes
  paths for `.claude/skills/`, `.claude/agents/`, `.claude/rules/`.
  The source of truth is `ArtifactType::default_dir()`. A new test
  `hook_script_covers_all_artifact_types` asserts, for every variant
  in `ALL_TYPES`, that hook.sh contains the expected path substring.
  Adding a fourth type without updating hook.sh fails CI. Derivation
  via test rather than runtime templating keeps the shellcheck gate
  on the static script intact.

### File-size gate

Two more waivers removed — only `tests/integration.rs` remains
(1162 lines, scheduled for a topic-split in a future release).

### Deferred to v0.13 / later

Three tasks from the original audit remain unlanded. Each is a focused
MR of its own:

- `ValidatedName` new-type threaded through filesystem-touching call
  sites (Bundled Enforcement for the name-validation layer).
- Migrate `Manifest`/`Lockfile` sections to
  `BTreeMap<ArtifactType, _>` with custom serde that preserves the
  `[skills]`/`[agents]`/`[rules]` TOML layout.
- Git-path hardening: consolidate `git_command` / `git_output` /
  `git_command_auth` wrappers; replace `GIT_ASKPASS` shell-script
  trick with an inline credential helper; add local-bare-repo
  integration tests.

Rune now targets ~14/15 on the feedback-audit (up from 10/15 at
v0.9.0): strong Inescapability (deny warnings, cargo-deny, file-size
gate, integrity check), strong Governance (fast CI, file-size gate,
discipline), strong Verifiability (Result-returning parsers, 76 tests
including 6 archive-path tests via httpmock), strong Brevity
(prescriptive errors, per-command files), moderate Derivation (serde +
clap + runtime AGENTS.md + hook-test; no spec-driven codegen chain,
which is appropriate for a CLI scope).

Stabilization pause. v0.13 onward picks up the three deferred items
as discrete MRs when the need arises.

## v0.11.0 (2026-04-23)

Structural pass on the registry module and a ground-up rewrite of the
archive-source fetch path. v0.10 landed the discipline under which
these larger refactors could actually be verified; v0.11 lands the
refactors.

### Changed

- **`src/registry.rs` split into submodules.** The 1197-line god-file
  became a 9-module tree under `src/registry/` (validate, cache, auth,
  git, archive, fs, paths, materialize, plus mod.rs). Each submodule
  is under 250 lines; the file-size CI waiver for registry.rs is
  removed. Public API unchanged — callers still import from
  `crate::registry::X`. One test import path changed:
  `rune::registry::collect_files_public` →
  `rune::registry::fs::collect_files_public`.

- **Archive fetch rewritten in Rust: ureq + tar + flate2.** Replaces
  the curl+tar shell-out. New `ArchiveResponse` enum with three
  unambiguous variants (`Fresh`, `NotModified`, `StaleOk`) replaces
  the prior three-overlapping-signals pattern (curl exit status +
  headers substring match + file existence) for detecting 304. Fixes
  the class of bug that triggered this whole audit:

  Before: curl -f returned 0 on 304 without writing the `-o` file;
  old code's 304 branch only fired on non-zero exit; flow fell through
  to `tar xzf` on a missing file; BSD tar emitted an opaque
  "m: No such file or directory" error.

  After: the HTTP client returns a typed status; `304` becomes a
  first-class enum variant that cannot reach `extract()`.

  The symptomatic short-circuit patch from v0.9.1 (commit db0bfe9) is
  gone; the root cause is gone with it.

### Added

- **Atomic-swap extraction** for archives. Two-phase
  `rename(dest → backup); rename(new → dest); rm backup` guarantees
  concurrent readers never observe a missing directory mid-swap. Prior
  implementation's single-step `rm -rf dest; rename(new, dest)` had a
  window where readers could see no dest at all.

- **Path-traversal guard** on tar entry extraction: archives with
  `..` components or absolute paths after strip-components error out
  instead of escaping the extraction directory. (The `tar` crate
  itself rejects these at the entry layer; our check is
  defense-in-depth.)

- **Archive integration tests** (`tests/archive.rs`, 6 tests via
  httpmock): 200 fresh extract, 304 preserves cache, 404 without
  cache errors, 404 with cache falls back to cached tree, truncated
  gzip fails cleanly, ETag roundtrip (first 200 writes ETag, second
  call sends `If-None-Match`). The 304 bug that started this audit
  would have been caught by any of these in 10 lines.

- **`RUNE_ARCHIVE_URL_<FS_NAME>`** environment override on the
  archive URL resolver. Narrow test hook used by the new integration
  tests; GitHub/GitLab URL templates remain for real registries.

### Dependencies

- Added: `ureq = 2` (blocking HTTP, rustls, no tokio), `tar = 0.4`,
  `flate2 = 1` (miniz_oxide backend, no OpenSSL).
- Added dev-dep: `httpmock = 0.7`.
- Removed runtime dep on `curl` and `tar` CLI tools (still required
  for `git`-source registries).

### Deferred

Five tasks from the originally-planned v0.11 scope were cancelled and
deferred. They're follow-up structural cleanups, not corrections:

- Split `commands/info.rs` + `commands/upstream.rs` into per-command
  files (removes two file-size waivers).
- Consolidate the three directory walkers (`copy_dir_recursive`,
  `walkdir_simple`, `collect_files`) into one.
- `ValidatedName` new-type threaded through filesystem-touching call
  sites (Bundled Enforcement for the name-validation layer).
- Migrate `Manifest` / `Lockfile` sections to
  `BTreeMap<ArtifactType, _>` with custom serde that preserves the
  existing TOML `[skills]` / `[agents]` / `[rules]` layout.
- Git-path hardening: consolidate `git_command` / `git_output` /
  `git_command_auth` wrappers, replace the `GIT_ASKPASS` shell-script
  trick with an inline credential helper, add a local-bare-repo
  integration test.

Each has a task pre-filed under the `rune-smell-free` spec in
synthesist; picking up any of them is a one-MR chunk of work.

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
