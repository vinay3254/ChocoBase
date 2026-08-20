package io.chocobase

class ChocoClient(val url: String, val apiKey: String, customHeaders: Map[String, String] = Map.empty) {
  val cleanUrl: String = url.replaceAll("/+$", "")
  val headers: Map[String, String] = Map(
    "apikey" -> apiKey,
    "Authorization" -> s"Bearer $apiKey",
    "Content-Type" -> "application/json"
  ) ++ customHeaders

  val auth: Auth = new Auth(cleanUrl, headers)
  val storage: Storage = new Storage(cleanUrl, headers)
  val functions: Functions = new Functions(cleanUrl, headers)

  def from(table: String): Postgrest = new Postgrest(cleanUrl, table, headers)
}

object ChocoClient {
  def create(url: String, apiKey: String, customHeaders: Map[String, String] = Map.empty): ChocoClient =
    new ChocoClient(url, apiKey, customHeaders)
}
