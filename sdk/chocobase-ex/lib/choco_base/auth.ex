defmodule ChocoBase.Auth do
  @moduledoc """
  Authentication client for ChocoBase.
  """

  def sign_up(%ChocoBase{} = client, username, password) do
    url = client.url <> "/v1/auth/signup"
    body = Jason.encode!(%{username: username, password: password})
    post(url, client.headers, body)
  end

  def sign_in(%ChocoBase{} = client, username, password) do
    url = client.url <> "/v1/auth/token"
    body = Jason.encode!(%{username: username, password: password})
    post(url, client.headers, body)
  end

  defp post(url, headers, body) do
    char_url = String.to_charlist(url)
    char_headers = Enum.map(headers, fn {k, v} -> {String.to_charlist(k), String.to_charlist(v)} end)

    case :httpc.request(:post, {char_url, char_headers, 'application/json', body}, [], []) do
      {:ok, {{_, 200, _}, _resp_headers, resp_body}} ->
        {:ok, Jason.decode!(to_string(resp_body))}

      {:ok, {{_, status, _}, _resp_headers, resp_body}} ->
        {:error, {status, Jason.decode!(to_string(resp_body))}}

      {:error, reason} ->
        {:error, reason}
    end
  end
end
