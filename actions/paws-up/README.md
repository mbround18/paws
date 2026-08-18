# paws up

Downloads a `paws` release binary for the runner it's running on, puts it on `PATH`, and runs
`paws init` to install the `dagger` CLI (most `paws` subcommands need it). Composite, not
Docker-based — deliberately, so `paws`'s own Dagger calls talk to the runner's real Docker daemon
directly, with no Docker-in-Docker nesting to work around.

## Usage

```yaml
- uses: mbround18/paws/actions/paws-up@main
  with:
    version: v0.0.1-prerelease.1 # or omit for the most recent release

- run: paws ci --toolchain rust
```

## Inputs

| Input | Default | Description |
| --- | --- | --- |
| `version` | `latest` | A specific tag (e.g. `v0.0.1-prerelease.1`), or `latest` for the most recent GitHub Release. **Prereleases are included** when resolving `latest` — this action exists to dogfood `paws` fast, including prerelease iteration, not just pin to stable. |
| `github-token` | `${{ github.token }}` | Used for the release-list/download API calls, to avoid low anonymous rate limits. |
| `install-dagger` | `true` | Also run `paws init` to install the `dagger` CLI. Set to `"false"` to skip if the runner already has it. |

## Outputs

| Output | Description |
| --- | --- |
| `version` | The resolved version that was actually installed (useful when `version: latest` was requested). |

## Platform resolution

OS and architecture come from `uname`; on Linux, gnu vs. musl libc is auto-detected (checking for
musl's loader at `/lib/ld-musl-*.so.1`) rather than assumed, since `uname` alone can't tell you
that. The resolved target must be one of `paws_release::known_targets()` in
`crates/paws-release/src/lib.rs` — see the root [`README.md`](../../README.md)'s command table
and [`docs/DEVELOPMENT.md`](../../docs/DEVELOPMENT.md#releases) for the current target matrix.

The binary is installed to `~/.local/bin` and that directory is added to `$GITHUB_PATH`, then
`paws --version` is run once as a sanity check before the step succeeds — a download that
"succeeded" but produced a binary that doesn't run should fail loudly here, not surface as a
confusing failure in whatever step tries to use `paws` next.
