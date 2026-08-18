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
) -> (u16, serde_json::Value) {
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

    let resp_str = String::from_utf8_lossy(&buf);
    let mut lines = resp_str.lines();
    let status_line = lines.next().unwrap();
    let status_code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();

    let body_idx = resp_str.find("\r\n\r\n").unwrap() + 4;
    let json_body: serde_json::Value =
        serde_json::from_str(&resp_str[body_idx..]).unwrap_or(serde_json::Value::Null);

    (status_code, json_body)
}

#[tokio::test]
async fn test_serverless_functions_deploy_and_invoke() {
    let file = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(file.path()).unwrap();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (_server, bound_addr) = HttpServer::bind(addr, db).await.unwrap();

    // 1. Unauthenticated deploy should be rejected with 403 Forbidden
    let (code, _) = send_http_request(
        bound_addr,
        "POST",
        "/v1/functions/v1/deploy",
        None,
        Some(r#"{"name": "greeter", "runtime": "transform"}"#),
    )
    .await;
    assert_eq!(code, 403);

    // 2. Create admin user
    let (code, _) = send_http_request(
        bound_addr,
        "POST",
        "/v1/auth/signup",
        None,
        Some(r#"{"username": "func_admin", "password": "rootpassword"}"#),
    )
    .await;
    assert_eq!(code, 201);

    // Elevate to admin
    let (code, _) = send_http_request(
        bound_addr,
        "POST",
        "/v1/sql",
        None,
        Some(r#"{"sql": "UPDATE _users SET role = 'admin' WHERE username = 'func_admin'"}"#),
    )
    .await;
    assert_eq!(code, 200);

    let (code, login_res) = send_http_request(
        bound_addr,
        "POST",
        "/v1/auth/token",
        None,
        Some(r#"{"username": "func_admin", "password": "rootpassword"}"#),
    )
    .await;
    assert_eq!(code, 200);
    let admin_token = login_res["access_token"].as_str().unwrap();

    // 3. Deploy function with admin token
    let deploy_payload = r#"{
        "name": "greeter",
        "runtime": "transform",
        "script": "return { message: 'Hello ' + input.name }",
        "verify_jwt": false,
        "timeout_ms": 3000
    }"#;
    let (code, deploy_res) = send_http_request(
        bound_addr,
        "POST",
        "/v1/functions/v1/deploy",
        Some(&format!("Bearer {admin_token}")),
        Some(deploy_payload),
    )
    .await;
    assert_eq!(code, 201);
    assert_eq!(deploy_res["status"], "deployed");
    assert_eq!(deploy_res["function"]["name"], "greeter");

    // 4. List deployed functions
    let (code, list_res) =
        send_http_request(bound_addr, "GET", "/v1/functions/v1", None, None).await;
    assert_eq!(code, 200);
    let funcs = list_res.as_array().unwrap();
    assert_eq!(funcs.len(), 1);
    assert_eq!(funcs[0]["name"], "greeter");

    // 5. Invoke function
    let invoke_payload = r#"{"echo": "Antigravity Assistant", "name": "World"}"#;
    let (code, invoke_res) = send_http_request(
        bound_addr,
        "POST",
        "/v1/functions/v1/greeter",
        None,
        Some(invoke_payload),
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(invoke_res["function"], "greeter");
    assert_eq!(invoke_res["status"], "executed");
    assert_eq!(invoke_res["echo"], "Antigravity Assistant");
}
