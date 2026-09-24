# `paws` CLI reference

Every command, and the conventions they share.

This page is written by hand and explains *why* the surface is shaped the way it is.
[`llms.txt`](../llms.txt) is the generated, machine-readable companion: it lists every flag
verbatim from the same `clap` definition the binary parses, so it can never drift. When the two
disagree, `llms.txt` (and `paws <command> --help`) is right.

Regenerate `llms.txt` with `paws llms generate`; CI does it automatically on every push to `main`.

## Contents

- [Conventions](#conventions)
- [Commands](#commands)
- [Environment variables](#environment-variables)
- [`paws.toml`](#pawstoml)
- [Known inconsistencies](#known-inconsistencies)

## Conventions

These hold across the whole surface. Learning them once means most flags need no lookup.

**Everything is a long flag.** There are no positional arguments and no short flags except
`-h`/`-V`. A `paws` invocation reads the same in a workflow file as it does in a shell, and adding
a flag can never change how an existing one is parsed.

**Nothing is written or published unless you ask.** Every command's default is the read-only or
build-only path: `paws docker` builds without pushing, `paws helm` lints without packaging,
`paws docs` builds rustdoc without publishing, `paws changelog` writes the local file and prints
the entry without committing. The verb that causes an outside effect is always an explicit flag —
`--push`, `--publish`, `--commit`, `--package`. Adding one of those flags is the only way to reach
a network write.

**A flag's value is validated at parse time when the set is fixed.** `--toolchain`, `--toolchains`
and `--increment` reject an unknown value before any work starts, list the accepted values in
`--help`, and suggest the nearest match on a typo. They read the same registries the dispatch
itself reads, so the list cannot fall behind the implementation.

**Comma-separated or repeated, interchangeably.** Every multi-value flag accepts both
`--image a,b` and `--image a --image b`. Where two such flags are paired — `--image` with
`--target`, `--package` with `--binary-name` — they are matched 1:1 in the order given.

**`--repository` is always `owner/repo`, and always falls back to `$GITHUB_REPOSITORY`.** So is
every other GitHub input: the flag is there for running outside Actions, and the env var means you
never have to thread it through a workflow.

**`--silent` suppresses live progress, it does not reduce output.** Commands that drive a Dagger
build stream its progress to the terminal by default. `--silent` holds the output until the
pipeline finishes, then prints it — on failure you still get everything. It is for log-capture
contexts, not for quieting a build.

**`--dry-run` resolves and prints, and touches nothing.** On `paws assign` it prints who would be
assigned; on `paws publish` it builds, tests and packages but skips the registry.

**Names are shared across commands.** `--pr-labels`, `--repository`, `--branch`, `--source`,
`--output`, `--silent`, `--verbose` and `--dry-run` mean the same thing wherever they appear. Where
a name was ambiguous it was changed and the old spelling kept as an alias — see
[Known inconsistencies](#known-inconsistencies).

## Commands

### `paws init`

Installs the `dagger` CLI. Most other commands need it on `PATH`; `actions/paws-up` runs this for
you unless you pass `install-dagger: false`. Pin which version it installs with `paws.toml`'s
`[tools] dagger = "..."` — unpinned, two runs weeks apart can leave different engines behind.

### `paws ci`

Builds, lints and tests a project in one of 15 toolchains. `--toolchain` is the only required
input; each toolchain detects its own project layout from there (package manager, framework, test
runner, build system) rather than needing more flags.

`--toolchain-version` pins the toolchain. Omitted, `paws` reads the version file the ecosystem
already uses, then `paws.toml`, then its own default — and prints which one it used and why. A
native version file deliberately outranks `paws.toml`, so `paws ci` never builds against a
different compiler than a local build in the same directory.

Three flags are gated to one toolchain each, and say so: `--targets` (go), `--coverage` (rust),
`--publish-artifacts` (esp32).

### `paws docker`

Builds a container image, then tags and publishes it. Which tags get pushed — and whether anything
is pushed at all — is decided from the ref the build is running on, not from a flag: a push to
`--default-branch` or any tag push publishes; a feature branch or PR does not, unless it carries
`--canary-label` or you pass `--push`.

`--image` and `--target` together build several images from one Dockerfile in one run, against one
engine, so whatever the stages share is built once rather than once per invocation.

The tag flags are additive and each defaults off: `--with-latest`, `--tag-rollup` (`:3` and `:3.2`
alongside `:3.2.1`), `--tag-sha`, `--tag-branch`, `--tag-pr`, `--tag-schedule`. `--version-prefix`
(or its `--no-prefix` shorthand) controls the prefix on the version tag and its rollup cascade.

Note that `--label` sets OCI labels on the built image, while `--pr-labels` supplies the pull
request's labels for the push gating above. They are unrelated.

### `paws semver`

Computes the next version. The increment comes from `--increment` if given, otherwise the merged
PR's labels, otherwise the branch name, otherwise patch. On a push to `main` the PR labels are
looked up automatically from the commit, because `github.event.pull_request` does not exist on a
push event.

Runs fully offline when `--base` names the version to bump from:

```sh
paws semver --base v1.2.3 --increment minor    # -> v1.3.0
```

Without `--base` it resolves the last matching tag from the GitHub API, which needs
`$GITHUB_REPOSITORY` and a token. `--push` additionally creates the annotated tag and the matching
GitHub Release through the API — no local git identity or worktree needed.

### `paws changelog`

Renders a `CHANGELOG.md` entry for `--version` from the commit/PR history since the previous tag.
Writes the local file and prints the entry; `--commit` also commits it back through the Contents
API, with a `[skip ci]` marker so it cannot retrigger itself.

### `paws release`

Cross-compiles, smoke-tests, packages and uploads a release binary per `--target`. Ask the binary
what it can build with `--list-targets` rather than keeping your own copy of the list.
`--no-upload` stops before GitHub. `--local-build` builds against paws's embedded generic Linux
builder, for repos that have no `builders/` directory of their own.

The smoke test is what catches a binary that builds but does not run; `--skip-smoke-test` exists
and is not recommended.

### `paws publish`

Publishes a package to its registry — `--target rust-crate` (crates.io or another Cargo registry)
today. `--dry-run` verifies a package is publish-ready without a registry token.

### `paws helm`

Lints every chart it finds (`charts/*/Chart.yaml`, or a root `Chart.yaml`). `--package` also packages
them to `--output`. `--publish` instead does a per-chart GitHub Release plus a real `index.yaml` on
`--pages-branch`, so `helm repo add` against the repo's Pages URL works; it packages internally, so
it is mutually exclusive with `--package`.

### `paws audit`

Runs the security/compliance scanner suite and summarizes the findings. No flags.

### `paws docs`

Builds rustdoc for the workspace. `--provider github-pages` also publishes it, auto-selecting the
Git Trees or Pages-deployment mechanism from the repository's live Pages configuration.
`cloudflare-pages` and `s3` are recognized and fail immediately as unimplemented, rather than being
silently ignored — see [ROADMAP.md](ROADMAP.md).

### `paws provision`

Installs toolchains concurrently rather than one at a time. `--toolchains` accepts a narrower set
than `paws ci --toolchain`: only the ecosystems with a real installer (`rust`, `node`, `python`,
`go`, `esp32`). A JDK or a Ruby has no single obviously-right version manager the way
`rustup`/`corepack`/`uv` do, so `paws` expects those to already be on the runner.

`--toolchain` is accepted as an alias, since `paws ci` spells it singular.

### `paws assign`

Assigns an issue or PR to its `CODEOWNERS` — a PR to the owners of the files it changes, an issue
to the owners of the catch-all `*` rule. GitHub has no default-assignee setting, and `CODEOWNERS`
alone only requests reviews. Teams and email entries are skipped (GitHub cannot assign them);
anything already assigned is left alone unless you pass `--force`. `--dry-run` prints who would be
assigned.

### `paws cache`

Reports which Dagger build-cache backend (`dagger-cloud`, `github-actions`, or none) `paws ci` and
`paws docker` would select right now, and why — running the same detection they use internally.
`--json` is for a CI step asserting the expected backend actually activated, without grepping build
log text.

### `paws workflow generate`

Detects a repo's ecosystem(s) and emits a starter CI workflow wiring in `actions/paws-up` plus the
matching `paws` commands, to `--output`. `--provider github` is the only origin implemented today.

### `paws llms generate`

Regenerates [`llms.txt`](../llms.txt) from this CLI's own `clap` metadata, so it cannot drift from
real behavior. `--publish` commits it through the Contents API, skipping the commit when the content
is unchanged.

### `paws mcp setup` / `paws mcp serve`

`setup` writes or merges an MCP client config (`--client claude-code` for a project-local
`.mcp.json`, `--client claude-desktop` for the global one). `serve` runs the server over stdio,
exposing every command above as an MCP tool that calls the same Rust functions the CLI calls — not a
subprocess proxy. One argument definition drives both surfaces, so an MCP tool's inputs are exactly
its command's flags.

### `paws auth github-app`

Mints a GitHub App installation access token and prints it to stdout, and nothing else, so
`TOKEN=$(paws auth github-app)` works. Every command that needs a GitHub token already picks up App
auth automatically from the same env vars; this exists for handing the raw token to another tool.

## Environment variables

The full table is in [`llms.txt`](../llms.txt), generated alongside the flag reference. In short:

- `$GITHUB_REPOSITORY`, `$GITHUB_SHA`, `$GITHUB_REF`, `$GITHUB_REF_NAME` and `$GITHUB_EVENT_PATH`
  are the fallbacks for the corresponding flags, and are already set inside GitHub Actions.
- `$GITHUB_TOKEN` (or `$GH_TOKEN`) authenticates every GitHub API path.
- `$GH_APP_CLIENT_ID` plus `$GH_APP_PRIVATE_KEY` work in place of a token: `paws` mints a
  short-lived installation token itself, so no separate token-minting Action is needed.
- `$DOCKERHUB_USERNAME`/`$DOCKER_TOKEN` and `$GHCR_USERNAME`/`$GHCR_TOKEN` authenticate the two
  registries with dedicated flags. Any other registry derives its token variable from its host:
  uppercased, non-alphanumerics replaced with `_`, suffixed `_TOKEN`.
- `$PAWS_CACHE_SAVE` and `$PAWS_CACHE_MAX_BYTES` control engine-state caching. See
  [README.md](../README.md#build-cache-on-github-actions).

## `paws.toml`

Optional, at the repository root. Needed only for toolchains with no native version file, or to pin
the tools `paws` itself installs and runs:

```toml
[toolchains]
ruby = "3.3.0"
dotnet = "9.0"

[tools]
dagger = "0.18.10"    # what `paws init` installs
semgrep = "1.99.0"    # the scanner images `paws audit` runs
```

## Known inconsistencies

Recorded rather than quietly tolerated, so the next person to notice one finds out whether it is
deliberate.

**`--labels` was renamed to `--pr-labels`** on `paws docker` and `paws semver`. On `paws docker` it
sat one character away from `--label`, which sets OCI image labels and is entirely unrelated.
`--labels` still works as an alias on both commands, so existing workflows do not break.

**`paws release` never streams build progress.** Every other Dagger-driven command streams by
default and takes `--silent` to buffer; `release` always buffers, and has no `--silent` flag because
there is nothing to suppress. It is also the slowest command, so it is the one where live progress
would help most. Adding streaming means switching its `paws_dagger::core` calls to
`core_streaming`, which needs verifying against all six cross-compile targets.

**`--json` exists only on `paws cache`.** Nothing else has a machine-readable output mode, so
scripting any other command means parsing its human-readable text. `paws semver` partly compensates
by writing `$GITHUB_OUTPUT`.

**`--output` means a file on `paws llms`/`workflow`/`changelog`, and a directory on `paws helm`.**
Each matches what that command produces, but the name does not distinguish them.

**`paws llms` and `paws workflow` each have exactly one subcommand**, `generate`, so `paws llms`
alone does nothing. Kept as a group deliberately: both are expected to grow siblings (a `validate`,
another origin), and flattening them now would be a breaking change to un-break later.
