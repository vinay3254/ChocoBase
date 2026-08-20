package io.chocobase

import com.google.gson.Gson
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody

data class User(val id: Long, val username: String, val role: String)

data class AuthResponse(
    val access_token: String? = null,
    val refresh_token: String? = null,
    val user: User? = null,
    val error: String? = null
)

class AuthClient(
    private val url: String,
    private val httpClient: OkHttpClient,
    private val headers: Map<String, String>
) {
    private val gson = Gson()
    private val jsonType = "application/json; charset=utf-8".toMediaType()

    fun signUp(username: String, password: String): AuthResponse {
        val payload = gson.toJson(mapOf("username" to username, "password" to password))
        val builder = Request.Builder().url("$url/v1/auth/signup").post(payload.toRequestBody(jsonType))
        headers.forEach { (k, v) -> builder.addHeader(k, v) }

        httpClient.newCall(builder.build()).execute().use { response ->
            val body = response.body?.string() ?: "{}"
            return gson.fromJson(body, AuthResponse::class.java)
        }
    }

    fun signIn(username: String, password: String): AuthResponse {
        val payload = gson.toJson(mapOf("username" to username, "password" to password))
        val builder = Request.Builder().url("$url/v1/auth/token").post(payload.toRequestBody(jsonType))
        headers.forEach { (k, v) -> builder.addHeader(k, v) }

        httpClient.newCall(builder.build()).execute().use { response ->
            val body = response.body?.string() ?: "{}"
            return gson.fromJson(body, AuthResponse::class.java)
        }
    }
}
