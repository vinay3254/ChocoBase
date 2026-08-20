defmodule ChocoBase.Storage do
  @moduledoc """
  Object Storage client for Elixir.
  """

  def create_signed_url(%ChocoBase{} = client, bucket, path, expires_in \\ 3600) do
    url = client.url <> "/v1/storage/v1/object/sign/" <> bucket <> "/" <> path
    body = Jason.encode!(%{expires_in: expires_in})
    char_url = String.to_charlist(url)
    char_headers = Enum.map(client.headers, fn {k, v} -> {String.to_charlist(k), String.to_charlist(v)} end)

    case :httpc.request(:post, {char_url, char_headers, 'application/json', body}, [], []) do
      {:ok, {{_, 200, _}, _resp_headers, resp_body}} ->
        decoded = Jason.decode!(to_string(resp_body))
        {:ok, client.url <> decoded["signed_url"]}

      {:ok, {{_, status, _}, _resp_headers, resp_body}} ->
        {:error, {status, to_string(resp_body)}}

      {:error, reason} ->
        {:error, reason}
    end
  end
end
