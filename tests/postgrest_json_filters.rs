//! Integration test for PostgREST nested and chained JSON arrow filters.

use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_postgrest_json_filters() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    // Create table with JSON column
    db.execute("CREATE TABLE users_profile (id INTEGER PRIMARY KEY, metadata JSON)").unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    // Insert 2 rows
    let insert_body = r#"[
        {"id": 1, "metadata": {"user": {"name": "Alice", "age": 30}}},
        {"id": 2, "metadata": {"user": {"name": "Bob", "age": 25}}}
    ]"#;
    let _ = send_req(addr, "POST", "/rest/v1/users_profile", insert_body).await;

    // 1. Single arrow text filter: metadata->>user (or metadata->user->>name)
    let (_, resp1) = send_req(addr, "GET", "/rest/v1/users_profile?metadata->user->>name=eq.Alice", "").await;
    let arr1: serde_json::Value = serde_json::from_str(&resp1).unwrap();
    assert_eq!(arr1.as_array().unwrap().len(), 1);
    assert_eq!(arr1[0]["id"], 1);

    // 2. Chained filter with not.eq
    let (_, resp2) = send_req(addr, "GET", "/rest/v1/users_profile?metadata->user->>name=not.eq.Alice", "").await;
    let arr2: serde_json::Value = serde_json::from_str(&resp2).unwrap();
    assert_eq!(arr2.as_array().unwrap().len(), 1);
    assert_eq!(arr2[0]["id"], 2);

    server.shutdown();
}

async fn send_req(addr: std::net::SocketAddr, method: &str, path: &str, body: &str) -> (String, String) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let cl_hdr = if method == "POST" || method == "PATCH" || method == "PUT" {
        format!("Content-Length: {}\r\nContent-Type: application/json\r\n", body.len())
    } else {
        String::new()
    };
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\n{cl_hdr}Connection: close\r\n\r\n{body}"
    );
    stream.write_all(req.as_bytes()).await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let resp = String::from_utf8_lossy(&buf).to_string();
    let (headers, body_str) = resp.split_once("\r\n\r\n").unwrap_or((&resp, ""));
    (headers.to_string(), body_str.to_string())
}
