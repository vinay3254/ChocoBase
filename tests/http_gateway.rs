use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use dbengine::{HttpServer, SharedDatabase};

async fn send_http_request(addr: SocketAddr, method: &str, path: &str, body: Option<&str>) -> (u16, serde_json::Value) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let body_str = body.unwrap_or("");
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body_str}",
        body_str.len()
    );

    stream.write_all(req.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();

    let resp_str = String::from_utf8_lossy(&buf);
    let mut lines = resp_str.lines();
    let status_line = lines.next().unwrap();
    let status_code: u16 = status_line.split_whitespace().nth(1).unwrap().parse().unwrap();

    let body_idx = resp_str.find("\r\n\r\n").unwrap() + 4;
    let json_body: serde_json::Value = serde_json::from_str(&resp_str[body_idx..]).unwrap();

    (status_code, json_body)
}

#[tokio::test]
async fn http_gateway_end_to_end_rest_api() {
    let file = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(file.path()).unwrap();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (_server, bound_addr) = HttpServer::bind(addr, db).await.unwrap();

    // 1. Health check
    let (code, health) = send_http_request(bound_addr, "GET", "/v1/health", None).await;
    assert_eq!(code, 200);
    assert_eq!(health["status"], "healthy");
    assert_eq!(health["engine"], "ChocoBase");

    // 2. Create table via POST /v1/sql
    let (code, res) = send_http_request(
        bound_addr,
        "POST",
        "/v1/sql",
        Some(r#"{"sql": "CREATE TABLE products (id INTEGER PRIMARY KEY, title TEXT, price INTEGER)"}"#),
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(res["status"], "ok");

    // 3. Insert row via POST /v1/sql
    let (code, res) = send_http_request(
        bound_addr,
        "POST",
        "/v1/sql",
        Some(r#"{"sql": "INSERT INTO products (id, title, price) VALUES (1, 'Keyboard', 99), (2, 'Mouse', 49)"}"#),
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(res["status"], "ok");

    // 4. Query rows via POST /v1/sql
    let (code, res) = send_http_request(
        bound_addr,
        "POST",
        "/v1/sql",
        Some(r#"{"sql": "SELECT title, price FROM products ORDER BY price DESC"}"#),
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(res["status"], "ok");
    assert_eq!(res["result"]["Rows"]["rows"].as_array().unwrap().len(), 2);

    // 5. List tables via GET /v1/tables
    let (code, res) = send_http_request(bound_addr, "GET", "/v1/tables", None).await;
    assert_eq!(code, 200);
    let tables = res["tables"].as_array().unwrap();
    assert!(tables.iter().any(|t| t == "products"));

    // 6. Inspect schema via GET /v1/tables/products
    let (code, res) = send_http_request(bound_addr, "GET", "/v1/tables/products", None).await;
    assert_eq!(code, 200);
    assert_eq!(res["table"], "products");

    // 7. Metrics via GET /v1/metrics
    let (code, metrics) = send_http_request(bound_addr, "GET", "/v1/metrics", None).await;
    assert_eq!(code, 200);
    assert!(metrics["page_count"].as_u64().unwrap() > 0);

    // 8. Admin Studio Dashboard via GET /dashboard
    let mut stream = TcpStream::connect(bound_addr).await.unwrap();
    stream.write_all(b"GET /dashboard HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let resp = String::from_utf8_lossy(&buf);
    assert!(resp.starts_with("HTTP/1.1 200 OK"));
    assert!(resp.contains("ChocoBase Studio"));
}
