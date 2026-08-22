// A minimal, deliberately clean object: builds and tests successfully, for
// `paws ci --toolchain kotlin`'s "clean run" acceptance scenario, matching
// examples/java-maven-fixture's role for Java.
package com.example

object Calculator {
    fun add(a: Int, b: Int): Int = a + b
}
