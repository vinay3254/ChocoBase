module ChocoBase

using HTTP
using JSON3

export ChocoClient, from, sign_up, sign_in, select, eq, limit, execute, create_signed_url, invoke

struct ChocoClient
    url::String
    api_key::String
    headers::Dict{String, String}

    function ChocoClient(url::String, api_key::String, custom_headers::Dict{String, String}=Dict{String, String}())
        clean_url = rstrip(url, '/')
        base_headers = Dict(
            "apikey" => api_key,
            "Authorization" => "Bearer " * api_key,
            "Content-Type" => "application/json"
        )
        merge!(base_headers, custom_headers)
        new(clean_url, api_key, base_headers)
    end
end

include("Auth.jl")
include("Postgrest.jl")
include("Storage.jl")
include("Functions.jl")

end # module
