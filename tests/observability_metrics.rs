use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_health_metrics_and_jwks_endpoints() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    // 1. Health check
    let (status, _, body) = send_raw_request(addr, "GET", "/health", "").await;
    assert_eq!(status, 200);
    let health_json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(health_json["status"], "healthy");
    assert_eq!(health_json["engine"], "ChocoBase");

    // 2. Prometheus Metrics
    let (m_status, m_ct, m_body) = send_raw_request(addr, "GET", "/metrics", "").await;
    assert_eq!(m_status, 200);
    assert!(m_ct.contains("text/plain"));
    assert!(m_body.contains("chocobase_http_requests_total"));
    assert!(m_body.contains("chocobase_uptime_seconds"));

    // 3. Public JWKS Key Distribution
    let (jwks_status, _, jwks_body) =
        send_raw_request(addr, "GET", "/.well-known/jwks.json", "").await;
    assert_eq!(jwks_status, 200);
    let jwks_json: serde_json::Value = serde_json::from_str(&jwks_body).unwrap();
    let keys = jwks_json["keys"].as_array().unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0]["kid"], "k1");
    assert_eq!(keys[0]["alg"], "HS256");

    server.shutdown();
}

async fn send_raw_request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: &str,
) -> (u16, String, String) {
    let mut socket = TcpStream::connect(addr).await.unwrap();

    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );

    socket.write_all(req.as_bytes()).await.unwrap();
    socket.flush().await.unwrap();

    let mut response_buf = Vec::new();
    socket.read_to_end(&mut response_buf).await.unwrap();

    let s = String::from_utf8_lossy(&response_buf);
    let status_line = s.lines().next().unwrap_or("");
    let status_code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(500);

    let content_type = s
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("content-type:"))
        .unwrap_or("")
        .to_string();

    let body_start = s.find("\r\n\r\n").map(|i| i + 4).unwrap_or(s.len());
    let resp_body = s[body_start..].to_string();

    (status_code, content_type, resp_body)
}
