use dbengine::engine::SharedDatabase;
use dbengine::http::rate_limit::RateLimiter;
use dbengine::http::HttpServer;
use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_rate_limiter_unit_and_sliding_window() {
    let limiter = RateLimiter::new();

    // Max 2 requests in 5-second window
    assert!(limiter.check_rate_limit("client:1", 2, 5).is_ok());
    assert!(limiter.check_rate_limit("client:1", 2, 5).is_ok());

    // 3rd request should fail with retry_after <= 5
    let err = limiter.check_rate_limit("client:1", 2, 5);
    assert!(err.is_err());
    let retry_after = err.unwrap_err();
    assert!((1..=5).contains(&retry_after));

    // Different key should not be throttled
    assert!(limiter.check_rate_limit("client:2", 2, 5).is_ok());

    // Clear rate limits
    limiter.clear();
    assert!(limiter.check_rate_limit("client:1", 2, 5).is_ok());
}

#[tokio::test]
async fn test_http_rate_limiting_throttle_and_retry_after() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db.clone())
        .await
        .unwrap();

    // 1st request -> 200 OK
    let (s1, _, _) = send_http_get(addr, "/v1/test/rate-limit").await;
    assert_eq!(s1, 200);

    // 2nd request -> 200 OK
    let (s2, _, _) = send_http_get(addr, "/v1/test/rate-limit").await;
    assert_eq!(s2, 200);

    // 3rd request -> 429 Too Many Requests
    let (s3, headers3, body3) = send_http_get(addr, "/v1/test/rate-limit").await;
    assert_eq!(s3, 429, "expected HTTP 429 Too Many Requests");

    // Must include Retry-After header
    let has_retry_after = headers3
        .iter()
        .any(|h| h.to_lowercase().starts_with("retry-after:"));
    assert!(
        has_retry_after,
        "expected Retry-After header in 429 response"
    );

    let json: serde_json::Value = serde_json::from_str(&body3).unwrap();
    assert!(json.get("error").is_some());
    assert!(json.get("retry_after").is_some());

    server.shutdown();
}

async fn send_http_get(addr: SocketAddr, path: &str) -> (u16, Vec<String>, String) {
    let mut socket = TcpStream::connect(addr).await.unwrap();

    let req = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");

    socket.write_all(req.as_bytes()).await.unwrap();
    socket.flush().await.unwrap();

    let mut response_buf = Vec::new();
    socket.read_to_end(&mut response_buf).await.unwrap();

    let s = String::from_utf8_lossy(&response_buf);
    let mut header_lines = Vec::new();
    let mut lines = s.lines();

    let status_line = lines.next().unwrap_or("");
    let status_code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(500);

    for line in lines.by_ref() {
        if line.is_empty() {
            break;
        }
        header_lines.push(line.to_string());
    }

    let body_start = s.find("\r\n\r\n").map(|i| i + 4).unwrap_or(s.len());
    let resp_body = s[body_start..].to_string();

    (status_code, header_lines, resp_body)
}
