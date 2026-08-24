# Tasks: `paws ci --toolchain rust --coverage`

**Input**: Design documents from `/home/mbruno/development/paws/specs/004-rust-coverage/`
(`plan.md`, `spec.md`, `research.md`, `data-model.md`, `contracts/`, `quickstart.md`)

## Rules

- Keep changes backward-compatible unless explicitly declared breaking (omitting `--coverage`
  MUST produce byte-identical `dagger_pipeline_args` output to today).
- Pair contract changes with tests in the same PR (Constitution Principle V).
- Keep docs/examples (`docs/ROADMAP.md`, `builders/README.md`, `--help` text) in sync with
  behavior (Development Workflow).
- `cargo test --workspace` MUST pass with zero failures before this feature is considered done.

## Phase 1: Setup

- [X] T001 Create `builders/rust/Dockerfile`: `rust:1-bookworm` base, `rustup component add llvm-tools-preview`, `cargo install cargo-llvm-cov`, `BUILDER_VERSION`/`BUILDER_REVISION`/`BUILDER_CREATED` `ARG`s + OCI `LABEL`s matching `builders/java/Dockerfile`'s exact template shape (contracts/paws-ci-coverage-contract.md §3)

## Phase 2: Foundational (blocking prerequisites)

**Purpose**: crate-level scaffolding the single user story below needs before its own tests/CLI wiring can be written — must complete before Phase 3.

- [X] T002 Add a `RUST_COVERAGE_DOCKERFILE` `include_str!("../../../builders/rust/Dockerfile")` constant and a `write_builder_dockerfile()` function (mirrors `paws-tauri`'s/`paws-java`'s own same-named function shape) in `crates/paws-rust/src/lib.rs` (research.md R2)
- [X] T003 Extend `dagger_pipeline_args`'s signature to accept `coverage: bool` and `builder_dir: Option<&str>` trailing parameters in `crates/paws-rust/src/lib.rs` — stub the new branch as a no-op for now (both default to today's exact behavior); data-model.md's extended signature

**Checkpoint**: `cargo build -p paws-rust` succeeds; every existing `paws-rust` test still passes with the new parameters defaulted off.

---

## Phase 3: User Story 1 - `paws ci --toolchain rust --coverage` (Priority: P1)

**Goal**: `paws ci --toolchain rust --coverage` builds against a new `builders/rust` image, runs the existing `fmt`/`clippy`/`build`/`test` sequence unchanged, then appends a `cargo llvm-cov` coverage summary — all opt-in, byte-identical default behavior when omitted.

**Independent Test**: run `paws ci --toolchain rust --coverage` against this repo; confirm the existing pipeline output is followed by a non-trivial coverage summary table. Run `paws ci --toolchain rust` (no `--coverage`) and confirm output is unchanged from before this feature. Run `paws ci --toolchain node --coverage` and confirm a clear rejection error.

- [X] T004 [P] [US1] Unit test: `dagger_pipeline_args` with `coverage=false`, `builder_dir=None` produces byte-identical output to the pre-feature baseline (regression, mirrors existing `pipeline_uses_the_rust_bookworm_image`/`pipeline_runs_the_full_fmt_clippy_build_test_sequence_in_order` assertions) in `crates/paws-rust/src/lib.rs`
- [X] T005 [P] [US1] Unit test: `coverage=true` on a non-wasm project appends `cargo,llvm-cov,--workspace,--summary-only` as a new step after `cargo,test,--verbose`, with `cargo test --verbose` itself unchanged (Clarifications, Session 2026-08-23) in `crates/paws-rust/src/lib.rs`
- [X] T006 [P] [US1] Unit test: `coverage=true` swaps the opening chain to `host,directory,--path=<builder_dir>,docker-build` instead of `container,from,--address=rust:1-bookworm` (contracts §2, research.md R2) in `crates/paws-rust/src/lib.rs`
- [X] T007 [P] [US1] Unit test: `coverage=true` on a wasm project (`is_wasm=true`) is a no-op — output identical to `coverage=false` on the same wasm project, no coverage step, no error (research.md R5) in `crates/paws-rust/src/lib.rs`
- [X] T008 [US1] Implement the `coverage` branch in `dagger_pipeline_args`: opening-chain swap, appended `cargo llvm-cov` step, wasm no-op — makes T004–T007 pass, in `crates/paws-rust/src/lib.rs`
- [X] T009 [P] [US1] Unit test: `--coverage` combined with any `--toolchain` other than `rust` fails with a clear, non-zero-exit error before any pipeline runs (research.md R4, mirrors `--targets`'s existing out-of-`--toolchain go` rejection) in `crates/paws-cli-core/src/lib.rs`
- [X] T010 [US1] Add `--coverage` flag to `CiArgs`, wire `run_ci`'s toolchain-gating validation, and dispatch `write_builder_dockerfile()`'s result into `dagger_pipeline_args`'s new params only when `coverage` is set — makes T009 pass, in `crates/paws-cli-core/src/lib.rs`

**Checkpoint**: `paws ci --toolchain rust --coverage` works end-to-end; default (no-flag) behavior is provably unchanged.

---

## Phase 4: Polish & Cross-Cutting Concerns

- [X] T011 [P] Add a `rust` service to `compose.yml`, matching the existing `java` service's shape exactly (`BUILDER_VERSION`/`BUILDER_REVISION`/`BUILDER_CREATED` args, `cache-rust` registry cache tag, GHCR + Docker Hub tags) (research.md R3)
- [X] T012 [P] Add `rust` to `.github/workflows/release.yaml`'s `build-builders` job's `matrix.builder` list (research.md R3)
- [X] T013 [P] Add a one-line mention of `rust/` joining the `tauri-linux`/`tauri-android`/`java` consumer-project-builder category in `builders/README.md`
- [X] T014 [P] Update `docs/ROADMAP.md`'s "Test coverage reporting" table: Rust row moves from 📋 to ✅, with a short note on what landed
- [X] T015 Run `cargo test --workspace` and confirm zero failures and zero changed expectations in any pre-existing `paws-rust` test (spec Validation Plan; Constitution Principle V)
- [X] T016 Run quickstart.md's end-to-end dogfooding scenario (`paws ci --toolchain rust --coverage` against `paws`'s own repo) and the incomplete-coverage fixture proof (quickstart.md §4). Ran for real once manually: `paws ci --toolchain rust --coverage` against this repo completed successfully end-to-end (fmt/clippy/build/test unchanged, `cargo llvm-cov` summary appended, 69.39%/71.30%/70.41% region/function/line totals). Along the way found and fixed one pre-existing, unrelated `cargo fmt --check` failure in `crates/paws-node/src/lib.rs` that blocked *any* `--toolchain rust` run, coverage or not — a pure rustfmt formatting fix, no logic change, `paws-node`'s own test suite unaffected (18/18 still pass). The incomplete-coverage-fixture proof is now a real, automated, repeatable test — see T018.
- [X] T018 [P] [US1] Add `examples/rust-coverage-fixture/` (a crate with one deliberately untested branch) and `crates/paws-rust/tests/e2e_coverage.rs` (docker-gated, mirrors `paws-docker`'s own `tests/e2e_docker_daemon.rs` pattern): builds `builders/rust/Dockerfile`, runs `cargo llvm-cov --workspace --summary-only` against the fixture inside it, and asserts the reported coverage is genuinely between 0% and 100% — proving the tool measures a real gap, not a fixed number, as part of `cargo test --workspace` rather than only a one-off manual run
- [X] T017 [P] Verify `builders/rust/Dockerfile` builds cleanly via `docker buildx bake -f compose.yml rust` (or an equivalent local build), independent of a live `paws ci --coverage` run (quickstart.md "Definition of done")

## Dependencies & Execution Order

- **Phase 1 (Setup)** → **Phase 2 (Foundational)**: T001 (the Dockerfile) must exist before T002 can `include_str!` it. T002/T003 are otherwise independent of each other (different concerns, same file — coordinate on merge order).
- **Phase 3 (US1)**: T004–T007 (tests) can be written in parallel once Phase 2 lands, since they exercise `dagger_pipeline_args` from the outside. T008 (implementation) makes them pass. T009 is independent of T004–T008 (different crate, `paws-cli-core` vs `paws-rust`) and can be written in parallel with them; T010 depends on T008 being done (it dispatches into `dagger_pipeline_args`'s new params) as well as making T009 pass.
- **Phase 4 (Polish)**: T011–T014 are independent of each other and of Phase 3's code (different files entirely — `compose.yml`, workflow YAML, two docs files) and can start as soon as Phase 1's `builders/rust/Dockerfile` exists (T001), in parallel with Phase 2/3. T015–T017 depend on Phase 3 being complete.

```
Setup (T001)
    ↓
Foundational (T002-T003)
    ↓
US1 (T004-T010) ──────────────┐
                               ├── Polish tail (T015-T017)
Polish head (T011-T014) ───────┘   (can start right after T001, parallel to Foundational/US1)
```

## Parallel Execution Examples

- **Within Phase 3**: T004, T005, T006, T007 (all test tasks) can be written in parallel before T008 lands, since they exercise `dagger_pipeline_args` from the outside and don't depend on each other's code. T009 can be written in parallel with all of them (different crate).
- **Across phases**: T011–T014 (Polish head: `compose.yml`, `release.yaml`, two docs files) only depend on T001 (the Dockerfile existing), not on any of Phase 2/3's Rust code — a second contributor/session can start these as soon as T001 merges, in parallel with Phase 2/3.

## Implementation Strategy

**MVP scope**: this feature is small enough that the full User Story 1 (T001–T010) is the natural single unit of delivery — there's no meaningful smaller slice that's independently useful (a `--coverage` flag that doesn't yet build the coverage-capable image, or an image with no flag wired to it, is not shippable on its own).

**Incremental delivery**:
1. Ship Setup + Foundational + User Story 1 (T001–T010) as one PR — the complete, working feature.
2. Ship Polish (T011–T017) either in the same PR or a fast-follow — the `compose.yml`/`release.yaml` registration doesn't block `paws ci --toolchain rust --coverage` from working locally (it always builds `builders/rust/Dockerfile` fresh via Dagger regardless of whether a prebuilt tag exists anywhere), so it's safe to land slightly after the core feature if needed, but should not be dropped — it's what keeps CI verifying the Dockerfile itself and populating the build cache for real users.
