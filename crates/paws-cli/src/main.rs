use anyhow::Context;
use clap::{Parser, Subcommand};
use paws_audit::{RepositorySignals, select_audit_scanners};
use paws_dagger::{DaggerCall, call};
use paws_docker::{DockerFactsInput, GithubContext as DockerGithubContext, resolve_docker_facts};
use paws_provision::{Ecosystem, Installer, provision_with_timing, real_installer};
use paws_release::{GitHubReleaseClient, archive_name, package_zip};
use paws_semver::{GitHubGraphQlTagSource, Increment, SemverRequest, compute_new_version};

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
    ecosystems
}

/// Calls a `gh-reusable` Dagger pipeline function, prints its human-readable
/// `markdown` report field (falling back to the raw output if the response
/// isn't the expected `{success, markdown, ...}` shape), and returns whether
/// it reported success — callers use this to decide their own exit code
/// rather than always exiting 0 on a successful `dagger call` process spawn.
async fn call_pipeline_report(function: &str, args: Vec<String>) -> anyhow::Result<bool> {
    let output = call(DaggerCall {
        module: GH_REUSABLE_DAGGER_MODULE.into(),
        function: function.into(),
        args,
    })
    .await?;

    let parsed: serde_json::Value =
        serde_json::from_str(&output).unwrap_or(serde_json::Value::Null);
    if let Some(markdown) = parsed.get("markdown").and_then(|v| v.as_str()) {
        println!("{markdown}");
    } else {
        println!("{output}");
    }

    Ok(parsed
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(true))
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

/// Interim reference to `gh-reusable`'s existing TS Dagger module. Each
/// subcommand below routes through this until it gets its own native crate
/// (see specs/001-paws-core-cli/tasks.md task groups 2-4); this constant is
/// expected to shrink to nothing as that happens, not grow.
///
/// Pinned to a commit, not `main`: floating `main` was verified broken as of
/// 2026-08-18 (`33b7761`) — the module's vendored Dagger TS SDK bundle threw
/// `TypeError: Cannot read properties of undefined (reading 'ClassDeclaration')`
/// at runtime against this environment's dagger v0.21.8 engine, while this
/// pinned commit was verified working end-to-end (`rust-build-and-test`
/// against `examples/rust-fixture`). Bump only after re-verifying a real
/// `dagger call` against the new commit succeeds, not on trust.
const GH_REUSABLE_DAGGER_MODULE: &str = "github.com/mbround18/gh-reusable/packages/dagger-module@7fbda5676b56479aa458b1ecdc0313ed1a1cc934";

/// paws: run-anywhere CI/CD pipelines, backed by Dagger.
#[derive(Parser)]
#[command(name = "paws", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build and test a language target (node, rust, python, tauri, tauri-android, ...).
    Ci {
        #[arg(long)]
        toolchain: Option<String>,
        /// Print per-ecosystem provisioning start/elapsed timing to stderr.
        #[arg(long)]
        verbose: bool,
    },
    /// Build and gate a container image the same way `docker-facts` + `docker-release` do.
    Docker {
        /// Image name, e.g. "ghcr.io/example/app". Falls back to $GITHUB_REPOSITORY.
        #[arg(long)]
        image: Option<String>,
        /// Version to tag with. Falls back to $GITHUB_SHA (short).
        #[arg(long)]
        version: Option<String>,
        /// Additional registries to mirror tags into, comma-separated.
        #[arg(long, value_delimiter = ',')]
        registries: Vec<String>,
        #[arg(long)]
        dockerfile: Option<String>,
        #[arg(long)]
        context: Option<String>,
        #[arg(long, default_value = "canary")]
        canary_label: String,
        /// Force push regardless of branch/tag/label gating.
        #[arg(long)]
        push: bool,
        #[arg(long)]
        with_latest: bool,
        #[arg(long)]
        target: Option<String>,
        #[arg(long)]
        prepend_target: bool,
        /// PR labels to check against --canary-label, comma-separated.
        #[arg(long, value_delimiter = ',')]
        labels: Vec<String>,
        #[arg(long, default_value = "main")]
        default_branch: String,
    },
    /// Compute the next semantic version from PR labels or an explicit increment,
    /// matching `actions/semver`'s current behavior.
    Semver {
        /// Base version to start from instead of the last matching tag.
        #[arg(long)]
        base: Option<String>,
        /// Prefix used to filter/build tag versions, e.g. "chart-name-".
        #[arg(long)]
        prefix: Option<String>,
        /// Explicit increment (major, minor, patch); skips label/branch inference.
        #[arg(long)]
        increment: Option<Increment>,
        #[arg(long, default_value = "major")]
        major_label: String,
        #[arg(long, default_value = "minor")]
        minor_label: String,
        #[arg(long, default_value = "patch")]
        patch_label: String,
        /// PR/commit labels to check against major/minor/patch-label, comma-separated.
        #[arg(long, value_delimiter = ',')]
        labels: Vec<String>,
        /// Branch name used for fallback inference when no configured label matches.
        #[arg(long, default_value = "main")]
        branch: String,
        /// Whether this is a PR build (produces a `-pr.<sha>` prerelease).
        #[arg(long)]
        pr: bool,
    },
    /// Install the `dagger` CLI (most other subcommands need it on PATH).
    Init,
    /// Run the audit/compliance scanner suite.
    Audit,
    /// Publish generated docs (e.g. rustdoc) to GitHub Pages.
    Docs,
    /// Provision toolchains concurrently (rust, node, python, ...).
    Provision {
        /// Comma-separated ecosystems to install, e.g. "rust,node,python".
        #[arg(long, value_delimiter = ',')]
        toolchains: Vec<String>,
        /// Print per-ecosystem provisioning start/elapsed timing to stderr.
        #[arg(long)]
        verbose: bool,
    },
    /// Cross-target build, package, and publish a release binary to GitHub Releases.
    Release {
        /// Rust target triple to build, e.g. "x86_64-unknown-linux-gnu".
        /// Must be one of `paws_release::known_targets()` — each maps to a
        /// `./builders/<dir>` Dockerfile the build runs through Dagger.
        #[arg(long)]
        target: String,
        /// Host path to the source tree to build.
        #[arg(long, default_value = ".")]
        source: String,
        /// Cargo package to build (produces one [[bin]]).
        #[arg(long, default_value = "paws-cli")]
        package: String,
        /// Binary name as declared in the package's [[bin]] section.
        #[arg(long, default_value = "paws")]
        binary_name: String,
        /// Release tag, e.g. "v0.0.1-prerelease.1". Falls back to $GITHUB_REF_NAME.
        #[arg(long)]
        tag: Option<String>,
        /// Mark the GitHub Release as a prerelease.
        #[arg(long)]
        prerelease: bool,
        /// "owner/repo". Falls back to $GITHUB_REPOSITORY.
        #[arg(long)]
        repository: Option<String>,
        /// Build and package only; skip the GitHub upload.
        #[arg(long)]
        no_upload: bool,
        /// Skip the post-build smoke test (not recommended — it's what
        /// catches a binary that builds but doesn't actually run).
        #[arg(long)]
        skip_smoke_test: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Ci { toolchain, verbose } => {
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
                    if !is_tauri && !missing.is_empty() {
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
                        let output = paws_dagger::core(&args).await?;
                        print!("{output}");
                        println!("ci: tauri build succeeded");
                    } else {
                        println!(
                            "ci: {} project using {} ({})",
                            project.framework.as_str(),
                            project.package_manager.as_str(),
                            dir.display()
                        );
                        let args =
                            paws_node::dagger_pipeline_args(&project, &dir.to_string_lossy());
                        let output = paws_dagger::core(&args).await?;
                        print!("{output}");
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
                    let output = paws_dagger::core(&args).await?;
                    print!("{output}");
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
                    let output = paws_dagger::core(&args).await?;
                    print!("{output}");
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
                    println!("ci: rust project ({})", dir.display());
                    let args = paws_rust::dagger_pipeline_args(&dir.to_string_lossy());
                    let output = paws_dagger::core(&args).await?;
                    print!("{output}");
                    println!("ci: rust build/test succeeded");
                }
                Some(other) => anyhow::bail!(
                    "unsupported --toolchain '{other}'; expected 'node', 'rust', 'python', 'tauri', or 'tauri-android'"
                ),
                None => anyhow::bail!("--toolchain is required (e.g. --toolchain node)"),
            }
        }
        Commands::Docker {
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
        } => {
            let image = image
                .or_else(|| std::env::var("GITHUB_REPOSITORY").ok())
                .ok_or_else(|| {
                    anyhow::anyhow!("--image is required (or set $GITHUB_REPOSITORY)")
                })?;
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

            // Locally-resolved preview so `paws docker` stays useful (and
            // testable per spec.md User Story 3) without needing `dagger` —
            // the real `dockerRelease` function below recomputes context/
            // dockerfile/target/push itself from the same compose-resolution
            // and gating rules (`paws-docker` is a parity port of that logic,
            // not a second source of truth for the actual build).
            eprintln!(
                "docker: resolved locally -> context={} dockerfile={} target={} push={}",
                facts.context, facts.dockerfile, facts.target, facts.push
            );

            paws_dagger::ensure_available().await?;
            let source = workspace.to_string_lossy().to_string();
            let mut args = vec!["--image".into(), image, "--source".into(), source];
            if let Some(dockerfile) = dockerfile {
                args.extend(["--dockerfile".into(), dockerfile]);
            }
            if let Some(context) = context {
                args.extend(["--context".into(), context]);
            }
            if let Some(target) = target {
                args.extend(["--target".into(), target]);
            }
            args.extend(["--canary-label".into(), canary_label]);
            args.extend(["--default-branch".into(), default_branch]);
            if !registries.is_empty() {
                args.extend(["--registries-csv".into(), registries.join(",")]);
            }
            if !labels.is_empty() {
                args.extend(["--pr-labels-csv".into(), labels.join(",")]);
            }
            if push {
                args.push("--force-push".into());
            }
            if prepend_target {
                args.push("--prepend-target".into());
            }
            if let Ok(v) = std::env::var("GITHUB_EVENT_NAME") {
                args.extend(["--event-name".into(), v]);
            }
            if let Ok(v) = std::env::var("GITHUB_REF") {
                args.extend(["--ref".into(), v]);
            }
            if let Ok(v) = std::env::var("GITHUB_SHA") {
                args.extend(["--sha".into(), v]);
            }
            // NOTE: version/tag control (`--semver-prefix`/`--semver-increment`/
            // `--tags-csv`) isn't wired through yet — it needs the same
            // existing-tag lookup `paws-semver`'s `TagSource` already does,
            // just not plumbed into this subcommand. Until then the real
            // `dockerRelease` call falls back to its own defaults
            // (patch increment, no prefix, no existing tags) for versioning.
            let succeeded = call_pipeline_report("docker-release", args).await?;
            if !succeeded {
                anyhow::bail!("docker release failed: see report above");
            }
        }
        Commands::Semver {
            base,
            prefix,
            increment,
            major_label,
            minor_label,
            patch_label,
            labels,
            branch,
            pr,
        } => {
            let request = SemverRequest {
                base,
                prefix,
                explicit_increment: increment,
                major_label,
                minor_label,
                patch_label,
                labels,
                branch_name: branch,
                sha: std::env::var("GITHUB_SHA").unwrap_or_default(),
                is_pr: pr,
                github_ref: std::env::var("GITHUB_REF").ok(),
            };
            let owner = std::env::var("GITHUB_REPOSITORY_OWNER").unwrap_or_default();
            let repo = std::env::var("GITHUB_REPOSITORY")
                .ok()
                .and_then(|r| r.split('/').next_back().map(str::to_string))
                .unwrap_or_default();
            let token = std::env::var("GITHUB_TOKEN").unwrap_or_default();
            let tag_source = GitHubGraphQlTagSource { owner, repo, token };

            let version = compute_new_version(&tag_source, &request).await?;
            println!("{version}");
        }
        Commands::Init => {
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
        }
        Commands::Audit => {
            // Local pre-check only: `paws-audit`'s detection logic decides
            // whether it's worth spinning up `dagger` at all (spec.md's
            // "outside a Cargo/Node/Docker project entirely" edge case). The
            // actual scan run is a single call to the real `audit` function,
            // which does its own detection/scanner-selection/aggregation
            // internally — `paws-audit`'s aggregation logic isn't reinvoked
            // here since it would just be redoing what that call already did.
            let signals = collect_repository_signals();
            let detection = paws_audit::detect_language_families(&signals);
            let scanners = select_audit_scanners(&detection, true);
            if !scanners.iter().any(|s| s.should_run) {
                println!("audit: no recognizable project markers found here; nothing to scan.");
                return Ok(());
            }

            paws_dagger::ensure_available().await?;
            let source = std::env::current_dir()?.to_string_lossy().to_string();
            let succeeded = call_pipeline_report("audit", vec!["--source".into(), source]).await?;
            if !succeeded {
                anyhow::bail!("audit failed: see scanner findings above");
            }
        }
        Commands::Docs => {
            let workspace = std::env::current_dir()?;
            let docs_dir = paws_docs::build_docs(&workspace).await?;
            println!("docs: built at {}", docs_dir.display());
        }
        Commands::Provision {
            toolchains,
            verbose,
        } => {
            if toolchains.is_empty() {
                anyhow::bail!("--toolchains is required (e.g. --toolchains rust,node,python)");
            }
            let ecosystems = toolchains
                .iter()
                .map(|t| t.parse::<Ecosystem>())
                .collect::<anyhow::Result<Vec<_>>>()?;
            run_provisioning(ecosystems, verbose).await?;
            println!("provision: all requested toolchains provisioned successfully");
        }
        Commands::Release {
            target,
            source,
            package,
            binary_name,
            tag,
            prerelease,
            repository,
            no_upload,
            skip_smoke_test,
        } => {
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

            println!(
                "release: building {binary_name} for {target} via {}...",
                target_config.builder_dir
            );
            let binary_path = paws_release::build_binary(&paws_release::BuildRequest {
                builder_dir: target_config.builder_dir,
                source_dir: &source,
                triple: &target,
                package: &package,
                binary_name: &binary_name,
                builder_version: &raw_tag,
            })
            .await?;
            println!("release: built {}", binary_path.display());

            match (&target_config.smoke, skip_smoke_test) {
                (_, true) => println!("release: --skip-smoke-test set, skipping"),
                (None, false) => {
                    println!(
                        "release: no execution environment available for {target}, skipping smoke test (build/link success only)"
                    );
                }
                (Some(spec), false) => {
                    println!("release: smoke testing...");
                    let smoke_output = paws_release::smoke_test(&binary_path, spec).await?;
                    println!("release: smoke test output: {}", smoke_output.trim());
                }
            }

            let archive = archive_name(&binary_name, &version, &target);
            let archive_path = std::path::Path::new("target")
                .join("release-archives")
                .join(&archive);
            let relative_binary = binary_path.to_string_lossy().to_string();
            package_zip(&std::env::current_dir()?, &archive_path, &[relative_binary]).await?;
            println!("release: packaged {}", archive_path.display());

            if no_upload {
                println!("release: --no-upload set, skipping GitHub upload");
                return Ok(());
            }

            let tag = tag.ok_or_else(|| {
                anyhow::anyhow!("--tag is required to upload (or set $GITHUB_REF_NAME)")
            })?;
            let repository = repository
                .or_else(|| std::env::var("GITHUB_REPOSITORY").ok())
                .ok_or_else(|| {
                    anyhow::anyhow!("--repository is required (or set $GITHUB_REPOSITORY)")
                })?;
            let (owner, repo) = repository.split_once('/').ok_or_else(|| {
                anyhow::anyhow!("--repository must be \"owner/repo\", got {repository}")
            })?;
            let token = std::env::var("GITHUB_TOKEN")
                .or_else(|_| std::env::var("GH_TOKEN"))
                .map_err(|_| {
                    anyhow::anyhow!(
                        "GITHUB_TOKEN (or GH_TOKEN) must be set to upload a release asset"
                    )
                })?;

            let client = GitHubReleaseClient::new(owner.to_string(), repo.to_string(), token);
            let release_id = client.get_or_create_release(&tag, prerelease).await?;
            client.upload_asset(release_id, &archive_path).await?;
            println!("release: uploaded {archive} to {repository}@{tag}");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::Cli;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
