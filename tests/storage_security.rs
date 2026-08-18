use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use dbengine::auth::{sign_jwt, SessionClaims, DEFAULT_DEV_JWT_SECRET};
use dbengine::{HttpServer, SharedDatabase};

async fn send_http_request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    auth_header: Option<&str>,
    body: Option<&str>,
) -> (u16, String, serde_json::Value) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let body_str = body.unwrap_or("");
    let auth = match auth_header {
        Some(h) => format!("Authorization: {h}\r\n"),
        None => String::new(),
    };
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body_str}",
        body_str.len()
    );

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

    let body_idx = resp_str.find("\r\n\r\n").unwrap() + 4;
    let raw_body = resp_str[body_idx..].to_string();
    let json_body: serde_json::Value =
        serde_json::from_str(&raw_body).unwrap_or(serde_json::Value::Null);

    (status_code, raw_body, json_body)
}

#[tokio::test]
async fn test_per_user_storage_authorization_and_signed_urls() {
    let file = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(file.path()).unwrap();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (_server, bound_addr) = HttpServer::bind(addr, db).await.unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Create tokens for User A (ID 10) and User B (ID 20)
    let claims_a = SessionClaims::new(10, "alice", "user", now + 3600);
    let token_a = format!("Bearer {}", sign_jwt(&claims_a, DEFAULT_DEV_JWT_SECRET));

    let claims_b = SessionClaims::new(20, "bob", "user", now + 3600);
    let token_b = format!("Bearer {}", sign_jwt(&claims_b, DEFAULT_DEV_JWT_SECRET));

    // 1. Create a PRIVATE bucket
    let bucket_body = r#"{"id": "vault", "public": false}"#;
    let (code, _, _) = send_http_request(
        bound_addr,
        "POST",
        "/v1/storage/v1/bucket",
        Some(&token_a),
        Some(bucket_body),
    )
    .await;
    assert_eq!(code, 201);

    // 2. User A uploads a private document
    let user_a_data = "Alice's Confidental Financial Records";
    let (code, _, _) = send_http_request(
        bound_addr,
        "POST",
        "/v1/storage/v1/object/vault/alice_tax.txt",
        Some(&token_a),
        Some(user_a_data),
    )
    .await;
    assert_eq!(code, 200);

    // 3. User A can download their own private file
    let (code, body, _) = send_http_request(
        bound_addr,
        "GET",
        "/v1/storage/v1/object/vault/alice_tax.txt",
        Some(&token_a),
        None,
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(body, user_a_data);

    // 4. CROSS-USER UNAUTHORIZED ACCESS: User B attempts to download User A's private file -> 403 Forbidden!
    let (code, _, _) = send_http_request(
        bound_addr,
        "GET",
        "/v1/storage/v1/object/vault/alice_tax.txt",
        Some(&token_b),
        None,
    )
    .await;
    assert_eq!(code, 403);

    // 5. CROSS-USER DELETION: User B attempts to delete User A's file -> 403 Forbidden!
    let (code, _, _) = send_http_request(
        bound_addr,
        "DELETE",
        "/v1/storage/v1/object/vault/alice_tax.txt",
        Some(&token_b),
        None,
    )
    .await;
    assert_eq!(code, 403);

    // 6. User A generates a Signed URL for their file
    let sign_req = r#"{"expiresIn": 60}"#;
    let (code, _, sign_res) = send_http_request(
        bound_addr,
        "POST",
        "/v1/storage/v1/object/sign/vault/alice_tax.txt",
        Some(&token_a),
        Some(sign_req),
    )
    .await;
    assert_eq!(code, 200);
    let signed_url = sign_res["signedURL"].as_str().unwrap();

    // 7. Signed URL allows unauthenticated client to download valid file
    let (code, body, _) = send_http_request(bound_addr, "GET", signed_url, None, None).await;
    assert_eq!(code, 200);
    assert_eq!(body, user_a_data);

    // 8. Tampered signed URL is rejected -> 401 Unauthorized
    let tampered_url = format!("{signed_url}tampered");
    let (code, _, _) = send_http_request(bound_addr, "GET", &tampered_url, None, None).await;
    assert_eq!(code, 401);

    // 9. User B cannot generate signed URL for User A's private file -> 403 Forbidden
    let (code, _, _) = send_http_request(
        bound_addr,
        "POST",
        "/v1/storage/v1/object/sign/vault/alice_tax.txt",
        Some(&token_b),
        Some(sign_req),
    )
    .await;
    assert_eq!(code, 403);
}
