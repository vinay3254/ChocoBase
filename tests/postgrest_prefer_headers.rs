//! Integration test for PostgREST Prefer headers (count=exact and return=minimal/representation).

use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_prefer_count_and_return_representation() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    // Create table
    db.execute("CREATE TABLE tasks (id INTEGER PRIMARY KEY, title TEXT, done BOOLEAN)").unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    // 1. Insert 3 tasks with Prefer: return=representation
    let insert_body = r#"[{"id": 1, "title": "Buy Milk", "done": false}, {"id": 2, "title": "Deploy ChocoBase", "done": true}, {"id": 3, "title": "Write Spec", "done": true}]"#;
    let (resp1_hdrs, resp1_body) = send_req(addr, "POST", "/rest/v1/tasks", insert_body, Some("return=representation")).await;
    assert!(resp1_hdrs.contains("201 Created"));
    assert!(resp1_hdrs.contains("Preference-Applied: return=representation"));
    let inserted_arr: serde_json::Value = serde_json::from_str(&resp1_body).unwrap();
    assert_eq!(inserted_arr.as_array().unwrap().len(), 3);

    // 2. Query with offset=1, limit=1 and Prefer: count=exact
    let (resp2_hdrs, resp2_body) = send_req(addr, "GET", "/rest/v1/tasks?offset=1&limit=1", "", Some("count=exact")).await;
    assert!(resp2_hdrs.contains("200 OK"));
    assert!(resp2_hdrs.contains("Content-Range: 1-1/3"));
    assert!(resp2_hdrs.contains("Range-Unit: items"));
    assert!(resp2_hdrs.contains("Preference-Applied: count=exact"));
    let page_arr: serde_json::Value = serde_json::from_str(&resp2_body).unwrap();
    assert_eq!(page_arr.as_array().unwrap().len(), 1);
    assert_eq!(page_arr[0]["id"], 2);

    // 3. Update with Prefer: return=minimal
    let (resp3_hdrs, _) = send_req(addr, "PATCH", "/rest/v1/tasks?id=eq.1", r#"{"done": true}"#, Some("return=minimal")).await;
    assert!(resp3_hdrs.contains("204 No Content"));
    assert!(resp3_hdrs.contains("Preference-Applied: return=minimal"));

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
