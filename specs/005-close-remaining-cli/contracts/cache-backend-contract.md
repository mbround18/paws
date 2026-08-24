# Contract: `CacheBackend` (Dagger build-cache reuse)

## 1) Selection contract (FR-005)

No new CLI flag anywhere (`paws docker`, `paws ci`) — selection is automatic, checked once per
invocation, in this fixed order:

1. `$DAGGER_CLOUD_TOKEN` present → `CacheBackend::DaggerCloud`.
2. Else, `$ACTIONS_CACHE_URL` or `$ACTIONS_RESULTS_URL` present → `CacheBackend::GitHubActionsCache`.
3. Else → `CacheBackend::None` (today's behavior, unchanged — FR-007).

## 2) Verifiability contract (FR-006)

Exactly one log line per invocation naming the selected backend (or `none`), emitted before the
`dagger` subprocess is invoked — never inferred indirectly from build timing.

## 3) `DaggerCloud` provider contract

- No new code beyond detection + the log line (research.md R6) — `$DAGGER_CLOUD_TOKEN` already
  reaches the `dagger` subprocess via normal environment inheritance.
- Degrades to `None`'s behavior (not a hard failure) if the token turns out to be invalid/expired
  at the Dagger Cloud service level (Edge Cases) — `paws` cannot itself validate the token before
  handing it to `dagger`.

## 4) `GitHubActionsCache` provider contract

- Only activates inside a real GitHub Actions job (same detection signature
  `paws_environment::CiContext::detect()`'s `Provider::GitHub` already uses).
- Wraps the existing `paws_dagger::core`/`core_streaming` call sites: restore before, save after —
  no change to either function's public signature.
- Uses the GitHub Actions Cache REST API directly (the same API `actions/cache@vN` itself calls),
  not the `actions/cache` GitHub Action — `paws` has no mechanism to invoke another Action from
  inside its own subprocess model (Constitution Principle II's single-call-site spirit extends
  here: no shelling out to `actions/cache`, just the same REST API it uses).

## 5) Backward-compatibility contract (FR-007)

With neither `$DAGGER_CLOUD_TOKEN` nor `$ACTIONS_CACHE_URL`/`$ACTIONS_RESULTS_URL` set,
`paws docker`/`paws ci`'s Dagger invocation is byte-identical to today — same subprocess args,
same behavior, just one additional `cache: no backend detected (full rebuild)` log line.
