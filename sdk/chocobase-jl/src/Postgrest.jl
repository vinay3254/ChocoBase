struct PostgrestQuery
    client::ChocoClient
    table::String
    params::Dict{String, String}

    PostgrestQuery(client::ChocoClient, table::String) = new(client, table, Dict{String, String}())
    PostgrestQuery(client::ChocoClient, table::String, params::Dict{String, String}) = new(client, table, params)
end

function from(client::ChocoClient, table::String)
    return PostgrestQuery(client, table)
end

function select(query::PostgrestQuery, columns::String="*")
    new_params = copy(query.params)
    new_params["select"] = columns
    return PostgrestQuery(query.client, query.table, new_params)
end

function eq(query::PostgrestQuery, column::String, value)
    new_params = copy(query.params)
    new_params[column] = "eq.$(value)"
    return PostgrestQuery(query.client, query.table, new_params)
end

function limit(query::PostgrestQuery, count::Int)
    new_params = copy(query.params)
    new_params["limit"] = string(count)
    return PostgrestQuery(query.client, query.table, new_params)
end

function execute(query::PostgrestQuery)
    query_str = HTTP.URIs.escapeuri(query.params)
    url = "$(query.client.url)/rest/v1/$(query.table)" * (isempty(query_str) ? "" : "?$(query_str)")
    resp = HTTP.get(url, query.client.headers)
    return JSON3.read(String(resp.body))
end
