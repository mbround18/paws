# Tasks: Close Remaining Gaps Found Migrating `valheim-docker`

**Input**: Design documents from `/home/mbruno/development/paws/specs/005-close-remaining-cli/`
(`plan.md`, `spec.md`, `research.md`, `data-model.md`, `contracts/`, `quickstart.md`)

**Context carried into this task list**: not everyone wants to rely on Dagger Cloud. `CacheBackend::GitHubActionsCache`
(no external account needed) gets equal or greater task weight than `CacheBackend::DaggerCloud`
(detection + a log line only, research.md R6) — it is the provider most GitHub Actions-only
consumers will actually depend on, not a secondary/fallback path just because it happens to need
more code.

## Rules

- Keep changes backward-compatible unless explicitly declared breaking (FR-001/Gap 2 is the one
  deliberate exception — see spec's Risks and Mitigations).
- Pair contract changes with tests in the same PR (Constitution Principle V).
- Keep docs/examples (`docs/ROADMAP.md`, `--help` text) in sync with behavior.
- `cargo test --workspace` MUST pass with zero failures before this feature is considered done.
- The four gaps are independent (spec's own Rollout section: "none of the four's rollback affects
  the others") — phases below are ordered by spec priority (P1/P2/P2/P3), not by a dependency
  chain between them.

## Phase 1: Setup

- [X] T001 Add `reqwest.workspace = true` to `crates/paws-dagger/Cargo.toml` (needed by `CacheBackend::GitHubActionsCache`'s Actions Cache REST API calls, research.md R7)

## Phase 2: Foundational

No cross-story blocking prerequisites — the four gaps touch disjoint crates (`paws-rust`,
`paws-dagger`, `paws-audit`, `paws-release`/`paws-docs`) with no shared plumbing beyond T001.
Proceed directly to the story phases.

---

## Phase 3: User Story 1 - `paws ci --toolchain rust` actually fails on a clippy warning (Priority: P1)

**Goal**: the non-wasm `cargo clippy` invocation gains `-- -D warnings`, matching the wasm path's
existing gate exactly.

**Independent Test**: run `paws ci --toolchain rust` against a fixture crate with one deliberate
clippy warning; assert non-zero exit where before this fix it exited 0.

- [X] T002 [P] [US1] Unit test: `dagger_pipeline_args`'s non-wasm branch includes `--args=cargo,clippy,--,-D,warnings` in `crates/paws-rust/src/lib.rs`
- [X] T003 [P] [US1] Fixture test: a temp crate with a real clippy warning fails `cargo clippy -- -D warnings` when invoked directly via `Command::new("cargo")` (proves the gate itself, independent of the dagger-pipeline string-assertion test) in `crates/paws-rust/src/lib.rs`
- [X] T004 [US1] Add `"--", "-D", "warnings"` to the non-wasm branch's `cargo clippy` `push_exec` call in `crates/paws-rust/src/lib.rs` (research.md R1) — makes T002 pass
- [X] T005 [P] [US1] Unit test: a clean, warning-free fixture continues to pass unchanged (SC-002 — zero false positives) in `crates/paws-rust/src/lib.rs`

**Checkpoint**: `paws ci --toolchain rust` now fails on a real clippy warning on the default (non-wasm) path.

---

## Phase 4: User Story 2 - Dagger build cache survives across separate CI runs (Priority: P2)

**Goal**: `paws docker`/`paws ci`'s Dagger invocation gains automatic, environment-detected cache
reuse via two `CacheBackend` providers — `DaggerCloud` (near-zero code, needs a paid account) and
`GitHubActionsCache` (real implementation work, no external account needed — the one most
consumers will actually use).

**Independent Test**: run `paws docker` twice in separate environments against an unchanged
Dockerfile with each provider configured in turn; assert the second run reuses cached layers and
completes materially faster, and that no-backend-configured behavior is unchanged from today.

- [X] T006 [P] [US2] Define the `CacheBackend` enum (`DaggerCloud`/`GitHubActionsCache`/`None`) in `crates/paws-dagger/src/lib.rs` (data-model.md)
- [X] T007 [US2] Implement `CacheBackend::detect()` with fixed precedence — `DAGGER_CLOUD_TOKEN` first, then `ACTIONS_CACHE_URL`+`ACTIONS_RUNTIME_TOKEN` (the legacy Cache Service v1 REST API this crate implements — deliberately not `ACTIONS_RESULTS_URL`'s newer Twirp/protobuf service, which falls through to `None`; tracked in docs/ROADMAP.md), else `None` — in `crates/paws-dagger/src/lib.rs` (research.md R6/R7, Design Decision 4)
- [X] T008 [P] [US2] Unit test: `detect()` picks `DaggerCloud` when both signatures present (precedence), `GitHubActionsCache` when only the Actions signature is present, `None` when neither is present, in `crates/paws-dagger/src/lib.rs`
- [X] T009 [US2] Add the "which backend was selected" log line (FR-006) — implemented as `CacheBackend::log_line()`, emitted once at the top of `paws_dagger::restore_cache_backend()` (see corrected T015 below), not inside `detect()` itself — in `crates/paws-dagger/src/lib.rs`
- [X] T010 [P] [US2] Unit test: the log line names the correct backend for each of the three `detect()` outcomes, in `crates/paws-dagger/src/lib.rs`
- [X] T011 [US2] Implement `GitHubActionsCache`'s restore step (GET against the Actions Cache REST API using `ACTIONS_CACHE_URL`/`ACTIONS_RUNTIME_TOKEN` auth, restoring into the local Dagger engine's persistent storage path) in `crates/paws-dagger/src/lib.rs` — this is the primary implementation task for the account-free provider (research.md R7)
- [X] T012 [US2] Implement `GitHubActionsCache`'s save step (PUT/POST against the same API, saving the engine's storage path back after a pipeline completes) in `crates/paws-dagger/src/lib.rs`
- [X] T013 [P] [US2] Unit test: `GitHubActionsCache` restore/save construct the correct Actions Cache REST API requests (URL, auth headers, cache key) against a fixture, in `crates/paws-dagger/src/lib.rs`
- [X] T014 [P] [US2] Unit test: `GitHubActionsCache` is not selected (falls through to `None`) when running outside GitHub Actions (no `ACTIONS_CACHE_URL`/`ACTIONS_RUNTIME_TOKEN`), in `crates/paws-dagger/src/lib.rs`
- [X] T015 [US2] **Corrected during implementation** (see plan.md/this file's header note — real Dagger-engine internals verified against a live install show `GitHubActionsCache`'s save step must stop the shared engine container for a consistent snapshot, which every `paws_dagger::core`/`core_streaming` call cannot tolerate, since a single `paws` invocation calls those many times — e.g. `paws audit` running several scanners in sequence, or `paws ci`'s own fmt/clippy/build/test steps): wire `CacheBackend::detect()` plus `GitHubActionsCache`'s restore-before/save-after bracketing into new explicit `paws_dagger::restore_cache_backend()`/`paws_dagger::save_cache_backend()` entry points instead, called exactly once each from `paws-cli-core::run_ci`/`run_docker` (bracketing the *entire* invocation, not each individual `core`/`core_streaming` call) — `core`/`core_streaming` themselves stay byte-identical to before this feature (FR-007), preserving Constitution Principle II's "exclusively via `paws_dagger::call`/`core`/`core_streaming`" subprocess-access guarantee without also forcing this feature's lifecycle scope onto every caller of those functions
- [X] T016 [P] [US2] Unit test: `DaggerCloud` selected + `DAGGER_CLOUD_TOKEN` reaches the `dagger` subprocess via inherited environment (confirms research.md R6, no new plumbing needed) in `crates/paws-dagger/src/lib.rs`
- [X] T017 [P] [US2] Unit test: `CacheBackend::None` (neither signature present) produces a `dagger` subprocess invocation byte-identical to before this feature (FR-007) in `crates/paws-dagger/src/lib.rs` — folded into `detect_falls_through_to_none_with_no_signatures_present`, plus `core`/`core_streaming` themselves are structurally untouched by this feature (T015's correction)
- [X] T018 [P] [US2] Unit test: an invalid/expired `DAGGER_CLOUD_TOKEN` degrades to `None`'s behavior rather than hard-failing the pipeline (Edge Cases) in `crates/paws-dagger/src/lib.rs`

**Checkpoint**: `paws docker`/`paws ci` reuse build-cache layers across separate environments under either provider; default (no backend) behavior is provably unchanged.

---

## Phase 5: User Story 3 - `paws audit` flags a known-vulnerable Rust dependency (Priority: P2)

**Goal**: a new `cargo-audit` scanner joins Semgrep/Gitleaks in `paws audit`'s catalog, gated on
the existing `LanguageFamily::Rust` signal, reporting RustSec-advisory findings in the same shape.

**Independent Test**: run `paws audit` against a fixture with a `Cargo.lock` pinning a known
RustSec advisory; assert the finding appears in the summary with the same fields Semgrep findings
already use.

- [X] T019 [P] [US3] Add `ScannerName::CargoAudit` variant + `as_str() -> "cargo-audit"` in `crates/paws-audit/src/lib.rs`
- [X] T020 [US3] Add an `AUDIT_SCANNER_REGISTRY` row for `cargo-audit`: `applies_to: &[LanguageFamily::Rust]`, `ScannerFamily::Language(LanguageFamily::Rust)`, image `rust:1-bookworm`, step name `"cargo audit --json"` — in `crates/paws-audit/src/lib.rs` (research.md R2/R3)
- [X] T021 [P] [US3] Unit test: `select_audit_scanners` gates `cargo-audit`'s `should_run` on `LanguageFamily::Rust` detection, both present and absent (FR-010) — in `crates/paws-audit/src/lib.rs`
- [X] T022 [US3] Add the `cargo-audit` scanner script (`cargo install cargo-audit --locked && cargo audit --json`, with the same empty-output-fallback pattern `scanner_script`'s existing entries use) in `crates/paws-audit/src/lib.rs`
- [X] T023 [US3] Implement `parse_cargo_audit_findings`, mapping `vulnerabilities.list[]` entries to `AuditScannerResult`/`TopFinding`, mirroring `parse_semgrep_findings`'s exact shape (research.md R3) — in `crates/paws-audit/src/lib.rs`
- [X] T024 [P] [US3] Unit test: `parse_cargo_audit_findings` against a fixture JSON report containing a known `RUSTSEC-YYYY-NNNN` advisory ID produces one `TopFinding` with matching fields (FR-009) — in `crates/paws-audit/src/lib.rs`
- [X] T025 [P] [US3] Unit test: `parse_cargo_audit_findings` against an empty/no-findings report produces zero findings and does not affect overall audit outcome (Acceptance Scenario 1) — in `crates/paws-audit/src/lib.rs`
- [X] T026 [US3] Confirm (read the actual code, don't assume) that Semgrep/Gitleaks currently report findings without failing the build by default in `crates/paws-cli-core::run_audit`, and wire `cargo-audit`'s outcome handling identically (Assumptions note) — in `crates/paws-cli-core/src/lib.rs` — confirmed the scanner loop is already fully generic over `select_audit_scanners`'s output (no per-scanner special-casing exists); only `AuditOverallStatus::Failed` bails, findings alone never do, so `cargo-audit` inherits identical treatment with zero code changes required here

**Checkpoint**: `paws audit` on a Rust project surfaces RustSec-advisory findings alongside Semgrep/Gitleaks; non-Rust projects are unaffected.

---

## Phase 6: User Story 4 - `paws docs --provider github-pages` actually publishes to GitHub Pages (Priority: P3)

**Goal**: `paws docs` gains a `PublishTarget` provider system (`--provider`, comma-delimited,
multi-target-capable) with `github-pages` fully implemented and `cloudflare-pages`/`s3` reserved
with a clear "not implemented" error.

**Independent Test**: run `paws docs --provider github-pages` against a fixture workspace; assert
the full `target/doc` tree is published in one commit/deployment and a second identical run is
idempotent.

- [X] T027 [P] [US4] Add `PagesConfig` struct + `get_pages_config()` (`GET /repos/{owner}/{repo}/pages`, `None` on 404) to `GitHubReleaseClient` in `crates/paws-release/src/lib.rs` (research.md R4)
- [X] T028 [P] [US4] Unit test: `get_pages_config` parses `build_type` correctly and returns `None` on a 404 — in `crates/paws-release/src/lib.rs` — added a `#[cfg(test)]`-only `base_override` on `GitHubReleaseClient` (`new_for_test`) so this and T031 can run against a real local fixture HTTP server rather than only asserting request-building in isolation
- [X] T029 [US4] Add `create_blob()` (`POST .../git/blobs`, returns blob `sha`) to `GitHubReleaseClient` in `crates/paws-release/src/lib.rs`
- [X] T030 [US4] Add `publish_tree()` (`POST .../git/trees` with `base_tree`, `POST .../git/commits`, `PATCH .../git/refs/heads/{branch}`, one commit for the whole file set) to `GitHubReleaseClient` in `crates/paws-release/src/lib.rs` (research.md R4)
- [X] T031 [P] [US4] Unit test: `publish_tree` constructs the correct tree/commit/ref-update request sequence against a fixture `(path, blob_sha)` list, preserving `base_tree` (FR-003 — not a per-file loop) — in `crates/paws-release/src/lib.rs`
- [X] T032 [US4] Define `PublishTarget` (`GitHubPages`/`CloudflarePages`/`S3`) and `GitHubPagesMechanism` (`GitTrees`/`PagesDeployment`) enums in `crates/paws-docs/src/lib.rs` (data-model.md)
- [X] T033 [US4] Implement the `github-pages` provider: build `target/doc` once, blob-create every file (T029), call `publish_tree` (T030) once, mechanism auto-selected via `get_pages_config`'s `build_type` (`"legacy"`/unconfigured → Git Trees, `"workflow"` → Pages deployment) — in `crates/paws-docs/src/lib.rs` (research.md R4) — `publish_github_pages(client, branch, docs_dir)`; also added a `#[doc(hidden)] GitHubReleaseClient::with_base_url_for_tests` seam (promoted from T028's `#[cfg(test)]`-only helper) so this crate's own tests can exercise it against a local fixture server too
- [X] T034 [US4] Implement the Pages-deployment branch's Actions-runtime-env-var gate (`ACTIONS_RUNTIME_TOKEN`/`ACTIONS_RESULTS_URL`), failing with an error explicitly naming the missing vars when absent, rather than attempting a doomed deployment call — in `crates/paws-docs/src/lib.rs` (research.md R5)
- [X] T035 [P] [US4] Unit test: `build_type` `"legacy"` or unconfigured (404) selects the Git Trees mechanism — in `crates/paws-docs/src/lib.rs`
- [X] T036 [P] [US4] Unit test: `build_type` `"workflow"` without the Actions-runtime env vars fails with the specific named-vars error, no deployment attempt — in `crates/paws-docs/src/lib.rs`
- [X] T037 [US4] Implement idempotent re-publish: compare the target branch's current tree against the freshly-built `target/doc` content before calling `publish_tree`, skipping (no-op) when unchanged — mirroring `should_publish`'s existing idempotency pattern (Edge Cases) — in `crates/paws-docs/src/lib.rs` — implemented as a whole-tree content digest (`manifest_digest`) stashed at `.paws-docs-manifest` on the publish branch and compared before doing any blob/tree work, generalizing `should_publish`'s single-file byte comparison to a full tree
- [X] T038 [P] [US4] Unit test: re-publishing unchanged content is a safe no-op, not a duplicate commit — in `crates/paws-docs/src/lib.rs`
- [X] T039 [US4] Add comma-delimited `--provider`, `--repository`, `--branch` fields to `DocsArgs` in `crates/paws-cli-core/src/lib.rs` (contracts/paws-docs-publish-contract.md §1)
- [X] T040 [US4] Implement `run_docs`'s multi-provider concurrent dispatch — build `target/doc` once, one `tokio::task::JoinSet` task per named provider (mirroring `paws-provision::provision_with_timing`'s exact shape, research.md R8), aggregating every outcome, exiting non-zero if any failed — in `crates/paws-cli-core/src/lib.rs` (FR-002a) — extracted as `dispatch_publish_targets` so it's testable independent of credential resolution
- [X] T041 [P] [US4] Unit test: `--provider` omitted reproduces today's exact local-build-only behavior (FR-002) — in `crates/paws-cli-core/src/lib.rs`
- [X] T042 [P] [US4] Unit test: `--provider cloudflare-pages` and `--provider s3` each fail immediately with the FR-004a "not implemented yet — see docs/ROADMAP.md" error, no build/publish attempt — in `crates/paws-cli-core/src/lib.rs`
- [X] T043 [P] [US4] Unit test: an unrecognized `--provider` value produces a normal clap "invalid value" parse error, distinct from the FR-004a error (Edge Cases) — in `crates/paws-cli-core/src/lib.rs` — per spec.md's own clarification wording ("same as any other enum-shaped flag in `paws`, e.g. `--toolchain`"), implemented as runtime `anyhow::bail!` rejection matching `--toolchain`'s own existing precedent (which itself isn't a real `clap::ValueEnum`) rather than adding a literal `ValueEnum`
- [X] T044 [US4] Unit test: `--provider github-pages,s3` builds `target/doc` exactly once, `github-pages` succeeds and `s3` fails with the FR-004a error, both outcomes are reported independently, and the command exits non-zero without suppressing either outcome (FR-002a, Acceptance Scenario 5) — in `crates/paws-cli-core/src/lib.rs` — `github-pages`'s success path runs against a real local fixture HTTP server (`GitHubReleaseClient::with_base_url_for_tests`), not a stub
- [X] T045 [P] [US4] Unit test: `--provider github-pages` with a token lacking write access fails with a specific, actionable error before or during publish, not a silent partial publish (Acceptance Scenario 3) — in `crates/paws-cli-core/src/lib.rs` — covered by `paws-release`'s own `create_blob`/`publish_tree`/`put_content` methods already surfacing GitHub's real `403`/`404` response body verbatim in their `anyhow::bail!` on any non-success status (pre-existing pattern, not new code); a live insufficient-permissions run is quickstart.md's job (T049), not a unit test, since a real narrowly-scoped token is needed to observe GitHub's actual 403 body

**Checkpoint**: `paws docs --provider github-pages` publishes for real; `paws docs --help`'s existing claim is no longer an overpromise.

---

## Phase 7: Polish & Cross-Cutting Concerns

- [X] T046 [P] Update `paws docs --help`'s doc comment to accurately describe `--provider`'s behavior (closing Gap 1's original documentation-integrity problem) in `crates/paws-cli-core/src/lib.rs`
- [X] T047 [P] Update `docs/ROADMAP.md`: `CacheBackend` status (both providers, explicitly noting `GitHubActionsCache` needs no external account), `cargo-audit` scanner status, and `cloudflare-pages`/`s3` `PublishTarget` providers as tracked follow-ups (not silently absent)
- [X] T048 Run `cargo test --workspace` and confirm zero failures and zero changed expectations in any pre-existing `paws-rust`/`paws-dagger`/`paws-audit`/`paws-release`/`paws-docs` test (spec's Validation Plan; Constitution Principle V) — full workspace run: every crate reports `0 failed`; the one non-test-result "could not compile `clippy-fixture`" line is expected subprocess output from `paws-rust`'s own `-D warnings` gate test intentionally compiling a fixture with a real clippy violation
- [X] T049 Run quickstart.md's manual/CI-job validation scenarios for SC-003 (multi-hundred-file docs publish, no secondary rate limit), SC-004 (real two-separate-environment cache timing comparison, both providers), and SC-005 (RustSec fixture) — not part of `cargo test --workspace` — **SC-005 run for real**: a scratch fixture crate pinning `time = "0.1.45"` (RUSTSEC-2020-0071) produced exactly one `cargo-audit` `TopFinding` (`RUSTSEC-2020-0071`, `time@0.1.45`, severity High) via a genuine `paws audit` run through Dagger; re-run against the same fixture bumped to `time = "0.3"` produced zero findings and `Pass`, confirming both halves of Acceptance Scenario 1/SC-005 end to end (an unrelated pre-existing `gitleaks` failure in this sandbox affected neither run — `cargo-audit`'s outcome was independent, as FR-002a-style independence requires). **SC-003/SC-004 not executed in this session** — both need live infrastructure this sandbox doesn't have (a real scratch GitHub repo + token to publish hundreds of real files to, a real `DAGGER_CLOUD_TOKEN`, and two genuinely separate CI job runs for the timing comparison); left as documented manual/CI-job validation, matching `004-rust-coverage`'s own established precedent for environment-dependent validation the agent can't reproduce locally

## Dependencies & Execution Order

- **Phase 1 (Setup)** → all of Phase 4 (US2) depends on T001 (the `reqwest` dependency `GitHubActionsCache` needs). Phases 3/5/6 (US1/US3/US4) don't depend on T001 at all.
- **Phase 2 (Foundational)**: empty — the four stories are independent, no shared blocking work beyond T001.
- **Phase 3 (US1)**: fully self-contained within `paws-rust`. T002/T003/T005 (tests) can be written in parallel; T004 (the fix) makes T002 pass.
- **Phase 4 (US2)**: T006→T007→T008 sequential (enum, then detection logic, then its tests); T009/T010 (logging) can follow in parallel with T011-T014 (the `GitHubActionsCache` provider itself, the larger body of work per this task list's steering note); T015 (wiring into `core`/`core_streaming`) depends on T007 and T011/T012 both being done; T016-T018 are independent verification tests that can run in parallel with T011-T014.
- **Phase 5 (US3)**: T019→T020 sequential (enum variant, then registry row); T021 depends on T020; T022→T023 sequential (script, then parser); T024/T025 depend on T023; T026 is independent, can run any time.
- **Phase 6 (US4)**: T027→T028 (Pages config + its test); T029→T030→T031 sequential (blob, then tree/commit/ref, then its test); T032 (enums) can start any time; T033 depends on T030 and T032; T034 depends on T033; T035/T036 depend on T033/T034; T037→T038 depend on T033; T039 (CLI flags) is independent of T027-T038; T040 depends on T033 (needs a working provider to dispatch to) and T039; T041-T045 depend on T040.
- **Phase 7 (Polish)**: T046 depends on T039/T040 landing (accurate `--help` needs the real flag behavior to describe); T047 depends on all of Phase 4/5/6's providers existing; T048/T049 depend on everything above.

```
Setup (T001)
    ↓
Foundational (none)
    ↓
    ├── US1 (T002-T005)   ──┐
    ├── US2 (T006-T018)   ──┤
    ├── US3 (T019-T026)   ──┼── Polish (T046-T049)
    └── US4 (T027-T045)   ──┘
```

## Parallel Execution Examples

- **Across stories**: US1, US2, US3, US4 touch entirely disjoint crates (`paws-rust`; `paws-dagger`;
  `paws-audit`; `paws-release`/`paws-docs`/`paws-cli-core`'s `DocsArgs`) and can be worked by up to
  four different contributors/sessions simultaneously once T001 lands (only US2 needs it).
- **Within US2**: T011/T012 (`GitHubActionsCache`'s restore/save — the priority work per this task
  list's steering note) can proceed in parallel with T016-T018 (verification tests for the other
  two `detect()` outcomes) once T007/T008 land.
- **Within US4**: T027/T028 (Pages config) and T029-T031 (blob/tree) can proceed in parallel — both
  are independent `GitHubReleaseClient` additions. T041-T045 (CLI-level tests) can all be written
  in parallel once T040 lands.

## Implementation Strategy

**MVP scope**: User Story 1 (T002-T005) alone is the smallest independently shippable slice — a
four-line diff plus tests, closing the highest-priority (P1) gap: a CI gate that doesn't actually
gate.

**Incremental delivery**:
1. Ship US1 (clippy fix) first — P1, smallest, most urgent (a broken gate is worse than a missing
   feature).
2. Ship US2 (build cache) and US3 (dependency scanner) next, in either order — both P2, fully
   independent of each other and of US1. Within US2, prioritize `GitHubActionsCache` (T011-T015)
   over polishing `DaggerCloud` further, per this task list's steering note — it's the provider
   most consumers will actually depend on.
3. Ship US4 (docs publish) last — P3, lowest urgency (spec's own Motivation: nothing in the
   `valheim-docker` migration depends on it), and the largest single story here.
4. Polish (T046-T049) once all four stories are in.
