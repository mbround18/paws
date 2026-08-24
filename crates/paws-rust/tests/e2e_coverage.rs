//! End-to-end harness for `paws ci --toolchain rust --coverage`
//! (specs/004-rust-coverage/quickstart.md §4): proves `cargo llvm-cov`
//! actually measures a real gap against `examples/rust-coverage-fixture`
//! (a deliberately untested branch), not just running and reporting a
//! fixed/fake number. Gated on `docker` being present on `PATH` — skips
//! (rather than failing) when it isn't, mirroring `paws-docker`'s own
//! `tests/e2e_docker_daemon.rs` gating pattern.

use std::path::{Path, PathBuf};
use std::process::Command;

fn docker_available() -> bool {
    Command::new("docker")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn repo_root() -> PathBuf {
    // crates/paws-rust -> repo root
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("failed to resolve repo root")
}

#[test]
fn coverage_measures_a_real_gap_in_the_fixture() {
    if !docker_available() {
        eprintln!("skipping: docker not on PATH");
        return;
    }

    let root = repo_root();
    let builder_dir = root.join("builders/rust");
    let fixture_dir = root.join("examples/rust-coverage-fixture");

    let image_tag = "paws-e2e-rust-coverage-fixture:test-builder";
    let build_status = Command::new("docker")
        .arg("build")
        .arg("-t")
        .arg(image_tag)
        .arg(&builder_dir)
        .status()
        .expect("failed to spawn docker build for builders/rust");
    assert!(
        build_status.success(),
        "docker build failed for builders/rust/Dockerfile"
    );

    let output = Command::new("docker")
        .args(["run", "--rm"])
        .arg("-v")
        .arg(format!("{}:/src", fixture_dir.display()))
        .arg("-w")
        .arg("/src")
        .arg(image_tag)
        .args(["cargo", "llvm-cov", "--summary-only"])
        .output()
        .expect("failed to spawn docker run for cargo llvm-cov");

    let _ = Command::new("docker")
        .args(["rmi", "-f", image_tag])
        .status();

    assert!(
        output.status.success(),
        "cargo llvm-cov failed inside the builders/rust image:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let total_line = stdout
        .lines()
        .find(|line| line.trim_start().starts_with("TOTAL"))
        .unwrap_or_else(|| panic!("no TOTAL line in cargo llvm-cov output:\n{stdout}"));

    // The TOTAL line's region-coverage percentage is the 3rd whitespace-
    // separated column (Regions, Missed Regions, Cover%, ...) — parse it
    // and assert it's genuinely below 100%, proving the "positive" branch
    // examples/rust-coverage-fixture's test deliberately never exercises
    // shows up as a real, measured gap rather than the tool trivially
    // reporting full coverage regardless of what's actually tested.
    let cover_pct: f64 = total_line
        .split_whitespace()
        .nth(2)
        .and_then(|s| s.trim_end_matches('%').parse().ok())
        .unwrap_or_else(|| panic!("couldn't parse Cover% from TOTAL line: {total_line}"));

    assert!(
        cover_pct < 100.0,
        "expected sub-100% coverage (the fixture has a deliberately untested branch), got {cover_pct}%:\n{stdout}"
    );
    assert!(
        cover_pct > 0.0,
        "expected non-zero coverage (one test does run), got {cover_pct}%:\n{stdout}"
    );
}
