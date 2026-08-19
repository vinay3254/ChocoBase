use dbengine::auth::ExecutionContext;
use dbengine::backup::record_pitr_entry;
use dbengine::engine::{ExecResult, SharedDatabase};
use dbengine::http::HttpServer;
use dbengine::types::value::Value;
use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_point_in_time_recovery_wal_replay_and_cutoff() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    // 1. Initial schema & data at t=100
    db.execute("CREATE TABLE orders (id INTEGER PRIMARY KEY, total INTEGER NOT NULL)")
        .unwrap();
    db.execute("INSERT INTO orders (id, total) VALUES (1, 100)")
        .unwrap();

    let base_dump = db.dump_sql().unwrap();

    // 2. Log WAL mutations across discrete timestamps
    db.with_db(|d| {
        record_pitr_entry(d, 200, "INSERT INTO orders (id, total) VALUES (2, 200)").unwrap();
        record_pitr_entry(d, 300, "INSERT INTO orders (id, total) VALUES (3, 300)").unwrap();
        record_pitr_entry(d, 400, "INSERT INTO orders (id, total) VALUES (4, 400)").unwrap();
        Ok(())
    })
    .unwrap();

    // 3. Restore to point-in-time t=350 (should replay mutations 200 and 300, excluding 400)
    let tmp_restore = NamedTempFile::new().unwrap();
    let restore_db = SharedDatabase::create(tmp_restore.path()).unwrap();

    // Seed the target DB with the WAL log
    restore_db
        .with_db(|d| {
            record_pitr_entry(d, 200, "INSERT INTO orders (id, total) VALUES (2, 200)").unwrap();
            record_pitr_entry(d, 300, "INSERT INTO orders (id, total) VALUES (3, 300)").unwrap();
            record_pitr_entry(d, 400, "INSERT INTO orders (id, total) VALUES (4, 400)").unwrap();
            Ok(())
        })
        .unwrap();

    let count = restore_db
        .restore_to_point_in_time(&base_dump, 350)
        .unwrap();
    assert!(count >= 3);

    // 4. Query target database to verify PITR cutoff
    let res = restore_db
        .execute("SELECT id FROM orders ORDER BY id ASC")
        .unwrap();
    if let ExecResult::Rows { rows, .. } = res {
        let ids: Vec<i64> = rows
            .into_iter()
            .map(|r| match r[0] {
                Value::Integer(i) => i,
                _ => 0,
            })
            .collect();
        assert_eq!(
            ids,
            vec![1, 2, 3],
            "PITR at t=350 must contain orders 1, 2, 3 only"
        );
    } else {
        panic!("expected rows");
    }

    // 5. Test HTTP POST /v1/admin/pitr/restore endpoint
    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), restore_db.clone())
        .await
        .unwrap();

    let http_payload = serde_json::json!({
        "base_dump": base_dump,
        "target_timestamp_ms": 250
    });

    let (status, _, resp_body) =
        send_admin_post(addr, "/v1/admin/pitr/restore", &http_payload.to_string()).await;
    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_str(&resp_body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["target_timestamp_ms"], 250);

    let res2 = restore_db
        .execute_with_context(
            "SELECT id FROM orders ORDER BY id ASC",
            &ExecutionContext::admin(),
        )
        .unwrap();
    if let ExecResult::Rows { rows, .. } = res2 {
        let ids: Vec<i64> = rows
            .into_iter()
            .map(|r| match r[0] {
                Value::Integer(i) => i,
                _ => 0,
            })
            .collect();
        assert_eq!(
            ids,
            vec![1, 2],
            "PITR at t=250 via HTTP must contain orders 1, 2 only"
        );
    }

    server.shutdown();
}

async fn send_admin_post(addr: SocketAddr, path: &str, body: &str) -> (u16, Vec<String>, String) {
    let mut socket = TcpStream::connect(addr).await.unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let claims = dbengine::auth::SessionClaims::new(1, "admin", "admin", now + 3600);
    let admin_token = dbengine::auth::sign_jwt(&claims, dbengine::auth::DEFAULT_DEV_JWT_SECRET);

    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {admin_token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
