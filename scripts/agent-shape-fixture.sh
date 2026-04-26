#!/usr/bin/env bash
#
# Seed a realistic rune fixture for the jig agent-shape battery.
#
# Produces fixtures/agent-shape-realistic/ populated with:
#   - home/.config/rune/config.toml pointing at three local registries
#   - registries/labs-runes/   (writable git registry, multiple skills + an
#     imported skill carrying pedigree)
#   - registries/upstream-mirror/ (read-only git registry, one skill that
#     advanced past the imported version)
#   - registries/anthropic-mirror/ (read-only git registry)
#   - project/.claude/ with manifest + lockfile + synced skills/agents/rules,
#     including one DRIFTED local edit and one entry pointing at an
#     unconfigured registry to surface a REGISTRY MISSING / drift state
#
# Idempotent: wipes and rebuilds the fixture on every invocation so each
# trial starts from identical state. Uses an isolated $HOME so the user's
# real ~/.config/rune is never touched.
#
# Usage: run by jig during fixture setup. Can also be run manually from
# any cwd; the script computes paths relative to itself.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
FIXTURE_DIR="$REPO_ROOT/fixtures/agent-shape-realistic"

# Hard reset.
rm -rf "$FIXTURE_DIR"
mkdir -p "$FIXTURE_DIR"

# Isolate from the real environment. rune resolves config and cache via
# dirs::home_dir(), which honors $HOME on darwin/linux. Strip RUNE_*
# overrides so this script never accidentally writes to the user's
# real registries.
export HOME="$FIXTURE_DIR/home"
unset XDG_CONFIG_HOME XDG_CACHE_HOME RUNE_PROJECT
mkdir -p "$HOME/.config/rune" "$HOME/.cache/rune"

REGISTRIES_DIR="$FIXTURE_DIR/registries"
PROJECT_DIR="$FIXTURE_DIR/project"
mkdir -p "$REGISTRIES_DIR" "$PROJECT_DIR"

# --- helper: seed a git registry ---
# Layout: skills at registry root (legacy flat layout), agents/ and rules/
# as sibling subdirectories. This matches nomograph/runes and is the
# layout `rune upstream` requires for pedigree-based update detection.
seed_registry() {
  local name="$1"
  local dir="$REGISTRIES_DIR/$name"
  mkdir -p "$dir/agents" "$dir/rules"
  (
    cd "$dir"
    git init -q
    git config user.email "seed@fixture.local"
    git config user.name "Fixture Seed"
  )
}

write_skill() {
  local registry="$1" skill="$2" body="$3" extra_frontmatter="${4:-}"
  local skill_dir="$REGISTRIES_DIR/$registry/$skill"
  mkdir -p "$skill_dir"
  {
    echo "---"
    echo "name: $skill"
    echo "description: $body"
    if [[ -n "$extra_frontmatter" ]]; then
      printf '%s\n' "$extra_frontmatter"
    fi
    echo "---"
    echo "# $skill"
    echo
    echo "$body"
  } > "$skill_dir/SKILL.md"
}

write_agent() {
  local registry="$1" agent="$2" body="$3"
  {
    echo "---"
    echo "name: $agent"
    echo "description: $body"
    echo "---"
    echo "# $agent"
    echo
    echo "$body"
  } > "$REGISTRIES_DIR/$registry/agents/$agent.md"
}

write_rule() {
  local registry="$1" rule="$2" body="$3"
  {
    echo "---"
    echo "name: $rule"
    echo "description: $body"
    echo "---"
    echo "$body"
  } > "$REGISTRIES_DIR/$registry/rules/$rule.md"
}

commit_registry() {
  local name="$1" message="$2"
  (
    cd "$REGISTRIES_DIR/$name"
    git add -A
    git commit -q -m "$message"
  )
}

# --- registry: upstream-mirror (read-only, holds the upstream v2 of an
#     imported skill so `rune upstream` has something to report) ---
seed_registry upstream-mirror
write_skill upstream-mirror researcher-skill "research workflow for literature review"
commit_registry upstream-mirror "v1: researcher-skill"
# Advance upstream so the imported v1 in labs-runes is behind.
write_skill upstream-mirror researcher-skill \
  "research workflow for literature review (revised: adds PubMed step)"
commit_registry upstream-mirror "v2: add PubMed step"

# --- registry: anthropic-mirror (read-only, second upstream) ---
seed_registry anthropic-mirror
write_skill anthropic-mirror feedback-audit "audit project feedback signal quality"
write_skill anthropic-mirror documentation "write technical documentation"
commit_registry anthropic-mirror "seed anthropic-mirror"

# --- registry: labs-runes (writable, project's home registry) ---
seed_registry labs-runes
write_skill labs-runes no-emdash "never use em dashes in generated text"
write_skill labs-runes feedback-audit "audit project feedback signal quality"
write_agent labs-runes researcher "research subagent for literature review"
write_rule labs-runes commit-message-style "use imperative mood in commit messages"

# Seed an imported skill: pedigree references upstream-mirror at an old
# commit hash, so `rune upstream` flags it as out-of-date against the v2
# we just committed above.
mkdir -p "$REGISTRIES_DIR/labs-runes/researcher-skill"
{
  echo "---"
  echo "name: researcher-skill"
  echo "description: research workflow for literature review"
  echo "origin: upstream-mirror"
  echo "origin_path: researcher-skill"
  echo "imported: 2026-03-15"
  echo "upstream_commit: 0000001"
  echo "modified: false"
  echo "---"
  echo "# researcher-skill"
  echo
  echo "research workflow for literature review"
} > "$REGISTRIES_DIR/labs-runes/researcher-skill/SKILL.md"
commit_registry labs-runes "seed labs-runes"

# --- rune config pointing at the three registries ---
cat > "$HOME/.config/rune/config.toml" <<EOF
[[registry]]
name = "labs-runes"
url = "$REGISTRIES_DIR/labs-runes"
git_email = "seed@fixture.local"
git_name = "Fixture Seed"

[[registry]]
name = "upstream-mirror"
url = "$REGISTRIES_DIR/upstream-mirror"
readonly = true

[[registry]]
name = "anthropic-mirror"
url = "$REGISTRIES_DIR/anthropic-mirror"
readonly = true
EOF

# --- project: init, sync items in via rune itself so the lockfile is
#     real and matches what `rune check` expects ---
cd "$PROJECT_DIR"
git init -q
git config user.email "seed@fixture.local"
git config user.name "Fixture Seed"

rune init > /dev/null
rune add no-emdash --from labs-runes > /dev/null
rune add feedback-audit --from labs-runes > /dev/null
rune add researcher --from labs-runes -t agent > /dev/null
rune add commit-message-style --from labs-runes -t rule > /dev/null
rune add researcher-skill --from labs-runes > /dev/null

# Induce a DRIFTED state: edit the no-emdash skill locally so `rune check`
# reports drift and the agent has something to push.
cat >> "$PROJECT_DIR/.claude/skills/no-emdash/SKILL.md" <<'EOF'

## Local addition

Project also forbids en-dashes in headings.
EOF

# Induce a REGISTRY MISSING state: insert a manifest entry whose
# registry is not in the rune config. `rune doctor` should surface this
# with a prescriptive fix. Inject under existing [agents] section so we
# don't create a duplicate header.
manifest="$PROJECT_DIR/.claude/rune.toml"
if ! grep -q "ghost-helper" "$manifest"; then
  awk '
    {print}
    /^\[agents\]/ {print "ghost-helper = \"deprecated-internal\""}
  ' "$manifest" > "$manifest.tmp"
  mv "$manifest.tmp" "$manifest"
fi

# Commit the project as it stands so the agent sees a real git history
# (some tasks may want to git diff or git log).
git add -A
git commit -q -m "seed fixture project state" > /dev/null

cd "$FIXTURE_DIR"
echo "fixture seeded at $FIXTURE_DIR"
echo "  HOME=$HOME"
echo "  project=$PROJECT_DIR"
echo "  registries: labs-runes (writable), upstream-mirror (ro), anthropic-mirror (ro)"
