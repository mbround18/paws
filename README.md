# paws

Run-anywhere CI/CD pipelines, backed by [Dagger](https://dagger.io), shipped as a single Rust binary.

Not tied to GitHub Actions: the same pipeline logic runs in CI or on your laptop.

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

## CI

`.github/workflows/ci.yaml` has two jobs:
- **`test`** — `cargo build`/`cargo test --workspace`/`cargo clippy`/the SC-004 dagger-call-site
  lint. `cargo test --workspace` also runs `paws-docker`'s real-Docker-daemon e2e suite and
  `paws-provision`'s concurrency-timing test (SC-005) — no separate CI steps needed for either.
- **`ci-e2e`** — installs the real `dagger` CLI and runs `paws ci --toolchain rust` and
  `--toolchain node` end-to-end against `examples/rust-fixture`/`examples/node-fixture` (FR-008),
  kept as its own job since it depends on external infrastructure (a Dagger engine, `gh-reusable`
  being reachable on GitHub) the fast unit-test job doesn't need.

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

## Examples / fixtures

`examples/` holds small, real fixture projects `paws`'s own tests and CI exercise
subcommands against — see [`examples/README.md`](examples/README.md).

## Architecture decisions

Significant, trade-off-laden architectural decisions are recorded in
[`docs/adr/`](docs/adr/README.md) as ADRs — e.g.
[0001](docs/adr/0001-route-container-execution-through-dagger.md), why `paws release` routes
every build/smoke-test through `dagger core` instead of shelling to `docker`/`cross` directly.

## Principles

See [`.specify/memory/constitution.md`](.specify/memory/constitution.md) for the project's
formal governing principles (one crate per domain, subprocess-first Dagger access, incremental
SDK adoption, parity testing, reliability/testability-first) and development workflow rules.
