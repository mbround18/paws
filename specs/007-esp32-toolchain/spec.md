# Feature Specification: `paws ci --toolchain esp32`

## Summary

Add `esp32` as a new `paws ci --toolchain` value: build/lint/test an ESP-IDF-based Rust firmware
project (the `esp-idf-sys`/`esp-idf-svc` + `embuild` stack, targeting `xtensa-esp32*-espidf` or
`riscv32im*-esp-espidf`) inside a new dedicated `builders/esp32` image (Rust + `espup`-installed
ESP toolchain + `libclang` + `espflash`), the same shape every other multi-toolchain consumer
project builder (`tauri-android`, `flatpak`, `java`) already has. Paired with a second, opt-in
capability on the same pipeline: `--publish-artifacts`, which uploads the built bootloader +
firmware ELF to the GitHub Release matching the current tag, reusing `paws-release`'s existing
`GitHubReleaseClient` rather than inventing a second GitHub-API client. First concrete driver:
`mbround18/ha-kiosk`'s `firmware/` crate, which has no CI today (`cargo check`/`cargo test` are
run by hand, per that repo's `CONTRIBUTING.md`).

## Clarifications

### Session 2026-08-24

- Q: Does "CI and releases" mean build/lint/test only, or does it also mean publishing a
  flashable release artifact to a GitHub Release? → A: Both — this spec covers
  `paws ci --toolchain esp32` (build/lint/test) **and** a new artifact-publishing capability, not
  build/test alone.

## Motivation and Problem Statement

`paws` has no embedded/microcontroller toolchain today — confirmed directly: zero references to
`esp32`, `esp-idf`, `espflash`, `xtensa`, or any `riscv32*` target anywhere in this repo (the
`--toolchain` enum, every `builders/*`, `docs/ROADMAP.md`, and `paws-provision`'s `Ecosystem`
list). A real consumer project (`ha-kiosk`, an ESP32-P4 kiosk firmware) has no CI at all as a
result: no build check on push/PR, no lint gate, and no automated way to attach a flashable
release artifact to a tagged GitHub Release — a maintainer has to run `cargo build --release` and
`espflash flash` by hand every time, with no verification anyone else's change didn't break the
build until they personally try to flash it. This closes that gap the same way `paws ci
--toolchain rust`/`go`/`java` already closes it for their respective ecosystems, plus the
artifact-publishing step a firmware project specifically needs and no existing `paws` toolchain
provides for a *consumer* project (`paws release` only ever cross-compiles `paws`'s own binary,
not a caller's project - confirmed by reading `crates/paws-release/src/lib.rs` in full).

## Scope

### In scope

- A new `esp32` value for `paws ci --toolchain`, following the exact `--toolchain <x>` dispatch
  pattern already used by `rust`/`go`/`java`/`kotlin`/`tauri`/`tauri-android`/`flatpak`
  (`crates/paws-cli-core/src/lib.rs`'s `run_ci_pipeline`).
- Detection (`crates/paws-esp32`, new crate): a project is an ESP32 target when its `Cargo.toml`
  depends on `esp-idf-sys` or `esp-idf-svc`, or its `.cargo/config.toml` sets `build.target` to an
  `*-espidf` triple — mirrors `paws-rust`'s existing `is_wasm`-style marker-file detection, not a
  new detection mechanism.
- A new `builders/esp32` image: `rust:1-bookworm` base, `espup` (the official ESP Rust toolchain
  installer — "shell to the real installer, don't reimplement it", per `docs/ROADMAP.md`'s "How a
  new stack gets added" step 2) installing the Xtensa-patched toolchain and the `riscv32im*-esp-
  espidf` targets, `libclang`/`clang` (for `esp-idf-sys`'s `bindgen` step, `LIBCLANG_PATH` set as
  a Dockerfile `ENV` rather than the runtime auto-detection `ha-kiosk`'s own `flasher` crate needs
  locally — a container can just install the library properly), `python3` + pip (ESP-IDF's own
  `embuild`-driven CMake/component build shells out to Python), and `espflash` (`cargo install
  espflash`) for the packaging step below. Joins the `tauri-android`/`flatpak`/`java` "needs
  multiple toolchains combined, no single public image provides it" category from `docs/
  ROADMAP.md`'s builder-image policy.
- Pipeline steps (mirrors `paws-rust`'s existing chain, adjusted for the ESP-IDF build system):
  `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo build --release` (the actual
  `embuild`-driven ESP-IDF + Rust cross-compile). No `cargo test --workspace` step against the
  embedded target itself — `ha-kiosk`'s own `CONTRIBUTING.md` already documents why
  (`firmware`'s `[[bin]]` has `harness = false`; real tests live in the separate host-target
  `firmware-core` crate). Detects a sibling host-testable crate the same way `paws-rust` already
  runs a workspace's non-embedded members, and runs `cargo test` against *that* crate specifically
  when one exists, rather than against the embedded target.
- A new opt-in `--publish-artifacts` flag on `paws ci`, valid only with `--toolchain esp32`
  (mirrors `--coverage`'s existing `--toolchain rust`-only gating precedent from
  `specs/004-rust-coverage`). When set and the build succeeds, uploads the built bootloader
  (`bootloader.bin`) and firmware ELF as assets on the GitHub Release matching the current tag
  (`$GITHUB_REF`/an explicit `--tag`), reusing `paws-release`'s `GitHubReleaseClient::
  get_or_create_release`/`upload_asset_with` rather than a second GitHub-API client.
- `docs/ROADMAP.md`: a new `## Embedded (ESP32 / no_std targets)` section (own subsection, not
  shoehorned into "Other language ecosystems"'s table — that table's Output Type column doesn't
  fit "flashable firmware binary + bootloader", the same reasoning that already gave Tauri/Android
  its own callout rather than a table row).
- `examples/esp32-fixture`: a minimal real `esp-idf-svc` "blink" project (the ROADMAP explicitly
  requires every stack be "verified for real, end to end" against something real, not a synthetic
  unit-test-only fixture).

### Out of scope

- Actually flashing a physical device from CI — `espflash` in the builder image is for
  *packaging* (`espflash save-image` / producing the `.bin` artifacts to upload), not
  `espflash flash`, which needs a real USB-attached board no CI runner has.
- `xtensa-esp32s3`/other Xtensa-family targets beyond what `espup install` covers by default in
  this first cut — `espup`'s default install already covers the common Xtensa + RISC-V ESP32
  variants; narrowing to a specific chip list is a non-goal here.
- QEMU-based smoke-testing of the built firmware (running it, not just building it) — no
  `paws`/Dagger QEMU integration exists today for any toolchain; a real follow-up, not bundled
  into this first cut.
- Any change to `paws release`'s own cross-compile matrix — that subcommand stays scoped to
  `paws`'s own binary; ESP32 support is entirely new surface under `paws ci`, not an addition to
  `paws release`.
- `no_std` (non-`esp-idf`, bare-metal `esp-hal`) firmware projects — this spec's detection and
  builder image target the `esp-idf-sys`/`embuild` stack specifically, matching `ha-kiosk`'s own
  firmware; a bare-metal `esp-hal` project has a different (simpler - no ESP-IDF/Python/CMake)
  toolchain shape and is a plausible fast-follow, not bundled here.
- Non-GitHub release targets for `--publish-artifacts` (GitLab, a private artifact store, etc.) —
  matches every other `paws` publish path's GitHub-first precedent (`paws semver --push`,
  `paws helm --publish`).

## Affected Contracts

- **`paws ci` CLI contract**: a new `--toolchain esp32` enum value (additive) and a new
  `--publish-artifacts` flag gated to `--toolchain esp32` only (additive, same shape as
  `--coverage`'s `--toolchain rust` gating) — no existing `--toolchain` value's behavior changes.
- **New `paws-esp32` crate contract**: `is_esp32_project(dir)`, `write_builder_dockerfile()`,
  `dagger_pipeline_args(source_dir, builder_dir)` — same three-function shape as
  `paws-kotlin`/`paws-java`, for a future reader's discoverability (grepping for
  `write_builder_dockerfile` finds every builder-embedding crate, this one included).
- **`paws-release`'s `GitHubReleaseClient` becomes a shared dependency**, not `paws-release`-
  private — `crates/paws-esp32` depends on it directly for `--publish-artifacts`. This is a
  visibility widening (`pub` reuse across crates) worth calling out explicitly since every other
  consumer of that type today is `paws-release` itself; no behavior change to `paws-release`'s own
  existing callers.
- **No `gh-reusable` contract to stay in parity with** — no ESP32/embedded function exists there
  to port (confirmed absent, same as `004-rust-coverage`'s coverage step); this is new,
  `paws`-native capability.
- **`builders/README.md`/`compose.yml`/`.github/workflows/release.yaml`**: `esp32` joins the
  `build-builders` matrix and the consumer-project-builder category list, following the exact
  registration pattern every other `builders/*` entry already has.

## Runtime and Defaults Impact

- No new `PipelineDefaults` fields for the base `esp32` toolchain path — matches `--coverage`'s
  precedent (a CLI-flag-gated pipeline branch needs no shared runtime default).
- `--publish-artifacts` needs `$GITHUB_TOKEN`/`$GH_TOKEN` (already the standard env `paws`
  reads for every other GitHub-Release-touching path - `paws semver --push`, `paws helm
  --publish`) and `$GITHUB_REPOSITORY` — no new env var name introduced, reusing the existing
  convention rather than adding a fourth way to say "the token."
- Container implications: a new, large (`espup`'s toolchain + ESP-IDF checkout + Python env -
  multi-GB, comparable to `tauri-android`'s baked-in SDK/NDK) `builders/esp32` image. Per
  `builders/README.md`'s already-documented history, this **must** build on its own runner in the
  `build-builders` matrix (one runner per builder) — building it alongside others on a single
  runner is the exact "no space left on device" failure already hit and fixed for the existing
  builders, and there's no reason to expect an ESP-IDF-sized image to be exempt.
- Default (no `--publish-artifacts`) `paws ci --toolchain esp32` pipeline has no GitHub API
  interaction at all — build/lint/test only, same "additive flag changes nothing by default"
  shape as `--coverage`.

## Security and Permissions Impact

- `--publish-artifacts` is the first `paws ci` flag (as opposed to a dedicated subcommand like
  `paws semver --push`/`paws helm --publish`) that writes to GitHub on the caller's behalf — worth
  flagging explicitly rather than treating as "just like coverage." It reuses the exact same
  token/permission model those existing subcommands already use (a `repo`-scoped
  `$GITHUB_TOKEN`/`$GH_TOKEN` with `contents: write` in Actions), not a new permission shape.
- No new secret *names* introduced (see Runtime and Defaults Impact) — reduces the chance of a
  consumer repo needing to configure yet another differently-named token.
- `espup`'s installer and ESP-IDF's own toolchain download happen at *image build time*
  (`builders/esp32`), not per-pipeline-run — a compromised or unreachable upstream at
  pipeline-run time can't affect a build using the already-published image, matching every other
  builder's trust boundary.
- No scanner/policy behavior change — `paws audit` is unrelated to this feature (unless/until a
  follow-up adds ESP32 to `paws-audit`'s `LanguageFamily` detection list, explicitly out of scope
  here).

## Validation Plan

- `paws ci --toolchain esp32`, run for real against `examples/esp32-fixture` (this repo's own new
  fixture) and, per this feature's own stated driver, against a real clone of `ha-kiosk`'s
  `firmware/` crate — matches every existing toolchain's "verified for real, end to end" bar, not
  fixture-only.
- `cargo fmt --check`/`cargo clippy -- -D warnings` genuinely fail the pipeline on a fixture with
  a deliberately unformatted file / a deliberate clippy lint — proves the gates are real, not
  pass-through.
- `cargo build --release` genuinely produces `bootloader.bin` + a firmware ELF in the fixture's
  target directory — proves the ESP-IDF/`embuild` cross-compile actually completes inside the
  container, not just that `cargo` exits 0 on a trivial host-target build.
- `--publish-artifacts` against a real (test) GitHub repo + tag: asset(s) actually appear on the
  matching Release, re-running is idempotent (existing same-named asset replaced, not duplicated -
  matches `upload_asset_with`'s existing `SkipIfExisting`/replace semantics), and a missing
  `$GITHUB_TOKEN` fails with a clear, actionable error rather than a bare API 401.
- `--publish-artifacts` combined with any `--toolchain` other than `esp32` fails with a clear
  error (mirrors `--coverage`'s existing out-of-`--toolchain rust` rejection message shape).
- `docker buildx bake -f compose.yml esp32` builds successfully and both registry pushes
  (GHCR + Docker Hub) actually land — re-verified via `docker buildx imagetools inspect`, per
  `builders/README.md`'s documented history of both refs *silently* failing to land in the past.
- `cargo test --workspace` continues to pass with zero failures workspace-wide (Constitution
  Principle V) for `paws`'s own repo.

## Rollout and Rollback

- Ships as a pure additive `--toolchain` value + opt-in flag — zero impact on any existing
  `paws ci` caller using a different `--toolchain`.
- If a regression is found in the `esp32` path, a consumer stops passing `--toolchain esp32` (or
  `--publish-artifacts`) — no `paws` version rollback required, no other toolchain affected.
- `ha-kiosk` adopting this is the real-world proof of adoption this spec is written against
  (matches `docs/mbround18.md`'s existing per-repo adoption tracker pattern) - once merged, wiring
  `ha-kiosk`'s own `.github/workflows/` to call `paws ci --toolchain esp32` (and, on tag push,
  `--publish-artifacts`) is a follow-up change in that repo, not part of this spec's scope.
- A bare-metal `esp-hal` (`no_std`, non-`esp-idf`) toolchain variant, QEMU smoke-testing, and
  `paws-audit` detection are each independent, separately-scoped follow-ups (see Out of scope) -
  this spec deliberately doesn't pre-commit their design.
