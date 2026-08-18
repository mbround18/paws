# Development guide

Architecture, crate layout, CI/release internals, and contributor-facing detail. For "what is
`paws` and how do I use it," see the top-level [`README.md`](../README.md). For what language/
stack support is planned versus already wired, see [`docs/ROADMAP.md`](ROADMAP.md).

## Origin

`paws` grew out of [gh-reusable](https://github.com/MBRound18/gh-reusable)'s reusable-workflow
library. That repo's `specs/002-reusable-rust-pipeline/` spec (copied here under `specs/`)
is the original inspiration for a first-class Rust pipeline contract — this project takes
that idea further: instead of adding Rust as one more supported language inside a
TypeScript-orchestrated system, the orchestrator itself is Rust.

## Layout

- `crates/paws-cli` — the `paws` binary. `clap`-based subcommands (`ci`, `docker`, `semver`,
  `audit`, `docs`, ...) are the narrative/user-facing layer.
- `crates/paws-core` — shared contract types (defaults, pipeline config shapes).
- `crates/paws-dagger` — wraps the `dagger` CLI. Deliberately **not** built on the
  `dagger-sdk` Rust crate yet — Dagger's own README marks that SDK experimental and
  "not for anything mission-critical." Pipeline logic goes through this crate so the
  day the SDK is trustworthy, only this crate needs to change. The interim `ci`/`docker`/
  `audit` subcommands call into `gh-reusable`'s real Dagger module through here, pinned to a
  known-good commit (see `GH_REUSABLE_DAGGER_MODULE` in `crates/paws-cli/src/main.rs`) rather
  than trusting its floating `main` branch, which was verified broken (a stale vendored SDK
  bundle threw at runtime) as of 2026-08-18.
- `crates/paws-semver` — native Rust port of `actions/semver` (no `dagger` CLI needed).
  The pilot crate for eventually evaluating `dagger-sdk`.
- `crates/paws-audit` — native Rust port of the audit/compliance aggregation logic
  (language detection, scanner selection, finding normalization/aggregation); running the
  actual `semgrep`/`gitleaks` containers still goes through `paws-dagger`.
- `crates/paws-docker` — native Rust port of `docker-facts`/`docker-release`'s resolution
  logic (compose discovery, tag generation, push gating); has a real e2e test suite that
  builds `examples/`' fixtures against an actual Docker daemon, including a BuildKit-only
  fixture. Building/pushing the image itself still goes through `paws-dagger`.
- `crates/paws-provision` — concurrent toolchain provisioning (`tokio::JoinSet`-based),
  aggregating per-ecosystem install results without one failure hiding another's. Real
  installers shell to `rustup`/`corepack`/`uv`; `paws ci` uses this internally whenever a repo
  needs more than one ecosystem (FR-015), rather than a sequential setup loop.
- `crates/paws-docs` — thin wrapper around `cargo doc --workspace --no-deps`; doesn't need the
  `dagger` CLI at all.
- `crates/paws-release` — cross-target build + smoke-test + package (`zip`) + GitHub Release
  publish. Build and smoke-test both route through `paws-dagger::core` (moduleless `dagger core`
  pipelines against `./builders/*` Dockerfiles) — never a direct `docker`/`cross` spawn, so
  `paws release` needs nothing beyond the `dagger` CLI. Only the GitHub REST API calls (plain
  HTTPS, no process spawn) and packaging (`zip`, a host utility, not a second build backend)
  fall outside that seam.
- `crates/paws-node` — native multi-package-manager Node support: detects npm/yarn/pnpm/bun
  (lockfile-first, falling back to `package.json`'s `packageManager` field) and Vite/Next.js
  frameworks, and builds the `dagger core` pipeline `paws ci --toolchain node` runs. No `dagger`
  CLI dependency itself — it only produces the argument list `paws-dagger::core` executes.
- `crates/paws-tauri` — Tauri desktop-app support, layered on `paws-node` (a Tauri project is a
  Node project with `src-tauri/tauri.conf.json`). Doesn't reimplement the frontend-then-Rust build
  ordering itself — Tauri's own CLI already sequences that via `tauri.conf.json`'s
  `beforeBuildCommand`. Builds against `builders/tauri-linux/Dockerfile` (Rust + Node + the
  GTK/WebKit libs Tauri's Linux backend needs), embedded into the binary via `include_str!` and
  materialized to a temp dir at runtime (`write_builder_dockerfile`) — a plain repo-relative path
  would resolve against the *target* repo `paws ci` is running in, not `paws`'s own source tree,
  since `paws` (unlike `paws-release`, which only ever builds itself) is meant to run from
  anywhere. Linux-only for now.

## CI

`.github/workflows/ci.yaml` has two jobs:
- **`test`** — `cargo build`/`cargo test --workspace`/`cargo clippy`/the SC-004 container-
  engine-call-site lint (`scripts/check-dagger-callsites.sh`, which also enforces
  [ADR-0001](adr/0001-route-container-execution-through-dagger.md)'s `docker`/`cross` rule).
  `cargo test --workspace` also runs `paws-docker`'s real-Docker-daemon e2e suite and
  `paws-provision`'s concurrency-timing test (SC-005) — no separate CI steps needed for either.
- **`ci-e2e`** — installs the real `dagger` CLI and runs `paws ci --toolchain rust` and
  `--toolchain node` end-to-end against `examples/rust-fixture`/`examples/node-fixture` (FR-008),
  kept as its own job since it depends on external infrastructure (a Dagger engine, `gh-reusable`
  being reachable on GitHub) the fast unit-test job doesn't need.

`main` is protected by a ruleset requiring the `build, test, lint` check before merge (no force
push, no deletion). The repo owner can always bypass it (break-glass); Renovate's GitHub App can
bypass it specifically when merging its own PRs, so its automerge (see `renovate.json`) is never
blocked by the ruleset itself — it's still gated on that same status check passing first, since
Renovate waits for required checks regardless of branch protection.

## Releases

`.github/workflows/release.yaml` triggers on any `v*` tag push (or manual dispatch). It first
calls `ci.yaml` as a reusable workflow — a release build is gated by the same fmt/clippy/test/
build check as every other push, dogfooding `paws ci` on `paws` itself — then, per target, builds
`paws` with `paws release` (bootstrapped from a host-native build of itself), smoke-tests it,
packages a `.zip`, and uploads it to the tag's GitHub Release, marked prerelease iff the tag
contains a `-` (semver convention).

Every build and smoke test goes through `dagger core` (see `crates/paws-release` and
`builders/`) — `dagger`'s own multi-platform container execution (backed by the runner's QEMU
`binfmt_misc` registration) handles the aarch64 legs, and a Wine-enabled base image handles the
Windows one, so the CI job needs nothing beyond Rust + the `dagger` CLI (no `cross`, no
`docker/setup-qemu-action`, no separate Wine setup). A binary that builds but doesn't run never
reaches a GitHub Release: the smoke test runs before packaging/upload.

Current target matrix (`paws_release::known_targets()`): `x86_64-unknown-linux-gnu`,
`aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`,
`x86_64-pc-windows-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin` — see `builders/README.md`
for each one's Dockerfile. The first 5 were verified end-to-end in development (real build + real
execution, not just "it compiled"). The 2 macOS targets build real, verified Mach-O binaries
(`builders/macos/`, via osxcross, SDK fetched automatically and checksum-verified from
`joseluisq/macosx-sdks`) but aren't smoke-tested — no Mach-O execution environment is available to
`dagger`/Wine — so `paws release` builds and packages them while honestly reporting the smoke test
as skipped; see `builders/macos/README.md`.

## Reusable GitHub Actions

`./actions/*` holds composite GitHub Actions for consuming `paws` from other workflows —
`actions/setup-paws` downloads a release binary for the runner and puts it on `PATH`
(`uses: mbround18/paws/actions/setup-paws@main`). Composite rather than Docker-based, for the
same reason as `paws release`'s own build path (docs/adr/0001): a Docker-based action runs inside
a container, which would put `paws`'s own Dagger/Docker calls behind an extra
Docker-in-Docker layer. See `actions/setup-paws/README.md` for inputs/outputs.

Note: `release.yaml`'s own bootstrap step still builds `paws` from source (`cargo build --release
-p paws-cli`) rather than using `setup-paws`, deliberately — the very first release has nothing
to download yet, and building from source has no bootstrap-order dependency on a prior release
existing. `setup-paws` is for other consumers of `paws`, not `paws`'s own release pipeline.

## Dependency updates

[Renovate](https://docs.renovatebot.com/) is configured via `renovate.json`: minor/patch updates
automerge once CI passes; major updates open a PR for manual review. 0.x packages (where a
"minor" bump isn't guaranteed backwards compatible under semver) are excluded from automerge.

## Examples / fixtures

`examples/` holds small, real fixture projects `paws`'s own tests and CI exercise
subcommands against — see [`examples/README.md`](../examples/README.md).

## Architecture decisions

Significant, trade-off-laden architectural decisions are recorded in
[`docs/adr/`](adr/README.md) as ADRs — e.g.
[0001](adr/0001-route-container-execution-through-dagger.md), why `paws release` routes
every build/smoke-test through `dagger core` instead of shelling to `docker`/`cross` directly.

## Principles

See [`.specify/memory/constitution.md`](../.specify/memory/constitution.md) for the project's
formal governing principles (one crate per domain, subprocess-first Dagger access, incremental
SDK adoption, parity testing, reliability/testability-first) and development workflow rules.

## Building from source

```sh
cargo build --workspace
cargo test --workspace
```

See [`specs/001-paws-core-cli/quickstart.md`](../specs/001-paws-core-cli/quickstart.md) for a
subcommand-by-subcommand usage walkthrough, including ones still routed through the interim
`paws-dagger` path.
