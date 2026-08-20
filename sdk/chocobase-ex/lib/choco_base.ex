defmodule ChocoBase do
  @moduledoc """
  Official Elixir client for ChocoBase.
  """

  defstruct [:url, :api_key, :headers]

  @type t :: %__MODULE__{
          url: String.t(),
          api_key: String.t(),
          headers: [{String.t(), String.t()}]
        }

  def new(url, api_key, custom_headers \\ %{}) do
    clean_url = String.trim_trailing(url, "/")

    base_headers = [
      {"apikey", api_key},
      {"authorization", "Bearer " <> api_key},
      {"content-type", "application/json"}
    ]

    headers =
      Enum.reduce(custom_headers, base_headers, fn {k, v}, acc ->
        [{to_string(k), to_string(v)} | acc]
      end)

    %__MODULE__{
      url: clean_url,
      api_key: api_key,
      headers: headers
    }
  end

  def from(%__MODULE__{} = client, table) do
    ChocoBase.Postgrest.new(client, table)
  end
end
