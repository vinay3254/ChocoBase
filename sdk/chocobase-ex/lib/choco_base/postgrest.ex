defmodule ChocoBase.Postgrest do
  @moduledoc """
  PostgREST query builder for Elixir.
  """

  defstruct [:client, :table, :params]

  def new(%ChocoBase{} = client, table) do
    %__MODULE__{
      client: client,
      table: table,
      params: %{}
    }
  end

  def select(%__MODULE__{} = q, columns \\ "*") do
    %{q | params: Map.put(q.params, "select", columns)}
  end

  def eq(%__MODULE__{} = q, column, value) do
    %{q | params: Map.put(q.params, to_string(column), "eq." <> to_string(value))}
  end

  def limit(%__MODULE__{} = q, count) do
    %{q | params: Map.put(q.params, "limit", to_string(count))}
  end

  def execute(%__MODULE__{client: client, table: table, params: params}) do
    query = URI.encode_query(params)
    url = client.url <> "/rest/v1/" <> table <> if(query != "", do: "?" <> query, else: "")
    char_url = String.to_charlist(url)
    char_headers = Enum.map(client.headers, fn {k, v} -> {String.to_charlist(k), String.to_charlist(v)} end)

    case :httpc.request(:get, {char_url, char_headers}, [], []) do
      {:ok, {{_, 200, _}, _resp_headers, resp_body}} ->
        decoded = Jason.decode!(to_string(resp_body))
        {:ok, if(is_map(decoded) and Map.has_key?(decoded, "rows"), do: decoded["rows"], else: decoded)}

      {:ok, {{_, status, _}, _resp_headers, resp_body}} ->
        {:error, {status, to_string(resp_body)}}

      {:error, reason} ->
        {:error, reason}
    end
  end
end
