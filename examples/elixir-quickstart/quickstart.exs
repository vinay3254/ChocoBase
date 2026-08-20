# Elixir Quickstart Script
# Run with: elixir -r ../../sdk/chocobase-ex/lib/choco_base.ex quickstart.exs

IO.puts("🍫 ChocoBase Elixir & Phoenix Quickstart")

client = ChocoBase.new("http://localhost:8080", "anon_dev_token")

# 1. PostgREST: Query table
{:ok, rows} =
  client
  |> ChocoBase.from("users")
  |> ChocoBase.Postgrest.select("id, username, role")
  |> ChocoBase.Postgrest.limit(5)
  |> ChocoBase.Postgrest.execute()

IO.puts("Fetched #{length(rows)} rows from users table.")

IO.puts("✅ Elixir Quickstart completed successfully!")
