defmodule Acme.MixProject do
  use Mix.Project

  def project do
    [
      app: :acme,
      version: "0.1.0",
      elixir: "~> 1.16",
      aliases: aliases(),
      deps: deps()
    ]
  end

  def application do
    [extra_applications: [:logger]]
  end

  defp deps do
    []
  end

  defp aliases do
    [
      "ecto.reset": ["ecto.drop", "ecto.create", "ecto.migrate"],
      precommit: ["compile --warnings-as-errors", "format --check-formatted", "test"]
    ]
  end
end
