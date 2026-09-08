//! The `clap` command-line surface: `Cli`, `Commands`, and one `*Args`
//! struct per subcommand.
//!
//! Split out of `lib.rs`, which held the whole CLI definition and every
//! `run_*` implementation in one 3,700-line file. These types are pure
//! declarations — they name flags, defaults and help text and contain no
//! behavior — so keeping them apart from the code that acts on them makes
//! both easier to read. They are re-exported from the crate root, so nothing
//! outside this crate changes.
//!
//! Every struct derives `Deserialize`/`JsonSchema` as well as `clap::Args`:
//! `paws-mcp` exposes each subcommand as an MCP tool by deserializing the
//! same type the CLI parses, so one definition drives both surfaces.

use clap::{Parser, Subcommand};
use paws_core::Toolchain;
use paws_semver::Increment;

/// Default values for `*Args` fields that carry a clap `default_value`
/// other than the field type's own `Default::default()`. `clap::Args`
/// applies these when a CLI flag is omitted; `serde::Deserialize` has no
/// equivalent unless each field also names one of these via
/// `#[serde(default = "...")]` — otherwise an MCP tool call that omits the
/// field (exactly as a CLI invocation omitting the flag would) fails
/// deserialization with "missing field", even though the CLI treats it as
/// optional. Every field below this fixes was found failing exactly that
/// way in `paws-mcp`'s own tests.
pub mod field_defaults {
    pub fn llms_txt() -> String {
        "llms.txt".to_string()
    }
    pub fn main_branch() -> String {
        "main".to_string()
    }
    pub fn canary() -> String {
        "canary".to_string()
    }
    pub fn major() -> String {
        "major".to_string()
    }
    pub fn minor() -> String {
        "minor".to_string()
    }
    pub fn patch() -> String {
        "patch".to_string()
    }
    pub fn paws_bot_name() -> String {
        "paws-bot".to_string()
    }
    pub fn paws_bot_email() -> String {
        "paws-bot@users.noreply.github.com".to_string()
    }
    pub fn dot() -> String {
        ".".to_string()
    }
    pub fn tmp() -> String {
        "tmp".to_string()
    }
    pub fn gh_pages() -> String {
        "gh-pages".to_string()
    }
    pub fn index_yaml() -> String {
        "index.yaml".to_string()
    }
    pub fn paws_cli_package() -> Vec<String> {
        vec!["paws-cli".to_string()]
    }
    pub fn paws_binary_name() -> Vec<String> {
        vec!["paws".to_string()]
    }
    pub fn github() -> String {
        "github".to_string()
    }
    pub fn paws_workflow_path() -> String {
        ".github/workflows/paws.yml".to_string()
    }
    pub fn changelog_path() -> String {
        paws_core::DEFAULT_CHANGELOG_PATH.to_string()
    }
}

/// paws: run-anywhere CI/CD pipelines, backed by Dagger.
#[derive(Debug, Parser)]
#[command(name = "paws", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Build and test a language target (node, rust, python, go, java, kotlin, ruby,
    /// php, dotnet, elixir, tauri, tauri-android, flatpak, esp32).
    Ci(CiArgs),
    /// Build and gate a container image the same way `docker-facts` + `docker-release` do.
    Docker(DockerArgs),
    /// Compute the next semantic version from PR labels or an explicit increment,
    /// matching `actions/semver`'s current behavior.
    Semver(SemverArgs),
    /// Install the `dagger` CLI (most other subcommands need it on PATH).
    Init(InitArgs),
    /// Run the audit/compliance scanner suite.
    Audit(AuditArgs),
    /// Build rustdoc; optionally publish it with --provider (github-pages
    /// implemented; cloudflare-pages/s3 recognized but not implemented yet).
    Docs(DocsArgs),
    /// Provision toolchains concurrently (rust, node, python, ...).
    Provision(ProvisionArgs),
    /// Lint (and optionally package) Helm chart(s) — `charts/*/Chart.yaml`
    /// or a root `Chart.yaml`.
    Helm(HelmArgs),
    /// Cross-target build, package, and publish a release binary to GitHub Releases.
    Release(ReleaseArgs),
    /// Model Context Protocol integration: expose every `paws` subcommand as
    /// an MCP tool, calling the same Rust code the CLI calls — not a CLI
    /// subprocess proxy.
    #[command(subcommand)]
    Mcp(McpCommand),
    /// Generate `llms.txt`, a machine-readable summary of `paws`'s own
    /// tooling surface (see <https://llmstxt.org>).
    #[command(subcommand)]
    Llms(LlmsCommand),
    /// Generate a starter CI workflow for a *consumer* repo: detects its
    /// ecosystem(s) and emits a workflow wiring in `paws-up` plus the
    /// matching `paws` subcommands.
    #[command(subcommand)]
    Workflow(WorkflowCommand),
    /// Credential helpers — mint tokens `paws` (or other tools) can use.
    #[command(subcommand)]
    Auth(AuthCommand),
    /// Publish a package to its registry (`--target rust-crate` for
    /// crates.io today).
    Publish(PublishArgs),
    /// Generate a `CHANGELOG.md` entry from commit/PR history between two
    /// refs — a `paws`-native replacement for `mbround18/auto` (and
    /// similar changelog actions), standalone so it can be run on its own
    /// (e.g. to preview an entry) or chained after `paws semver --push`.
    Changelog(ChangelogArgs),
    /// Reports which Dagger build-cache backend (`dagger-cloud`,
    /// `github-actions`, or none) `paws ci`/`paws docker` would select
    /// right now, and why — the same detection they use internally, not a
    /// separate guess. `--json` for scripting (e.g. a CI step asserting
    /// the expected backend actually activated, without grepping build log
    /// text for it).
    Cache(CacheArgs),
}

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Mint a GitHub App installation access token and print it to stdout
    /// (nothing else goes to stdout, so `TOKEN=$(paws auth github-app)`
    /// works as a shell capture) — the same mechanism
    /// `actions/create-github-app-token` provides as a separate Action,
    /// done natively so no extra CI step is needed. Every other `paws`
    /// subcommand that needs a GitHub token (`semver --push`, `helm
    /// --publish`, `release`, `llms generate --publish`) already picks up
    /// App auth automatically via the same `$GH_APP_CLIENT_ID`/
    /// `$GH_APP_PRIVATE_KEY` env vars — this subcommand exists for cases
    /// that want the raw token directly (e.g. handing it to another tool).
    GithubApp(GithubAppLoginArgs),
}

#[derive(Debug, Clone, clap::Args, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct GithubAppLoginArgs {
    /// The GitHub App's Client ID (the `Iv23...`-style string). Falls back
    /// to $`GH_APP_CLIENT_ID`.
    #[arg(long)]
    #[serde(default)]
    pub client_id: Option<String>,
    /// The GitHub App's private key, PEM-encoded, given directly. Falls
    /// back to $`GH_APP_PRIVATE_KEY`. Mutually exclusive with
    /// --private-key-file in practice — if both are given, the file wins.
    #[arg(long)]
    #[serde(default)]
    pub private_key: Option<String>,
    /// Path to a file containing the GitHub App's private key. Falls back
    /// to $`GH_APP_PRIVATE_KEY_FILE`.
    #[arg(long)]
    #[serde(default)]
    pub private_key_file: Option<String>,
    /// "owner/repo" the App is installed on. Falls back to
    /// $`GITHUB_REPOSITORY`.
    #[arg(long)]
    #[serde(default)]
    pub repository: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum McpCommand {
    /// Write/merge an MCP client config so `paws mcp serve` is discoverable.
    Setup(McpSetupArgs),
    /// Run the MCP server (stdio transport). Exposes every `paws` subcommand
    /// as an MCP tool by calling this crate's `run_*` functions directly.
    Serve(McpServeArgs),
}

#[derive(Debug, Clone, clap::Args, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct McpSetupArgs {
    /// Which MCP client config to write/merge into: "claude-code" (default,
    /// project-local `.mcp.json`) or "claude-desktop" (global,
    /// platform-specific `claude_desktop_config.json`).
    #[arg(long)]
    #[serde(default)]
    pub client: Option<String>,
}

#[derive(Debug, Clone, clap::Args, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct McpServeArgs {}

#[derive(Debug, Subcommand)]
pub enum LlmsCommand {
    /// Generate `llms.txt` from this CLI's own `--help` metadata.
    Generate(GenerateArgs),
}

#[derive(Debug, Subcommand)]
pub enum WorkflowCommand {
    /// Detect this repo's ecosystem(s) and generate a starter CI workflow.
    Generate(WorkflowGenerateArgs),
}

#[derive(Debug, Clone, clap::Args, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct WorkflowGenerateArgs {
    /// Which CI origin to generate for. Only "github" is implemented today
    /// — more origins (e.g. "gitlab") are planned; this keys off the same
    /// idea as `paws_environment::Provider` (currently GitHub-only too)
    /// rather than a separate abstraction.
    #[arg(long, default_value = "github")]
    #[serde(default = "field_defaults::github")]
    pub provider: String,
    /// Path to write the generated workflow file to.
    #[arg(long, default_value = ".github/workflows/paws.yml")]
    #[serde(default = "field_defaults::paws_workflow_path")]
    pub output: String,
}

#[derive(Debug, Clone, clap::Args, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct GenerateArgs {
    /// Path to write the generated file to.
    #[arg(long, default_value = "llms.txt")]
    #[serde(default = "field_defaults::llms_txt")]
    pub output: String,
    /// After generating, commit the file to GitHub via the Contents API
    /// (reuses `paws_release::GitHubReleaseClient`, the same mechanism
    /// `paws helm --publish` uses for `index.yaml` — no local git identity
    /// needed). Skips the commit if the generated content is unchanged.
    #[arg(long)]
    #[serde(default)]
    pub publish: bool,
    /// Branch to publish to. Only used with `--publish`.
    #[arg(long, default_value = "main")]
    #[serde(default = "field_defaults::main_branch")]
    pub branch: String,
    /// "owner/repo" to publish to. Falls back to $`GITHUB_REPOSITORY`. Only
    /// used with `--publish`.
    #[arg(long)]
    #[serde(default)]
    pub repository: Option<String>,
}

#[derive(Debug, Clone, clap::Args, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct CiArgs {
    /// Directory to build, relative to the current one. Defaults to the
    /// current directory.
    ///
    /// Lets a monorepo build a package in a subdirectory without the caller
    /// having to `cd` first — `paws ci --toolchain node --source web`.
    #[arg(long)]
    #[serde(default)]
    pub source: Option<String>,
    /// Which toolchain to build. Clap lists the accepted values from
    /// `paws_core::TOOLCHAINS`, so `--help` can't fall behind what `paws ci`
    /// actually dispatches.
    ///
    /// For `node`, the package manager
    /// (npm/yarn/pnpm/bun) and framework (Vite, Next.js, or plain) are
    /// auto-detected from lockfiles/package.json — no separate flag needed
    /// — and a Playwright e2e project (`@playwright/test` dependency or a
    /// playwright.config.*) is detected automatically too, running
    /// `npx playwright install --with-deps && npx playwright test`
    /// instead of the plain build+test pipeline. For `esp32`, builds an
    /// ESP-IDF/`esp-idf-sys` Rust firmware project (fmt/clippy/build, plus
    /// `cargo test` against any host-testable sibling crate — the embedded
    /// target itself has no test story) against a dedicated `builders/esp32`
    /// image (`espup`-installed ESP Rust toolchain, `libclang`, `espflash`).
    /// `ruby` (Bundler), `php` (Composer), `dotnet` (the .NET SDK), and
    /// `elixir` (Mix) each detect their own project layout too: the Ruby
    /// test runner (`rake` vs `rspec`), and whether a `PHPUnit` suite or a
    /// `Microsoft.NET.Test.Sdk` test project exists at all before running
    /// one.
    #[arg(long)]
    #[serde(default)]
    pub toolchain: Option<Toolchain>,
    /// Build against a specific version of `--toolchain`, e.g.
    /// `--toolchain rust --toolchain-version 1.90.0`.
    ///
    /// Omitted, `paws` reads the ecosystem's own version file
    /// (`rust-toolchain.toml`, `.nvmrc`, `.python-version`, `.ruby-version`,
    /// `go.mod`, `.tool-versions`, ...), then a `[toolchains]` entry in
    /// `paws.toml`, then its built-in default — and prints which one it used.
    /// Reading those files is what keeps `paws ci` on the same toolchain a
    /// local build uses.
    #[arg(long)]
    #[serde(default)]
    pub toolchain_version: Option<String>,
    /// Print per-ecosystem provisioning start/elapsed timing to stderr.
    #[arg(long)]
    #[serde(default)]
    pub verbose: bool,
    /// Suppress dagger's live build progress; only print output once
    /// the pipeline finishes (or on failure). Default is streamed live.
    #[arg(long)]
    #[serde(default)]
    pub silent: bool,
    /// Cross-compile to these GOOS/GOARCH pairs instead of the native
    /// build, e.g. "linux/amd64,darwin/arm64,windows/amd64" — only valid
    /// with `--toolchain go`. Binaries land in `dist/` under the project
    /// root; `go test` is skipped for every target (none of them can run
    /// in this build container, native or not).
    #[arg(long, value_delimiter = ',')]
    #[serde(default)]
    pub targets: Vec<String>,
    /// Also run `cargo llvm-cov` after the normal test step and print a
    /// coverage summary — only valid with `--toolchain rust`. Builds
    /// against a dedicated `builders/rust` image (pre-installed
    /// `cargo-llvm-cov`) instead of pulling `rust:1-bookworm` directly;
    /// the default (flag omitted) pipeline is unaffected. A no-op on a
    /// wasm project (`cargo test` is already skipped there for the same
    /// reason coverage can't be measured).
    #[arg(long)]
    #[serde(default)]
    pub coverage: bool,
    /// After a successful build, upload the built bootloader
    /// (`bootloader.bin`) and firmware ELF as assets on the GitHub Release
    /// matching the current tag ($`GITHUB_REF_NAME`) — only valid with
    /// `--toolchain esp32` (mirrors `--coverage`'s existing `--toolchain
    /// rust`-only gating). Needs $`GITHUB_TOKEN/$GH_TOKEN` and
    /// $`GITHUB_REPOSITORY` set — no new env var name, reusing the same
    /// convention every other GitHub-Release-touching `paws` subcommand
    /// already reads (`paws semver --push`, `paws helm --publish`). A
    /// missing token/tag fails with a clear, actionable error rather than a
    /// bare API 401. Default (flag omitted): no GitHub API interaction at
    /// all, same "additive flag changes nothing by default" shape as
    /// `--coverage`.
    #[arg(long)]
    #[serde(default)]
    pub publish_artifacts: bool,
}

#[derive(Debug, Clone, clap::Args, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct DockerArgs {
    /// Image name, e.g. "ghcr.io/example/app". Falls back to $`GITHUB_REPOSITORY`.
    /// A registry host here selects the registry to publish to — "ghcr.io/..."
    /// publishes to ghcr.io without also needing --registries. An unqualified
    /// name ("owner/app") is a Docker Hub reference, as docker itself reads it.
    #[arg(long)]
    #[serde(default)]
    pub image: Option<String>,
    /// Version to tag with. Falls back to $`GITHUB_SHA` (short).
    #[arg(long)]
    #[serde(default)]
    pub version: Option<String>,
    /// Additional registries to mirror tags into, comma-separated.
    #[arg(long, value_delimiter = ',')]
    #[serde(default)]
    pub registries: Vec<String>,
    /// Path to the Dockerfile to build, relative to the repo root. Falls
    /// back to auto-detection (`Dockerfile` at the repo root, or a
    /// `compose.yml` service's own `dockerfile`/`context`).
    #[arg(long)]
    #[serde(default)]
    pub dockerfile: Option<String>,
    /// Build context directory, relative to the repo root. Falls back to
    /// auto-detection alongside `--dockerfile`.
    #[arg(long)]
    #[serde(default)]
    pub context: Option<String>,
    /// PR label that, when present, pushes the image for that PR build too
    /// (normally only a push to --default-branch or a tag push pushes).
    #[arg(long, default_value = "canary")]
    #[serde(default = "field_defaults::canary")]
    pub canary_label: String,
    /// Force push regardless of branch/tag/label gating.
    #[arg(long)]
    #[serde(default)]
    pub push: bool,
    /// Also tag and push `:latest` alongside `--version`, but only when
    /// the build is actually pushing and the ref is a real (non-prerelease)
    /// version tag — a plain branch/PR build never gets `:latest`
    /// regardless of this flag.
    #[arg(long)]
    #[serde(default)]
    pub with_latest: bool,
    /// Build a specific stage of a multi-stage Dockerfile instead of the
    /// final stage.
    #[arg(long)]
    #[serde(default)]
    pub target: Option<String>,
    /// Prefix the image tag with `--target`'s name, e.g. `<target>-<version>`
    /// instead of just `<version>`. Only used with `--target`.
    #[arg(long)]
    #[serde(default)]
    pub prepend_target: bool,
    /// PR labels to check against --canary-label, comma-separated.
    #[arg(long, value_delimiter = ',')]
    #[serde(default)]
    pub labels: Vec<String>,
    /// The repo's default branch. A push directly to this branch, or any
    /// tag push, always pushes the image — --canary-label/--push only
    /// matter for everything else (feature branches, PRs).
    #[arg(long, default_value = "main")]
    #[serde(default = "field_defaults::main_branch")]
    pub default_branch: String,
    /// Docker Hub username to authenticate publishing with. Falls back to
    /// $`DOCKERHUB_USERNAME`. Required (here or via env) to actually push
    /// to docker.io — without it, `dockerRelease` builds but can't
    /// authenticate, so `push=true` still publishes nothing.
    #[arg(long)]
    #[serde(default)]
    pub dockerhub_username: Option<String>,
    /// GHCR username to authenticate publishing with. Falls back to
    /// $`GHCR_USERNAME`. Required (here or via env) to actually push to
    /// ghcr.io. The password is read from $`GHCR_TOKEN`, falling back to
    /// $`GITHUB_TOKEN` — which is what a GitHub Actions workflow already has.
    #[arg(long)]
    #[serde(default)]
    pub ghcr_username: Option<String>,
    /// Username(s) for registries in --registries other than
    /// docker.io/ghcr.io (Artifactory, a private registry, etc.), as
    /// "<registry>=<username>" pairs, comma-separated — e.g.
    /// "myco.jfrog.io=deploy-bot". These are built and published
    /// natively through Dagger (`Container.withRegistryAuth`), not via
    /// the docker.io/ghcr.io-only `dockerRelease` call. The matching
    /// token/password is read from an env var derived from the
    /// registry: uppercased, every non-alphanumeric character replaced
    /// with `_`, suffixed `_TOKEN` — e.g. "myco.jfrog.io" reads
    /// $`MYCO_JFROG_IO_TOKEN`.
    #[arg(long, value_delimiter = ',')]
    #[serde(default)]
    pub registry_username: Vec<String>,
    /// Suppress dagger's live build/publish progress; only print output
    /// once each pipeline finishes (or on failure). Default is streamed
    /// live.
    #[arg(long)]
    #[serde(default)]
    pub silent: bool,
    /// Also publish `major` and `major.minor` rollup tags (e.g. `:3` and
    /// `:3.2` alongside `:v3.2.1`) for release-quality version tags — the
    /// pattern consumers pinning to a major version for stability need.
    /// Gated identically to `--with-latest`: only on a real (non-prerelease)
    /// version tag build. Off by default; omitting this flag produces
    /// byte-identical output to before this flag existed. This is a
    /// `paws`-native tag scheme, not a byte-for-byte port of
    /// `crazy-max/ghaction-docker-meta`'s semver tag output.
    #[arg(long)]
    #[serde(default)]
    pub tag_rollup: bool,
    /// Also include a `sha-<sha>` tag unconditionally, alongside whatever
    /// other tags this build already produces — not only as the fallback
    /// primary tag when no version/ref-based tag applies (that fallback
    /// behavior is unaffected by this flag). Only produces a tag when
    /// `--version` is itself sha-shaped.
    #[arg(long)]
    #[serde(default)]
    pub tag_sha: bool,
    /// On a branch-push build (not a tag, not a PR, not a scheduled run),
    /// also tag with the branch name (`/` and other non-tag-safe
    /// characters replaced with `-`).
    #[arg(long)]
    #[serde(default)]
    pub tag_branch: bool,
    /// On a `pull_request`-triggered build, also tag with `pr-<number>`,
    /// where the number is parsed from `$GITHUB_REF`
    /// (`refs/pull/<number>/merge`) — no separate PR-number input needed.
    #[arg(long)]
    #[serde(default)]
    pub tag_pr: bool,
    /// On a `schedule`-triggered build, also tag with the literal tag
    /// `schedule` (a stable, overwritable pointer, like `:latest` —  not a
    /// timestamped/nightly-dated tag).
    #[arg(long)]
    #[serde(default)]
    pub tag_schedule: bool,
}

#[derive(Debug, Clone, clap::Args, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct PublishArgs {
    /// Publish target — only "rust-crate" today (crates.io or another
    /// Cargo registry).
    #[arg(long)]
    #[serde(default)]
    pub target: Option<String>,
    /// Path to the package to publish. Defaults to the current directory.
    #[arg(long)]
    #[serde(default)]
    pub source: Option<String>,
    /// Registry to publish to. Defaults to crates.io.
    #[arg(long)]
    #[serde(default)]
    pub registry: Option<String>,
    /// Build/test/package only — skip the actual publish step. Useful for
    /// verifying a package is publish-ready without a registry token.
    #[arg(long)]
    #[serde(default)]
    pub dry_run: bool,
    /// Suppress dagger's live build progress; only print output once
    /// the pipeline finishes (or on failure). Default is streamed live.
    #[arg(long)]
    #[serde(default)]
    pub silent: bool,
}

#[derive(Debug, Clone, clap::Args, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct ChangelogArgs {
    /// The version this changelog entry is for, e.g. "v1.3.0".
    #[arg(long)]
    pub version: String,
    /// Overrides the auto-resolved previous ref/tag that starts the commit
    /// range. Falls back to the same prefix-aware last-tag resolution
    /// `paws semver` already implements (see --prefix).
    #[arg(long)]
    #[serde(default)]
    pub previous_ref: Option<String>,
    /// Prefix used to filter/resolve the previous tag, e.g. "chart-name-"
    /// — same meaning as `paws semver --prefix`, only used when
    /// --previous-ref is omitted.
    #[arg(long)]
    #[serde(default)]
    pub prefix: Option<String>,
    /// Path to the target CHANGELOG.md, relative to the current directory.
    #[arg(long, default_value = "CHANGELOG.md")]
    #[serde(default = "field_defaults::changelog_path")]
    pub output: String,
    /// Also commit the updated CHANGELOG.md back to --branch via the
    /// GitHub Contents API, with a commit message carrying a `[skip ci]`
    /// loop-avoidance marker. Off by default — without this flag, only the
    /// local file is written (and the rendered entry printed to stdout).
    #[arg(long)]
    #[serde(default)]
    pub commit: bool,
    /// "owner/repo" to operate against. Falls back to $`GITHUB_REPOSITORY`.
    #[arg(long)]
    #[serde(default)]
    pub repository: Option<String>,
    /// Branch to commit to. Only used with --commit.
    #[arg(long, default_value = "main")]
    #[serde(default = "field_defaults::main_branch")]
    pub branch: String,
}

#[derive(Debug, Clone, clap::Args, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct SemverArgs {
    /// Base version to start from instead of the last matching tag.
    #[arg(long)]
    #[serde(default)]
    pub base: Option<String>,
    /// Prefix used to filter/build tag versions, e.g. "chart-name-".
    #[arg(long)]
    #[serde(default)]
    pub prefix: Option<String>,
    /// Explicit increment (major, minor, patch); skips label/branch inference.
    #[arg(long)]
    #[serde(default)]
    pub increment: Option<Increment>,
    /// PR/commit label name that triggers a major bump.
    #[arg(long, default_value = "major")]
    #[serde(default = "field_defaults::major")]
    pub major_label: String,
    /// PR/commit label name that triggers a minor bump.
    #[arg(long, default_value = "minor")]
    #[serde(default = "field_defaults::minor")]
    pub minor_label: String,
    /// PR/commit label name that triggers a patch bump.
    #[arg(long, default_value = "patch")]
    #[serde(default = "field_defaults::patch")]
    pub patch_label: String,
    /// PR/commit labels to check against major/minor/patch-label, comma-separated.
    #[arg(long, value_delimiter = ',')]
    #[serde(default)]
    pub labels: Vec<String>,
    /// Branch name used for fallback inference when no configured label matches.
    #[arg(long, default_value = "main")]
    #[serde(default = "field_defaults::main_branch")]
    pub branch: String,
    /// Whether this is a PR build (produces a `-pr.<sha>` prerelease).
    #[arg(long)]
    #[serde(default)]
    pub pr: bool,
    /// Create and push the computed version as an annotated git tag,
    /// then create a matching GitHub Release with auto-generated notes
    /// (GitHub's git/tags + git/refs + releases APIs — no local git
    /// identity or worktree needed). Replaces a hand-rolled
    /// `git tag`/`git push`/`gh release create` step in the calling
    /// workflow.
    #[arg(long)]
    #[serde(default)]
    pub push: bool,
    /// Tagger identity attributed to the pushed tag.
    #[arg(long, default_value = "paws-bot")]
    #[serde(default = "field_defaults::paws_bot_name")]
    pub tagger_name: String,
    /// Email attributed to the pushed tag, alongside --tagger-name.
    #[arg(long, default_value = "paws-bot@users.noreply.github.com")]
    #[serde(default = "field_defaults::paws_bot_email")]
    pub tagger_email: String,
}

#[derive(Debug, Clone, clap::Args, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct InitArgs {}

#[derive(Debug, Clone, clap::Args, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct AuditArgs {}

#[derive(Debug, Clone, clap::Args, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct CacheArgs {
    /// Print machine-readable JSON instead of the human-readable summary —
    /// `{"backend": "...", "api_version": "...", "base_url": "..."}`
    /// (`api_version`/`base_url` omitted for `dagger-cloud`/`none`).
    #[arg(long)]
    #[serde(default)]
    pub json: bool,
}

#[derive(Debug, Clone, clap::Args, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct DocsArgs {
    /// Comma-delimited publish destination(s): "github-pages",
    /// "cloudflare-pages", "s3". Omitted (the default): builds
    /// target/doc locally only, nothing published — same as before this
    /// flag existed. "cloudflare-pages"/"s3" are recognized but not
    /// implemented yet (see docs/ROADMAP.md); "github-pages" publishes for
    /// real, auto-selecting the Git Trees or Pages-deployment mechanism
    /// from the repository's live Pages configuration.
    #[arg(long, value_delimiter = ',')]
    #[serde(default)]
    pub provider: Vec<String>,
    /// "owner/repo" to publish to. Falls back to $`GITHUB_REPOSITORY`. Only
    /// used when --provider is given.
    #[arg(long)]
    #[serde(default)]
    pub repository: Option<String>,
    /// Branch to publish to (the "github-pages" provider only). Only used
    /// when --provider includes "github-pages".
    #[arg(long, default_value = "main")]
    #[serde(default = "field_defaults::main_branch")]
    pub branch: String,
}

#[derive(Debug, Clone, clap::Args, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct ProvisionArgs {
    /// Comma-separated ecosystems to install, e.g. "rust,node,python,go".
    #[arg(long, value_delimiter = ',')]
    #[serde(default)]
    pub toolchains: Vec<String>,
    /// Print per-ecosystem provisioning start/elapsed timing to stderr.
    #[arg(long)]
    #[serde(default)]
    pub verbose: bool,
}

#[derive(Debug, Clone, clap::Args, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct HelmArgs {
    /// Host path to the source tree to scan for chart(s).
    #[arg(long, default_value = ".")]
    #[serde(default = "field_defaults::dot")]
    pub source: String,
    /// Also `helm package` every chart after linting, exported to `--output`.
    #[arg(long)]
    #[serde(default)]
    pub package: bool,
    /// Host directory packaged `.tgz`s are exported to (only with `--package`).
    #[arg(long, default_value = "tmp")]
    #[serde(default = "field_defaults::tmp")]
    pub output: String,
    /// Publish: a per-chart GitHub Release (tag `<chart>-<version>`,
    /// asset uploaded only if missing) plus a real Helm `index.yaml`
    /// pushed to `--pages-branch`, so `helm repo add` against this
    /// repo's GitHub Pages URL works. Does its own packaging
    /// internally (per-chart, not the flat `--output` directory);
    /// mutually exclusive with `--package`.
    #[arg(long)]
    #[serde(default)]
    pub publish: bool,
    /// "owner/repo" to publish releases/index.yaml to. Falls back to
    /// $`GITHUB_REPOSITORY`. Only used with `--publish`.
    #[arg(long)]
    #[serde(default)]
    pub repository: Option<String>,
    /// Branch `index.yaml` is published to. Only used with `--publish`.
    #[arg(long, default_value = "gh-pages")]
    #[serde(default = "field_defaults::gh_pages")]
    pub pages_branch: String,
    /// Path to `index.yaml` on `--pages-branch`. Only used with `--publish`.
    #[arg(long, default_value = "index.yaml")]
    #[serde(default = "field_defaults::index_yaml")]
    pub index_path: String,
    /// Suppress dagger's live build progress; only print output once
    /// the pipeline finishes (or on failure). Default is streamed live.
    #[arg(long)]
    #[serde(default)]
    pub silent: bool,
}

#[derive(Debug, Clone, clap::Args, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct ReleaseArgs {
    /// Rust target triple to build, e.g. "x86_64-unknown-linux-gnu".
    /// Must be one of `paws_release::known_targets()` — each maps to a
    /// `./builders/<dir>` Dockerfile the build runs through Dagger.
    ///
    /// Optional only so `--list-targets` can be asked without naming one;
    /// a real build still requires it and fails clearly if it is missing.
    #[arg(long)]
    #[serde(default)]
    pub target: Option<String>,
    /// Print every target triple `paws release` knows how to build, one per
    /// line, and exit.
    ///
    /// Exists so the release workflow and `scripts/verify-release.sh` can ask
    /// the binary what the full target set is instead of keeping their own
    /// copies of the list — the same drift that let `paws workflow generate`
    /// fall six toolchains behind `paws ci`.
    #[arg(long)]
    #[serde(default)]
    pub list_targets: bool,
    /// Host path to the source tree to build.
    #[arg(long, default_value = ".")]
    #[serde(default = "field_defaults::dot")]
    pub source: String,
    /// Cargo package(s) to build, comma-separated (one [[bin]] each) —
    /// e.g. "agent,server". Paired 1:1 with --binary-name.
    #[arg(long, default_value = "paws-cli", value_delimiter = ',')]
    #[serde(default = "field_defaults::paws_cli_package")]
    pub package: Vec<String>,
    /// Binary name(s) as declared in each package's [[bin]] section,
    /// comma-separated, paired 1:1 with --package. All built binaries
    /// are packaged into one archive.
    #[arg(long, default_value = "paws", value_delimiter = ',')]
    #[serde(default = "field_defaults::paws_binary_name")]
    pub binary_name: Vec<String>,
    /// Build locally via Dagger's `docker-build` against paws's embedded
    /// generic Rust-Linux builder instead of pulling paws's own prebuilt
    /// builder image. Use this outside paws's own repo (e.g. a target
    /// repo with no `builders/` directory) — only
    /// `paws_release::local_build_targets()` are supported.
    #[arg(long)]
    #[serde(default)]
    pub local_build: bool,
    /// Release tag, e.g. "v0.0.1-prerelease.1". Falls back to $`GITHUB_REF_NAME`.
    #[arg(long)]
    #[serde(default)]
    pub tag: Option<String>,
    /// Mark the GitHub Release as a prerelease.
    #[arg(long)]
    #[serde(default)]
    pub prerelease: bool,
    /// "owner/repo". Falls back to $`GITHUB_REPOSITORY`.
    #[arg(long)]
    #[serde(default)]
    pub repository: Option<String>,
    /// Build and package only; skip the GitHub upload.
    #[arg(long)]
    #[serde(default)]
    pub no_upload: bool,
    /// Skip the post-build smoke test (not recommended — it's what
    /// catches a binary that builds but doesn't actually run).
    #[arg(long)]
    #[serde(default)]
    pub skip_smoke_test: bool,
}
