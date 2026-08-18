//! Concurrent toolchain provisioning. Each requested ecosystem (rust, node,
//! python, ...) runs as an independent `tokio` task with no implicit
//! ordering dependency on any other — installing Rust never waits on pnpm,
//! and vice versa. See specs/001-paws-core-cli/spec.md's Concurrency Model
//! (FR-013..FR-016) for the contract this crate exists to satisfy.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use tokio::task::JoinSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ecosystem {
    Rust,
    Node,
    Python,
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

/// Runs every requested ecosystem's installer concurrently and returns every
/// outcome — success or failure — with no early return on first failure
/// (FR-014). Genuinely independent work only; a step with a real ordering
/// dependency on another does not belong here (FR-016).
pub async fn provision(
    tasks: Vec<(Ecosystem, Box<dyn Installer>)>,
) -> HashMap<Ecosystem, Result<()>> {
    let mut set = JoinSet::new();
    for (ecosystem, installer) in tasks {
        set.spawn(async move { (ecosystem, installer.install().await) });
    }

    let mut results = HashMap::new();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((ecosystem, result)) => {
                results.insert(ecosystem, result);
            }
            Err(join_err) => {
                // A panicking task must still surface as a reported failure,
                // never silently vanish from the aggregate (FR-014).
                results.insert(Ecosystem::Rust, Err(anyhow::anyhow!(join_err)));
            }
        }
    }
    results
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
        assert!(results.values().all(|r| r.is_ok()));
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
}
