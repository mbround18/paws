# Phase 0 Research: Reusable Rust Pipeline Architecture

## Decision 1: Reusable contract shape and compatibility mode

- **Decision**: Introduce the new Rust pipeline as an additive reusable contract, preserving existing `rust-build-n-test` and `publish` behavior as compatibility surfaces.
- **Rationale**: Existing downstream consumers should not be forced into immediate migration or altered default behavior.
- **Alternatives considered**:
  - Replace existing Rust workflows directly (higher breakage risk).
  - Keep separate docs/publish workflows with no unified contract (continues fragmentation).

## Decision 2: Publish gating and secret requirement

- **Decision**: Keep publish explicitly opt-in and fail fast on missing `CARGO_REGISTRY_TOKEN` when publish mode is enabled.
- **Rationale**: This enforces FR-005/FR-006 safety guarantees and avoids ambiguous publish-time failures.
- **Alternatives considered**:
  - Implicit publish when token is present (accidental release risk).
  - Late validation during `cargo publish` (slower, less actionable errors).

## Decision 3: Docs publication path

- **Decision**: Add an explicit docs publish intent and wire docs deployment through shared reusable behavior with mode-specific permissions.
- **Rationale**: Repository currently has no reusable GitHub Pages docs publish workflow; explicit opt-in keeps default CI non-mutating.
- **Alternatives considered**:
  - Always generate/publish docs during baseline Rust CI (unnecessary permissions and runtime cost).
  - Repository-specific ad hoc docs shell steps in workflow YAML (violates thin-workflow goal).

## Decision 4: Dagger contract extension strategy

- **Decision**: Keep workflows thin and evolve Dagger `@func()` contracts in `packages/dagger-module/src/index.ts` for Rust baseline + optional publish/docs behavior.
- **Rationale**: This aligns with repository architecture and existing governance tests that enforce call-name contract integrity.
- **Alternatives considered**:
  - Implement mode logic in workflow YAML conditionals (contract drift risk).
  - Introduce wrapper scripts outside Dagger module (extra maintenance surface).

## Decision 5: Governance and validation strategy

- **Decision**: Validate via focused `@gh-reusable/dagger-pipelines` suites plus workspace-wide `pnpm` checks.
- **Rationale**: Focused suites provide fast contract regression detection; workspace checks ensure cross-package safety.
- **Alternatives considered**:
  - Full workspace checks only (slower iteration).
  - Narrow Rust-only tests with no governance coverage (misses contract drift).
