# Tasks: Expose the GitHub Actions Cache Runtime Vars `GitHubActionsCache` Depends On

**Input**: Design documents from `/home/mbruno/development/paws/specs/006-paws-doesn-expose/`
**Prerequisites**: plan.md, research.md, data-model.md, contracts/, quickstart.md

## Rules

- Keep changes backward-compatible unless explicitly declared breaking.
- Pair contract changes with tests in the same PR.
- Keep docs/examples in sync with behavior.

## Story Map

This spec has exactly one user story (spec.md's own framing: "this is the only story in this
spec"):

- **US1 (P1)** — `GitHubActionsCache` actually activates on a real GitHub-hosted runner via
  `paws-up`, with no extra consumer step, while remaining safely `None` everywhere else.

## Phase 1: Setup

- [x] T001 Confirm current `dagger`/`docker` toolchain and `cargo test -p paws-dagger` both run
      cleanly on this branch before making changes (baseline sanity check; no file changes).

## Phase 2: Foundational

*(No cross-story blocking infrastructure needed — this spec's entire scope is one story's fix.)*

## Phase 3: User Story 1 — `GitHubActionsCache` activates on a real runner (P1)

**Goal**: `paws-up` alone (no extra consumer step) makes `$ACTIONS_CACHE_URL`/
`$ACTIONS_RUNTIME_TOKEN` reach `paws docker`/`paws ci`'s process environment on a real
GitHub-hosted runner, without ever fabricating them elsewhere.

**Independent Test**: run a real GitHub Actions job using the fixed `paws-up`, invoke
`paws docker` against a trivial Dockerfile, and confirm the job log reads
`cache: using github-actions` (see quickstart.md's positive-path steps).

### Tests for User Story 1 (FR-007)

- [x] T002 [P] [US1] Add negative-path unit tests to
      `crates/paws-dagger/src/lib.rs` (or its test module) asserting `CacheBackend::detect()`
      returns `CacheBackend::None` when `ACTIONS_CACHE_URL`/`ACTIONS_RUNTIME_TOKEN` are set to
      `""` (both empty, and one-sided empty) — per
      `specs/006-paws-doesn-expose/contracts/cache-backend-detect.md`'s test obligations table.
      Run tests serially or via `serial_test`/env-mutex pattern already used by `005`'s existing
      `detect()` tests to avoid cross-test env-var races.

### Implementation for User Story 1

- [x] T003 [US1] Add the non-empty-value guard to `CacheBackend::detect()`'s
      `GitHubActionsCache` branch in `crates/paws-dagger/src/lib.rs`, per plan.md's Design
      Decisions ("Defense-in-depth") and data-model.md — treat `Ok("")` the same as absent for
      both `ACTIONS_CACHE_URL` and `ACTIONS_RUNTIME_TOKEN`, with no change to `DaggerCloud`
      precedence, the enum shape, or `log_line()`.
- [x] T004 [US1] Add the new `actions/github-script@<PINNED_SHA>` step to
      `actions/paws-up/action.yml`, placed before the existing `paws init` step, per
      `specs/006-paws-doesn-expose/contracts/paws-up-action.md` — inline script exports
      `ACTIONS_RUNTIME_TOKEN`/`ACTIONS_CACHE_URL` to `$GITHUB_ENV` only when each is truthy
      (never exports an empty string). Resolve and record the actual pinned commit SHA (and its
      corresponding `vX.Y.Z` tag as a comment) for `actions/github-script`, matching FR-003 and
      this repo's existing pin style for other action references.
- [x] T005 [US1] Add a doc comment to the new `paws-up` step (matching the file's existing
      inline-comment style) explaining why it exists and what breaks (cache silently stays
      `None`, nothing else) if it's ever removed or the dependency stops working — per Risks and
      Mitigations in spec.md.
- [x] T006 [US1] Add or extend a real-runner validation workflow under
      `.github/workflows/` that uses the fixed `paws-up` against a trivial fixture Dockerfile,
      asserts the job log contains `cache: using github-actions` (FR-004, SC-001), and runs a
      second job against the same unchanged Dockerfile to produce a timing comparison
      demonstrating a materially faster second run (SC-003) — per
      `specs/006-paws-doesn-expose/research.md` R5 and `quickstart.md`.
- [x] T007 [US1] Update `docs/ROADMAP.md`'s `005-close-remaining-cli` entry (and
      `paws docker --help`/`paws ci --help` text, if either currently implies zero-extra-setup
      activation beyond what's now true) to accurately describe activation requirements after
      this fix (FR-006).

## Phase 4: Polish & Cross-Cutting Concerns

- [x] T008 [P] Run `cargo test --workspace` and confirm zero regressions, including all of
      `005`'s pre-existing `CacheBackend::detect()` tests still passing unmodified (Validation
      Plan).
- [x] T009 [P] Run `quickstart.md`'s negative-path validation (`paws docker`/`paws ci` outside a
      GitHub Actions job) to manually confirm `cache: no backend detected (full rebuild)` still
      prints, closing the loop on FR-005/SC-002 outside the automated test in T002.

## Dependencies

- T001 (Setup) has no dependencies; run first.
- T002 (tests) should be written before or alongside T003 (implementation) — TDD-style, but not
  strictly blocking since this is a small, well-specified change; both belong to US1 and can land
  in the same commit.
- T003 and T004 are independent of each other (different files/languages: Rust vs. YAML) and can
  be done in parallel.
- T005 depends on T004 (same file, same step).
- T006 depends on T003 and T004 both being in place (validates the real end-to-end behavior of
  both changes together).
- T007 has no hard code dependency but should land after T003/T004 are settled, so the docs
  describe the actual shipped behavior.
- T008/T009 (Polish) run after all of Phase 3 is complete.

## Parallel Execution Example

```
# T003 and T004 touch different files/toolchains and can run in parallel:
Task: "Add non-empty-value guard to CacheBackend::detect() in crates/paws-dagger/src/lib.rs"
Task: "Add actions/github-script step to actions/paws-up/action.yml"
```

## Implementation Strategy

**MVP = all of US1** (there is only one story). Suggested order: T001 → T002/T003 →
T004 → T005 → T006 → T007 → T008/T009. T003 (Rust guard) and T004 (workflow step) are the two
load-bearing changes; everything else is tests, docs, or validation around them.
