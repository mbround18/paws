# Implementation Plan: Reusable Rust Pipeline Architecture

## Inputs

- Linked feature spec: `specs/002-reusable-rust-pipeline/spec.md`
- Affected paths:
  - `.github/workflows/rust-build-n-test.yaml`
  - `.github/workflows/rust-docs-publish.yaml`
  - `.github/workflows/publish.yaml`
  - `.github/workflows/test-rust-build-n-test.yaml`
  - `.github/actions/dagger-run/action.yml` (wiring-only verification)
  - `packages/dagger-module/src/index.ts`
  - `packages/dagger-module/src/types.ts`
  - `packages/dagger-module/src/reporting.ts`
  - `packages/dagger-pipelines/src/*.test.ts` (governance/compatibility)
  - `packages/dagger-pipelines/src/workflows.ts` (shared compliance helpers)
  - `packages/dagger-pipelines/package.json` (focused compliance scripts if needed)

## Summary

Add an additive reusable Rust pipeline contract that keeps build/test as the default, adds explicit opt-in publish/docs modes, and preserves compatibility for current `rust-build-n-test` and publish consumers via thin workflow adapters backed by Dagger `@func()` contracts.

## Technical Context

**Language/Version**: TypeScript (workspace baseline), YAML reusable workflow contracts, Dagger module functions, Rust toolchain defaults via `defaults.json`  
**Primary Dependencies**: `@dagger.io/dagger`, `vitest`, `yaml`, reusable workflow_call interfaces  
**Storage**: Repository files and workflow contract declarations  
**Testing**: `pnpm run test`, `pnpm run typecheck`, `pnpm --filter @gh-reusable/dagger-pipelines run test -- ...`  
**Target Platform**: GitHub Actions (`ubuntu-latest`) and local Linux/macOS development  
**Project Type**: Monorepo (reusable workflows + Dagger-first execution + governance test suite)  
**Constraints**: Workflows stay thin; core behavior in `packages/dagger-module/src/index.ts`; least-privilege permissions; non-breaking migration for existing Rust consumers  
**Scale/Scope**: Rust workflow contract surfaces (`rust-build-n-test`, `publish`) and Dagger Rust/publication entrypoints

Resolved clarifications from Phase 0 are captured in `research.md`:

- No existing reusable GitHub Pages docs publish workflow exists today.
- Current compatibility surfaces are `rust-build-n-test.yaml` and `publish.yaml` (`target: rust-crate`).
- Governance tests already enforce workflow input/secret usage and Dagger call-name alignment.

## Constitution Check

_GATE: Must pass before Phase 0 research. Re-check after Phase 1 design._

`.specify/memory/constitution.md` is not present in this repository. For this feature, gates are derived from repository-enforced governance and must be treated as provisional until a constitution artifact exists:

1. **Dagger-First Gate** — Workflow YAML remains adapter-only; behavior changes land in Dagger module `@func()` logic.
2. **Contract Integrity Gate** — `workflow_call` inputs/secrets, Dagger call names, and public `@func()` entrypoints remain aligned and validated by tests.
3. **Security Gate** — Default permissions stay least-privilege; publish/docs scopes only elevate when enabled.
4. **Compatibility Gate** — Existing `rust-build-n-test` and `publish` contracts remain usable for downstream consumers.

**Pre-Phase 0 Gate Status**: PROVISIONAL PASS (derived gates only)

## Phase 0 Research Summary

See `specs/002-reusable-rust-pipeline/research.md`.

Key outcomes:

- Reuse existing Rust call naming conventions (`rust-build-and-test`, `publish-rust-crate`) and add new docs call with kebab-case alignment.
- Add docs publishing as an explicit opt-in contract path (no default mutation).
- Extend governance tests to cover new Rust reusable contract surfaces and compatibility expectations.

## Phase 1 Design Artifacts

- Data model: `specs/002-reusable-rust-pipeline/data-model.md`
- Contracts: `specs/002-reusable-rust-pipeline/contracts/reusable-rust-pipeline-contract.md`
- Quickstart: `specs/002-reusable-rust-pipeline/quickstart.md`

## Design Decisions

- Add one reusable Rust pipeline contract surface that always runs build/test and conditionally executes publish/docs branches.
- Keep workflow logic thin and delegate mode handling/gating to Dagger module `@func()` contracts.
- Preserve existing consumer behavior through additive adapter strategy:
  - Keep `.github/workflows/rust-build-n-test.yaml` interface stable.
  - Keep `.github/workflows/publish.yaml` rust-crate path stable.
  - Introduce migration-compatible path for docs publish without forcing current consumers to change.
- Validate with governance tests in `packages/dagger-pipelines` (workflow, integration, publish/release compatibility).

## Project Structure

### Documentation (this feature)

```text
specs/002-reusable-rust-pipeline/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
└── contracts/
    └── reusable-rust-pipeline-contract.md
```

### Source Code (planned implementation touch points)

```text
.github/workflows/
├── rust-build-n-test.yaml
├── rust-docs-publish.yaml
├── publish.yaml
└── test-rust-build-n-test.yaml

packages/dagger-module/src/
├── index.ts
├── reporting.ts
└── types.ts

packages/dagger-pipelines/src/
├── workflows.ts
├── workflow-governance.test.ts
├── dagger-module-integration.test.ts
├── rust-workflow.test.ts
├── publish-workflow.test.ts
├── release-workflows.test.ts
├── security-workflows.test.ts
└── standards-defaults.test.ts
```

## Work Breakdown

1. **Workflow contract updates**
   - Define/reinforce reusable Rust inputs and conditional secrets.
   - Add docs publish contract surface with explicit opt-in defaults.
   - Ensure permission scopes are mode-specific and least-privilege.
2. **Dagger contract updates**
   - Add/adjust Rust `@func()` entrypoint(s) for unified baseline + optional publish/docs operations.
   - Enforce fail-fast token checks when publish is enabled.
   - Keep call names kebab-case compatible for workflow invocation.
3. **Governance/tests updates**
   - Extend `packages/dagger-pipelines` tests for new contract keys, call names, and compatibility paths.
   - Add/adjust focused compliance scripts when new tests are added.
4. **Rollout and compatibility**
   - Additive rollout preserving existing `rust-build-n-test` and `publish` consumers.
   - Validate parity in repo-level test workflows before recommending downstream migration.

## Contract Safety Checklist

- [x] Reusable workflow input/secret declarations match usage
- [x] Dagger call names remain aligned with module `@func()` names
- [x] Defaults are sourced from `defaults.json` where applicable
- [x] Permissions are explicit and least-privilege
- [x] Publish/docs toggles are opt-in and fail-safe
- [x] Existing `rust-build-n-test` and publish consumers retain compatibility

## Validation Matrix

| Contract surface                                | Validation                                                                                                           |
| ----------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| Rust reusable workflow wiring and declarations  | `pnpm --filter @gh-reusable/dagger-pipelines run test -- src/workflow-governance.test.ts`                            |
| Dagger `@func()` ↔ workflow call-name alignment | `pnpm --filter @gh-reusable/dagger-pipelines run test -- src/dagger-module-integration.test.ts`                      |
| Publish workflow interface + compatibility      | `pnpm --filter @gh-reusable/dagger-pipelines run test -- src/publish-workflow.test.ts src/release-workflows.test.ts` |
| Runtime default/toolchain consistency           | `pnpm --filter @gh-reusable/dagger-pipelines run test -- src/standards-defaults.test.ts`                             |
| Security/audit baseline regression              | `pnpm --filter @gh-reusable/dagger-pipelines run test -- src/security-workflows.test.ts src/audit-workflow.test.ts`  |
| Workspace confidence                            | `pnpm run typecheck && pnpm run test`                                                                                |

## Rollout and Compatibility Strategy

1. Ship additive reusable Rust pipeline changes with defaults equivalent to current build/test behavior.
2. Keep current `rust-build-n-test` and `publish` interfaces operational while introducing docs mode.
3. Validate compatibility via existing and updated governance suites before downstream rollout.
4. Migrate downstream repos in phases:
   - Phase A: build/test only
   - Phase B: publish enabled repos
   - Phase C: docs-enabled repos
5. Rollback plan:
   - Revert callers to prior workflow refs and disable publish/docs flags.
   - Preserve baseline build/test path as safe default.

## Post-Design Constitution Check

All four gates remain satisfied by this design. No unresolved constitutional violations.

## Implementation Notes (Completed)

- Added shared reusable docs workflow `.github/workflows/rust-docs-publish.yaml` with Pages-scoped permissions and contract-safe docs-path validation.
- Updated `.github/workflows/rust-build-n-test.yaml` to invoke Dagger `rust-pipeline` with publish/docs opt-in inputs and conditional token wiring.
- Added Dagger `@func()` contracts `rustPipeline` and `rustDocsBuild` in `packages/dagger-module/src/index.ts` with mode-aware reporting outputs.
- Extended report/type surfaces for rust mode outcomes in `packages/dagger-module/src/reporting.ts` and `packages/dagger-module/src/types.ts`.
- Added governance coverage across workflow, release, security, integration, and rust-specific tests.
