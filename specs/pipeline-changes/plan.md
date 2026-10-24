# Implementation Plan: Mod Upload and Validation Pipeline for GitHub Environment-backed Publishing

**Branch**: `008-mod-upload-validation` | **Date**: 2026-10-24 | **Spec**: `specs/pipeline-changes/spec.md`

**Input**: Feature specification from `/specs/pipeline-changes/spec.md`

## Summary

Implement an opt-in mod publishing pipeline that is safe-by-default, environment-aware, and validation-first. The design introduces reusable-workflow inputs for provider-specific auth, GitHub Environment precedence, and a Dagger-backed build/test/package/upload orchestration layer that also supports the mixed-language `mbround18/valheim-mods` repository pattern. The project will keep existing behavior unchanged unless a caller explicitly turns on the upload flow.

## Technical Context

**Language/Version**: Rust toolchain for Dagger orchestration and CLI integration; GitHub Actions workflow YAML; mixed-language mod repos may include Rust, C#, .NET, and other toolchains. Default target parity remains Rust-first with cross-language execution adapters.

**Primary Dependencies**: GitHub Actions reusable workflows, Dagger subprocess orchestration, `paws-core` defaults, `paws-cli-core` input parsing, external registry APIs for Hexium and Thunderstore, manifest validation tooling.

**Storage**: File-system based artifact staging under temporary workspace; no DB needed for initial release.

**Testing**: `cargo test --workspace`, targeted crate tests for new package validation and upload contracts, GitHub Actions workflow smoke tests for environment secret resolution and validation gate behavior.

**Target Platform**: GitHub-hosted CI with GitHub Environment secrets; local dev and CI runners for dry-run validation; provider APIs are external network services.

**Project Type**: CLI + CI workflow + reusable GitHub workflow pipeline.

**Performance Goals**: Build/test/package validation should complete within a normal release pipeline window; no network upload happens until validation passes; artifacts should be hash-checked and staged in a temporary directory to reduce duplication.

**Constraints**: No secrets in CLI args; support mixed-language repos without hardcoding a single language; default to no-op unless opted in; validation must block upload; registry auth should prefer environment-scoped secret sources.

**Scale/Scope**: Supports a small number of registry targets (Hexium + Thunderstore) and a mod repo build graph with multiple toolchains and package outputs.

## Constitution Check

- **Principle I (One Crate Per Domain)**: PASS. The feature is implemented in domain-aligned crates and workflow wrappers: workflow contract stays in reusable GitHub Actions; Dagger orchestration stays in `paws-dagger`/relevant crate(s); packaging/validation logic remains under dedicated toolchain crates rather than being embedded in `paws-cli`.
- **Principle II (Subprocess-First Dagger Access, Single Call Site)**: PASS. Any Dagger invocation remains through `paws-dagger::call` and not direct SDK calls. The change adds orchestration, not a new Dagger spawn path.
- **Principle III (Incremental SDK Adoption)**: PASS. No adoption of the Rust Dagger SDK is required for this feature; it is a workflow and packaging pipeline addition.
- **Principle IV (Parity Testing Over Reimplementation-From-Memory)**: PASS. The design explicitly models the registry behavior from the target APIs/doc specs and the project's existing workflow conventions rather than re-creating ad hoc assumptions.
- **Principle V (Reliability & Testability First)**: PASS. Validation-before-upload, credential masking, and opt-in defaults are all designed to be testable and fail-safe.
- **Technical Constraints — No secrets on the command line**: PASS. Secrets are read from environment variables and GitHub Environment mappings only.
- **Gate result**: PASS; no policy violation requires an exception.

## Project Structure

### Source Code (repository root)

```text
actions/
├── paws-up/
│   └── action.yml
crates/
├── paws-cli-core/
├── paws-core/
├── paws-dagger/
├── paws-dotnet/
├── paws-node/
├── paws-publish/
├── paws-python/
├── paws-rust/
└── paws-run/

docs/
├── ROADMAP.md
└── DEVELOPMENT.md
specs/
└── pipeline-changes/
    ├── spec.md
    ├── plan.md
    ├── research.md
    ├── data-model.md
    ├── quickstart.md
    └── contracts/
        └── mod-upload-contract.md
```

**Structure Decision**: Keep the feature as a workflow plus Dagger contract addition rather than a monolithic app. Reusable workflow and Dagger interface logic live as thin orchestration layers; actual language-specific build/test/package details stay under relevant crate/domain bindings.

## Complexity Tracking

No constitution violations. No complexity exceptions required.

## Design Decisions

1. **Credential resolution model**
   - Prefer GitHub Environment values for `ACCESS_KEY`, `USERNAME`, `PASSWORD`, and `TEAM`.
   - Support explicit fallback only when the caller opts in and the value is not present in the environment.
   - Normalize to a single `ResolvedCredentials` type before upload requests are constructed.

2. **Validation-first pipeline**
   - Stage ordering: `build` → `test` → `package` → `validate` → `upload`.
   - Any failed stage blocks later stages and returns a structured error code.
   - Validation includes manifest and package-shape checks and registry-specific contract checks.

3. **Mixed-language mod support**
   - Treat `mbround18/valheim-mods` as canonical first-class target: a repo may include Rust, C#, and other toolchains.
   - The pipeline uses a language matrix rather than a single-language assumption.
   - Package assembly occurs after all required language builds pass, and final artifact validation happens before upload.

4. **Backward-compatible defaults**
   - `mod_upload.enabled` defaults to `false`.
   - The new reusable workflow is opt-in and does not change existing callers.
   - All new inputs are safe no-ops unless explicitly enabled.

5. **Security controls**
   - No secrets in logs; every credential is masked or redacted.
   - Secrets must be resolved from environment-scoped sources if present.
   - Validation also checks for secret leakage in staged artifacts or generated manifests.

## Workstreams

1. **Workflow contract and credential resolution**
   - Define reusable workflow inputs and outputs.
   - Add environment resolution precedence and missing-secret failure semantics.
   - Add provider-specific credential normalization.

2. **Dagger orchestration**
   - Add `build_mod_artifact`, `test_mod_artifact`, `package_mod_artifact`, and `run_valheim_mod_pipeline` functions.
   - Keep public signatures stable and explicit, treating them as public contracts.
   - Ensure the pipeline returns structured pass/fail status and per-stage results.

3. **Validation and security gating**
   - Implement manifest validation and package-shape checks before upload.
   - Add secret redaction and build artifact scan checks.
   - Ensure upload is blocked on validation failures.

4. **Mixed-language repo support**
   - Support a configurable language matrix and artifact assembly flow for Rust/C#/other modules.
   - Add a package contract for output naming and artifact staging.
   - Ensure validation operates on the final package contents rather than source directories.

5. **Validation and docs**
   - Add a quickstart proving the end-to-end release path.
   - Document rollout/rollback in automation and release docs.
   - Keep workflow descriptions and runtime defaults in sync.

## Contract-Safety Checklist

- [x] Workflow declarations remain additive and opt-in.
- [x] Dagger function signatures are explicitly modeled as public contracts.
- [x] Secrets remain environment-scoped and never passed via CLI flags.
- [x] Default runtime baseline remains safe and no-op until explicitly enabled.
- [x] Security and scan behavior are documented before implementation.

## Validation Matrix

| Surface | Validation |
| --- | --- |
| Workflow auth | GitHub Environment precedence and missing-secret failure tested with a reusable workflow smoke case |
| Validation gate | Invalid mod manifests fail before upload; valid ones proceed |
| Multi-language build | Rust/C#/other-language mod repos pass through build/test/package stages via language matrix |
| Upload | Hexium and Thunderstore upload requests are generated only after validation succeeds |
| Security | Job logs and summaries redact secrets and rejection reasons are structured |
| Backward compatibility | Existing non-upload callers keep default behavior and do not trigger publish steps |
