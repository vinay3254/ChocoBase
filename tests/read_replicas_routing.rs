use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_read_replicas_provisioning_and_query_routing() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    // 1. Base table on Primary
    db.execute(
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, score INTEGER NOT NULL)",
    )
    .unwrap();
    db.execute("INSERT INTO users (id, name, score) VALUES (1, 'Alice', 100)")
        .unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db.clone())
        .await
        .unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let claims = dbengine::auth::SessionClaims::new(1, "admin", "admin", now + 3600);
    let admin_token = dbengine::auth::sign_jwt(&claims, dbengine::auth::DEFAULT_DEV_JWT_SECRET);

    // 2. Provision Read Replica 'replica_east'
    let rep_payload = serde_json::json!({
        "id": "replica_east"
    });
    let (c_status, _, c_body) = send_http_req(
        addr,
        "POST",
        "/v1/admin/replicas",
        Some(&admin_token),
        &rep_payload.to_string(),
    )
    .await;
    assert_eq!(c_status, 201);
    let c_json: serde_json::Value = serde_json::from_str(&c_body).unwrap();
    assert_eq!(c_json["replica"]["id"], "replica_east");

    // 3. List Replicas
    let (list_status, _, list_body) =
        send_http_req(addr, "GET", "/v1/admin/replicas", Some(&admin_token), "").await;
    assert_eq!(list_status, 200);
    let list_json: serde_json::Value = serde_json::from_str(&list_body).unwrap();
    let reps = list_json["replicas"].as_array().unwrap();
    assert!(reps.iter().any(|r| r["id"] == "replica_east"));

    // 4. Test Query Routing: SELECT query routes to replica
    let read_query = serde_json::json!({
        "sql": "SELECT id, name, score FROM users WHERE id = 1"
    });
    let (q_status, _, q_body) = send_http_req(
        addr,
        "POST",
        "/v1/sql",
        Some(&admin_token),
        &read_query.to_string(),
    )
    .await;
    assert_eq!(q_status, 200);
    let q_json: serde_json::Value = serde_json::from_str(&q_body).unwrap();
    assert_eq!(q_json["route"], "replica");

    // 5. Test Query Routing: INSERT query routes strictly to primary
    let write_query = serde_json::json!({
        "sql": "INSERT INTO users (id, name, score) VALUES (2, 'Bob', 200)"
    });
    let (w_status, _, w_body) = send_http_req(
        addr,
        "POST",
        "/v1/sql",
        Some(&admin_token),
        &write_query.to_string(),
    )
    .await;
    assert_eq!(w_status, 200);
    let w_json: serde_json::Value = serde_json::from_str(&w_body).unwrap();
    assert_eq!(w_json["route"], "primary");

    // 6. Decommission Replica
    let (d_status, _, _) = send_http_req(
        addr,
        "DELETE",
        "/v1/admin/replicas/replica_east",
        Some(&admin_token),
        "",
    )
    .await;
    assert_eq!(d_status, 200);

    server.shutdown();
}

async fn send_http_req(
    addr: SocketAddr,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: &str,
) -> (u16, Vec<String>, String) {
    let mut socket = TcpStream::connect(addr).await.unwrap();

    let auth_hdr = match token {
        Some(t) => format!("Authorization: Bearer {t}\r\n"),
        None => String::new(),
    };

    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\n{auth_hdr}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
