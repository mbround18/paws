# Tasks: Paws Core CLI — Rust-native Reimplementation of gh-reusable's Pipeline Surface

## Rules

- Keep changes backward-compatible unless explicitly declared breaking (this is pre-1.0, but treat every shipped subcommand's flags as a contract from the moment it merges).
- Pair contract changes with tests in the same PR.
- Keep `README.md`'s subcommand list and this file's checklist in sync as work lands.
- Every ported behavior must name the exact `gh-reusable` file/function it's asserting parity against (plan.md's Contract-Safety Checklist).

## Tasks

### 0. Prerequisites

- [ ] Run `/speckit-constitution` to establish `paws`'s project principles formally (plan.md's Constitution Check flagged this as missing)
- [x] Confirm `actions/semver`'s tagless-repo default version by reading its TS source — resolved in spec.md FR-011 (`{prefix}0.0.0`, default prefix `v`, from `actions/semver/src/tag.js`)
- [x] Extract `docker-facts`'s multi-service `docker-compose.yml` resolution rule from source — resolved in spec.md FR-012 (first `image:`-matching service wins, from `packages/dagger-pipelines/src/docker-parity.ts`)

### 1. Workspace and CLI wiring

- [ ] Wire `paws-cli`'s `Ci`, `Docker`, `Semver`, `Audit`, `Docs` handlers to call through `paws-dagger::call` instead of printing `(unimplemented)`
- [ ] Add a `dagger` CLI presence check with the FR-010 actionable error message, run once at `paws` startup
- [ ] Add a CI job (or local script) that greps for `Command::new("dagger")` outside `crates/paws-dagger` and fails if found (SC-004)

### 2. `paws semver` (User Story 2, P1 — pilot crate)

- [ ] Create `crates/paws-semver` with `Cargo.toml` + `src/lib.rs`
- [ ] Port last-tag lookup (GitHub GraphQL query, matching `actions/semver`'s current query) behind a trait so it's mockable in tests
- [ ] Port increment precedence: explicit `--increment` > `major-label` > `minor-label` > `patch-label` (spec FR-003)
- [ ] Add fixture tests: tagged repo + label set, explicit increment override, tagless repo default (`{prefix}0.0.0` per resolved FR-011), PR-label-vs-branch-name precedence, prefix inference from existing `v`-prefixed tags
- [ ] Wire `paws-cli`'s `Semver` subcommand to `paws-semver`
- [ ] Write `specs/001-paws-core-cli/quickstart.md` using `paws semver` as the first working example

### 3. `paws audit` (User Story 4, P2)

- [ ] Create `crates/paws-audit` with `Cargo.toml` + `src/lib.rs`
- [ ] Port `AuditSummary`/`AuditScannerResult`/`ComplianceStatus` shapes from `packages/dagger-module/src/audit-types.ts`
- [ ] Port scanner aggregation logic from `audit-logic.ts` (confidence ranking, failed/skipped scanner handling)
- [ ] Add fixture tests: no-findings pass, single-finding summary shape match
- [ ] Wire `paws-cli`'s `Audit` subcommand to `paws-audit`

### 4. `paws docker` (User Story 3, P2)

- [ ] Create `crates/paws-docker` with `Cargo.toml` + `src/lib.rs`
- [ ] Port `docker-facts` Dockerfile/context discovery (compose-first, fallback to `./Dockerfile`/`.`)
- [ ] Port tag generation (branch/version/target/multi-registry) and push-gating (`canary_label`, `force_push`)
- [ ] Build the Docker e2e fixture harness (real or test-double daemon) required by spec FR-007
- [ ] Add fixture tests: compose-defined build, no-compose fallback, canary-label gating, force-push override, **multi-service compose file (one matching image, one not) per resolved FR-012**
- [ ] Wire `paws-cli`'s `Docker` subcommand to `paws-docker`

### 5. `paws provision` (User Story 5, P2 — concurrency foundation)

- [ ] Create `crates/paws-provision` with `Cargo.toml` (`tokio` dependency) + `src/lib.rs`
- [ ] Define the per-ecosystem installer shape (trait or async fn — decide during implementation per plan.md's Open Questions) for rust/rustup, node/pnpm, python/uv
- [ ] Implement the `JoinSet`-based orchestrator: launch one task per requested ecosystem, await all, return an aggregated `{ecosystem: Result<...>}` (FR-013)
- [ ] Verify no early-return-on-first-failure path exists — every ecosystem's outcome must be present in the aggregated result even when others fail (FR-014)
- [ ] Add a timing-based test: 3 mock installers with known sleep durations; assert total wall-clock ≈ max(durations), not sum (SC-005)
- [ ] Add a failure-isolation test: one of three mock installers fails; assert the other two still report success in the aggregate (FR-014)
- [ ] Wire `paws-cli`'s new `Provision` subcommand (`paws provision --toolchains rust,node,python`) to `paws-provision`
- [ ] Wire `paws ci` to call `paws-provision` internally when the target repo needs more than one ecosystem, instead of a sequential setup loop (FR-015)

### 6. `paws ci` (User Story 1, P1)

- [ ] Wire `paws ci --toolchain node` through `paws-dagger` (install/lint/test, matching `pnpmBuildAndTest`)
- [ ] Wire `paws ci --toolchain rust` through `paws-dagger` (matching `rustBuildAndTest`)
- [ ] Add real fixture projects (one Node, one Rust) exercised end-to-end in `paws`'s own CI (spec FR-008)

### 7. `paws docs` (User Story 6, P3)

- [ ] Wire `paws docs` to run `cargo doc` against the current workspace and produce a stable output path
- [ ] Confirm idempotency (safe to re-run without side effects)

### 8. Governance and CI bootstrap

- [ ] Decide and implement `paws`'s own CI: start with plain `cargo test --workspace` on GitHub Actions; revisit dogfooding `paws ci` once task 6 ships
- [ ] Add the SC-004 grep-lint as a required CI check
- [ ] Add `cargo test --workspace` as a required CI check (SC-002)
- [ ] Add the SC-005 provisioning-concurrency timing check as a required CI check once `paws-provision` ships

### 9. Documentation and propagation

- [ ] Update `README.md`'s crate layout section as `paws-semver`/`paws-audit`/`paws-docker`/`paws-provision` land
- [ ] Update `specs/001-paws-core-cli/quickstart.md` incrementally as each subcommand ships
- [ ] Note in this repo's `specs/002-reusable-rust-pipeline/` (inspiration copy) that it has been superseded by `001-paws-core-cli` for anything beyond pure reference
