//! Rust half of the multi-ecosystem fixture: a minimal, clean crate that builds and
//! tests successfully, co-located with a Node project (see `package.json` in this
//! same directory) to simulate a repo with both a Rust crate and a Node/pnpm
//! workspace (spec.md's User Story 5, acceptance scenario 3), used to verify that
//! `paws ci`/`paws provision` provisions multiple detected ecosystems concurrently.

pub fn multiply(a: i32, b: i32) -> i32 {
    a * b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiplies_two_numbers() {
        assert_eq!(multiply(2, 3), 6);
    }
}
