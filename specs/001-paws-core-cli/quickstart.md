# Quickstart: `paws`

## Build

```sh
cargo build --workspace
```

Produces the `paws` binary at `target/debug/paws`.

## `paws semver` — compute the next version

`paws semver` is the first fully-native subcommand (no `dagger` CLI required) and ports
`actions/semver`'s behavior directly into Rust — see `crates/paws-semver`.

### Against a real GitHub repo

Reads the last tag via GitHub's GraphQL API (needs `GITHUB_TOKEN` for private repos or to
avoid low unauthenticated rate limits):

```sh
export GITHUB_TOKEN=ghp_...
export GITHUB_REPOSITORY_OWNER=your-org
export GITHUB_REPOSITORY=your-org/your-repo
paws semver --branch main
```

### Without network access, using an explicit base

```sh
paws semver --base v1.0.0 --prefix v --branch main
# -> v1.0.1 (default patch increment)

paws semver --base v1.0.0 --prefix v --increment major --branch main
# -> v2.0.0 (--increment always wins over label/branch inference)
```

### Label-driven increments

```sh
paws semver --base v1.0.0 --prefix v --labels major --branch main
# -> v2.0.0 (a configured label wins over branch-name inference)

paws semver --base v1.0.0 --prefix v --branch feat/new-thing
# -> v1.1.0 (no labels present; "feat/" branch name infers a minor bump)
```

### PR builds

```sh
GITHUB_SHA=abcdef1234567890 paws semver --base v1.0.0 --prefix v --pr --branch fix/bug
# -> v1.0.1-pr.abcdef1
```

Full precedence (tag-ref passthrough, `--increment` override, label inference,
branch-name fallback, prefix inference from existing tags) is documented in
`specs/001-paws-core-cli/spec.md`'s FR-011 and exercised by `crates/paws-semver`'s test suite.

## `paws docker` — resolve build facts

`paws docker` natively resolves dockerfile/context/target/tags/push (see `crates/paws-docker`);
the actual build/push still goes through `paws-dagger` (needs `dagger` on `PATH`).

```sh
paws docker --image ghcr.io/example/app --version 1.0.0 --context examples/docker-compose-fixture
# resolves to the "app" service's context/dockerfile/target from that compose file (FR-012)

paws docker --image ghcr.io/example/plain --version 1.0.0 --context examples/docker-fixture
# no compose file there -> falls back to ./Dockerfile and "." (spec.md's edge case)

paws docker --image ghcr.io/example/app --version 1.0.0 --push
# --push forces the push decision regardless of branch/tag/canary-label gating
```

`crates/paws-docker/tests/e2e_docker_daemon.rs` builds `examples/`'s fixtures against a real
Docker daemon when one is available, including `examples/docker-buildkit-fixture` — a
Dockerfile that only builds via `docker buildx build`, proving the BuildKit path actually works
rather than silently falling back to the legacy builder.

## `paws provision` — concurrent toolchain setup

Real installers (`rustup`, `corepack`+pnpm, `uv`) run concurrently via `paws-provision`'s
`tokio::JoinSet`-based orchestrator — one failure never hides another's outcome:

```sh
paws provision --toolchains rust,node,python --verbose
# each ecosystem's install starts at roughly the same instant, not sequentially
```

`paws ci` calls this internally (FR-015) whenever the current directory has markers for more
than one ecosystem (`Cargo.toml` + `package.json`, etc.) — see `examples/multi-ecosystem-fixture`.

## `paws docs` — build workspace documentation

Native (no `dagger` CLI needed); wraps `cargo doc --workspace --no-deps` and produces a stable,
idempotent output path:

```sh
paws docs
# docs: built at /path/to/paws/target/doc
```

## `paws ci` — build and test a language target

Routes through `paws-dagger` to `gh-reusable`'s real, pinned Dagger module (needs `dagger` on
`PATH` — see https://docs.dagger.io/install). Verified end-to-end against real fixtures:

```sh
cd examples/rust-fixture && paws ci --toolchain rust
# ✅ rust-build-and-test — cargo fmt/clippy/build/test/release build, all green

cd examples/node-fixture && paws ci --toolchain node
# ✅ pnpm-build-and-test — pnpm install/build/test, all green
```

Exits non-zero and prints the pipeline's markdown report (including the specific failure) when a
step fails — try it against `examples/node-fixture-with-lint-failure` for a real failing case.

## `paws audit` — run the scanner suite

Also routes through `paws-dagger` (same real module, `audit` function). Without `dagger`
installed, or in a repo with no recognizable project markers, it fails fast/short-circuits
rather than silently doing nothing:

```sh
paws audit
# (in a repo with no recognizable Rust/Node/Python/Go/Docker markers)
# audit: no recognizable project markers found here; nothing to scan.

paws ci --toolchain rust
# (without `dagger` installed)
# Error: `dagger` CLI not found on PATH. Install it from https://docs.dagger.io/install and re-run `paws`.
```

`docker`'s actual build/push (as opposed to `paws docker`'s local fact preview) goes through the
same real module too. All three are pinned to a known-good `gh-reusable` commit rather than a
floating branch — see `GH_REUSABLE_DAGGER_MODULE` in `crates/paws-cli/src/main.rs` for why.

## `paws release` — build, smoke-test, package, and publish a release binary

Routes through `paws-dagger` (needs `dagger` on `PATH`) against `./builders/*` Dockerfiles — no
`docker`/`cross`/Wine/QEMU setup needed independently of `dagger` itself:

```sh
paws release --target x86_64-unknown-linux-gnu --tag v0.0.1-prerelease.1 --no-upload
# release: building paws for x86_64-unknown-linux-gnu via builders/linux-gnu...
# release: built target/dagger-release/x86_64-unknown-linux-gnu/paws
# release: smoke testing...
# release: smoke test output: paws 0.1.0
# release: packaged target/release-archives/paws-0.0.1-prerelease.1-x86_64-unknown-linux-gnu.zip

paws release --target aarch64-unknown-linux-gnu --tag v0.0.1-prerelease.1 --no-upload
# same triple family, aarch64 — the smoke test runs under Dagger's own
# QEMU-backed --platform=linux/arm64 execution, not a manual qemu/cross setup

paws release --target x86_64-pc-windows-gnu --tag v0.0.1-prerelease.1 --no-upload
# builds via builders/windows-gnu (mingw-w64); smoke-tested by running the
# real .exe under Wine, inside a Dagger container

paws release --target aarch64-apple-darwin --tag v0.0.1-prerelease.1 --no-upload
# builds via builders/macos (osxcross; SDK auto-fetched + checksum-verified
# from joseluisq/macosx-sdks) -> a real Mach-O binary, but no smoke test:
# release: no execution environment available for aarch64-apple-darwin,
# skipping smoke test (build/link success only)
```

`--target` must be one of `paws_release::known_targets()` (`x86_64`/`aarch64` ×
`unknown-linux-gnu`/`unknown-linux-musl`/`apple-darwin`, plus `x86_64-pc-windows-gnu`) — each maps
to a `./builders/<dir>` Dockerfile. Drop `--no-upload` (and set `GITHUB_TOKEN`/`GITHUB_REPOSITORY`,
or pass `--repository owner/repo`) to actually create/update the GitHub Release for the tag and
upload the archive as an asset. `.github/workflows/release.yaml` runs this per-target on every
`v*` tag push, gated by `paws ci` via `ci.yaml` as a reusable workflow first.

## Tests

```sh
cargo test --workspace
```

## SC-004 lint (no direct `dagger` spawns outside `paws-dagger`)

```sh
./scripts/check-dagger-callsites.sh
```
