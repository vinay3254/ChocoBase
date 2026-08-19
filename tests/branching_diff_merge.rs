use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_branching_schema_diff_and_merge_lifecycle() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    // 1. Base table in main DB
    db.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
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

    // 2. Create branch 'staging' via POST /v1/branches
    let create_payload = serde_json::json!({
        "name": "staging"
    });
    let (c_status, _, _) = send_http_req(
        addr,
        "POST",
        "/v1/branches",
        Some(&admin_token),
        &create_payload.to_string(),
    )
    .await;
    assert_eq!(c_status, 201);

    // 3. Mutate staging branch via POST /v1/branches/staging/sql
    let ddl1 = serde_json::json!({
        "sql": "CREATE TABLE feature_flags (id INTEGER PRIMARY KEY, flag TEXT NOT NULL)"
    });
    let (d1_status, _, _) = send_http_req(
        addr,
        "POST",
        "/v1/branches/staging/sql",
        Some(&admin_token),
        &ddl1.to_string(),
    )
    .await;
    assert_eq!(d1_status, 200);

    let ddl2 = serde_json::json!({
        "sql": "ALTER TABLE users ADD COLUMN bio TEXT"
    });
    let (d2_status, _, _) = send_http_req(
        addr,
        "POST",
        "/v1/branches/staging/sql",
        Some(&admin_token),
        &ddl2.to_string(),
    )
    .await;
    assert_eq!(d2_status, 200);

    // 4. Test Schema Diff via GET /v1/branches/staging/diff
    let (diff_status, _, diff_body) = send_http_req(
        addr,
        "GET",
        "/v1/branches/staging/diff",
        Some(&admin_token),
        "",
    )
    .await;
    assert_eq!(diff_status, 200);
    let diff_json: serde_json::Value = serde_json::from_str(&diff_body).unwrap();
    assert_eq!(diff_json["branch_name"], "staging");

    let added_tables = diff_json["added_tables"].as_array().unwrap();
    assert!(added_tables.iter().any(|t| t == "feature_flags"));

    let modified_tables = diff_json["modified_tables"].as_array().unwrap();
    assert!(modified_tables.iter().any(|t| t["table_name"] == "users"));

    // 5. Test Merge via POST /v1/branches/staging/merge
    let (merge_status, _, merge_body) = send_http_req(
        addr,
        "POST",
        "/v1/branches/staging/merge",
        Some(&admin_token),
        "",
    )
    .await;
    assert_eq!(merge_status, 200);
    let merge_json: serde_json::Value = serde_json::from_str(&merge_body).unwrap();
    assert_eq!(merge_json["status"], "merged");

    // 6. Verify main DB now contains feature_flags and altered users schema
    assert!(db.table_schema("feature_flags").is_some());
    let users_schema = db.table_schema("users").expect("users table");
    assert!(users_schema.columns.iter().any(|c| c.name == "bio"));

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
