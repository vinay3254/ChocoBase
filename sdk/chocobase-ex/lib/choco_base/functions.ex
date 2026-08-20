defmodule ChocoBase.Functions do
  @moduledoc """
  Serverless Edge Functions client for Elixir.
  """

  def invoke(%ChocoBase{} = client, function_name, body \\ %{}) do
    url = client.url <> "/v1/functions/v1/" <> function_name
    encoded_body = Jason.encode!(body)
    char_url = String.to_charlist(url)
    char_headers = Enum.map(client.headers, fn {k, v} -> {String.to_charlist(k), String.to_charlist(v)} end)

    case :httpc.request(:post, {char_url, char_headers, 'application/json', encoded_body}, [], []) do
      {:ok, {{_, 200, _}, _resp_headers, resp_body}} ->
        {:ok, Jason.decode!(to_string(resp_body))}

      {:ok, {{_, status, _}, _resp_headers, resp_body}} ->
        {:error, {status, to_string(resp_body)}}

      {:error, reason} ->
        {:error, reason}
    end
  end
end
