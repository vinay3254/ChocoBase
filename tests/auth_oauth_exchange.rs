use dbengine::auth::oauth::{
    exchange_code_for_token, generate_authorize_url, resolve_user_profile,
};
use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_oauth_flow_token_exchange_and_profile_resolution() {
    // 1. Authorize URL generation
    let auth_resp =
        generate_authorize_url("github", "https://app.example.com/auth/callback").unwrap();
    assert_eq!(auth_resp.provider, "github");
    assert!(auth_resp.url.contains("github.com/login/oauth/authorize"));
    assert!(auth_resp.url.contains("client_id=chocobase_client"));

    // 2. Token exchange
    let token_resp = exchange_code_for_token(
        "github",
        "gh_code_valid_12345",
        "https://app.example.com/auth/callback",
        Some("gh_secret_67890"),
    )
    .unwrap();
    assert_eq!(token_resp.token_type, "bearer");
    assert!(token_resp.access_token.contains("gh_code_valid_12345"));

    // 3. User profile resolution
    let profile = resolve_user_profile(
        "github",
        &token_resp.access_token,
        Some("octocat@github.com"),
        Some("octocat"),
    )
    .unwrap();
    assert_eq!(profile.username, "octocat");
    assert_eq!(profile.email, "octocat@github.com");
    assert_eq!(profile.provider_user_id, "github_octocat");
    assert!(profile.avatar_url.unwrap().contains("octocat.png"));

    // 4. Full HTTP OAuth Callback integration endpoint
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db.clone())
        .await
        .unwrap();

    let callback_body = serde_json::json!({
        "provider": "github",
        "code": "gh_code_valid_12345",
        "email": "octocat@github.com",
        "username": "octocat"
    });

    let (status, _, resp_body) =
        send_post(addr, "/v1/auth/oauth/callback", &callback_body.to_string()).await;
    assert_eq!(status, 200);

    let json: serde_json::Value = serde_json::from_str(&resp_body).unwrap();
    assert!(json.get("access_token").is_some());
    assert!(json.get("refresh_token").is_some());
    assert_eq!(json["user"]["username"], "octocat");
    assert_eq!(json["user"]["role"], "user");

    server.shutdown();
}

async fn send_post(addr: SocketAddr, path: &str, body: &str) -> (u16, Vec<String>, String) {
    let mut socket = TcpStream::connect(addr).await.unwrap();

    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );

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
