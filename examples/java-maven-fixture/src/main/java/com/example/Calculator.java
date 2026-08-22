// A minimal, deliberately clean class: builds and tests successfully, for
// `paws ci --toolchain java`'s "clean run" acceptance scenario, matching
// examples/rust-fixture's role for `--toolchain rust`.
package com.example;

public class Calculator {
    public static int add(int a, int b) {
        return a + b;
    }
}
