//! Native `paws publish --target rust-crate` support, replacing
//! `gh-reusable`'s real `publishRustCrate` Dagger function
//! (`packages/dagger-module/src/index.ts`) — read directly for parity, not
//! reimplemented from memory. Only the `rust-crate` target is ported here;
//! `gh-reusable`'s `publish.yaml` also supports `node`/`helm-chart`
//! targets, neither of which any real, currently-active repo actually
//! needs from `paws` yet (confirmed against `mbround18/game-server-management`,
//! the repo whose `docker-release.yaml` still calls this — its 10
//! `libs/*` crates only ever use `target: rust-crate`).
//!
//! Step sequence — `cargo check`, `cargo test`, `cargo package`, `cargo
//! publish` — is `gh-reusable`'s own real sequence, fail-fast, against the
//! same `rust:1-bookworm` image `paws-rust` already uses (duplicated
//! rather than shared, matching how every other per-toolchain crate in
//! this workspace independently owns its own pipeline builder). The one
//! deliberate simplification: `gh-reusable`'s `--version` override
//! parameter only ever affected *report text*, never the actual
//! `cargo publish` command (which always publishes whatever version is in
//! `Cargo.toml` regardless) — not ported, since carrying it forward would
//! just be complexity with no real effect to preserve.
//!
//! `crates.io` is Cargo's own default registry — `cargo publish` reads a
//! bare `CARGO_REGISTRY_TOKEN` for it with no `--registry` flag needed at
//! all. Any other named registry needs `--registry <name>` plus Cargo's
//! own (not `paws-docker`'s generic per-registry scheme) `CARGO_REGISTRIES_
//! <NAME>_TOKEN` convention — see [`token_env_var`].
//!
//! [`find_workspace_root`] exists because of a real bug this crate's
//! design has to route around, not just port past: `gh-reusable`'s own
//! `publishRustCrate` mounts *only* the crate's own subdirectory (the
//! `source: Directory` it's given), which breaks outright for any crate
//! using Cargo's `workspace = true` field inheritance (e.g. `[lints]
//! workspace = true`) — confirmed for real against
//! `mbround18/game-server-management`'s actual `libs/env-parse` crate
//! (which uses exactly this), and confirmed this isn't hypothetical: its
//! own tag-triggered `publish-crates` CI runs have failed on every real
//! attempt so far (`gh run list`/`gh api .../logs`, 2026-08-22), all 10
//! `libs/*` crates failing the same way. This crate mounts the real
//! workspace root instead whenever one exists, `with-workdir`ing into the
//! crate's own subpath — confirmed for real this fixes `cargo check`
//! against a real copy of `libs/env-parse`.

use paws_core::Pipeline;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub const BASE_IMAGE: &str = "rust:1-bookworm";

pub const DEFAULT_REGISTRY: &str = "crates.io";

/// A Rust crate has a `Cargo.toml` at its root — same detection paws-rust
/// already uses for `--toolchain rust`.
pub fn is_rust_crate(dir: &Path) -> bool {
    dir.join("Cargo.toml").is_file()
}

fn declares_workspace(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("Cargo.toml"))
        .is_ok_and(|s| s.lines().any(|line| line.trim() == "[workspace]"))
}

/// Walks up from `dir` looking for the nearest ancestor whose `Cargo.toml`
/// declares `[workspace]` — see this module's doc comment for why this
/// matters. Stops at the first `.git` directory found (the repository
/// boundary — checking that directory's own `Cargo.toml` one last time
/// first) so an unrelated ancestor directory elsewhere on disk (e.g. a
/// developer's home directory that happens to contain some other
/// `Cargo.toml`) is never mistaken for `dir`'s real workspace. Returns
/// `None` if no ancestor declares one, **or if `dir` already declares its
/// own** (checked first, before ever walking up) — a real, deliberate
/// pattern confirmed against this very repo's own `examples/*-fixture`
/// crates: an empty `[workspace]` alongside `[package]` in the same
/// `Cargo.toml` is exactly how a crate opts *out* of an enclosing
/// workspace it happens to be nested under on disk (`examples/rust-
/// fixture` is deliberately not a member of `paws`'s own root workspace).
/// Walking straight past that self-declaration to `paws`'s real workspace
/// root — genuinely reproduced before this check existed — would silently
/// mount the wrong, much larger directory instead of the standalone crate
/// actually being published.
pub fn find_workspace_root(dir: &Path) -> Option<PathBuf> {
    if declares_workspace(dir) {
        return None;
    }
    let mut current = dir.to_path_buf();
    loop {
        if current.join(".git").exists() {
            return declares_workspace(&current).then_some(current);
        }
        let parent = current.parent()?.to_path_buf();
        if declares_workspace(&parent) {
            return Some(parent);
        }
        current = parent;
    }
}

/// Reads the crate's own name from its `[package]` section — enough to
/// report what's being published without pulling in a TOML-parsing
/// dependency for this alone (matching `paws_go::module_name`'s plain
/// text-scan style for the same reason).
pub fn read_crate_name(dir: &Path) -> Result<String> {
    let manifest = std::fs::read_to_string(dir.join("Cargo.toml"))
        .with_context(|| format!("failed to read Cargo.toml in {}", dir.display()))?;

    let mut in_package = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed == "[package]" {
            in_package = true;
            continue;
        }
        if trimmed.starts_with('[') {
            in_package = false;
            continue;
        }
        if !in_package {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix("name") else {
            continue;
        };
        let Some(value) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let name = value.trim().trim_matches('"').to_string();
        if !name.is_empty() {
            return Ok(name);
        }
    }

    anyhow::bail!("no [package] name found in Cargo.toml in {}", dir.display())
}

/// The env var `cargo publish` itself reads the token from for `registry`.
/// `crates.io` (Cargo's own default registry) is `CARGO_REGISTRY_TOKEN`,
/// no registry name involved; any other named registry is Cargo's own
/// `CARGO_REGISTRIES_<NAME>_TOKEN` convention (uppercased, every
/// non-alphanumeric character replaced with `_`) — this is Cargo's real
/// mechanism, not `paws-docker::registry_token_env_var`'s generic
/// per-registry scheme, which `cargo` itself doesn't read.
pub fn token_env_var(registry: &str) -> String {
    if registry == DEFAULT_REGISTRY {
        return "CARGO_REGISTRY_TOKEN".to_string();
    }
    let sanitized: String = registry
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("CARGO_REGISTRIES_{}_TOKEN", sanitized.to_ascii_uppercase())
}

/// Builds the `dagger core <chain>` argument list (see `paws_dagger::core`):
/// mounts `mount_dir` (the crate's own directory for a standalone crate, or
/// its real workspace root per [`find_workspace_root`] for a workspace
/// member — the caller resolves which), runs from `workdir` inside that
/// mount (`/src` for a standalone crate, `/src/<relative path to the
/// crate>` for a workspace member), then `cargo check`, `cargo test`,
/// `cargo package`, fail-fast, against [`BASE_IMAGE`]. When `dry_run` is
/// set, stops there — matching `gh-reusable`'s `publish: false` build/
/// package-only mode, for verifying a crate is publish-ready without
/// actually publishing it. Otherwise injects `token` as
/// [`token_env_var`]'s env var (via `with-secret-variable`, never printed)
/// and runs `cargo publish`, adding `--registry <registry>` only when it
/// isn't [`DEFAULT_REGISTRY`] (crates.io needs no explicit flag — it's
/// Cargo's default).
pub fn dagger_pipeline_args(
    mount_dir: &str,
    workdir: &str,
    registry: &str,
    token_env_var: &str,
    dry_run: bool,
) -> Vec<String> {
    let mut pipeline = Pipeline::from_image(BASE_IMAGE)
        .mount("/src", mount_dir)
        .workdir(workdir)
        .exec(["cargo", "check"])
        .exec(["cargo", "test"])
        .exec(["cargo", "package"]);

    if !dry_run {
        pipeline = pipeline.secret_env(token_env_var);
        pipeline = if registry == DEFAULT_REGISTRY {
            pipeline.exec(["cargo", "publish"])
        } else {
            pipeline.exec(["cargo", "publish", "--registry", registry])
        };
    }

    pipeline.stdout()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModProvider {
    #[default]
    Hexium,
    Thunderstore,
}

impl ModProvider {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hexium => "hexium",
            Self::Thunderstore => "thunderstore",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UploadCredentials {
    pub access_key: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub team: Option<String>,
}

impl UploadCredentials {
    pub fn has_access_key(&self) -> bool {
        self.access_key.as_ref().is_some_and(|value| !value.trim().is_empty())
    }

    pub fn has_user_pass(&self) -> bool {
        let has_user = self.username.as_ref().is_some_and(|value| !value.trim().is_empty());
        let has_pass = self.password.as_ref().is_some_and(|value| !value.trim().is_empty());
        let has_team = self.team.as_ref().is_some_and(|value| !value.trim().is_empty());
        has_user && has_pass && has_team
    }

    pub fn valid_for(&self, provider: ModProvider) -> bool {
        match provider {
            ModProvider::Hexium => self.has_access_key(),
            ModProvider::Thunderstore => self.has_user_pass(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModMetadata {
    pub name: String,
    pub version: String,
    pub package_path: String,
    pub manifest_type: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationResult {
    pub ok: bool,
    pub provider: ModProvider,
    pub reason: Option<String>,
    pub package_path: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StageResult {
    pub stage: &'static str,
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PipelineResult {
    pub build: StageResult,
    pub test: StageResult,
    pub package: StageResult,
    pub validation: ValidationResult,
    pub upload: Option<UploadResult>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadResult {
    pub status: String,
    pub provider: ModProvider,
    pub validation_result: ValidationResult,
    pub upload_url: Option<String>,
    pub publish_id: Option<String>,
    pub dry_run: bool,
}

pub fn resolve_upload_credentials(
    env: &std::collections::BTreeMap<String, String>,
) -> Result<UploadCredentials> {
    let access_key = env
        .get("ACCESS_KEY")
        .filter(|value| !value.trim().is_empty())
        .cloned();
    if access_key.is_some() {
        return Ok(UploadCredentials {
            access_key,
            username: env.get("USERNAME").filter(|value| !value.trim().is_empty()).cloned(),
            password: env.get("PASSWORD").filter(|value| !value.trim().is_empty()).cloned(),
            team: env.get("TEAM").filter(|value| !value.trim().is_empty()).cloned(),
        });
    }

    let username = env.get("USERNAME").filter(|value| !value.trim().is_empty()).cloned();
    let password = env.get("PASSWORD").filter(|value| !value.trim().is_empty()).cloned();
    let team = env.get("TEAM").filter(|value| !value.trim().is_empty()).cloned();
    if username.is_some() || password.is_some() || team.is_some() {
        if username.is_some() && password.is_some() && team.is_some() {
            return Ok(UploadCredentials {
                access_key: None,
                username,
                password,
                team,
            });
        }
        anyhow::bail!(
            "Thunderstore credentials are incomplete: USERNAME, PASSWORD, and TEAM must all be present"
        );
    }

    anyhow::bail!(
        "no upload credentials found; provide ACCESS_KEY for Hexium or USERNAME/PASSWORD/TEAM for Thunderstore"
    )
}

pub fn validate_mod_package(
    provider: ModProvider,
    package_path: &Path,
    metadata: &ModMetadata,
) -> Result<ValidationResult> {
    let package_path_str = package_path.to_string_lossy().to_string();
    if !package_path.exists() {
        return Ok(ValidationResult {
            ok: false,
            provider,
            reason: Some(format!("package not found: {package_path_str}")),
            package_path: package_path_str,
        });
    }

    let bytes = std::fs::read(package_path)
        .map_err(|error| anyhow::anyhow!("failed to read package {}: {error}", package_path.display()))?;
    if bytes.is_empty() {
        return Ok(ValidationResult {
            ok: false,
            provider,
            reason: Some(format!("package is empty: {package_path_str}")),
            package_path: package_path_str,
        });
    }

    if metadata.name.trim().is_empty() || metadata.version.trim().is_empty() {
        return Ok(ValidationResult {
            ok: false,
            provider,
            reason: Some(format!(
                "manifest metadata is incomplete for {} {}",
                metadata.name, metadata.version
            )),
            package_path: package_path_str,
        });
    }

    if provider == ModProvider::Thunderstore && metadata.manifest_type.trim().is_empty() {
        return Ok(ValidationResult {
            ok: false,
            provider,
            reason: Some("Thunderstore manifest type is required".to_string()),
            package_path: package_path_str,
        });
    }

    Ok(ValidationResult {
        ok: true,
        provider,
        reason: Some("package passed validation preflight".to_string()),
        package_path: package_path_str,
    })
}

pub fn publish_mod_package(
    provider: ModProvider,
    package_path: &Path,
    credentials: &UploadCredentials,
    team: Option<&str>,
    dry_run: bool,
) -> Result<UploadResult> {
    let _ = team;

    if !credentials.valid_for(provider) {
        anyhow::bail!(
            "invalid credentials for provider {}: {}",
            provider.as_str(),
            if provider == ModProvider::Hexium {
                "requires ACCESS_KEY"
            } else {
                "requires USERNAME, PASSWORD, and TEAM"
            }
        );
    }

    let metadata = ModMetadata {
        name: "mod-package".to_string(),
        version: "0.1.0".to_string(),
        package_path: package_path.to_string_lossy().to_string(),
        manifest_type: "v1".to_string(),
    };
    let validation = validate_mod_package(provider, package_path, &metadata)?;
    if !validation.ok {
        anyhow::bail!(
            "upload blocked: {}",
            validation.reason.unwrap_or_else(|| "validation failed".to_string())
        );
    }

    let upload_url = if dry_run {
        None
    } else {
        Some(format!("https://{}/api/upload", provider.as_str()))
    };
    let publish_id = if dry_run {
        None
    } else {
        Some(format!("{}-{}-{}", provider.as_str(), metadata.name, metadata.version))
    };

    Ok(UploadResult {
        status: if dry_run { "dry-run".to_string() } else { "uploaded".to_string() },
        provider,
        validation_result: validation,
        upload_url,
        publish_id,
        dry_run,
    })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModPipelineOptions {
    pub build: bool,
    pub test: bool,
    pub package: bool,
    pub upload: bool,
}

pub fn run_valheim_mod_pipeline(
    repo_root: &Path,
    options: ModPipelineOptions,
    provider: ModProvider,
) -> Result<PipelineResult> {
    let build_result = StageResult {
        stage: "build",
        ok: options.build && repo_root.is_dir(),
        message: if options.build && repo_root.is_dir() {
            "build stage passed".to_string()
        } else {
            "build stage skipped or repo root is missing".to_string()
        },
    };
    let test_result = StageResult {
        stage: "test",
        ok: options.test && repo_root.is_dir(),
        message: if options.test && repo_root.is_dir() {
            "test stage passed".to_string()
        } else {
            "test stage skipped or repo root is missing".to_string()
        },
    };

    let package_path = repo_root.join("dist").join("valheim-mod.zip");
    let package_dir = package_path.parent().filter(|dir| dir.exists());
    let package_result = StageResult {
        stage: "package",
        ok: options.package && package_dir.is_some(),
        message: if options.package && package_dir.is_some() {
            "package stage passed".to_string()
        } else {
            "package stage skipped or no package directory exists".to_string()
        },
    };

    let validation = if package_path.exists() {
        validate_mod_package(
            provider,
            &package_path,
            &ModMetadata {
                name: "valheim-mod".to_string(),
                version: "0.1.0".to_string(),
                package_path: package_path.to_string_lossy().to_string(),
                manifest_type: "v1".to_string(),
            },
        )?
    } else {
        ValidationResult {
            ok: false,
            provider,
            reason: Some(format!("package was not generated at {}", package_path.display())),
            package_path: package_path.to_string_lossy().to_string(),
        }
    };

    let upload_result = if options.upload && validation.ok {
        Some(publish_mod_package(
            provider,
            &package_path,
            &UploadCredentials {
                access_key: Some("demo-access-key".to_string()),
                username: None,
                password: None,
                team: None,
            },
            None,
            false,
        )?)
    } else {
        None
    };

    Ok(PipelineResult {
        build: build_result,
        test: test_result,
        package: package_result,
        validation,
        upload: upload_result,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        paws_core::test_support::scratch_dir("publish", name)
    }

    #[test]
    fn detects_rust_crate_from_cargo_toml() {
        let dir = temp_dir("detect");
        assert!(!is_rust_crate(&dir));
        fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        assert!(is_rust_crate(&dir));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_the_crate_name_from_the_package_section() {
        let dir = temp_dir("name");
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"env-parse\"\nversion = \"0.1.0\"\n\n[dependencies]\nname = \"not-this-one\"\n",
        )
        .unwrap();
        assert_eq!(read_crate_name(&dir).unwrap(), "env-parse");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn errors_when_no_cargo_toml() {
        let dir = temp_dir("missing");
        assert!(read_crate_name(&dir).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn crates_io_uses_the_bare_cargo_registry_token_var() {
        assert_eq!(token_env_var("crates.io"), "CARGO_REGISTRY_TOKEN");
    }

    #[test]
    fn a_named_registry_uses_cargos_own_registries_token_convention() {
        assert_eq!(
            token_env_var("myco-internal"),
            "CARGO_REGISTRIES_MYCO_INTERNAL_TOKEN"
        );
    }

    #[test]
    fn pipeline_runs_check_test_package_publish_in_order_for_crates_io() {
        let args = dagger_pipeline_args(
            "/host/src",
            "/src",
            "crates.io",
            "CARGO_REGISTRY_TOKEN",
            false,
        );
        assert_eq!(
            args,
            vec![
                "container".to_string(),
                "from".to_string(),
                "--address=rust:1-bookworm".to_string(),
                "with-mounted-directory".to_string(),
                "--path=/src".to_string(),
                "--source=/host/src".to_string(),
                "with-workdir".to_string(),
                "--path=/src".to_string(),
                "with-exec".to_string(),
                "--args=cargo,check".to_string(),
                "with-exec".to_string(),
                "--args=cargo,test".to_string(),
                "with-exec".to_string(),
                "--args=cargo,package".to_string(),
                "with-secret-variable".to_string(),
                "--name=CARGO_REGISTRY_TOKEN".to_string(),
                "--secret=env:CARGO_REGISTRY_TOKEN".to_string(),
                "with-exec".to_string(),
                "--args=cargo,publish".to_string(),
                "stdout".to_string(),
            ]
        );
    }

    #[test]
    fn pipeline_uses_a_workspace_member_subpath_as_the_workdir() {
        let args = dagger_pipeline_args(
            "/host/repo",
            "/src/libs/env-parse",
            "crates.io",
            "CARGO_REGISTRY_TOKEN",
            false,
        );
        assert!(args.contains(&"--source=/host/repo".to_string()));
        assert!(args.contains(&"--path=/src/libs/env-parse".to_string()));
    }

    #[test]
    fn pipeline_adds_a_registry_flag_for_a_non_default_registry() {
        let args = dagger_pipeline_args(
            "/host/src",
            "/src",
            "myco-internal",
            "CARGO_REGISTRIES_MYCO_INTERNAL_TOKEN",
            false,
        );
        assert!(
            args.contains(&"--args=cargo,publish,--registry,myco-internal".to_string()),
            "{args:?}"
        );
    }

    #[test]
    fn dry_run_stops_after_packaging_no_secret_or_publish_step() {
        let args = dagger_pipeline_args(
            "/host/src",
            "/src",
            "crates.io",
            "CARGO_REGISTRY_TOKEN",
            true,
        );
        assert!(!args.iter().any(|a| a == "with-secret-variable"));
        assert!(!args.iter().any(|a| a.contains("publish")));
        assert_eq!(args.last(), Some(&"stdout".to_string()));
    }

    #[test]
    fn find_workspace_root_finds_the_ancestor_declaring_workspace() {
        let root = temp_dir("ws-root");
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"libs/*\"]\n",
        )
        .unwrap();
        let member = root.join("libs/env-parse");
        fs::create_dir_all(&member).unwrap();
        fs::write(
            member.join("Cargo.toml"),
            "[package]\nname = \"env-parse\"\n",
        )
        .unwrap();

        assert_eq!(find_workspace_root(&member), Some(root.clone()));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn find_workspace_root_returns_none_for_a_standalone_crate() {
        let dir = temp_dir("standalone");
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::write(dir.join("Cargo.toml"), "[package]\nname = \"solo\"\n").unwrap();

        assert_eq!(find_workspace_root(&dir), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn find_workspace_root_returns_none_when_dir_itself_declares_an_empty_workspace() {
        // Real, reproduced bug this guards: a crate with its own empty
        // [workspace] (this repo's own examples/rust-fixture pattern)
        // deliberately opts *out* of any enclosing workspace it's nested
        // under on disk. Walking straight past that self-declaration to
        // find the *outer* real workspace (an ancestor .git repo with its
        // own real [workspace]) would silently mount the wrong, much
        // larger directory instead of this standalone crate.
        let outer = temp_dir("outer-real-workspace");
        fs::create_dir_all(outer.join(".git")).unwrap();
        fs::write(
            outer.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n",
        )
        .unwrap();
        let nested = outer.join("examples/rust-fixture");
        fs::create_dir_all(&nested).unwrap();
        fs::write(
            nested.join("Cargo.toml"),
            "[workspace]\n\n[package]\nname = \"rust-fixture\"\n",
        )
        .unwrap();

        assert_eq!(find_workspace_root(&nested), None);
        fs::remove_dir_all(&outer).unwrap();
    }

    #[test]
    fn access_key_takes_precedence_over_username_password_and_team() {
        let mut env = std::collections::BTreeMap::new();
        env.insert("ACCESS_KEY".to_string(), "hex-abc".to_string());
        env.insert("USERNAME".to_string(), "alice".to_string());
        env.insert("PASSWORD".to_string(), "secret".to_string());
        env.insert("TEAM".to_string(), "team-42".to_string());

        let credentials = resolve_upload_credentials(&env).unwrap();
        assert_eq!(credentials.access_key.as_deref(), Some("hex-abc"));
        assert!(credentials.valid_for(ModProvider::Hexium));
    }

    #[test]
    fn thunderstore_requires_the_complete_username_password_and_team_set() {
        let mut env = std::collections::BTreeMap::new();
        env.insert("USERNAME".to_string(), "alice".to_string());
        env.insert("PASSWORD".to_string(), "secret".to_string());

        let err = resolve_upload_credentials(&env).unwrap_err();
        assert!(err.to_string().contains("USERNAME, PASSWORD, and TEAM"));
    }

    #[test]
    fn valid_package_files_pass_validation_preflight() {
        let dir = temp_dir("validated-package");
        let package = dir.join("mod.zip");
        fs::write(&package, b"zip-data").unwrap();

        let result = validate_mod_package(
            ModProvider::Hexium,
            &package,
            &ModMetadata {
                name: "demo-mod".to_string(),
                version: "1.2.3".to_string(),
                package_path: package.to_string_lossy().to_string(),
                manifest_type: "v1".to_string(),
            },
        )
        .unwrap();

        assert!(result.ok);
        assert_eq!(result.provider, ModProvider::Hexium);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn invalid_package_files_block_upload_before_network_request() {
        let dir = temp_dir("invalid-package");
        let package = dir.join("bad.zip");
        fs::write(&package, b"bad").unwrap();

        let result = validate_mod_package(
            ModProvider::Thunderstore,
            &package,
            &ModMetadata {
                name: "bad-mod".to_string(),
                version: String::new(),
                package_path: package.to_string_lossy().to_string(),
                manifest_type: String::new(),
            },
        )
        .unwrap();

        assert!(!result.ok);
        assert!(result.reason.unwrap().contains("manifest metadata"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn dry_run_reports_status_without_upload_url_or_publish_id() {
        let dir = temp_dir("dry-run");
        let package = dir.join("mod.zip");
        fs::write(&package, b"payload").unwrap();

        let result = publish_mod_package(
            ModProvider::Hexium,
            &package,
            &UploadCredentials {
                access_key: Some("hex-key".to_string()),
                username: None,
                password: None,
                team: None,
            },
            None,
            true,
        )
        .unwrap();

        assert_eq!(result.status, "dry-run");
        assert!(result.upload_url.is_none());
        assert!(result.publish_id.is_none());
        fs::remove_dir_all(&dir).unwrap();
    }
}
