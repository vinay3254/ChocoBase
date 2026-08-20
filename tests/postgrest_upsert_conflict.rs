//! Integration test for PostgREST upsert with on_conflict and resolution=ignore-duplicates.

use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_postgrest_upsert_on_conflict() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    // Create table
    db.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT, display_name TEXT)").unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    // 1. Initial Insert
    let insert_body = r#"{"id": 1, "email": "alice@example.com", "display_name": "Alice"}"#;
    let (resp1_hdrs, _) = send_req(addr, "POST", "/rest/v1/users", insert_body, None).await;
    assert!(resp1_hdrs.contains("201 Created"));

    // 2. Upsert with on_conflict=id and merge-duplicates
    let upsert_body = r#"{"id": 1, "email": "alice@example.com", "display_name": "Alice Updated"}"#;
    let (resp2_hdrs, resp2_body) = send_req(addr, "POST", "/rest/v1/users?on_conflict=id", upsert_body, Some("resolution=merge-duplicates")).await;
    assert!(resp2_hdrs.contains("201 Created"));
    let upsert_res: serde_json::Value = serde_json::from_str(&resp2_body).unwrap();
    assert_eq!(upsert_res[0]["display_name"], "Alice Updated");

    // 3. Insert with resolution=ignore-duplicates (duplicate id)
    let dup_body = r#"{"id": 1, "email": "alice@example.com", "display_name": "Should Be Ignored"}"#;
    let (resp3_hdrs, _) = send_req(addr, "POST", "/rest/v1/users?on_conflict=id", dup_body, Some("resolution=ignore-duplicates")).await;
    assert!(resp3_hdrs.contains("201 Created"));

    // 4. Verify display_name remained "Alice Updated"
    let (_, get_body) = send_req(addr, "GET", "/rest/v1/users?id=eq.1", "", None).await;
    let get_res: serde_json::Value = serde_json::from_str(&get_body).unwrap();
    assert_eq!(get_res[0]["display_name"], "Alice Updated");

    server.shutdown();
}

async fn send_req(addr: std::net::SocketAddr, method: &str, path: &str, body: &str, prefer: Option<&str>) -> (String, String) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let pref_hdr = if let Some(p) = prefer {
        format!("Prefer: {p}\r\n")
    } else {
        String::new()
    };
    let cl_hdr = if method == "POST" || method == "PATCH" || method == "PUT" {
        format!("Content-Length: {}\r\nContent-Type: application/json\r\n", body.len())
    } else {
        String::new()
    };
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\n{pref_hdr}{cl_hdr}Connection: close\r\n\r\n{body}"
    );
    stream.write_all(req.as_bytes()).await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let resp = String::from_utf8_lossy(&buf).to_string();
    let (headers, body_str) = resp.split_once("\r\n\r\n").unwrap_or((&resp, ""));
    (headers.to_string(), body_str.to_string())
}
