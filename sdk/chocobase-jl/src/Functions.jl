function invoke(client::ChocoClient, function_name::String, body::Dict=Dict())
    url = "$(client.url)/v1/functions/v1/$(function_name)"
    payload = JSON3.write(body)
    resp = HTTP.post(url, client.headers, payload)
    return JSON3.read(String(resp.body))
end
