# Research: Mod Upload and Validation Pipeline

## Decision 1: Use GitHub Environment precedence for credentials

**Decision**: The release workflow resolves credentials from GitHub Environment-scoped values before workflow or repository defaults. The auth contract is explicit: `ACCESS_KEY` or `USERNAME`/`PASSWORD`/`TEAM` are validated before upload begins.

**Rationale**: This matches the user's requirement to favor GitHub Environments, is consistent with secured release patterns, and keeps sensitive values out of repo-level configuration. It also protects production publish flows from accidental reuse of a generic secret set.

**Alternatives considered**:
- Repository secret only: rejected because it does not honor the environment-first requirement.
- Plain runner env only: rejected because it is weaker and less reviewable than GitHub Environments.

## Decision 2: Make validation mandatory before upload

**Decision**: The final package is validated in a dedicated pre-upload stage; invalid archives fail the job and never reach the registry API.

**Rationale**: Manifest validation reduces the chance of publishing broken or malformed mod packages. This aligns directly with the Thunderstore validator and Hexium packaging requirements and keeps registry operations idempotent and safe.

**Alternatives considered**:
- Upload first, validate later: rejected because it creates invalid-release risk.
- Optional validation: rejected because the user explicitly requested validation before upload.

## Decision 3: Treat the build/test/package pipeline as an explicit public contract

**Decision**: The workflow and Dagger interface will expose `build`, `test`, `package`, `validate`, and `upload` as separate stages, each independently testable and optionally runnable as a chained workflow.

**Rationale**: Mixed-language mod repos need individual stage observability. Users want a full process for `valheim-mods` projects, not just a single upload script. Separate stages make rollback and partial validation easier.

**Alternatives considered**:
- One monolithic script: rejected because it hides per-stage failure, lowers observability, and is harder to validate and rollback.
- Upload-only flow: rejected because it does not support the required full release lifecycle.

## Decision 4: Default to no-op publishing

**Decision**: The upload flow is disabled unless explicitly enabled through workflow inputs or Dagger options. Existing workflows remain unchanged.

**Rationale**: This preserves backward compatibility and makes the feature safe by default.

**Alternatives considered**:
- Publishing on every build: rejected because it would create accidental external side effects and fail the backward-compatibility requirement.

## Decision 5: Support a multi-language toolchain matrix for mod repos

**Decision**: The pipeline will accept a `language_matrix` config so a mod repo may build Rust, C#, and other supported toolchains before packaging.

**Rationale**: The requirement specifically calls out `mbround18/valheim-mods`, which is not a single-language project.

**Alternatives considered**:
- Hard-code Rust-only support: rejected because it would not satisfy the mixed-language requirement.
- Repo-specific one-off logic: rejected because it would not generalize to a reusable feature contract.

## Open questions resolved

- The platform will use GitHub Environment secrets as the first-class credential source.
- Validation is a required gate before remote upload.
- The workflow is opt-in and backward-compatible.
- The mod lifecycle is build → test → package → validate → upload.
- The contract supports both Hexium and Thunderstore.
