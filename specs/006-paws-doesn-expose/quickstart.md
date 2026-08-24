# Quickstart: Validating GitHub Actions Cache Activation

## Prerequisites

- A GitHub Actions workflow (in this repo, or a throwaway fixture repo) with a job that:
  1. Uses the fixed `actions/paws-up` (this branch, or a release built from it).
  2. Runs `paws docker` (or `paws ci`) against any Dockerfile-having target — a trivial fixture
     Dockerfile is enough.
- Runs on a GitHub-hosted runner (the legacy Cache Service v1 API is not guaranteed present on
  arbitrary self-hosted runners).

## Positive-path validation (FR-004, SC-001)

1. Trigger the workflow.
2. Open the job log for the `paws docker`/`paws ci` step.
3. Confirm the log contains exactly: `cache: using github-actions`.
   - If it instead reads `cache: no backend detected (full rebuild)`, the fix did not activate —
     check that the new `actions/github-script` step in `paws-up` actually ran and exported both
     vars (inspect its own step log; `actions/github-script` prints nothing by default for a
     no-return script, so absence of an error is the expected/silent-success case).

## Negative-path validation (FR-005, SC-002)

1. Run `paws docker`/`paws ci` **outside** a GitHub Actions job (a bare local shell, or via a
   different CI provider if available) using a `paws-up`-equivalent-installed `paws` binary.
2. Confirm the log reads `cache: no backend detected (full rebuild)`, not
   `cache: using github-actions`.
3. Run `cargo test -p paws-dagger` and confirm the new empty-string negative-path unit tests
   (contracts/cache-backend-detect.md) pass.

## Timing validation (SC-003)

1. Run the positive-path workflow once against a Dockerfile with no local Dagger cache history
   (a cold run) — record wall-clock duration.
2. Without changing the Dockerfile, run the same workflow again in a separate job.
3. Confirm the second run's `paws docker`/`paws ci` step completes in materially less wall-clock
   time than the first, attributable to cache restoration (visible via the job's own duration in
   the Actions UI, or `gh run view --json jobs`).

## Full workspace regression check

```sh
cargo test --workspace
```

Must pass with zero regressions and no change to any existing `CacheBackend::detect()` test's
expected result (Validation Plan).
