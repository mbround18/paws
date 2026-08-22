// A minimal, deliberately clean class -- the only thing distinguishing this
// fixture from examples/java-gradle-fixture is build.gradle's explicit
// `java.toolchain.languageVersion = JavaLanguageVersion.of(25)`. Confirms
// builders/java's JDK 21 + JDK 25 split really resolves a real toolchain
// requirement, not just launches Gradle successfully.
package com.example;

public class Calculator {
    public static int add(int a, int b) {
        return a + b;
    }
}
