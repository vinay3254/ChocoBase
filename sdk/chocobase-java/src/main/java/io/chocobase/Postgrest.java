package io.chocobase;

import com.google.gson.Gson;
import java.net.URI;
import java.net.URLEncoder;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

public class Postgrest {
    private final String baseUrl;
    private final String table;
    private final HttpClient httpClient;
    private final Map<String, String> headers;
    private final Map<String, String> params = new HashMap<>();
    private final Gson gson = new Gson();

    public Postgrest(String baseUrl, String table, HttpClient httpClient, Map<String, String> headers) {
        this.baseUrl = baseUrl;
        this.table = table;
        this.httpClient = httpClient;
        this.headers = headers;
    }

    public Postgrest select(String columns) {
        params.put("select", columns);
        return this;
    }

    public Postgrest eq(String column, Object value) {
        params.put(column, "eq." + value);
        return this;
    }

    public Postgrest limit(int count) {
        params.put("limit", String.valueOf(count));
        return this;
    }

    public List<Map<String, Object>> execute() throws Exception {
        StringBuilder query = new StringBuilder();
        boolean first = true;
        for (Map.Entry<String, String> entry : params.entrySet()) {
            query.append(first ? "?" : "&");
            query.append(URLEncoder.encode(entry.getKey(), StandardCharsets.UTF_8));
            query.append("=");
            query.append(URLEncoder.encode(entry.getValue(), StandardCharsets.UTF_8));
            first = false;
        }

        String url = baseUrl + "/rest/v1/" + table + query;
        HttpRequest.Builder builder = HttpRequest.newBuilder()
                .uri(URI.create(url))
                .GET();

        headers.forEach(builder::header);
        HttpResponse<String> resp = httpClient.send(builder.build(), HttpResponse.BodyHandlers.ofString());
        return gson.fromJson(resp.body(), List.class);
    }
}
