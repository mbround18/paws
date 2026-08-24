//! A crate with a deliberately untested branch, for
//! `paws ci --toolchain rust --coverage`'s "the tool measures a real gap,
//! not a fixed number" acceptance scenario (specs/004-rust-coverage/
//! quickstart.md §4).

pub fn classify(n: i32) -> &'static str {
    if n < 0 {
        "negative"
    } else if n == 0 {
        "zero"
    } else {
        // Deliberately never exercised by the test below.
        "positive"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_negative_and_zero_only() {
        assert_eq!(classify(-1), "negative");
        assert_eq!(classify(0), "zero");
        // No test calls classify() with a positive number — the "positive"
        // branch/region above must show up as uncovered.
    }
}
