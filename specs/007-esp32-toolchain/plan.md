# Implementation Plan: `paws ci --toolchain esp32`

## Inputs

- Spec path: `specs/007-esp32-toolchain/spec.md`
- Affected contracts/files:
  - `crates/paws-esp32/` (new crate: `Cargo.toml`, `src/lib.rs`)
  - `builders/esp32/Dockerfile` (new)
  - `crates/paws-cli-core/src/lib.rs` (`CiArgs.publish_artifacts`, `--toolchain esp32` match arm
    in `run_ci_pipeline`, gating validation, doc comments, error message's expected-values list)
  - `crates/paws-provision/src/lib.rs` (`Ecosystem::Esp32`, `install_esp32` shelling to `espup`)
  - `compose.yml` (new `esp32` service)
  - `.github/workflows/release.yaml` (`build-builders` matrix gains `esp32`)
  - `builders/README.md` (mention `esp32/` joining the consumer-project builder category)
  - `docs/ROADMAP.md` (new `## Embedded (ESP32 / no_std targets)` section)
  - `examples/esp32-fixture/` (new: minimal real `esp-idf-svc` project)
  - `Cargo.toml` (workspace member list gains `crates/paws-esp32`)
  - Phase 0/1 artifacts: `research.md`, `data-model.md`,
    `contracts/paws-ci-esp32-contract.md`, `quickstart.md`

## Constitution Check

_GATE: evaluated before Phase 0 research; re-evaluated below after Phase 1 design._

| Principle | Assessment |
|---|---|
| I. One Crate Per Domain | New `crates/paws-esp32` crate, matching `paws-kotlin`/`paws-go`'s precedent of one crate per toolchain domain rather than growing `paws-rust` to cover a second, structurally different (ESP-IDF/`embuild`, no `cargo test` against the embedded target itself, artifact publishing) pipeline shape. `paws-cli-core` stays thin: only dispatch + cross-toolchain flag gating (`--publish-artifacts` requires `--toolchain esp32`), matching `--coverage`'s existing precedent. **PASS**. |
| II. Subprocess-First Dagger Access | All Dagger interaction goes through `paws_dagger::core`, same call-site pattern as every other toolchain crate — no new `Command::new("dagger")`. `--publish-artifacts`' GitHub upload is plain HTTPS via the existing `GitHubReleaseClient` (already subprocess-free, direct `reqwest`-style calls per its own doc comment), not a Dagger step — it runs *after* the Dagger pipeline completes, operating on the pipeline's output artifact. **PASS**. |
| III. Incremental SDK Adoption | Not applicable — no Dagger SDK involvement, same as every other toolchain crate. **PASS (not applicable)**. |
| IV. Parity Testing Over Reimplementation-From-Memory | Not a `gh-reusable` port (spec Motivation/Affected Contracts) — no ESP32 function exists there to port. New, `paws`-native capability, disclaimed explicitly. **PASS**. |
| V. Reliability & Testability First | Every new branch (project detected/not, fmt/clippy/build steps, host-testable-sibling-crate detected/not, `--publish-artifacts` on/off, missing-token error, out-of-`--toolchain esp32` rejection) gets a named unit test in Workstream 5 below. **PASS**, contingent on tasks.md enumerating each one. |
| Tech constraint: no secrets on CLI | `--publish-artifacts` reads `$GITHUB_TOKEN`/`$GH_TOKEN` from the environment only, same as `paws semver --push`/`paws helm --publish` — never accepted as a CLI flag value (which would leak into shell history/process listings). **PASS**. |
| Tech constraint: shared defaults live in one place | No new `PipelineDefaults` field — `publish_artifacts`' only "default" is `false`, expressed once in `CiArgs`, same reasoning `004-rust-coverage`'s Design Decision 5 already established for `coverage`. **PASS**. |
| Tech constraint: no swallowed concurrent failures | Not applicable — no concurrent orchestration in this feature (the two uploads in `--publish-artifacts`, bootloader + firmware ELF, run sequentially; a failure on either propagates, doesn't get silently swallowed). **PASS**. |

**Pre-Phase 0 Gate Status**: PASS, no unresolved conflicts.
**Post-Phase 1 Gate Status**: PASS — Phase 1 design surfaced that `GitHubReleaseClient` needs its
visibility widened from `paws-release`-internal to workspace-`pub` (research.md R2), which spec.md
already named explicitly under Affected Contracts; no new Constitution concern introduced by that
widening.

## Design Decisions

1. **`GitHubReleaseClient` is reused from `paws-release`, not duplicated or moved.** `paws-esp32`
   takes a normal path dependency on `paws-release` for just this type. Alternative considered:
   extract it into a new shared `paws-github` crate — rejected for this first cut as premature
   (exactly one additional consumer today); revisit if a third `paws ci` toolchain needs GitHub
   Release upload later, at which point extraction pays for itself. Matches this codebase's
   general "don't abstract until there's a second real consumer" pattern (e.g. `paws-rust`'s
   `dagger_pipeline_args` staying a plain function, not a trait, until `004-rust-coverage` gave it
   a second call shape).

2. **`esp32`'s pipeline chain is `fmt → clippy → build → (conditional) host-crate test`, not
   `fmt → clippy → test → build`** like `paws-rust`'s default order. The embedded target itself
   has no `cargo test` story (spec's Scope, citing `ha-kiosk`'s own `CONTRIBUTING.md` finding:
   `harness = false` on the `[[bin]]` skips even *compiling* `#[cfg(test)]` code for that target).
   Reordering `build` before the conditional test step means a host-testable sibling crate (e.g.
   `ha-kiosk`'s `firmware-core`) still gets tested even if detecting it is imperfect, without ever
   silently skipping the one step (`build`) that actually has to succeed for this toolchain to
   mean anything.

3. **Host-testable sibling crate detection: workspace member whose `Cargo.toml` has no
   `esp-idf-sys`/`esp-idf-svc` dependency and no `*-espidf` target override.** Not a hardcoded name
   (`firmware-core` is `ha-kiosk`-specific, not a generic convention) — a project without any such
   sibling simply gets no test step, same as `paws-rust`'s existing `is_wasm` short-circuit (design
   decision 4, `004-rust-coverage`) skips testing outright rather than erroring.

4. **`builders/esp32` follows the `tauri-android` embed-and-rebuild-in-`build-builders` pattern**
   (research.md R1), not `paws-release`'s pull-only prebuilt pattern — directly resolves spec.md's
   Runtime and Defaults Impact sizing/matrix-isolation note. `write_builder_dockerfile()` is named
   identically to `paws-kotlin`/`paws-tauri`'s counterparts.

5. **`--publish-artifacts` uploads exactly two assets: `bootloader.bin` and the firmware ELF**,
   found by well-known relative paths under the built project's target directory (ESP-IDF's own
   fixed build-output layout — `embuild`/`esp-idf-sys` don't let a project customize this), not a
   configurable glob. Keeps the first cut's contract narrow and predictable; a project needing
   additional artifacts (a merged combined image, a partition table) is a follow-up, not blocking
   this spec.

6. **`Ecosystem::Esp32` in `paws-provision` shells out to `espup install`**, mirroring
   `install_rust`/`install_go`'s "shell to the real installer" precedent from `docs/ROADMAP.md`'s
   "How a new stack gets added" step 2 — no attempt to reimplement ESP-IDF/Xtensa toolchain
   installation logic `espup` already solves correctly.

## Workstreams

1. **`crates/paws-esp32`** — new crate: `is_esp32_project(dir)` (Design Decision-adjacent to
   spec's detection rule: `esp-idf-sys`/`esp-idf-svc` dependency or `*-espidf` target in
   `.cargo/config.toml`), `find_host_testable_sibling(workspace_root)` (Design Decision 3),
   `write_builder_dockerfile()` + embedded Dockerfile constant, `dagger_pipeline_args(source_dir,
   builder_dir)` assembling the `fmt`/`clippy`/`build`/conditional-`test` chain (Design Decision
   2), `publish_artifacts(release_client, tag, target_dir)` wrapping two
   `GitHubReleaseClient::upload_asset_with` calls (Design Decision 5) — three-plus-one-function
   shape per spec's Affected Contracts.
2. **`builders/esp32/Dockerfile`** — `rust:1-bookworm` base, `espup install` (Design Decision 6's
   image-build-time counterpart — the actual toolchain fetch happens here, once, not per
   pipeline run), `libclang`/`clang` + `LIBCLANG_PATH` env, `python3`+pip, `cargo install
   espflash`, standard OCI labels matching `builders/java/Dockerfile`'s template.
3. **CLI wiring** — `CiArgs.publish_artifacts: bool` in `crates/paws-cli-core/src/lib.rs`;
   `run_ci_pipeline` gains the `Some("esp32") => { ... }` match arm (detect → error if missing →
   `write_builder_dockerfile()` → `dagger_pipeline_args()` → `run_dagger_core()` → conditionally
   `publish_artifacts()` when the flag is set and the pipeline succeeded); validates
   `--publish-artifacts` requires `--toolchain esp32` before dispatch (mirrors `--coverage`'s
   existing rejection precedent); updates the "expected toolchain values" error message and the
   `run_ci_pipeline` doc comment.
4. **Provisioning** — `Ecosystem::Esp32` variant + `install_esp32()` in `paws-provision`
   (Design Decision 6).
5. **Tests** (Constitution Principle V — pairs with every workstream above): `paws-esp32` unit
   tests for detection (positive/negative), pipeline-arg chain shape with/without a detected host-
   testable sibling, `write_builder_dockerfile()` output; `paws-cli-core` test for the out-of-
   `--toolchain esp32` `--publish-artifacts` rejection; `paws-release`/`paws-esp32` boundary test
   confirming `GitHubReleaseClient` is reachable and usable from the new crate (Constitution I's
   "widened visibility" note from the Constitution Check table).
6. **Builder-registry wiring** — `compose.yml`'s new `esp32` service (template: `tauri-android`'s
   existing block, given comparable image size/toolchain-bundling shape);
   `.github/workflows/release.yaml`'s `build-builders` matrix gains `esp32` on its own runner leg
   (spec's Runtime and Defaults Impact — disk-space isolation, not shared with other builders);
   `builders/README.md` gets a paragraph joining `esp32/` to the
   `tauri-linux`/`tauri-android`/`java`/`flatpak` consumer-project-builder category, following
   that section's existing per-builder write-up depth.
7. **Fixture + docs** — `examples/esp32-fixture` (a minimal real `esp-idf-svc` "blink" project,
   per `examples/README.md`'s existing fixture conventions); `docs/ROADMAP.md`'s new `## Embedded
   (ESP32 / no_std targets)` section (spec's Scope); `paws ci --help`/`--publish-artifacts --help`
   documented via this codebase's doc-comment-is-help-text convention.

## Contract-Safety Checklist

- [x] Workflow declarations and references stay consistent — `.github/workflows/release.yaml`'s
      `build-builders` matrix addition is the only workflow YAML touched, following the exact
      existing pattern for every other builder entry (own runner leg, per Workstream 6)
- [x] Dagger call names align with module `@func()` names — N/A, no Dagger Cloud module involved
      (`paws-dagger` is a subprocess wrapper, matching every other toolchain crate)
- [x] Runtime standards come from a single shared source — `rust:1-bookworm` stays the one pinned
      base tag `builders/esp32/Dockerfile` references, matching `paws-rust`'s/`builders/rust`'s
      existing pin (no second, drifting Rust base image introduced)
- [x] Permissions are explicit and least-privilege — `--publish-artifacts` reuses the existing
      `$GITHUB_TOKEN`/`$GH_TOKEN` `contents: write` model `paws semver --push`/`paws helm
      --publish` already establish; no new permission shape (spec's Security and Permissions
      Impact)
- [x] Security implications are documented — spec's Security and Permissions Impact section
      explicitly flags `--publish-artifacts` as the first `paws ci` flag (not a dedicated
      subcommand) that writes to GitHub, and states why that's an acceptable, precedented reuse
      rather than new surface

## Validation Matrix

| Surface | Validation |
| -------------------------- | ---------- |
| `paws-esp32` detection + pipeline-arg construction | `cargo test -p paws-esp32` (Workstream 5); positive/negative detection, chain-shape tests |
| CLI wiring (`paws-cli-core`) | `cargo test -p paws-cli-core` — out-of-`--toolchain esp32` `--publish-artifacts` rejection test |
| `paws-provision` | `cargo test -p paws-provision` — `Ecosystem::Esp32` recognized, shells to `espup` (mocked/asserted invocation, not a real install in unit tests) |
| `builders/esp32/Dockerfile` | `docker buildx bake -f compose.yml esp32` + `docker buildx imagetools inspect` confirming both GHCR + Docker Hub refs landed (spec's Validation Plan, citing `builders/README.md`'s documented history of this silently failing before) |
| End-to-end dogfooding | `paws ci --toolchain esp32` against `examples/esp32-fixture` AND a real `ha-kiosk` `firmware/` clone (spec's Validation Plan) |
| `--publish-artifacts` | Against a real test repo + tag: assets land, idempotent re-run, missing-token clear error (spec's Validation Plan) |
| Workspace-wide regression | `cargo test --workspace` — zero failures, zero changed expectations in any existing crate's tests (Constitution Principle V) |
