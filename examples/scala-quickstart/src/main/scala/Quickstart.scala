package com.example

import io.chocobase.ChocoClient

object Quickstart extends App {
  println("🍫 ChocoBase Scala Quickstart (Akka / Play / Spark / Big Data)")

  val client = ChocoClient.create("http://localhost:8080", "anon_dev_token")

  // 1. Auth: Sign up
  val auth = client.auth.signUp("scala_dev", "secure_password_123")
  println(s"Auth User: ${auth("user")("username")}")

  // 2. PostgREST: Query table
  val rows = client.from("metrics").select("id, timestamp, cpu_load").limit(5).execute()
  println(s"Retrieved metrics: $rows")

  // 3. Storage: Signed URL
  val signedUrl = client.storage.from("datasets").createSignedUrl("data.parquet", 3600)
  println(s"Signed download URL: $signedUrl")

  // 4. Edge Functions: Invoke
  val res = client.functions.invoke("calculate-aggregate", ujson.Obj("partition" -> 4))
  println(s"Function result: $res")

  println("✅ Scala Quickstart completed successfully!")
}
