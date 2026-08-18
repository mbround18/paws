//! End-to-end harness required by spec.md FR-007: `paws-docker`'s resolved
//! facts must actually be buildable by a real Docker daemon, not just
//! internally self-consistent. Gated on `docker` being present on `PATH` —
//! skips (rather than failing) when it isn't, mirroring `paws-dagger`'s
//! `ensure_available` gating pattern for `dagger`.

use std::path::{Path, PathBuf};
use std::process::Command;

use paws_docker::{DockerFactsInput, GithubContext, resolve_docker_facts};

fn docker_available() -> bool {
    Command::new("docker")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn buildx_available() -> bool {
    Command::new("docker")
        .args(["buildx", "version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

#[test]
fn no_compose_fixture_builds_with_real_docker() {
    if !docker_available() {
        eprintln!("skipping: docker not on PATH");
        return;
    }

    let workspace = examples_dir().join("docker-fixture");
    let facts = resolve_docker_facts(
        &DockerFactsInput {
            image: "paws-e2e-docker-fixture".to_string(),
            version: "0.0.1".to_string(),
            ..Default::default()
        },
        &GithubContext {
            workspace: workspace.clone(),
            event_name: "push".to_string(),
            git_ref: "refs/heads/main".to_string(),
            default_branch: "main".to_string(),
            pr_labels: vec![],
        },
    );

    let status = Command::new("docker")
        .arg("build")
        .arg("-f")
        .arg(workspace.join(facts.dockerfile.trim_start_matches("./")))
        .arg("-t")
        .arg("paws-e2e-docker-fixture:test")
        .arg(workspace.join(facts.context.trim_start_matches("./")))
        .status()
        .expect("failed to spawn docker build");

    assert!(
        status.success(),
        "docker build failed for the no-compose fixture"
    );

    let _ = Command::new("docker")
        .args(["rmi", "-f", "paws-e2e-docker-fixture:test"])
        .status();
}

#[test]
fn compose_fixture_resolved_service_builds_with_real_docker() {
    if !docker_available() {
        eprintln!("skipping: docker not on PATH");
        return;
    }

    let workspace = examples_dir().join("docker-compose-fixture");
    let facts = resolve_docker_facts(
        &DockerFactsInput {
            image: "ghcr.io/example/app".to_string(),
            version: "0.0.1".to_string(),
            ..Default::default()
        },
        &GithubContext {
            workspace: workspace.clone(),
            event_name: "push".to_string(),
            git_ref: "refs/heads/main".to_string(),
            default_branch: "main".to_string(),
            pr_labels: vec![],
        },
    );

    assert_eq!(facts.target, "runtime");

    let mut cmd = Command::new("docker");
    cmd.arg("build")
        .arg("-f")
        .arg(workspace.join(facts.dockerfile.trim_start_matches("./")))
        .arg("--target")
        .arg(&facts.target)
        .arg("-t")
        .arg("paws-e2e-compose-fixture:test");
    for (key, value) in &facts.build_args {
        cmd.arg("--build-arg").arg(format!("{key}={value}"));
    }
    cmd.arg(workspace.join(facts.context.trim_start_matches("./")));

    let status = cmd.status().expect("failed to spawn docker build");
    assert!(
        status.success(),
        "docker build failed for the compose-resolved fixture"
    );

    let _ = Command::new("docker")
        .args(["rmi", "-f", "paws-e2e-compose-fixture:test"])
        .status();
}

#[test]
fn buildkit_only_fixture_fails_on_legacy_builder_and_succeeds_via_buildx() {
    if !docker_available() || !buildx_available() {
        eprintln!("skipping: docker/buildx not on PATH");
        return;
    }

    let fixture = examples_dir().join("docker-buildkit-fixture");

    let legacy = Command::new("docker")
        .env("DOCKER_BUILDKIT", "0")
        .arg("build")
        .arg("-t")
        .arg("paws-e2e-buildkit-fixture-legacy:test")
        .arg(&fixture)
        .status()
        .expect("failed to spawn legacy docker build");
    assert!(
        !legacy.success(),
        "buildkit-only fixture must NOT build on the legacy builder"
    );

    let buildkit = Command::new("docker")
        .args([
            "buildx",
            "build",
            "--load",
            "-t",
            "paws-e2e-buildkit-fixture:test",
        ])
        .arg(&fixture)
        .status()
        .expect("failed to spawn buildx build");
    assert!(
        buildkit.success(),
        "buildkit-only fixture must build via `docker buildx build`"
    );

    let _ = Command::new("docker")
        .args(["rmi", "-f", "paws-e2e-buildkit-fixture:test"])
        .status();
}
