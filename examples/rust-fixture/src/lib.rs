//! A minimal, deliberately clean crate: builds and tests successfully, for
//! `paws ci --toolchain rust`'s "clean run" acceptance scenario.

pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_two_numbers() {
        assert_eq!(add(2, 3), 5);
    }
}
