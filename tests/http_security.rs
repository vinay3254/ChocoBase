use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use dbengine::{HttpServer, SharedDatabase};

async fn send_raw_http(addr: SocketAddr, req: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(req.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();

    let resp_str = String::from_utf8_lossy(&buf).to_string();
    let status_code: u16 = resp_str
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();

    (status_code, resp_str)
}

#[tokio::test]
async fn test_cors_preflight_and_security_headers() {
    let file = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(file.path()).unwrap();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (_server, bound_addr) = HttpServer::bind(addr, db).await.unwrap();

    // 1. Health check returns security headers
    let req = "GET /v1/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    let (code, resp) = send_raw_http(bound_addr, req).await;
    assert_eq!(code, 200);
    assert!(resp.contains("X-Content-Type-Options: nosniff"));
    assert!(resp.contains("X-Frame-Options: DENY"));

    // 2. Preflight OPTIONS request
    let opt_req = "OPTIONS /v1/sql HTTP/1.1\r\nHost: localhost\r\nOrigin: http://localhost:3000\r\nAccess-Control-Request-Method: POST\r\nConnection: close\r\n\r\n";
    let (code, resp) = send_raw_http(bound_addr, opt_req).await;
    assert_eq!(code, 204);
    assert!(resp.contains("Access-Control-Allow-Methods"));
}

#[tokio::test]
async fn test_payload_budget_and_header_limits() {
    let file = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(file.path()).unwrap();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (_server, bound_addr) = HttpServer::bind(addr, db).await.unwrap();

    // 1. Content-Length exceeding 10MB budget -> 413 Payload Too Large
    let huge_req = format!(
        "POST /v1/sql HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        15 * 1024 * 1024
    );
    let (code, resp) = send_raw_http(bound_addr, &huge_req).await;
    assert_eq!(code, 413);
    assert!(resp.contains("payload too large"));
}
