use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use serde_json::json;
use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_oauth_authorize_and_callback_flow() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    // 1. Request OAuth authorization URL for GitHub
    let auth_req = json!({
        "provider": "github",
        "redirect_uri": "https://myapp.com/oauth/callback"
    })
    .to_string();

    let auth_res = send_request(addr, "POST", "/v1/auth/oauth/authorize", None, &auth_req).await;
    assert_eq!(auth_res["status"], "ok");
    assert_eq!(auth_res["provider"], "github");
    assert!(auth_res["url"].as_str().unwrap().contains("github.com"));

    // 2. Exchange authorization code in callback
    let callback_req = json!({
        "provider": "github",
        "code": "gh_auth_code_12345",
        "username": "octocat",
        "email": "octocat@github.com"
    })
    .to_string();

    let cb_res = send_request(addr, "POST", "/v1/auth/oauth/callback", None, &callback_req).await;
    assert_eq!(cb_res["status"], "ok");
    let access_token = cb_res["access_token"].as_str().unwrap();
    let refresh_token = cb_res["refresh_token"].as_str().unwrap();
    assert!(!access_token.is_empty());
    assert!(!refresh_token.is_empty());
    assert_eq!(cb_res["user"]["username"], "octocat");

    // 3. Verify access token works on authenticated RPC
    let rpc_res = send_request(
        addr,
        "POST",
        "/v1/rpc/current_user",
        Some(access_token),
        "{}",
    )
    .await;
    assert_eq!(rpc_res["role"], "user");

    server.shutdown();
}

async fn send_request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: &str,
) -> serde_json::Value {
    let mut socket = TcpStream::connect(addr).await.unwrap();

    let auth_header = match token {
        Some(t) => format!("Authorization: Bearer {t}\r\n"),
        None => String::new(),
    };

    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{auth_header}Connection: close\r\n\r\n{body}",
        body.len()
    );

    socket.write_all(req.as_bytes()).await.unwrap();
    socket.flush().await.unwrap();

    let mut response_buf = Vec::new();
    socket.read_to_end(&mut response_buf).await.unwrap();

    let s = String::from_utf8_lossy(&response_buf);
    let body_start = s.find("\r\n\r\n").unwrap() + 4;
    serde_json::from_str(&s[body_start..]).unwrap_or(serde_json::Value::Null)
}
