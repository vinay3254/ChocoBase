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
    let json_body: serde_json::Value =
        serde_json::from_str(&resp_str[body_idx..]).unwrap_or(serde_json::Value::Null);

    (status_code, json_body)
}

#[tokio::test]
async fn test_isolated_function_runtime_and_timeout_enforcement() {
    let file = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(file.path()).unwrap();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (_server, bound_addr) = HttpServer::bind(addr, db).await.unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let admin_claims = SessionClaims::new(1, "admin", "admin", now + 3600);
    let admin_token = format!("Bearer {}", sign_jwt(&admin_claims, DEFAULT_DEV_JWT_SECRET));

    // 1. Deploy a real script function
    let script_code = if cfg!(windows) {
        r#"echo {"greeting":"hello from isolated sandbox","env_val":"%TEST_SECRET%"}"#
    } else {
        r#"echo '{"greeting":"hello from isolated sandbox","env_val":"'$TEST_SECRET'"}'"#
    };

    let deploy_payload = serde_json::json!({
        "name": "isolated_greeter",
        "runtime": "process",
        "script": script_code,
        "timeout_ms": 5000,
        "env": {
            "TEST_SECRET": "my_sandboxed_secret"
        },
        "verify_jwt": false
    });

    let (code, deploy_res) = send_http_request(
        bound_addr,
        "POST",
        "/v1/functions/v1/deploy",
        Some(&admin_token),
        Some(&deploy_payload.to_string()),
    )
    .await;
    assert_eq!(code, 201);
    assert_eq!(deploy_res["status"], "deployed");

    // 2. Invoke real function
    let (code, invoke_res) = send_http_request(
        bound_addr,
        "POST",
        "/v1/functions/v1/isolated_greeter",
        None,
        Some(r#"{"msg": "hi"}"#),
    )
    .await;
    assert_eq!(code, 200);
    let output_text = if let Some(g) = invoke_res.get("greeting").and_then(|v| v.as_str()) {
        g.to_string()
    } else {
        invoke_res["output"].as_str().unwrap_or("").to_string()
    };
    assert!(output_text.contains("hello from isolated sandbox"));

    // 3. Deploy a function with a short timeout (150ms) that sleeps for 3 seconds
    let sleep_script = if cfg!(windows) {
        "ping 127.0.0.1 -n 4 >nul && echo {\"status\":\"finished\"}"
    } else {
        "sleep 3 && echo '{\"status\":\"finished\"}'"
    };

    let timeout_deploy = serde_json::json!({
        "name": "sleepy_function",
        "runtime": "process",
        "script": sleep_script,
        "timeout_ms": 150,
        "verify_jwt": false
    });

    let (code, _) = send_http_request(
        bound_addr,
        "POST",
        "/v1/functions/v1/deploy",
        Some(&admin_token),
        Some(&timeout_deploy.to_string()),
    )
    .await;
    assert_eq!(code, 201);

    // 4. Invoke sleepy function -> MUST time out and return error safely
    let (code, err_res) = send_http_request(
        bound_addr,
        "POST",
        "/v1/functions/v1/sleepy_function",
        None,
        None,
    )
    .await;
    assert_eq!(code, 400);
    assert!(err_res["error"].as_str().unwrap().contains("timed out"));
}
