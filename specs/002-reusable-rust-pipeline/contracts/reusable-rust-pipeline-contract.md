# Reusable Rust Pipeline Contract

This contract defines externally visible behavior for the reusable Rust pipeline and its compatibility with existing Rust workflow surfaces.

## 1) Reusable Workflow Interface Contract (`.github/workflows/*`)

- The reusable Rust contract MUST execute build/test by default.
- Publish and docs publish MUST be explicit opt-in inputs and default to disabled.
- Inputs/secrets declared in workflow_call MUST be referenced in workflow implementation.
- Secrets contract must distinguish:
  - always-optional secrets,
  - conditionally-required secrets (for publish/docs modes).

## 2) Dagger Public Function Contract (`packages/dagger-module/src/index.ts`)

- Workflow call names MUST align with kebab-case `@func()` names.
- Rust baseline and optional publish/docs behavior MUST be implemented in Dagger contracts, not workflow shell logic.
- Publish contract MUST fail early with clear error when publish intent is enabled but token is missing.
- Canonical publish secret name is `CARGO_REGISTRY_TOKEN`; publish-enabled runs must fail before mutation if it is absent.
- Any new docs-related `@func()` is a public contract surface and must be included in integration tests.

## 3) Permission and Security Contract

- Default mode (build/test only) uses least-privilege permissions.
- Publish/docs permissions elevate only in corresponding enabled modes.
- Secret use must be explicit in the contract and must not be required when the mode is disabled.

## 4) Compatibility Contract

- Existing `rust-build-n-test` consumers remain supported without changing build/test intent.
- Existing rust publish consumers using `publish.yaml` target `rust-crate` remain supported.
- Migration to unified contract is additive and phased; no forced breaking change for baseline consumers.

### Baseline invocation contract

- The reusable Rust workflow invokes Dagger `rust-pipeline` with:
  - `publish=false` by default,
  - `publish_docs=false` by default,
  - `toolchain`, `components`, `target`, and `name` passthrough unchanged from previous build/test surface.
- `CARGO_REGISTRY_TOKEN` is conditionally required only when `publish=true`.
- Docs deployment is delegated through shared reusable workflow `.github/workflows/rust-docs-publish.yaml`.

## 5) Governance Test Contract (`packages/dagger-pipelines`)

Required coverage for this feature:

- `src/workflow-governance.test.ts` for input/secret reference integrity.
- `src/dagger-module-integration.test.ts` for `@func()` ↔ workflow call alignment.
- `src/publish-workflow.test.ts` and `src/release-workflows.test.ts` for publish/release compatibility.
- `src/standards-defaults.test.ts` for toolchain/defaults alignment and explicit permissions.

## Versioning and Change Policy

- Default expectation: non-breaking for existing Rust consumers.
- If any breaking behavior becomes necessary, release notes must include migration and rollback guidance plus explicit compatibility classification.
