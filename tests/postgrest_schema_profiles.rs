//! Integration test for PostgREST Accept-Profile and Content-Profile headers.

use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_postgrest_schema_profiles() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    // Create table with schema prefix
    db.execute("CREATE TABLE analytics.events (id INTEGER PRIMARY KEY, name TEXT)").unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    // 1. Insert via Content-Profile: analytics
    let insert_body = r#"{"id": 1, "name": "page_view"}"#;
    let (resp1_hdrs, _) = send_req(addr, "POST", "/rest/v1/events", insert_body, Some("analytics"), None).await;
    assert!(resp1_hdrs.contains("201 Created"));
    assert!(resp1_hdrs.contains("Content-Profile: analytics"));

    // 2. Query via Accept-Profile: analytics
    let (resp2_hdrs, resp2_body) = send_req(addr, "GET", "/rest/v1/events", "", None, Some("analytics")).await;
    assert!(resp2_hdrs.contains("200 OK"));
    assert!(resp2_hdrs.contains("Content-Profile: analytics"));
    let arr: serde_json::Value = serde_json::from_str(&resp2_body).unwrap();
    assert_eq!(arr.as_array().unwrap().len(), 1);
    assert_eq!(arr[0]["name"], "page_view");

    server.shutdown();
}

async fn send_req(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    body: &str,
    content_profile: Option<&str>,
    accept_profile: Option<&str>,
) -> (String, String) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let cp_hdr = if let Some(p) = content_profile {
        format!("Content-Profile: {p}\r\n")
    } else {
        String::new()
    };
    let ap_hdr = if let Some(p) = accept_profile {
        format!("Accept-Profile: {p}\r\n")
    } else {
        String::new()
    };
    let cl_hdr = if method == "POST" || method == "PATCH" || method == "PUT" {
        format!("Content-Length: {}\r\nContent-Type: application/json\r\n", body.len())
    } else {
        String::new()
    };
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\n{cp_hdr}{ap_hdr}{cl_hdr}Connection: close\r\n\r\n{body}"
    );
    stream.write_all(req.as_bytes()).await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let resp = String::from_utf8_lossy(&buf).to_string();
    let (headers, body_str) = resp.split_once("\r\n\r\n").unwrap_or((&resp, ""));
    (headers.to_string(), body_str.to_string())
}
