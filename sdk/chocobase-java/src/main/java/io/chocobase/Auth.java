package io.chocobase;

import com.google.gson.Gson;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.util.HashMap;
import java.util.Map;

public class Auth {
    private final String baseUrl;
    private final HttpClient httpClient;
    private final Map<String, String> headers;
    private final Gson gson = new Gson();

    public Auth(String baseUrl, HttpClient httpClient, Map<String, String> headers) {
        this.baseUrl = baseUrl;
        this.httpClient = httpClient;
        this.headers = headers;
    }

    public Map<String, Object> signUp(String username, String password) throws Exception {
        Map<String, String> body = new HashMap<>();
        body.put("username", username);
        body.put("password", password);
        return post("/v1/auth/signup", body);
    }

    public Map<String, Object> signIn(String username, String password) throws Exception {
        Map<String, String> body = new HashMap<>();
        body.put("username", username);
        body.put("password", password);
        return post("/v1/auth/token", body);
    }

    private Map<String, Object> post(String path, Object body) throws Exception {
        String json = gson.toJson(body);
        HttpRequest.Builder builder = HttpRequest.newBuilder()
                .uri(URI.create(baseUrl + path))
                .header("Content-Type", "application/json")
                .POST(HttpRequest.BodyPublishers.ofString(json));

        headers.forEach(builder::header);
        HttpResponse<String> resp = httpClient.send(builder.build(), HttpResponse.BodyHandlers.ofString());
        return gson.fromJson(resp.body(), Map.class);
    }
}
