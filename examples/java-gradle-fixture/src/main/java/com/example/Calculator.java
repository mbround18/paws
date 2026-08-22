// A minimal, deliberately clean class: builds and tests successfully, for
// `paws ci --toolchain java`'s "clean run" acceptance scenario, matching
// examples/java-maven-fixture's role for the Maven build system.
package com.example;

public class Calculator {
    public static int add(int a, int b) {
        return a + b;
    }
}
