package com.example

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test

class CalculatorTest {
    @Test
    fun addsTwoNumbersThroughTheJavaAdder() {
        assertEquals(5, Calculator.add(2, 3))
    }
}
