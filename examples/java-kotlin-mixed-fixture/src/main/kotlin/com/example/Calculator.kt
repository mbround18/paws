// The Kotlin half of a mixed Java+Kotlin module: calls straight into the
// Java-compiled Adder class, proving real interop, not just two languages
// sitting side by side unused.
package com.example

object Calculator {
    fun add(a: Int, b: Int): Int = Adder.add(a, b)
}
