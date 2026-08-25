require "minitest/autorun"
require "calculator"

class CalculatorTest < Minitest::Test
  def test_add
    assert_equal 5, Calculator.add(2, 3)
  end

  def test_multiply
    assert_equal 6, Calculator.multiply(2, 3)
  end
end
