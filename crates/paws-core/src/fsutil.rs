//! Directory walking shared by the toolchain crates' project detection.
//!
//! Three crates hand-rolled the same recursive walk — `paws_go::go_files`,
//! `paws_kotlin::kotlin_files`, `paws_dotnet::project_files` — each with its
//! own copy of the skip-hidden-directories rule and its own subtly different
//! list of build-output directories to ignore.

use std::path::{Path, PathBuf};

/// Directory names that never hold project sources worth detecting on:
/// dependency trees and build output, in every ecosystem `paws` supports.
///
/// One list rather than one per crate — `paws-go` skipped `vendor`,
/// `paws-dotnet` skipped `bin`/`obj`, and `paws-kotlin` skipped neither, so
/// which stale artifacts could confuse detection depended on the language.
pub const IGNORED_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "vendor",
    "bin",
    "obj",
    "build",
    "dist",
    "out",
    ".venv",
    "venv",
    "__pycache__",
];

/// Every file under `dir` (recursively) that `keep` accepts.
///
/// Hidden directories (a leading `.`) and [`IGNORED_DIRS`] are not descended
/// into. An unreadable directory yields nothing rather than failing: detection
/// is a best-effort signal, and a permissions error deep in a tree should not
/// turn "is this a Go project?" into an error the user has to act on.
pub fn find_files<F>(dir: &Path, keep: F) -> Vec<PathBuf>
where
    F: Fn(&Path) -> bool + Copy,
{
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || IGNORED_DIRS.contains(&name) {
                continue;
            }
            files.extend(find_files(&path, keep));
        } else if keep(&path) {
            files.push(path);
        }
    }
    files
}

/// [`find_files`] for the common case of matching on file extension.
pub fn find_files_with_extension(dir: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    find_files(dir, |path| {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| extensions.contains(&e))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("paws-fsutil-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(root: &Path, relative: &str) {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }

    #[test]
    fn finds_matching_files_recursively_and_skips_noise_directories() {
        let dir = scratch("walk");
        touch(&dir, "main.go");
        touch(&dir, "internal/app/server.go");
        touch(&dir, "README.md");
        touch(&dir, "vendor/github.com/x/dep.go");
        touch(&dir, ".git/hooks/pre-commit.go");
        touch(&dir, "node_modules/pkg/index.go");

        let mut found: Vec<String> = find_files_with_extension(&dir, &["go"])
            .iter()
            .map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().to_string())
            .collect();
        found.sort();

        assert_eq!(found, vec!["internal/app/server.go", "main.go"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn several_extensions_match_in_one_walk() {
        let dir = scratch("multi-ext");
        touch(&dir, "Main.kt");
        touch(&dir, "build.gradle.kts");
        touch(&dir, "notes.txt");

        let found = find_files_with_extension(&dir, &["kt", "kts"]);
        assert_eq!(found.len(), 2, "found {found:?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unreadable_directory_yields_nothing_rather_than_failing() {
        let missing = std::env::temp_dir().join("paws-fsutil-does-not-exist");
        assert!(find_files_with_extension(&missing, &["go"]).is_empty());
    }
}
