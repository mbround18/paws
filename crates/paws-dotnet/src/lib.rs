//! Native .NET CI support for `paws ci --toolchain dotnet` (the
//! `C# / .NET` row in `docs/ROADMAP.md`). No `gh-reusable` precedent — its
//! Dagger module never had a .NET function of any kind (see
//! `packages/dagger-module/src/index.ts`) — so the step sequence here
//! (`dotnet restore`, `dotnet build --no-restore -c Release`, `dotnet test
//! --no-build -c Release`) is a new native implementation following the
//! ordinary restore/build/test split the .NET SDK itself is designed
//! around: each step reuses the previous one's output rather than silently
//! re-running it, so a failure names the phase that actually broke.
//!
//! Language-agnostic within the SDK: a C#, F#, or VB project is the same
//! `dotnet` invocation against a different project-file extension, so this
//! crate detects `.csproj`/`.fsproj`/`.vbproj` alike rather than being
//! C#-specific.
//!
//! No `builders/*` image: `mcr.microsoft.com/dotnet/sdk` is Microsoft's own
//! published SDK image and already has the full toolchain (see
//! `docs/ROADMAP.md`'s "How a new stack gets added"). The Blazor/MAUI rows
//! in that table are deliberately *not* covered here — MAUI needs
//! Android/iOS/Windows SDK workloads (and, for Apple targets, a macOS
//! host) this pipeline has no story for, the same reason the Swift/Flutter
//! rows are still open.

use paws_core::Pipeline;
use std::path::{Path, PathBuf};

use anyhow::Result;

/// .NET's release train alternates LTS (even majors, 3 years of support)
/// and STS (odd majors, 18 months), so unlike Ruby/Go/Rust "latest major"
/// is *not* automatically the right default — `mcr.microsoft.com/dotnet/
/// sdk:latest` can point at an STS release that goes out of support in
/// well under two years. This pins the current LTS, .NET 10 (confirmed
/// directly against the MCR tag list — a real floating `10.0` tag exists,
/// tracking that major's patch releases). A genuine Renovate target on
/// each new LTS, like the Temurin pin in `builders/java` (see
/// `docs/ROADMAP.md`'s "Base image version policy").
pub const DEFAULT_DOTNET_SDK_VERSION: &str = "10.0";

fn base_image(sdk_version: &str) -> String {
    format!("mcr.microsoft.com/dotnet/sdk:{sdk_version}")
}

/// A project file the .NET SDK understands, by extension.
fn is_project_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("csproj" | "fsproj" | "vbproj")
    )
}

/// A solution file — either the classic `.sln` or the XML `.slnx` format
/// the modern SDK's `dotnet new sln` emits by default (confirmed for real
/// against `mcr.microsoft.com/dotnet/sdk:10.0`, which produced a `.slnx`,
/// not a `.sln`, when `examples/dotnet-fixture` was scaffolded).
fn is_solution_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("sln" | "slnx")
    )
}

fn entries(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    read.flatten().map(|e| e.path()).collect()
}

/// Every project file under `dir`, recursing but skipping hidden
/// directories plus `bin`/`obj` (the SDK's own build output, which
/// contains generated project files) — same walking strategy as
/// `paws_go::go_files`/`paws_kotlin::kotlin_files`.
fn project_files(dir: &Path) -> Vec<PathBuf> {
    paws_core::find_files(dir, is_project_file)
}

#[derive(Debug)]
pub struct DotnetProject {
    /// Whether a test project exists — a project file with a
    /// `Microsoft.NET.Test.Sdk` package reference, which is what makes
    /// `dotnet test` able to discover and run anything at all (xUnit,
    /// `NUnit`, and `MSTest` templates all bring it in). A structural check on
    /// real content, not a `*.Tests` name convention.
    pub has_tests: bool,
}

/// A .NET project is a directory `dotnet restore` can be pointed at with no
/// arguments: it holds either a solution file or a project file at its root
/// (the SDK only searches the working directory itself, not below it).
pub fn is_dotnet_project(dir: &Path) -> bool {
    entries(dir)
        .iter()
        .any(|p| p.is_file() && (is_solution_file(p) || is_project_file(p)))
}

pub fn detect_project(dir: &Path) -> Result<DotnetProject> {
    let root = entries(dir);
    let solutions: Vec<_> = root
        .iter()
        .filter(|p| p.is_file() && is_solution_file(p))
        .collect();
    let root_projects: Vec<_> = root
        .iter()
        .filter(|p| p.is_file() && is_project_file(p))
        .collect();

    if solutions.is_empty() && root_projects.is_empty() {
        anyhow::bail!(
            "no .NET solution (.sln/.slnx) or project (.csproj/.fsproj/.vbproj) file found in {}",
            dir.display()
        );
    }
    // The SDK itself refuses to guess here (MSB1011), so fail with an
    // actionable message rather than letting `dotnet restore` do it with a
    // bare error code deep inside a container log.
    if solutions.len() > 1 {
        anyhow::bail!(
            "more than one solution file in {} — the .NET SDK can't pick between them; keep one at the root",
            dir.display()
        );
    }
    if solutions.is_empty() && root_projects.len() > 1 {
        anyhow::bail!(
            "more than one project file in {} and no solution file — add a .sln/.slnx so the .NET SDK knows what to build",
            dir.display()
        );
    }

    let has_tests = project_files(dir)
        .iter()
        .any(|p| std::fs::read_to_string(p).is_ok_and(|s| s.contains("Microsoft.NET.Test.Sdk")));

    Ok(DotnetProject { has_tests })
}

/// Builds the `dagger core <chain>` argument list (see `paws_dagger::core`)
/// for `project`, using [`DEFAULT_DOTNET_SDK_VERSION`].
pub fn dagger_pipeline_args(project: &DotnetProject, source_dir: &str) -> Vec<String> {
    dagger_pipeline_args_with_version(project, source_dir, DEFAULT_DOTNET_SDK_VERSION)
}

/// Same as [`dagger_pipeline_args`], with an explicit SDK version (selects
/// the `mcr.microsoft.com/dotnet/sdk:<version>` image tag).
pub fn dagger_pipeline_args_with_version(
    project: &DotnetProject,
    source_dir: &str,
    sdk_version: &str,
) -> Vec<String> {
    Pipeline::from_image(&base_image(sdk_version))
        .mount("/src", source_dir)
        .workdir("/src")
        // First-run telemetry/welcome output is pure noise in a CI log and
        // the SDK writes a sentinel file for it on every fresh container.
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .exec(["dotnet", "restore"])
        .exec(["dotnet", "build", "--no-restore", "-c", "Release"])
        .exec_if(
            project.has_tests,
            ["dotnet", "test", "--no-build", "-c", "Release"],
        )
        .stdout()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        paws_core::test_support::scratch_dir("dotnet", name)
    }

    #[test]
    fn detects_a_single_root_project_file() {
        let dir = temp_dir("single-project");
        assert!(!is_dotnet_project(&dir));
        fs::write(dir.join("App.csproj"), "<Project/>").unwrap();
        assert!(is_dotnet_project(&dir));
        assert!(detect_project(&dir).is_ok());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn detects_fsproj_and_vbproj_too() {
        for ext in ["fsproj", "vbproj"] {
            let dir = temp_dir(ext);
            fs::write(dir.join(format!("App.{ext}")), "<Project/>").unwrap();
            assert!(is_dotnet_project(&dir), "{ext} should be detected");
            fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn errors_when_nothing_buildable_is_present() {
        let dir = temp_dir("empty");
        let err = detect_project(&dir).unwrap_err().to_string();
        assert!(err.contains("no .NET solution"), "{err}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn errors_on_multiple_root_projects_without_a_solution() {
        let dir = temp_dir("ambiguous");
        fs::write(dir.join("A.csproj"), "<Project/>").unwrap();
        fs::write(dir.join("B.csproj"), "<Project/>").unwrap();
        let err = detect_project(&dir).unwrap_err().to_string();
        assert!(err.contains("no solution file"), "{err}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_solution_disambiguates_multiple_projects() {
        let dir = temp_dir("solution");
        fs::write(dir.join("A.csproj"), "<Project/>").unwrap();
        fs::write(dir.join("B.csproj"), "<Project/>").unwrap();
        fs::write(dir.join("All.slnx"), "<Solution/>").unwrap();
        assert!(detect_project(&dir).is_ok());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn errors_on_multiple_solutions() {
        let dir = temp_dir("two-solutions");
        fs::write(dir.join("A.sln"), "").unwrap();
        fs::write(dir.join("B.slnx"), "").unwrap();
        let err = detect_project(&dir).unwrap_err().to_string();
        assert!(err.contains("more than one solution"), "{err}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn detects_a_test_project_from_its_test_sdk_reference() {
        let dir = temp_dir("tests");
        fs::write(dir.join("App.slnx"), "<Solution/>").unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/App.csproj"), "<Project/>").unwrap();
        assert!(!detect_project(&dir).unwrap().has_tests);

        fs::create_dir_all(dir.join("tests")).unwrap();
        fs::write(
            dir.join("tests/App.Tests.csproj"),
            "<Project><ItemGroup><PackageReference Include=\"Microsoft.NET.Test.Sdk\" Version=\"17.14.1\" /></ItemGroup></Project>",
        )
        .unwrap();
        assert!(detect_project(&dir).unwrap().has_tests);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn build_output_directories_are_not_scanned_for_test_projects() {
        let dir = temp_dir("skips-obj");
        fs::write(dir.join("App.csproj"), "<Project/>").unwrap();
        fs::create_dir_all(dir.join("obj")).unwrap();
        fs::write(dir.join("obj/Generated.csproj"), "Microsoft.NET.Test.Sdk").unwrap();
        assert!(!detect_project(&dir).unwrap().has_tests);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pipeline_restores_builds_and_tests() {
        let args = dagger_pipeline_args(&DotnetProject { has_tests: true }, "/host/src");
        assert_eq!(args[0], "container");
        assert_eq!(args[2], "--address=mcr.microsoft.com/dotnet/sdk:10.0");
        assert!(args.contains(&"--args=dotnet,restore".to_string()));
        assert!(args.contains(&"--args=dotnet,build,--no-restore,-c,Release".to_string()));
        assert!(args.contains(&"--args=dotnet,test,--no-build,-c,Release".to_string()));
        assert_eq!(args.last(), Some(&"stdout".to_string()));
    }

    #[test]
    fn pipeline_skips_the_test_step_without_a_test_project() {
        let args = dagger_pipeline_args(&DotnetProject { has_tests: false }, "/host/src");
        assert!(!args.iter().any(|a| a.contains("dotnet,test")));
        assert!(args.contains(&"--args=dotnet,build,--no-restore,-c,Release".to_string()));
    }

    #[test]
    fn pipeline_respects_an_explicit_sdk_version() {
        let args = dagger_pipeline_args_with_version(
            &DotnetProject { has_tests: false },
            "/host/src",
            "9.0",
        );
        assert_eq!(args[2], "--address=mcr.microsoft.com/dotnet/sdk:9.0");
    }
}
