defmodule ChocoBase.MixProject do
  use Mix.Project

  def project do
    [
      app: :chocobase,
      version: "0.1.0",
      elixir: "~> 1.12",
      start_permanent: Mix.env() == :prod,
      deps: deps(),
      description: "Official Elixir and Phoenix client library for ChocoBase database platform.",
      package: package()
    ]
  end

  def application do
    [
      extra_applications: [:logger, :inets, :ssl]
    ]
  end

  defp deps do
    [
      {:jason, "~> 1.4"}
    ]
  end

  defp package do
    [
      licenses: ["MIT"],
      links: %{"GitHub" => "https://github.com/vinay3254/ChocoBase"}
    ]
  end
end
