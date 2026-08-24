# Research: Exposing GitHub Actions Cache Runtime Vars to `paws-up`

## R1: Is a JS/Node step strictly required to read `$ACTIONS_RUNTIME_TOKEN`/`$ACTIONS_CACHE_URL`?

- **Decision**: Yes — confirmed, not assumed. `ACTIONS_RUNTIME_TOKEN` and `ACTIONS_CACHE_URL`
  are populated by the `actions/runner` process into the *runner's own* process environment
  before it launches a step's process, but a plain `run:` (bash) step is executed as a
  freshly-spawned child process that does **not** inherit them — the runner only forwards a
  curated environment (job env, `GITHUB_*` context vars, secrets explicitly referenced) to a
  shell step. A JS-based action step, by contrast, runs *inside* the runner's own Node.js
  worker process (`@actions/exec`'s `toolrunner` invokes the action's `main.js` in-process via
  `require`, not as a separate subprocess with a filtered env), so `process.env` there is the
  runner's real internal environment, vars and all. This is exactly why
  `crazy-max/ghaction-github-runtime` (and Dagger's own published GitHub Actions cache-volume
  guide, which recommends that exact action) exists: it's a ~15-line JS action whose entire body
  is `core.exportVariable('ACTIONS_RUNTIME_TOKEN', process.env['ACTIONS_RUNTIME_TOKEN'])` (and
  the `ACTIONS_CACHE_URL` equivalent), writing to `$GITHUB_ENV` so later shell steps inherit them
  as normal job env for the rest of the job.
- **Alternatives considered**:
  - *Parse a well-known file/socket the runner exposes.* No such stable, documented surface
    exists for these two vars outside the Node action environment; the runner does not write
    them to a step-readable file.
  - *Read them via the `github` context in the workflow YAML itself and pass as an input.* The
    `github` context is populated from the workflow/event payload and repo settings, not from
    the runner's internal actions-runtime environment — `ACTIONS_RUNTIME_TOKEN`/
    `ACTIONS_CACHE_URL` are never exposed there either.
  - *Have `paws docker`/`paws ci` itself fetch a cache-service token via the GitHub API.* No
    public API issues this token; it's minted internally by the runner for the job's lifetime
    and is not retrievable except from the in-process Node action environment.
- **Conclusion**: a JS/Node-based step is the only confirmed mechanism. FR-002's tradeoff is
  real and must be resolved (R2), not sidestepped.

## R2: Third-party action vs. `paws`-native JS/TS action vs. `actions/github-script`

- **Decision**: use `actions/github-script`, GitHub's own first-party JS action, with a small
  inline script (embedded directly in `paws-up/action.yml`, no external repo dependency beyond
  `actions/github-script` itself) that does the same two `core.exportVariable` calls
  `crazy-max/ghaction-github-runtime` does. Pin `actions/github-script` to a specific commit SHA
  (not `@v7`), matching FR-003.
- **Rationale**:
  - It resolves the exact tradeoff named in Risks and Mitigations without either horn: it does
    **not** introduce a narrow, single-purpose third-party action whose maintenance is out of
    this project's control (the risk named for option (a)), and it does **not** require
    `paws` to build, publish, and maintain its own compiled JS/TS action — a new tooling
    category and build/release pipeline this repo has none of today (the risk named for
    option (b), i.e. "first-ever JS-action tooling").
  - `actions/github-script` is maintained by GitHub itself (`actions/` org), is one of the most
    widely used actions in the entire GitHub Actions ecosystem, and already ships exactly the
    `@actions/core`/`@actions/github` toolkit needed — the inline script is ~4 lines, trivially
    auditable in a code review of `paws-up/action.yml` itself with no separate repo to vet.
  - It still satisfies FR-003 (pin to a SHA) and the Security and Permissions Impact section's
    "reviewed for what else it does with the runner environment" requirement — `github-script`'s
    own action code is well-known and does nothing with `ACTIONS_RUNTIME_TOKEN` on its own; the
    behavior that touches the token lives in `paws-up`'s own inline script, fully visible in this
    repo, not hidden inside a third-party action's compiled `dist/index.js`.
- **Alternatives considered**:
  - `crazy-max/ghaction-github-runtime` pinned to a SHA: works, but is exactly the narrow
    single-maintainer third-party dependency the Risks section flags — its entire compiled
    `dist/` would need review (not just its README) to confirm it does only what it claims, and
    every future `paws-up` change would need to re-verify no supply-chain drift if the pin is
    ever bumped.
  - A `paws`-native compiled JS/TS action shipped from `actions/paws-runtime-vars/`: would need
    its own `package.json`, build step, and `dist/` commit convention (the standard GitHub
    Actions JS-action pattern) — new tooling surface with no precedent anywhere else in this
    bash-only-actions repo, for a ~4-line script. Rejected as scope creep relative to
    `actions/github-script` achieving the identical outcome with zero new build tooling.
- **Mechanism**: a new step in `actions/paws-up/action.yml`, placed before the existing
  `paws init` step (so the vars are already exported to `$GITHUB_ENV` by the time any subsequent
  step — including `paws init` itself, if it ever needs them, and definitely any consumer step
  invoking `paws docker`/`paws ci`) runs:

  ```yaml
  - uses: actions/github-script@<pinned-sha> # vX.Y.Z
    if: ${{ always() }}
    with:
      script: |
        core.exportVariable('ACTIONS_RUNTIME_TOKEN', process.env['ACTIONS_RUNTIME_TOKEN'] || '')
        core.exportVariable('ACTIONS_CACHE_URL', process.env['ACTIONS_CACHE_URL'] || '')
  ```

  Exporting an empty string when a var is genuinely absent (e.g. a context where the legacy
  Cache Service v1 API isn't available at all) is safe: `CacheBackend::detect()` uses
  `std::env::var(...).is_ok()`/pattern-matched `Ok(...)` semantics on both vars together, and an
  empty string is still `Ok("")`, which — **this must be handled**, see R3 — would otherwise
  cause a false-positive `GitHubActionsCache { base_url: "", token: "" }` selection, violating
  FR-005. Resolved in R3.

## R3: Avoiding a false-positive `GitHubActionsCache` selection when the vars are exported empty

- **Decision**: `CacheBackend::detect()` must treat an empty-string value for either var as
  "absent," not merely check `Ok(_)`. Change `detect()`'s `GitHubActionsCache` branch to
  additionally require both values be non-empty after `env::var()` succeeds — this is the
  smallest change that closes the gap without altering `detect()`'s existing precedence logic
  (`DaggerCloud` still wins when both signatures are present) or its public shape (still just
  `CacheBackend::detect() -> Self`, same enum, no new params).
- **Rationale**: `actions/github-script`'s inline step in R2 always exports both vars (so the
  step is unconditionally idempotent and side-effect-free to add — no `if:` branching needed on
  whether they exist yet), falling back to `''` when the runner-internal env doesn't have them
  (a self-hosted runner without cache-service access, or in principle any future runner
  configuration where they're absent). Without this change, every job — including ones with no
  real cache-service access — would present `ACTIONS_CACHE_URL=""`/`ACTIONS_RUNTIME_TOKEN=""` as
  `Ok("")` to `std::env::var`, and `detect()`'s existing `if let (Ok(_), Ok(_))` match would
  incorrectly select `GitHubActionsCache` with empty/useless credentials — precisely the
  false-positive FR-005/FR-007/edge-case-2 forbid.
- **Alternatives considered**:
  - *Don't export empty strings; only call `core.exportVariable` conditionally, in JS, guarded
    on `process.env['ACTIONS_RUNTIME_TOKEN']` being truthy.* Considered and preferred as the
    simpler fix — it avoids ever writing an empty var to `$GITHUB_ENV` at all, so `detect()`
    genuinely never observes `Ok("")` and needs no change. **Selected instead of the Rust-side
    guard as the primary fix** (see Design Decisions in plan.md); the Rust-side non-empty check
    is *also* added as defense-in-depth per FR-005's "MUST NOT fabricate... in every context"
    wording, since a future caller of `CacheBackend::detect()` (not necessarily `paws-up`) could
    otherwise reintroduce the same class of bug by exporting an empty var some other way.
  - *Leave `detect()` unchanged and rely solely on the JS step never exporting empties.* Rejected
    as a single point of failure for FR-005 — the whole point of this spec's Risk section is not
    repeating `005`'s "looked correct in isolation, broken in the real environment" pattern; a
    defense-in-depth check in `detect()` itself is cheap and directly testable via FR-007's unit
    test (env vars set to `""` explicitly).

## R4: `paws init` — does the fix belong there too?

- **Decision**: No change needed to `crates/paws-cli-core/src/lib.rs::run_init`. `run_init`
  installs the `dagger` CLI binary; it is not the process that later runs `paws docker`/`paws ci`
  in the same job (those are separate `run:` steps in the consumer's own workflow, or the
  `paws-up` composite's own steps). The var-exposure problem is entirely about what's in
  `$GITHUB_ENV` by the time *later* steps in the job run — that's a `paws-up` (workflow-level)
  concern, not something `paws init`'s own process environment can influence for other, later
  processes. Confirmed against Scope's own framing ("if the fix instead... belongs in `paws
  init`... resolved in plan.md, not assumed here") — resolved here as: it doesn't.
- **Alternatives considered**: having `paws init` itself shell out to read the vars and write
  `$GITHUB_ENV`. Rejected — `paws init` runs as a bash step's subprocess exactly like any other
  `paws` invocation, so it has the identical "vars aren't in this process's env at all" problem
  this whole spec exists to fix; it cannot read what was never given to it.

## R5: Validation approach for FR-004 (real-runner validation)

- **Decision**: add a CI job (or extend an existing one) in this repo's own
  `.github/workflows/` that uses the fixed `paws-up` against a trivial fixture Dockerfile, and
  asserts the job log contains `cache: using github-actions` (SC-001) via a `grep` step against
  the captured log, or by making `paws docker` write that specific line to `$GITHUB_STEP_SUMMARY`
  in addition to stdout for straightforward workflow-level assertion. Run twice in sequence
  (or as two dependent jobs) against an unchanged Dockerfile to produce the SC-003 timing
  comparison, using each job's own wall-clock duration (`$GITHUB_JOB` timing, visible in the
  Actions UI / retrievable via `gh run view --json jobs`) as the metric — no new instrumentation
  needed in `paws` itself.
- **Rationale**: matches the Validation Plan's own wording ("in `paws`'s own repo, or a
  throwaway fixture repo") and re-uses this repo's existing CI infrastructure rather than
  standing up a separate fixture repo, keeping the validation co-located with the fix and
  re-run automatically on every future `paws-up` change.
