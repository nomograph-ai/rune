BINARY := rune

.PHONY: help build install clean test lint check agent-skills-symlink check-symlinks dev skill

# Create the .agent/skills compat symlink fresh on every build.
# Pointing .agent/skills at .claude/skills (relative) lets agent
# harnesses that read from .agent/ (per the AGENTS.md ecosystem
# convention) see the same skills rune exposes via .claude/.
# The symlink is intentionally untracked in git: tools that rewrite
# symlinks (e.g. some rune sync versions) cannot break a file that
# was never committed. Each clone or build regenerates it relative,
# which is the only form that survives both the local checkout and
# the CI clone path.
help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*##' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*##"}; {printf "  %-15s %s\n", $$1, $$2}'

agent-skills-symlink:
	@mkdir -p .agent
	@if [ ! -L .agent/skills ] || [ "$$(readlink .agent/skills)" != "../.claude/skills" ]; then \
		rm -rf .agent/skills; \
		ln -s ../.claude/skills .agent/skills; \
	fi

build: agent-skills-symlink ## Build release binary
	cargo build --release
	cp target/release/$(BINARY) $(BINARY)

install: agent-skills-symlink ## Install binary via cargo
	cargo install --path .

clean: ## Remove all build artifacts (cargo target/ + dist/ + binary)
	cargo clean
	rm -f $(BINARY)
	rm -rf dist/ 2>/dev/null || true

test: build ## Run all tests
	cargo test

lint: ## Run clippy
	cargo clippy -- -D warnings

check: build ## Quick smoke-check (binary --help)
	./$(BINARY) help > /dev/null

dev: build ## Build and run help
	./$(BINARY) help

skill: build ## Build and run skill subcommand
	./$(BINARY) skill

# CI gate: fail if any committed symlink resolves to an absolute path.
# Backstop against the .agent/skills recurrence class -- even if a
# tool rewrites a symlink locally, a commit that includes an
# absolute target fails this check before reaching expensive CI
# jobs. Run via `make check-symlinks` locally or in pipeline.
check-symlinks: ## Check that no committed symlinks use absolute paths
	@bash scripts/check-symlinks.sh

.DEFAULT_GOAL := help
