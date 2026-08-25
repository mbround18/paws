# A deliberately tiny library: enough for `rake test` to have something real
# to exercise, nothing more.
module Calculator
  module_function

  def add(a, b)
    a + b
  end

  def multiply(a, b)
    a * b
  end
end
