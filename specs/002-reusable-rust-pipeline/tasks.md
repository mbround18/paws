# Tasks: Reusable Rust Pipeline Architecture

**Input**: Design documents from `/specs/002-reusable-rust-pipeline/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/

## Execution Rules

- Keep workflow YAML thin; implement mode logic in Dagger `@func()` contracts.
- Pair every workflow/module contract change with governance tests in the same PR.
- Preserve compatibility for existing `rust-build-n-test` and `publish` (`target: rust-crate`) consumers.

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Establish feature tracking artifacts and validation logs.

- [x] T001 Create Rust pipeline contract matrix in specs/002-reusable-rust-pipeline/checklists/contract-matrix.md
- [x] T002 Create feature validation evidence log in specs/002-reusable-rust-pipeline/checklists/validation-log.md
- [x] T003 [P] Create docs publish prerequisites checklist in specs/002-reusable-rust-pipeline/checklists/docs-pages-prereqs.md

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Add shared primitives for mode-aware contract validation/reporting.

**⚠️ CRITICAL**: No user story work starts until this phase is complete.

- [x] T004 Add reusable conditional input/secret assertion helpers in packages/dagger-pipelines/src/workflows.ts
- [x] T005 [P] Extend pipeline report structures for rust mode outcomes in packages/dagger-module/src/types.ts
- [x] T006 [P] Add mode-aware reporting helpers for publish/docs decisions in packages/dagger-module/src/reporting.ts
- [x] T007 Wire shared workflow utilities exports for updated governance tests in packages/dagger-pipelines/src/index.ts

**Checkpoint**: Shared validation and reporting primitives are ready for all stories.

---

## Phase 3: User Story 1 - Run reusable Rust CI baseline (Priority: P1) 🎯 MVP

**Goal**: Deliver a reusable Rust contract that always runs build/test by default.

**Independent Test**: Invoke Rust reusable workflow with defaults and confirm only build/test paths run.

### Tests for User Story 1

- [x] T008 [P] [US1] Add baseline Rust contract input/secret reference assertions in packages/dagger-pipelines/src/workflow-governance.test.ts
- [x] T009 [P] [US1] Add Rust baseline `@func()` call-name alignment checks in packages/dagger-pipelines/src/dagger-module-integration.test.ts
- [x] T010 [P] [US1] Add Rust reusable defaults/permission assertions in packages/dagger-pipelines/src/standards-defaults.test.ts

### Implementation for User Story 1

- [x] T011 [US1] Add unified Rust reusable contract inputs/defaults in .github/workflows/rust-build-n-test.yaml
- [x] T012 [US1] Implement baseline reusable Rust pipeline `@func()` orchestration in packages/dagger-module/src/index.ts
- [x] T013 [P] [US1] Update Rust parity workflow to validate baseline reusable contract path in .github/workflows/test-rust-build-n-test.yaml
- [x] T014 [US1] Document baseline invocation and compatibility notes in specs/002-reusable-rust-pipeline/contracts/reusable-rust-pipeline-contract.md

**Checkpoint**: US1 is independently testable with default build/test behavior.

---

## Phase 4: User Story 2 - Publish crate safely when requested (Priority: P1)

**Goal**: Gate publish execution behind explicit intent and required crates token.

**Independent Test**: Run publish-enabled scenario with and without `CARGO_REGISTRY_TOKEN` and verify fail-fast gating behavior.

### Tests for User Story 2

- [x] T015 [P] [US2] Add publish gate assertions for `publish` input and rust token handling in packages/dagger-pipelines/src/publish-workflow.test.ts
- [x] T016 [P] [US2] Add release compatibility assertions for rust publish adapter behavior in packages/dagger-pipelines/src/release-workflows.test.ts
- [x] T017 [P] [US2] Add mode-specific permission assertions for publish-enabled Rust runs in packages/dagger-pipelines/src/security-workflows.test.ts

### Implementation for User Story 2

- [x] T018 [US2] Enforce fail-fast required CARGO_REGISTRY_TOKEN validation when publish is enabled in packages/dagger-module/src/index.ts
- [x] T019 [US2] Add publish opt-in contract wiring for Rust reusable workflow in .github/workflows/rust-build-n-test.yaml
- [x] T020 [US2] Preserve `target: rust-crate` compatibility while delegating through updated publish path in .github/workflows/publish.yaml
- [x] T021 [US2] Add publish-gating decision details to pipeline summaries in packages/dagger-module/src/reporting.ts

**Checkpoint**: US2 blocks accidental publish and enforces required token contract.

---

## Phase 5: User Story 3 - Publish Rust docs to GitHub Pages via shared path (Priority: P2)

**Goal**: Provide optional docs publication to GitHub Pages through shared reusable behavior.

**Independent Test**: Enable docs mode and verify docs generation + GitHub Pages publication only after successful baseline checks.

### Tests for User Story 3

- [x] T022 [P] [US3] Add docs-mode input/secret reference coverage in packages/dagger-pipelines/src/workflow-governance.test.ts
- [x] T023 [P] [US3] Add docs publication compatibility assertions, including actionable failure-path messaging checks, in packages/dagger-pipelines/src/release-workflows.test.ts
- [x] T024 [P] [US3] Add docs publish `@func()` call-name integration coverage in packages/dagger-pipelines/src/dagger-module-integration.test.ts

### Implementation for User Story 3

- [x] T025 [US3] Implement shared Rust docs generation/publication pipeline contract in packages/dagger-module/src/index.ts
- [x] T026 [US3] Add docs publish opt-in and Pages permission gating in .github/workflows/rust-build-n-test.yaml
- [x] T027 [US3] Add reusable docs publication adapter workflow in .github/workflows/rust-docs-publish.yaml
- [x] T028 [US3] Add docs-mode parity coverage in .github/workflows/test-rust-build-n-test.yaml
- [x] T029 [US3] Document docs publication prerequisites and rollback notes in specs/002-reusable-rust-pipeline/quickstart.md

**Checkpoint**: US3 enables optional GitHub Pages docs publication without changing default CI behavior.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Final consistency, command validation, and rollout readiness.

- [x] T030 [P] Reconcile final contract/design notes with implemented behavior in specs/002-reusable-rust-pipeline/plan.md
- [x] T031 [P] Add/update focused compliance aliases for Rust stories in packages/dagger-pipelines/package.json
- [x] T032 Run focused validation commands and record evidence in specs/002-reusable-rust-pipeline/checklists/validation-log.md
- [x] T033 Run workspace validation commands and record final pass/fail status in specs/002-reusable-rust-pipeline/checklists/validation-log.md

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup)**: No dependencies.
- **Phase 2 (Foundational)**: Depends on Phase 1; blocks all user stories.
- **Phase 3 (US1)**: Depends on Phase 2; MVP.
- **Phase 4 (US2)**: Depends on Phase 2 and US1 contract wiring.
- **Phase 5 (US3)**: Depends on Phase 2 and US1 baseline contract.
- **Phase 6 (Polish)**: Depends on completion of targeted user stories.

### User Story Dependencies

- **US1 (P1)**: Starts after foundational work; no story dependency.
- **US2 (P1)**: Starts after US1 baseline contract is in place.
- **US3 (P2)**: Starts after US1 baseline contract; independent of US2 publish internals.

### Within Each User Story

- Write/expand tests first and confirm they fail for missing behavior.
- Implement Dagger module contract logic before workflow adapter rewiring.
- Finish documentation updates after behavior and tests pass.

---

## Parallel Opportunities

- Setup: T003 can run with T001-T002.
- Foundational: T005-T006 can run in parallel after T004.
- US1 tests: T008-T010 can run in parallel.
- US2 tests: T015-T017 can run in parallel.
- US3 tests: T022-T024 can run in parallel.
- Polish: T030-T031 can run in parallel before T032-T033.

---

## Parallel Example: User Story 1

```bash
Task: "T008 [US1] Add baseline contract assertions in packages/dagger-pipelines/src/workflow-governance.test.ts"
Task: "T009 [US1] Add Rust call-name alignment checks in packages/dagger-pipelines/src/dagger-module-integration.test.ts"
Task: "T010 [US1] Add defaults/permission assertions in packages/dagger-pipelines/src/standards-defaults.test.ts"
```

## Parallel Example: User Story 2

```bash
Task: "T015 [US2] Add publish gate assertions in packages/dagger-pipelines/src/publish-workflow.test.ts"
Task: "T016 [US2] Add release compatibility assertions in packages/dagger-pipelines/src/release-workflows.test.ts"
Task: "T017 [US2] Add publish permission assertions in packages/dagger-pipelines/src/security-workflows.test.ts"
```

## Parallel Example: User Story 3

```bash
Task: "T022 [US3] Add docs-mode governance assertions in packages/dagger-pipelines/src/workflow-governance.test.ts"
Task: "T023 [US3] Add docs compatibility assertions in packages/dagger-pipelines/src/release-workflows.test.ts"
Task: "T024 [US3] Add docs call-name integration assertions in packages/dagger-pipelines/src/dagger-module-integration.test.ts"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1 and Phase 2.
2. Deliver US1 tests + implementation (T008-T014).
3. Validate baseline behavior with focused governance suites before expanding modes.

### Incremental Delivery

1. Ship MVP baseline reusable Rust contract (US1).
2. Add publish gating and token requirements (US2).
3. Add optional docs publication to GitHub Pages via shared path (US3).
4. Complete polish and full validation evidence capture (Phase 6).

### Final Validation Commands

- `pnpm --filter @gh-reusable/dagger-pipelines run test -- src/workflow-governance.test.ts`
- `pnpm --filter @gh-reusable/dagger-pipelines run test -- src/dagger-module-integration.test.ts`
- `pnpm --filter @gh-reusable/dagger-pipelines run test -- src/publish-workflow.test.ts src/release-workflows.test.ts`
- `pnpm --filter @gh-reusable/dagger-pipelines run test -- src/standards-defaults.test.ts src/security-workflows.test.ts src/audit-workflow.test.ts`
- `pnpm run typecheck`
- `pnpm run test`
