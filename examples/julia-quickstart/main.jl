# Julia Quickstart Example
include("../../sdk/chocobase-jl/src/ChocoBase.jl")
using .ChocoBase

println("🍫 ChocoBase Julia Quickstart (AI / ML & Numerical Computing)")

client = ChocoClient("http://localhost:8080", "anon_dev_token")

# 1. PostgREST: Query table
res = from(client, "embeddings") |>
      q -> select(q, "id, model, vector_dim") |>
      q -> limit(q, 5) |>
      execute

println("Query executed successfully.")

# 2. Storage: Signed URL
signed_url = create_signed_url(client, "models", "weights.bin", 3600)
println("Signed download URL: ", signed_url)

# 3. Edge Functions: Invoke
fn_res = invoke(client, "inference", Dict("prompt" => "Vector search query"))
println("Function inference result: ", fn_res)

println("✅ Julia Quickstart completed successfully!")
