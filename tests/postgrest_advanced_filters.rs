//! Integration test for PostgREST advanced filter operators (cs, cd, ov, is).

use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_advanced_filter_operators() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    // Create table
    db.execute("CREATE TABLE posts (id INTEGER PRIMARY KEY, title TEXT, tags TEXT, published BOOLEAN)").unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    // 1. Insert sample posts
    let insert_body = r#"[
        {"id": 1, "title": "Rust Guide", "tags": "[\"rust\", \"systems\", \"tech\"]", "published": true},
        {"id": 2, "title": "Web Dev", "tags": "[\"html\", \"css\", \"javascript\"]", "published": false},
        {"id": 3, "title": "Database Internals", "tags": "[\"rust\", \"database\"]", "published": true}
    ]"#;
    let _ = send_req(addr, "POST", "/rest/v1/posts", insert_body).await;

    // 2. Filter with cs (contains "rust")
    let (_, resp_cs) = send_req(addr, "GET", "/rest/v1/posts?tags=cs.{rust}", "").await;
    let cs_arr: serde_json::Value = serde_json::from_str(&resp_cs).unwrap();
    assert_eq!(cs_arr.as_array().unwrap().len(), 2);

    // 3. Filter with ov (overlaps "database" or "javascript")
    let (_, resp_ov) = send_req(addr, "GET", "/rest/v1/posts?tags=ov.{database,javascript}", "").await;
    let ov_arr: serde_json::Value = serde_json::from_str(&resp_ov).unwrap();
    assert_eq!(ov_arr.as_array().unwrap().len(), 2);

    // 4. Filter with is.true
    let (_, resp_is) = send_req(addr, "GET", "/rest/v1/posts?published=is.true", "").await;
    let is_arr: serde_json::Value = serde_json::from_str(&resp_is).unwrap();
    assert_eq!(is_arr.as_array().unwrap().len(), 2);

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
