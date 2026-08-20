package io.chocobase

class Postgrest(baseUrl: String, table: String, headers: Map[String, String], params: Map[String, String] = Map.empty) {
  def select(columns: String = "*"): Postgrest =
    new Postgrest(baseUrl, table, headers, params + ("select" -> columns))

  def eq(column: String, value: Any): Postgrest =
    new Postgrest(baseUrl, table, headers, params + (column -> s"eq.$value"))

  def limit(count: Int): Postgrest =
    new Postgrest(baseUrl, table, headers, params + ("limit" -> count.toString))

  def execute(): ujson.Value = {
    val queryStr = params.map { case (k, v) => s"$k=$v" }.mkString("&")
    val url = s"$baseUrl/rest/v1/$table" + (if (queryStr.nonEmpty) s"?$queryStr" else "")
    val resp = requests.get(url, headers = headers)
    ujson.read(resp.text())
  }
}
