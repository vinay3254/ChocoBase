use dbengine::auth::ExecutionContext;
use dbengine::engine::SharedDatabase;
use dbengine::http::storage::cleanup_expired_objects;
use dbengine::http::HttpServer;
use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_storage_lifecycle_rules_and_background_cleanup() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db.clone())
        .await
        .unwrap();

    // 1. Create bucket: temp_cache
    let (b_status, _, _) = send_json_req(
        addr,
        "POST",
        "/v1/storage/v1/bucket",
        r#"{"id":"temp_cache","name":"temp_cache","public":true}"#,
        true,
    )
    .await;
    assert_eq!(b_status, 201);

    // 2. Set lifecycle rule: prefix "logs/", expiry_days = 2
    let (l_status, _, l_body) = send_json_req(
        addr,
        "POST",
        "/v1/storage/v1/bucket/temp_cache/lifecycle",
        r#"{"prefix":"logs/","expiry_days":2}"#,
        true,
    )
    .await;
    assert_eq!(l_status, 201);
    let l_json: serde_json::Value = serde_json::from_str(&l_body).unwrap();
    assert_eq!(l_json["prefix"], "logs/");
    assert_eq!(l_json["expiry_days"], 2);

    // 3. GET lifecycle rules
    let (get_status, _, get_body) = send_json_req(
        addr,
        "GET",
        "/v1/storage/v1/bucket/temp_cache/lifecycle",
        "",
        true,
    )
    .await;
    assert_eq!(get_status, 200);
    let rules_json: serde_json::Value = serde_json::from_str(&get_body).unwrap();
    assert_eq!(rules_json.as_array().unwrap().len(), 1);

    // 4. Upload fresh object: logs/today.log
    let (up_status, _, _) = send_json_req(
        addr,
        "POST",
        "/v1/storage/v1/object/temp_cache/logs/today.log",
        "current logs content",
        true,
    )
    .await;
    assert_eq!(up_status, 200);

    // 5. Seed an expired object: logs/ancient.log (created 5 days ago)
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let ancient_time = now - (5 * 86400);
    let seed_sql = format!(
        "INSERT INTO _storage_objects (id, bucket_id, name, owner_id, content_type, size_bytes, metadata, created_at, updated_at) VALUES ('temp_cache/logs/ancient.log', 'temp_cache', 'logs/ancient.log', NULL, 'text/plain', 12, '{{}}', {ancient_time}, {ancient_time})"
    );
    db.execute_with_context(&seed_sql, &ExecutionContext::admin())
        .unwrap();

    // 6. Run background cleanup
    let purged = cleanup_expired_objects(&db);
    assert_eq!(purged, 1, "ancient log must be purged");

    // 7. Verify fresh object still exists
    let (fresh_status, _, _) = send_json_req(
        addr,
        "GET",
        "/v1/storage/v1/object/public/temp_cache/logs/today.log",
        "",
        false,
    )
    .await;
    assert_eq!(fresh_status, 200);

    server.shutdown();
}

async fn send_json_req(
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: &str,
    as_admin: bool,
) -> (u16, Vec<String>, String) {
    let mut socket = TcpStream::connect(addr).await.unwrap();

    let admin_hdr = if as_admin {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let claims = dbengine::auth::SessionClaims::new(1, "admin", "admin", now + 3600);
        let admin_token = dbengine::auth::sign_jwt(&claims, dbengine::auth::DEFAULT_DEV_JWT_SECRET);
        format!("Authorization: Bearer {admin_token}\r\n")
    } else {
        String::new()
    };

    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\n{admin_hdr}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
