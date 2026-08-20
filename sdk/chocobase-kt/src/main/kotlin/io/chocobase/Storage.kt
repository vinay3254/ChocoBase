package io.chocobase

import com.google.gson.Gson
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody

class StorageBucket(
    private val baseUrl: String,
    private val bucket: String,
    private val httpClient: OkHttpClient,
    private val headers: Map<String, String>
) {
    private val gson = Gson()
    private val jsonType = "application/json; charset=utf-8".toMediaType()

    fun createSignedUrl(path: String, expiresIn: Int = 3600): String? {
        val payload = gson.toJson(mapOf("expires_in" to expiresIn))
        val builder = Request.Builder()
            .url("$baseUrl/v1/storage/v1/object/sign/$bucket/$path")
            .post(payload.toRequestBody(jsonType))

        headers.forEach { (k, v) -> builder.addHeader(k, v) }

        httpClient.newCall(builder.build()).execute().use { response ->
            if (response.isSuccessful) {
                val body = response.body?.string() ?: return null
                val map = gson.fromJson(body, Map::class.java)
                val signedUrl = map["signed_url"] as? String ?: return null
                return "$baseUrl$signedUrl"
            }
            return null
        }
    }
}

class StorageClient(
    private val baseUrl: String,
    private val httpClient: OkHttpClient,
    private val headers: Map<String, String>
) {
    fun from(bucket: String): StorageBucket {
        return StorageBucket(baseUrl, bucket, httpClient, headers)
    }
}
