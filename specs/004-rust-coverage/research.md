# Phase 0 Research: `paws ci --toolchain rust --coverage`

Source spec: `specs/004-rust-coverage/spec.md`. Both of the spec's Clarifications (Session
2026-08-23) are resolved; this document resolves the remaining implementation-level unknowns by
reading the actual precedent already in this codebase, per Constitution Principle IV's spirit
(read the real source before guessing).

## R1: Exact `cargo llvm-cov` invocation and output format

**Decision**: `cargo llvm-cov --workspace --summary-only`, run as its own `with-exec` step after
the existing `cargo test --verbose` step (Clarifications).

**Rationale**: this is the exact command already run manually against this repo (see this
session's own coverage report) — it produces the per-crate + `TOTAL` summary table (Regions,
Missed Regions, Cover%, Functions, Lines, Branches) directly to stdout, no extra flags needed.
`--workspace` is safe to pass unconditionally: it's a no-op for a single-crate (non-workspace)
consumer repo — `cargo` commands with `--workspace` on a standalone crate just include that one
crate — so `paws-rust`'s pipeline doesn't need to detect workspace-vs-single-crate before
choosing flags.

**Alternatives considered**: `--html`/`--lcov` output: rejected for this spec — Out of Scope
explicitly defers machine-readable report export to a follow-up (`--coverage-report <path>`);
`--summary-only` is the minimal flag that produces exactly the printed-to-stdout summary this
spec's Scope calls for.

## R2: Builder-image pattern to follow

**Decision**: the `tauri-linux`/`tauri-android`/`java` pattern (`crates/paws-tauri`,
`crates/paws-java`) — a Dockerfile under `builders/rust/`, embedded into the `paws` binary at
compile time via `include_str!`, materialized to a temp directory at pipeline-run time, and built
fresh through Dagger's `host directory --path=<dir> docker-build` chain on every `--coverage` run
(Dagger's own BuildKit layer cache makes repeat runs fast after the first, same as every other
builder in this family — confirmed in `builders/README.md`'s cache-registry-import verification
note). **Not** `paws-release`'s own builder pattern (`linux-gnu`/`macos`/etc.), which is
pull-only against a prebuilt registry tag and fails loudly if that tag is missing — that pattern
exists specifically for building **`paws` itself** across a release matrix; `builders/rust` builds
**consumer** projects, the same category `tauri-linux`/`java` are already in.

**Rationale**: `builders/README.md` draws this exact category line explicitly: "`tauri-linux/`/
`tauri-android/`/`java/` are different — they're what `paws ci --toolchain tauri`/.../`java`/
`kotlin` build *user* projects against ... embedded into the `paws` binary rather than read from
this directory at runtime." `builders/rust` is functionally identical in kind — a `paws ci`
consumer-project builder, not a `paws`-itself release target.

**Design consequence**: `crates/paws-rust` gains the same `write_builder_dockerfile()` /
`RUST_COVERAGE_DOCKERFILE` (`include_str!`) shape `paws-tauri`/`paws-java` already have, and
`dagger_pipeline_args` gains a `coverage: bool` parameter that swaps the pipeline's opening chain
from `container from --address=rust:1-bookworm` to `host directory --path=<builder_dir>
docker-build` when set — mirroring exactly how `paws-tauri`'s/`paws-java`'s own pipeline-args
functions already branch on which opening chain to use.

## R3: `compose.yml` / release-matrix wiring for the new builder

**Decision**: add a `rust` service to `compose.yml` (identical shape to the existing `java`
service: `BUILDER_VERSION`/`BUILDER_REVISION`/`BUILDER_CREATED` build args, a `cache-rust`
registry cache tag, `ghcr.io/mbround18/paws-builders:rust-${VERSION:-dev}` +
`docker.io/mbround18/paws-builders:rust-${VERSION:-dev}` tags), and add `rust` to
`.github/workflows/release.yaml`'s `build-builders` job's `matrix.builder` list.

**Rationale**: `compose.yml`'s own header comment states it "Builds every `./builders/*`
Dockerfile" — every existing builder, including the consumer-project ones (`java`, `tauri-linux`,
`tauri-android`, `flatpak`, `helm`) that `paws` never pulls a prebuilt tag for at runtime, still
gets a `compose.yml` service and a `build-builders` matrix entry. This is CI build-verification
(catching a broken `builders/rust/Dockerfile` before a consumer hits it) plus registry-cache
population (so a cold CI runner still hits BuildKit cache on `docker-build`, per
`builders/README.md`'s verified finding) — not because `paws ci --toolchain rust --coverage`
itself ever pulls the prebuilt tag (it doesn't; see R2).

**Design consequence**: this feature's task list includes `compose.yml` and
`.github/workflows/release.yaml` edits alongside the Rust-crate changes — a slightly wider surface
than spec.md's Affected Contracts section named directly, but one that follows the exact,
already-established "how a new builder gets added" pattern this repo's own `docs/ROADMAP.md`
documents ("How a new stack gets added" / `builders/README.md`).

## R4: `--coverage` CLI gating pattern

**Decision**: reject `--coverage` with a clear error when `--toolchain` isn't `rust` — the same
shape `--targets` already uses for its own `--toolchain go`-only gating (confirmed in
`docs/ROADMAP.md`: "`--targets` is rejected outside `--toolchain go` with a clear error").

**Rationale**: reuses an established, already-shipped UX pattern in the same `CiArgs`/`run_ci`
code path rather than inventing a new one — `--coverage` and `--targets` are both
toolchain-scoped opt-in flags on the same subcommand.

## R5: `--coverage` combined with a wasm Rust project

**Decision**: `--coverage` is silently a no-op on a detected wasm project (`is_wasm_project`) —
the existing wasm pipeline (which already skips `cargo test` entirely, per `paws-rust`'s doc
comment: "a `cdylib` compiled for wasm32 can't run on the host") runs completely unchanged, with
no coverage step appended and no error raised.

**Rationale**: coverage data requires the test binary to actually execute, and the wasm pipeline
already can't do that on the host for the same reason `cargo test` itself is skipped there — this
isn't a new limitation `--coverage` introduces, it's the same one `cargo test` already has on this
path. Erroring would be surprising for a flag that's supposed to be purely additive/opt-in
elsewhere; silently doing nothing matches how every other trigger-gated opt-in flag in this
codebase behaves when its precondition isn't met (e.g. `003-release-parity-docker`'s
`--tag-branch` producing no tag on a non-branch-push build, rather than erroring).

## Summary of new surface

| File | Change |
|---|---|
| `builders/rust/Dockerfile` (new) | `rust:1-bookworm` base + `rustup component add llvm-tools-preview` + `cargo install cargo-llvm-cov`, OCI labels matching `builders/java/Dockerfile`'s shape |
| `crates/paws-rust/src/lib.rs` | `include_str!` embed + `write_builder_dockerfile()`; `dagger_pipeline_args` gains a `coverage: bool` param, swapping the opening chain (R2) and appending a `cargo llvm-cov` step (R1) when set and not a wasm project (R5) |
| `crates/paws-cli-core/src/lib.rs` | `CiArgs` gains `--coverage`; `run_ci` rejects it outside `--toolchain rust` (R4) |
| `compose.yml` | new `rust` service (R3) |
| `.github/workflows/release.yaml` | `rust` added to `build-builders`'s matrix (R3) |
| `builders/README.md` | one-line mention of `rust/` joining the consumer-project builder category |
