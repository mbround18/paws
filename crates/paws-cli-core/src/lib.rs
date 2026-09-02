mod args;

pub use args::*;

use std::fmt::Write as _;

use anyhow::Context;
use paws_audit::{RepositorySignals, select_audit_scanners};
use paws_core::Toolchain;
use paws_docker::{
    DockerFactsInput, GithubContext as DockerGithubContext, native_publish_pipeline_args,
    resolve_docker_facts,
};
use paws_provision::{Ecosystem, Installer, provision_with_timing, real_installer};
use paws_release::{AssetUploadMode, GitHubReleaseClient, archive_name, package_zip};
use paws_semver::{GitHubGraphQlTagSource, SemverRequest, compute_new_version};

pub mod action_metadata;
pub mod mcp_setup;

/// Detects which of the ecosystems `paws-provision` knows about are needed in
/// the current directory, purely from marker files (mirrors `paws-audit`'s
/// signal-based detection, scoped to what `paws-provision` actually supports).
/// Ecosystems whose marker files sit directly in `dir`.
///
/// Takes the directory rather than assuming the process's own, so `--source`
/// provisions for the project actually being built rather than for whatever
/// happens to be at the repo root.
///
/// Walks `paws_core::TOOLCHAINS` rather than a marker table of its own. A
/// toolchain contributes here only if it has both a filename marker and an
/// installer, which is what keeps this to the ecosystems that can actually be
/// provisioned without also guessing that every `Cargo.toml` is an ESP32
/// firmware project.
fn detect_needed_ecosystems(dir: &std::path::Path) -> Vec<Ecosystem> {
    let mut found = Vec::new();
    for info in paws_core::TOOLCHAINS {
        let Some(ecosystem) = Ecosystem::for_toolchain(info.toolchain) else {
            continue;
        };
        if found.contains(&ecosystem) {
            continue;
        }
        if info.markers.iter().any(|marker| dir.join(marker).exists()) {
            found.push(ecosystem);
        }
    }
    found
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
        Commands::Changelog(args) => run_changelog(args).await,
        Commands::Cache(args) => run_cache(args).await,
        Commands::Mcp(McpCommand::Setup(args)) => mcp_setup::run_mcp_setup(args).await,
        Commands::Mcp(McpCommand::Serve(_)) => anyhow::bail!(
            "`paws mcp serve` must be invoked through the `paws` binary directly, not through \
             paws_cli::execute (see main.rs)"
        ),
    }
}

/// Runs `paws ci`, bracketing the whole invocation's Dagger pipeline work
/// with a single restore-before/save-after `CacheBackend` cycle (FR-006)
/// rather than wrapping every individual `paws_dagger::core`/
/// `core_streaming` call — a single `paws ci` run can call those many
/// times (e.g. fmt/clippy/build/test each being their own pipeline step),
/// and restoring/saving the shared Dagger engine container around each one
/// individually would repeatedly stop/restart it, which is both wasteful
/// and unsafe for adjacent calls within the same invocation.
pub async fn run_ci(args: CiArgs) -> anyhow::Result<()> {
    let backend = paws_dagger::restore_cache_backend().await;
    let result = run_ci_pipeline(args).await;
    paws_dagger::save_cache_backend(&backend).await;
    result
}

/// Resolve `--source` against the current directory, failing with a clear
/// message rather than an opaque "not found" from whatever runs next.
fn resolve_source_dir(source: Option<&str>) -> anyhow::Result<std::path::PathBuf> {
    let current = std::env::current_dir()?;
    let Some(source) = source else {
        return Ok(current);
    };

    let resolved = current.join(source);
    if !resolved.is_dir() {
        anyhow::bail!(
            "--source {source} is not a directory (looked in {})",
            current.display()
        );
    }

    // Canonicalize so the path handed to Dagger's host mount is absolute and
    // free of `..` segments.
    Ok(resolved.canonicalize()?)
}

async fn run_ci_pipeline(args: CiArgs) -> anyhow::Result<()> {
    let CiArgs {
        source,
        toolchain,
        toolchain_version,
        verbose,
        silent,
        targets,
        coverage,
        publish_artifacts,
    } = args;

    if !targets.is_empty() && toolchain != Some(Toolchain::Go) {
        anyhow::bail!("--targets is only valid with --toolchain go");
    }
    if coverage && toolchain != Some(Toolchain::Rust) {
        anyhow::bail!("--coverage is only valid with --toolchain rust");
    }
    if publish_artifacts && toolchain != Some(Toolchain::Esp32) {
        anyhow::bail!("--publish-artifacts is only valid with --toolchain esp32");
    }

    // Resolved once; every toolchain below builds from here rather than from
    // whatever directory the caller happened to be in.
    let source_dir = resolve_source_dir(source.as_deref())?;
    if source.is_some() {
        println!("ci: building {}", source_dir.display());
    }

    // FR-015: provisioning must go through the same concurrent path as
    // `paws provision`, never a sequential loop, whenever the target
    // repo needs more than one ecosystem.
    let needed = detect_needed_ecosystems(&source_dir);
    if needed.len() > 1 {
        run_provisioning(needed, verbose).await?;
    }

    // Resolve the toolchain version once, here, so every `ci_*` below builds
    // against the same answer and the log names where it came from.
    // `paws.toml` is discovered by walking up, so `--source web` in a monorepo
    // still sees the repo-root config.
    let (config, _) = paws_core::PawsConfig::discover(&source_dir)?;
    let version = toolchain.map(|toolchain| {
        toolchain.resolve_version(
            &source_dir,
            toolchain_version.as_deref(),
            config.toolchain_version(toolchain.as_str()),
        )
    });
    if let (Some(toolchain), Some(version)) = (toolchain, &version) {
        println!("ci: {toolchain} {}", version.describe());
    }
    let image = match (toolchain, &version) {
        (Some(toolchain), Some(version)) => toolchain.image_for(&version.version),
        _ => None,
    };

    paws_dagger::ensure_available().await?;
    match toolchain {
        Some(Toolchain::Node | Toolchain::Tauri) => {
            ci_node_or_tauri(&source_dir, silent, toolchain).await?;
        }
        Some(Toolchain::TauriAndroid) => ci_tauri_android(&source_dir, silent).await?,
        Some(Toolchain::Python) => ci_python(&source_dir, silent, image.as_deref()).await?,
        Some(Toolchain::Rust) => ci_rust(&source_dir, silent, coverage, image.as_deref()).await?,
        Some(Toolchain::Go) => ci_go(&source_dir, silent, &targets, image.as_deref()).await?,
        Some(Toolchain::Java) => ci_java(&source_dir, silent).await?,
        Some(Toolchain::Kotlin) => ci_kotlin(&source_dir, silent).await?,
        Some(Toolchain::Ruby) => ci_ruby(&source_dir, silent, image.as_deref()).await?,
        Some(Toolchain::Php) => ci_php(&source_dir, silent, image.as_deref()).await?,
        Some(Toolchain::Dotnet) => ci_dotnet(&source_dir, silent, image.as_deref()).await?,
        Some(Toolchain::Elixir) => ci_elixir(&source_dir, silent, image.as_deref()).await?,
        Some(Toolchain::Flatpak) => ci_flatpak(&source_dir, silent).await?,
        Some(Toolchain::Esp32) => ci_esp32(&source_dir, silent, publish_artifacts).await?,
        None => anyhow::bail!("--toolchain is required (e.g. --toolchain node)"),
    }
    Ok(())
}

/// `paws ci` for Node and Tauri desktop builds.
async fn ci_node_or_tauri(
    source_dir: &std::path::Path,
    silent: bool,
    toolchain: Option<Toolchain>,
) -> anyhow::Result<()> {
    let dir = source_dir.to_path_buf();
    let is_tauri = paws_tauri::is_tauri_project(&dir);
    if toolchain == Some(Toolchain::Tauri) && !is_tauri {
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
    Ok(())
}

/// `paws ci` for Tauri Android builds.
async fn ci_tauri_android(source_dir: &std::path::Path, silent: bool) -> anyhow::Result<()> {
    let dir = source_dir.to_path_buf();
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
    Ok(())
}

/// `paws ci` for uv-based Python projects.
async fn ci_python(
    source_dir: &std::path::Path,
    silent: bool,
    image: Option<&str>,
) -> anyhow::Result<()> {
    let dir = source_dir.to_path_buf();
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
    let args = image.map_or_else(
        || paws_python::dagger_pipeline_args(&project, &dir.to_string_lossy()),
        |image| {
            paws_python::dagger_pipeline_args_with_image(&project, &dir.to_string_lossy(), image)
        },
    );
    run_dagger_core(&args, silent).await?;
    println!("ci: python build/test succeeded");
    Ok(())
}

/// `paws ci` for Cargo projects, with optional llvm-cov coverage.
async fn ci_rust(
    source_dir: &std::path::Path,
    silent: bool,
    coverage: bool,
    image: Option<&str>,
) -> anyhow::Result<()> {
    let dir = source_dir.to_path_buf();
    if !paws_rust::is_rust_project(&dir) {
        anyhow::bail!(
            "--toolchain rust given, but no Cargo.toml found in {}",
            dir.display()
        );
    }
    let is_wasm = paws_rust::is_wasm_project(&dir);
    println!(
        "ci: rust project{}{} ({})",
        if is_wasm {
            " (wasm32-unknown-unknown)"
        } else {
            ""
        },
        if coverage && !is_wasm {
            " + coverage"
        } else {
            ""
        },
        dir.display()
    );
    let builder_dir = if coverage && !is_wasm {
        Some(
            paws_rust::write_builder_dockerfile()
                .context("failed to materialize the rust builder Dockerfile")?,
        )
    } else {
        None
    };
    let builder_dir_str = builder_dir.as_ref().map(|d| d.to_string_lossy());
    let args = paws_rust::dagger_pipeline_args_with_image(
        &dir.to_string_lossy(),
        is_wasm,
        coverage,
        builder_dir_str.as_deref(),
        image.unwrap_or(paws_rust::BASE_IMAGE),
    );
    run_dagger_core(&args, silent).await?;
    println!("ci: rust build/test succeeded");
    Ok(())
}

/// `paws ci` for Go projects, native or cross-compiled.
async fn ci_go(
    source_dir: &std::path::Path,
    silent: bool,
    targets: &[String],
    image: Option<&str>,
) -> anyhow::Result<()> {
    let dir = source_dir.to_path_buf();
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
        let args = paws_go::dagger_pipeline_args_with_image(
            &dir.to_string_lossy(),
            is_wasm,
            image.unwrap_or(paws_go::BASE_IMAGE),
        );
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
    Ok(())
}

/// `paws ci` for Maven and Gradle projects.
async fn ci_java(source_dir: &std::path::Path, silent: bool) -> anyhow::Result<()> {
    let dir = source_dir.to_path_buf();
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
    Ok(())
}

/// `paws ci` for Gradle-built Kotlin projects.
async fn ci_kotlin(source_dir: &std::path::Path, silent: bool) -> anyhow::Result<()> {
    let dir = source_dir.to_path_buf();
    paws_kotlin::detect_project(&dir)
        .context("failed to detect a Kotlin project in the current directory")?;
    println!("ci: kotlin project ({})", dir.display());
    let builder_dir = paws_kotlin::write_builder_dockerfile()
        .context("failed to materialize the java builder Dockerfile")?;
    let args =
        paws_kotlin::dagger_pipeline_args(&dir.to_string_lossy(), &builder_dir.to_string_lossy());
    run_dagger_core(&args, silent).await?;
    println!("ci: kotlin build/test succeeded");
    Ok(())
}

/// `paws ci` for Bundler projects.
async fn ci_ruby(
    source_dir: &std::path::Path,
    silent: bool,
    image: Option<&str>,
) -> anyhow::Result<()> {
    let dir = source_dir.to_path_buf();
    let project = paws_ruby::detect_project(&dir)
        .context("failed to detect a Ruby project in the current directory")?;
    println!(
        "ci: ruby project testing via {} ({})",
        project.test_runner.as_str(),
        dir.display()
    );
    let args = image.map_or_else(
        || paws_ruby::dagger_pipeline_args(&project, &dir.to_string_lossy()),
        |image| paws_ruby::dagger_pipeline_args_with_image(&project, &dir.to_string_lossy(), image),
    );
    run_dagger_core(&args, silent).await?;
    println!("ci: ruby install/test succeeded");
    Ok(())
}

/// `paws ci` for Composer projects.
async fn ci_php(
    source_dir: &std::path::Path,
    silent: bool,
    image: Option<&str>,
) -> anyhow::Result<()> {
    let dir = source_dir.to_path_buf();
    let project = paws_php::detect_project(&dir)
        .context("failed to detect a PHP project in the current directory")?;
    println!(
        "ci: php project ({}){}",
        dir.display(),
        if project.has_phpunit {
            ""
        } else {
            " — no phpunit config, skipping tests"
        }
    );
    let args = image.map_or_else(
        || paws_php::dagger_pipeline_args(&project, &dir.to_string_lossy()),
        |image| paws_php::dagger_pipeline_args_with_image(&project, &dir.to_string_lossy(), image),
    );
    run_dagger_core(&args, silent).await?;
    println!("ci: php install/test succeeded");
    Ok(())
}

/// `paws ci` for .NET SDK projects.
async fn ci_dotnet(
    source_dir: &std::path::Path,
    silent: bool,
    image: Option<&str>,
) -> anyhow::Result<()> {
    let dir = source_dir.to_path_buf();
    let project = paws_dotnet::detect_project(&dir)
        .context("failed to detect a .NET project in the current directory")?;
    println!(
        "ci: dotnet project ({}){}",
        dir.display(),
        if project.has_tests {
            ""
        } else {
            " — no test project, skipping dotnet test"
        }
    );
    let args = image.map_or_else(
        || paws_dotnet::dagger_pipeline_args(&project, &dir.to_string_lossy()),
        |image| {
            paws_dotnet::dagger_pipeline_args_with_image(&project, &dir.to_string_lossy(), image)
        },
    );
    run_dagger_core(&args, silent).await?;
    println!("ci: dotnet build/test succeeded");
    Ok(())
}

/// `paws ci` for Mix projects.
async fn ci_elixir(
    source_dir: &std::path::Path,
    silent: bool,
    image: Option<&str>,
) -> anyhow::Result<()> {
    let dir = source_dir.to_path_buf();
    let project = paws_elixir::detect_project(&dir)
        .context("failed to detect an Elixir project in the current directory")?;
    println!(
        "ci: elixir project ({}){}",
        dir.display(),
        if project.has_lockfile {
            ""
        } else {
            " — no mix.lock committed"
        }
    );
    let args = image.map_or_else(
        || paws_elixir::dagger_pipeline_args(&dir.to_string_lossy()),
        |image| paws_elixir::dagger_pipeline_args_with_image(&dir.to_string_lossy(), image),
    );
    run_dagger_core(&args, silent).await?;
    println!("ci: elixir build/test succeeded");
    Ok(())
}

/// `paws ci` for Flatpak manifests.
async fn ci_flatpak(source_dir: &std::path::Path, silent: bool) -> anyhow::Result<()> {
    let dir = source_dir.to_path_buf();
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
    Ok(())
}

/// `paws ci` for ESP-IDF firmware, with optional artifact publishing.
async fn ci_esp32(
    source_dir: &std::path::Path,
    silent: bool,
    publish_artifacts: bool,
) -> anyhow::Result<()> {
    let dir = source_dir.to_path_buf();
    if !paws_esp32::is_esp32_project(&dir) {
        anyhow::bail!(
            "--toolchain esp32 given, but no esp-idf-sys/esp-idf-svc dependency or \
                 *-espidf .cargo/config.toml target found in {}",
            dir.display()
        );
    }

    // ha-kiosk's own firmware/ crate is deliberately NOT a
    // workspace member of its own build (a heavy, differently-
    // toolchained embedded target pinned to its own
    // rust-toolchain.toml — see that repo's root Cargo.toml) but
    // does sit as a sibling directory next to a real workspace
    // (firmware-core/, flasher/) — so the search for a
    // host-testable sibling (Design Decision 3) starts one level
    // up from the ESP32 project itself, not inside it. Reuses
    // `paws_publish::find_workspace_root` (same as the `rust-crate`
    // publish path below) rather than a bare `dir.parent()` guess —
    // it actually verifies an ancestor declares `[workspace]`
    // instead of assuming the parent directory is one.
    let workspace_root = paws_publish::find_workspace_root(&dir);
    let host_test_dir = workspace_root
        .as_deref()
        .and_then(paws_esp32::find_host_testable_sibling);

    let (mount_dir, project_subpath, host_test_subpath) = match (&workspace_root, &host_test_dir) {
        (Some(root), Some(sibling)) => {
            // `strip_prefix`, not `.file_name()` — a workspace
            // member declared with a nested path (e.g.
            // `members = ["crates/firmware-core"]`) has to keep
            // its full path relative to `root`, or the
            // container's `with-workdir` points at a directory
            // that doesn't exist.
            let project_subpath = dir
                .strip_prefix(root)
                .unwrap_or(&dir)
                .to_string_lossy()
                .into_owned();
            let sibling_subpath = sibling
                .strip_prefix(root)
                .unwrap_or(sibling)
                .to_string_lossy()
                .into_owned();
            (root.clone(), project_subpath, Some(sibling_subpath))
        }
        _ => (dir.clone(), ".".to_string(), None),
    };

    println!(
        "ci: esp32 project{} ({})",
        if host_test_subpath.is_some() {
            " + host-testable sibling test"
        } else {
            ""
        },
        dir.display()
    );
    let builder_dir = paws_esp32::write_builder_dockerfile()
        .context("failed to materialize the esp32 builder Dockerfile")?;
    let args = paws_esp32::dagger_pipeline_args(
        &mount_dir.to_string_lossy(),
        &project_subpath,
        &builder_dir.to_string_lossy(),
        host_test_subpath.as_deref(),
    );
    run_dagger_core(&args, silent).await?;
    println!("ci: esp32 build/test succeeded");

    if publish_artifacts {
        let repository = std::env::var("GITHUB_REPOSITORY")
            .context("--publish-artifacts requires $GITHUB_REPOSITORY to be set")?;
        let (owner, repo) = repository.split_once('/').ok_or_else(|| {
            anyhow::anyhow!("$GITHUB_REPOSITORY must be \"owner/repo\", got {repository}")
        })?;
        let token = std::env::var("GITHUB_TOKEN")
            .or_else(|_| std::env::var("GH_TOKEN"))
            .context("--publish-artifacts requires $GITHUB_TOKEN or $GH_TOKEN to be set")?;
        let tag = std::env::var("GITHUB_REF_NAME").context(
            "--publish-artifacts requires $GITHUB_REF_NAME to be set (the tag to \
                 publish assets to)",
        )?;

        let triple = paws_esp32::target_triple(&dir)?;
        let binary_name = paws_esp32::binary_name(&dir)?;
        let release_dir = dir.join("target").join(&triple).join("release");

        // The build in `dagger_pipeline_args` above ran entirely
        // inside the ephemeral Dagger container — `cargo build
        // --release` never wrote anything to this host's
        // `release_dir`. Re-run the same build (Dagger's own
        // content-addressed caching makes the fmt/clippy/build
        // steps effectively free the second time) as a separate
        // `dagger core` chain whose terminal call actually exports
        // that directory back to the host (see
        // `dagger_export_pipeline_args`'s doc comment for why this
        // can't just be appended onto the pipeline above).
        tokio::fs::create_dir_all(&release_dir)
            .await
            .with_context(|| {
                format!(
                    "failed to create {} for the esp32 export",
                    release_dir.display()
                )
            })?;
        let export_args = paws_esp32::dagger_export_pipeline_args(
            &mount_dir.to_string_lossy(),
            &project_subpath,
            &builder_dir.to_string_lossy(),
            &triple,
            &release_dir.to_string_lossy(),
        );
        run_dagger_core(&export_args, silent).await?;

        let client = GitHubReleaseClient::new(owner.to_string(), repo.to_string(), token);
        let release_id = client.get_or_create_release(&tag, false).await?;
        paws_esp32::publish_artifacts(&client, release_id, &release_dir, &binary_name)
            .await
            .context("failed to publish esp32 build artifacts")?;
        println!("ci: esp32 artifacts published to the {tag} release");
    }
    Ok(())
}

/// Runs `paws docker`, bracketing the whole invocation with a single
/// restore-before/save-after `CacheBackend` cycle — see [`run_ci`]'s doc
/// comment for why this happens once per invocation rather than once per
/// `paws_dagger::core`/`core_streaming` call.
pub async fn run_docker(args: DockerArgs) -> anyhow::Result<()> {
    let backend = paws_dagger::restore_cache_backend().await;
    let result = run_docker_pipeline(args).await;
    paws_dagger::save_cache_backend(&backend).await;
    result
}

// One linear transaction: resolve facts from flags + compose + the GitHub
// environment, decide the tag matrix and whether to push, then build and
// publish each target. Every step reads the previous step's output, and the
// pure decision-making half already lives in `paws-docker`
// (`resolve_docker_facts`, `generate_tag_matrix`, `plan_publish`) — what is
// left here is the I/O sequence, which splitting would spread across private
// helpers callable in exactly one order.
#[allow(clippy::too_many_lines)]
async fn run_docker_pipeline(args: DockerArgs) -> anyhow::Result<()> {
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
        tag_rollup,
        tag_sha,
        tag_branch,
        tag_pr,
        tag_schedule,
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
            tag_rollup,
            tag_sha,
            tag_branch,
            tag_pr,
            tag_schedule,
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

    // Planning lives in paws-docker as a pure function so it can be
    // table-tested without a Dagger daemon or a registry — see
    // `plan_publish_targets`.
    let extra_usernames: Vec<(String, String)> = parse_registry_usernames(&registry_username)?
        .into_iter()
        .collect();

    let targets = paws_docker::plan_publish_targets(&paws_docker::PublishPlanInput {
        image: &image,
        tags: &facts.tags,
        registries: &registries,
        dockerhub_username: dockerhub_username.as_deref(),
        ghcr_username: ghcr_username.as_deref(),
        extra_usernames: &extra_usernames,
        ghcr_token_present: std::env::var("GHCR_TOKEN").is_ok(),
        github_token_present: std::env::var("GITHUB_TOKEN").is_ok(),
    });

    if facts.push {
        // Recorded per target so the run can close with a ledger, rather than
        // leaving per-target chatter as the only evidence of what happened.
        let mut outcomes: Vec<paws_docker::PublishOutcome> = Vec::new();

        for target in &targets {
            let paws_docker::PublishTarget {
                registry,
                tags,
                username,
                token_env_var,
                credentials_required,
                origin,
            } = target;
            if tags.is_empty() {
                outcomes.push(paws_docker::PublishOutcome::NoTags {
                    registry: registry.clone(),
                });
                continue;
            }
            let username = match username {
                Some(u) => u,
                None if *credentials_required => {
                    let flag = if *registry == "ghcr.io" {
                        "--ghcr-username"
                    } else {
                        "--registry-username"
                    };
                    anyhow::bail!(
                        "no username configured for {registry}, which {} asks to \
                         publish to — pass {flag} (or set the matching *_USERNAME env \
                         var), and set ${token_env_var}",
                        origin.flag()
                    );
                }
                None => {
                    println!(
                        "docker: no username configured for {registry}, skipping publish \
                         ({} tag(s))",
                        tags.len()
                    );
                    outcomes.push(paws_docker::PublishOutcome::Skipped {
                        registry: registry.clone(),
                        reason: paws_docker::SkipReason::NoUsername,
                    });
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
                outcomes.push(paws_docker::PublishOutcome::Skipped {
                    registry: registry.clone(),
                    reason: paws_docker::SkipReason::NoToken {
                        env_var: token_env_var.clone(),
                    },
                });
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

            outcomes.push(paws_docker::PublishOutcome::Published {
                registry: registry.clone(),
                tags: tags.clone(),
            });
        }

        // Always close with what actually happened. A run that publishes
        // nothing has repeatedly gone unnoticed because success was the only
        // signal it gave.
        println!("{}", paws_docker::publish_summary(&outcomes));

        // Downstream steps routinely need the tags that were actually
        // published — to alias one, scan it, or record it in a release.
        let published: Vec<String> = outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                paws_docker::PublishOutcome::Published { tags, .. } => Some(tags.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        paws_environment::write_outputs(&[
            ("image", &image),
            ("tags", &published.join("\n")),
            ("published-count", &published.len().to_string()),
        ])
        .context("writing $GITHUB_OUTPUT")?;

        // Asked to push, pushed nothing: that is a failure, not a quiet
        // success. Every silent under-publish this tool has had would have
        // surfaced here at the moment it was introduced.
        if let Some(error) = paws_docker::nothing_published_error(&outcomes) {
            anyhow::bail!(error);
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
            let (mount_dir, workdir) = paws_publish::find_workspace_root(&dir).map_or_else(
                || (dir.clone(), std::path::PathBuf::from("/src")),
                |root| {
                    let relative = dir.strip_prefix(&root).unwrap_or(&dir);
                    let workdir = std::path::Path::new("/src").join(relative);
                    (root, workdir)
                },
            );
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

    // So a workflow can use steps.<id>.outputs.version instead of scraping
    // stdout, which breaks the moment this prints one more line.
    paws_environment::write_outputs(&[("version", &version)]).context("writing $GITHUB_OUTPUT")?;

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

pub async fn run_changelog(args: ChangelogArgs) -> anyhow::Result<()> {
    let ChangelogArgs {
        version,
        previous_ref,
        prefix,
        output,
        commit,
        repository,
        branch,
    } = args;

    let (owner, repo, token) = if let Some(repository) = repository {
        let (owner, repo) = repository.split_once('/').ok_or_else(|| {
            anyhow::anyhow!("--repository must be \"owner/repo\", got {repository}")
        })?;
        let token = paws_environment::resolve_github_token(owner, repo).await?;
        (owner.to_string(), repo.to_string(), token)
    } else {
        let ctx = paws_environment::CiContext::detect()
            .await
            .context("paws changelog needs $GITHUB_REPOSITORY (or --repository)")?;
        (ctx.owner, ctx.repo, ctx.token)
    };

    let tag_source = GitHubGraphQlTagSource {
        owner: owner.clone(),
        repo: repo.clone(),
        token: token.clone(),
    };
    let previous_ref =
        paws_changelog::resolve_previous_ref(&tag_source, previous_ref, prefix).await?;

    let provider =
        paws_changelog::GitHubHistoryProvider::new(owner.clone(), repo.clone(), token.clone());
    let date = paws_changelog::today_iso_date();
    let entry =
        paws_changelog::build_entry(&provider, &version, &date, &previous_ref, &version).await?;

    let rendered = paws_changelog::append_to_file(std::path::Path::new(&output), &entry).await?;
    println!("{rendered}");

    if commit {
        let client = GitHubReleaseClient::new(owner, repo, token);
        paws_changelog::commit_back(&client, &output, &branch)
            .await
            .with_context(|| format!("failed to commit {output}@{branch}"))?;
        eprintln!("changelog: committed {output}@{branch}");
    }

    Ok(())
}

// `async` with nothing to await, deliberately: every `run_*` entry point
// shares one signature so `execute`'s dispatch and `paws-mcp`'s tool
// handlers can call them uniformly. Dropping it here would make this the
// one command both callers have to special-case.
#[allow(clippy::unused_async)]
pub async fn run_cache(args: CacheArgs) -> anyhow::Result<()> {
    let status = paws_cache::CacheStatus::detect();
    if args.json {
        println!("{}", status.to_json());
    } else {
        println!("{}", status.to_text());
    }
    Ok(())
}

pub async fn run_init(_args: InitArgs) -> anyhow::Result<()> {
    // `[tools] dagger = "..."` in paws.toml pins the engine, so two `paws
    // init` runs weeks apart leave the same version behind.
    let (config, _) = paws_core::PawsConfig::discover(&std::env::current_dir()?)?;
    let pinned = config.tool_version("dagger");
    let install_dir = paws_dagger::install_cli_with_version(pinned)
        .await
        .context("failed to install the dagger CLI")?;
    match pinned {
        Some(version) => println!(
            "dagger CLI {version} installed to {} (pinned by paws.toml)",
            install_dir.display()
        ),
        None => println!("dagger CLI installed to {}", install_dir.display()),
    }

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
            // SAFETY: `std::env::set_var` is unsafe in edition 2024 because a
            // concurrent read from another thread is UB. This is the only
            // env mutation in any `paws` subcommand, and it runs here before
            // `run_init` has spawned or awaited anything that reads the
            // environment — `install_cli` above has already returned, and
            // `ensure_available` below is what consumes the new PATH.
            //
            // Narrowly allowed rather than workspace-wide: the point of
            // `unsafe_code = "deny"` is that a second call site has to
            // justify itself here too.
            #[allow(unsafe_code)]
            unsafe {
                std::env::set_var("PATH", joined);
            };
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
    let mut scanners = select_audit_scanners(&detection, true);
    // `[tools]` in paws.toml repins any scanner image, so a repo can hold a
    // scanner back (or move it forward) without waiting on a paws release.
    let (config, _) = paws_core::PawsConfig::discover(&std::env::current_dir()?)?;
    paws_audit::apply_tool_versions(&mut scanners, &config.tools);
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
        // A scanner run measured in milliseconds cannot approach u64::MAX;
        // saturating says so rather than wrapping the way `as` would.
        let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

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

/// One requested `--provider` value's publish outcome — mirrors
/// `paws-provision::ProvisionOutcome`'s shape (data-model.md), collected
/// via the same `JoinSet` pattern so every provider's result (success or a
/// specific failure) is reported independently and none is ever hidden by
/// another's (FR-002a).
struct PublishOutcome {
    target: paws_docs::PublishTarget,
    result: anyhow::Result<()>,
    elapsed: std::time::Duration,
}

pub async fn run_docs(args: DocsArgs) -> anyhow::Result<()> {
    let DocsArgs {
        provider,
        repository,
        branch,
    } = args;

    let workspace = std::env::current_dir()?;
    let docs_dir = paws_docs::build_docs(&workspace).await?;
    println!("docs: built at {}", docs_dir.display());

    if provider.is_empty() {
        return Ok(());
    }

    // Parse every --provider value before any publish work starts (Edge
    // Cases) — one bad value must fail the whole command up front, not
    // after some providers already ran.
    let targets = provider
        .iter()
        .map(|value| value.parse::<paws_docs::PublishTarget>())
        .collect::<anyhow::Result<Vec<_>>>()?;

    let (owner, repo, token) = if let Some(repository) = repository {
        let (owner, repo) = repository.split_once('/').ok_or_else(|| {
            anyhow::anyhow!("--repository must be \"owner/repo\", got {repository}")
        })?;
        let token = paws_environment::resolve_github_token(owner, repo).await?;
        (owner.to_string(), repo.to_string(), token)
    } else {
        let ctx = paws_environment::CiContext::detect()
            .await
            .context("paws docs --provider needs $GITHUB_REPOSITORY (or --repository)")?;
        (ctx.owner, ctx.repo, ctx.token)
    };
    let client = std::sync::Arc::new(GitHubReleaseClient::new(owner, repo, token));
    let mut outcomes = dispatch_publish_targets(client, docs_dir, branch, targets).await;
    outcomes.sort_by_key(|o| o.target.as_str());

    let mut any_failed = false;
    for outcome in &outcomes {
        match &outcome.result {
            Ok(()) => println!(
                "docs: {} succeeded ({:.1}s)",
                outcome.target.as_str(),
                outcome.elapsed.as_secs_f64()
            ),
            Err(err) => {
                any_failed = true;
                eprintln!("docs: {} failed: {err}", outcome.target.as_str());
            }
        }
    }

    if any_failed {
        let failures: Vec<String> = outcomes
            .iter()
            .filter_map(|o| {
                o.result
                    .as_ref()
                    .err()
                    .map(|err| format!("{}: {err}", o.target.as_str()))
            })
            .collect();
        anyhow::bail!("docs: --provider failed for: {}", failures.join("; "));
    }
    Ok(())
}

/// Runs every named `--provider` target concurrently against the same
/// already-built `docs_dir`, mirroring `paws-provision::provision_with_timing`'s
/// exact `JoinSet` shape (research.md R8) — every outcome (success or a
/// specific failure) is collected, none short-circuits the others
/// (FR-002a). Extracted from [`run_docs`] so tests can drive it directly
/// against an already-constructed (and, in tests, fixture-backed) client
/// without needing real `$GITHUB_TOKEN`/network access just to reach this
/// part of the pipeline.
async fn dispatch_publish_targets(
    client: std::sync::Arc<GitHubReleaseClient>,
    docs_dir: std::path::PathBuf,
    branch: String,
    targets: Vec<paws_docs::PublishTarget>,
) -> Vec<PublishOutcome> {
    let docs_dir = std::sync::Arc::new(docs_dir);
    let mut set = tokio::task::JoinSet::new();
    let mut target_by_id = std::collections::HashMap::new();
    for target in targets {
        let client = client.clone();
        let docs_dir = docs_dir.clone();
        let branch = branch.clone();
        let started_at = std::time::Instant::now();
        let handle = set.spawn(async move {
            let result = match target {
                paws_docs::PublishTarget::GitHubPages => {
                    paws_docs::publish_github_pages(&client, &branch, &docs_dir).await
                }
                paws_docs::PublishTarget::CloudflarePages | paws_docs::PublishTarget::S3 => {
                    Err(paws_docs::not_implemented_error(target))
                }
            };
            (result, started_at.elapsed())
        });
        target_by_id.insert(handle.id(), target);
    }

    let mut outcomes = Vec::new();
    while let Some(joined) = set.join_next_with_id().await {
        match joined {
            Ok((id, (result, elapsed))) => {
                outcomes.push(PublishOutcome {
                    target: target_by_id[&id],
                    result,
                    elapsed,
                });
            }
            Err(join_err) => {
                let target = target_by_id[&join_err.id()];
                outcomes.push(PublishOutcome {
                    target,
                    result: Err(anyhow::anyhow!(join_err)),
                    elapsed: std::time::Duration::ZERO,
                });
            }
        }
    }
    outcomes
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

// Lint/package/publish for every discovered chart, in dependency order, then
// one `index.yaml` update covering all of them. The per-chart work cannot be
// lifted out without also lifting the accumulated index state it feeds.
#[allow(clippy::too_many_lines)]
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

// Cross-compile, smoke-test, package, and upload one target — four phases
// that each consume the previous phase's artifact path. `paws-release` owns
// the logic for all four; this is the sequencing.
#[allow(clippy::too_many_lines)]
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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct DetectedWorkflowInputs {
    /// One entry per toolchain that was detected, in `paws_core::TOOLCHAINS`
    /// order, holding the directories it was found in relative to the repo
    /// root ("." is the root itself).
    ///
    /// A list rather than a field per language: this used to name `rust`,
    /// `node` and `python` explicitly, which is why a repo full of Go or Ruby
    /// generated an empty workflow long after `paws ci` learned to build both.
    toolchains: Vec<(Toolchain, Vec<String>)>,
    docker: bool,
    helm: bool,
}

impl DetectedWorkflowInputs {
    const fn any(&self) -> bool {
        !self.toolchains.is_empty() || self.docker || self.helm
    }
}

/// How far below the repo root to look for projects. Deep enough for the
/// common `web/`, `apps/web/` and `packages/thing/` layouts without walking a
/// whole tree.
const PROJECT_SEARCH_DEPTH: usize = 3;

/// Directories that never hold a project worth its own CI job.
const SKIPPED_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "vendor",
    "coverage",
    ".venv",
    "venv",
    "__pycache__",
];

/// Directories at or under `root` holding `marker`, relative to `root`.
///
/// A directory holding the marker is not descended into. That is what keeps a
/// Cargo workspace one project rather than one per member, while still finding
/// `web/` and `e2e/` in a repo whose root has no package.json — the case where
/// flat root-only detection produced a workflow covering half the repo.
///
/// A directory containing its own `.git` is skipped: a nested checkout is a
/// different repository, not part of this one's CI. That covers vendored
/// reference clones and submodules.
///
/// Gitignored directories without their own `.git` are still found — there is
/// no gitignore parsing here. The generated workflow is a starting point, and
/// an extra step is easier to notice and delete than a missing one is to
/// notice at all, which is the failure this replaced.
fn discover_projects(root: &std::path::Path, marker: &str, max_depth: usize) -> Vec<String> {
    fn walk(
        dir: &std::path::Path,
        root: &std::path::Path,
        marker: &str,
        depth: usize,
        max_depth: usize,
        found: &mut Vec<String>,
    ) {
        if dir.join(marker).exists() {
            let relative = dir.strip_prefix(root).unwrap_or(dir);
            found.push(if relative.as_os_str().is_empty() {
                ".".to_string()
            } else {
                relative.to_string_lossy().replace('\\', "/")
            });
            // Found here, so anything nested belongs to this project.
            return;
        }

        if depth >= max_depth {
            return;
        }

        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut children: Vec<std::path::PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| !name.starts_with('.') && !SKIPPED_DIRS.contains(&name))
            })
            // A nested repository is not this repository.
            .filter(|path| !path.join(".git").exists())
            .collect();
        // Stable output regardless of filesystem ordering.
        children.sort();

        for child in children {
            walk(&child, root, marker, depth + 1, max_depth, found);
        }
    }

    let mut found = Vec::new();
    walk(root, root, marker, 0, max_depth, &mut found);
    found
}

/// Every toolchain `paws workflow generate` can recognize from marker files
/// alone, named for a message.
fn workflow_detectable_toolchains() -> String {
    let names: Vec<&str> = paws_core::TOOLCHAINS
        .iter()
        .filter(|info| !info.markers.is_empty())
        .map(|info| info.name)
        .collect();
    names.join("/")
}

/// Finds every marker-detectable toolchain under `dir`, in registry order.
///
/// A toolchain is reported once with all the directories it was found in, so
/// a monorepo with `web/` and `e2e/` gets two Node steps rather than two Node
/// entries. Toolchains whose detection needs real logic have no markers and
/// are skipped — see `ToolchainInfo::markers`.
fn detect_workflow_toolchains(dir: &std::path::Path) -> Vec<(Toolchain, Vec<String>)> {
    let mut detected = Vec::new();
    for info in paws_core::TOOLCHAINS {
        if info.markers.is_empty() {
            continue;
        }
        let mut dirs = Vec::new();
        for marker in info.markers {
            for found in discover_projects(dir, marker, PROJECT_SEARCH_DEPTH) {
                // A Gradle project has both `build.gradle` and
                // `build.gradle.kts` in some layouts, and a Maven project
                // beside it shares the same toolchain — one step per
                // directory, not one per marker that matched it.
                if !dirs.contains(&found) {
                    dirs.push(found);
                }
            }
        }
        if !dirs.is_empty() {
            detected.push((info.toolchain, dirs));
        }
    }
    detected
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

    // One step per project. A project outside the repo root gets --source, so
    // a monorepo does not need a working-directory on every step.
    for (toolchain, dirs) in &detected.toolchains {
        for dir in dirs {
            let _ = if dir == "." {
                writeln!(out, "      - run: paws ci --toolchain {toolchain}")
            } else {
                writeln!(
                    out,
                    "      - run: paws ci --toolchain {toolchain} --source {dir}"
                )
            };
        }
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
        // Discovered rather than read from the flat root-only signal map, so
        // a project in a subdirectory gets its own step. Every toolchain with
        // a filename marker participates; the ones without one (esp32,
        // flatpak, kotlin, dotnet, tauri) need their own crate's detection
        // logic and are left to an explicit `--toolchain`.
        toolchains: detect_workflow_toolchains(&dir),
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
            "workflow: no recognizable project markers found here (checked {}, Docker and \
             Helm); nothing to generate.",
            workflow_detectable_toolchains()
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

    let mut kinds: Vec<String> = Vec::new();
    // Name the directories, not just the ecosystems: in a monorepo "node" alone
    // does not tell you which projects were picked up.
    for (toolchain, dirs) in &detected.toolchains {
        for dir in dirs {
            kinds.push(if dir == "." {
                toolchain.to_string()
            } else {
                format!("{toolchain}:{dir}")
            });
        }
    }
    if detected.docker {
        kinds.push("docker".to_string());
    }
    if detected.helm {
        kinds.push("helm".to_string());
    }
    println!("workflow: generated {output} ({})", kinds.join(", "));

    Ok(())
}

/// Renders one `clap::Command` (and its subcommands) as an `llms.txt`
/// section. Module-level rather than nested inside `render_llms_txt`: it
/// recurses, and an item declared after statements reads as if it were
/// scoped to the code above it.
fn render_command(cmd: &clap::Command, prefix: &str, out: &mut String) {
    let name = format!("{prefix}{}", cmd.get_name());
    let _ = write!(out, "## paws {name}\n\n");
    if let Some(about) = cmd.get_about() {
        let _ = write!(out, "{about}\n\n");
    }

    let flags: Vec<_> = cmd
        .get_arguments()
        .filter(|a| a.get_long().is_some())
        .collect();
    if !flags.is_empty() {
        for flag in flags {
            let long = flag.get_long().unwrap_or_default();
            let help = flag.get_help().map(ToString::to_string).unwrap_or_default();
            let default = flag
                .get_default_values()
                .first()
                .map(|v| v.to_string_lossy().to_string());
            match default {
                Some(default) if !default.is_empty() => {
                    let _ = writeln!(out, "- `--{long}` (default: `{default}`) — {help}");
                }
                _ => {
                    let _ = writeln!(out, "- `--{long}` — {help}");
                }
            }
        }
        out.push('\n');
    }

    for sub in cmd.get_subcommands() {
        render_command(sub, &format!("{name} "), out);
    }
}

/// Renders `llms.txt` (the <https://llmstxt.org> convention) purely from
/// this CLI's own `clap::Command` metadata — the exact same source that
/// drives `--help`, so it can never drift from real CLI behavior.
pub fn render_llms_txt() -> String {
    use clap::CommandFactory;

    let root = Cli::command();
    let mut out = String::new();

    out.push_str("# paws\n\n");
    let about = root
        .get_about()
        .map(ToString::to_string)
        .unwrap_or_default();
    if !about.is_empty() {
        let _ = write!(out, "> {about}\n\n");
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

    for sub in root.get_subcommands() {
        render_command(sub, "", &mut out);
    }

    if let Ok(actions) = action_metadata::discover_actions()
        && !actions.is_empty()
    {
        out.push_str("## GitHub Actions\n\n");
        out.push_str(
            "paws also ships composite GitHub Actions for wiring into a *consumer* repo's own \
             CI/CD, separate from the CLI subcommands above — `paws workflow generate` scaffolds \
             a starter workflow using these automatically.\n\n",
        );
        for action in &actions {
            let _ = write!(out, "### {}\n\n", action.id);
            if !action.description.is_empty() {
                let _ = write!(out, "{}\n\n", action.description);
            }

            out.push_str("```yaml\n");
            let _ = writeln!(out, "- uses: {}", action.usage);
            if !action.inputs.is_empty() {
                out.push_str("  with:\n");
                for input in &action.inputs {
                    let value = input.default.clone().unwrap_or_else(|| "...".to_string());
                    let _ = writeln!(out, "    {}: {value}", input.name);
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
                    let _ = writeln!(
                        out,
                        "- `{}` ({requiredness}{default}) — {}",
                        input.name, input.description
                    );
                }
                out.push('\n');
            }
            if !action.outputs.is_empty() {
                out.push_str("**Outputs**\n\n");
                for output in &action.outputs {
                    let _ = writeln!(out, "- `{}` — {}", output.name, output.description);
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
    existing.is_none_or(|existing| existing != generated)
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
// `std::env::set_var`/`remove_var` are unsafe in edition 2024, and these
// tests exist precisely to exercise env-var-driven behavior. Access is
// serialized within this module, which is what makes it sound.
#[allow(unsafe_code)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    // T009: `--coverage` outside `--toolchain rust` fails fast, before any
    // Dagger/Docker interaction — the gating check runs before
    // `paws_dagger::ensure_available()`, so this needs no real toolchain
    // present to test, mirroring `--targets`'s existing out-of-`--toolchain
    // go` rejection shape.
    #[tokio::test]
    async fn coverage_is_rejected_outside_toolchain_rust() {
        let args = CiArgs {
            source: None,
            toolchain_version: None,
            toolchain: Some(Toolchain::Node),
            verbose: false,
            silent: true,
            targets: vec![],
            coverage: true,
            publish_artifacts: false,
        };
        let err = run_ci(args).await.unwrap_err();
        assert!(
            err.to_string()
                .contains("--coverage is only valid with --toolchain rust"),
            "unexpected error message: {err}"
        );
    }

    #[tokio::test]
    async fn coverage_with_no_toolchain_at_all_is_also_rejected() {
        let args = CiArgs {
            source: None,
            toolchain_version: None,
            toolchain: None,
            verbose: false,
            silent: true,
            targets: vec![],
            coverage: true,
            publish_artifacts: false,
        };
        let err = run_ci(args).await.unwrap_err();
        assert!(
            err.to_string()
                .contains("--coverage is only valid with --toolchain rust"),
            "unexpected error message: {err}"
        );
    }

    // Workstream 5: --publish-artifacts is rejected outside --toolchain
    // esp32, before any Dagger/Docker/GitHub interaction — same gating
    // shape as --coverage's existing --toolchain rust-only rejection above.
    #[tokio::test]
    async fn publish_artifacts_is_rejected_outside_toolchain_esp32() {
        let args = CiArgs {
            source: None,
            toolchain_version: None,
            toolchain: Some(Toolchain::Rust),
            verbose: false,
            silent: true,
            targets: vec![],
            coverage: false,
            publish_artifacts: true,
        };
        let err = run_ci(args).await.unwrap_err();
        assert!(
            err.to_string()
                .contains("--publish-artifacts is only valid with --toolchain esp32"),
            "unexpected error message: {err}"
        );
    }

    #[tokio::test]
    async fn publish_artifacts_with_no_toolchain_at_all_is_also_rejected() {
        let args = CiArgs {
            source: None,
            toolchain_version: None,
            toolchain: None,
            verbose: false,
            silent: true,
            targets: vec![],
            coverage: false,
            publish_artifacts: true,
        };
        let err = run_ci(args).await.unwrap_err();
        assert!(
            err.to_string()
                .contains("--publish-artifacts is only valid with --toolchain esp32"),
            "unexpected error message: {err}"
        );
    }

    // T041: omitting --provider entirely reproduces today's exact
    // local-build-only behavior (FR-002) — no credentials, no network.
    #[tokio::test]
    async fn omitting_provider_only_builds_docs_locally() {
        let args = DocsArgs {
            provider: vec![],
            repository: None,
            branch: field_defaults::main_branch(),
        };
        run_docs(args).await.unwrap();
    }

    // T043: an unrecognized --provider value fails distinctly from
    // FR-004a's "not implemented yet" error, before any build/publish
    // work is attempted.
    #[tokio::test]
    async fn unrecognized_provider_value_fails_distinctly_from_not_implemented() {
        let args = DocsArgs {
            provider: vec!["azure-static-web-apps".to_string()],
            repository: Some("octo/repo".to_string()),
            branch: field_defaults::main_branch(),
        };
        let err = run_docs(args).await.unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("invalid --provider value"),
            "unexpected error: {message}"
        );
        assert!(!message.contains("not implemented yet"));
    }

    // T042: cloudflare-pages/s3 each fail immediately with the fixed
    // FR-004a error, naming the specific provider, with zero network calls
    // (dispatch_publish_targets is exercised directly, bypassing
    // credential resolution entirely — these two targets never touch the
    // client at all).
    #[tokio::test]
    async fn cloudflare_pages_and_s3_fail_with_the_not_implemented_error() {
        let client = std::sync::Arc::new(GitHubReleaseClient::new(
            "octo".into(),
            "repo".into(),
            "t".into(),
        ));
        let outcomes = dispatch_publish_targets(
            client,
            std::path::PathBuf::from("/nonexistent"),
            "main".to_string(),
            vec![
                paws_docs::PublishTarget::CloudflarePages,
                paws_docs::PublishTarget::S3,
            ],
        )
        .await;
        assert_eq!(outcomes.len(), 2);
        for outcome in &outcomes {
            let err = outcome.result.as_ref().unwrap_err().to_string();
            assert!(
                err.contains("not implemented yet"),
                "unexpected error: {err}"
            );
            assert!(
                err.contains(outcome.target.as_str()),
                "unexpected error: {err}"
            );
        }
    }

    /// Serves one canned `(status, body)` response per entry, in order, on
    /// freshly-accepted connections — matches `paws-release`'s/`paws-docs`'s
    /// own fixture-server test helpers.
    async fn serve_fixture_responses(
        listener: tokio::net::TcpListener,
        responses: Vec<(u16, serde_json::Value)>,
    ) -> Vec<String> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut requests = Vec::new();
        for (status, body) in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let n = socket.read(&mut buf).await.unwrap();
            requests.push(String::from_utf8_lossy(&buf[..n]).into_owned());

            let payload = body.to_string();
            let status_line = match status {
                200 => "200 OK",
                404 => "404 Not Found",
                other => panic!("unsupported fixture status {other}"),
            };
            let response = format!(
                "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                payload.len(),
                payload
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.ok();
        }
        requests
    }

    // T044/T045: `github-pages` (against a fixture standing in for a real,
    // sufficiently-privileged token) succeeds while `s3` fails with the
    // FR-004a error in the same run — both outcomes are reported
    // independently (dispatch_publish_targets never short-circuits), and
    // the caller (run_docs, exercised separately above) would exit
    // non-zero without suppressing either.
    #[tokio::test]
    async fn github_pages_succeeds_and_s3_fails_independently_in_the_same_run() {
        let dir =
            std::env::temp_dir().join(format!("paws-cli-core-docs-fixture-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.html"), "<html>hi</html>").unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        // publish_github_pages's exact sequence for one file + the
        // manifest: get_pages_config (404 -> Git Trees) -> get_content
        // (manifest, 404 -> not up to date) -> create_blob x2 (file +
        // manifest) -> publish_tree's 5-request sequence.
        let responses = vec![
            (404, serde_json::Value::Null),
            (404, serde_json::Value::Null),
            (200, serde_json::json!({ "sha": "blob-index" })),
            (200, serde_json::json!({ "sha": "blob-manifest" })),
            (
                200,
                serde_json::json!({ "object": { "sha": "parent-commit-sha" } }),
            ),
            (
                200,
                serde_json::json!({ "tree": { "sha": "base-tree-sha" } }),
            ),
            (200, serde_json::json!({ "sha": "new-tree-sha" })),
            (200, serde_json::json!({ "sha": "new-commit-sha" })),
            (200, serde_json::json!({ "ref": "refs/heads/main" })),
        ];
        let server = tokio::spawn(serve_fixture_responses(listener, responses));

        let client = std::sync::Arc::new(
            GitHubReleaseClient::new("octo".into(), "repo".into(), "t".into())
                .with_base_url_for_tests(format!("http://{addr}")),
        );
        let outcomes = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            dispatch_publish_targets(
                client,
                dir.clone(),
                "main".to_string(),
                vec![
                    paws_docs::PublishTarget::GitHubPages,
                    paws_docs::PublishTarget::S3,
                ],
            ),
        )
        .await
        .expect("dispatch_publish_targets should not hang");

        assert_eq!(outcomes.len(), 2);
        let github_pages = outcomes
            .iter()
            .find(|o| o.target == paws_docs::PublishTarget::GitHubPages)
            .unwrap();
        assert!(
            github_pages.result.is_ok(),
            "{:?}",
            github_pages.result.as_ref().err()
        );
        let s3 = outcomes
            .iter()
            .find(|o| o.target == paws_docs::PublishTarget::S3)
            .unwrap();
        assert!(s3.result.is_err());
        assert!(
            s3.result
                .as_ref()
                .unwrap_err()
                .to_string()
                .contains("not implemented yet")
        );

        server.await.unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn llms_txt_covers_every_subcommand() {
        let rendered = render_llms_txt();
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
        let rendered = render_llms_txt();
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
        let rendered = render_llms_txt();
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
            toolchains: vec![(Toolchain::Rust, vec![".".to_string()])],
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

    /// The monorepo case: a project outside the root gets --source, so the
    /// generated workflow needs no working-directory on any step.
    #[test]
    fn workflow_render_points_subdirectory_projects_at_their_source() {
        let detected = DetectedWorkflowInputs {
            toolchains: vec![
                (Toolchain::Rust, vec![".".to_string()]),
                (Toolchain::Node, vec!["web".to_string(), "e2e".to_string()]),
            ],
            ..Default::default()
        };
        let rendered = render_github_workflow(&detected).expect("something was detected");

        // The root project takes no --source.
        assert!(rendered.contains("- run: paws ci --toolchain rust\n"));
        assert!(!rendered.contains("--toolchain rust --source"));

        // One step per node project, each pointed at its own directory.
        assert!(rendered.contains("paws ci --toolchain node --source web"));
        assert!(rendered.contains("paws ci --toolchain node --source e2e"));
        assert_eq!(rendered.matches("--toolchain node").count(), 2);
    }

    /// The README advertised five toolchains while `paws ci` dispatched
    /// fourteen — the front page undersold the tool for months because
    /// nothing tied the two together. `docs/ROADMAP.md` is deliberately not
    /// checked here: it is a narrative document about what is *verified*
    /// where, not a mirror of the accepted values.
    #[test]
    fn the_readme_names_every_toolchain_paws_ci_accepts() {
        let readme = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../README.md"),
        )
        .expect("README.md sits at the repo root");

        let missing: Vec<&str> = paws_core::TOOLCHAINS
            .iter()
            .map(|info| info.name)
            .filter(|name| !readme.contains(*name))
            .collect();

        assert!(
            missing.is_empty(),
            "README.md does not mention {missing:?} — `paws ci` accepts them, so the \
             \"Language / stack support\" section needs updating"
        );
    }

    /// The gap this registry closed: `paws ci` grew from 3 toolchains to 14
    /// while `paws workflow generate` still only knew Rust/Node/Python, so a
    /// Go or Ruby repo generated a workflow that built nothing. Anything with
    /// a filename marker must now produce a step.
    #[test]
    fn workflow_generation_covers_every_marker_detectable_toolchain() {
        let dir = project_scratch("every-toolchain");
        let mut expected = Vec::new();
        for info in paws_core::TOOLCHAINS {
            let Some(marker) = info.markers.first() else {
                continue;
            };
            // Each in its own directory: several toolchains would otherwise
            // be detected at the root and the assertion could not tell which
            // marker produced which step.
            let project = dir.join(info.name);
            std::fs::create_dir_all(&project).unwrap();
            std::fs::write(project.join(marker), "").unwrap();
            expected.push(info.toolchain);
        }

        let detected = detect_workflow_toolchains(&dir);
        let found: Vec<Toolchain> = detected.iter().map(|(t, _)| *t).collect();
        assert_eq!(
            found, expected,
            "every marker-detectable toolchain is found"
        );

        let rendered = render_github_workflow(&DetectedWorkflowInputs {
            toolchains: detected,
            ..Default::default()
        })
        .expect("markers were planted");
        for toolchain in expected {
            assert!(
                rendered.contains(&format!(
                    "paws ci --toolchain {toolchain} --source {toolchain}"
                )),
                "generated workflow should build the {toolchain} project, got:\n{rendered}"
            );
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Toolchains whose detection needs their own crate's logic must stay out
    /// of marker-based generation — `esp32` shares `Cargo.toml` with `rust`
    /// and `tauri` shares `package.json` with `node`, so guessing either from
    /// a filename would emit two steps for one project.
    #[test]
    fn specialization_toolchains_are_not_guessed_from_markers() {
        for toolchain in [
            Toolchain::Esp32,
            Toolchain::Tauri,
            Toolchain::TauriAndroid,
            Toolchain::Flatpak,
            Toolchain::Kotlin,
            Toolchain::Dotnet,
        ] {
            assert!(
                toolchain.markers().is_empty(),
                "{toolchain} must not be marker-detectable"
            );
        }
    }

    // --- project discovery -------------------------------------------------

    fn project_scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("paws-discover-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn touch(dir: &std::path::Path, relative: &str) {
        let path = dir.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }

    /// A Cargo workspace is one project, not one per member.
    #[test]
    fn discovery_does_not_descend_into_a_directory_that_already_matched() {
        let root = project_scratch("workspace");
        touch(&root, "Cargo.toml");
        touch(&root, "crates/core/Cargo.toml");
        touch(&root, "crates/server/Cargo.toml");

        assert_eq!(discover_projects(&root, "Cargo.toml", 3), vec!["."]);
        std::fs::remove_dir_all(&root).ok();
    }

    /// The bug this replaced: root-only detection saw no package.json and
    /// generated a workflow covering half the repo.
    #[test]
    fn discovery_finds_projects_in_subdirectories() {
        let root = project_scratch("monorepo");
        touch(&root, "Cargo.toml");
        touch(&root, "web/package.json");
        touch(&root, "e2e/package.json");

        // Sorted, so the generated workflow is stable across filesystems.
        assert_eq!(
            discover_projects(&root, "package.json", 3),
            vec!["e2e", "web"]
        );
        assert_eq!(discover_projects(&root, "Cargo.toml", 3), vec!["."]);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn discovery_skips_build_output_and_hidden_directories() {
        let root = project_scratch("noise");
        touch(&root, "web/package.json");
        touch(&root, "node_modules/some-dep/package.json");
        touch(&root, "web/node_modules/other/package.json");
        touch(&root, "dist/package.json");
        touch(&root, ".cache/package.json");

        assert_eq!(discover_projects(&root, "package.json", 3), vec!["web"]);
        std::fs::remove_dir_all(&root).ok();
    }

    /// A vendored reference clone or submodule is a different repository.
    #[test]
    fn discovery_skips_nested_checkouts() {
        let root = project_scratch("nested-git");
        touch(&root, "web/package.json");
        touch(&root, "upstream/package.json");
        std::fs::create_dir_all(root.join("upstream/.git")).unwrap();

        assert_eq!(discover_projects(&root, "package.json", 3), vec!["web"]);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn discovery_respects_the_depth_limit() {
        let root = project_scratch("deep");
        touch(&root, "a/b/c/d/package.json");

        assert!(discover_projects(&root, "package.json", 3).is_empty());
        assert_eq!(discover_projects(&root, "package.json", 4), vec!["a/b/c/d"]);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn discovery_finds_nothing_in_an_empty_tree() {
        let root = project_scratch("bare");
        assert!(discover_projects(&root, "package.json", 3).is_empty());
        std::fs::remove_dir_all(&root).ok();
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

    // --- --source resolution and ecosystem detection ----------------------

    /// Same idiom the other crates use for scratch dirs — no extra dependency.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("paws-ci-source-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    #[test]
    fn no_source_flag_means_the_current_directory() {
        let resolved = resolve_source_dir(None).expect("should resolve");
        assert_eq!(resolved, std::env::current_dir().unwrap());
    }

    #[test]
    fn a_source_that_is_not_a_directory_fails_with_the_path_it_looked_in() {
        let error = resolve_source_dir(Some("definitely-not-here"))
            .unwrap_err()
            .to_string();

        assert!(error.contains("definitely-not-here"), "got {error}");
        // The message must say where it looked; "not a directory" alone sends
        // you hunting.
        assert!(error.contains("looked in"), "got {error}");
    }

    #[test]
    fn a_file_is_not_a_valid_source() {
        let dir = scratch("file-not-dir");
        std::fs::write(dir.join("Cargo.toml"), "").unwrap();

        let previous = std::env::current_dir().unwrap();
        std::env::set_current_dir(&dir).unwrap();
        let result = resolve_source_dir(Some("Cargo.toml"));
        std::env::set_current_dir(previous).unwrap();

        assert!(
            result.is_err(),
            "a file must not resolve as a source directory"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ecosystems_are_detected_in_the_directory_given_not_the_process_one() {
        let root = scratch("detect");
        std::fs::write(root.join("Cargo.toml"), "").unwrap();
        let web = root.join("web");
        std::fs::create_dir_all(&web).unwrap();
        std::fs::write(web.join("package.json"), "{}").unwrap();

        // This is the monorepo case: --source web must provision for node,
        // not for the rust project at the repo root.
        assert_eq!(detect_needed_ecosystems(&root), vec![Ecosystem::Rust]);
        assert_eq!(detect_needed_ecosystems(&web), vec![Ecosystem::Node]);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_polyglot_directory_detects_every_ecosystem_present() {
        let dir = scratch("polyglot");
        for marker in ["Cargo.toml", "package.json", "pyproject.toml", "go.mod"] {
            std::fs::write(dir.join(marker), "").unwrap();
        }

        // Order follows `paws_core::TOOLCHAINS`, which is also the order
        // `--toolchain`'s help lists them in. It carries no meaning for
        // provisioning itself — `provision_with_timing` runs every ecosystem
        // as an independent task with no ordering between them (FR-013).
        assert_eq!(
            detect_needed_ecosystems(&dir),
            vec![
                Ecosystem::Node,
                Ecosystem::Rust,
                Ecosystem::Python,
                Ecosystem::Go
            ]
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_empty_directory_needs_no_provisioning() {
        let dir = scratch("empty");
        assert!(detect_needed_ecosystems(&dir).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
