package io.chocobase

import com.google.gson.Gson
import com.google.gson.reflect.TypeToken
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import okhttp3.OkHttpClient
import okhttp3.Request

class QueryBuilder(
    private val baseUrl: String,
    private val table: String,
    private val httpClient: OkHttpClient,
    private val headers: Map<String, String>
) {
    private val params = mutableMapOf<String, String>()
    private val gson = Gson()

    fun select(columns: String = "*"): QueryBuilder {
        params["select"] = columns
        return this
    }

    fun eq(column: String, value: Any): QueryBuilder {
        params[column] = "eq.$value"
        return this
    }

    fun limit(count: Int): QueryBuilder {
        params["limit"] = count.toString()
        return this
    }

    fun execute(): List<Map<String, Any>> {
        val urlBuilder = "$baseUrl/rest/v1/$table".toHttpUrlOrNull()?.newBuilder() ?: return emptyList()
        params.forEach { (k, v) -> urlBuilder.addQueryParameter(k, v) }

        val builder = Request.Builder().url(urlBuilder.build()).get()
        headers.forEach { (k, v) -> builder.addHeader(k, v) }

        httpClient.newCall(builder.build()).execute().use { response ->
            val body = response.body?.string() ?: "[]"
            val type = object : TypeToken<List<Map<String, Any>>>() {}.type
            return try {
                gson.fromJson(body, type)
            } catch (_: Exception) {
                emptyList()
            }
        }
    }
}
