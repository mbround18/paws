# Implementation Plan: `paws ci --toolchain rust --coverage`

## Inputs

- Spec path: `specs/004-rust-coverage/spec.md`
- Affected contracts/files:
  - `builders/rust/Dockerfile` (new)
  - `crates/paws-rust/src/lib.rs` (`dagger_pipeline_args` extended, new `write_builder_dockerfile`)
  - `crates/paws-cli-core/src/lib.rs` (`CiArgs.coverage`, `run_ci` gating + wiring)
  - `compose.yml` (new `rust` service)
  - `.github/workflows/release.yaml` (`build-builders` matrix gains `rust`)
  - `builders/README.md` (mention `rust/` joining the consumer-project builder category)
  - `docs/ROADMAP.md` (already updated during spec/clarify — coverage table + Rust pilot note)
  - Phase 0/1 artifacts: `research.md`, `data-model.md`, `contracts/paws-ci-coverage-contract.md`, `quickstart.md`

## Constitution Check

_GATE: evaluated before Phase 0 research; re-evaluated below after Phase 1 design._

| Principle | Assessment |
|---|---|
| I. One Crate Per Domain | No new crate — this is a `paws-rust`-domain pipeline extension plus `paws-cli-core` wiring, matching how `--targets` (a `paws-go`-domain flag) was added without a new crate. `paws-cli-core` stays thin: gating logic (`--coverage` requires `--toolchain rust`) belongs there since it's cross-toolchain validation, not Rust-specific domain logic. **PASS**. |
| II. Subprocess-First Dagger Access | All Dagger interaction goes through `paws_dagger::core` (unchanged call site pattern) — no new `Command::new("dagger")`. **PASS**. |
| III. Incremental SDK Adoption | Not applicable — no Dagger SDK involvement. **PASS (not applicable)**. |
| IV. Parity Testing Over Reimplementation-From-Memory | Not a `gh-reusable` port (spec Summary/Affected Contracts) — `rustBuildAndTest` has no coverage step to port. New, `paws`-native capability; disclaimed explicitly rather than implied. **PASS**. |
| V. Reliability & Testability First | Every new branch (`coverage` on/off, wasm/non-wasm × coverage on/off, out-of-toolchain rejection) gets a named unit test in Workstream 4 below. **PASS**, contingent on tasks.md enumerating each one. |
| Tech constraint: no secrets on CLI | Not applicable — no secret-bearing input anywhere in this feature. **PASS**. |
| Tech constraint: shared defaults live in one place | No new shared default needed — `coverage`'s only "default" is `false`, expressed once in `CiArgs`' `#[serde(default)]`, no `PipelineDefaults` field warranted (nothing else needs to know this default). **PASS**. |
| Tech constraint: no swallowed concurrent failures | Not applicable — no concurrent orchestration in this feature. **PASS (not applicable)**. |

**Pre-Phase 0 Gate Status**: PASS, no unresolved conflicts.
**Post-Phase 1 Gate Status**: PASS — Phase 1 design (data-model.md, contracts/) surfaced the
`compose.yml`/`release.yaml` builder-registration surface (research.md R3), which is wider than
spec.md's Affected Contracts named directly, but doesn't change any Constitution assessment above.

## Design Decisions

1. **`dagger_pipeline_args` gains two additive trailing parameters (`coverage: bool`,
   `builder_dir: Option<&str>`), not a second parallel function.** Matches
   `003-release-parity-docker`'s own precedent (`generate_tags` → `generate_tag_matrix`) for
   *internal* restructuring versus a *new public function* — here the simpler case, since there's
   no existing-signature backward-compatibility constraint to protect (this crate's only caller,
   `run_ci`, is in the same workspace and updated in the same change). Alternative considered:
   a `RustCiOptions` struct bundling `is_wasm`/`coverage`/`builder_dir` — rejected as premature for
   three fields, matching this crate's own existing two-bool-parameter shape.

2. **`builders/rust` follows the `tauri-linux`/`java` embed-and-rebuild-every-run pattern, not
   `paws-release`'s pull-only prebuilt pattern** (research.md R2) — directly resolves the spec's
   Clarifications-Session-2026-08-23 decision. `write_builder_dockerfile()` is named identically
   to its `paws-tauri`/`paws-java` counterparts for discoverability (a future reader grepping for
   that name across crates finds every builder-embedding crate at once).

3. **`compose.yml`/`release.yaml` gain a `rust` builder entry** (research.md R3) — wider than
   spec.md's Affected Contracts named directly, but required for this builder to follow the same
   CI-verified, cache-populated path every other consumer-project builder already has. Called out
   explicitly here rather than silently expanding scope mid-implementation.

4. **`--coverage` is a silent no-op on a wasm project, not an error** (research.md R5) — a
   judgment call made during planning (not escalated to a third clarification question) because
   it directly mirrors an existing, already-shipped precedent in this exact crate (`is_wasm`
   already skips `cargo test` for a documented, analogous reason) and in
   `003-release-parity-docker` (`--tag-branch` on a non-branch-push build). If this turns out
   surprising in practice, it's a one-line change to make it error instead — not a design decision
   worth blocking implementation on.

5. **`CiArgs.coverage` needs no `PipelineDefaults` entry.** Unlike `003-release-parity-docker`'s
   `changelog_path` (a value another code path also needed to know), `coverage`'s default (`false`)
   is only ever consulted at the single `run_ci` call site — no second consumer, so no shared-state
   need per the constitution's "shared defaults live in one place" constraint (that constraint
   protects against *duplicated* hardcoding, not every boolean flag needing a `PipelineDefaults`
   entry).

## Workstreams

1. **`builders/rust/Dockerfile`** — new file, `rust:1-bookworm` base + `llvm-tools-preview` +
   `cargo-llvm-cov`, OCI labels matching `builders/java/Dockerfile`'s template exactly
   (contracts/paws-ci-coverage-contract.md §3).
2. **`paws-rust` pipeline extension** — `write_builder_dockerfile()` + embedded Dockerfile
   constant; `dagger_pipeline_args` gains `coverage`/`builder_dir` params, conditional opening
   chain (research.md R2), appended `cargo llvm-cov` step (research.md R1), wasm no-op (research.md
   R5).
3. **CLI wiring** — `CiArgs.coverage` in `crates/paws-cli-core/src/lib.rs`; `run_ci` validates
   `--coverage` requires `--toolchain rust` (research.md R4) before dispatch; threads
   `write_builder_dockerfile()`'s result through to `dagger_pipeline_args` only when `coverage` is
   set (no builder-image cost paid on the default path).
4. **Tests** (Constitution Principle V — pairs with every workstream above): `paws-rust` unit
   tests for byte-identical default output with the new params defaulted off, coverage step
   appended in the right position for a non-wasm project, wasm-project no-op, and opening-chain
   swap to `docker-build` when `coverage` is set; `paws-cli-core` test for the out-of-`--toolchain
   rust` rejection error.
5. **Builder-registry wiring** — `compose.yml`'s new `rust` service (research.md R3, template:
   `java`'s existing block); `.github/workflows/release.yaml`'s `build-builders` matrix gains
   `rust`; `builders/README.md` gets a one-line mention joining `rust/` to the
   `tauri-linux`/`tauri-android`/`java` consumer-project-builder category.
6. **Docs** — `docs/ROADMAP.md`'s coverage table row for Rust moves from 📋 to ✅ once implemented
   (already scaffolded during spec/clarify); `paws ci --help` documents `--coverage`'s scope and
   `--toolchain rust`-only gating via its own doc comment (this codebase's established
   doc-comment-is-help-text convention, confirmed in `003-release-parity-docker`).

## Contract-Safety Checklist

- [x] Workflow declarations and references stay consistent — `.github/workflows/release.yaml`'s
      `build-builders` matrix addition is the only workflow YAML touched, following the exact
      existing pattern for every other builder entry
- [x] Dagger call names align with module `@func()` names — N/A, no Dagger module (`paws-dagger`
      is a subprocess wrapper, not a Dagger Cloud module with named functions)
- [x] Runtime standards come from a single shared source — `rust:1-bookworm` stays the one pinned
      base tag both the default pipeline and `builders/rust/Dockerfile` reference (Design
      Decision 2/5) — no second, drifting pin introduced
- [x] Permissions are explicit and least-privilege — no new secrets/permissions anywhere in this
      feature (spec's Security and Permissions Impact); `build-builders`' existing GHCR/Docker Hub
      push credentials already cover a `rust` matrix entry, no new credential needed
- [x] Security implications are documented — spec's Security and Permissions Impact section
      states there are none; this plan introduces no new surface beyond what that section covers

## Validation Matrix

| Surface | Validation |
| -------------------------- | ---------- |
| `paws-rust` pipeline-arg construction | `cargo test -p paws-rust` (Workstream 4); byte-identical-default regression test for the new params |
| CLI wiring (`paws-cli-core`) | `cargo test -p paws-cli-core` — out-of-`--toolchain rust` rejection test |
| `builders/rust/Dockerfile` | `docker buildx bake -f compose.yml rust` (or equivalent local build) — quickstart.md §"Definition of done" |
| End-to-end dogfooding | `paws ci --toolchain rust --coverage` against `paws`'s own repo (quickstart.md §2), and against a deliberately-incomplete fixture (quickstart.md §4) |
| Workspace-wide regression | `cargo test --workspace` — zero failures, zero changed expectations in any existing `paws-rust` test (Constitution Principle V) |
