// The Java half of a mixed Java+Kotlin module -- the target for
// docs/ROADMAP.md's Java + Kotlin row. Called from the Kotlin half
// (Calculator.kt) below, proving Gradle's java/kotlin plugins compile and
// link both languages together in one build with no special paws-kotlin
// handling needed.
package com.example;

public class Adder {
    public static int add(int a, int b) {
        return a + b;
    }
}
