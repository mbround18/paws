# Contract: `paws_dagger::CacheBackend::detect()`

## Public shape (unchanged from `005`)

```rust
pub enum CacheBackend {
    DaggerCloud,
    GitHubActionsCache { base_url: String, token: String },
    None,
}

impl CacheBackend {
    pub fn detect() -> Self;
    pub fn log_line(&self) -> String;
}
```

No new variant, no new field, no new method, no new parameter. Same signature, same enum.

## Behavior change (this spec)

| `$DAGGER_CLOUD_TOKEN` | `$ACTIONS_CACHE_URL` | `$ACTIONS_RUNTIME_TOKEN` | Before this spec | After this spec |
| --- | --- | --- | --- | --- |
| set (any value) | — | — | `DaggerCloud` | `DaggerCloud` (unchanged) |
| unset | non-empty | non-empty | `GitHubActionsCache` | `GitHubActionsCache` (unchanged) |
| unset | `""` (empty) | non-empty | `GitHubActionsCache { base_url: "", .. }` (**bug**) | `None` (fixed) |
| unset | non-empty | `""` (empty) | `GitHubActionsCache { .., token: "" }` (**bug**) | `None` (fixed) |
| unset | `""` | `""` | `GitHubActionsCache { "", "" }` (**bug**) | `None` (fixed) |
| unset | unset | unset | `None` | `None` (unchanged) |

Only the empty-string rows change behavior. Every row `005`'s existing tests already cover
(non-empty-vs-unset) is unaffected.

## Test obligations (FR-007)

- Existing `005` tests for `detect()`'s precedent (`DaggerCloud` wins; unset → `None`; both
  non-empty → `GitHubActionsCache` with the right `base_url`/`token`) MUST continue to pass
  unmodified.
- New negative test: with `$ACTIONS_CACHE_URL`/`$ACTIONS_RUNTIME_TOKEN` both set to `""` in the
  test process env, `detect()` MUST return `CacheBackend::None`.
- New negative test (one-sided empty): one var non-empty, the other `""` — MUST return `None`,
  not a `GitHubActionsCache` with a half-empty field.

## Non-goals

- Does not change `DaggerCloud` vs. `GitHubActionsCache` precedence.
- Does not add `$ACTIONS_RESULTS_URL` support.
- Does not change `log_line()`'s three message strings.
