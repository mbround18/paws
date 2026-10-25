.PHONY: build release fixtures clean complexity quality clippy complexity-clippy llvm-lines deps-check cargo-chef cargo-chef-plan cargo-chef-cook

PARALLEL ?= 4

# Toolchain images paws ci/docker pulls directly (see docs/ROADMAP.md's
# "Base image version policy") -- kept as an explicit allowlist rather than
# any blanket `docker image prune -a`/`system prune`, since this machine's
# Docker daemon is shared with other, unrelated projects (their images/
# volumes/containers must never be touched by paws's own cleanup).
PAWS_IMAGES := \
	rust:1-bookworm \
	golang:1-bookworm \
	node:lts-trixie \
	oven/bun:1-debian \
	astral/uv:python3.13-trixie-slim \
	eclipse-temurin:21-jdk-jammy

build:
	cargo build --workspace

release:
	cargo build --workspace --release

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

complexity-clippy:
	cargo clippy --workspace --all-targets -- -W clippy::cognitive_complexity -W clippy::too_many_lines

cargo-chef:
	@cargo install --locked cargo-chef >/dev/null 2>&1 || cargo install --locked cargo-chef --force >/dev/null 2>&1 || true

cargo-chef-plan: cargo-chef
	@mkdir -p .cargo/chef
	cargo chef prepare --recipe-path .cargo/chef/recipe.json

cargo-chef-cook: cargo-chef
	@mkdir -p .cargo/chef
	cargo chef cook --release --recipe-path .cargo/chef/recipe.json

llvm-lines:
	@cargo install --locked cargo-llvm-lines >/dev/null 2>&1 || cargo install --locked cargo-llvm-lines --force >/dev/null 2>&1 || true
	cargo llvm-lines --all-targets

deps-check:
	@if cargo deny --version >/dev/null 2>&1; then cargo deny check; else echo "cargo deny not installed; install with: cargo install cargo-deny --locked"; fi
	@if cargo machete --version >/dev/null 2>&1; then cargo machete; else echo "cargo machete not installed; install with: cargo install cargo-machete --locked"; fi
	@if cargo udeps --version >/dev/null 2>&1; then cargo udeps --workspace --all-targets; else echo "cargo udeps not installed; install with: cargo install cargo-udeps --locked"; fi

quality: clippy complexity-clippy

complexity: quality llvm-lines deps-check

# Runs the full fixture gauntlet (scripts/run-fixtures.sh) for real, through
# Dagger, against every example under examples/ -- depends on release so
# it's always exercising the binary the gauntlet's failure/success claims
# are actually about. Override concurrency with `make fixtures PARALLEL=8`.
fixtures: release
	PARALLEL=$(PARALLEL) ./scripts/run-fixtures.sh

# Reclaims disk space `make fixtures` (or any real paws ci/docker run)
# accumulates: Dagger's own internal build cache (freed by recreating its
# engine container -- it's stateless from paws's point of view and
# auto-recreates on next `dagger`/`paws` invocation, so this is safe even
# though the container shows as "running"), the host Docker daemon's
# build cache, and paws's own pulled toolchain images specifically (see
# $(PAWS_IMAGES) above) -- never a blanket prune, since other projects'
# images/volumes/containers may share this same Docker daemon. Every step
# is best-effort (`-` prefix): a container/image that's already gone is
# not a failure.
clean:
	@echo "Removing the Dagger engine container (frees its internal build cache; auto-recreates on next use)..."
	-docker rm -f $$(docker ps -aq --filter "name=^dagger-engine-") 2>/dev/null
	@echo "Pruning the host Docker daemon's build cache..."
	-docker builder prune -f
	@echo "Removing paws's own pulled toolchain images..."
	-docker rmi $(PAWS_IMAGES) 2>/dev/null
	@echo "done."
