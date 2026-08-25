defmodule CalculatorTest do
  use ExUnit.Case, async: true

  doctest Calculator

  test "add" do
    assert Calculator.add(2, 3) == 5
  end

  test "multiply" do
    assert Calculator.multiply(2, 3) == 6
  end
end
