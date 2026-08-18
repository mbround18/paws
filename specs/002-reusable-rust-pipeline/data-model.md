# Data Model: Reusable Rust Pipeline Architecture

## Entity: RustPipelineInvocation

- **Fields**
  - `workflowRef` (string; reusable workflow identifier)
  - `toolchain` (string; defaults to repository Rust standard)
  - `components` (string CSV; defaults to `clippy,rustfmt`)
  - `target` (string CSV; optional additional targets)
  - `publishEnabled` (boolean; default `false`)
  - `docsPublishEnabled` (boolean; default `false`)
  - `runsOn` (string; runner label)
- **Validation Rules**
  - `publishEnabled=false` and `docsPublishEnabled=false` must still execute baseline build/test.
  - `toolchain` default must align with `defaults.json`.

## Entity: ConditionalSecretRequirement

- **Fields**
  - `secretName` (enum: `CARGO_REGISTRY_TOKEN`, docs deploy secret(s))
  - `requiredWhen` (expression over pipeline mode flags)
  - `failureMessage` (string)
- **Validation Rules**
  - `CARGO_REGISTRY_TOKEN` required only when `publishEnabled=true`.
  - Missing required conditional secret must fail before mutating operations.

## Entity: RustPublishOperation

- **Fields**
  - `target` (string; `rust-crate`)
  - `registry` (string; default `crates.io`)
  - `versionOverride` (string; optional)
  - `publishAttempted` (boolean)
  - `publishResult` (enum: `skipped`, `success`, `failed`)
- **Validation Rules**
  - `publishAttempted=true` only when `publishEnabled=true`.
  - `publishResult=skipped` when publish intent is disabled.

## Entity: RustDocsPublishOperation

- **Fields**
  - `docsPublishEnabled` (boolean)
  - `docsBuildSucceeded` (boolean)
  - `pagesPublishAttempted` (boolean)
  - `pagesPublishResult` (enum: `skipped`, `success`, `failed`)
  - `artifactPath` (string; docs output path)
- **Validation Rules**
  - Docs publish requires successful baseline checks.
  - Empty/invalid docs artifact must produce actionable failure.
  - `pagesPublishAttempted=false` when docs mode disabled.

## Entity: CompatibilityAdapterContract

- **Fields**
  - `legacySurface` (enum: `rust-build-n-test`, `publish-rust-crate`)
  - `newSurface` (string; new reusable Rust contract mapping)
  - `compatibilityMode` (enum: `full`, `transitional`)
  - `migrationNotes` (string)
- **Validation Rules**
  - Existing build/test-only consumers must remain non-breaking.
  - Publish consumers must retain explicit token-gated behavior.

## Relationships

- `RustPipelineInvocation` 1→1 `RustPublishOperation` (optional execution)
- `RustPipelineInvocation` 1→1 `RustDocsPublishOperation` (optional execution)
- `RustPipelineInvocation` 1→N `ConditionalSecretRequirement`
- `CompatibilityAdapterContract` maps legacy surfaces to new invocation paths

## State Transitions

### RustPublishOperation

- `skipped` (default) → `success` (publish enabled + valid token + checks pass)
- `skipped` (default) → `failed` (publish enabled + validation/check failure)
- `failed` → `success` (after remediation and rerun)

### RustDocsPublishOperation

- `skipped` (default) → `success` (docs enabled + docs generation + pages publish pass)
- `skipped` (default) → `failed` (docs enabled but preconditions/artifacts invalid)
- `failed` → `success` (after remediation and rerun)
