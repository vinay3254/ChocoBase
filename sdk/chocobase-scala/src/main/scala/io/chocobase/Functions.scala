package io.chocobase

class Functions(baseUrl: String, headers: Map[String, String]) {
  def invoke(functionName: String, body: ujson.Value = ujson.Obj()): ujson.Value = {
    val url = s"$baseUrl/v1/functions/v1/$functionName"
    val resp = requests.post(url, headers = headers, data = body.render())
    ujson.read(resp.text())
  }
}
