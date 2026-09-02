//! Native Elixir CI support for `paws ci --toolchain elixir`. No
//! `gh-reusable` precedent — its Dagger module never had an Elixir/BEAM
//! function of any kind (see `packages/dagger-module/src/index.ts`) — so
//! the step sequence here is a new native implementation following
//! ordinary Mix conventions: install Hex/rebar, fetch deps, compile, test.
//!
//! `mix local.hex --force`/`mix local.rebar --force` run first because a
//! fresh container has neither installed and `mix deps.get` would
//! otherwise stop to prompt for them interactively (Hex for Elixir
//! packages, rebar3 for the Erlang ones many real dependency trees pull
//! in) — the same reason every real Elixir Dockerfile opens with those two
//! lines. Both are no-ops for a project with no dependencies, so this
//! costs nothing in the trivial case.
//!
//! `mix compile --warnings-as-errors` is deliberate rather than a plain
//! `mix compile`: Elixir's compiler warnings cover genuinely broken code
//! (an undefined function, an unreachable clause) that would otherwise
//! only surface at runtime, and `paws-rust`'s `clippy -D warnings` sets
//! the same precedent for what a `paws ci` gate means. `mix test` then
//! recompiles under `MIX_ENV=test` itself — that's Mix's own behavior, not
//! a redundant step this crate adds.
//!
//! No `builders/*` image: the official `elixir` image already carries
//! Elixir, Erlang/OTP, and Mix (see `docs/ROADMAP.md`'s "How a new stack
//! gets added"). Umbrella projects need no special handling — `mix
//! deps.get`/`compile`/`test` at an umbrella root already fan out across
//! every app in `apps/`.

use paws_core::Pipeline;
use std::path::Path;

use anyhow::Result;

/// The official `elixir` image publishes floating `otp-<major>` tags that
/// track the latest Elixir release built against that OTP major (confirmed
/// directly against Docker Hub's tag list, alongside the fully-pinned
/// `<elixir>-otp-<otp>` ones). Elixir floats — its releases are
/// backward-compatible and there's no LTS concept — while the OTP major
/// stays pinned, because *that* is the axis where a bump is a real
/// judgment call: NIF/native dependencies and Erlang-level behavior change
/// across OTP majors in ways Elixir minor releases don't. A genuine
/// Renovate target on each OTP major, in the same
/// `automerge: false` spirit as the Temurin pin (see `docs/ROADMAP.md`'s
/// "Base image version policy").
pub const BASE_IMAGE: &str = "elixir:otp-28";

/// A Mix project has a `mix.exs` at its root — the file every `mix`
/// subcommand used here requires to resolve the project.
pub fn is_elixir_project(dir: &Path) -> bool {
    dir.join("mix.exs").is_file()
}

#[derive(Debug)]
pub struct ElixirProject {
    /// Whether `mix.lock` is committed. It doesn't change the commands
    /// (`mix deps.get` honors a committed lock automatically and Mix has
    /// no `--frozen` equivalent to `uv sync`'s), but it *is* what makes the
    /// dependency resolution reproducible, so it's worth reporting.
    pub has_lockfile: bool,
}

pub fn detect_project(dir: &Path) -> Result<ElixirProject> {
    if !is_elixir_project(dir) {
        anyhow::bail!("no mix.exs found in {}", dir.display());
    }
    Ok(ElixirProject {
        has_lockfile: dir.join("mix.lock").is_file(),
    })
}

/// Builds the `dagger core <chain>` argument list (see `paws_dagger::core`)
/// for `source_dir`: Hex/rebar install, `mix deps.get`, `mix compile
/// --warnings-as-errors`, `mix test`.
pub fn dagger_pipeline_args(source_dir: &str) -> Vec<String> {
    dagger_pipeline_args_with_image(source_dir, BASE_IMAGE)
}

/// [`dagger_pipeline_args`] against an explicit image — see
/// `paws_core::Toolchain::image_for`.
pub fn dagger_pipeline_args_with_image(source_dir: &str, image: &str) -> Vec<String> {
    Pipeline::from_image(image)
        .mount("/src", source_dir)
        .workdir("/src")
        .exec(["mix", "local.hex", "--force"])
        .exec(["mix", "local.rebar", "--force"])
        .exec(["mix", "deps.get"])
        .exec(["mix", "compile", "--warnings-as-errors"])
        .exec(["mix", "test"])
        .stdout()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        paws_core::test_support::scratch_dir("elixir", name)
    }

    #[test]
    fn detects_elixir_project_from_mix_exs() {
        let dir = temp_dir("detect");
        assert!(!is_elixir_project(&dir));
        fs::write(dir.join("mix.exs"), "defmodule X.MixProject do end").unwrap();
        assert!(is_elixir_project(&dir));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn errors_when_no_mix_exs() {
        let dir = temp_dir("no-mix");
        assert!(detect_project(&dir).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn detects_lockfile_presence() {
        let dir = temp_dir("lockfile");
        fs::write(dir.join("mix.exs"), "").unwrap();
        assert!(!detect_project(&dir).unwrap().has_lockfile);
        fs::write(dir.join("mix.lock"), "%{}").unwrap();
        assert!(detect_project(&dir).unwrap().has_lockfile);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pipeline_installs_hex_and_rebar_before_fetching_deps() {
        let args = dagger_pipeline_args("/host/src");
        assert_eq!(args[0], "container");
        assert_eq!(args[2], "--address=elixir:otp-28");
        let hex = args
            .iter()
            .position(|a| a == "--args=mix,local.hex,--force")
            .expect("hex install step");
        let rebar = args
            .iter()
            .position(|a| a == "--args=mix,local.rebar,--force")
            .expect("rebar install step");
        let deps = args
            .iter()
            .position(|a| a == "--args=mix,deps.get")
            .expect("deps.get step");
        assert!(hex < deps && rebar < deps);
    }

    #[test]
    fn pipeline_compiles_warnings_as_errors_then_tests() {
        let args = dagger_pipeline_args("/host/src");
        let compile = args
            .iter()
            .position(|a| a == "--args=mix,compile,--warnings-as-errors")
            .expect("compile step");
        let test = args
            .iter()
            .position(|a| a == "--args=mix,test")
            .expect("test step");
        assert!(compile < test);
        assert_eq!(args.last(), Some(&"stdout".to_string()));
    }
}
