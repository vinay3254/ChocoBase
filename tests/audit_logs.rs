use dbengine::audit::{query_audit_logs, record_audit_log};
use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_audit_log_recording_filtering_and_http_access() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    // 1. Record discrete audit events
    record_audit_log(
        &db,
        Some(10),
        "AUTH_LOGIN",
        "user:10",
        Some("192.168.1.50"),
        serde_json::json!({ "method": "password", "user_agent": "Mozilla/5.0" }),
    )
    .unwrap();

    record_audit_log(
        &db,
        Some(10),
        "TABLE_MUTATION",
        "table:orders",
        Some("192.168.1.50"),
        serde_json::json!({ "mutation": "INSERT", "row_id": 101 }),
    )
    .unwrap();

    record_audit_log(
        &db,
        Some(20),
        "STORAGE_DELETE",
        "bucket:vault/secret.pdf",
        Some("10.0.0.1"),
        serde_json::json!({ "deleted_by": "bob" }),
    )
    .unwrap();

    // 2. Query with action filter
    let mutation_logs = query_audit_logs(&db, Some("TABLE_MUTATION"), None, 10).unwrap();
    assert_eq!(mutation_logs.len(), 1);
    assert_eq!(mutation_logs[0].target, "table:orders");
    assert_eq!(mutation_logs[0].metadata["row_id"], 101);

    // 3. Query with user_id filter
    let user20_logs = query_audit_logs(&db, None, Some(20), 10).unwrap();
    assert_eq!(user20_logs.len(), 1);
    assert_eq!(user20_logs[0].action, "STORAGE_DELETE");

    // 4. Test HTTP GET /v1/admin/audit-logs
    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db.clone())
        .await
        .unwrap();

    // Unauthenticated / non-admin access must be rejected (403)
    let (unauth_status, _, _) = send_audit_req(addr, "/v1/admin/audit-logs", false).await;
    assert_eq!(unauth_status, 403);

    // Admin access with query filter
    let (admin_status, _, admin_body) =
        send_audit_req(addr, "/v1/admin/audit-logs?action=AUTH_LOGIN", true).await;
    assert_eq!(admin_status, 200);
    let logs_json: serde_json::Value = serde_json::from_str(&admin_body).unwrap();
    let logs_arr = logs_json.as_array().expect("array of logs");
    assert_eq!(logs_arr.len(), 1);
    assert_eq!(logs_arr[0]["action"], "AUTH_LOGIN");
    assert_eq!(logs_arr[0]["ip_address"], "192.168.1.50");

    server.shutdown();
}

async fn send_audit_req(
    addr: SocketAddr,
    path: &str,
    as_admin: bool,
) -> (u16, Vec<String>, String) {
    let mut socket = TcpStream::connect(addr).await.unwrap();

    let auth_hdr = if as_admin {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let claims = dbengine::auth::SessionClaims::new(1, "admin", "admin", now + 3600);
        let admin_token = dbengine::auth::sign_jwt(&claims, dbengine::auth::DEFAULT_DEV_JWT_SECRET);
        format!("Authorization: Bearer {admin_token}\r\n")
    } else {
        String::new()
    };

    let req = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\n{auth_hdr}Connection: close\r\n\r\n");

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
