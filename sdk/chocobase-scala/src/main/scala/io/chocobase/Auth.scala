package io.chocobase

class Auth(baseUrl: String, headers: Map[String, String]) {
  def signUp(username: String, password: String): ujson.Value = {
    val url = s"$baseUrl/v1/auth/signup"
    val payload = ujson.Obj("username" -> username, "password" -> password).render()
    val resp = requests.post(url, headers = headers, data = payload)
    ujson.read(resp.text())
  }

  def signIn(username: String, password: String): ujson.Value = {
    val url = s"$baseUrl/v1/auth/token"
    val payload = ujson.Obj("username" -> username, "password" -> password).render()
    val resp = requests.post(url, headers = headers, data = payload)
    ujson.read(resp.text())
  }
}
