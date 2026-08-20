package io.chocobase

import okhttp3.OkHttpClient

class ChocoClient(
    val url: String,
    val apiKey: String,
    customHeaders: Map<String, String>? = null
) {
    val httpClient = OkHttpClient()
    val headers = mutableMapOf(
        "apikey" to apiKey,
        "Authorization" to "Bearer $apiKey",
        "Content-Type" to "application/json"
    ).apply {
        customHeaders?.let { putAll(it) }
    }

    val auth = AuthClient(url, httpClient, headers)
    val storage = StorageClient(url, httpClient, headers)
    val functions = FunctionsClient(url, httpClient, headers)

    fun from(table: String): QueryBuilder {
        return QueryBuilder(url, table, httpClient, headers)
    }
}

fun createClient(url: String, apiKey: String, customHeaders: Map<String, String>? = null): ChocoClient {
    return ChocoClient(url, apiKey, customHeaders)
}
