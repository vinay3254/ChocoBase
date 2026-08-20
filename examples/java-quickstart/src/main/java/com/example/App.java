package com.example;

import io.chocobase.ChocoClient;
import java.util.List;
import java.util.Map;

public class App {
    public static void main(String[] args) throws Exception {
        System.out.println("🍫 ChocoBase Java & Spring Boot Quickstart");

        ChocoClient client = ChocoClient.createClient("http://localhost:8080", "anon_dev_token");

        // 1. Auth: Sign up
        Map<String, Object> auth = client.auth.signUp("java_dev", "secure_password_123");
        System.out.println("Auth User: " + auth.get("user"));

        // 2. PostgREST: Query table
        List<Map<String, Object>> rows = client.from("users").select("id, username, role").limit(5).execute();
        System.out.println("Fetched " + (rows != null ? rows.size() : 0) + " user rows.");

        // 3. Storage: Signed URL
        String signedUrl = client.storage.from("documents").createSignedUrl("invoice.pdf", 3600);
        System.out.println("Signed download URL: " + signedUrl);

        // 4. Edge Functions: Invoke
        Object res = client.functions.invoke("health-check", Map.of("ping", true));
        System.out.println("Function result: " + res);

        System.out.println("✅ Java Quickstart completed successfully!");
    }
}
