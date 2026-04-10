# Changelog

## v0.7.0 (2026-04-10)

### Added

- `rune add` now accepts multiple skill names in one invocation:
  `rune add skill-a skill-b skill-c --from nomograph`
- `rune add --all --from <registry>` adds every skill a registry exposes.
- `rune prune` removes manifest entries whose registry is not configured on
  the current machine. Fixes permanently-broken manifests after migration.
- `rune doctor` now checks manifest entries for unconfigured registries and
  suggests `rune prune` when stale entries are found.
