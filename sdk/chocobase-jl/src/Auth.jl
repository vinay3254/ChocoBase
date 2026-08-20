function sign_up(client::ChocoClient, username::String, password::String)
    url = "$(client.url)/v1/auth/signup"
    body = JSON3.write(Dict("username" => username, "password" => password))
    resp = HTTP.post(url, client.headers, body)
    return JSON3.read(String(resp.body))
end

function sign_in(client::ChocoClient, username::String, password::String)
    url = "$(client.url)/v1/auth/token"
    body = JSON3.write(Dict("username" => username, "password" => password))
    resp = HTTP.post(url, client.headers, body)
    return JSON3.read(String(resp.body))
end
