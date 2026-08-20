package io.chocobase.example

import io.chocobase.createClient

fun main() {
    println("🍫 ChocoBase Kotlin / Android Quickstart")

    val client = createClient("http://localhost:8080", "anon_dev_token")

    // 1. Auth: Sign up
    val auth = client.auth.signUp("kotlin_dev", "secure_password_123")
    println("Auth User: ${auth.user?.username ?: "anon"}")

    // 2. Database: PostgREST Query
    val rows = client.from("profiles").select("id, username, email").limit(5).execute()
    println("Profiles: $rows")

    // 3. Storage: Signed URL
    val signedUrl = client.storage.from("media").createSignedUrl("banner.png", 3600)
    println("Signed download URL: $signedUrl")

    // 4. Edge Functions
    val result = client.functions.invoke("calculate", mapOf("x" to 10, "y" to 20))
    println("Function result: $result")

    println("✅ Kotlin Quickstart completed successfully!")
}
