# Feature Specification: Mod Upload and Validation Pipeline for GitHub Environment-backed Publishing

**Feature Branch**: `008-mod-upload-validation`

**Created**: 2026-10-24

**Status**: Draft

**Input**: User description: "i want a way to upload favoring github enviornments and using the secret ACCESS_KEY or USERNAME and PASSWORD and TEAM inputs to upload mods to https://hexium.gg/api/docs/ and https://thunderstore.io/api/docs/ we should support mod validation before uploading too https://thunderstore.io/tools/manifest-v1-validator/ https://hexium.gg/packaging"

## Summary

Add a GitHub Actions / reusable-workflow upload path that publishes mod packages to the Hexium and Thunderstore APIs using GitHub Environment-scoped configuration and explicit credentials. The pipeline will prefer GitHub Environment variables and secrets, support either `ACCESS_KEY` or `USERNAME` + `PASSWORD` + `TEAM` credentials, and will validate package manifests before upload. Validation must happen before any remote publish, and failures must block release publication while preserving a clear audit trail for the workflow.

This feature must also support the full `mbround18/valheim-mods` repository workflow: multi-language mod projects spanning Rust, C#, and any other supported language used by the mod toolchain. The pipeline must cover the end-to-end mod lifecycle: build, test, package, validate, and then upload the final artifact to the chosen registry. The build/test/package/upload flow is treated as a public contract and must work for mod repos that contain mixed-language tooling and generated package outputs.

This change is intentionally additive: existing workflows remain unchanged unless a caller opts into the upload stage. The new upload contract is treated as a public API surface across reusable workflows and Dagger functions.

## Motivation and Problem Statement

The project needs a secure, repeatable way to publish mod packages to external mod hosting APIs without embedding credentials in the repository or in ad hoc shell scripts. The target registries have different auth patterns but share the same overall need: validate package metadata, verify the bundle shape, and then publish only when the package is acceptable.

The current repo does not define a standard contract for:

- reusable workflow inputs and secret usage for mod upload
- provider-specific auth resolution (`ACCESS_KEY` vs. `USERNAME`/`PASSWORD` + `TEAM`)
- manifest validation before upload
- environment-scoped execution that prefers GitHub Environments over plain repository secrets or generic runner env
- Dagger-side functions that may be reused by CI and local dev flows

The result is inconsistent behavior, difficult auditing, and risk of invalid packages being uploaded to registries with low tolerance for malformed metadata.

## Scope

### In scope

- Reusable workflow inputs and secret contracts for mod upload
- GitHub Environment preference when publishing from GitHub-hosted workflows
- Full support for the `mbround18/valheim-mods` repository and its mixed-language mod build graph (Rust, C#, and other mod toolchain languages)
- End-to-end mod lifecycle support: build, test, package, validate, and upload
- Support for both Hexium and Thunderstore publishers
- Validation of mod manifest/package metadata before remote upload
- Dagger `@func()` interfaces for upload and validation orchestration
- Clear failure semantics and output reporting for dry-run and publish modes
- Masking, redaction, and permission scoping for credentials and audit outputs

### Out of scope

- New registry-specific upload clients beyond Hexium and Thunderstore support
- Uploading arbitrary binary artifacts outside the package/mod bundle contract
- Non-GitHub hosted CI providers as first-class publishing targets
- Automatic promotion to a registry without explicit caller approval or validation success
- Removal of existing workflows or behavior when the upload step is not configured

## Affected Contracts

- Reusable workflow contracts:
  - `mod_upload` job inputs must include `provider`, `environment`, `validate_only`, `dry_run`, `package_path`, `team`, `access_key`, `username`, `password`, `artifact_name`, and optional `allow_unvalidated_publish`.
  - Reusable workflow secrets must resolve in precedence order: GitHub Environment > workflow env > repo-level secret fallback, with explicit failure if required values are missing.
  - Output contract must include a stable `status`, `provider`, `artifact`, `validation_result`, `upload_url`, and `publish_id` when available.
  - `valheim_mod_build_test_package` reusable workflow inputs must include `language_matrix`, `mod_root`, `build_target`, `test_target`, `package_target`, `artifact_name`, `registry_provider`, and optional `publish_on_success`.
  - For the `mbround18/valheim-mods` scenario, the workflow must support mixed-language build orchestration across Rust, C#, and other language modules without forcing a single-language assumption.

- Dagger module `@func()` contracts:
  - `validate_mod_package(provider: Provider, package_path: Path, metadata: ModMetadata) -> ValidationResult`
  - `publish_mod_package(provider: Provider, package_path: Path, credentials: UploadCredentials, team: Optional[str], dry_run: bool) -> UploadResult`
  - `resolve_upload_credentials(env: EnvironmentConfig) -> ResolvedCredentials`
  - `build_upload_request(...) -> UploadRequest`
  - `scan_mod_package(...) -> PackageScanReport`
  - `build_mod_artifact(repo_root: Path, language_matrix: List[str], build_target: Optional[str]) -> BuildResult`
  - `test_mod_artifact(repo_root: Path, language_matrix: List[str], test_target: Optional[str]) -> TestResult`
  - `package_mod_artifact(repo_root: Path, package_target: Optional[str], manifest: ModManifest) -> PackageResult`
  - `run_valheim_mod_pipeline(repo_root: Path, build: bool, test: bool, package: bool, upload: bool, provider: Provider) -> PipelineResult`

- Downstream consumer behavior:
  - Existing callers that do not pass a provider or credentials remain unaffected.
  - Upload is disabled by default unless the caller opts in through workflow inputs or a Dagger module invocation.
  - Validation failure blocks publication even when a credential set is available.

## Runtime and Defaults Impact

- `defaults.json` changes:
  - `mod_upload.enabled: false` (default safety gate)
  - `mod_upload.validate_before_upload: true`
  - `mod_upload.dry_run: false`
  - `mod_upload.provider: "none"`
  - `mod_upload.environment_preference: "github_environment"`
  - `mod_upload.require_explicit_provider: true`

- Runtime/container implications:
  - The validation container must include mod-manifest tooling for Thunderstore and Hexium packaging checks.
  - The upload step must run in a non-root, ephemeral workspace to avoid persisting credentials or package snapshots.
  - Temporary package staging directories must be cleaned after upload or validation failure.
  - Artifact hashing and manifest validation should run before the upload request is constructed.

- Backward compatibility:
  - This is backward-compatible because upload remains opt-in; existing workflows and CI jobs continue to work unchanged unless they explicitly request the upload pipeline.
  - Existing outputs and public workflow inputs remain valid if omitted; new inputs default to safe no-op behavior.

## Security and Permissions Impact

- Required permission changes and rationale:
  - Add `contents: read` for repository contents and manifest inspection.
  - Add `id-token: write` only if the environment requires OIDC-based provider auth; otherwise avoid it.
  - Add `environment: <name>` usage for environment-scoped deployments and secrets.
  - Do not require `write-all` or broad repository-level write privileges for the publish step.

- Secret handling requirements:
  - Prefer GitHub Environment secrets for `ACCESS_KEY`, `USERNAME`, `PASSWORD`, and `TEAM`.
  - If both `ACCESS_KEY` and user/pass forms are present, the workflow MUST use a deterministic precedence: `ACCESS_KEY` for providers that support it, otherwise `USERNAME` + `PASSWORD` + `TEAM`.
  - Credentials MUST never be echoed to logs, job summaries, or debug output.
  - Secrets must be masked and redacted before upload requests are constructed.

- Scanner/policy changes:
  - Pre-upload validation must include manifest schema validation and package integrity checks before remote publication.
  - Secrets scanning must fail if credentials are found in generated artifact manifests, built package files, or workflow summary output.
  - Secret redaction must be enforced in job logs and any debug artifacts.
  - Validation failure must produce a non-zero exit status and a clear reason code for the caller.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Publish a mod to Hexium from a GitHub Environment (Priority: P1)

As a maintainer, I want to publish a mod package to Hexium using a GitHub Environment and a scoped `ACCESS_KEY`, so that only the intended environment can push the artifact and failing packages do not get uploaded.

**Why this priority**: Hexium is a direct publishing target and is the simplest high-value path for safe, environment-based deployment.

**Independent Test**: run the reusable workflow against a test package in a GitHub Environment with an `ACCESS_KEY` configured and assert the upload request succeeds only after validation passes.

**Acceptance Scenarios**:

1. **Given** a valid Hexium package and a GitHub Environment with `ACCESS_KEY`, **When** the upload pipeline runs, **Then** it resolves the environment-scoped secret and uploads the package after validation succeeds.
2. **Given** a malformed or invalid package manifest, **When** the workflow runs, **Then** validation fails before the upload request is sent and the job exits non-zero.
3. **Given** the required secret is missing, **When** the workflow runs, **Then** the job fails with a clear message and no upload request is attempted.

---

### User Story 2 - Publish a mod to Thunderstore with username/password credentials (Priority: P1)

As a maintainer, I want to upload to Thunderstore using `USERNAME`, `PASSWORD`, and `TEAM` inputs, so the same pipeline works across registries with different auth contracts.

**Why this priority**: Thunderstore requires the multi-field credential pattern and is a high-frequency publishing scenario for mod packages.

**Independent Test**: run the same upload flow against a test package and confirm the request uses the resolved username/password/team combination and only publishes after manifest validation passes.

**Acceptance Scenarios**:

1. **Given** a valid Thunderstore package with `USERNAME`/`PASSWORD`/`TEAM`, **When** the upload workflow runs, **Then** it resolves those values from the GitHub Environment or workflow metadata and uploads the package.
2. **Given** the `TEAM` value is missing, **When** the workflow runs, **Then** it fails validation for the upload request before contacting the registry API.
3. **Given** the manifest is invalid according to Thunderstore rules, **When** the workflow runs, **Then** the upload is blocked and the failure is explained with the validation output.

---

### User Story 3 - Validation-only mode and dry-run safety (Priority: P2)

As a maintainer, I want to run validation-only or dry-run uploads, so I can confirm the package shape and registry contract without publishing an artifact.

**Why this priority**: This reduces failed releases and improves confidence before any external publish.

**Independent Test**: run the validation-only mode and confirm the package passes schema checks without making a network call to either registry API.

**Acceptance Scenarios**:

1. **Given** a package with valid metadata but `validate_only=true`, **When** the pipeline runs, **Then** it validates and reports success without posting to the registry.
2. **Given** a dry-run request with provider credentials present, **When** the workflow runs, **Then** it does not publish the artifact but emits a structured validation result and upload preview.

---

### Edge Cases

- What happens when both `ACCESS_KEY` and `USERNAME`/`PASSWORD` are present? → The workflow resolves one deterministic precedence and refuses to merge conflicting credentials silently.
- What happens when a package fails manifest validation? → The upload stage is skipped and a validation report is emitted to the workflow summary.
- What happens when a GitHub Environment is not configured? → The workflow fails with a clear error unless the caller explicitly allows fallback to repo-level secrets, which must be a documented, opt-in behavior.
- What happens if the registry requires a different package layout than the provided artifact? → The package is rejected before upload and a structured validation error is returned.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The reusable workflow MUST support a provider selection for Hexium and Thunderstore, with a required explicit provider value when upload is enabled.
- **FR-002**: The workflow MUST prefer GitHub Environment secrets and variables over repository-level secrets when resolving `ACCESS_KEY`, `USERNAME`, `PASSWORD`, and `TEAM`.
- **FR-003**: The pipeline MUST support both auth patterns: `ACCESS_KEY` for providers that accept it and `USERNAME` + `PASSWORD` + `TEAM` for Thunderstore-style credentials.
- **FR-004**: Validation MUST run before any upload request is sent, and invalid package metadata MUST fail the job with a non-zero exit code.
- **FR-005**: The Dagger upload flow MUST expose public `@func()` signatures that accept explicit upload credentials, provider selection, package path, and validation mode.
- **FR-006**: The workflow MUST not emit raw secret values to stdout, stderr, logs, or job summaries.
- **FR-007**: When validation-only or dry-run is requested, the system MUST report status without performing a remote publish.
- **FR-008**: If the target registry is unsupported or the required auth contract is missing, the workflow MUST fail fast with a clear, machine-readable error reason.
- **FR-009**: The system MUST output a stable status object containing success/failure, provider, validation status, and remote upload metadata when available.
- **FR-010**: The upload path MUST be backward-compatible by default: no upload occurs unless the caller explicitly enables it.
- **FR-011**: The workflow MUST support the `mbround18/valheim-mods` repository layout and mixed-language mod build graph, including Rust, C#, and additional language modules in a single mod project pipeline.
- **FR-012**: The end-to-end mod pipeline MUST support build, test, package, validation, and upload as explicit stages, with each stage able to run independently or as a full chain.
- **FR-013**: The pipeline MUST fail if the build or test stage fails before a package is created or uploaded, and it MUST not skip packaging when a package artifact would be empty or malformed.
- **FR-014**: The package step MUST produce registry-compatible output for the selected provider and must validate the final archive before network upload begins.

### Key Entities

- **Upload Target**: One of the supported registry providers (`hexium`, `thunderstore`).
- **Credential Set**: Either an `ACCESS_KEY` credential or a resolved `USERNAME` + `PASSWORD` + `TEAM` tuple.
- **Validation Result**: The result of schema and package checks before remote publication.
- **Upload Request**: The final registry payload assembled only after validation passes.
- **GitHub Environment Scope**: The environment-bounded configuration source whose values override generic repository defaults.
- **Valheim Mod Project**: A mixed-language mod repository (for example `mbround18/valheim-mods`) composed of buildable Rust/C#/other modules that must be built, tested, packaged, and uploaded as one coherent mod artifact.
- **Build/Test/Package Pipeline**: The end-to-end mod process that compiles the project, runs tests, produces the distributable package, validates packaging, and then publishes it to the chosen registry.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A valid package can be published to Hexium using GitHub Environment-scoped `ACCESS_KEY` credentials with no repository secret leakage.
- **SC-002**: A valid package can be published to Thunderstore using `USERNAME` + `PASSWORD` + `TEAM` values, with validation failing before publish if the manifest is invalid.
- **SC-003**: Validation-only and dry-run modes complete without making a network upload request while returning structured pass/fail results.
- **SC-004**: 100% of failing validation cases result in a blocked publish and a human-readable reason in the workflow output.
- **SC-005**: The workflow remains backward-compatible for all existing callers that do not enable upload.
- **SC-006**: A `mbround18/valheim-mods`-style project can run through a complete build/test/package/upload pipeline for Rust, C#, or other supported mod languages, with each stage emitting a clear pass/fail result and with upload blocked on any failed stage.
- **SC-007**: Multi-language package assembly produces a final artifact that passes registry validation and does not upload if the built package is empty, structurally broken, or mismatched to the manifest.

## Assumptions

- GitHub Actions environment-scoped secrets are the preferred runtime source of truth for publish credentials.
- A package is only valid when its metadata conforms to registry-specific requirements and local validation passes.
- The publish pipeline will treat credential resolution and manifest validation as public contracts, not as private implementation details.
- Registry APIs may reject invalid packages even when the upload request is otherwise well-formed, so local validation is required before network calls.
- Existing workflows without upload configuration remain valid and will not be modified automatically.
- The `mbround18/valheim-mods` repository is a first-class target and may contain mixed-language build steps, generated artifacts, and packaging logic that must be orchestrated in a single configurable pipeline.
- The build/test/package/upload workflow is defined as a generic mod pipeline and not as a repo-specific hack: any comparable mod repo with Rust/C#/other-language modules should be able to use the same contract.

## Runtime and Defaults Impact

- `defaults.json` update summary:
  - default upload disabled; safe no-op baseline
  - validation before publish enabled by default
  - environment preference enabled
  - explicit provider required for any upload action

- Container impacts:
  - Add registry validator tooling and package integrity commands to the CI container image.
  - Keep the upload process isolated from repository state and ephemeral workspace state.
  - Ensure package extraction and validation run in a secure scratch directory to avoid accidental leakage of secrets into artifacts.

## Security and Permissions Impact

- Required permissions and controls:
  - `contents: read` for source and package inspection
  - `environment: <name>` for environment-scoped secret consumption
  - minimal secret exposure; no secret values in logs
  - pre-flight validation to prevent malformed package uploads

- Security scan behavior changes:
  - Scanner must be updated to treat generated upload metadata and package manifest files as sensitive if they contain registry credentials.
  - Validation steps must fail on latent secrets or invalid package structure.
  - The scan contract must include a rule to reject secret-like values in build artifacts and logs.

## Validation Plan

- Run a real workflow in a GitHub Environment with valid Hexium `ACCESS_KEY` credentials and verify a valid package uploads only after the manifest validator passes.
- Run a real workflow with valid Thunderstore `USERNAME`/`PASSWORD`/`TEAM` values and verify successful upload with a manifest-valid mod bundle.
- Run validation-only mode on a deliberately malformed package and confirm the upload request is absent while the reason is reported clearly.
- Run a dry-run against a valid package and confirm there is no registry network call and the job emits the expected preview output.
- Verify the workflow fails fast when a required secret or provider is missing.
- Check all logs and summaries for masked secret output and confirm no credential values are exposed.

## Rollout and Rollback

- Rollout strategy:
  - Introduce the feature behind a default-disabled upload gate in `defaults.json`.
  - Release it as an additive workflow capability with the provider and auth contract explicitly documented.
  - Enable opt-in adoption by teams that need registry publishing and maintain a visible validation summary in the workflow results.

- Rollback strategy:
  - Set `mod_upload.enabled=false` in the runtime defaults to disable all upload actions without removing validation support.
  - Revert to the prior no-publish baseline in the workflow by removing the upload step from the call site.
  - Keep validation-only and dry-run stages available to allow safe rollback while preserving package integrity checks.
  - If a provider-specific issue occurs, disable that provider by setting `mod_upload.provider="none"` and maintain the rest of the pipeline unchanged.

## Assumptions

- GitHub Actions environment-scoped configuration is the preferred execution context for mod publication.
- A registry upload should be prevented whenever validation fails or required credentials are absent.
- This feature is additive and should not require changes to existing workflows that do not opt in.
- Mod validation is an explicit step, not an optional afterthought, because invalid registry packages are costly and hard to retract.
