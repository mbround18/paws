# Implementation Plan: Expose the GitHub Actions Cache Runtime Vars `GitHubActionsCache` Depends On

## Inputs

- Spec path: `specs/006-paws-doesn-expose/spec.md`
- Affected contracts/files:
  - `actions/paws-up/action.yml` (new step, before `paws init`)
  - `crates/paws-dagger/src/lib.rs` (`CacheBackend::detect()` — defense-in-depth non-empty check)
  - `docs/ROADMAP.md` (`005-close-remaining-cli` entry correction/confirmation)
  - `paws docker --help` / `paws ci --help` text, if either overpromises zero-setup activation
  - A new or extended `.github/workflows/*.yml` real-runner validation job (FR-004)

## Constitution Check

- **Principle II (Subprocess-First Dagger Access, Single Call Site)**: unaffected —
  `CacheBackend::detect()` and its callers are unchanged in *how* they invoke `dagger`; this
  spec only changes what's in the process environment before `detect()` runs. No new
  `Command::new("dagger")` call site introduced.
- **Principle IV (Parity Testing Over Reimplementation-From-Memory)**: N/A for the `paws-up`
  workflow change (no `gh-reusable` precedent for this — confirmed in spec's Assumptions,
  `gh-reusable` never had this capability). The `CacheBackend::detect()` Rust change is a
  narrowing of existing `005`-authored logic, not a port.
- **Principle V (Reliability & Testability First)**: FR-007 requires a negative-path unit test
  (empty-string env vars) alongside FR-004's real-runner positive-path validation — both ship in
  this change, satisfying "contract changes MUST be paired with tests in the same PR."
- **Technical Constraints — "No secrets on the command line"**: `ACTIONS_RUNTIME_TOKEN` is
  exported to `$GITHUB_ENV`, not passed as a CLI flag or logged — consistent with this
  constraint; the token reaches `paws docker`/`paws ci` the same way `DAGGER_CLOUD_TOKEN` already
  does (inherited process environment).
- **Gate result**: PASS, no violations to justify.

## Design Decisions

- **Mechanism (FR-002, resolves spec's Open Question)**: add a step to `actions/paws-up/action.yml`
  using `actions/github-script`, pinned to a specific commit SHA, running an inline ~4-line
  script that reads `process.env['ACTIONS_RUNTIME_TOKEN']`/`process.env['ACTIONS_CACHE_URL']`
  (available to it because `github-script` runs in-process inside the runner's own Node worker,
  unlike a bash `run:` step) and calls `core.exportVariable` for each **only when the value is
  present** (guarded, not unconditional — see below), writing them into `$GITHUB_ENV` for every
  later step in the job, including any consumer step invoking `paws docker`/`paws ci`. Full
  rationale and rejected alternatives (a third-party action, a `paws`-native JS/TS action) are in
  `research.md` R1/R2.
- **Placement**: the new step runs *before* the existing `paws init` step in `paws-up`, so the
  vars are already in job env by the time anything Dagger-related runs.
- **Empty-value handling (FR-005, resolves research.md R3)**: the inline script only calls
  `core.exportVariable` when `process.env[...]` is truthy — it does **not** export an empty
  string when a var is genuinely absent (e.g. self-hosted runner without legacy Cache Service v1
  access). This means a context without real cache-service access simply never gets these two
  vars added to `$GITHUB_ENV` at all, and `CacheBackend::detect()`'s existing `if let (Ok(_),
  Ok(_))` match correctly falls through to `None` with **no Rust-side code change required** for
  correctness in the common case.
- **Defense-in-depth (FR-005, still required)**: `CacheBackend::detect()` additionally rejects an
  empty-string value for either var even though the primary fix (guarded export) should never
  produce one — this closes the failure mode where a future caller of the same env-population
  pattern (a different composite action, a consumer's own pre-existing `ghaction-github-runtime`
  step per edge-case-3) exports `""` instead of omitting the var. Implemented as a `.filter(|v|
  !v.is_empty())` (or equivalent) on both `env::var()` results before the branch matches.
  `detect()`'s public shape, precedent (`DaggerCloud` wins when both present), and the enum
  itself are unchanged (Affected Contracts: "No change to `CacheBackend`'s public shape").
- **`paws init` untouched** (research.md R4) — the fix is entirely workflow-level; `run_init`
  gains no new responsibility.
- **Idempotency vs. a consumer's own pre-existing export step** (edge case 3): `core
  .exportVariable`/`$GITHUB_ENV` writes are last-write-wins for the remainder of the job by
  design (documented GitHub Actions behavior) — no special-casing needed; `paws-up`'s step simply
  runs and, if a consumer's workflow already exported the same vars earlier or later, whichever
  runs last wins, and both write the same real values sourced from the same runner-internal
  environment, so there's no meaningful divergence to guard against.
- **Failure degradation** (edge case 1): `actions/github-script`'s step failing/timing out is
  treated as any composite-action step failure — it does **not** get `continue-on-error: true`,
  because a failure there is either (a) `actions/github-script` itself being broken (rare, would
  also break countless other consumers' workflows, treated as an infra incident not a paws-up
  concern) or (b) a genuine problem worth surfacing rather than silently masking. Absence of the
  two vars afterward still degrades cleanly to `CacheBackend::None` per existing FR-005/FR-007
  logic — the step's job is only to populate env vars, and downstream `detect()` behavior is
  unaffected by whether this step ran cleanly or the vars simply weren't there to begin with.

## Workstreams

1. **Workflow/action changes**: add the pinned `actions/github-script` step to
   `actions/paws-up/action.yml`, with a doc-comment (matching the file's existing style)
   explaining why it exists and what breaks if it's removed (cache silently stays off).
2. **Dagger module updates**: add the non-empty-value guard to
   `CacheBackend::detect()` in `crates/paws-dagger/src/lib.rs`; add the FR-007 negative-path unit
   test (env vars present but empty-string → `CacheBackend::None`), plus confirm existing
   `detect()` tests are unaffected.
3. **Governance/tests updates**: add/extend a real-GitHub-Actions-runner CI workflow validating
   FR-004/SC-001 (`cache: using github-actions` in the log) and SC-003 (second-run timing
   improvement against an unchanged Dockerfile), per research.md R5.
4. **Docs/examples updates**: update `docs/ROADMAP.md`'s `005-close-remaining-cli` entry and any
   `paws docker --help`/`paws ci --help` text to reflect that `paws-up` alone is now sufficient
   (FR-006) — confirming rather than rewriting if the existing text already avoids overpromising
   beyond "auto-selected (no CLI flag)."

## Contract-Safety Checklist

- [x] Workflow declarations and references stay consistent — `paws-up`'s new step is additive;
      no existing input/output/step id changes.
- [x] Dagger call names align with module `@func()` names — N/A, no Dagger module changes; this
      spec is entirely about GitHub Actions workflow env exposure, not Dagger module functions.
- [x] Runtime standards come from `defaults.json` — N/A, no new runtime default introduced (the
      pinned SHA for `actions/github-script` is an action-version pin, not a `PipelineDefaults`
      value; consistent with how other action pins in this repo are handled today).
- [x] Permissions are explicit and least-privilege — `actions/github-script`'s step needs no
      explicit `permissions:` grant beyond what `paws-up`'s consumers already carry (it doesn't
      call the GitHub API; it only reads its own process env and writes `$GITHUB_ENV`).
- [x] Security implications are documented — spec's own Security and Permissions Impact section,
      reaffirmed here: `ACTIONS_RUNTIME_TOKEN` exposure to later steps is the pre-existing `005`
      design assumption, not a new grant introduced by this spec.

## Validation Matrix

| Surface                    | Validation |
| --------------------------- | ---------- |
| Workflow governance        | `paws-up`'s new step reviewed for pinned-SHA compliance (FR-003); `actions/github-script`'s own action code is GitHub-maintained and not separately vetted line-by-line, but the inline script it runs is fully visible in this repo's own diff. |
| Module integration         | `cargo test -p paws-dagger` covers `detect()`'s existing precedent tests plus the new FR-007 empty-string negative test; `cargo test --workspace` passes with zero regressions (Validation Plan). |
| Security workflow behavior | `ACTIONS_RUNTIME_TOKEN` reaches later steps via `$GITHUB_ENV` only, never logged or passed as a CLI flag; the exporting script does nothing else with the runner environment. |
| Runtime defaults policy    | N/A — no new `PipelineDefaults` value; the SHA pin lives in `paws-up/action.yml` itself, matching precedent for other action version references in this repo. |
| Real-runner behavior (FR-004) | A CI job in this repo using the fixed `paws-up` asserts `cache: using github-actions` in the log (SC-001) and a materially faster second run against an unchanged Dockerfile (SC-003), per research.md R5. |
