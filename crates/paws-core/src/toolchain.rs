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
}

/// Every toolchain `paws ci` can build. The single source of truth.
pub const TOOLCHAINS: &[ToolchainInfo] = &[
    ToolchainInfo {
        toolchain: Toolchain::Node,
        name: "node",
        markers: &["package.json"],
        provisions: Some("node"),
    },
    ToolchainInfo {
        toolchain: Toolchain::Rust,
        name: "rust",
        markers: &["Cargo.toml"],
        provisions: Some("rust"),
    },
    ToolchainInfo {
        toolchain: Toolchain::Python,
        name: "python",
        markers: &["pyproject.toml"],
        provisions: Some("python"),
    },
    ToolchainInfo {
        toolchain: Toolchain::Go,
        name: "go",
        markers: &["go.mod"],
        provisions: Some("go"),
    },
    ToolchainInfo {
        toolchain: Toolchain::Java,
        name: "java",
        // Both build systems `paws-java` detects. A repo with either is a
        // Java project as far as picking a CI step goes; `paws-java` decides
        // Maven vs Gradle for itself.
        markers: &["pom.xml", "build.gradle", "build.gradle.kts"],
        provisions: None,
    },
    ToolchainInfo {
        toolchain: Toolchain::Kotlin,
        name: "kotlin",
        // A Gradle build file alone doesn't make a Kotlin project — it takes
        // real `.kt` sources under it, which is a source walk, not a marker.
        markers: &[],
        provisions: None,
    },
    ToolchainInfo {
        toolchain: Toolchain::Ruby,
        name: "ruby",
        markers: &["Gemfile"],
        provisions: None,
    },
    ToolchainInfo {
        toolchain: Toolchain::Php,
        name: "php",
        markers: &["composer.json"],
        provisions: None,
    },
    ToolchainInfo {
        toolchain: Toolchain::Dotnet,
        name: "dotnet",
        // `*.csproj`/`*.sln` are globs, not filenames.
        markers: &[],
        provisions: None,
    },
    ToolchainInfo {
        toolchain: Toolchain::Elixir,
        name: "elixir",
        markers: &["mix.exs"],
        provisions: None,
    },
    ToolchainInfo {
        toolchain: Toolchain::Tauri,
        name: "tauri",
        // A Tauri repo is also a Node repo; marker-based detection would
        // report both and generate two CI steps for one project.
        markers: &[],
        provisions: Some("node"),
    },
    ToolchainInfo {
        toolchain: Toolchain::TauriAndroid,
        name: "tauri-android",
        markers: &[],
        provisions: Some("node"),
    },
    ToolchainInfo {
        toolchain: Toolchain::Flatpak,
        name: "flatpak",
        // The manifest can sit at the root or under `packaging/flatpak/`,
        // and is matched by app-id shape, not by name.
        markers: &[],
        provisions: None,
    },
    ToolchainInfo {
        toolchain: Toolchain::Esp32,
        name: "esp32",
        // Shares `Cargo.toml` with `rust`; told apart by an
        // `esp-idf-sys`/`esp-idf-svc` dependency or an `*-espidf` target.
        markers: &[],
        provisions: Some("esp32"),
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
