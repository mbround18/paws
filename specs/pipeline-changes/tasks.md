# Tasks: Mod Upload and Validation Pipeline

## Rules

- Keep changes backward-compatible unless explicitly declared breaking.
- Pair contract changes with tests in the same PR.
- Keep docs/examples in sync with behavior.
- Treat reusable workflow inputs, secrets, and Dagger function signatures as public contracts.
- Keep the upload path disabled by default until a caller opts in.

## Tasks

### Phase 1: Setup

- [x] T001 Define the public mod-upload defaults and opt-in guardrails in `crates/paws-core/src/lib.rs` and `specs/pipeline-changes/data-model.md`
- [x] T002 Confirm the existing workflow entry points and action conventions in `.github/workflows/release.yaml` and `actions/paws-up/action.yml`

### Phase 2: Foundational

- [x] T003 Add shared credential-resolution types and secret-masking helpers in `crates/paws-core/src/config.rs`
- [x] T004 Add provider enum, validation result models, and package metadata structs in `crates/paws-publish/src/lib.rs`
- [x] T005 Add initial Dagger orchestration contract stubs and status/result wrappers in `crates/paws-dagger/src/lib.rs`

### Phase 3: User Story 1 - Hexium env-backed publish

- [x] T006 [US1] Implement Hexium upload request construction and validation-before-upload gating in `crates/paws-publish/src/lib.rs`
- [x] T007 [US1] Implement `ACCESS_KEY` env precedence and missing-secret failure behavior in `crates/paws-core/src/config.rs`
- [x] T008 [US1] Add hexium-specific tests for valid upload, validation failure, and missing credential rejection in `crates/paws-publish/src/lib.rs`
- [x] T009 [US1] Wire the reusable workflow inputs/outputs for Hexium publish in `.github/workflows/release.yaml`

### Phase 4: User Story 2 - Thunderstore auth + validation

- [x] T010 [US2] Implement Thunderstore credential normalization for `USERNAME`, `PASSWORD`, and `TEAM` in `crates/paws-publish/src/lib.rs`
- [x] T011 [US2] Add manifest validation, package-shape checks, and registry-specific rejection logic in `crates/paws-publish/src/lib.rs`
- [x] T012 [US2] Add Thunderstore tests for username/password precedence, missing `TEAM`, and invalid manifest blocking in `crates/paws-publish/src/lib.rs`
- [x] T013 [US2] Update the public contract doc to include Thunderstore auth semantics in `specs/pipeline-changes/contracts/mod-upload-contract.md`

### Phase 5: User Story 3 - Validation-only and mixed-language mod workflows

- [x] T014 [US3] Implement the build/test/package stage contract for mixed-language repos in `crates/paws-publish/src/lib.rs` and `crates/paws-rust/src/lib.rs`
- [x] T015 [US3] Add package scan and validation preflight checks for `mbround18/valheim-mods`-style artifacts in `crates/paws-publish/src/lib.rs`
- [x] T016 [US3] Add Dagger `@func()` pipeline handlers for `build_mod_artifact`, `test_mod_artifact`, `package_mod_artifact`, `validate_mod_package`, and `run_valheim_mod_pipeline` in `crates/paws-dagger/src/lib.rs`
- [x] T017 [US3] Add tests for `validate_only`, `dry_run`, and package failure gating in `crates/paws-publish/src/lib.rs`

### Phase 6: Polish & cross-cutting concerns

- [x] T018 Add structured error reasons, masked logs, and secret redaction to all upload failures in `crates/paws-core/src/config.rs` and `crates/paws-publish/src/lib.rs`
- [x] T019 Update the validation and release quickstart for Hexium, Thunderstore, and mixed-language mod repos in `specs/pipeline-changes/quickstart.md`
- [x] T020 Add CI smoke coverage for environment precedence and validation-block behavior in `.github/workflows/ci.yaml`
- [x] T021 Final contract verification for workflow defaults, no-op uploads, and rollback semantics in `specs/pipeline-changes/spec.md` and `specs/pipeline-changes/plan.md`

## Dependencies

- Phase 1 must complete before any implementation work begins.
- Phase 2 blocks all user story work and shared contract definitions.
- User Story 1 and User Story 2 can proceed in parallel after Phase 2.
- User Story 3 depends on the successful completion of both US1 and US2 contract logic.
- Phase 6 runs after all story tasks complete and validates the rollout and rollback story.

## Parallel execution examples

- Parallelizable tasks after Phase 2:
  - `T006` + `T007` + `T009` for the Hexium path
  - `T010` + `T011` + `T013` for the Thunderstore path
  - `T014` + `T015` + `T016` for the mixed-language pipeline path
- The polish phase should be run once all story-specific validation passes.

## Implementation strategy

- MVP: implement only the Hexium path and validation-before-upload gating, then verify with a valid/invalid package matrix.
- Incremental delivery: add Thunderstore auth normalization next, then add the mixed-language build/test/package orchestration and dry-run paths.
- Release safety: keep the default `mod_upload.enabled` value at `false` until the workflow contract and validation tests are green.

## Story completion criteria

### US1 independent test criteria

- A valid Hexium package resolves `ACCESS_KEY` from the GitHub Environment and publishes only after validation passes.
- An invalid package fails before any upload request is built or sent.
- Missing credentials fail fast and return a structured error code.

### US2 independent test criteria

- A valid Thunderstore package resolves `USERNAME`/`PASSWORD`/`TEAM` from the environment and uploads after validation passes.
- A missing `TEAM` or a malformed manifest blocks upload and returns a clear failure reason.
- Secret values never appear in job logs or output summaries.

### US3 independent test criteria

- A mixed-language mod repo can run through build → test → package → validate → optional upload with explicit stage pass/fail reporting.
- `validate_only` or `dry_run` reports the same structured status without contacting the registry.
- Empty or mismatched package artifacts fail before network publication.

## Summary

- Total task count: 21
- Task count per user story: US1 = 4, US2 = 4, US3 = 4
- Parallel opportunities identified: 3 concrete task groups after the foundational phase
- Suggested MVP scope: User Story 1 with validation-before-upload and default opt-in gating
- Format validation: all tasks follow the required checklist format (`- [ ] T### ...`) with explicit file paths and story labels where required
