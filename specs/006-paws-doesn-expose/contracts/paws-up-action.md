# Contract: `actions/paws-up/action.yml`

## Before this spec

```yaml
runs:
  using: "composite"
  steps:
    - id: install
      shell: bash
      # ... downloads/installs the paws binary ...
    - if: ${{ inputs.install-dagger == 'true' }}
      shell: bash
      run: paws init
```

## After this spec

```yaml
runs:
  using: "composite"
  steps:
    - id: install
      shell: bash
      # ... downloads/installs the paws binary (unchanged) ...

    # Exposes $ACTIONS_RUNTIME_TOKEN/$ACTIONS_CACHE_URL to every later step in this job
    # via $GITHUB_ENV. GitHub Actions withholds these two vars from a plain `run:` (bash)
    # step's process environment — they only reach a JS/Node-based action step, which runs
    # in-process inside the runner's own worker. Without this step,
    # `paws_dagger::CacheBackend::detect()` can never select `GitHubActionsCache`; it always
    # falls through to `None` (full rebuild, but never wrong/broken — see FR-005/FR-007).
    # If this step (or its `actions/github-script` dependency) ever breaks, the only observed
    # effect is the cache backend silently staying `None` — nothing else in `paws-up` depends
    # on it. Pinned to a commit SHA, not a floating tag (FR-003).
    - uses: actions/github-script@<PINNED_SHA> # vX.Y.Z
      with:
        script: |
          const token = process.env['ACTIONS_RUNTIME_TOKEN'];
          const url = process.env['ACTIONS_CACHE_URL'];
          if (token) core.exportVariable('ACTIONS_RUNTIME_TOKEN', token);
          if (url) core.exportVariable('ACTIONS_CACHE_URL', url);

    - if: ${{ inputs.install-dagger == 'true' }}
      shell: bash
      run: paws init
```

## Contract guarantees

- **Inputs/outputs unchanged**: `version`, `github-token`, `install-dagger` inputs and the
  `version` output are untouched by this spec — no new required input, no new output.
- **Additive only**: exactly one new step is inserted; no existing step's `id`, `shell`, or
  behavior changes.
- **No new required consumer action**: a consumer using `paws-up` exactly as today (no
  workflow YAML change on their side) automatically gets the new step on their next run against
  `version: latest`, or on their next deliberate version bump if pinned.
- **Fails closed, not open**: if the new step's underlying vars are absent (non-GitHub-Actions
  context, or an Actions job without legacy Cache Service v1 access), no vars are exported and
  `CacheBackend::detect()` correctly resolves `None` — see
  [`cache-backend-detect.md`](./cache-backend-detect.md).
