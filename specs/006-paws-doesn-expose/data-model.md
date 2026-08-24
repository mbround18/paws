# Data Model: Expose GitHub Actions Cache Runtime Vars

This spec introduces no new persistent entities, database rows, or long-lived data structures.
It changes environment-variable *plumbing* between a workflow step and a later process, and
narrows one existing detection function's input validation. Documented here for completeness
per the plan template.

## Entities

### Runtime Token Exposure Step (workflow-level, not a Rust type)

- **What it is**: a step in `actions/paws-up/action.yml` (`uses: actions/github-script@<sha>`)
  that runs before the existing `paws init` step.
- **Fields** (inline script logic, not a struct):
  - Reads `process.env['ACTIONS_RUNTIME_TOKEN']` (string, may be absent).
  - Reads `process.env['ACTIONS_CACHE_URL']` (string, may be absent).
- **Behavior / validation rule**: for each of the two values, if truthy (non-empty), call
  `core.exportVariable(name, value)`; if absent/empty, do nothing for that var. Never writes an
  empty-string value to `$GITHUB_ENV` (this is the rule research.md R3 depends on).
- **Lifecycle**: runs once per job, writes to `$GITHUB_ENV` (a file the runner reads after each
  step to merge into subsequent steps' environment) — standard GitHub Actions job-env semantics,
  not a `paws`-managed lifecycle.

### `CacheBackend` (`crates/paws-dagger/src/lib.rs`, existing type — narrowed, not reshaped)

- **No new variants, no new fields.** Existing shape from `005` is unchanged:
  ```rust
  pub enum CacheBackend {
      DaggerCloud,
      GitHubActionsCache { base_url: String, token: String },
      None,
  }
  ```
- **Changed validation rule in `detect()`**: the `GitHubActionsCache` branch now additionally
  requires `base_url` and `token` be non-empty after `env::var()` succeeds — an `Ok("")` for
  either is treated the same as `Err(_)` (i.e. falls through toward `None`, or continues checking
  as if that var were absent). This is a stricter *input validation* rule on an existing function,
  not a new entity or a new field.
- **Relationships**: `CacheBackend::detect()`'s only relationship to the new workflow step is
  temporal/environmental — it reads whatever `$GITHUB_ENV` (merged into process env by the
  runner) contains by the time `paws docker`/`paws ci` invokes it. No direct code dependency
  between the two.

## State Transitions

None — `CacheBackend::detect()` remains a pure, stateless read of the current process
environment at call time, exactly as in `005`. This spec does not introduce caching of the
detection result, retries, or any multi-step state machine.
