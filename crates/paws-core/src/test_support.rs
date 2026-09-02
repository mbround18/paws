//! Helpers for the toolchain crates' own tests.
//!
//! Behind the `testing` feature so nothing here is compiled into a real
//! `paws` binary. Declared as a `[dev-dependencies]` feature by the crates
//! that use it.

use std::path::PathBuf;

/// A fresh, empty directory for one test to work in.
///
/// Named `paws-<crate>-test-<name>-<pid>`: the crate and test name make a
/// leftover directory traceable, and the pid keeps two `cargo test` runs (or
/// two crates' suites running in parallel) from sharing one path. Any
/// existing directory at that path is removed first, so a test that panicked
/// before its cleanup doesn't poison the next run.
///
/// Every toolchain crate had its own byte-identical copy of this, differing
/// only in the crate name in the prefix.
pub fn scratch_dir(crate_name: &str, name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "paws-{crate_name}-test-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scratch_dir_is_empty_even_when_one_was_left_behind() {
        let dir = scratch_dir("core", "reuse");
        std::fs::write(dir.join("leftover"), "").unwrap();

        let again = scratch_dir("core", "reuse");
        assert_eq!(again, dir, "the same test name resolves to the same path");
        assert!(
            !again.join("leftover").exists(),
            "a previous run's files must not survive"
        );

        std::fs::remove_dir_all(&again).ok();
    }

    #[test]
    fn different_tests_never_share_a_directory() {
        assert_ne!(scratch_dir("core", "a"), scratch_dir("core", "b"));
        assert_ne!(scratch_dir("go", "same"), scratch_dir("rust", "same"));
    }
}
