//! Integration test for GoTrue Auth API via /auth/v1/ route prefix.

use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_auth_v1_routing() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    // 1. Sign up user via /auth/v1/signup with email
    let signup_payload = r#"{"email": "alex@example.com", "password": "SecretPassword123!"}"#;
    let res_signup = send_post(addr, "/auth/v1/signup", signup_payload, None).await;
    assert_eq!(res_signup["status"], "ok");
    assert_eq!(res_signup["user"]["username"], "alex@example.com");
    let access_token = res_signup["access_token"].as_str().unwrap();

    // 2. Sign in via /auth/v1/token
    let token_payload = r#"{"email": "alex@example.com", "password": "SecretPassword123!"}"#;
    let res_token = send_post(addr, "/auth/v1/token", token_payload, None).await;
    assert!(res_token["access_token"].is_string());
    assert_eq!(res_token["user"]["username"], "alex@example.com");

    // 3. Get user session via /auth/v1/user with Authorization header
    let res_user = send_get_auth(addr, "/auth/v1/user", access_token).await;
    assert_eq!(res_user["aud"], "authenticated");
    assert_eq!(res_user["email"], "alex@example.com");

    server.shutdown();
}

async fn send_post(addr: std::net::SocketAddr, path: &str, body: &str, auth: Option<&str>) -> serde_json::Value {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let auth_hdr = if let Some(tok) = auth {
        format!("Authorization: Bearer {tok}\r\n")
    } else {
        String::new()
    };
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\n{auth_hdr}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(req.as_bytes()).await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let resp = String::from_utf8_lossy(&buf);
    let body_str = resp.split("\r\n\r\n").nth(1).unwrap_or("{}");
    serde_json::from_str(body_str).unwrap_or(serde_json::json!({}))
}

async fn send_get_auth(addr: std::net::SocketAddr, path: &str, token: &str) -> serde_json::Value {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let resp = String::from_utf8_lossy(&buf);
    let body_str = resp.split("\r\n\r\n").nth(1).unwrap_or("{}");
    serde_json::from_str(body_str).unwrap_or(serde_json::json!({}))
}
