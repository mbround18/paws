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
        /// Suppress dagger's live build progress; only print output once
        /// the pipeline finishes (or on failure). Default is streamed live.
        #[arg(long)]
        silent: bool,
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
        /// Docker Hub username to authenticate publishing with. Falls back to
        /// $DOCKERHUB_USERNAME. Required (here or via env) to actually push
        /// to docker.io — without it, `dockerRelease` builds but can't
        /// authenticate, so `push=true` still publishes nothing.
        #[arg(long)]
        dockerhub_username: Option<String>,
        /// GHCR username to authenticate publishing with. Falls back to
        /// $GHCR_USERNAME. Required (here or via env) to actually push to
        /// ghcr.io.
        #[arg(long)]
        ghcr_username: Option<String>,
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
        registry_username: Vec<String>,
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
        /// Create and push the computed version as an annotated git tag,
        /// then create a matching GitHub Release with auto-generated notes
        /// (GitHub's git/tags + git/refs + releases APIs — no local git
        /// identity or worktree needed). Replaces a hand-rolled
        /// `git tag`/`git push`/`gh release create` step in the calling
        /// workflow.
        #[arg(long)]
        push: bool,
        /// Tagger identity attributed to the pushed tag.
        #[arg(long, default_value = "paws-bot")]
        tagger_name: String,
        #[arg(long, default_value = "paws-bot@users.noreply.github.com")]
        tagger_email: String,
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
    /// Lint (and optionally package) Helm chart(s) — `charts/*/Chart.yaml`
    /// or a root `Chart.yaml`.
    Helm {
        /// Host path to the source tree to scan for chart(s).
        #[arg(long, default_value = ".")]
        source: String,
        /// Also `helm package` every chart after linting, exported to `--output`.
        #[arg(long)]
        package: bool,
        /// Host directory packaged `.tgz`s are exported to (only with `--package`).
        #[arg(long, default_value = "tmp")]
        output: String,
        /// Publish: a per-chart GitHub Release (tag `<chart>-<version>`,
        /// asset uploaded only if missing) plus a real Helm `index.yaml`
        /// pushed to `--pages-branch`, so `helm repo add` against this
        /// repo's GitHub Pages URL works. Does its own packaging
        /// internally (per-chart, not the flat `--output` directory);
        /// mutually exclusive with `--package`.
        #[arg(long)]
        publish: bool,
        /// "owner/repo" to publish releases/index.yaml to. Falls back to
        /// $GITHUB_REPOSITORY. Only used with `--publish`.
        #[arg(long)]
        repository: Option<String>,
        /// Branch `index.yaml` is published to. Only used with `--publish`.
        #[arg(long, default_value = "gh-pages")]
        pages_branch: String,
        /// Path to `index.yaml` on `--pages-branch`. Only used with `--publish`.
        #[arg(long, default_value = "index.yaml")]
        index_path: String,
        /// Suppress dagger's live build progress; only print output once
        /// the pipeline finishes (or on failure). Default is streamed live.
        #[arg(long)]
        silent: bool,
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
        /// Cargo package(s) to build, comma-separated (one [[bin]] each) —
        /// e.g. "agent,server". Paired 1:1 with --binary-name.
        #[arg(long, default_value = "paws-cli", value_delimiter = ',')]
        package: Vec<String>,
        /// Binary name(s) as declared in each package's [[bin]] section,
        /// comma-separated, paired 1:1 with --package. All built binaries
        /// are packaged into one archive.
        #[arg(long, default_value = "paws", value_delimiter = ',')]
        binary_name: Vec<String>,
        /// Build locally via Dagger's `docker-build` against paws's embedded
        /// generic Rust-Linux builder instead of pulling paws's own prebuilt
        /// builder image. Use this outside paws's own repo (e.g. a target
        /// repo with no `builders/` directory) — only
        /// `paws_release::local_build_targets()` are supported.
        #[arg(long)]
        local_build: bool,
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
        Commands::Ci {
            toolchain,
            verbose,
            silent,
        } => {
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
                    } else if project.has_playwright {
                        println!(
                            "ci: playwright project using {} ({})",
                            project.package_manager.as_str(),
                            dir.display()
                        );
                        let args = paws_node::playwright_dagger_pipeline_args(
                            &project,
                            &dir.to_string_lossy(),
                        );
                        run_dagger_core(&args, silent).await?;
                        println!("ci: playwright tests succeeded");
                    } else {
                        println!(
                            "ci: {} project using {} ({})",
                            project.framework.as_str(),
                            project.package_manager.as_str(),
                            dir.display()
                        );
                        let args =
                            paws_node::dagger_pipeline_args(&project, &dir.to_string_lossy());
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
                    println!("ci: rust project ({})", dir.display());
                    let args = paws_rust::dagger_pipeline_args(&dir.to_string_lossy());
                    run_dagger_core(&args, silent).await?;
                    println!("ci: rust build/test succeeded");
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
                    "unsupported --toolchain '{other}'; expected 'node', 'rust', 'python', 'tauri', 'tauri-android', or 'flatpak'"
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
            dockerhub_username,
            ghcr_username,
            registry_username,
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
            let dockerhub_username =
                resolve_docker_credential(dockerhub_username, "DOCKERHUB_USERNAME");
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
                        paws_dagger::core(&publish_args)
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
                let build_only_args =
                    paws_docker::build_only_pipeline_args(&paws_docker::BuildSpec {
                        context: &facts.context,
                        dockerfile: &facts.dockerfile,
                        target: &facts.target,
                        build_args: &facts.build_args,
                    });
                run_dagger_core(&build_only_args, false).await?;
                println!("docker: build succeeded");
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
            push,
            tagger_name,
            tagger_email,
        } => {
            let ctx = paws_environment::CiContext::detect()
                .context("paws semver needs a supported CI provider's env vars")?;
            let labels = if labels.is_empty() {
                match paws_semver::fetch_pr_labels_for_commit(
                    &ctx.owner, &ctx.repo, &ctx.sha, &ctx.token,
                )
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
                    paws_dagger::core(&paws_audit::scanner_json_pipeline_args(&source, scanner))
                        .await;
                let exit_code_output = paws_dagger::core(
                    &paws_audit::scanner_exit_code_pipeline_args(&source, scanner),
                )
                .await;
                let duration_ms = started.elapsed().as_millis() as u64;

                let result = match (raw_json, exit_code_output) {
                    (Ok(raw_json), Ok(exit_code_raw)) => {
                        let exit_code = exit_code_raw.trim().parse::<i32>().ok();
                        let (findings_count, top_findings) =
                            paws_audit::parse_scanner_findings(scanner.name, &raw_json);
                        let status =
                            paws_audit::normalize_scanner_status(exit_code, findings_count);
                        paws_audit::AuditScannerResult {
                            name: scanner.name.as_str().to_string(),
                            family: scanner.family,
                            status,
                            findings_count,
                            duration_ms,
                            failure_reason: (status == paws_audit::ScannerStatus::Failed).then(
                                || format!("{} exited {:?}", scanner.name.as_str(), exit_code),
                            ),
                            top_findings,
                        }
                    }
                    (Err(err), _) | (_, Err(err)) => paws_audit::create_failed_scanner_result(
                        scanner,
                        duration_ms,
                        err.to_string(),
                    ),
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
        Commands::Helm {
            source,
            package,
            output,
            publish,
            repository,
            pages_branch,
            index_path,
            silent,
        } => {
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
                let token = std::env::var("GITHUB_TOKEN")
                    .or_else(|_| std::env::var("GH_TOKEN"))
                    .map_err(|_| {
                        anyhow::anyhow!("GITHUB_TOKEN (or GH_TOKEN) must be set to publish")
                    })?;
                let client = GitHubReleaseClient::new(owner.to_string(), repo.to_string(), token);

                let existing = client.get_content(&index_path, &pages_branch).await?;
                let existing_index_file = if let Some(existing) = &existing {
                    let path = std::env::temp_dir().join("paws-helm-existing-index.yaml");
                    tokio::fs::write(&path, &existing.content).await.context(
                        "failed to stage the existing index.yaml for the publish pipeline",
                    )?;
                    println!("helm: seeding from the existing {index_path}@{pages_branch}");
                    Some(path)
                } else {
                    println!(
                        "helm: no existing {index_path}@{pages_branch} found, publishing fresh"
                    );
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
        }
        Commands::Release {
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
        } => {
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
