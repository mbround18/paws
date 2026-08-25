defmodule Calculator.MixProject do
  use Mix.Project

  def project do
    [
      app: :calculator,
      version: "0.1.0",
      elixir: "~> 1.17",
      start_permanent: Mix.env() == :prod,
      deps: deps()
    ]
  end

  def application do
    [extra_applications: [:logger]]
  end

  # Deliberately empty: the fixture's job is to prove the pipeline runs
  # `mix deps.get`/`compile`/`test` for real, not to pull dependencies.
  defp deps do
    []
  end
end
