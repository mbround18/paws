# Feature Specification: Reusable Rust Pipeline Architecture

## Summary

Introduce a new reusable Rust pipeline contract for downstream repositories that centralizes build/test execution, optional crates.io publishing, and optional Rust docs publication to GitHub Pages through shared pipeline code.

## Motivation and Problem Statement

Rust support exists today, but build/test, publishing, and docs publishing are not yet unified under one reusable contract with explicit gating and secret requirements. This creates duplication risk and makes downstream adoption less predictable.

This feature ensures Rust consumers get one clear reusable interface with explicit inputs/secrets, safe publish behavior, and compatibility with existing Rust workflow entrypoints already used in this repository.

## Scope

### In scope

- Define a new reusable Rust pipeline contract that supports build + test by default.
- Add optional crates.io publishing behavior guarded by explicit publish intent.
- Enforce crates token requirements when publish intent is enabled.
- Add optional Rust docs publication path to GitHub Pages using shared pipeline behavior.
- Define least-privilege permissions and required/optional secrets for each execution mode.
- Define compatibility expectations with existing `rust-build-n-test` and publish workflow usage patterns.

### Out of scope

- Replacing non-Rust language pipeline contracts.
- Changing crate versioning policy or release governance outside pipeline execution behavior.
- Introducing new package registry targets beyond crates.io in this iteration.

## Affected Contracts

- Reusable workflow contract:
  - New reusable Rust pipeline entrypoint with explicit inputs: `toolchain`, `components`, `target`, `name`, `publish`, `registry`, `version`, `publish_docs`, `docs_path`, and `runs-on`.
  - Explicit secrets contract distinguishing always-required vs conditionally-required secrets: `DAGGER_CLOUD_TOKEN` (optional), `CARGO_REGISTRY_TOKEN` (conditionally required when `publish: true`).
- Dagger module `@func()` contracts:
  - Public function names and call names remain explicit, stable, and aligned to kebab-case naming conventions used by workflow callers (`rust-build-and-test`, `publish-rust-crate`, and docs-publish entrypoint when added).
  - Thin workflow adapters continue delegating core behavior through `.github/actions/dagger-run`.
- Downstream compatibility expectations:
  - Existing `rust-build-n-test` and publish workflow consumers must remain compatible via adapter or equivalent contract continuity.
  - Migration path must avoid forced breaking changes for consumers that only need build/test.

## Runtime and Defaults Impact

- Rust pipeline execution must align with repository runtime defaults and shared reusable workflow expectations.
- Publish and docs paths must remain opt-in, so default Rust CI behavior remains build/test focused.
- Any additional runtime assumptions for docs generation/publication must be documented in the reusable contract.

## Security and Permissions Impact

- Pipeline must use least-privilege permissions by default for build/test-only runs.
- Publishing and docs publication modes must elevate permissions only when their corresponding option is enabled.
- Crates publishing must require a crates token only when publish is enabled, and must fail safely before release mutation when missing.
- Secret usage must be explicit in the reusable workflow contract so downstream repositories know exactly what to provide.

## Risks and Mitigations

- **Risk**: Downstream repos accidentally trigger publish behavior.  
  **Mitigation**: Publish is opt-in and disabled by default with explicit input gating.
- **Risk**: Missing `CARGO_REGISTRY_TOKEN` causes ambiguous failures.  
  **Mitigation**: Validate token presence up front when publish is enabled and return a clear failure reason.
- **Risk**: Docs publication introduces broader permissions than needed.  
  **Mitigation**: Scope docs permissions to docs-enabled runs only and keep build/test permissions minimal.
- **Risk**: Existing Rust workflow consumers regress during migration.  
  **Mitigation**: Preserve compatibility expectations and validate with current `rust-build-n-test` and publish usage patterns.

## User Scenarios & Testing _(mandatory)_

### User Story 1 - Run reusable Rust CI baseline (Priority: P1)

As a downstream repository maintainer, I want a shared Rust pipeline that always runs build and test so I can adopt repository-standard Rust CI behavior without custom pipeline logic.

**Why this priority**: Build/test is the baseline path every Rust consumer needs, and it establishes the reusable contract foundation.

**Independent Test**: Invoke the reusable pipeline with default options and confirm build/test completes with no publish or docs mutation behavior.

**Acceptance Scenarios**:

1. **Given** a downstream repo uses the reusable Rust pipeline with default inputs, **When** the workflow runs, **Then** build and test execute successfully and no publish step is attempted.
2. **Given** build or test fails, **When** the reusable pipeline completes, **Then** the workflow reports failure and no publish or docs publication is performed.

---

### User Story 2 - Publish crate safely when requested (Priority: P1)

As a release maintainer, I want crates.io publish to run only when explicitly enabled and properly authenticated so releases are intentional and secure.

**Why this priority**: Publishing is high-impact and must be strictly gated to prevent accidental or unsafe releases.

**Independent Test**: Run one invocation with publish enabled and valid token, and one invocation with publish enabled but token omitted; verify gated behavior in both.

**Acceptance Scenarios**:

1. **Given** publish is enabled and crates token is provided, **When** pipeline checks pass, **Then** the crate publish path runs.
2. **Given** publish is enabled and crates token is missing, **When** the workflow starts publish validation, **Then** it fails with a clear contract error before any publish action.
3. **Given** publish is disabled, **When** the workflow runs, **Then** crates token is not required and no publish action is attempted.

---

### User Story 3 - Publish Rust docs to GitHub Pages via shared path (Priority: P2)

As a documentation owner, I want optional Rust docs publication through shared reusable behavior so docs deployment is consistent across repositories.

**Why this priority**: Docs publication is valuable but secondary to core CI and release safety.

**Independent Test**: Enable docs publication in a pipeline run and verify docs are generated and published through the shared publication path after successful checks.

**Acceptance Scenarios**:

1. **Given** docs publish is enabled and prerequisites are satisfied, **When** the workflow completes build/test successfully, **Then** generated Rust docs are published to GitHub Pages through shared pipeline behavior.
2. **Given** docs publish is disabled, **When** the workflow runs, **Then** no docs publication is attempted.

### Edge Cases

- Publish enabled with empty or invalid crates token input.
- Publish enabled while build/test fails earlier in the run.
- Docs publish enabled but docs generation output is empty or invalid.
- Build/test-only consumers that pass no publish/docs inputs.
- Existing `rust-build-n-test` and publish consumers that rely on prior call contracts.

## Requirements _(mandatory)_

### Functional Requirements

- **FR-001**: The repository MUST provide one new reusable Rust pipeline contract consumable by downstream repositories.
- **FR-002**: The reusable Rust pipeline MUST execute build + test as the default behavior.
- **FR-003**: The reusable Rust pipeline MUST expose an explicit publish enablement input that defaults to disabled.
- **FR-004**: The reusable Rust pipeline MUST expose an explicit docs publication enablement input that defaults to disabled.
- **FR-005**: When publish enablement is true, the pipeline MUST require `CARGO_REGISTRY_TOKEN` before attempting any publish action.
- **FR-006**: When publish enablement is true and crates token is missing, the pipeline MUST fail with a clear contract validation error before mutating release state.
- **FR-007**: When publish enablement is false, the crates token MUST be treated as optional and no crates publish action may run.
- **FR-008**: When docs publication is enabled, generated Rust docs MUST be published to GitHub Pages via shared reusable pipeline behavior.
- **FR-009**: Docs publication MUST only run after successful required preconditions for that run, including successful baseline build/test.
- **FR-010**: Workflow inputs and secrets MUST be explicitly documented as required, optional, and conditionally-required.
- **FR-011**: Workflow permissions MUST follow least-privilege defaults, with any elevated scopes limited to publish-enabled or docs-enabled runs.
- **FR-012**: The design MUST preserve compatibility expectations with existing `rust-build-n-test` and publish workflow contracts through non-breaking consumer behavior or documented adapters.
- **FR-013**: Reusable workflow call naming and public Dagger function naming MUST remain explicit and aligned with kebab-case `@func` call conventions.

### Key Entities

- **Rust Pipeline Invocation**: A single execution request of the reusable Rust workflow with explicit inputs, secrets, and mode flags.
- **Publish Intent**: The explicit workflow input that determines whether crates publishing is permitted in that run.
- **Docs Publish Intent**: The explicit workflow input that determines whether Rust docs are published to GitHub Pages in that run.
- **Conditional Secret Requirement**: A contract rule that marks a secret as mandatory only when a specific optional capability is enabled.

## Success Criteria _(mandatory)_

### Measurable Outcomes

- **SC-001**: 100% of default Rust pipeline runs execute build/test without requiring publish or docs secrets.
- **SC-002**: 100% of runs with publish enabled and missing crates token fail before any publish mutation.
- **SC-003**: 100% of runs with publish disabled skip crates publication regardless of token presence.
- **SC-004**: At least 95% of downstream Rust consumers can adopt the new reusable contract without changing their core build/test intent.
- **SC-005**: 100% of docs-enabled successful runs produce a completed docs publication outcome to GitHub Pages or a clear, actionable failure reason.

## Assumptions

- Downstream consumers using Rust pipelines already rely on reusable workflow-based integration patterns.
- Existing Rust workflows in this repository represent the baseline compatibility target for migration.
- Publishing to crates.io is a controlled release operation and should remain opt-in.
- GitHub Pages publication for Rust docs is allowed only when explicitly enabled by repository owners.

## Validation Plan

- Validate contract-level behavior for each mode:
  - build/test only,
  - build/test + publish enabled,
  - build/test + docs publish enabled,
  - build/test + both optional modes enabled.
- Validate publish gating scenarios:
  - publish enabled with valid token,
  - publish enabled with missing token,
  - publish disabled with and without token.
- Validate least-privilege permissions for default, publish-enabled, and docs-enabled runs.
- Validate compatibility by exercising existing `rust-build-n-test` and publish workflow usage expectations against the new reusable contract path.
- Validate that workflow adapters remain thin and delegate core behavior through shared Dagger module contracts.

## Rollout and Rollback

- Roll out the reusable Rust pipeline as an additive contract and migrate current Rust workflow entrypoints in controlled phases.
- Keep compatibility adapters for existing `rust-build-n-test` and publish integration expectations during migration.
- If regressions are found, roll back by reverting consumers to the prior known-good reusable workflow reference and disabling optional publish/docs flags in affected repos.
- Resume rollout only after validation confirms contract compatibility and publish/docs gating behavior is restored.
