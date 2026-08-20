package io.chocobase;

import com.google.gson.Gson;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.util.HashMap;
import java.util.Map;

public class Storage {
    private final String baseUrl;
    private final HttpClient httpClient;
    private final Map<String, String> headers;
    private final Gson gson = new Gson();

    public Storage(String baseUrl, HttpClient httpClient, Map<String, String> headers) {
        this.baseUrl = baseUrl;
        this.httpClient = httpClient;
        this.headers = headers;
    }

    public StorageBucket from(String bucket) {
        return new StorageBucket(baseUrl, bucket, httpClient, headers);
    }

    public static class StorageBucket {
        private final String baseUrl;
        private final String bucket;
        private final HttpClient httpClient;
        private final Map<String, String> headers;
        private final Gson gson = new Gson();

        public StorageBucket(String baseUrl, String bucket, HttpClient httpClient, Map<String, String> headers) {
            this.baseUrl = baseUrl;
            this.bucket = bucket;
            this.httpClient = httpClient;
            this.headers = headers;
        }

        public String createSignedUrl(String path, int expiresIn) throws Exception {
            String url = baseUrl + "/v1/storage/v1/object/sign/" + bucket + "/" + path;
            Map<String, Integer> payload = new HashMap<>();
            payload.put("expires_in", expiresIn);

            HttpRequest.Builder builder = HttpRequest.newBuilder()
                    .uri(URI.create(url))
                    .header("Content-Type", "application/json")
                    .POST(HttpRequest.BodyPublishers.ofString(gson.toJson(payload)));

            headers.forEach(builder::header);
            HttpResponse<String> resp = httpClient.send(builder.build(), HttpResponse.BodyHandlers.ofString());
            Map<String, Object> map = gson.fromJson(resp.body(), Map.class);
            if (map != null && map.containsKey("signed_url")) {
                return baseUrl + map.get("signed_url");
            }
            return null;
        }
    }
}
