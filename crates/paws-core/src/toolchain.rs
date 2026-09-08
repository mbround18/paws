//! The one place that knows which toolchains `paws` supports.
//!
//! Before this module, "which languages does paws build?" was answered
//! independently in four places — `paws ci`'s dispatch match, the
//! provisioning marker table, `paws workflow generate`'s step list, and a
//! hand-written error string — and three of them had already drifted apart
//! (14 toolchains dispatched, 4 auto-provisioned, 3 workflow steps emitted).
//! Every one of those now reads [`TOOLCHAINS`], so adding a toolchain is one
//! entry rather than four edits and a good memory.
//!
//! `paws-provision`'s `Ecosystem` and `paws-audit`'s `LanguageFamily` stay
//! separate types on purpose — they answer narrower questions (what can be
//! *installed*, what can be *scanned*) over sets that are neither a subset
//! nor a superset of this one — but both derive their mapping from here, and
//! each has a test that fails if this table names something they don't know.

use serde::{Deserialize, Serialize};

use crate::version::{ResolvedVersion, VersionSource};

/// A `paws ci --toolchain <x>` value.
///
/// Ordering follows [`TOOLCHAINS`], which is the order `--help` and every
/// generated list presents them in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum, schemars::JsonSchema))]
pub enum Toolchain {
    Node,
    Rust,
    Python,
    Go,
    Java,
    Kotlin,
    Ruby,
    Php,
    Dotnet,
    Elixir,
    Tauri,
    TauriAndroid,
    Flatpak,
    Esp32,
}

/// What the rest of the workspace needs to know about a toolchain without
/// depending on the crate that implements it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolchainInfo {
    pub toolchain: Toolchain,
    /// The `--toolchain` value, and the name used in every message.
    pub name: &'static str,
    /// Files whose presence at a directory root identifies this toolchain by
    /// filename alone.
    ///
    /// Empty when detection needs real logic that lives in the toolchain's
    /// own crate — a dependency scan (`esp32`), a manifest search
    /// (`flatpak`), a source-file walk (`kotlin`, `dotnet`), or a check that
    /// only distinguishes a specialization of another toolchain (`tauri`,
    /// which is also always a Node project). Marker-free toolchains are
    /// dispatched by an explicit `--toolchain`, never guessed at.
    pub markers: &'static [&'static str],
    /// The `paws provision` ecosystem that installs this toolchain, when
    /// `paws-provision` has an installer for it. `None` is a real answer,
    /// not a gap: a JDK has no single obviously-right version manager the
    /// way `rustup`/`corepack`/`uv` do (see `docs/ROADMAP.md`).
    pub provisions: Option<&'static str>,
    /// Version files this ecosystem already uses, highest precedence first.
    ///
    /// These are other tools' files — `rustup` reads `rust-toolchain.toml`,
    /// `nvm` reads `.nvmrc` — and `paws` reads them so `paws ci` builds
    /// against the same toolchain a local build would. Empty means the
    /// ecosystem has no such convention, and the version comes from
    /// `paws.toml` or the default.
    pub version_files: &'static [VersionSource],
    /// The version used when nothing else names one, and the tag that goes
    /// into [`ToolchainInfo::image`].
    pub default_version: &'static str,
    /// The container image, with `{version}` where the tag varies.
    ///
    /// `None` for toolchains that build against a `builders/*` Dockerfile
    /// rather than a pulled image — their version is baked into that
    /// Dockerfile, so it is not a knob this table can turn.
    pub image_template: Option<&'static str>,
}

/// Every toolchain `paws ci` can build. The single source of truth.
pub const TOOLCHAINS: &[ToolchainInfo] = &[
    ToolchainInfo {
        toolchain: Toolchain::Node,
        name: "node",
        markers: &["package.json"],
        provisions: Some("node"),
        version_files: &[
            VersionSource::Bare(".nvmrc"),
            VersionSource::Bare(".node-version"),
            VersionSource::ToolVersions("nodejs"),
        ],
        default_version: "22",
        image_template: None,
    },
    ToolchainInfo {
        toolchain: Toolchain::Rust,
        name: "rust",
        markers: &["Cargo.toml"],
        provisions: Some("rust"),
        version_files: &[
            VersionSource::RustToolchain("rust-toolchain.toml"),
            VersionSource::RustToolchain("rust-toolchain"),
            VersionSource::ToolVersions("rust"),
        ],
        default_version: "1",
        image_template: Some("rust:{version}-bookworm"),
    },
    ToolchainInfo {
        toolchain: Toolchain::Python,
        name: "python",
        markers: &["pyproject.toml"],
        provisions: Some("python"),
        version_files: &[
            VersionSource::Bare(".python-version"),
            VersionSource::ToolVersions("python"),
        ],
        default_version: "3.13",
        image_template: Some("astral/uv:python{version}-trixie-slim"),
    },
    ToolchainInfo {
        toolchain: Toolchain::Go,
        name: "go",
        markers: &["go.mod"],
        provisions: Some("go"),
        version_files: &[
            VersionSource::GoDirective("go.mod"),
            VersionSource::Bare(".go-version"),
            VersionSource::ToolVersions("golang"),
        ],
        default_version: "1",
        image_template: Some("golang:{version}-bookworm"),
    },
    ToolchainInfo {
        toolchain: Toolchain::Java,
        name: "java",
        // Both build systems `paws-java` detects. A repo with either is a
        // Java project as far as picking a CI step goes; `paws-java` decides
        // Maven vs Gradle for itself.
        markers: &["pom.xml", "build.gradle", "build.gradle.kts"],
        provisions: None,
        version_files: &[
            VersionSource::Bare(".java-version"),
            VersionSource::ToolVersions("java"),
        ],
        default_version: "21",
        image_template: None,
    },
    ToolchainInfo {
        toolchain: Toolchain::Kotlin,
        name: "kotlin",
        // A Gradle build file alone doesn't make a Kotlin project — it takes
        // real `.kt` sources under it, which is a source walk, not a marker.
        markers: &[],
        provisions: None,
        version_files: &[VersionSource::Bare(".java-version")],
        default_version: "21",
        image_template: None,
    },
    ToolchainInfo {
        toolchain: Toolchain::Ruby,
        name: "ruby",
        markers: &["Gemfile"],
        provisions: None,
        version_files: &[
            VersionSource::Bare(".ruby-version"),
            VersionSource::ToolVersions("ruby"),
        ],
        default_version: "trixie",
        image_template: Some("ruby:{version}"),
    },
    ToolchainInfo {
        toolchain: Toolchain::Php,
        name: "php",
        markers: &["composer.json"],
        provisions: None,
        version_files: &[
            VersionSource::Bare(".php-version"),
            VersionSource::ToolVersions("php"),
        ],
        default_version: "2",
        image_template: Some("composer:{version}"),
    },
    ToolchainInfo {
        toolchain: Toolchain::Dotnet,
        name: "dotnet",
        // `*.csproj`/`*.sln` are globs, not filenames.
        markers: &[],
        provisions: None,
        version_files: &[
            VersionSource::Bare(".dotnet-version"),
            VersionSource::ToolVersions("dotnet"),
        ],
        default_version: "10.0",
        image_template: Some("mcr.microsoft.com/dotnet/sdk:{version}"),
    },
    ToolchainInfo {
        toolchain: Toolchain::Elixir,
        name: "elixir",
        markers: &["mix.exs"],
        provisions: None,
        version_files: &[
            VersionSource::Bare(".exenv-version"),
            VersionSource::ToolVersions("elixir"),
        ],
        default_version: "otp-28",
        image_template: Some("elixir:{version}"),
    },
    ToolchainInfo {
        toolchain: Toolchain::Tauri,
        name: "tauri",
        // A Tauri repo is also a Node repo; marker-based detection would
        // report both and generate two CI steps for one project.
        markers: &[],
        provisions: Some("node"),
        version_files: &[
            VersionSource::Bare(".nvmrc"),
            VersionSource::Bare(".node-version"),
        ],
        default_version: "22",
        image_template: None,
    },
    ToolchainInfo {
        toolchain: Toolchain::TauriAndroid,
        name: "tauri-android",
        markers: &[],
        provisions: Some("node"),
        version_files: &[
            VersionSource::Bare(".nvmrc"),
            VersionSource::Bare(".node-version"),
        ],
        default_version: "22",
        image_template: None,
    },
    ToolchainInfo {
        toolchain: Toolchain::Flatpak,
        name: "flatpak",
        // The manifest can sit at the root or under `packaging/flatpak/`,
        // and is matched by app-id shape, not by name.
        markers: &[],
        provisions: None,
        version_files: &[],
        default_version: "latest",
        image_template: None,
    },
    ToolchainInfo {
        toolchain: Toolchain::Esp32,
        name: "esp32",
        // Shares `Cargo.toml` with `rust`; told apart by an
        // `esp-idf-sys`/`esp-idf-svc` dependency or an `*-espidf` target.
        markers: &[],
        provisions: Some("esp32"),
        version_files: &[
            VersionSource::RustToolchain("rust-toolchain.toml"),
            VersionSource::RustToolchain("rust-toolchain"),
        ],
        default_version: "esp",
        image_template: None,
    },
];

impl Toolchain {
    /// Every toolchain, in `TOOLCHAINS` order.
    pub fn all() -> impl Iterator<Item = Self> {
        TOOLCHAINS.iter().map(|info| info.toolchain)
    }

    pub fn info(&self) -> &'static ToolchainInfo {
        TOOLCHAINS
            .iter()
            .find(|info| info.toolchain == *self)
            .expect("every Toolchain variant has a TOOLCHAINS entry (asserted by test)")
    }

    pub fn as_str(&self) -> &'static str {
        self.info().name
    }

    /// Files that identify this toolchain by filename alone; empty when
    /// detection needs the toolchain crate's own logic.
    pub fn markers(&self) -> &'static [&'static str] {
        self.info().markers
    }

    /// The `paws provision` ecosystem that installs this toolchain, if any.
    pub fn provisions(&self) -> Option<&'static str> {
        self.info().provisions
    }

    /// Resolves which version of this toolchain to build against.
    ///
    /// Precedence is `flag > version file > paws.toml > default` — see
    /// [`crate::version`] for why a native version file outranks paws's own
    /// config.
    pub fn resolve_version(
        &self,
        dir: &std::path::Path,
        flag: Option<&str>,
        configured: Option<&str>,
    ) -> ResolvedVersion {
        crate::version::resolve(
            dir,
            flag,
            self.info().version_files,
            configured,
            self.info().default_version,
        )
    }

    /// The container image to build against for `version`, or `None` for a
    /// toolchain that builds from a `builders/*` Dockerfile instead.
    ///
    /// Rust channel names get translated: `rust-toolchain.toml` legitimately
    /// says `stable`, but there is no `rust:stable-bookworm` tag on Docker
    /// Hub — the moving tag is `rust:1-bookworm`. Passing the channel through
    /// verbatim would produce a pull failure that reads like a network error
    /// rather than a version mismatch.
    // `{version}` in an image template is a placeholder this function
    // substitutes, not a `format!` argument — the lint cannot tell the
    // difference.
    #[allow(clippy::literal_string_with_formatting_args)]
    pub fn image_for(self, version: &str) -> Option<String> {
        let template = self.info().image_template?;
        let tag = self.image_tag_for(version);
        Some(template.replace("{version}", &tag))
    }

    /// Maps a resolved version onto the tag its image registry actually
    /// publishes.
    fn image_tag_for(self, version: &str) -> String {
        match self {
            Self::Rust | Self::Esp32 => match version {
                // `rustup`'s moving channels have no same-named image tag.
                // `1` is Docker Hub's equivalent moving tag for stable.
                "stable" | "latest" => "1".to_string(),
                // A dated nightly (`nightly-2026-01-01`) has no image at all;
                // fall back to the moving stable tag rather than 404.
                v if v.starts_with("nightly") || v.starts_with("beta") => "1".to_string(),
                v => v.to_string(),
            },
            _ => version.to_string(),
        }
    }

    /// The toolchains a `--toolchain` value may name, rendered the way an
    /// error message wants them: `'node', 'rust', ... or 'esp32'`.
    ///
    /// Built from [`TOOLCHAINS`] rather than typed out, so the list in the
    /// error can't fall behind the list the match actually accepts.
    pub fn expected_values() -> String {
        let names: Vec<String> = TOOLCHAINS
            .iter()
            .map(|info| format!("'{}'", info.name))
            .collect();
        match names.split_last() {
            Some((last, rest)) if !rest.is_empty() => format!("{}, or {last}", rest.join(", ")),
            _ => names.join(", "),
        }
    }
}

impl std::fmt::Display for Toolchain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Toolchain {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<Self> {
        TOOLCHAINS
            .iter()
            .find(|info| info.name == s)
            .map(|info| info.toolchain)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "unsupported --toolchain '{s}'; expected {}",
                    Self::expected_values()
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    /// The `expect` in `Toolchain::info` is only safe while every variant has
    /// an entry. A new variant with no `TOOLCHAINS` row fails here rather
    /// than panicking at run time.
    #[test]
    fn every_variant_has_a_registry_entry() {
        for toolchain in [
            Toolchain::Node,
            Toolchain::Rust,
            Toolchain::Python,
            Toolchain::Go,
            Toolchain::Java,
            Toolchain::Kotlin,
            Toolchain::Ruby,
            Toolchain::Php,
            Toolchain::Dotnet,
            Toolchain::Elixir,
            Toolchain::Tauri,
            Toolchain::TauriAndroid,
            Toolchain::Flatpak,
            Toolchain::Esp32,
        ] {
            assert_eq!(toolchain.info().toolchain, toolchain);
        }
    }

    #[test]
    fn names_are_unique_and_round_trip_through_from_str() {
        let mut seen = std::collections::HashSet::new();
        for info in TOOLCHAINS {
            assert!(
                seen.insert(info.name),
                "duplicate toolchain name {}",
                info.name
            );
            assert_eq!(Toolchain::from_str(info.name).unwrap(), info.toolchain);
        }
    }

    #[test]
    fn an_unknown_name_lists_every_supported_value() {
        let error = Toolchain::from_str("rusty").unwrap_err().to_string();
        for info in TOOLCHAINS {
            assert!(
                error.contains(&format!("'{}'", info.name)),
                "error should name {}, got: {error}",
                info.name
            );
        }
    }

    /// The marker table drives both provisioning detection and workflow
    /// generation, so a marker that belongs to two toolchains would make a
    /// repo detect as both.
    #[test]
    fn no_marker_is_claimed_by_two_toolchains() {
        let mut owner: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
        for info in TOOLCHAINS {
            for marker in info.markers {
                if let Some(previous) = owner.insert(marker, info.name) {
                    panic!("{marker} is claimed by both {previous} and {}", info.name);
                }
            }
        }
    }

    #[test]
    fn every_toolchain_names_a_default_version() {
        for info in TOOLCHAINS {
            assert!(
                !info.default_version.is_empty(),
                "{} has no default version",
                info.name
            );
        }
    }

    /// A template that never substitutes would silently build every project
    /// against one hard-coded tag, which is the bug this table replaced.
    #[test]
    fn every_image_template_actually_uses_the_version() {
        for info in TOOLCHAINS {
            let Some(template) = info.image_template else {
                continue;
            };
            assert!(
                template.contains("{version}"),
                "{}'s image template ignores the resolved version: {template}",
                info.name
            );
            let rendered = info.toolchain.image_for("9.9.9").unwrap();
            assert!(
                rendered.contains("9.9.9"),
                "{} did not substitute the version: {rendered}",
                info.name
            );
        }
    }

    /// `rust-toolchain.toml` legitimately says `stable`, but `rust:stable-*`
    /// is not a tag anyone publishes.
    #[test]
    fn rust_channel_names_map_onto_tags_that_exist() {
        assert_eq!(
            Toolchain::Rust.image_for("stable").unwrap(),
            "rust:1-bookworm"
        );
        assert_eq!(
            Toolchain::Rust.image_for("nightly-2026-01-01").unwrap(),
            "rust:1-bookworm"
        );
        assert_eq!(
            Toolchain::Rust.image_for("1.90.0").unwrap(),
            "rust:1.90.0-bookworm"
        );
    }

    /// The toolchains that build from `builders/*` have their version baked
    /// into the Dockerfile, so this table must not pretend otherwise.
    #[test]
    fn builder_backed_toolchains_expose_no_image() {
        for toolchain in [
            Toolchain::Java,
            Toolchain::Kotlin,
            Toolchain::Tauri,
            Toolchain::Flatpak,
            Toolchain::Esp32,
        ] {
            assert!(
                toolchain.image_for("1.0").is_none(),
                "{toolchain} builds from a Dockerfile and has no pullable image"
            );
        }
    }

    /// Reading the repo's own version file is the whole point.
    #[test]
    fn a_rust_toolchain_file_wins_over_the_built_in_default() {
        let dir = crate::test_support::scratch_dir("toolchain", "rust-version-file");
        std::fs::write(
            dir.join("rust-toolchain.toml"),
            "[toolchain]\nchannel = \"1.90.0\"\n",
        )
        .unwrap();

        let resolved = Toolchain::Rust.resolve_version(&dir, None, None);
        assert_eq!(resolved.version, "1.90.0");
        assert_eq!(
            Toolchain::Rust.image_for(&resolved.version).unwrap(),
            "rust:1.90.0-bookworm"
        );

        // ...and an explicit flag still wins over the file.
        let flagged = Toolchain::Rust.resolve_version(&dir, Some("1.88.0"), None);
        assert_eq!(flagged.version, "1.88.0");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn expected_values_reads_as_a_sentence() {
        let rendered = Toolchain::expected_values();
        assert!(rendered.starts_with("'node', 'rust',"));
        assert!(rendered.ends_with("or 'esp32'"));
    }

    #[test]
    fn serde_uses_the_cli_spelling() {
        let json = serde_json::to_string(&Toolchain::TauriAndroid).unwrap();
        assert_eq!(json, "\"tauri-android\"");
        let parsed: Toolchain = serde_json::from_str("\"tauri-android\"").unwrap();
        assert_eq!(parsed, Toolchain::TauriAndroid);
        assert_eq!(parsed.as_str(), "tauri-android");
    }
}
