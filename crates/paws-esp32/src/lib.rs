//! Native ESP32 (ESP-IDF/`embuild`) CI support for `paws ci --toolchain
//! esp32` (specs/007-esp32-toolchain). No `gh-reusable` function exists to
//! port for parity — this is new, `paws`-native capability, same as
//! `paws-go`/`paws-kotlin` (see this crate's spec's Affected Contracts).
//!
//! Targets the `esp-idf-sys`/`esp-idf-svc` + `embuild` stack specifically
//! (`xtensa-esp32*-espidf`/`riscv32im*-esp-espidf` triples), matching
//! `mbround18/ha-kiosk`'s `firmware/` crate — this spec's first concrete
//! driver — not a bare-metal `esp-hal`/`no_std` project (a different,
//! simpler toolchain shape, out of scope here; see spec.md's Out of scope).
//!
//! Pipeline chain is `fmt → clippy → build → (conditional) host-crate
//! test`, deliberately reordered from `paws-rust`'s default `fmt → clippy →
//! test → build` (see this spec's Design Decision 2): the embedded target
//! itself has no `cargo test` story at all (`ha-kiosk`'s own
//! `CONTRIBUTING.md`: `[[bin]] harness = false` skips even *compiling*
//! `#[cfg(test)]` code for that target), so `build` — the one step that
//! actually has to succeed for this toolchain to mean anything — runs
//! before the conditional test step, rather than being silently skippable
//! alongside it.
//!
//! A host-testable sibling crate (Design Decision 3) is detected the same
//! generic way `paws-publish::find_workspace_root` locates a real workspace
//! root rather than assuming a fixed layout: this crate mounts the
//! *workspace root* (not just the ESP32 project's own subdirectory) so a
//! sibling crate like `ha-kiosk`'s `firmware-core` — a workspace member
//! sitting next to `firmware/`, not nested inside it — is actually present
//! in the container to test.

use paws_core::Pipeline;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
pub use paws_release::{AssetUploadMode, GitHubReleaseClient};

/// The `builders/esp32` Dockerfile, embedded at compile time — same
/// embed-and-materialize-to-a-temp-dir pattern every other per-toolchain
/// crate in this workspace uses (`paws-kotlin`/`paws-java`/`paws-rust`),
/// since `paws ci` runs from inside whatever *target* repo it's checking,
/// not from inside `paws`'s own source tree.
const ESP32_DOCKERFILE: &str = include_str!("../../../builders/esp32/Dockerfile");

/// Writes the embedded `builders/esp32` Dockerfile to a temp directory and
/// returns that directory's path, suitable for [`dagger_pipeline_args`]'s
/// `builder_dir` argument — mirrors `paws-kotlin`'s/`paws-tauri`'s own
/// same-named function.
pub fn write_builder_dockerfile() -> Result<PathBuf> {
    paws_core::write_builder_dockerfile("esp32", ESP32_DOCKERFILE)
}

/// Whether `manifest` (a `Cargo.toml`'s raw text) names an ESP-IDF
/// dependency — a purpose-built, deliberate signal (unlike e.g. a stray
/// mention in a comment), matching `paws_rust::is_wasm_project`'s
/// substring-on-manifest-text detection style rather than pulling in a
/// TOML-parsing dependency for this alone.
fn manifest_depends_on_esp_idf(manifest: &str) -> bool {
    manifest.contains("esp-idf-sys") || manifest.contains("esp-idf-svc")
}

/// Whether `config` (a `.cargo/config.toml`'s raw text) sets `build.target`
/// to an `*-espidf` triple (`xtensa-esp32*-espidf`/`riscv32im*-esp-espidf`)
/// — checked as a plain substring on a `target = "..."` line rather than a
/// full TOML parse, same style as this crate's manifest check.
fn config_targets_espidf(config: &str) -> bool {
    config
        .lines()
        .any(|line| line.contains("target") && line.contains("-espidf"))
}

/// A project is an ESP32 (ESP-IDF/`embuild`) target when its `Cargo.toml`
/// depends on `esp-idf-sys`/`esp-idf-svc`, or its `.cargo/config.toml` sets
/// `build.target` to an `*-espidf` triple — mirrors `paws-rust`'s existing
/// `is_wasm_project`-style marker-file detection (spec.md's Scope), not a
/// new detection mechanism.
pub fn is_esp32_project(dir: &Path) -> bool {
    if let Ok(manifest) = std::fs::read_to_string(dir.join("Cargo.toml"))
        && manifest_depends_on_esp_idf(&manifest)
    {
        return true;
    }
    if let Ok(config) = std::fs::read_to_string(dir.join(".cargo").join("config.toml"))
        && config_targets_espidf(&config)
    {
        return true;
    }
    false
}

/// The reverse of [`is_esp32_project`] for a single candidate directory —
/// used by [`find_host_testable_sibling`] to confirm a workspace member is
/// genuinely *not* an ESP32 target (no `esp-idf-sys`/`esp-idf-svc`
/// dependency, no `*-espidf` target override) rather than just "isn't the
/// one we started from".
fn is_not_esp32_crate(dir: &Path) -> bool {
    dir.join("Cargo.toml").is_file() && !is_esp32_project(dir)
}

/// Parses a root `Cargo.toml`'s `[workspace]` `members = [...]` list —
/// same lightweight, purpose-built string scan every other Cargo.toml
/// signal in this workspace uses (`paws_rust::is_wasm_project`,
/// `paws_go::module_name`) rather than pulling in a TOML-parsing
/// dependency for this alone. Returns an empty vec if there's no
/// `[workspace]`/`members` section — the caller falls back to scanning
/// immediate subdirectories in that case.
fn workspace_members(root_manifest: &str) -> Vec<String> {
    let Some(start) = root_manifest.find("members") else {
        return Vec::new();
    };
    let after = &root_manifest[start..];
    let Some(open) = after.find('[') else {
        return Vec::new();
    };
    let Some(close) = after[open..].find(']') else {
        return Vec::new();
    };
    let list = &after[open + 1..open + close];
    list.split(',')
        .filter_map(|entry| {
            let trimmed = entry.trim().trim_matches('"').trim_matches('\'');
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect()
}

/// Finds a workspace member under `workspace_root` that's host-testable —
/// a real `cargo test` can run against it on the build container's own host
/// target, not the embedded ESP32 target (Design Decision 3): a
/// `Cargo.toml` with no `esp-idf-sys`/`esp-idf-svc` dependency and no
/// `*-espidf` target override. Not a hardcoded name (`firmware-core` is
/// `ha-kiosk`-specific, not a generic convention) — the first matching
/// workspace member wins.
///
/// Falls back to scanning `workspace_root`'s immediate subdirectories (each
/// one containing a `Cargo.toml`) when `workspace_root`'s own `Cargo.toml`
/// has no `[workspace]`/`members` list to read (or no `Cargo.toml` at all)
/// — a project without any such sibling simply gets no test step (`None`),
/// same as `paws-rust`'s existing `is_wasm` short-circuit skips testing
/// outright rather than erroring.
pub fn find_host_testable_sibling(workspace_root: &Path) -> Option<PathBuf> {
    let root_manifest = std::fs::read_to_string(workspace_root.join("Cargo.toml")).ok();
    let members = root_manifest
        .as_deref()
        .map(workspace_members)
        .unwrap_or_default();

    if !members.is_empty() {
        return members
            .into_iter()
            .map(|member| workspace_root.join(member))
            .find(|candidate| is_not_esp32_crate(candidate));
    }

    let Ok(entries) = std::fs::read_dir(workspace_root) else {
        return None;
    };
    let mut candidates: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    candidates.sort();
    candidates.into_iter().find(|path| is_not_esp32_crate(path))
}

fn relative_workdir(subpath: &str) -> String {
    if subpath == "." || subpath.is_empty() {
        "/src".to_string()
    } else {
        format!("/src/{subpath}")
    }
}

/// Builds the `dagger core <chain>` argument list (see `paws_dagger::core`)
/// for an ESP32 project: `mount_dir` (a host path — the workspace root when
/// a host-testable sibling exists, otherwise the ESP32 project directory
/// itself) is mounted at `/src`; `builder_dir` (see
/// [`write_builder_dockerfile`]) is built via `docker-build`, matching
/// `paws-kotlin`'s/`paws-tauri`'s always-build-the-embedded-Dockerfile
/// shape (Design Decision 4) rather than `paws-rust --coverage`'s
/// conditional swap.
///
/// `project_subpath` is `mount_dir`'s relative path to the ESP32 crate
/// itself (`"."` when `mount_dir` *is* the ESP32 crate, i.e. no sibling was
/// found) — `fmt`/`clippy`/`build` all run with the container's workdir set
/// there. `host_test_subpath` (from [`find_host_testable_sibling`],
/// relative to `mount_dir`) is `None` when no host-testable sibling exists
/// (no test step at all) or `Some(path)` to run `cargo test` with the
/// workdir switched to that sibling afterward (Design Decision 2: `fmt →
/// clippy → build → conditional test`, `build` never gated behind a test
/// step that might not even exist for this project).
/// The `host directory ... docker-build ... with-mounted-directory ...
/// with-workdir ... cargo fmt ... cargo clippy ... cargo build --release`
/// prefix shared by [`dagger_pipeline_args`] and
/// [`dagger_export_pipeline_args`] — kept as one function so the two
/// pipelines can never drift into building/testing under different
/// conditions (e.g. one linting and the other not).
fn build_and_lint_prefix(mount_dir: &str, project_subpath: &str, builder_dir: &str) -> Vec<String> {
    Pipeline::from_builder_image(builder_dir)
        .mount("/src", mount_dir)
        .workdir(&relative_workdir(project_subpath))
        .exec(["cargo", "fmt", "--", "--check"])
        .exec(["cargo", "clippy", "--", "-D", "warnings"])
        .exec(["cargo", "build", "--release"])
        .into_args()
}

pub fn dagger_pipeline_args(
    mount_dir: &str,
    project_subpath: &str,
    builder_dir: &str,
    host_test_subpath: Option<&str>,
) -> Vec<String> {
    let mut pipeline = Pipeline::from_raw(build_and_lint_prefix(
        mount_dir,
        project_subpath,
        builder_dir,
    ));

    if let Some(test_subpath) = host_test_subpath {
        pipeline = pipeline
            .workdir(&relative_workdir(test_subpath))
            .exec(["cargo", "test"]);
    }

    pipeline.stdout()
}

/// Builds the `dagger core <chain>` argument list to (re-)build an ESP32
/// project and export its `target/<target_triple>/release` directory (the
/// bootloader + firmware ELF [`publish_artifacts`] uploads) to a host path
/// — a *separate* `dagger core` invocation from [`dagger_pipeline_args`],
/// not something appended onto it: Dagger's `core` chain is linear, one
/// terminal call whose return type has to match what's asked for, and
/// `directory ... export` terminates on a bool, not a `Container` —
/// nothing (e.g. `dagger_pipeline_args`'s conditional host-test
/// `with-workdir`/`with-exec`) can chain after it. Shares
/// `build_and_lint_prefix` with [`dagger_pipeline_args`] so this never
/// exports artifacts from a build that skipped fmt/clippy.
///
/// `target_triple` ([`target_triple`]) and `host_release_dir` (the host
/// path [`publish_artifacts`] later reads `bootloader.bin`/the ELF from)
/// are both the caller's responsibility to keep in sync with
/// [`bootloader_path`]/[`firmware_elf_path`]'s expectations.
pub fn dagger_export_pipeline_args(
    mount_dir: &str,
    project_subpath: &str,
    builder_dir: &str,
    target_triple: &str,
    host_release_dir: &str,
) -> Vec<String> {
    Pipeline::from_raw(build_and_lint_prefix(
        mount_dir,
        project_subpath,
        builder_dir,
    ))
    .export_directory(&format!("target/{target_triple}/release"), host_release_dir)
}

/// Reads the `.cargo/config.toml`'s `build.target` triple (e.g.
/// `riscv32imafc-esp-espidf`) — needed to locate the real
/// `target/<triple>/release` directory `cargo build --release` produces,
/// since ESP-IDF/`embuild` builds are always cross-compiles (there's no
/// plain `target/release` the way a host-target crate has).
pub fn target_triple(dir: &Path) -> Result<String> {
    let config = std::fs::read_to_string(dir.join(".cargo").join("config.toml"))
        .with_context(|| format!("failed to read .cargo/config.toml in {}", dir.display()))?;
    config
        .lines()
        .find_map(|line| {
            let line = line.trim();
            if !line.contains("target") || line.starts_with('#') {
                return None;
            }
            let (_, value) = line.split_once('=')?;
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if value.ends_with("-espidf") {
                Some(value.to_string())
            } else {
                None
            }
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                ".cargo/config.toml in {} has no build.target set to an *-espidf triple",
                dir.display()
            )
        })
}

/// Reads the built binary's name — the `[[bin]] name = "..."` in
/// `Cargo.toml` if set (`ha-kiosk`'s `firmware/Cargo.toml` sets this
/// explicitly), falling back to `[package] name` otherwise (Cargo's own
/// default binary-name rule when no `[[bin]]` table is present).
pub fn binary_name(dir: &Path) -> Result<String> {
    let manifest = std::fs::read_to_string(dir.join("Cargo.toml"))
        .with_context(|| format!("failed to read Cargo.toml in {}", dir.display()))?;

    if let Some(bin_start) = manifest.find("[[bin]]") {
        let after = &manifest[bin_start + "[[bin]]".len()..];
        let bin_block = after
            .lines()
            .take_while(|line| !line.trim_start().starts_with('['));
        if let Some(name) = bin_block.map(str::trim).find_map(|line| {
            line.strip_prefix("name")
                .and_then(|rest| rest.trim_start().strip_prefix('='))
                .map(|v| v.trim().trim_matches('"').trim_matches('\'').to_string())
        }) {
            return Ok(name);
        }
    }

    manifest
        .lines()
        .find_map(|line| {
            let line = line.trim();
            line.strip_prefix("name")
                .and_then(|rest| rest.trim_start().strip_prefix('='))
                .map(|v| v.trim().trim_matches('"').trim_matches('\'').to_string())
        })
        .ok_or_else(|| anyhow::anyhow!("Cargo.toml in {} has no [package] name", dir.display()))
}

/// Well-known relative paths under an ESP-IDF/`embuild` release build's
/// output layout — not a configurable glob (Design Decision 5): the
/// bootloader binary always lands at `bootloader/bootloader.bin` under the
/// release directory, and the firmware ELF is always named after the
/// `[[bin]]`, directly under it.
pub fn bootloader_path(release_dir: &Path) -> PathBuf {
    release_dir.join("bootloader").join("bootloader.bin")
}

/// See [`bootloader_path`] — the firmware ELF's well-known path.
pub fn firmware_elf_path(release_dir: &Path, binary_name: &str) -> PathBuf {
    release_dir.join(binary_name)
}

/// Uploads the built bootloader (`bootloader.bin`) and firmware ELF as
/// assets on the GitHub Release `release_id` (from
/// `GitHubReleaseClient::get_or_create_release`), reusing
/// `paws-release`'s existing `GitHubReleaseClient` rather than a second
/// GitHub-API client (Design Decision 1) — `paws-esp32` takes a normal path
/// dependency on `paws-release` for just this type, not a duplicate or a
/// moved copy.
///
/// Both assets upload with [`AssetUploadMode::Clobber`] — a re-run of the
/// same tag replaces the existing asset rather than duplicating it, the
/// same idempotency `paws release`'s own binary uploads already rely on.
pub async fn publish_artifacts(
    client: &GitHubReleaseClient,
    release_id: u64,
    release_dir: &Path,
    binary_name: &str,
) -> Result<()> {
    let bootloader = bootloader_path(release_dir);
    client
        .upload_asset_with(
            release_id,
            &bootloader,
            "application/octet-stream",
            AssetUploadMode::Clobber,
        )
        .await
        .with_context(|| format!("failed to upload {}", bootloader.display()))?;

    let firmware_elf = firmware_elf_path(release_dir, binary_name);
    client
        .upload_asset_with(
            release_id,
            &firmware_elf,
            "application/octet-stream",
            AssetUploadMode::Clobber,
        )
        .await
        .with_context(|| format!("failed to upload {}", firmware_elf.display()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        paws_core::test_support::scratch_dir("esp32", name)
    }

    #[test]
    fn does_not_detect_a_plain_rust_project() {
        let dir = temp_dir("plain-rust");
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        assert!(!is_esp32_project(&dir));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn detects_esp32_project_from_esp_idf_svc_dependency() {
        let dir = temp_dir("esp-idf-svc-dep");
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"firmware\"\n\n[dependencies]\nesp-idf-svc = \"0.52\"\n",
        )
        .unwrap();
        assert!(is_esp32_project(&dir));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn detects_esp32_project_from_esp_idf_sys_dependency() {
        let dir = temp_dir("esp-idf-sys-dep");
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"firmware\"\n\n[dependencies]\nesp-idf-sys = \"0.36\"\n",
        )
        .unwrap();
        assert!(is_esp32_project(&dir));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn detects_esp32_project_from_espidf_target_in_cargo_config() {
        let dir = temp_dir("espidf-target-config");
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"firmware\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::create_dir_all(dir.join(".cargo")).unwrap();
        fs::write(
            dir.join(".cargo").join("config.toml"),
            "[build]\ntarget = \"riscv32imafc-esp-espidf\"\n",
        )
        .unwrap();
        assert!(is_esp32_project(&dir));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn write_builder_dockerfile_materializes_the_embedded_dockerfile() {
        let dir = write_builder_dockerfile().unwrap();
        let contents = fs::read_to_string(dir.join("Dockerfile")).unwrap();
        assert_eq!(contents, ESP32_DOCKERFILE);
    }

    #[test]
    fn embedded_dockerfile_uses_the_rust_bookworm_base_and_installs_espup() {
        assert!(ESP32_DOCKERFILE.contains("FROM rust:1-bookworm"));
        assert!(ESP32_DOCKERFILE.contains("espup"));
        assert!(ESP32_DOCKERFILE.contains("espflash"));
        assert!(ESP32_DOCKERFILE.contains("LIBCLANG_PATH"));
    }

    // find_host_testable_sibling ------------------------------------------------

    #[test]
    fn finds_a_host_testable_sibling_from_workspace_members() {
        let root = temp_dir("workspace-sibling");
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nresolver = \"2\"\nmembers = [\"flasher\", \"firmware-core\"]\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("flasher")).unwrap();
        fs::write(
            root.join("flasher").join("Cargo.toml"),
            "[package]\nname = \"flasher\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("firmware-core")).unwrap();
        fs::write(
            root.join("firmware-core").join("Cargo.toml"),
            "[package]\nname = \"firmware-core\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let found = find_host_testable_sibling(&root).unwrap();
        assert_eq!(found, root.join("flasher"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn skips_a_workspace_member_that_is_itself_an_esp32_target() {
        let root = temp_dir("workspace-skip-esp32-member");
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"embedded-thing\", \"firmware-core\"]\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("embedded-thing")).unwrap();
        fs::write(
            root.join("embedded-thing").join("Cargo.toml"),
            "[package]\nname = \"embedded-thing\"\n\n[dependencies]\nesp-idf-svc = \"0.52\"\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("firmware-core")).unwrap();
        fs::write(
            root.join("firmware-core").join("Cargo.toml"),
            "[package]\nname = \"firmware-core\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let found = find_host_testable_sibling(&root).unwrap();
        assert_eq!(found, root.join("firmware-core"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn returns_none_when_no_workspace_and_no_sibling_crates_exist() {
        let root = temp_dir("no-sibling");
        assert!(find_host_testable_sibling(&root).is_none());
        fs::remove_dir_all(&root).unwrap();
    }

    // dagger_pipeline_args --------------------------------------------------

    #[test]
    fn pipeline_builds_the_builder_then_runs_fmt_clippy_build_in_order_with_no_test_step() {
        let args = dagger_pipeline_args("/host/src", ".", "/builder/dir", None);
        assert_eq!(args[0], "host");
        assert_eq!(args[1], "directory");
        assert_eq!(args[2], "--path=/builder/dir");
        assert_eq!(args[3], "docker-build");

        let expected = [
            "--args=cargo,fmt,--,--check",
            "--args=cargo,clippy,--,-D,warnings",
            "--args=cargo,build,--release",
        ];
        let positions: Vec<usize> = expected
            .iter()
            .map(|step| args.iter().position(|a| a == step).unwrap())
            .collect();
        assert!(
            positions.windows(2).all(|w| w[0] < w[1]),
            "fmt/clippy/build must run in order: {positions:?}"
        );
        assert!(
            !args.iter().any(|a| a.contains("cargo,test")),
            "no test step when no host-testable sibling was found"
        );
        assert_eq!(args.last(), Some(&"stdout".to_string()));
    }

    #[test]
    fn pipeline_runs_test_against_the_sibling_after_build_when_one_is_found() {
        let args = dagger_pipeline_args(
            "/host/workspace",
            "firmware",
            "/builder/dir",
            Some("firmware-core"),
        );
        let build_pos = args
            .iter()
            .position(|a| a == "--args=cargo,build,--release")
            .unwrap();
        let workdir_switch_pos = args
            .iter()
            .position(|a| a == "--path=/src/firmware-core")
            .unwrap();
        let test_pos = args.iter().position(|a| a == "--args=cargo,test").unwrap();
        assert!(build_pos < workdir_switch_pos);
        assert!(workdir_switch_pos < test_pos);
        assert_eq!(args.last(), Some(&"stdout".to_string()));
    }

    #[test]
    fn pipeline_sets_the_initial_workdir_to_the_project_subpath() {
        let args = dagger_pipeline_args("/host/workspace", "firmware", "/builder/dir", None);
        assert!(args.contains(&"--path=/src/firmware".to_string()));
    }

    #[test]
    fn export_pipeline_builds_lints_then_exports_the_release_dir_to_the_host() {
        // Regression test: the pipeline `paws ci --toolchain esp32
        // --publish-artifacts` originally ran built the release binary
        // entirely inside the ephemeral Dagger container and never
        // exported it anywhere, so `publish_artifacts` (which reads from a
        // host path) had nothing real to upload.
        let args = dagger_export_pipeline_args(
            "/host/workspace",
            "firmware",
            "/builder/dir",
            "riscv32imafc-esp-espidf",
            "/host/workspace/firmware/target/riscv32imafc-esp-espidf/release",
        );
        let expected = [
            "--args=cargo,fmt,--,--check",
            "--args=cargo,clippy,--,-D,warnings",
            "--args=cargo,build,--release",
            "--path=target/riscv32imafc-esp-espidf/release",
            "--path=/host/workspace/firmware/target/riscv32imafc-esp-espidf/release",
        ];
        let positions: Vec<usize> = expected
            .iter()
            .map(|step| args.iter().position(|a| a == step).unwrap())
            .collect();
        assert!(
            positions.windows(2).all(|w| w[0] < w[1]),
            "fmt/clippy/build must run before the directory is selected/exported: {positions:?}"
        );
        assert_eq!(args[args.len() - 4], "directory");
        assert_eq!(args[args.len() - 2], "export");
        assert!(
            !args.contains(&"stdout".to_string()),
            "the export chain's terminal call is `export`, not `stdout` — nothing can chain \
             after it (a bool, not a Container), so it can't also run the conditional host-test \
             step dagger_pipeline_args supports"
        );
    }

    // target_triple / binary_name ---------------------------------------

    #[test]
    fn target_triple_reads_the_espidf_build_target() {
        let dir = temp_dir("target-triple");
        fs::create_dir_all(dir.join(".cargo")).unwrap();
        fs::write(
            dir.join(".cargo").join("config.toml"),
            "[build]\ntarget = \"riscv32imafc-esp-espidf\"\n",
        )
        .unwrap();
        assert_eq!(target_triple(&dir).unwrap(), "riscv32imafc-esp-espidf");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn target_triple_errors_without_an_espidf_target() {
        let dir = temp_dir("target-triple-missing");
        fs::create_dir_all(dir.join(".cargo")).unwrap();
        fs::write(dir.join(".cargo").join("config.toml"), "[build]\n").unwrap();
        assert!(target_triple(&dir).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn binary_name_prefers_the_bin_table_name_over_the_package_name() {
        let dir = temp_dir("binary-name-bin-table");
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"firmware\"\nversion = \"0.1.0\"\n\n[[bin]]\nname = \"firmware\"\nharness = false\n",
        )
        .unwrap();
        assert_eq!(binary_name(&dir).unwrap(), "firmware");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn binary_name_falls_back_to_the_package_name_with_no_bin_table() {
        let dir = temp_dir("binary-name-package-only");
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"my-firmware\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        assert_eq!(binary_name(&dir).unwrap(), "my-firmware");
        fs::remove_dir_all(&dir).unwrap();
    }

    // publish_artifacts / bootloader_path / firmware_elf_path ---------------

    #[test]
    fn bootloader_and_elf_paths_follow_the_esp_idf_release_layout() {
        let release_dir = Path::new("/src/target/riscv32imafc-esp-espidf/release");
        assert_eq!(
            bootloader_path(release_dir),
            Path::new("/src/target/riscv32imafc-esp-espidf/release/bootloader/bootloader.bin")
        );
        assert_eq!(
            firmware_elf_path(release_dir, "firmware"),
            Path::new("/src/target/riscv32imafc-esp-espidf/release/firmware")
        );
    }

    // A boundary test confirming GitHubReleaseClient is reachable and
    // constructible from this crate (Constitution I's "widened visibility"
    // note) — doesn't hit the network, just proves the type/re-export work.
    #[test]
    fn github_release_client_is_reachable_from_this_crate() {
        let _client = GitHubReleaseClient::new(
            "octo".to_string(),
            "repo".to_string(),
            "fake-token".to_string(),
        );
    }
}
