package io.chocobase

import com.google.gson.Gson
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody

class FunctionsClient(
    private val baseUrl: String,
    private val httpClient: OkHttpClient,
    private val headers: Map<String, String>
) {
    private val gson = Gson()
    private val jsonType = "application/json; charset=utf-8".toMediaType()

    fun invoke(functionName: String, body: Any? = null): Map<String, Any> {
        val payload = if (body != null) gson.toJson(body) else "{}"
        val builder = Request.Builder()
            .url("$baseUrl/v1/functions/v1/$functionName")
            .post(payload.toRequestBody(jsonType))

        headers.forEach { (k, v) -> builder.addHeader(k, v) }

        httpClient.newCall(builder.build()).execute().use { response ->
            val respBody = response.body?.string() ?: "{}"
            return try {
                gson.fromJson(respBody, Map::class.java) as Map<String, Any>
            } catch (_: Exception) {
                mapOf("response" to respBody)
            }
        }
    }
}
