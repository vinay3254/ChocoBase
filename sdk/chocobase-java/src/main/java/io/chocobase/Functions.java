package io.chocobase;

import com.google.gson.Gson;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.util.Map;

public class Functions {
    private final String baseUrl;
    private final HttpClient httpClient;
    private final Map<String, String> headers;
    private final Gson gson = new Gson();

    public Functions(String baseUrl, HttpClient httpClient, Map<String, String> headers) {
        this.baseUrl = baseUrl;
        this.httpClient = httpClient;
        this.headers = headers;
    }

    public Object invoke(String functionName, Object body) throws Exception {
        String json = body != null ? gson.toJson(body) : "{}";
        HttpRequest.Builder builder = HttpRequest.newBuilder()
                .uri(URI.create(baseUrl + "/v1/functions/v1/" + functionName))
                .header("Content-Type", "application/json")
                .POST(HttpRequest.BodyPublishers.ofString(json));

        headers.forEach(builder::header);
        HttpResponse<String> resp = httpClient.send(builder.build(), HttpResponse.BodyHandlers.ofString());
        return gson.fromJson(resp.body(), Object.class);
    }
}
