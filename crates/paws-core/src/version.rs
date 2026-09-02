//! Resolving which version of a toolchain or tool to use.
//!
//! Before this module, version control was three unrelated mechanisms and a
//! lot of nothing: `paws-python`/`paws-dotnet` took a version parameter that
//! no CLI flag could reach, `paws-provision` read a one-off `$PAWS_GO_VERSION`,
//! and every other toolchain hard-coded its image tag. Nothing read the
//! version files repos already have — a `rust-toolchain.toml` sitting next to
//! `Cargo.toml` was ignored, so `paws ci` could build against a different
//! compiler than `cargo build` on the same machine.
//!
//! ## Precedence
//!
//! Highest wins:
//!
//! 1. **An explicit flag** (`--toolchain-version`) — the caller said so.
//! 2. **The ecosystem's own version file** (`rust-toolchain.toml`, `.nvmrc`,
//!    `.python-version`, …). This deliberately outranks paws's own config:
//!    `rustup` and `nvm` already obey these files, so if `paws` disagreed,
//!    `paws ci` would build against a different toolchain than a local
//!    `cargo build` in the same directory. Agreeing with the repo's existing
//!    tools matters more than agreeing with paws's config.
//! 3. **`paws.toml`** — a repo-wide pin for toolchains that have no native
//!    version file, and for paws's own tools.
//! 4. **The built-in default**.

use std::path::{Path, PathBuf};

/// Where a resolved version came from. Surfaced in `paws ci`'s output so a
/// surprising toolchain is traceable without re-running with more flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionOrigin {
    /// An explicit `--toolchain-version`/`--tool-version` flag.
    Flag,
    /// The ecosystem's own version file, e.g. `rust-toolchain.toml`.
    VersionFile(PathBuf),
    /// A `[toolchains]`/`[tools]` entry in `paws.toml`.
    Config,
    /// The compiled-in default.
    Default,
}

impl std::fmt::Display for VersionOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Flag => f.write_str("--toolchain-version"),
            Self::VersionFile(path) => write!(f, "{}", path.display()),
            Self::Config => f.write_str("paws.toml"),
            Self::Default => f.write_str("default"),
        }
    }
}

/// A resolved version and the reason it was chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedVersion {
    pub version: String,
    pub origin: VersionOrigin,
}

impl ResolvedVersion {
    /// `"1.90.0 (rust-toolchain.toml)"` — what `paws ci` prints, so the
    /// toolchain in use and the reason for it are one line in the log.
    pub fn describe(&self) -> String {
        match &self.origin {
            VersionOrigin::Default => self.version.clone(),
            origin => format!("{} ({origin})", self.version),
        }
    }
}

/// How to read a version out of one file. Each ecosystem writes its pin
/// differently, and none of them are TOML-with-a-version-key except Rust's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionSource {
    /// The whole file is the version, trimmed: `.nvmrc`, `.python-version`,
    /// `.ruby-version`, `.java-version`. A leading `v` or `<name>-` prefix is
    /// stripped, since `.nvmrc` conventionally holds `v20.11.0` and
    /// `.ruby-version` holds `ruby-3.3.0`.
    ///
    /// Only the first non-empty, non-comment line is read: `pyenv` allows
    /// several versions in `.python-version` and the first is the primary.
    Bare(&'static str),
    /// `rust-toolchain.toml`'s `[toolchain] channel = "..."`, or the legacy
    /// bare `rust-toolchain` file whose entire contents are the channel.
    RustToolchain(&'static str),
    /// `go.mod`'s `go 1.23.4` directive.
    GoDirective(&'static str),
    /// One `<tool> <version>` line in an `asdf`/`mise` `.tool-versions`.
    /// The payload is the *tool* name to look for, not a file name — the file
    /// is always `.tool-versions` ([`TOOL_VERSIONS_FILE`]).
    ToolVersions(&'static str),
}

/// The file [`VersionSource::ToolVersions`] reads. `asdf` and `mise` both use
/// this one name, and it holds every tool, so the variant carries the tool to
/// look up rather than a path.
pub const TOOL_VERSIONS_FILE: &str = ".tool-versions";

impl VersionSource {
    /// The file this source reads, relative to the project directory.
    pub const fn file_name(&self) -> &'static str {
        match self {
            Self::Bare(name) | Self::RustToolchain(name) | Self::GoDirective(name) => name,
            // The payload here is the tool to look up, not a path.
            Self::ToolVersions(_) => TOOL_VERSIONS_FILE,
        }
    }

    /// Reads and parses this source under `dir`, or `None` when the file is
    /// absent, unreadable, or names no version.
    ///
    /// A malformed file is `None` rather than an error on purpose: the file
    /// belongs to another tool, and `paws` failing to build because someone's
    /// `.tool-versions` has a line it doesn't understand would be worse than
    /// falling through to the next source.
    pub fn read(&self, dir: &Path) -> Option<String> {
        let path = dir.join(self.file_name());
        let contents = std::fs::read_to_string(&path).ok()?;
        let parsed = match self {
            Self::Bare(_) => parse_bare(&contents),
            Self::RustToolchain(_) => parse_rust_toolchain(&contents),
            Self::GoDirective(_) => parse_go_directive(&contents),
            Self::ToolVersions(tool) => parse_tool_versions(&contents, tool),
        }?;
        let parsed = parsed.trim();
        (!parsed.is_empty()).then(|| parsed.to_string())
    }
}

/// First meaningful line, with a `v` or `<word>-` prefix stripped.
fn parse_bare(contents: &str) -> Option<String> {
    let line = meaningful_lines(contents).next()?;
    Some(strip_version_prefix(line).to_string())
}

/// `.nvmrc` holds `v20.11.0`; `.ruby-version` holds `ruby-3.3.0`. Both mean
/// the bare version. A prefix is only stripped when what follows starts with a
/// digit, so a genuine alias like `lts/iron` survives untouched.
fn strip_version_prefix(value: &str) -> &str {
    if let Some(rest) = value.strip_prefix('v')
        && rest.starts_with(|c: char| c.is_ascii_digit())
    {
        return rest;
    }
    if let Some((_, rest)) = value.split_once('-')
        && rest.starts_with(|c: char| c.is_ascii_digit())
    {
        return rest;
    }
    value
}

/// `[toolchain] channel = "1.90.0"`, or a legacy bare file holding just the
/// channel name.
fn parse_rust_toolchain(contents: &str) -> Option<String> {
    for line in meaningful_lines(contents) {
        if let Some((key, value)) = line.split_once('=')
            && key.trim() == "channel"
        {
            return Some(value.trim().trim_matches(['"', '\'']).to_string());
        }
    }
    // No `channel =` key: the legacy format is the whole file.
    let first = meaningful_lines(contents).next()?;
    (!first.contains('[') && !first.contains('=')).then(|| first.to_string())
}

/// The `go 1.23.4` line in a `go.mod`. Not `toolchain go1.x`, which names the
/// toolchain used to build, not the language version the module targets.
fn parse_go_directive(contents: &str) -> Option<String> {
    meaningful_lines(contents)
        .find_map(|line| line.strip_prefix("go "))
        .map(|version| version.trim().to_string())
}

/// The `<tool> <version>` line naming `tool` in an asdf/mise `.tool-versions`.
fn parse_tool_versions(contents: &str, tool: &str) -> Option<String> {
    meaningful_lines(contents).find_map(|line| {
        let (name, rest) = line.split_once(char::is_whitespace)?;
        (name == tool).then(|| {
            // asdf allows several fallback versions on one line; the first is
            // the one that would actually be used.
            rest.split_whitespace()
                .next()
                .unwrap_or_default()
                .to_string()
        })
    })
}

/// Non-empty lines with `#` comments and inline comments removed.
fn meaningful_lines(contents: &str) -> impl Iterator<Item = &str> {
    contents.lines().filter_map(|line| {
        let line = line.split('#').next().unwrap_or_default().trim();
        (!line.is_empty()).then_some(line)
    })
}

/// Applies the precedence documented on this module.
///
/// `sources` are tried in order, so a toolchain listing several version files
/// gets a defined winner rather than whichever `read_dir` happened to return.
pub fn resolve(
    dir: &Path,
    flag: Option<&str>,
    sources: &[VersionSource],
    configured: Option<&str>,
    default: &str,
) -> ResolvedVersion {
    if let Some(version) = flag.map(str::trim).filter(|v| !v.is_empty()) {
        return ResolvedVersion {
            version: version.to_string(),
            origin: VersionOrigin::Flag,
        };
    }
    for source in sources {
        if let Some(version) = source.read(dir) {
            return ResolvedVersion {
                version,
                origin: VersionOrigin::VersionFile(PathBuf::from(source.file_name())),
            };
        }
    }
    if let Some(version) = configured.map(str::trim).filter(|v| !v.is_empty()) {
        return ResolvedVersion {
            version: version.to_string(),
            origin: VersionOrigin::Config,
        };
    }
    ResolvedVersion {
        version: default.to_string(),
        origin: VersionOrigin::Default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        crate::test_support::scratch_dir("core-version", name)
    }

    fn write(dir: &Path, name: &str, contents: &str) {
        std::fs::write(dir.join(name), contents).unwrap();
    }

    #[test]
    fn rust_toolchain_toml_yields_its_channel() {
        let dir = scratch("rust-toml");
        write(
            &dir,
            "rust-toolchain.toml",
            "[toolchain]\nchannel = \"1.90.0\"\ncomponents = [\"clippy\"]\n",
        );
        let source = VersionSource::RustToolchain("rust-toolchain.toml");
        assert_eq!(source.read(&dir).as_deref(), Some("1.90.0"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_legacy_bare_rust_toolchain_file_is_the_channel() {
        let dir = scratch("rust-legacy");
        write(&dir, "rust-toolchain", "nightly-2026-01-01\n");
        let source = VersionSource::RustToolchain("rust-toolchain");
        assert_eq!(source.read(&dir).as_deref(), Some("nightly-2026-01-01"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn nvmrc_and_ruby_version_shed_their_conventional_prefixes() {
        let dir = scratch("prefixes");
        write(&dir, ".nvmrc", "v20.11.0\n");
        write(&dir, ".ruby-version", "ruby-3.3.0\n");
        assert_eq!(
            VersionSource::Bare(".nvmrc").read(&dir).as_deref(),
            Some("20.11.0")
        );
        assert_eq!(
            VersionSource::Bare(".ruby-version").read(&dir).as_deref(),
            Some("3.3.0")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `lts/iron` is a real `.nvmrc` value and is not a version with a
    /// stripped prefix — the `v`/`-` rules must not mangle it.
    #[test]
    fn an_alias_is_left_alone() {
        let dir = scratch("alias");
        write(&dir, ".nvmrc", "lts/iron\n");
        assert_eq!(
            VersionSource::Bare(".nvmrc").read(&dir).as_deref(),
            Some("lts/iron")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// pyenv allows several versions; the first is the primary one.
    #[test]
    fn only_the_first_meaningful_line_is_read() {
        let dir = scratch("multiline");
        write(
            &dir,
            ".python-version",
            "# managed by pyenv\n3.12.1\n3.11.8\n",
        );
        assert_eq!(
            VersionSource::Bare(".python-version").read(&dir).as_deref(),
            Some("3.12.1")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn go_mod_yields_the_language_directive_not_the_toolchain_line() {
        let dir = scratch("gomod");
        write(
            &dir,
            "go.mod",
            "module example.com/x\n\ngo 1.23.4\n\ntoolchain go1.24.0\n",
        );
        assert_eq!(
            VersionSource::GoDirective("go.mod").read(&dir).as_deref(),
            Some("1.23.4")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tool_versions_picks_the_named_tool_and_its_first_version() {
        let dir = scratch("toolversions");
        write(
            &dir,
            ".tool-versions",
            "# mise\nnodejs 20.11.0 18.19.0\nruby 3.3.0\n",
        );
        assert_eq!(
            VersionSource::ToolVersions("nodejs").read(&dir).as_deref(),
            Some("20.11.0")
        );
        assert_eq!(
            VersionSource::ToolVersions("ruby").read(&dir).as_deref(),
            Some("3.3.0")
        );
        assert_eq!(VersionSource::ToolVersions("python").read(&dir), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `ToolVersions` carries a tool name, every other variant carries a
    /// file name. Conflating the two made `.tool-versions` lookups read a file
    /// named after the tool and silently find nothing.
    #[test]
    fn tool_versions_reads_the_shared_file_not_a_file_named_after_the_tool() {
        assert_eq!(
            VersionSource::ToolVersions("nodejs").file_name(),
            TOOL_VERSIONS_FILE
        );
        assert_eq!(VersionSource::Bare(".nvmrc").file_name(), ".nvmrc");
    }

    #[test]
    fn a_missing_or_empty_file_resolves_to_nothing() {
        let dir = scratch("missing");
        assert_eq!(VersionSource::Bare(".nvmrc").read(&dir), None);
        write(&dir, ".nvmrc", "\n#only a comment\n");
        assert_eq!(VersionSource::Bare(".nvmrc").read(&dir), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn precedence_runs_flag_then_file_then_config_then_default() {
        let dir = scratch("precedence");
        let sources = [VersionSource::Bare(".python-version")];

        // Nothing anywhere: the built-in default.
        let r = resolve(&dir, None, &sources, None, "3.13");
        assert_eq!(r.version, "3.13");
        assert_eq!(r.origin, VersionOrigin::Default);

        // paws.toml beats the default.
        let r = resolve(&dir, None, &sources, Some("3.12"), "3.13");
        assert_eq!(r.version, "3.12");
        assert_eq!(r.origin, VersionOrigin::Config);

        // A native version file beats paws.toml — the repo's own tools obey
        // this file, so paws must agree with them.
        write(&dir, ".python-version", "3.11.8\n");
        let r = resolve(&dir, None, &sources, Some("3.12"), "3.13");
        assert_eq!(r.version, "3.11.8");
        assert_eq!(
            r.origin,
            VersionOrigin::VersionFile(PathBuf::from(".python-version"))
        );

        // An explicit flag beats everything.
        let r = resolve(&dir, Some("3.10"), &sources, Some("3.12"), "3.13");
        assert_eq!(r.version, "3.10");
        assert_eq!(r.origin, VersionOrigin::Flag);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_empty_flag_or_config_value_is_ignored_rather_than_used() {
        let dir = scratch("empty-values");
        let r = resolve(&dir, Some("  "), &[], Some(""), "1.0");
        assert_eq!(r.version, "1.0");
        assert_eq!(r.origin, VersionOrigin::Default);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn describe_names_the_source_unless_it_is_the_default() {
        assert_eq!(
            ResolvedVersion {
                version: "1.90.0".into(),
                origin: VersionOrigin::VersionFile(PathBuf::from("rust-toolchain.toml")),
            }
            .describe(),
            "1.90.0 (rust-toolchain.toml)"
        );
        assert_eq!(
            ResolvedVersion {
                version: "1".into(),
                origin: VersionOrigin::Default,
            }
            .describe(),
            "1"
        );
    }
}
