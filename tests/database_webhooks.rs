use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use dbengine::webhooks::{WebhookConfig, WebhookManager};
use serde_json::json;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

#[tokio::test]
async fn test_webhook_lifecycle_and_http_endpoints() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    // 1. Sign up user to obtain token
    let signup_body = json!({
        "username": "admin_wh",
        "password": "Password123!"
    })
    .to_string();

    let token = send_request(addr, "POST", "/v1/auth/signup", None, &signup_body).await;
    let token_jwt = token["access_token"].as_str().unwrap();

    // 2. Register webhook
    let mut headers = HashMap::new();
    headers.insert("X-Custom-Auth".to_string(), "secret-hook-token".to_string());

    let hook_cfg = WebhookConfig {
        id: "hook_orders".to_string(),
        table_name: "orders".to_string(),
        events: vec!["INSERT".to_string(), "UPDATE".to_string()],
        target_url: "http://127.0.0.1:9999/webhook".to_string(),
        headers,
        active: true,
        max_retries: 3,
    };

    let post_res = send_request(
        addr,
        "POST",
        "/v1/webhooks",
        Some(token_jwt),
        &serde_json::to_string(&hook_cfg).unwrap(),
    )
    .await;
    assert_eq!(post_res["status"], "created");

    // 3. List webhooks
    let list_res = send_request(addr, "GET", "/v1/webhooks", Some(token_jwt), "").await;
    let hooks = list_res["webhooks"].as_array().unwrap();
    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0]["id"], "hook_orders");

    // 4. Delete webhook
    let del_res = send_request(
        addr,
        "DELETE",
        "/v1/webhooks/hook_orders",
        Some(token_jwt),
        "",
    )
    .await;
    assert_eq!(del_res["status"], "deleted");

    // Verify empty list
    let list_res2 = send_request(addr, "GET", "/v1/webhooks", Some(token_jwt), "").await;
    assert_eq!(list_res2["webhooks"].as_array().unwrap().len(), 0);

    server.shutdown();
}

#[tokio::test]
async fn test_webhook_event_dispatcher_on_database_mutation() {
    // 1. Start a mock HTTP receiver server
    let mock_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mock_addr = mock_listener.local_addr().unwrap();
    let received_payloads: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));

    let recv_clone = Arc::clone(&received_payloads);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = mock_listener.accept().await {
            let mut buf = vec![0u8; 4096];
            let n = socket.read(&mut buf).await.unwrap_or(0);
            if n > 0 {
                let s = String::from_utf8_lossy(&buf[..n]);
                if let Some(pos) = s.find("\r\n\r\n") {
                    let body = &s[pos + 4..];
                    if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(body) {
                        recv_clone.lock().await.push(json_val);
                    }
                }
                let _ = socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await;
            }
        }
    });

    // 2. Start ChocoBase with Webhook Manager
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    let webhook_mgr = Arc::new(WebhookManager::new());
    webhook_mgr.clone().start_dispatcher(db.subscribe());

    // Register active webhook targeting our mock listener
    let hook_cfg = WebhookConfig {
        id: "hook_dispatch".to_string(),
        table_name: "customers".to_string(),
        events: vec!["INSERT".to_string()],
        target_url: format!("http://127.0.0.1:{}/dispatch", mock_addr.port()),
        headers: HashMap::new(),
        active: true,
        max_retries: 3,
    };
    webhook_mgr.add_webhook(hook_cfg).await;

    // 3. Perform mutation in database
    db.execute("CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT)")
        .unwrap();
    db.execute("INSERT INTO customers (id, name) VALUES (1, 'Acme Corp')")
        .unwrap();

    // 4. Wait for async webhook dispatch
    let start = std::time::Instant::now();
    let mut matched = false;
    while start.elapsed() < std::time::Duration::from_secs(3) {
        let items = received_payloads.lock().await;
        if !items.is_empty() {
            assert_eq!(items[0]["type"], "INSERT");
            assert_eq!(items[0]["table"], "customers");
            matched = true;
            break;
        }
        drop(items);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    assert!(matched, "webhook payload was not received by mock receiver");
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
