//! Concurrent toolchain provisioning. Each requested ecosystem (rust, node,
//! python, ...) runs as an independent `tokio` task with no implicit
//! ordering dependency on any other — installing Rust never waits on pnpm,
//! and vice versa. See specs/001-paws-core-cli/spec.md's Concurrency Model
//! (FR-013..FR-016) for the contract this crate exists to satisfy.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::{Context, Result};
use paws_core::Toolchain;
use tokio::process::Command;
use tokio::task::JoinSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ecosystem {
    Rust,
    Node,
    Python,
    Go,
    Esp32,
}

impl Ecosystem {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Node => "node",
            Self::Python => "python",
            Self::Go => "go",
            Self::Esp32 => "esp32",
        }
    }

    /// The ecosystem that installs `toolchain`, or `None` when no installer
    /// exists for it — a JDK or a Ruby, say, which `paws` expects to already
    /// be on the runner.
    ///
    /// Reads `paws_core`'s toolchain registry rather than keeping a second
    /// opinion about which toolchains are provisionable, so the two can only
    /// disagree by failing `every_provisioned_toolchain_names_a_real_ecosystem`.
    pub fn for_toolchain(toolchain: Toolchain) -> Option<Self> {
        toolchain.provisions().and_then(|name| name.parse().ok())
    }
}

impl std::str::FromStr for Ecosystem {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "rust" => Ok(Self::Rust),
            "node" => Ok(Self::Node),
            "python" => Ok(Self::Python),
            "go" => Ok(Self::Go),
            "esp32" => Ok(Self::Esp32),
            other => anyhow::bail!("unsupported ecosystem: {other}"),
        }
    }
}

type InstallFuture = Pin<Box<dyn Future<Output = Result<()>> + Send>>;

/// A single ecosystem's setup step. Still shells out to the real installer
/// (rustup, a Node/pnpm installer, `uv`) — this crate orchestrates, it does
/// not reimplement what those tools already do well.
pub trait Installer: Send + Sync {
    fn install(&self) -> InstallFuture;
}

impl<F, Fut> Installer for F
where
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    fn install(&self) -> InstallFuture {
        Box::pin(self())
    }
}

/// One ecosystem's provisioning outcome, with enough timing data to verify
/// SC-005 (wall-clock stays near the slowest single install, not the sum).
#[derive(Debug)]
pub struct ProvisionOutcome {
    pub result: Result<()>,
    pub started_at: std::time::Instant,
    pub elapsed: Duration,
}

/// Runs every requested ecosystem's installer concurrently and returns every
/// outcome — success or failure — with no early return on first failure
/// (FR-014). Genuinely independent work only; a step with a real ordering
/// dependency on another does not belong here (FR-016).
pub async fn provision(
    tasks: Vec<(Ecosystem, Box<dyn Installer>)>,
) -> HashMap<Ecosystem, Result<()>> {
    provision_with_timing(tasks)
        .await
        .into_iter()
        .map(|(ecosystem, outcome)| (ecosystem, outcome.result))
        .collect()
}

/// Same as [`provision`], but keeps per-ecosystem start time and elapsed
/// duration for `--verbose` reporting (spec.md User Story 5's acceptance
/// scenario 1: concurrency must be verifiable via timestamps).
pub async fn provision_with_timing(
    tasks: Vec<(Ecosystem, Box<dyn Installer>)>,
) -> HashMap<Ecosystem, ProvisionOutcome> {
    let mut set = JoinSet::new();
    let mut ecosystem_by_id = HashMap::new();
    let overall_start = std::time::Instant::now();

    for (ecosystem, installer) in tasks {
        let started_at = std::time::Instant::now();
        let handle = set.spawn(async move {
            let result = installer.install().await;
            (started_at, result)
        });
        ecosystem_by_id.insert(handle.id(), ecosystem);
    }

    let mut results = HashMap::new();
    while let Some(joined) = set.join_next_with_id().await {
        match joined {
            Ok((id, (started_at, result))) => {
                let ecosystem = ecosystem_by_id[&id];
                results.insert(
                    ecosystem,
                    ProvisionOutcome {
                        result,
                        started_at,
                        elapsed: started_at.elapsed(),
                    },
                );
            }
            Err(join_err) => {
                // A panicking task must still surface as a reported failure,
                // never silently vanish from the aggregate (FR-014) — and it
                // must be attributed to the ecosystem that actually panicked,
                // not a hardcoded placeholder.
                let ecosystem = ecosystem_by_id[&join_err.id()];
                results.insert(
                    ecosystem,
                    ProvisionOutcome {
                        result: Err(anyhow::anyhow!(join_err)),
                        started_at: overall_start,
                        elapsed: overall_start.elapsed(),
                    },
                );
            }
        }
    }
    results
}

async fn run_command(program: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(program)
        .args(args)
        .output()
        .await
        .with_context(|| format!("failed to spawn `{program}` — is it installed and on PATH?"))?;

    if !output.status.success() {
        anyhow::bail!(
            "`{program} {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

/// Real, idempotent installer for the Rust toolchain via `rustup`. A no-op
/// (fast, successful) if the toolchain is already installed.
pub async fn install_rust() -> Result<()> {
    run_command(
        "rustup",
        &["toolchain", "install", "stable", "--no-self-update"],
    )
    .await
}

/// Real, idempotent installer for the Node/pnpm toolchain via `corepack`
/// (ported intent of `setupNode`: ensure pnpm is present and activated, not
/// reinstall Node itself — see this crate's module doc on scope).
pub async fn install_node() -> Result<()> {
    run_command("corepack", &["enable"]).await?;
    run_command("corepack", &["prepare", "pnpm@latest", "--activate"]).await
}

/// Real, idempotent installer for a Python toolchain via `uv`.
pub async fn install_python() -> Result<()> {
    run_command("uv", &["python", "install"]).await
}

/// Pinned Go toolchain version `install_go` ensures is present, mirroring
/// `gh-reusable`'s own `DEFAULT_GO_VERSION` ("1.24" there, used as a
/// floating Docker image tag) with a concrete patch release here — Go's
/// `golang.org/dl` mechanism (see `install_go`) has no floating "latest"
/// alias the way `rustup`'s "stable" or `corepack prepare pnpm@latest` do,
/// so a real release has to be named. Overridable via `$PAWS_GO_VERSION`.
pub const DEFAULT_GO_VERSION: &str = "1.23.4";

/// Real, idempotent installer for a Go toolchain via `go install
/// golang.org/dl/goX.Y.Z@latest` followed by that version's own `download`
/// subcommand — Go's own official mechanism for fetching an additional
/// toolchain version alongside whatever `go` is already on PATH (ported
/// intent of `gh-reusable`'s `setupGo`, which pins the same way via a
/// Docker image tag instead of this host-level mechanism). Like
/// `install_rust` assumes `rustup`, `install_node` assumes `corepack`, and
/// `install_python` assumes `uv` are already present, this assumes a base
/// `go` binary is already on PATH to bootstrap from — this crate shells to
/// real installers, it doesn't reimplement them. The freshly-installed
/// `goX.Y.Z` binary is invoked by its full `$(go env GOPATH)/bin` path
/// rather than by bare name, since that directory isn't guaranteed to be
/// on PATH the way `~/.cargo/bin`/`corepack`'s shims typically are.
pub async fn install_go() -> Result<()> {
    let version =
        std::env::var("PAWS_GO_VERSION").unwrap_or_else(|_| DEFAULT_GO_VERSION.to_string());
    run_command(
        "go",
        &["install", &format!("golang.org/dl/go{version}@latest")],
    )
    .await?;

    let gopath_output = Command::new("go")
        .args(["env", "GOPATH"])
        .output()
        .await
        .context("failed to run `go env GOPATH` — is `go` installed and on PATH?")?;
    if !gopath_output.status.success() {
        anyhow::bail!(
            "`go env GOPATH` failed: {}",
            String::from_utf8_lossy(&gopath_output.stderr)
        );
    }
    let gopath = String::from_utf8_lossy(&gopath_output.stdout)
        .trim()
        .to_string();
    let go_bin = format!("{gopath}/bin/go{version}");

    run_command(&go_bin, &["download"]).await
}

/// Real, idempotent installer for the ESP32 (ESP-IDF/`embuild`) Rust
/// toolchain via `espup` (Design Decision 6, specs/007-esp32-toolchain) —
/// the official ESP Rust toolchain installer (Xtensa-patched `rustc` +
/// `riscv32im*-esp-espidf` targets), same "shell to the real installer,
/// don't reimplement it" precedent `install_rust`/`install_go` already
/// follow for `rustup`/`golang.org/dl`. Unlike those, this assumes `espup`
/// itself is already on PATH (installed via `cargo install espup` — the
/// same bootstrap assumption `install_go` makes about a base `go` binary).
pub async fn install_esp32() -> Result<()> {
    run_command("espup", &["install"]).await
}

/// Builds a real installer closure for `ecosystem`, matching it to the
/// concrete `install_*` function above.
pub fn real_installer(ecosystem: Ecosystem) -> Box<dyn Installer> {
    match ecosystem {
        Ecosystem::Rust => Box::new(install_rust),
        Ecosystem::Node => Box::new(install_node),
        Ecosystem::Python => Box::new(install_python),
        Ecosystem::Go => Box::new(install_go),
        Ecosystem::Esp32 => Box::new(install_esp32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn sleepy_ok(millis: u64) -> Box<dyn Installer> {
        Box::new(move || async move {
            tokio::time::sleep(Duration::from_millis(millis)).await;
            Ok(())
        })
    }

    fn sleepy_err(millis: u64) -> Box<dyn Installer> {
        Box::new(move || async move {
            tokio::time::sleep(Duration::from_millis(millis)).await;
            anyhow::bail!("install failed")
        })
    }

    #[tokio::test]
    async fn independent_installers_run_concurrently_not_sequentially() {
        let tasks: Vec<(Ecosystem, Box<dyn Installer>)> = vec![
            (Ecosystem::Rust, sleepy_ok(50)),
            (Ecosystem::Node, sleepy_ok(50)),
            (Ecosystem::Python, sleepy_ok(50)),
        ];

        let start = Instant::now();
        let results = provision(tasks).await;
        let elapsed = start.elapsed();

        // Sequential would be ~150ms; concurrent should stay close to the
        // single slowest task (~50ms), with headroom for scheduler jitter.
        assert!(
            elapsed < Duration::from_millis(120),
            "expected concurrent execution, took {elapsed:?}"
        );
        assert_eq!(results.len(), 3);
        assert!(results.values().all(std::result::Result::is_ok));
    }

    fn panicky() -> Box<dyn Installer> {
        Box::new(|| async move {
            panic!("installer panicked");
            #[allow(unreachable_code)]
            Ok(())
        })
    }

    #[tokio::test]
    async fn panicking_installer_is_attributed_to_its_own_ecosystem_not_hardcoded_rust() {
        let tasks: Vec<(Ecosystem, Box<dyn Installer>)> = vec![
            (Ecosystem::Rust, sleepy_ok(10)),
            (Ecosystem::Node, panicky()),
            (Ecosystem::Python, sleepy_ok(10)),
        ];

        let results = provision(tasks).await;

        assert_eq!(results.len(), 3);
        assert!(results[&Ecosystem::Rust].is_ok());
        assert!(
            results[&Ecosystem::Node].is_err(),
            "the panicking task's ecosystem must show the failure"
        );
        assert!(
            results[&Ecosystem::Python].is_ok(),
            "a sibling task's outcome must not be overwritten by another's panic"
        );
    }

    #[tokio::test]
    async fn one_failure_does_not_hide_other_outcomes() {
        let tasks: Vec<(Ecosystem, Box<dyn Installer>)> = vec![
            (Ecosystem::Rust, sleepy_ok(10)),
            (Ecosystem::Node, sleepy_err(10)),
            (Ecosystem::Python, sleepy_ok(10)),
        ];

        let results = provision(tasks).await;

        assert_eq!(results.len(), 3);
        assert!(results[&Ecosystem::Rust].is_ok());
        assert!(results[&Ecosystem::Node].is_err());
        assert!(results[&Ecosystem::Python].is_ok());
    }

    fn tool_on_path(bin: &str) -> bool {
        std::process::Command::new(bin)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    /// `go` has no `--version` flag (only the `version` subcommand) --
    /// `tool_on_path("go")` always reported false, silently skipping every
    /// Go-dependent test in this file even when a real `go` was on PATH;
    /// the skip's `eprintln!` was hidden by cargo test's default output
    /// capturing, so it looked like the tests were genuinely passing.
    fn go_on_path() -> bool {
        std::process::Command::new("go")
            .arg("version")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    #[tokio::test]
    async fn real_installers_provision_concurrently_when_their_tools_are_present() {
        let mut tasks: Vec<(Ecosystem, Box<dyn Installer>)> = Vec::new();
        if tool_on_path("rustup") {
            tasks.push((Ecosystem::Rust, real_installer(Ecosystem::Rust)));
        }
        if tool_on_path("corepack") {
            tasks.push((Ecosystem::Node, real_installer(Ecosystem::Node)));
        }
        if tool_on_path("uv") {
            tasks.push((Ecosystem::Python, real_installer(Ecosystem::Python)));
        }
        if go_on_path() {
            tasks.push((Ecosystem::Go, real_installer(Ecosystem::Go)));
        }
        if tasks.is_empty() {
            eprintln!("skipping: none of rustup/corepack/uv/go are on PATH");
            return;
        }

        let requested: Vec<Ecosystem> = tasks.iter().map(|(e, _)| *e).collect();
        let results = provision(tasks).await;

        for ecosystem in requested {
            let result = &results[&ecosystem];
            assert!(
                result.is_ok(),
                "real installer for {ecosystem:?} failed: {result:?}"
            );
        }
    }

    #[tokio::test]
    async fn install_go_fetches_the_pinned_version_when_go_is_on_path() {
        if !go_on_path() {
            eprintln!("skipping: `go` is not on PATH");
            return;
        }
        install_go().await.expect("install_go should succeed");

        // Idempotency: `go install .../dl/goX.Y.Z@latest` + `download` must
        // succeed again without erroring once the version is already
        // fetched, same contract as install_rust/install_node/
        // install_python being safe to call repeatedly.
        install_go()
            .await
            .expect("a second install_go call should be a no-op success, not an error");
    }

    /// `paws_core`'s registry names the provisioning ecosystem for each
    /// toolchain as a string, because `paws-core` can't depend on this crate.
    /// That string is only useful if it always parses — a typo, or an
    /// ecosystem named there before its installer exists here, would
    /// otherwise silently mean "not provisionable" and skip the install.
    #[test]
    fn every_provisioned_toolchain_names_a_real_ecosystem() {
        for info in paws_core::TOOLCHAINS {
            let Some(name) = info.provisions else {
                continue;
            };
            let parsed = name.parse::<Ecosystem>().unwrap_or_else(|_| {
                panic!(
                    "toolchain {} claims provisioning ecosystem {name:?}, which Ecosystem::FromStr \
                     does not recognize",
                    info.name
                )
            });
            assert_eq!(
                Ecosystem::for_toolchain(info.toolchain),
                Some(parsed),
                "Ecosystem::for_toolchain disagrees with the registry for {}",
                info.name
            );
        }
    }

    #[test]
    fn a_toolchain_with_no_installer_provisions_nothing() {
        assert_eq!(Ecosystem::for_toolchain(Toolchain::Java), None);
        assert_eq!(Ecosystem::for_toolchain(Toolchain::Ruby), None);
        // A Tauri build is a Node build underneath, so it does have one.
        assert_eq!(
            Ecosystem::for_toolchain(Toolchain::Tauri),
            Some(Ecosystem::Node)
        );
    }

    #[test]
    fn ecosystem_esp32_round_trips_through_as_str_and_from_str() {
        assert_eq!(Ecosystem::Esp32.as_str(), "esp32");
        assert_eq!(
            "esp32".parse::<Ecosystem>().unwrap(),
            Ecosystem::Esp32,
            "Ecosystem::FromStr must recognize \"esp32\""
        );
    }

    #[tokio::test]
    async fn install_esp32_shells_out_to_espup_install_and_fails_clearly_without_it() {
        // Not asserting success here — `espup` won't be on PATH in most
        // sandboxes/CI runners for this repo itself (it's only expected to
        // be present inside `builders/esp32`, per Design Decision 6: "shell
        // to the real installer", not reimplement it). What this pins down
        // is that a missing `espup` fails with a clear, actionable error
        // naming the missing binary, rather than panicking or hanging.
        if tool_on_path("espup") {
            install_esp32()
                .await
                .expect("install_esp32 should succeed when espup is genuinely on PATH");
            return;
        }
        let err = install_esp32().await.unwrap_err();
        assert!(
            err.to_string().contains("espup"),
            "error should name the missing `espup` binary: {err}"
        );
    }
}
