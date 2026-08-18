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
  anywhere. Desktop (`--toolchain tauri`) is Linux-only for now. Mobile (`--toolchain
  tauri-android`) builds against `builders/tauri-android/Dockerfile` (JDK 17 + Android SDK/NDK +
  Rust's Android cross targets), assuming the target repo already ran `tauri android init`
  (`src-tauri/gen/android` committed) — `paws` doesn't scaffold mobile projects itself. iOS has no
  builder image and isn't planned as one: `cargo tauri ios build` needs real Xcode/`xcodebuild`,
  which only runs under Apple's license on genuine macOS — see `docs/ROADMAP.md`.
- `crates/paws-python` — native port of `gh-reusable`'s real `pythonBuildAndTest` Dagger function
  (`packages/dagger-module/src/index.ts`): `uv sync --all-groups [--frozen] && uv build && uv run
  pytest` against `astral/uv:python<version>-trixie-slim`, a plain `container from` pipeline (no
  dedicated `builders/*` Dockerfile needed, unlike Tauri). `uv`-based projects only
  (`pyproject.toml`) — that's what `gh-reusable` actually supports, no poetry/pipenv/pip path
  exists there to port. `--frozen` is only passed when `uv.lock` is committed, the same
  lockfile-optional-install fix `paws-node` needed for `npm ci`.

## CI

`.github/workflows/ci.yaml` has two jobs:
- **`test`** — `cargo build`/`cargo test --workspace`/`cargo clippy`/the SC-004 container-
  engine-call-site lint (`scripts/check-dagger-callsites.sh`, which also enforces
  [ADR-0001](adr/0001-route-container-execution-through-dagger.md)'s `docker`/`cross` rule).
  `cargo test --workspace` also runs `paws-docker`'s real-Docker-daemon e2e suite and
  `paws-provision`'s concurrency-timing test (SC-005) — no separate CI steps needed for either.
- **`ci-e2e`** — installs the real `dagger` CLI and runs `paws ci --toolchain rust`,
  `--toolchain node`, and `--toolchain python` end-to-end against `examples/rust-fixture`/
  `examples/node-fixture`/`examples/python-fixture` (FR-008), kept as its own job since it depends
  on external infrastructure (a Dagger engine, `gh-reusable` being reachable on GitHub) the fast
  unit-test job doesn't need.

`main` is protected by a ruleset requiring the `build, test, lint` check before merge (no force
push, no deletion). The repo owner can always bypass it (break-glass); Renovate's GitHub App can
bypass it specifically when merging its own PRs, so its automerge (see `renovate.json`) is never
blocked by the ruleset itself — it's still gated on that same status check passing first, since
Renovate waits for required checks regardless of branch protection.

## Releases

`.github/workflows/release.yaml` triggers on any `v*` tag push (or manual dispatch). It first
calls `ci.yaml` as a reusable workflow — a release build is gated by the same fmt/clippy/test/
build check as every other push, dogfooding `paws ci` on `paws` itself — then runs two jobs in
parallel: `bootstrap` (builds a native `paws` binary once, uploaded as a build artifact) and
`build-builders`, a **matrix over each builder name** (one runner per builder, not all 7 on a
single runner — building them concurrently on one machine exhausted its disk) that builds
`builders/<name>/Dockerfile` and pushes it to both GHCR (`GHCR_TOKEN`) and Docker Hub
(`DOCKER_TOKEN`) in a single `docker compose build --push` against `../compose.yml` (`image:` is
the GHCR ref, `build.tags` adds the Docker Hub one) — deliberately never loading a full image into
the runner's local Docker daemon, which is what caused the disk exhaustion in the first place. It
then re-verifies both pushes actually landed (`docker buildx imagetools inspect`), since `docker
compose build --push` has been observed to silently load an image locally instead of pushing it
(exit 0, no error, only the registry cache actually landed) without reproducing locally against
the same compose file — a real gap the verification step exists to catch loudly instead of
surfacing later as a confusing "image not found" in `paws release`. Only once both jobs succeed
(`needs: [ci, bootstrap, build-builders]`) does the per-target matrix start: each leg downloads
the bootstrapped binary and runs `paws release`, which *pulls* the matching prebuilt image
(`prebuilt_image_candidate`/`remote_image_exists` in `crates/paws-release`/`crates/paws-dagger`)
rather than building any Dockerfile itself — deliberately pull-only, no local-build fallback, so a
`build-builders` failure fails loudly instead of silently getting papered over by every leg
quietly rebuilding its own copy. Each leg smoke-tests the binary, packages a `.zip`, and uploads
it to the tag's GitHub Release (`generate_release_notes: true`,
picking up merged PRs since the previous tag automatically), marked prerelease iff the tag
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
`actions/paws-up` downloads a release binary for the runner, puts it on `PATH`, and runs `paws
init` to install the `dagger` CLI (`uses: mbround18/paws/actions/paws-up@main`). Composite rather
than Docker-based, for the same reason as `paws release`'s own build path (docs/adr/0001): a
Docker-based action runs inside a container, which would put `paws`'s own Dagger/Docker calls
behind an extra Docker-in-Docker layer. See `actions/paws-up/README.md` for inputs/outputs.

`paws init` (`crates/paws-dagger::install_cli`) runs `dagger`'s own official install script
(the same one `.github/workflows/ci.yaml`/`release.yaml` already used inline), pinning `BIN_DIR`
to `$HOME/.local/bin` rather than the script's own default (`./bin`, relative to whatever the
current directory happens to be — confirmed by reading the script, not assumed) and appending
that directory to `$GITHUB_PATH` when running inside a GitHub Actions job. It shells to `sh`, not
`dagger`/`docker`/`cross`, so it sits outside ADR-0001's scope — it installs the tool that ADR is
about, it doesn't execute a pipeline.

Note: `release.yaml`'s own bootstrap step still builds `paws` from source (`cargo build --release
-p paws-cli`) rather than using `paws-up`, deliberately — the very first release has nothing
to download yet, and building from source has no bootstrap-order dependency on a prior release
existing. `paws-up` is for other consumers of `paws`, not `paws`'s own release pipeline.

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
