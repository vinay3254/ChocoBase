package io.chocobase

class StorageBucket(baseUrl: String, bucket: String, headers: Map[String, String]) {
  def createSignedUrl(path: String, expiresIn: Int = 3600): Option[String] = {
    val url = s"$baseUrl/v1/storage/v1/object/sign/$bucket/$path"
    val payload = ujson.Obj("expires_in" -> expiresIn).render()
    val resp = requests.post(url, headers = headers, data = payload)
    val json = ujson.read(resp.text())
    json.obj.get("signed_url").map(v => s"$baseUrl${v.str}")
  }
}

class Storage(baseUrl: String, headers: Map[String, String]) {
  def from(bucket: String): StorageBucket = new StorageBucket(baseUrl, bucket, headers)
}
