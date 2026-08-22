use anyhow::Context;
use clap::{Parser, Subcommand};
use paws_audit::{RepositorySignals, select_audit_scanners};
use paws_docker::{
    DockerFactsInput, GithubContext as DockerGithubContext, docker_hub_tags,
    native_publish_pipeline_args, registry_token_env_var, resolve_docker_facts, tags_for_registry,
};
use paws_provision::{Ecosystem, Installer, provision_with_timing, real_installer};
use paws_release::{AssetUploadMode, GitHubReleaseClient, archive_name, package_zip};
use paws_semver::{GitHubGraphQlTagSource, Increment, SemverRequest, compute_new_version};

pub mod action_metadata;
pub mod mcp_setup;

/// Default values for `*Args` fields that carry a clap `default_value`
/// other than the field type's own `Default::default()`. `clap::Args`
/// applies these when a CLI flag is omitted; `serde::Deserialize` has no
/// equivalent unless each field also names one of these via
/// `#[serde(default = "...")]` — otherwise an MCP tool call that omits the
/// field (exactly as a CLI invocation omitting the flag would) fails
/// deserialization with "missing field", even though the CLI treats it as
/// optional. Every field below this fixes was found failing exactly that
/// way in `paws-mcp`'s own tests.
mod field_defaults {
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
}

/// Detects which of the ecosystems `paws-provision` knows about are needed in
/// the current directory, purely from marker files (mirrors `paws-audit`'s
/// signal-based detection, scoped to what `paws-provision` actually supports).
fn detect_needed_ecosystems() -> Vec<Ecosystem> {
    let mut ecosystems = Vec::new();
    if std::path::Path::new("Cargo.toml").exists() {
        ecosystems.push(Ecosystem::Rust);
    }
    if std::path::Path::new("package.json").exists() {
        ecosystems.push(Ecosystem::Node);
    }
    if std::path::Path::new("pyproject.toml").exists() {
        ecosystems.push(Ecosystem::Python);
    }
    if std::path::Path::new("go.mod").exists() {
        ecosystems.push(Ecosystem::Go);
    }
    ecosystems
}

/// Runs a `dagger core <args>` pipeline, streaming its live progress to the
/// terminal by default (`paws_dagger::core_streaming`) — `--silent` falls
/// back to capturing everything and printing it only once the pipeline
/// finishes, for callers that want quiet logs (e.g. a CI system that
/// already buffers/collapses step output itself).
async fn run_dagger_core(args: &[String], silent: bool) -> anyhow::Result<()> {
    if silent {
        let output = paws_dagger::core(args).await?;
        print!("{output}");
    } else {
        paws_dagger::core_streaming(args).await?;
    }
    Ok(())
}

/// A flag value wins over its env-var fallback — mirrors every other
/// flag-or-$ENV resolution in this file (image/version/etc.).
fn resolve_docker_credential(flag: Option<String>, env_var: &str) -> Option<String> {
    flag.or_else(|| std::env::var(env_var).ok())
}

/// Parses `--registry-username`'s `"<registry>=<username>"` entries into a
/// lookup, erroring on anything that isn't a `key=value` pair rather than
/// silently ignoring a typo'd entry.
fn parse_registry_usernames(
    entries: &[String],
) -> anyhow::Result<std::collections::HashMap<String, String>> {
    let mut usernames = std::collections::HashMap::new();
    for entry in entries {
        let (registry, username) = entry.split_once('=').ok_or_else(|| {
            anyhow::anyhow!(
                "--registry-username entries must be \"<registry>=<username>\", got {entry:?}"
            )
        })?;
        usernames.insert(registry.to_string(), username.to_string());
    }
    Ok(usernames)
}

async fn run_provisioning(ecosystems: Vec<Ecosystem>, verbose: bool) -> anyhow::Result<()> {
    if ecosystems.is_empty() {
        return Ok(());
    }
    let tasks: Vec<(Ecosystem, Box<dyn Installer>)> = ecosystems
        .into_iter()
        .map(|e| (e, real_installer(e)))
        .collect();
    let requested: Vec<Ecosystem> = tasks.iter().map(|(e, _)| *e).collect();

    let outcomes = provision_with_timing(tasks).await;

    let mut failures = Vec::new();
    for ecosystem in requested {
        let outcome = &outcomes[&ecosystem];
        if verbose {
            eprintln!(
                "provision: {} started_at={:?} elapsed={:?} ok={}",
                ecosystem.as_str(),
                outcome.started_at,
                outcome.elapsed,
                outcome.result.is_ok()
            );
        }
        if let Err(err) = &outcome.result {
            failures.push(format!("{}: {err}", ecosystem.as_str()));
        }
    }

    if !failures.is_empty() {
        anyhow::bail!("provisioning failed for: {}", failures.join("; "));
    }
    Ok(())
}

/// File-presence signals `paws-audit`'s language detection reads, matching
/// `audit-logic.ts`'s `detectFamily` signal names exactly.
const AUDIT_SIGNAL_FILES: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    "package.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "package-lock.json",
    "pyproject.toml",
    "uv.lock",
    "poetry.lock",
    "requirements.txt",
    "setup.py",
    "go.mod",
    "go.sum",
    "Dockerfile",
    "docker-compose.yml",
    "docker-compose.yaml",
    "compose.yml",
    "compose.yaml",
];

fn collect_repository_signals() -> RepositorySignals {
    AUDIT_SIGNAL_FILES
        .iter()
        .map(|name| (name.to_string(), std::path::Path::new(name).exists()))
        .collect()
}

/// paws: run-anywhere CI/CD pipelines, backed by Dagger.
#[derive(Parser)]
#[command(name = "paws", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Build and test a language target (node, rust, python, tauri, tauri-android, ...).
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
    /// Publish generated docs (e.g. rustdoc) to GitHub Pages.
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
}

#[derive(Subcommand)]
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
    /// to $GH_APP_CLIENT_ID.
    #[arg(long)]
    #[serde(default)]
    pub client_id: Option<String>,
    /// The GitHub App's private key, PEM-encoded, given directly. Falls
    /// back to $GH_APP_PRIVATE_KEY. Mutually exclusive with
    /// --private-key-file in practice — if both are given, the file wins.
    #[arg(long)]
    #[serde(default)]
    pub private_key: Option<String>,
    /// Path to a file containing the GitHub App's private key. Falls back
    /// to $GH_APP_PRIVATE_KEY_FILE.
    #[arg(long)]
    #[serde(default)]
    pub private_key_file: Option<String>,
    /// "owner/repo" the App is installed on. Falls back to
    /// $GITHUB_REPOSITORY.
    #[arg(long)]
    #[serde(default)]
    pub repository: Option<String>,
}

#[derive(Subcommand)]
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

#[derive(Subcommand)]
pub enum LlmsCommand {
    /// Generate `llms.txt` from this CLI's own `--help` metadata.
    Generate(GenerateArgs),
}

#[derive(Subcommand)]
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
    /// "owner/repo" to publish to. Falls back to $GITHUB_REPOSITORY. Only
    /// used with `--publish`.
    #[arg(long)]
    #[serde(default)]
    pub repository: Option<String>,
}

#[derive(Debug, Clone, clap::Args, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct CiArgs {
    /// Which toolchain to build: node, rust, python, go, java, kotlin,
    /// tauri, or tauri-android. For `node`, the package manager
    /// (npm/yarn/pnpm/bun) and framework (Vite, Next.js, or plain) are
    /// auto-detected from lockfiles/package.json — no separate flag needed
    /// — and a Playwright e2e project (`@playwright/test` dependency or a
    /// playwright.config.*) is detected automatically too, running
    /// `npx playwright install --with-deps && npx playwright test`
    /// instead of the plain build+test pipeline.
    #[arg(long)]
    #[serde(default)]
    pub toolchain: Option<String>,
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
}

#[derive(Debug, Clone, clap::Args, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct DockerArgs {
    /// Image name, e.g. "ghcr.io/example/app". Falls back to $GITHUB_REPOSITORY.
    #[arg(long)]
    #[serde(default)]
    pub image: Option<String>,
    /// Version to tag with. Falls back to $GITHUB_SHA (short).
    #[arg(long)]
    #[serde(default)]
    pub version: Option<String>,
    /// Additional registries to mirror tags into, comma-separated.
    #[arg(long, value_delimiter = ',')]
    #[serde(default)]
    pub registries: Vec<String>,
    #[arg(long)]
    #[serde(default)]
    pub dockerfile: Option<String>,
    #[arg(long)]
    #[serde(default)]
    pub context: Option<String>,
    #[arg(long, default_value = "canary")]
    #[serde(default = "field_defaults::canary")]
    pub canary_label: String,
    /// Force push regardless of branch/tag/label gating.
    #[arg(long)]
    #[serde(default)]
    pub push: bool,
    #[arg(long)]
    #[serde(default)]
    pub with_latest: bool,
    #[arg(long)]
    #[serde(default)]
    pub target: Option<String>,
    #[arg(long)]
    #[serde(default)]
    pub prepend_target: bool,
    /// PR labels to check against --canary-label, comma-separated.
    #[arg(long, value_delimiter = ',')]
    #[serde(default)]
    pub labels: Vec<String>,
    #[arg(long, default_value = "main")]
    #[serde(default = "field_defaults::main_branch")]
    pub default_branch: String,
    /// Docker Hub username to authenticate publishing with. Falls back to
    /// $DOCKERHUB_USERNAME. Required (here or via env) to actually push
    /// to docker.io — without it, `dockerRelease` builds but can't
    /// authenticate, so `push=true` still publishes nothing.
    #[arg(long)]
    #[serde(default)]
    pub dockerhub_username: Option<String>,
    /// GHCR username to authenticate publishing with. Falls back to
    /// $GHCR_USERNAME. Required (here or via env) to actually push to
    /// ghcr.io.
    #[arg(long)]
    #[serde(default)]
    pub ghcr_username: Option<String>,
    /// Username(s) for registries in --registries other than docker.io/
    /// ghcr.io (Artifactory, a private registry, etc.), as
    /// "<registry>=<username>" pairs, comma-separated — e.g.
    /// "myco.jfrog.io=deploy-bot". These are built and published
    /// natively through Dagger (`Container.withRegistryAuth`), not via
    /// the docker.io/ghcr.io-only `dockerRelease` call. The matching
    /// token/password is read from an env var derived from the
    /// registry: uppercased, every non-alphanumeric character replaced
    /// with `_`, suffixed `_TOKEN` — e.g. "myco.jfrog.io" reads
    /// $MYCO_JFROG_IO_TOKEN.
    #[arg(long, value_delimiter = ',')]
    #[serde(default)]
    pub registry_username: Vec<String>,
    /// Suppress dagger's live build/publish progress; only print output
    /// once each pipeline finishes (or on failure). Default is streamed
    /// live.
    #[arg(long)]
    #[serde(default)]
    pub silent: bool,
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
    #[arg(long, default_value = "major")]
    #[serde(default = "field_defaults::major")]
    pub major_label: String,
    #[arg(long, default_value = "minor")]
    #[serde(default = "field_defaults::minor")]
    pub minor_label: String,
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
    #[arg(long, default_value = "paws-bot@users.noreply.github.com")]
    #[serde(default = "field_defaults::paws_bot_email")]
    pub tagger_email: String,
}

#[derive(Debug, Clone, clap::Args, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct InitArgs {}

#[derive(Debug, Clone, clap::Args, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct AuditArgs {}

#[derive(Debug, Clone, clap::Args, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct DocsArgs {}

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
    /// $GITHUB_REPOSITORY. Only used with `--publish`.
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
    #[arg(long)]
    pub target: String,
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
    /// Release tag, e.g. "v0.0.1-prerelease.1". Falls back to $GITHUB_REF_NAME.
    #[arg(long)]
    #[serde(default)]
    pub tag: Option<String>,
    /// Mark the GitHub Release as a prerelease.
    #[arg(long)]
    #[serde(default)]
    pub prerelease: bool,
    /// "owner/repo". Falls back to $GITHUB_REPOSITORY.
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

/// Dispatches every subcommand except `mcp serve`, which needs to depend on
/// `paws-mcp` (a crate that itself depends on this crate's lib for its tool
/// handlers) — keeping that edge out of `execute` avoids a build-graph cycle.
/// `paws-cli`'s own `main.rs` handles `mcp serve` directly instead.
pub async fn execute(command: Commands) -> anyhow::Result<()> {
    match command {
        Commands::Ci(args) => run_ci(args).await,
        Commands::Docker(args) => run_docker(args).await,
        Commands::Semver(args) => run_semver(args).await,
        Commands::Init(args) => run_init(args).await,
        Commands::Audit(args) => run_audit(args).await,
        Commands::Docs(args) => run_docs(args).await,
        Commands::Provision(args) => run_provision(args).await,
        Commands::Helm(args) => run_helm(args).await,
        Commands::Release(args) => run_release(args).await,
        Commands::Llms(LlmsCommand::Generate(args)) => run_llms_generate(args).await,
        Commands::Workflow(WorkflowCommand::Generate(args)) => run_workflow_generate(args).await,
        Commands::Auth(AuthCommand::GithubApp(args)) => run_auth_github_app(args).await,
        Commands::Publish(args) => run_publish(args).await,
        Commands::Mcp(McpCommand::Setup(args)) => mcp_setup::run_mcp_setup(args).await,
        Commands::Mcp(McpCommand::Serve(_)) => anyhow::bail!(
            "`paws mcp serve` must be invoked through the `paws` binary directly, not through \
             paws_cli::execute (see main.rs)"
        ),
    }
}

pub async fn run_ci(args: CiArgs) -> anyhow::Result<()> {
    let CiArgs {
        toolchain,
        verbose,
        silent,
        targets,
    } = args;

    if !targets.is_empty() && toolchain.as_deref() != Some("go") {
        anyhow::bail!("--targets is only valid with --toolchain go");
    }

    // FR-015: provisioning must go through the same concurrent path as
    // `paws provision`, never a sequential loop, whenever the target
    // repo needs more than one ecosystem.
    let needed = detect_needed_ecosystems();
    if needed.len() > 1 {
        run_provisioning(needed, verbose).await?;
    }

    paws_dagger::ensure_available().await?;
    match toolchain.as_deref() {
        Some("node") | Some("tauri") => {
            let dir = std::env::current_dir()?;
            let is_tauri = paws_tauri::is_tauri_project(&dir);
            if toolchain.as_deref() == Some("tauri") && !is_tauri {
                anyhow::bail!(
                    "--toolchain tauri given, but no src-tauri/tauri.conf.json found in {}",
                    dir.display()
                );
            }

            let project = paws_node::detect_project(&dir)
                .context("failed to detect a Node project in the current directory")?;
            let missing = project.missing_required_scripts();
            if !is_tauri && !project.has_playwright && !missing.is_empty() {
                anyhow::bail!(
                    "package.json is missing required script(s): {} (found package manager: {}, framework: {})",
                    missing.join(", "),
                    project.package_manager.as_str(),
                    project.framework.as_str()
                );
            }

            if is_tauri {
                println!(
                    "ci: tauri project using {} ({})",
                    project.package_manager.as_str(),
                    dir.display()
                );
                let builder_dir = paws_tauri::write_builder_dockerfile()
                    .context("failed to materialize the tauri-linux builder Dockerfile")?;
                let args = paws_tauri::dagger_pipeline_args(
                    &project,
                    &dir.to_string_lossy(),
                    &builder_dir.to_string_lossy(),
                );
                run_dagger_core(&args, silent).await?;
                println!("ci: tauri build succeeded");
            } else {
                println!(
                    "ci: {} project using {} ({}){}",
                    project.framework.as_str(),
                    project.package_manager.as_str(),
                    dir.display(),
                    if project.has_playwright {
                        " + playwright"
                    } else {
                        ""
                    }
                );
                let args = paws_node::dagger_pipeline_args(&project, &dir.to_string_lossy());
                run_dagger_core(&args, silent).await?;
                println!("ci: node build/test succeeded");
            }
        }
        Some("tauri-android") => {
            let dir = std::env::current_dir()?;
            if !paws_tauri::is_tauri_project(&dir) {
                anyhow::bail!(
                    "--toolchain tauri-android given, but no src-tauri/tauri.conf.json found in {}",
                    dir.display()
                );
            }
            let project = paws_node::detect_project(&dir)
                .context("failed to detect a Node project in the current directory")?;
            println!(
                "ci: tauri android project using {} ({})",
                project.package_manager.as_str(),
                dir.display()
            );
            let builder_dir = paws_tauri::write_android_builder_dockerfile()
                .context("failed to materialize the tauri-android builder Dockerfile")?;
            let args = paws_tauri::android_dagger_pipeline_args(
                &project,
                &dir.to_string_lossy(),
                &builder_dir.to_string_lossy(),
            );
            run_dagger_core(&args, silent).await?;
            println!("ci: tauri android build succeeded");
        }
        Some("python") => {
            let dir = std::env::current_dir()?;
            let project = paws_python::detect_project(&dir)
                .context("failed to detect a Python project in the current directory")?;
            println!(
                "ci: python project ({}) ({})",
                if project.has_lockfile {
                    "uv.lock present"
                } else {
                    "no uv.lock"
                },
                dir.display()
            );
            let args = paws_python::dagger_pipeline_args(&project, &dir.to_string_lossy());
            run_dagger_core(&args, silent).await?;
            println!("ci: python build/test succeeded");
        }
        Some("rust") => {
            let dir = std::env::current_dir()?;
            if !paws_rust::is_rust_project(&dir) {
                anyhow::bail!(
                    "--toolchain rust given, but no Cargo.toml found in {}",
                    dir.display()
                );
            }
            let is_wasm = paws_rust::is_wasm_project(&dir);
            println!(
                "ci: rust project{} ({})",
                if is_wasm {
                    " (wasm32-unknown-unknown)"
                } else {
                    ""
                },
                dir.display()
            );
            let args = paws_rust::dagger_pipeline_args(&dir.to_string_lossy(), is_wasm);
            run_dagger_core(&args, silent).await?;
            println!("ci: rust build/test succeeded");
        }
        Some("go") => {
            let dir = std::env::current_dir()?;
            if !paws_go::is_go_project(&dir) {
                anyhow::bail!(
                    "--toolchain go given, but no go.mod found in {}",
                    dir.display()
                );
            }
            if targets.is_empty() {
                let is_wasm = paws_go::is_wasm_project(&dir);
                println!(
                    "ci: go project{} ({})",
                    if is_wasm { " (js/wasm)" } else { "" },
                    dir.display()
                );
                let args = paws_go::dagger_pipeline_args(&dir.to_string_lossy(), is_wasm);
                run_dagger_core(&args, silent).await?;
                println!("ci: go build/test succeeded");
            } else {
                let parsed_targets = targets
                    .iter()
                    .map(|t| paws_go::Target::parse(t))
                    .collect::<anyhow::Result<Vec<_>>>()?;
                let module = paws_go::module_name(&dir)?;
                let dist_dir = dir.join("dist");
                println!(
                    "ci: go project ({}) cross-compiling to {}",
                    dir.display(),
                    targets.join(", ")
                );
                let args = paws_go::cross_dagger_pipeline_args(
                    &dir.to_string_lossy(),
                    &module,
                    &parsed_targets,
                    &dist_dir.to_string_lossy(),
                );
                run_dagger_core(&args, silent).await?;
                println!(
                    "ci: go cross-compile succeeded — binaries in {}",
                    dist_dir.display()
                );
            }
        }
        Some("java") => {
            let dir = std::env::current_dir()?;
            let build_system = paws_java::detect_project(&dir)
                .context("failed to detect a Java project in the current directory")?;
            println!(
                "ci: java project using {} ({})",
                build_system.as_str(),
                dir.display()
            );
            let builder_dir = paws_java::write_builder_dockerfile()
                .context("failed to materialize the java builder Dockerfile")?;
            let args = paws_java::dagger_pipeline_args(
                build_system,
                &dir.to_string_lossy(),
                &builder_dir.to_string_lossy(),
            );
            run_dagger_core(&args, silent).await?;
            println!("ci: java build/test succeeded");
        }
        Some("kotlin") => {
            let dir = std::env::current_dir()?;
            paws_kotlin::detect_project(&dir)
                .context("failed to detect a Kotlin project in the current directory")?;
            println!("ci: kotlin project ({})", dir.display());
            let builder_dir = paws_kotlin::write_builder_dockerfile()
                .context("failed to materialize the java builder Dockerfile")?;
            let args = paws_kotlin::dagger_pipeline_args(
                &dir.to_string_lossy(),
                &builder_dir.to_string_lossy(),
            );
            run_dagger_core(&args, silent).await?;
            println!("ci: kotlin build/test succeeded");
        }
        Some("flatpak") => {
            let dir = std::env::current_dir()?;
            let project = paws_flatpak::detect_project(&dir)
                .context("failed to detect a Flatpak manifest in the current directory")?;
            println!(
                "ci: flatpak project {} ({})",
                project.app_id,
                project.manifest_path.display()
            );
            let builder_dir = paws_flatpak::write_builder_dockerfile()
                .context("failed to materialize the flatpak builder Dockerfile")?;
            let args = paws_flatpak::dagger_pipeline_args(
                &project,
                &dir.to_string_lossy(),
                &builder_dir.to_string_lossy(),
            );
            run_dagger_core(&args, silent).await?;
            println!("ci: flatpak build succeeded");
        }
        Some(other) => anyhow::bail!(
            "unsupported --toolchain '{other}'; expected 'node', 'rust', 'python', 'go', 'java', 'kotlin', 'tauri', 'tauri-android', or 'flatpak'"
        ),
        None => anyhow::bail!("--toolchain is required (e.g. --toolchain node)"),
    }
    Ok(())
}

pub async fn run_docker(args: DockerArgs) -> anyhow::Result<()> {
    let DockerArgs {
        image,
        version,
        registries,
        dockerfile,
        context,
        canary_label,
        push,
        with_latest,
        target,
        prepend_target,
        labels,
        default_branch,
        dockerhub_username,
        ghcr_username,
        registry_username,
        silent,
    } = args;

    let image = image
        .or_else(|| std::env::var("GITHUB_REPOSITORY").ok())
        .ok_or_else(|| anyhow::anyhow!("--image is required (or set $GITHUB_REPOSITORY)"))?;
    let version = version.unwrap_or_else(|| {
        std::env::var("GITHUB_SHA")
            .map(|sha| sha.chars().take(7).collect())
            .unwrap_or_default()
    });
    let git_ref = std::env::var("GITHUB_REF").unwrap_or_default();
    let event_name = std::env::var("GITHUB_EVENT_NAME").unwrap_or_default();
    let workspace = std::env::current_dir()?;

    let facts = resolve_docker_facts(
        &DockerFactsInput {
            image: image.clone(),
            version,
            registries: registries.clone(),
            dockerfile: dockerfile.clone(),
            context: context.clone(),
            canary_label: Some(canary_label.clone()),
            force_push: push,
            with_latest,
            target: target.clone(),
            prepend_target,
        },
        &DockerGithubContext {
            workspace: workspace.clone(),
            event_name,
            git_ref,
            default_branch: default_branch.clone(),
            pr_labels: labels.clone(),
        },
    );

    println!(
        "docker: resolved -> context={} dockerfile={} target={} push={}",
        facts.context, facts.dockerfile, facts.target, facts.push
    );
    paws_dagger::ensure_available().await?;

    if facts.tags.is_empty() {
        println!("docker: no tags resolved, nothing to build/publish");
        return Ok(());
    }

    // Every registry publishes natively through Dagger now —
    // docker.io/ghcr.io included, not just the ones beyond them.
    // `paws` used to delegate docker.io/ghcr.io to `gh-reusable`'s
    // `dockerRelease` (a Dagger Function in a different repo); this
    // routes them through the exact same `Container.withRegistryAuth`
    // + `Container.publish` primitives already verified for real
    // this session for arbitrary registries — no reason for the two
    // known registries to go through a separate code path.
    let dockerhub_username = resolve_docker_credential(dockerhub_username, "DOCKERHUB_USERNAME");
    let ghcr_username = resolve_docker_credential(ghcr_username, "GHCR_USERNAME");
    let extra_usernames = parse_registry_usernames(&registry_username)?;

    // docker.io/ghcr.io mirror `dockerRelease`'s own graceful
    // degrade — missing credentials there just skips that
    // registry's publish (preserves existing behavior for repos
    // that only ever configured one of the two). A registry
    // reached via --registries (ghcr.io or a custom one) is an
    // explicit ask, so missing credentials for it fails loudly
    // instead — the whole reason `--registries` silently dropping
    // docker.io got caught earlier this session was a *silent*
    // under-publish; an explicit registry with no way to
    // authenticate deserves the same loud treatment, not a repeat.
    struct DockerPublishTarget<'a> {
        registry: String,
        tags: Vec<&'a str>,
        username: Option<&'a String>,
        token_env_var: String,
        credentials_required: bool,
    }

    let mut targets = vec![DockerPublishTarget {
        registry: "docker.io".to_string(),
        tags: docker_hub_tags(&facts.tags, &registries),
        username: dockerhub_username.as_ref(),
        token_env_var: "DOCKER_TOKEN".to_string(),
        credentials_required: false,
    }];
    for registry in &registries {
        let username = if registry == "ghcr.io" {
            ghcr_username.as_ref()
        } else {
            extra_usernames.get(registry)
        };
        let token_env_var = if registry == "ghcr.io" {
            "GHCR_TOKEN".to_string()
        } else {
            registry_token_env_var(registry)
        };
        targets.push(DockerPublishTarget {
            registry: registry.clone(),
            tags: tags_for_registry(&facts.tags, registry),
            username,
            token_env_var,
            credentials_required: registry != "ghcr.io",
        });
    }

    if facts.push {
        for target in &targets {
            let DockerPublishTarget {
                registry,
                tags,
                username,
                token_env_var,
                credentials_required,
            } = target;
            if tags.is_empty() {
                continue;
            }
            let username = match username {
                Some(u) => u,
                None if *credentials_required => {
                    anyhow::bail!(
                        "--registry-username is required for {registry} (got \
                         --registries including it, but no matching \
                         --registry-username entry) to actually publish"
                    );
                }
                None => {
                    println!(
                        "docker: no username configured for {registry}, skipping publish \
                         ({} tag(s))",
                        tags.len()
                    );
                    continue;
                }
            };
            let has_token = std::env::var(token_env_var).is_ok();
            if !has_token {
                if *credentials_required {
                    anyhow::bail!("${token_env_var} must be set to publish to {registry}");
                }
                println!(
                    "docker: ${token_env_var} not set, skipping publish to {registry} \
                     ({} tag(s))",
                    tags.len()
                );
                continue;
            }
            for tag in tags {
                println!("docker: publishing {tag} to {registry}...");
                let publish_args = native_publish_pipeline_args(
                    &paws_docker::BuildSpec {
                        context: &facts.context,
                        dockerfile: &facts.dockerfile,
                        target: &facts.target,
                        build_args: &facts.build_args,
                    },
                    &paws_docker::NativeRegistryPublish {
                        registry,
                        username,
                        token_env_var,
                        tag_address: tag,
                    },
                );
                run_dagger_core(&publish_args, silent)
                    .await
                    .with_context(|| format!("failed to publish {tag} to {registry}"))?;
                println!("docker: published {tag}");
            }
        }
    } else {
        let total_tags: usize = targets.iter().map(|t| t.tags.len()).sum();
        println!(
            "docker: build-only (push not resolved for this run) — validating the \
             Dockerfile still builds; {total_tags} tag(s) across {} registr{} would \
             have been published on a real push",
            targets.iter().filter(|t| !t.tags.is_empty()).count(),
            if targets.len() == 1 { "y" } else { "ies" }
        );
        let build_only_args = paws_docker::build_only_pipeline_args(&paws_docker::BuildSpec {
            context: &facts.context,
            dockerfile: &facts.dockerfile,
            target: &facts.target,
            build_args: &facts.build_args,
        });
        run_dagger_core(&build_only_args, silent).await?;
        println!("docker: build succeeded");
    }

    Ok(())
}

pub async fn run_publish(args: PublishArgs) -> anyhow::Result<()> {
    let PublishArgs {
        target,
        source,
        registry,
        dry_run,
        silent,
    } = args;

    match target.as_deref() {
        Some("rust-crate") => {
            let dir = match source {
                Some(s) => std::path::PathBuf::from(s),
                None => std::env::current_dir()?,
            };
            let dir = dir
                .canonicalize()
                .with_context(|| format!("failed to resolve {}", dir.display()))?;
            if !paws_publish::is_rust_crate(&dir) {
                anyhow::bail!(
                    "--target rust-crate given, but no Cargo.toml found in {}",
                    dir.display()
                );
            }
            let name = paws_publish::read_crate_name(&dir)?;
            let registry = registry.unwrap_or_else(|| paws_publish::DEFAULT_REGISTRY.to_string());
            let token_env_var = paws_publish::token_env_var(&registry);
            if !dry_run && std::env::var(&token_env_var).is_err() {
                anyhow::bail!(
                    "--target rust-crate needs ${token_env_var} set (registry: {registry}) — pass --dry-run to verify the package without publishing"
                );
            }
            // A workspace member (e.g. one crate among several in a real
            // repo, like mbround18/game-server-management's libs/*) needs
            // its real workspace root mounted, not just its own
            // subdirectory — see paws_publish's module doc for the real
            // bug (confirmed against that repo's own actual CI failures)
            // this routes around.
            let (mount_dir, workdir) = match paws_publish::find_workspace_root(&dir) {
                Some(root) => {
                    let relative = dir.strip_prefix(&root).unwrap_or(&dir);
                    let workdir = std::path::Path::new("/src").join(relative);
                    (root, workdir)
                }
                None => (dir.clone(), std::path::PathBuf::from("/src")),
            };
            println!(
                "publish: {name} -> {registry}{}",
                if dry_run { " (dry run)" } else { "" }
            );
            let args = paws_publish::dagger_pipeline_args(
                &mount_dir.to_string_lossy(),
                &workdir.to_string_lossy(),
                &registry,
                &token_env_var,
                dry_run,
            );
            run_dagger_core(&args, silent).await?;
            println!(
                "publish: {name} {}",
                if dry_run {
                    "packaged successfully (dry run, not published)"
                } else {
                    "published successfully"
                }
            );
        }
        Some(other) => anyhow::bail!("unsupported --target '{other}'; expected 'rust-crate'"),
        None => anyhow::bail!("--target is required (e.g. --target rust-crate)"),
    }

    Ok(())
}

pub async fn run_semver(args: SemverArgs) -> anyhow::Result<()> {
    let SemverArgs {
        base,
        prefix,
        increment,
        major_label,
        minor_label,
        patch_label,
        labels,
        branch,
        pr,
        push,
        tagger_name,
        tagger_email,
    } = args;

    let ctx = paws_environment::CiContext::detect()
        .await
        .context("paws semver needs a supported CI provider's env vars")?;
    let labels = if labels.is_empty() {
        match paws_semver::fetch_pr_labels_for_commit(&ctx.owner, &ctx.repo, &ctx.sha, &ctx.token)
            .await
        {
            Ok(found) => {
                if !found.is_empty() {
                    eprintln!("semver: auto-detected PR labels: {}", found.join(", "));
                }
                found
            }
            Err(err) => {
                eprintln!(
                    "semver: couldn't auto-detect PR labels for {}, falling back to branch/patch inference: {err:#}",
                    ctx.sha
                );
                Vec::new()
            }
        }
    } else {
        labels
    };
    let request = SemverRequest {
        base,
        prefix,
        explicit_increment: increment,
        major_label,
        minor_label,
        patch_label,
        labels,
        branch_name: branch,
        sha: ctx.sha.clone(),
        is_pr: pr,
        github_ref: ctx.git_ref.clone(),
    };
    let tag_source = GitHubGraphQlTagSource {
        owner: ctx.owner.clone(),
        repo: ctx.repo.clone(),
        token: ctx.token.clone(),
    };

    let version = compute_new_version(&tag_source, &request).await?;
    println!("{version}");

    if push {
        anyhow::ensure!(
            !ctx.sha.is_empty(),
            "paws semver --push needs a commit sha (GITHUB_SHA was empty)"
        );
        let author = paws_environment::TagAuthor {
            name: &tagger_name,
            email: &tagger_email,
        };
        paws_environment::push_tag(&ctx, &version, &author)
            .await
            .with_context(|| format!("failed to push tag/release {version}"))?;
        eprintln!("pushed tag {version} and created its release");
    }

    Ok(())
}

pub async fn run_init(_args: InitArgs) -> anyhow::Result<()> {
    let install_dir = paws_dagger::install_cli()
        .await
        .context("failed to install the dagger CLI")?;
    println!("dagger CLI installed to {}", install_dir.display());

    // Prepend to this process's own PATH so the sanity check below
    // (and any subcommand run later in the same shell invocation)
    // can find it immediately, without waiting on a shell restart —
    // this only affects this process and its children, so users
    // still need `$HOME/.local/bin` on PATH for future shells (the
    // `$GITHUB_PATH` append inside `install_cli` covers CI for free).
    if let Some(existing) = std::env::var_os("PATH") {
        let mut paths = vec![install_dir.clone()];
        paths.extend(std::env::split_paths(&existing));
        if let Ok(joined) = std::env::join_paths(paths) {
            unsafe { std::env::set_var("PATH", joined) };
        }
    }

    paws_dagger::ensure_available()
        .await
        .context("dagger was installed but isn't runnable")?;
    println!(
        "init: dagger is ready (add {} to PATH for future shells)",
        install_dir.display()
    );
    Ok(())
}

pub async fn run_audit(_args: AuditArgs) -> anyhow::Result<()> {
    // `paws-audit`'s detection logic decides whether it's worth
    // spinning up `dagger` at all (spec.md's "outside a Cargo/Node/
    // Docker project entirely" edge case).
    let signals = collect_repository_signals();
    let detection = paws_audit::detect_language_families(&signals);
    let scanners = select_audit_scanners(&detection, true);
    if !scanners.iter().any(|s| s.should_run) {
        println!("audit: no recognizable project markers found here; nothing to scan.");
        return Ok(());
    }

    paws_dagger::ensure_available().await?;
    let source = std::env::current_dir()?.to_string_lossy().to_string();

    // Each scanner runs natively through Dagger now (no `gh-reusable`
    // Dagger Function call) — one invocation reads the scanner's own
    // JSON report, a second (sharing the same build/exec, so Dagger's
    // own cache makes it fast) reads the exit code;
    // `normalize_scanner_status` needs both to tell "clean pass" from
    // "the scanner itself errored" apart.
    let mut scanner_results = Vec::with_capacity(scanners.len());
    for scanner in &scanners {
        if !scanner.should_run {
            scanner_results.push(paws_audit::create_skipped_scanner_result(scanner));
            continue;
        }
        println!("audit: running {}...", scanner.step_name);
        let started = std::time::Instant::now();
        let raw_json =
            paws_dagger::core(&paws_audit::scanner_json_pipeline_args(&source, scanner)).await;
        let exit_code_output = paws_dagger::core(&paws_audit::scanner_exit_code_pipeline_args(
            &source, scanner,
        ))
        .await;
        let duration_ms = started.elapsed().as_millis() as u64;

        let result = match (raw_json, exit_code_output) {
            (Ok(raw_json), Ok(exit_code_raw)) => {
                let exit_code = exit_code_raw.trim().parse::<i32>().ok();
                let (findings_count, top_findings) =
                    paws_audit::parse_scanner_findings(scanner.name, &raw_json);
                let status = paws_audit::normalize_scanner_status(exit_code, findings_count);
                paws_audit::AuditScannerResult {
                    name: scanner.name.as_str().to_string(),
                    family: scanner.family,
                    status,
                    findings_count,
                    duration_ms,
                    failure_reason: (status == paws_audit::ScannerStatus::Failed)
                        .then(|| format!("{} exited {:?}", scanner.name.as_str(), exit_code)),
                    top_findings,
                }
            }
            (Err(err), _) | (_, Err(err)) => {
                paws_audit::create_failed_scanner_result(scanner, duration_ms, err.to_string())
            }
        };
        scanner_results.push(result);
    }

    let summary = paws_audit::aggregate_audit_results(&scanner_results, &detection);
    println!(
        "{}",
        paws_audit::render_audit_intelligence_section(&summary)
    );

    if summary.overall_status == paws_audit::AuditOverallStatus::Failed {
        anyhow::bail!("audit failed: see scanner findings above");
    }
    Ok(())
}

pub async fn run_docs(_args: DocsArgs) -> anyhow::Result<()> {
    let workspace = std::env::current_dir()?;
    let docs_dir = paws_docs::build_docs(&workspace).await?;
    println!("docs: built at {}", docs_dir.display());
    Ok(())
}

pub async fn run_provision(args: ProvisionArgs) -> anyhow::Result<()> {
    let ProvisionArgs {
        toolchains,
        verbose,
    } = args;
    if toolchains.is_empty() {
        anyhow::bail!("--toolchains is required (e.g. --toolchains rust,node,python,go)");
    }
    let ecosystems = toolchains
        .iter()
        .map(|t| t.parse::<Ecosystem>())
        .collect::<anyhow::Result<Vec<_>>>()?;
    run_provisioning(ecosystems, verbose).await?;
    println!("provision: all requested toolchains provisioned successfully");
    Ok(())
}

pub async fn run_helm(args: HelmArgs) -> anyhow::Result<()> {
    let HelmArgs {
        source,
        package,
        output,
        publish,
        repository,
        pages_branch,
        index_path,
        silent,
    } = args;

    anyhow::ensure!(
        !(package && publish),
        "--package and --publish are mutually exclusive - --publish already packages \
         each chart internally"
    );

    let dir = std::path::Path::new(&source)
        .canonicalize()
        .unwrap_or_else(|_| source.clone().into());
    let project = paws_helm::detect_project(&dir)
        .context("failed to detect a Helm chart project in the given source directory")?;
    println!(
        "helm: found {} chart(s) in {}",
        project.charts.len(),
        dir.display()
    );

    paws_dagger::ensure_available().await?;
    let builder_dir = paws_helm::write_builder_dockerfile()
        .context("failed to materialize the helm builder Dockerfile")?;

    if publish {
        let repository = repository
            .or_else(|| std::env::var("GITHUB_REPOSITORY").ok())
            .ok_or_else(|| {
                anyhow::anyhow!("--repository is required (or set $GITHUB_REPOSITORY)")
            })?;
        let (owner, repo) = repository.split_once('/').ok_or_else(|| {
            anyhow::anyhow!("--repository must be \"owner/repo\", got {repository}")
        })?;
        let token = paws_environment::resolve_github_token(owner, repo).await?;
        let client = GitHubReleaseClient::new(owner.to_string(), repo.to_string(), token);

        let existing = client.get_content(&index_path, &pages_branch).await?;
        let existing_index_file = if let Some(existing) = &existing {
            let path = std::env::temp_dir().join("paws-helm-existing-index.yaml");
            tokio::fs::write(&path, &existing.content)
                .await
                .context("failed to stage the existing index.yaml for the publish pipeline")?;
            println!("helm: seeding from the existing {index_path}@{pages_branch}");
            Some(path)
        } else {
            println!("helm: no existing {index_path}@{pages_branch} found, publishing fresh");
            None
        };

        let publish_target = paws_helm::PublishTarget {
            owner,
            repo,
            existing_index_path: existing_index_file.as_deref(),
            container_packages_dir: "/out",
            container_index_path: "/idx/index.yaml",
        };

        let packages_dir = std::env::temp_dir().join("paws-helm-publish-packages");
        let index_out_dir = std::env::temp_dir().join("paws-helm-publish-index");
        tokio::fs::create_dir_all(&packages_dir).await?;
        tokio::fs::create_dir_all(&index_out_dir).await?;

        let packages_args = paws_helm::publish_packages_pipeline_args(
            &project,
            &dir.to_string_lossy(),
            &builder_dir.to_string_lossy(),
            &publish_target,
            &packages_dir.to_string_lossy(),
        );
        run_dagger_core(&packages_args, silent).await?;

        let index_args = paws_helm::publish_index_pipeline_args(
            &project,
            &dir.to_string_lossy(),
            &builder_dir.to_string_lossy(),
            &publish_target,
            &index_out_dir.join("index.yaml").to_string_lossy(),
        );
        run_dagger_core(&index_args, silent).await?;

        for chart in &project.charts {
            let tag = chart.tag();
            let archive_path = packages_dir
                .join(&chart.name)
                .join(chart.archive_file_name());
            let release_id = client.get_or_create_release(&tag, false).await?;
            let uploaded = client
                .upload_asset_with(
                    release_id,
                    &archive_path,
                    "application/gzip",
                    AssetUploadMode::SkipIfExisting,
                )
                .await?;
            println!(
                "helm: {} {} ({tag})",
                if uploaded {
                    "published"
                } else {
                    "already published, skipped"
                },
                chart.archive_file_name()
            );
        }

        let new_index = tokio::fs::read(index_out_dir.join("index.yaml"))
            .await
            .context("failed to read the generated index.yaml")?;
        client
            .put_content(
                &index_path,
                &pages_branch,
                &new_index,
                "Update index.yaml",
                existing.as_ref().map(|e| e.sha.as_str()),
            )
            .await?;
        println!("helm: published {index_path}@{pages_branch}");
    } else if package {
        let output_dir = std::path::Path::new(&output);
        std::fs::create_dir_all(output_dir)
            .context("failed to create the Helm package output directory")?;
        let host_output = output_dir
            .canonicalize()
            .context("failed to resolve the Helm package output directory")?;
        let args = paws_helm::package_pipeline_args(
            &project,
            &dir.to_string_lossy(),
            &builder_dir.to_string_lossy(),
            "/out",
            &host_output.to_string_lossy(),
        );
        run_dagger_core(&args, silent).await?;
        println!(
            "helm: lint + package succeeded, packages in {}",
            host_output.display()
        );
    } else {
        let args = paws_helm::lint_pipeline_args(
            &project,
            &dir.to_string_lossy(),
            &builder_dir.to_string_lossy(),
        );
        run_dagger_core(&args, silent).await?;
        println!("helm: lint succeeded");
    }

    Ok(())
}

pub async fn run_release(args: ReleaseArgs) -> anyhow::Result<()> {
    let ReleaseArgs {
        target,
        source,
        package,
        binary_name,
        local_build,
        tag,
        prerelease,
        repository,
        no_upload,
        skip_smoke_test,
    } = args;

    anyhow::ensure!(
        package.len() == binary_name.len(),
        "--package and --binary-name must list the same number of entries \
         (got {} package(s), {} binary-name(s))",
        package.len(),
        binary_name.len()
    );

    let tag = tag.or_else(|| std::env::var("GITHUB_REF_NAME").ok());
    let raw_tag = tag
        .clone()
        .ok_or_else(|| anyhow::anyhow!("--tag is required (or set $GITHUB_REF_NAME)"))?;
    // Archive names drop the "v" prefix (established convention, matches
    // prereleases already published); the prebuilt builder image tag does
    // not — `release.yaml`'s build-builders job tags it from the raw
    // ref/tag name (`v0.0.1-prerelease.2`), so `builder_version` below
    // must match that exactly, not the stripped archive-naming version.
    let version = raw_tag.trim_start_matches('v').to_string();

    let target_config = paws_release::target_config(&target).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown --target '{target}'; known targets: {}",
            paws_release::known_targets()
                .iter()
                .map(|t| t.triple)
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;

    paws_dagger::ensure_available().await?;

    let local_builder_dir = if local_build {
        Some(paws_release::write_generic_builder_dockerfile()?)
    } else {
        None
    };

    let mut binary_paths = Vec::with_capacity(package.len());
    for (pkg, bin_name) in package.iter().zip(binary_name.iter()) {
        let request = paws_release::BuildRequest {
            builder_dir: target_config.builder_dir,
            source_dir: &source,
            triple: &target,
            package: pkg,
            binary_name: bin_name,
            builder_version: &raw_tag,
        };

        let binary_path = if let Some(local_builder_dir) = &local_builder_dir {
            println!("release: building {bin_name} for {target} via local docker-build...");
            paws_release::build_binary_local(&request, local_builder_dir).await?
        } else {
            println!(
                "release: building {bin_name} for {target} via {}...",
                target_config.builder_dir
            );
            paws_release::build_binary(&request).await?
        };
        println!("release: built {}", binary_path.display());

        match (&target_config.smoke, skip_smoke_test) {
            (_, true) => println!("release: --skip-smoke-test set, skipping"),
            (None, false) => {
                println!(
                    "release: no execution environment available for {target}, skipping smoke test (build/link success only)"
                );
            }
            (Some(spec), false) => {
                println!("release: smoke testing {bin_name}...");
                let smoke_output = paws_release::smoke_test(&binary_path, spec).await?;
                println!("release: smoke test output: {}", smoke_output.trim());
            }
        }

        binary_paths.push(binary_path);
    }

    let archive_label = binary_name.join("+");
    let archive = archive_name(&archive_label, &version, &target);
    let archive_path = std::path::Path::new("target")
        .join("release-archives")
        .join(&archive);
    let relative_binaries: Vec<String> = binary_paths
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    package_zip(&std::env::current_dir()?, &archive_path, &relative_binaries).await?;
    println!("release: packaged {}", archive_path.display());

    if no_upload {
        println!("release: --no-upload set, skipping GitHub upload");
        return Ok(());
    }

    let tag = tag
        .ok_or_else(|| anyhow::anyhow!("--tag is required to upload (or set $GITHUB_REF_NAME)"))?;
    let repository = repository
        .or_else(|| std::env::var("GITHUB_REPOSITORY").ok())
        .ok_or_else(|| anyhow::anyhow!("--repository is required (or set $GITHUB_REPOSITORY)"))?;
    let (owner, repo) = repository
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("--repository must be \"owner/repo\", got {repository}"))?;
    let token = paws_environment::resolve_github_token(owner, repo).await?;

    let client = GitHubReleaseClient::new(owner.to_string(), repo.to_string(), token);
    let release_id = client.get_or_create_release(&tag, prerelease).await?;
    client.upload_asset(release_id, &archive_path).await?;
    println!("release: uploaded {archive} to {repository}@{tag}");

    Ok(())
}

/// Ecosystem/tooling signals `render_github_workflow` renders steps for —
/// kept separate from `RepositorySignals`'s raw filename map so the
/// rendering logic is independent of `paws-audit`'s specific signal-file
/// list and can be unit tested without touching the filesystem at all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct DetectedWorkflowInputs {
    rust: bool,
    node: bool,
    python: bool,
    docker: bool,
    helm: bool,
}

impl DetectedWorkflowInputs {
    fn any(&self) -> bool {
        self.rust || self.node || self.python || self.docker || self.helm
    }
}

/// Renders a starter GitHub Actions workflow wiring `paws-up` plus one
/// `paws ci --toolchain <x>`/`paws docker`/`paws helm` step per detected
/// signal — `None` when nothing was detected, so the caller can skip
/// writing a file entirely rather than emitting an empty/useless workflow.
fn render_github_workflow(detected: &DetectedWorkflowInputs) -> Option<String> {
    if !detected.any() {
        return None;
    }

    let mut out = String::new();
    out.push_str("# Generated by `paws workflow generate` — https://github.com/mbround18/paws\n");
    out.push_str("name: paws\n\n");
    out.push_str("on:\n  push:\n    branches: [main]\n  pull_request:\n\n");
    out.push_str("jobs:\n  paws:\n    runs-on: ubuntu-latest\n    steps:\n");
    out.push_str("      - uses: actions/checkout@v7\n\n");
    out.push_str("      - uses: mbround18/paws/actions/paws-up@main\n\n");

    if detected.rust {
        out.push_str("      - run: paws ci --toolchain rust\n");
    }
    if detected.node {
        out.push_str("      - run: paws ci --toolchain node\n");
    }
    if detected.python {
        out.push_str("      - run: paws ci --toolchain python\n");
    }
    if detected.docker {
        out.push_str(
            "      # Build-only by default — add --push plus registry credentials \
             (see `paws docker --help`) once you've set up registry secrets.\n",
        );
        out.push_str("      - run: paws docker\n");
    }
    if detected.helm {
        out.push_str("      - run: paws helm\n");
    }

    Some(out)
}

pub async fn run_workflow_generate(args: WorkflowGenerateArgs) -> anyhow::Result<()> {
    let WorkflowGenerateArgs { provider, output } = args;
    if provider != "github" {
        anyhow::bail!(
            "unsupported --provider '{provider}'; only 'github' is implemented today — more \
             origins (e.g. 'gitlab') are planned, see paws_environment::Provider"
        );
    }

    let signals = collect_repository_signals();
    let dir = std::env::current_dir()?;
    let detected = DetectedWorkflowInputs {
        rust: signals.get("Cargo.toml").copied().unwrap_or(false),
        node: signals.get("package.json").copied().unwrap_or(false),
        python: signals.get("pyproject.toml").copied().unwrap_or(false),
        docker: [
            "Dockerfile",
            "docker-compose.yml",
            "docker-compose.yaml",
            "compose.yml",
            "compose.yaml",
        ]
        .iter()
        .any(|f| signals.get(*f).copied().unwrap_or(false)),
        helm: paws_helm::detect_project(&dir).is_ok(),
    };

    let Some(rendered) = render_github_workflow(&detected) else {
        println!(
            "workflow: no recognizable project markers found here (checked Rust/Node/Python/\
             Docker/Helm); nothing to generate."
        );
        return Ok(());
    };

    if let Some(parent) = std::path::Path::new(&output).parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    tokio::fs::write(&output, &rendered)
        .await
        .with_context(|| format!("failed to write {output}"))?;

    let mut kinds = Vec::new();
    if detected.rust {
        kinds.push("rust");
    }
    if detected.node {
        kinds.push("node");
    }
    if detected.python {
        kinds.push("python");
    }
    if detected.docker {
        kinds.push("docker");
    }
    if detected.helm {
        kinds.push("helm");
    }
    println!("workflow: generated {output} ({})", kinds.join(", "));

    Ok(())
}

/// Renders `llms.txt` (the <https://llmstxt.org> convention) purely from
/// this CLI's own `clap::Command` metadata — the exact same source that
/// drives `--help`, so it can never drift from real CLI behavior.
pub fn render_llms_txt() -> String {
    use clap::CommandFactory;

    let root = Cli::command();
    let mut out = String::new();

    out.push_str("# paws\n\n");
    let about = root.get_about().map(|s| s.to_string()).unwrap_or_default();
    if !about.is_empty() {
        out.push_str(&format!("> {about}\n\n"));
    }
    out.push_str(
        "paws is a run-anywhere CI/CD CLI backed by Dagger. Every subcommand below is also \
         exposed as an MCP tool via `paws mcp serve` (see `paws mcp setup` to wire up an MCP \
         client), calling this same code directly.\n\n",
    );

    // A copy-pasteable bootstrap block, deliberately placed before the
    // per-command reference below — this is the part meant to be handed
    // directly to an AI coding assistant (Claude Code, Copilot, Cursor,
    // ...) that was pointed at this file (or its raw URL), so it can wire
    // `paws` into a repo without a human walking it through each step by
    // hand. Install commands mirror README.md's "Installation" section
    // verbatim — keep the two in sync if either changes.
    out.push_str(
        "## Quickstart for an AI agent\n\n\
         If you're an AI coding assistant reading this file (pasted in directly, or fetched from \
         <https://raw.githubusercontent.com/mbround18/paws/main/llms.txt>) and asked to wire \
         `paws` into the current repo, run these in order:\n\n\
         ```sh\n\
         # 1. Install the paws binary (detects OS/arch, puts it on PATH)\n\
         curl -fsSL https://raw.githubusercontent.com/mbround18/paws/main/scripts/install.sh | sh\n\n\
         # 2. Install dagger, which most paws subcommands need on PATH\n\
         paws init\n\n\
         # 3. Register paws as an MCP server for this client (writes/merges .mcp.json;\n\
         #    pass --client claude-desktop instead for Claude Desktop's global config)\n\
         paws mcp setup\n\n\
         # 4. Scaffold a starter GitHub Actions workflow for this repo, if it doesn't have\n\
         #    one yet (detects the repo's ecosystem(s) automatically)\n\
         paws workflow generate\n\
         ```\n\n\
         After step 3, restart/reload the MCP client (or start a new session) so it picks up \
         `.mcp.json` — every subcommand documented below then becomes available as an MCP tool \
         (`paws mcp serve`), calling the same code the CLI does, not a subprocess. In CI, prefer \
         `mbround18/paws/actions/paws-up@main` over the install script (see the \"GitHub \
         Actions\" section below) — it's the same install, packaged as a composite Action.\n\n",
    );

    fn render_command(cmd: &clap::Command, prefix: &str, out: &mut String) {
        let name = format!("{prefix}{}", cmd.get_name());
        out.push_str(&format!("## paws {name}\n\n"));
        if let Some(about) = cmd.get_about() {
            out.push_str(&format!("{about}\n\n"));
        }

        let flags: Vec<_> = cmd
            .get_arguments()
            .filter(|a| a.get_long().is_some())
            .collect();
        if !flags.is_empty() {
            for flag in flags {
                let long = flag.get_long().unwrap_or_default();
                let help = flag.get_help().map(|h| h.to_string()).unwrap_or_default();
                let default = flag
                    .get_default_values()
                    .first()
                    .map(|v| v.to_string_lossy().to_string());
                match default {
                    Some(default) if !default.is_empty() => {
                        out.push_str(&format!("- `--{long}` (default: `{default}`) — {help}\n"));
                    }
                    _ => {
                        out.push_str(&format!("- `--{long}` — {help}\n"));
                    }
                }
            }
            out.push('\n');
        }

        for sub in cmd.get_subcommands() {
            render_command(sub, &format!("{name} "), out);
        }
    }

    for sub in root.get_subcommands() {
        render_command(sub, "", &mut out);
    }

    if let Ok(actions) = crate::action_metadata::discover_actions()
        && !actions.is_empty()
    {
        out.push_str("## GitHub Actions\n\n");
        out.push_str(
            "paws also ships composite GitHub Actions for wiring into a *consumer* repo's own \
             CI/CD, separate from the CLI subcommands above — `paws workflow generate` scaffolds \
             a starter workflow using these automatically.\n\n",
        );
        for action in &actions {
            out.push_str(&format!("### {}\n\n", action.id));
            if !action.description.is_empty() {
                out.push_str(&format!("{}\n\n", action.description));
            }

            out.push_str("```yaml\n");
            out.push_str(&format!("- uses: {}\n", action.usage));
            if !action.inputs.is_empty() {
                out.push_str("  with:\n");
                for input in &action.inputs {
                    let value = input.default.clone().unwrap_or_else(|| "...".to_string());
                    out.push_str(&format!("    {}: {value}\n", input.name));
                }
            }
            out.push_str("```\n\n");

            if !action.inputs.is_empty() {
                out.push_str("**Inputs**\n\n");
                for input in &action.inputs {
                    let requiredness = if input.required {
                        "required"
                    } else {
                        "optional"
                    };
                    let default = input
                        .default
                        .as_ref()
                        .map(|d| format!(", default: `{d}`"))
                        .unwrap_or_default();
                    out.push_str(&format!(
                        "- `{}` ({requiredness}{default}) — {}\n",
                        input.name, input.description
                    ));
                }
                out.push('\n');
            }
            if !action.outputs.is_empty() {
                out.push_str("**Outputs**\n\n");
                for output in &action.outputs {
                    out.push_str(&format!("- `{}` — {}\n", output.name, output.description));
                }
                out.push('\n');
            }
        }
    }

    out
}

/// Pure comparison behind `run_llms_generate`'s publish loop-guard —
/// extracted so it's unit-testable without a real GitHub API call. `None`
/// (nothing published yet) always means "publish"; identical bytes means
/// "skip" (prevents committing on every push to `main`, including the
/// commit the publish itself just created, from retriggering forever).
fn should_publish(existing: Option<&[u8]>, generated: &[u8]) -> bool {
    match existing {
        None => true,
        Some(existing) => existing != generated,
    }
}

pub async fn run_llms_generate(args: GenerateArgs) -> anyhow::Result<()> {
    let GenerateArgs {
        output,
        publish,
        branch,
        repository,
    } = args;

    let rendered = render_llms_txt();
    tokio::fs::write(&output, &rendered)
        .await
        .with_context(|| format!("failed to write {output}"))?;
    println!("llms: generated {output} ({} bytes)", rendered.len());

    if !publish {
        return Ok(());
    }

    let (owner, repo, token) = if let Some(repository) = repository {
        let (owner, repo) = repository.split_once('/').ok_or_else(|| {
            anyhow::anyhow!("--repository must be \"owner/repo\", got {repository}")
        })?;
        let token = paws_environment::resolve_github_token(owner, repo).await?;
        (owner.to_string(), repo.to_string(), token)
    } else {
        let ctx = paws_environment::CiContext::detect()
            .await
            .context("paws llms generate --publish needs $GITHUB_REPOSITORY (or --repository)")?;
        (ctx.owner, ctx.repo, ctx.token)
    };

    let client = GitHubReleaseClient::new(owner, repo, token);
    let existing = client.get_content(&output, &branch).await?;

    // Loop guard: committing on every push to `main` (including the commit
    // this very publish creates) would retrigger the workflow forever if we
    // always wrote, even with unchanged content.
    if !should_publish(
        existing.as_ref().map(|e| e.content.as_slice()),
        rendered.as_bytes(),
    ) {
        println!("llms: {output}@{branch} already up to date, skipping publish");
        return Ok(());
    }

    client
        .put_content(
            &output,
            &branch,
            rendered.as_bytes(),
            // `[skip ci]` is GitHub Actions' own recognized marker (checked
            // against the pushed commit's message, no workflow YAML changes
            // needed) — without it, `should_publish`'s loop guard still
            // stops this from looping forever, but this publish's own push
            // event would otherwise retrigger one full redundant CI run
            // before the guard kicks in on the next one.
            "chore: regenerate llms.txt [skip ci]",
            existing.as_ref().map(|e| e.sha.as_str()),
        )
        .await?;
    println!("llms: published {output}@{branch}");

    Ok(())
}

/// Mints a GitHub App installation token and prints *only* the token to
/// stdout — see [`AuthCommand::GithubApp`]'s doc comment for why (shell
/// capture via `$(paws auth github-app)`). Diagnostics go to stderr, the
/// same stdout/stderr split `run_semver` already uses for its version
/// output.
pub async fn run_auth_github_app(args: GithubAppLoginArgs) -> anyhow::Result<()> {
    let GithubAppLoginArgs {
        client_id,
        private_key,
        private_key_file,
        repository,
    } = args;

    let client_id = client_id
        .or_else(|| std::env::var("GH_APP_CLIENT_ID").ok())
        .ok_or_else(|| anyhow::anyhow!("--client-id is required (or set $GH_APP_CLIENT_ID)"))?;

    let private_key_pem = if let Some(path) =
        private_key_file.or_else(|| std::env::var("GH_APP_PRIVATE_KEY_FILE").ok())
    {
        tokio::fs::read_to_string(&path)
            .await
            .with_context(|| format!("failed to read --private-key-file ({path})"))?
    } else {
        private_key
            .or_else(|| std::env::var("GH_APP_PRIVATE_KEY").ok())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "--private-key (or --private-key-file) is required (or set \
                     $GH_APP_PRIVATE_KEY/$GH_APP_PRIVATE_KEY_FILE)"
                )
            })?
    };

    let repository = repository
        .or_else(|| std::env::var("GITHUB_REPOSITORY").ok())
        .ok_or_else(|| anyhow::anyhow!("--repository is required (or set $GITHUB_REPOSITORY)"))?;
    let (owner, repo) = repository
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("--repository must be \"owner/repo\", got {repository}"))?;

    let creds = paws_environment::GitHubAppCredentials {
        client_id,
        private_key_pem,
    };
    let token = paws_environment::mint_github_app_installation_token(&creds, owner, repo).await?;

    eprintln!("auth: minted a GitHub App installation token for {owner}/{repo}");
    println!("{token}");

    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn llms_txt_covers_every_subcommand() {
        let rendered = super::render_llms_txt();
        for name in [
            "ci",
            "docker",
            "semver",
            "init",
            "audit",
            "docs",
            "provision",
            "helm",
            "release",
            "mcp setup",
            "mcp serve",
            "llms generate",
            "workflow generate",
            "auth github-app",
        ] {
            assert!(
                rendered.contains(&format!("## paws {name}")),
                "expected llms.txt to document `paws {name}`, got:\n{rendered}"
            );
        }
    }

    #[test]
    fn llms_txt_documents_the_paws_up_github_action() {
        let rendered = super::render_llms_txt();
        assert!(rendered.contains("## GitHub Actions"));
        assert!(rendered.contains("### paws-up"));
        assert!(rendered.contains("mbround18/paws/actions/paws-up@main"));
        assert!(rendered.contains("`version`"));
    }

    /// The whole point of this section: someone (or an agent) can paste
    /// `llms.txt`'s contents into an AI coding assistant and get `paws`
    /// wired into a repo without a human walking through each step —
    /// pin the exact commands that promise covers, and that it appears
    /// before the per-command reference (so a reader/agent sees "how do I
    /// start" before "here's every flag").
    #[test]
    fn llms_txt_has_an_ai_agent_bootstrap_section_before_the_command_reference() {
        let rendered = super::render_llms_txt();
        let bootstrap_pos = rendered
            .find("## Quickstart for an AI agent")
            .expect("expected an AI-agent quickstart section");
        let first_command_pos = rendered
            .find("## paws ci")
            .expect("expected the paws ci command section");
        assert!(
            bootstrap_pos < first_command_pos,
            "the AI-agent quickstart should appear before the per-command reference"
        );

        for expected in [
            "scripts/install.sh",
            "paws init",
            "paws mcp setup",
            "paws workflow generate",
            "mbround18/paws/actions/paws-up@main",
        ] {
            assert!(
                rendered.contains(expected),
                "expected the bootstrap section to mention {expected:?}"
            );
        }
    }

    #[test]
    fn workflow_render_includes_only_detected_ecosystems() {
        let detected = DetectedWorkflowInputs {
            rust: true,
            docker: true,
            ..Default::default()
        };
        let rendered = render_github_workflow(&detected).expect("something was detected");
        assert!(rendered.contains("paws ci --toolchain rust"));
        assert!(rendered.contains("paws docker"));
        assert!(!rendered.contains("paws ci --toolchain node"));
        assert!(!rendered.contains("paws ci --toolchain python"));
        assert!(!rendered.contains("paws helm"));
        assert!(rendered.contains("mbround18/paws/actions/paws-up@main"));
    }

    #[test]
    fn workflow_render_is_none_when_nothing_is_detected() {
        assert_eq!(
            render_github_workflow(&DetectedWorkflowInputs::default()),
            None
        );
    }

    #[test]
    fn should_publish_when_nothing_exists_yet() {
        assert!(should_publish(None, b"generated content"));
    }

    #[test]
    fn should_publish_skips_when_content_is_identical() {
        assert!(!should_publish(Some(b"same bytes"), b"same bytes"));
    }

    #[test]
    fn should_publish_when_content_differs() {
        assert!(should_publish(Some(b"old bytes"), b"new bytes"));
    }

    /// The whole point of deriving `serde::Deserialize` on the same struct
    /// `clap::Args` uses is that an MCP tool call omitting an optional
    /// field should behave exactly like a CLI invocation omitting the
    /// matching flag — no "missing field" errors for anything that isn't
    /// genuinely required. `paws-mcp`'s own tests caught `ProvisionArgs`
    /// failing this for real (a bare `{}` `tools/call` for `provision`
    /// errored with `missing field toolchains`); this test pins the fix and
    /// guards every other `*Args` struct against the same regression class.
    /// `ReleaseArgs.target` is the one field that's genuinely required on
    /// both sides (`target: String` has no clap `default_value` either), so
    /// it's deserialized from `{"target": "x"}` instead of `{}`.
    #[test]
    fn every_tool_args_struct_deserializes_from_an_empty_object() {
        serde_json::from_str::<McpSetupArgs>("{}").expect("McpSetupArgs");
        serde_json::from_str::<GenerateArgs>("{}").expect("GenerateArgs");
        serde_json::from_str::<CiArgs>("{}").expect("CiArgs");
        serde_json::from_str::<DockerArgs>("{}").expect("DockerArgs");
        serde_json::from_str::<SemverArgs>("{}").expect("SemverArgs");
        serde_json::from_str::<InitArgs>("{}").expect("InitArgs");
        serde_json::from_str::<AuditArgs>("{}").expect("AuditArgs");
        serde_json::from_str::<DocsArgs>("{}").expect("DocsArgs");
        serde_json::from_str::<ProvisionArgs>("{}").expect("ProvisionArgs");
        serde_json::from_str::<HelmArgs>("{}").expect("HelmArgs");
        let workflow: WorkflowGenerateArgs =
            serde_json::from_str("{}").expect("WorkflowGenerateArgs");
        assert_eq!(workflow.provider, "github");
        assert_eq!(workflow.output, ".github/workflows/paws.yml");
        serde_json::from_str::<GithubAppLoginArgs>("{}").expect("GithubAppLoginArgs");
        let release: ReleaseArgs =
            serde_json::from_str(r#"{"target": "x86_64-unknown-linux-gnu"}"#)
                .expect("ReleaseArgs with only the required field set");
        assert_eq!(release.source, ".");
        assert_eq!(release.package, vec!["paws-cli".to_string()]);
        assert_eq!(release.binary_name, vec!["paws".to_string()]);
    }

    /// Complements the empty-object test above: the *values* filled in by
    /// `#[serde(default = "...")]` must actually match clap's
    /// `default_value` for that same flag, not just be present — a typo'd
    /// default fn would pass the emptiness check but silently diverge from
    /// CLI behavior.
    #[test]
    fn serde_defaults_match_clap_default_values() {
        let generate: GenerateArgs = serde_json::from_str("{}").unwrap();
        assert_eq!(generate.output, "llms.txt");
        assert_eq!(generate.branch, "main");

        let docker: DockerArgs = serde_json::from_str("{}").unwrap();
        assert_eq!(docker.canary_label, "canary");
        assert_eq!(docker.default_branch, "main");

        let semver: SemverArgs = serde_json::from_str("{}").unwrap();
        assert_eq!(semver.major_label, "major");
        assert_eq!(semver.minor_label, "minor");
        assert_eq!(semver.patch_label, "patch");
        assert_eq!(semver.branch, "main");
        assert_eq!(semver.tagger_name, "paws-bot");
        assert_eq!(semver.tagger_email, "paws-bot@users.noreply.github.com");

        let helm: HelmArgs = serde_json::from_str("{}").unwrap();
        assert_eq!(helm.source, ".");
        assert_eq!(helm.output, "tmp");
        assert_eq!(helm.pages_branch, "gh-pages");
        assert_eq!(helm.index_path, "index.yaml");
    }
}
