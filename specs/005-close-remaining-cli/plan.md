# Implementation Plan: Close Remaining Gaps Found Migrating `valheim-docker`

## Inputs

- Spec path: `specs/005-close-remaining-cli/spec.md`
- Affected contracts/files:
  - `crates/paws-rust/src/lib.rs` (Gap 2: one-line clippy flag)
  - `crates/paws-audit/src/lib.rs` (Gap 4: new `ScannerName::CargoAudit` registry entry + `parse_cargo_audit_findings`)
  - `crates/paws-release/src/lib.rs` (Gap 1: `GitHubReleaseClient` gains `get_pages_config`/`create_blob`/`publish_tree`)
  - `crates/paws-docs/src/lib.rs` (Gap 1: `PublishTarget` provider dispatch)
  - `crates/paws-dagger/src/lib.rs` (Gap 3: `CacheBackend` detection/selection, wraps `core`/`core_streaming`)
  - `crates/paws-cli-core/src/lib.rs` (`DocsArgs` gains `--provider`/`--repository`/`--branch`; `run_docs` gains concurrent multi-provider dispatch)
  - `docs/ROADMAP.md` (Gap 1: `cloudflare-pages`/`s3` tracked as follow-ups; Gap 3/4 status updates)
  - Phase 0/1 artifacts: `research.md`, `data-model.md`, `contracts/*.md`, `quickstart.md` (all in this directory)

## Constitution Check

_GATE: evaluated before Phase 0 research; re-evaluated below after Phase 1 design._

| Principle | Assessment |
|---|---|
| I. One Crate Per Domain | No new crate — each gap's logic lands in the crate that already owns that domain (`paws-rust`, `paws-audit`, `paws-docs`, `paws-dagger`), matching how `004-rust-coverage` extended `paws-rust` in place. `paws-cli-core` stays thin: `run_docs`'s multi-provider dispatch is orchestration (which providers, in what order, aggregating results), not `github-pages`/`cloudflare-pages` publish logic itself, which belongs in `paws-docs`. **PASS**. |
| II. Subprocess-First Dagger Access | Gap 3's `CacheBackend` wraps `paws_dagger::core`/`core_streaming` — the two existing, sole call sites — rather than adding a new `Command::new("dagger")` anywhere else. No new subprocess spawn point. **PASS**. |
| III. Incremental SDK Adoption | Not applicable — no Dagger SDK involvement. **PASS (not applicable)**. |
| IV. Parity Testing Over Reimplementation-From-Memory | Explicitly not a `gh-reusable` port for any of the four gaps (spec's Assumptions section states this directly — none has a TS source to assert parity against). **PASS** (spec already reconciles this). |
| V. Reliability & Testability First | Every gap's FRs pair with named unit tests in the spec's own Validation Plan and this plan's Workstreams below. **PASS**, contingent on tasks.md enumerating each one. |
| Tech constraint: no secrets on CLI | `--repository`/`--branch` carry no secret; tokens resolve via `paws_environment::resolve_github_token`/env vars only, matching every other publish-capable subcommand. **PASS**. |
| Tech constraint: shared defaults live in one place | No new `PipelineDefaults` field needed for any of the four gaps — `--provider` has no default (explicit opt-in only, FR-002), `CacheBackend` selection is pure env-var detection, `cargo-audit`'s image/step-name live in `AUDIT_SCANNER_REGISTRY` alongside every other scanner's, matching the existing single-source-of-truth pattern for that catalog. **PASS**. |
| Tech constraint: no swallowed concurrent failures | **Directly on-point for this spec** — FR-002a's multi-provider docs-publish concurrency is exactly this constraint applied to a new orchestration site, resolved by reusing `paws-provision`'s already-compliant `JoinSet` pattern rather than writing new concurrent code that would need to prove compliance from scratch. **PASS**. |
| Tech constraint: no undeclared cross-task dependencies | FR-002a's providers have no ordering dependency on each other (each just needs the one shared `cargo doc` build, computed before any provider task starts, not awaited mid-task from within another provider). **PASS**. |

**Pre-Phase 0 Gate Status**: PASS, no unresolved conflicts.
**Post-Phase 1 Gate Status**: PASS — Phase 1 design (data-model.md, contracts/) surfaced two real environmental constraints (research.md R5, R7: Pages-deployment and Actions-cache mechanisms both only work inside a real GitHub Actions job) that don't change any Constitution assessment above but do shape FR-003/FR-005's error-handling requirements.

## Design Decisions

1. **`GitHubReleaseClient` gains three new methods rather than a new HTTP client** (research.md
   R4) — `get_pages_config`/`create_blob`/`publish_tree` reuse its existing `api_base()`/
   `auth_headers()` helpers. Alternative considered: a separate `PagesPublishClient` — rejected,
   since it would duplicate auth/base-URL logic `GitHubReleaseClient` already has for the same
   `owner`/`repo`/`token` triple.

2. **`cargo-audit` over `cargo-deny`** (research.md R2) — purpose-built, zero-config match for
   FR-008's specific RustSec-advisory ask, consistent with this catalog's existing
   zero-config-needed scanners. `cargo-deny`'s broader license/ban-list scope is real but
   out-of-scope per the spec's own Out of Scope section — not re-litigated here.

3. **`--provider`'s `github-pages` mechanism selection queries the target repo's live Pages
   config, not a CLI flag** (spec Clarifications) — and, per research.md R5, the `"workflow"`
   branch additionally checks for GitHub Actions' artifact-runtime env vars before attempting a
   deployment, since that API is unusable outside a real Actions job. This is a genuine
   environmental constraint discovered during planning, not assumed in the spec — FR-003's
   wording already accommodates it ("fails with a specific, actionable error").

4. **`CacheBackend::DaggerCloud` needs detection + a log line only; `GitHubActionsCache` needs a
   real restore/save wrapper** (research.md R6/R7) — these two providers are *not* symmetric in
   implementation cost despite being peers in the abstraction, and Design Decision 4 makes that
   asymmetry explicit rather than implying they're equal-effort alternatives. This asymmetry runs
   the *other* direction from provider importance, though: `GitHubActionsCache` needs no external
   account (Dagger Cloud is a paid/hosted third-party service, not everyone's willing default), so
   it's the provider most GitHub Actions-only consumers will actually use — Workstream 4 gives it
   equal or greater task weight than `DaggerCloud`, not treated as a secondary/fallback path just
   because `DaggerCloud`'s own code footprint happens to be smaller.

5. **FR-002a's multi-provider concurrency reuses `paws-provision::provision_with_timing`'s exact
   `JoinSet` shape**, not a new concurrency primitive (research.md R8) — the constitution's
   "no swallowed concurrent failures" constraint is already solved once in this workspace; this
   spec's job is applying that solution to a second orchestration site, not re-solving it.

6. **Gap 2 (clippy) ships as a deliberate default-behavior change, not gated behind a flag** —
   already decided in spec.md's Risks section; this plan does not revisit that decision, only
   confirms the one-line diff location (research.md R1).

## Workstreams

1. **Gap 2 — clippy gate** (`paws-rust`): one-line fix (research.md R1); pairs with a unit test
   asserting the non-wasm `push_exec` sequence now includes `-- -D warnings`, plus a
   fixture-based integration test proving a real warning now fails the pipeline (spec's Validation
   Plan).
2. **Gap 4 — `cargo-audit` scanner** (`paws-audit`): new `ScannerName::CargoAudit` +
   `AUDIT_SCANNER_REGISTRY` row (`ScannerFamily::Language(LanguageFamily::Rust)`, research.md
   R2/R3); `parse_cargo_audit_findings` mirroring `parse_semgrep_findings`; scanner script
   (`cargo install cargo-audit --locked && cargo audit --json`, `with-new-file` + `sh <path>`
   pattern matching existing scanners); confirm (not assume) Semgrep/Gitleaks's current
   report-don't-fail default before wiring this one the same way (spec's Assumptions note).
3. **Gap 1 — `paws docs` publish** (`paws-release`, `paws-docs`, `paws-cli-core`):
   `GitHubReleaseClient::get_pages_config`/`create_blob`/`publish_tree` (research.md R4);
   `PublishTarget` enum + `github-pages` provider implementation (mechanism auto-selection per
   R4/R5, `cloudflare-pages`/`s3` per FR-004a); `DocsArgs` gains `--provider`/`--repository`/
   `--branch`; `run_docs` gains the `JoinSet`-based multi-provider dispatch (research.md R8,
   contracts/paws-docs-publish-contract.md §4).
4. **Gap 3 — Dagger build cache** (`paws-dagger`): `CacheBackend` enum + `detect()` (research.md
   R6/R7, fixed precedence per Design Decision 4); `DaggerCloud` provider (detection + log line
   only); `GitHubActionsCache` provider (restore/save wrapper around `core`/`core_streaming`,
   using the Actions Cache REST API directly); the required "which backend was selected" log line
   (FR-006) at every `paws docker`/`paws ci` call site that goes through these two functions.
5. **Tests** (Constitution Principle V — pairs with every workstream above): unit tests per gap as
   named in spec's Validation Plan and this plan's Workstreams 1-4; a real two-separate-environment
   timing comparison for Gap 3's SC-004 (documented as a manual/CI-job validation in quickstart.md,
   not a `cargo test` case, matching `004-rust-coverage`'s own precedent for environment-dependent
   validation).
6. **Docs** — `docs/ROADMAP.md` gains entries for: Gap 3's `CacheBackend` (status, both providers),
   Gap 4's `cargo-audit` scanner (status), and Gap 1's `cloudflare-pages`/`s3` providers as
   explicit tracked follow-ups (not silently absent). `paws docs --help`'s own description is
   corrected to match actual behavior once `--provider github-pages` ships (closing the
   documentation-integrity gap Gap 1 exists to fix in the first place).

## Contract-Safety Checklist

- [x] Workflow declarations and references stay consistent — no `.github/workflows/*` changes in
      `paws` itself required by this spec (all four gaps are `paws`-internal capability additions);
      any consumer-side workflow adoption is downstream, out of scope (matches spec's Out of Scope)
- [x] Dagger call names align with module `@func()` names — N/A, no Dagger Cloud module involved;
      Gap 3 wraps the existing subprocess call sites, doesn't add new ones
- [x] Runtime standards come from a single shared source — `cargo-audit`'s image/step-name live in
      `AUDIT_SCANNER_REGISTRY` alongside every other scanner's, not hardcoded elsewhere
- [x] Permissions are explicit and least-privilege — Gap 1's `contents: write`-equivalent need
      matches `helm --publish`'s existing requirement exactly; Gap 3's Actions-cache provider needs
      only the same `actions: read/write`-equivalent scope `actions/cache`-style usage already
      implies; both documented in spec's Security and Permissions Impact
- [x] Security implications are documented — spec's Security and Permissions Impact section covers
      all four gaps; this plan adds no new surface beyond what that section and research.md R5/R7
      (environmental constraints on the Pages-deployment/Actions-cache mechanisms) already name

## Validation Matrix

| Surface | Validation |
| -------------------------- | ---------- |
| `paws-rust` clippy gate | `cargo test -p paws-rust` — new/existing tests per Workstream 1; SC-001/SC-002 |
| `paws-audit` `cargo-audit` scanner | `cargo test -p paws-audit` — catalog/gating/parsing tests per Workstream 2; SC-005 |
| `paws-release`/`paws-docs` publish | `cargo test -p paws-release -p paws-docs` — new client methods + provider dispatch per Workstream 3; SC-003 |
| `paws-dagger` `CacheBackend` | `cargo test -p paws-dagger` — detection/precedence tests per Workstream 4; SC-004 needs a real two-environment run (quickstart.md), not just unit tests |
| CLI wiring (`paws-cli-core`) | `--provider` parsing, multi-provider aggregation tests per Workstream 3/contracts §4 |
| Workspace-wide regression | `cargo test --workspace` — zero failures, zero changed expectations in any existing `paws-rust`/`paws-docs`/`paws-docker`/`paws-audit` test (spec's Validation Plan, Constitution Principle V) |
