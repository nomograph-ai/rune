# rune — skill / agent / rule registry sync

Sync reusable markdown artifacts (skills, subagents, rules) from git-based
registries into `.claude/skills/`, `.claude/agents/`, and `.claude/rules/`,
with a content-hash lockfile for drift detection and reproducible installs.

## Concepts

- **Registry**: a git repo hosting one or more of skills, agents, rules.
  Registries live in `~/.config/rune/config.toml` and can be read-only
  (upstream mirrors) or writable (registries you push to).
- **Manifest** (`.claude/rune.toml`): the project's binding of item names
  to registries. Supports `name = "registry"` (track registry's default
  branch), `name = "registry@ref"` (pin to a tag/branch/commit), or the
  table form.
- **Lock** (`.claude/rune.lock`): the synced-state snapshot. Records the
  exact content hash, source registry, and registry-side commit for every
  installed item. Commit it.
- **Pedigree**: YAML frontmatter on imported skills tracking `origin`,
  `imported`, `upstream_commit`, `modified`. Only skills use pedigree.

## Lifecycle

Two flows, same shape.

Editing an existing item:
```
edit .claude/skills/X/**   →  rune push X  →  git commit .claude/rune.lock
                                              rune sync on other machines
```

Publishing a new item to a writable registry:
```
rune add X --from <writable-registry>   # or write .claude/skills/X/ directly
edit .claude/skills/X/**
rune push X                              # first push creates the path in the registry
git commit .claude/rune.lock
```

## Critical rules

1. **Registry push is the authoritative publish.** Every modification to
   `.claude/skills/<name>/**`, `.claude/agents/<name>.md`, or
   `.claude/rules/<name>.md` must be followed by `rune push <name>` in the
   same session. A plain `git commit` in the project does not ship the
   edit to the registry; it orphans the change on one machine.

2. **`.claude/rune.lock` is generated. Don't hand-edit it.** `rune sync`
   regenerates the lock from the current registry state. Manual edits
   are overwritten silently. Use `rune add` / `rune remove` / `rune sync`
   to change lock state.

3. **"Unknown registry" errors mean drift, not data loss.** If a registry
   was renamed in `config.toml` (e.g. from a short form to a qualified
   form) after this project last synced, commands fail with "Unknown
   registry: X". Fix: add `aliases = ["X"]` to the renamed registry entry.
   Projects self-heal on the next sync. Do not rewrite `rune.lock` by
   hand.

4. **`REGISTRY MISSING` on a manifest entry is a bug.** The manifest
   references a registry that's not configured or an item that's not in
   the registry. Either rebind the entry to a configured registry, or
   push the item to a registry that should have it. `rune doctor`
   diagnoses which.

5. **Use `@ref` pinning for reproducibility.** If you need a specific
   version of a skill (tag, branch, or commit), write
   `name = "registry@v1.2.0"` in the manifest. Rune materializes a git
   worktree at that ref and enforces the resolved SHA in the lockfile.
   Tag-move attacks surface as a hard error on next sync. **Only works
   against `source = "git"` registries** — archive-source registries
   (tarball downloads) have no git history to resolve a ref against.
   Rune bails early with a prescriptive error if you try.

## When to use which

- **`rune check`** — at session start. Lists each item's drift status
  (CURRENT / DRIFTED / MISSING / REGISTRY MISSING) with a prescriptive
  `→ run: …` hint per non-current row. Exits non-zero if anything drifted.
- **`rune sync`** — at session start on a fresh machine, or after `git
  pull` brings new manifest/lock entries. Materializes every manifest
  entry; rewrites `rune.lock`; regenerates `AGENTS.md`. Integrity-checks
  pinned entries against the lock.
- **`rune push <name>`** — after editing a local item under `.claude/`.
  Copies the edit into the registry tree, commits, pushes. Auth is
  transient (never written to `.git/config`).
- **`rune add <name> --from <registry> [-t skill|agent|rule]`** — pull a
  fresh item into the project. Writes to the manifest + lock.
- **`rune remove <name> [-t skill|agent|rule]`** — drop an item from the
  manifest and delete its local files. Lockfile entry cleaned on the
  next sync. Type auto-detected from the manifest if omitted.
- **`rune import <skill>@<registry> [--to <writable>]`** — copy a skill
  from an upstream registry into your own writable one, recording
  pedigree for later `rune upstream` / `rune diff` / `rune update` cycles.
- **`rune upstream`** — list imported skills with newer upstream commits.
  `rune diff <skill>` inspects; `rune update <skill>` pulls.
- **`rune doctor`** — run when anything is wrong. Reports config health,
  registry cache state, hook install, lockfile vs manifest vs config
  alignment (including rename drift). Emits prescriptive fixes.
- **`rune ls [--registry <r>]`** — list project items or browse a
  registry's catalog.
- **`rune browse <registry>`** — same as `ls --registry`, plus
  descriptions from each item's frontmatter.

## Gotchas

- **`AGENTS.md` at the project root is generated.** `rune sync` rewrites
  it from the manifest. Don't edit it. Add non-rune content to a
  companion file if needed.
- **`.agent/skills` is a symlink.** Points at `.claude/skills/` so
  non-Claude agents (Cursor, Windsurf, Aider) see the same content. If
  it disappears, `rune sync` restores it.
- **Item names must match `[a-zA-Z0-9_-]+`.** Validation runs at every
  command entry; bad names bail early with the allowed charset named in
  the error.
