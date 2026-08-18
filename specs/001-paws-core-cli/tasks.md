# Tasks: Paws Core CLI — Rust-native Reimplementation of gh-reusable's Pipeline Surface

## Rules

- Keep changes backward-compatible unless explicitly declared breaking (this is pre-1.0, but treat every shipped subcommand's flags as a contract from the moment it merges).
- Pair contract changes with tests in the same PR.
- Keep `README.md`'s subcommand list and this file's checklist in sync as work lands.
- Every ported behavior must name the exact `gh-reusable` file/function it's asserting parity against (plan.md's Contract-Safety Checklist).

## Tasks

### 0. Prerequisites

- [x] Run `/speckit-constitution` to establish `paws`'s project principles formally (plan.md's Constitution Check flagged this as missing) — see `.specify/memory/constitution.md` v1.0.0
- [x] Confirm `actions/semver`'s tagless-repo default version by reading its TS source — resolved in spec.md FR-011 (`{prefix}0.0.0`, default prefix `v`, from `actions/semver/src/tag.js`)
- [x] Extract `docker-facts`'s multi-service `docker-compose.yml` resolution rule from source — resolved in spec.md FR-012 (first `image:`-matching service wins, from `packages/dagger-pipelines/src/docker-parity.ts`)

### 1. Workspace and CLI wiring

- [x] Wire `paws-cli`'s `Ci`, `Docker`, `Semver`, `Audit`, `Docs` handlers to call through `paws-dagger::call` instead of printing `(unimplemented)` — interim wiring against `GH_REUSABLE_DAGGER_MODULE`; `Semver`/`Audit`/`Docker` will be rewired to their native crates as tasks 30/39/48 land
- [x] Add a `dagger` CLI presence check with the FR-010 actionable error message, run once at `paws` startup — `paws_dagger::ensure_available()`, called first thing in `main()`
- [x] Add a CI job (or local script) that greps for `Command::new("dagger")` outside `crates/paws-dagger` and fails if found (SC-004) — `scripts/check-dagger-callsites.sh`; wiring it into actual CI is task group 8

### 2. `paws semver` (User Story 2, P1 — pilot crate)

- [x] Create `crates/paws-semver` with `Cargo.toml` + `src/lib.rs`
- [x] Port last-tag lookup (GitHub GraphQL query, matching `actions/semver`'s current query) behind a trait so it's mockable in tests — `TagSource` trait; `GitHubGraphQlTagSource` (live) and `FixtureTagSource` (tests)
- [x] Port increment precedence: explicit `--increment` > `major-label` > `minor-label` > `patch-label` (spec FR-003) — `detect_increment`, plus full FR-011 precedence (tag-ref passthrough, `base` override, branch-name fallback, PR prerelease, prefix inference)
- [x] Add fixture tests: tagged repo + label set, explicit increment override, tagless repo default (`{prefix}0.0.0` per resolved FR-011), PR-label-vs-branch-name precedence, prefix inference from existing `v`-prefixed tags — 9 tests in `crates/paws-semver/src/lib.rs`
- [x] Wire `paws-cli`'s `Semver` subcommand to `paws-semver` — no longer routes through `paws-dagger`/`ensure_available` since it doesn't need the `dagger` CLI
- [x] Write `specs/001-paws-core-cli/quickstart.md` using `paws semver` as the first working example

### 3. `paws audit` (User Story 4, P2)

- [x] Create `crates/paws-audit` with `Cargo.toml` + `src/lib.rs`
- [x] Port `AuditSummary`/`AuditScannerResult`/`ComplianceStatus` shapes from `packages/dagger-module/src/audit-types.ts` — ported as-is (the TS source has no `ComplianceStatus` type; `AuditOverallStatus` is the actual name)
- [x] Port scanner aggregation logic from `audit-logic.ts` (confidence ranking, failed/skipped scanner handling) — plus family detection, scanner selection, and semgrep/gitleaks finding normalization
- [x] Add fixture tests: no-findings pass, single-finding summary shape match — 4 tests in `crates/paws-audit/src/lib.rs`
- [x] Wire `paws-cli`'s `Audit` subcommand to `paws-audit` — detects signals from the cwd, still runs scanners via the interim `paws-dagger` wiring (real semgrep/gitleaks execution isn't native yet), and produces spec.md's "nothing to do here" message when no project markers are found

### 4. `paws docker` (User Story 3, P2)

- [x] Create `crates/paws-docker` with `Cargo.toml` + `src/lib.rs`
- [x] Port `docker-facts` Dockerfile/context discovery (compose-first, fallback to `./Dockerfile`/`.`) — path resolution simplified to always-relative-to-workspace (see the crate's module doc comment); values match, absolute-path string formatting does not
- [x] Port tag generation (branch/version/target/multi-registry) and push-gating (`canary_label`, `force_push`)
- [x] Build the Docker e2e fixture harness (real Docker daemon) required by spec FR-007 — `crates/paws-docker/tests/e2e_docker_daemon.rs`, gated on `docker`/`buildx` presence like `paws-dagger`'s `ensure_available`; also proves `examples/docker-buildkit-fixture` fails on the legacy builder and succeeds via `docker buildx build`
- [x] Add fixture tests: compose-defined build, no-compose fallback, canary-label gating, force-push override, **multi-service compose file (one matching image, one not) per resolved FR-012** — 8 unit tests + 3 e2e tests
- [x] Wire `paws-cli`'s `Docker` subcommand to `paws-docker`

### 5. `paws provision` (User Story 5, P2 — concurrency foundation)

- [x] Create `crates/paws-provision` with `Cargo.toml` (`tokio` dependency) + `src/lib.rs`
- [x] Define the per-ecosystem installer shape (trait or async fn — decide during implementation per plan.md's Open Questions) for rust/rustup, node/pnpm, python/uv — `Installer` trait (blanket-impl'd over `Fn() -> Future`), plus real `install_rust`/`install_node`/`install_python` shelling to `rustup`/`corepack`/`uv`
- [x] Implement the `JoinSet`-based orchestrator: launch one task per requested ecosystem, await all, return an aggregated `{ecosystem: Result<...>}` (FR-013)
- [x] Verify no early-return-on-first-failure path exists — every ecosystem's outcome must be present in the aggregated result even when others fail (FR-014) — also fixed a real bug found while doing this: a panicking task's failure was previously attributed to a hardcoded `Ecosystem::Rust` instead of the ecosystem that actually panicked (could silently overwrite Rust's real result); now tracked via `JoinSet::join_next_with_id`
- [x] Add a timing-based test: 3 mock installers with known sleep durations; assert total wall-clock ≈ max(durations), not sum (SC-005)
- [x] Add a failure-isolation test: one of three mock installers fails; assert the other two still report success in the aggregate (FR-014) — plus a same-shaped test for a *panicking* installer, and a real-installer integration test gated on rustup/corepack/uv being on `PATH`
- [x] Wire `paws-cli`'s new `Provision` subcommand (`paws provision --toolchains rust,node,python`) to `paws-provision` — `--verbose` prints per-ecosystem start/elapsed timestamps (spec.md User Story 5 acceptance scenario 1)
- [x] Wire `paws ci` to call `paws-provision` internally when the target repo needs more than one ecosystem, instead of a sequential setup loop (FR-015) — detects needed ecosystems from `Cargo.toml`/`package.json`/`pyproject.toml` markers in the cwd; verified against `examples/multi-ecosystem-fixture`

### 6. `paws ci` (User Story 1, P1)

- [x] Wire `paws ci --toolchain node` through `paws-dagger` (install/lint/test, matching `pnpmBuildAndTest`) — `function: "pnpm-build-and-test"`, now with the required `--source` argument (see below); also provisions concurrently first when the target repo needs more than one ecosystem (FR-015)
- [x] Wire `paws ci --toolchain rust` through `paws-dagger` (matching `rustBuildAndTest`) — same, `function: "rust-build-and-test"`
- [x] Add real fixture projects (one Node, one Rust) exercised end-to-end in `paws`'s own CI (spec FR-008) — **resolved**. What was actually blocking this: (1) `GH_REUSABLE_DAGGER_MODULE` was a placeholder never validated against a real `dagger call`; (2) the interim wiring never passed the `--source` argument every one of these functions requires, so every real call would have failed regardless; (3) `dagger`'s floating `main`-branch module reference was verified broken (`TypeError` from a stale vendored TS SDK bundle) — fixed by pinning to a commit verified working end-to-end. `examples/node-fixture` also needed a real `build` script + `pnpm-lock.yaml` to satisfy `pnpmBuildAndTest`'s actual contract (it only had `lint`/`test`). Verified locally: `paws ci --toolchain rust` and `--toolchain node` both run clean against a real `dagger` CLI + engine. `.github/workflows/ci.yaml` now has a `ci-e2e` job that installs `dagger` and runs both end-to-end on every push/PR.

### 7. `paws docs` (User Story 6, P3)

- [x] Wire `paws docs` to run `cargo doc` against the current workspace and produce a stable output path — new `crates/paws-docs`, no longer routes through `paws-dagger` (doesn't need the `dagger` CLI at all)
- [x] Confirm idempotency (safe to re-run without side effects) — test re-runs `build_docs` twice against the real workspace; second run is a fast no-op incremental build, not a failure or rebuild-from-scratch

### 8. Governance and CI bootstrap

- [x] Decide and implement `paws`'s own CI: start with plain `cargo test --workspace` on GitHub Actions; revisit dogfooding `paws ci` once task 6 ships — `.github/workflows/ci.yaml`; task 6 now shipped (real `dagger` calls verified working), and `.github/workflows/release.yaml` dogfoods `paws ci` by calling `ci.yaml` as a reusable workflow to gate every release
- [x] Add the SC-004 grep-lint as a required CI check
- [x] Add `cargo test --workspace` as a required CI check (SC-002)
- [x] Add the SC-005 provisioning-concurrency timing check as a required CI check once `paws-provision` shipped — covered by `cargo test --workspace` itself (`paws-provision`'s `independent_installers_run_concurrently_not_sequentially` test), no separate CI step needed

### 9. Documentation and propagation

- [x] Update `README.md`'s crate layout section as `paws-semver`/`paws-audit`/`paws-docker`/`paws-provision` land — plus `paws-docs`, and a new CI section
- [x] Update `specs/001-paws-core-cli/quickstart.md` incrementally as each subcommand ships
- [x] Note in this repo's `specs/002-reusable-rust-pipeline/` (inspiration copy) that it has been superseded by `001-paws-core-cli` for anything beyond pure reference

### 10. `paws release` — cross-target build + GitHub Release publish (dogfooding)

Added after the core surface (task groups 0-9) shipped, per direct request: `paws` should be
its own CI/CD tool, including releasing itself. Not in the original spec.md scope — tracked here
rather than spinning up a separate feature spec, since it's a natural extension of the same CLI.

- [x] Create `crates/paws-release`: `build_binary`, `smoke_test`, `package_zip`,
  `GitHubReleaseClient` (REST API: get-or-create release by tag, `--clobber`-style asset replace)
- [x] Wire `paws-cli`'s new `Release` subcommand (`paws release --target <triple> --tag <tag>
  [--prerelease] [--no-upload] [--skip-smoke-test]`)
- [x] `.github/workflows/release.yaml`: triggers on `v*` tag push or manual dispatch; calls
  `ci.yaml` as a reusable workflow first (dogfooding — a release never skips `paws ci`'s
  fmt/clippy/test/build gate); matrix over `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
  `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `x86_64-pc-windows-gnu`; prerelease
  flag derived from whether the tag contains a `-` (semver convention), not hardcoded

**Revised after review**: the first pass of this crate shelled directly to `cargo`/`cross`
(itself Docker-based) and to raw `docker run`/`docker/setup-qemu-action`/Wine containers for
smoke testing — bypassing `paws-dagger` entirely and undermining the project's own "backed by
Dagger" premise (SC-004's lint only checks for `Command::new("dagger")`, so nothing caught this).
Refactored per direct request; the decision and trade-offs are recorded in
[`docs/adr/0001-route-container-execution-through-dagger.md`](../../docs/adr/0001-route-container-execution-through-dagger.md):

- [x] Added `./builders/*` — one directory per target family, each a plain `Dockerfile` with OCI
  labels (`org.opencontainers.image.version`/`.revision`/`.created`, stamped via `--build-args`
  at build time) and a `builders/README.md`. `linux-gnu` (x86_64+aarch64 via a Debian cross-gcc),
  `linux-musl-x86_64`/`linux-musl-aarch64` (separate images, based on the maintained
  `messense/rust-musl-cross` — musl cross toolchains are per-arch, unlike glibc's, so one shared
  image doesn't work here), `windows-gnu` (mingw-w64), and `macos` (osxcross, per
  [jamwaffles' Linux→macOS cross-compile guide](https://jamwaffles.github.io/rust/2019/02/17/rust-cross-compile-linux-to-macos.html)).
- [x] **macOS SDK acquisition fully automated** (revised again after review, this time pointed at
  the user's own [`mbround18/setup-osxcross`](https://github.com/mbround18/setup-osxcross) — a
  GitHub Action doing equivalent setup): the SDK is fetched from
  [`joseluisq/macosx-sdks`](https://github.com/joseluisq/macosx-sdks)' GitHub releases and
  verified against that release's `sha256sum.txt`, no manual SDK download step needed. Cross-
  checked against a second real project, `joseluisq/docker-osxcross` (the base image behind
  `rust-linux-darwin-builder`) — confirmed it pulls from the identical source. Also picked up the
  `CARGO_TARGET_*_RUSTFLAGS` linker workaround (`-C link-arg=-undefined -C
  link-arg=dynamic_lookup`) from `setup-osxcross` that the first pass of `builders/macos/Dockerfile`
  had missed.
- [x] **Actually built and verified `builders/macos/` for real** (not just scaffolded): fixed a
  stale `OSXCROSS_COMMIT` pin (didn't exist — updated to the real current HEAD) and a missing
  `python3` build dependency, then hit and fixed two real cross-compilation issues beyond what
  `setup-osxcross`'s own scope covers (their action doesn't build anything with a C-component
  dependency): (1) `ring` (pulled in via `reqwest`'s `rustls-tls`, used by `paws-semver`/
  `paws-release`) needs `CC_*`/`CXX_*`/`AR_*` env vars, not just `CARGO_TARGET_*_LINKER` — cc-rs
  invokes those directly; (2) osxcross's SDK autodetection parses its own wrapper binary's file
  name for a darwin version suffix, which breaks once that binary is symlinked to a fixed
  unversioned name (needed since `CARGO_TARGET_*_LINKER` can't glob) — fixed via an explicit
  `OSXCROSS_SDKROOT` env var, which osxcross's wrapper (`wrapper/target.cpp`) checks before
  falling back to name-based detection; (3) the final link step invoked the system's plain GNU
  `ld` (which doesn't understand Mach-O-specific flags like `-dynamic`) instead of osxcross's own
  bundled `ld` — fixed via `-C link-arg=-fuse-ld=<osxcross's ld, symlinked to a fixed name>`.
  `paws release --target x86_64-apple-darwin` and `--target aarch64-apple-darwin` both run
  end-to-end through the real CLI and produce genuine Mach-O 64-bit executables (`file` confirms
  `Mach-O 64-bit x86_64/arm64 executable`). **Both are now in `known_targets()`** and in
  `release.yaml`'s matrix.
- [x] `TargetConfig.smoke` changed from `SmokeTestSpec` to `Option<SmokeTestSpec>` to represent
  this honestly: `None` for both macOS targets, since Dagger can't run a macOS container and Wine
  only emulates Windows PE, not Mach-O — no execution environment exists to smoke-test them.
  `paws release` build+links, prints an explicit "no execution environment available... skipping
  smoke test (build/link success only)" message, and packages the binary anyway rather than
  failing the leg or silently claiming it was verified. Also worth flagging for anyone relying on
  these two targets: the link succeeds via `-undefined dynamic_lookup`, which permits unresolved
  symbols at link time (resolved against real macOS frameworks only at actual runtime) — a
  successful cross-link is not the same strength of guarantee as the natively-verified Linux/
  Windows targets get from their smoke tests.
- [x] Added `paws_dagger::core()` — moduleless `dagger core <chain>` pipelines (`host directory
  ... docker-build`, `with-mounted-directory`, `with-exec`, `file`, `export`, `container
  --platform=...`) — confirmed by direct experimentation that Dagger supports this without a
  custom module, keeping `paws-dagger` the single seam that spawns `dagger` even for ad-hoc
  builds.
- [x] Rewrote `paws-release::build_binary` to build via `dagger core host directory
  --path=<builder> docker-build` + `with-mounted-directory` (the source tree) + `with-exec`
  (`cargo build --release --target ...`) + `file`/`export` — gets Dagger's own BuildKit layer
  caching for free (confirmed: an unchanged builder image and mounted source are reported
  `CACHED` on a second run), no separate cache setup needed.
- [x] Rewrote smoke testing as `paws_release::smoke_test`, also via `dagger core container
  --platform=<platform> from --address=<image> with-mounted-file ... with-exec`: native execution
  for the two x86_64 targets, Dagger's own QEMU-backed `--platform=linux/arm64` for the two
  aarch64 targets (confirmed the *same* underlying mechanism as before — host-level
  `binfmt_misc`, registered once via `docker run --privileged tonistiigi/binfmt --install all` —
  just invoked through Dagger's engine instead of a bare `docker run`), and a Wine-enabled base
  image (`scottyhardy/docker-wine`) run as a Dagger container for the Windows target.
- [x] **Re-verified all 5 targets against the new, dagger-only code path** (not just the old
  cross/docker path): `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
  `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `x86_64-pc-windows-gnu` each ran
  `paws release --target <triple> --tag v0.0.1-prerelease.1 --no-upload` through the actual CLI
  end-to-end — build, smoke test (each printing a correct `paws 0.1.0`), and packaging all
  succeeded, with zero direct `docker`/`cross`/`wine` spawns from `paws` itself (only `dagger`,
  via `paws-dagger`).
- [x] Simplified `.github/workflows/release.yaml` accordingly: no more `cross` install, no more
  `docker/setup-qemu-action`, no more separate Wine step — the job installs Rust + the `dagger`
  CLI and lets `paws release` do build + smoke test + package + upload for every matrix leg.
- [ ] **Not done — needs the user**: actually push a `v0.0.1-prerelease.N` tag to trigger a real
  run of `release.yaml` against the real `mbround18/paws` GitHub remote and iterate `N` until
  CI itself confirms it end-to-end. This is a real, visible, external action (creates public
  prereleases, spends Actions minutes) — deliberately left for the user to trigger rather than
  done unattended.
- [ ] `--version` currently reports `paws`'s `Cargo.toml` package version (`0.1.0`), not the
  release tag it was built for — cosmetic mismatch, not fixed yet.
