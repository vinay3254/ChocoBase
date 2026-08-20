function create_signed_url(client::ChocoClient, bucket::String, path::String, expires_in::Int=3600)
    url = "$(client.url)/v1/storage/v1/object/sign/$(bucket)/$(path)"
    body = JSON3.write(Dict("expires_in" => expires_in))
    resp = HTTP.post(url, client.headers, body)
    data = JSON3.read(String(resp.body))
    return haskey(data, :signed_url) ? "$(client.url)$(data[:signed_url])" : nothing
end
