use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

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
    let mut lines = resp_str.lines();
    let status_line = lines.next().unwrap();
    let status_code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();

    let body_idx = resp_str
        .find("\r\n\r\n")
        .map(|i| i + 4)
        .unwrap_or(resp_str.len());
    let raw_body = &resp_str[body_idx..];
    let json_body: serde_json::Value =
        serde_json::from_str(raw_body).unwrap_or(serde_json::Value::Null);

    (status_code, raw_body.to_string(), json_body)
}

#[tokio::test]
async fn test_signed_storage_download_urls_and_expiry() {
    let file = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(file.path()).unwrap();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (_server, bound_addr) = HttpServer::bind(addr, db).await.unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claims = dbengine::auth::SessionClaims::new(1, "admin_user", "admin", now + 3600);
    let token = format!(
        "Bearer {}",
        dbengine::auth::sign_jwt(&claims, dbengine::auth::DEFAULT_DEV_JWT_SECRET)
    );

    // 1. Create private bucket
    let (code, _, _) = send_http_request(
        bound_addr,
        "POST",
        "/v1/storage/v1/bucket",
        Some(&token),
        Some(r#"{"id": "confidential", "public": false}"#),
    )
    .await;
    assert_eq!(code, 201);

    // 2. Upload file to confidential bucket
    let (code, _, _) = send_http_request(
        bound_addr,
        "POST",
        "/v1/storage/v1/object/confidential/secret-doc.txt",
        Some(&token),
        Some("Top secret payload content 12345"),
    )
    .await;
    assert_eq!(code, 200);

    // 3. Anonymous direct download of private file without token -> 401 Unauthorized
    let (code, _, _) = send_http_request(
        bound_addr,
        "GET",
        "/v1/storage/v1/object/confidential/secret-doc.txt",
        None,
        None,
    )
    .await;
    assert_eq!(code, 401);

    // 4. Create signed URL valid for 3600 seconds
    let (code, _, sign_res) = send_http_request(
        bound_addr,
        "POST",
        "/v1/storage/v1/object/sign/confidential/secret-doc.txt",
        Some(&token),
        Some(r#"{"expiresIn": 3600}"#),
    )
    .await;
    assert_eq!(code, 200);
    let signed_url = sign_res["signedURL"].as_str().unwrap();
    let token = sign_res["token"].as_str().unwrap();
    let expires = sign_res["expiresAt"].as_u64().unwrap();

    // 5. Anonymous download using valid signed URL -> 200 OK + payload
    let (code, body, _) = send_http_request(bound_addr, "GET", signed_url, None, None).await;
    assert_eq!(code, 200);
    assert_eq!(body, "Top secret payload content 12345");

    // 6. Tampered token -> 401 Unauthorized
    let tampered_url = format!(
        "/v1/storage/v1/object/confidential/secret-doc.txt?token=badtoken123&expires={expires}"
    );
    let (code, _, _) = send_http_request(bound_addr, "GET", &tampered_url, None, None).await;
    assert_eq!(code, 401);

    // 7. Expired timestamp -> 401 Unauthorized
    let expired_url =
        format!("/v1/storage/v1/object/confidential/secret-doc.txt?token={token}&expires=1000");
    let (code, _, _) = send_http_request(bound_addr, "GET", &expired_url, None, None).await;
    assert_eq!(code, 401);
}
