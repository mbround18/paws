defmodule Calculator do
  @moduledoc "A deliberately tiny module for `mix test` to exercise."

  @spec add(number(), number()) :: number()
  def add(a, b), do: a + b

  @spec multiply(number(), number()) :: number()
  def multiply(a, b), do: a * b
end
