package io.chocobase;

import java.net.http.HttpClient;
import java.util.HashMap;
import java.util.Map;

public class ChocoClient {
    private final String url;
    private final String apiKey;
    private final HttpClient httpClient;
    private final Map<String, String> headers;

    public final Auth auth;
    public final Storage storage;
    public final Functions functions;

    public ChocoClient(String url, String apiKey) {
        this(url, apiKey, new HashMap<>());
    }

    public ChocoClient(String url, String apiKey, Map<String, String> customHeaders) {
        this.url = url.replaceAll("/+$", "");
        this.apiKey = apiKey;
        this.httpClient = HttpClient.newHttpClient();

        this.headers = new HashMap<>();
        this.headers.put("apikey", apiKey);
        this.headers.put("Authorization", "Bearer " + apiKey);
        this.headers.put("Content-Type", "application/json");
        if (customHeaders != null) {
            this.headers.putAll(customHeaders);
        }

        this.auth = new Auth(this.url, this.httpClient, this.headers);
        this.storage = new Storage(this.url, this.httpClient, this.headers);
        this.functions = new Functions(this.url, this.httpClient, this.headers);
    }

    public Postgrest from(String table) {
        return new Postgrest(this.url, table, this.httpClient, this.headers);
    }

    public static ChocoClient createClient(String url, String apiKey) {
        return new ChocoClient(url, apiKey);
    }
}
