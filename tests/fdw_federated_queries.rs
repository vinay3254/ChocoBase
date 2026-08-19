use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_fdw_federated_query_engine() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db.clone())
        .await
        .unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let claims = dbengine::auth::SessionClaims::new(1, "admin", "admin", now + 3600);
    let admin_token = dbengine::auth::sign_jwt(&claims, dbengine::auth::DEFAULT_DEV_JWT_SECRET);

    // 1. Register Foreign Server
    let srv_payload = serde_json::json!({
        "name": "stripe_gateway",
        "wrapper_type": "Mock",
        "options": {
            "url": "https://api.stripe.com"
        }
    });

    let (s_status, _, s_body) = send_http_req(
        addr,
        "POST",
        "/v1/admin/fdw/servers",
        Some(&admin_token),
        &srv_payload.to_string(),
    )
    .await;
    assert_eq!(s_status, 201);
    let s_json: serde_json::Value = serde_json::from_str(&s_body).unwrap();
    assert_eq!(s_json["status"], "created");

    // 2. Create Foreign Virtual Table referencing mock data
    let mock_records = serde_json::json!([
        { "id": 101, "name": "Acme Corp", "balance": 4500.50, "verified": true },
        { "id": 102, "name": "Globex Inc", "balance": 1200.00, "verified": false }
    ]);

    let tbl_payload = serde_json::json!({
        "name": "stripe_customers",
        "server_name": "stripe_gateway",
        "columns": [
            { "name": "id", "ty": "Integer", "not_null": true, "primary_key": true },
            { "name": "name", "ty": "Text", "not_null": true, "primary_key": false },
            { "name": "balance", "ty": "Float", "not_null": false, "primary_key": false },
            { "name": "verified", "ty": "Boolean", "not_null": false, "primary_key": false }
        ],
        "options": {
            "data": mock_records.to_string()
        }
    });

    let (t_status, _, t_body) = send_http_req(
        addr,
        "POST",
        "/v1/admin/fdw/tables",
        Some(&admin_token),
        &tbl_payload.to_string(),
    )
    .await;
    assert_eq!(t_status, 201);
    let t_json: serde_json::Value = serde_json::from_str(&t_body).unwrap();
    assert_eq!(t_json["status"], "created");

    // 3. Query Foreign Virtual Table
    let (q_status, _, q_body) = send_http_req(
        addr,
        "GET",
        "/v1/fdw/stripe_customers",
        Some(&admin_token),
        "",
    )
    .await;
    assert_eq!(q_status, 200);
    let q_json: serde_json::Value = serde_json::from_str(&q_body).unwrap();
    let rows = q_json["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0][0], 101);
    assert_eq!(rows[0][1], "Acme Corp");
    assert_eq!(rows[1][0], 102);
    assert_eq!(rows[1][1], "Globex Inc");

    // 4. Drop Foreign Table
    let (d_status, _, _) = send_http_req(
        addr,
        "DELETE",
        "/v1/admin/fdw/tables/stripe_customers",
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
