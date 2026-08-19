use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use dbengine::migration::{Migration, MigrationRunner};
use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_schema_migrations_rollback_lifecycle_and_api() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    // 1. Apply two migrations
    let m1 = Migration {
        version: 1,
        name: "create_products".to_string(),
        sql: "CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT NOT NULL)".to_string(),
    };
    let m2 = Migration {
        version: 2,
        name: "create_discounts".to_string(),
        sql: "CREATE TABLE discounts (id INTEGER PRIMARY KEY, rate INTEGER NOT NULL)".to_string(),
    };

    db.with_db(|d| {
        let mut runner = MigrationRunner::new(d);
        runner.apply_all(&[m1, m2])
    })
    .unwrap();

    // Verify both exist
    assert!(db.table_schema("products").is_some());
    assert!(db.table_schema("discounts").is_some());

    // 2. Roll back migration 2
    let rolled = db
        .with_db(|d| {
            let mut runner = MigrationRunner::new(d);
            runner.rollback_last("DROP TABLE discounts")
        })
        .unwrap();

    assert!(rolled.is_some());
    assert_eq!(rolled.unwrap().version, 2);
    assert!(db.table_schema("discounts").is_none());
    assert!(db.table_schema("products").is_some());

    // 3. Roll back migration 1 via HTTP admin API
    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db.clone())
        .await
        .unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let claims = dbengine::auth::SessionClaims::new(1, "admin", "admin", now + 3600);
    let admin_token = dbengine::auth::sign_jwt(&claims, dbengine::auth::DEFAULT_DEV_JWT_SECRET);

    let http_payload = serde_json::json!({
        "down_sql": "DROP TABLE products"
    });

    let (status, _, resp_body) = send_admin_post(
        addr,
        "/v1/admin/migrations/rollback",
        &admin_token,
        &http_payload.to_string(),
    )
    .await;
    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_str(&resp_body).unwrap();
    assert_eq!(json["status"], "rolled_back");
    assert_eq!(json["migration"]["version"], 1);

    assert!(db.table_schema("products").is_none());

    server.shutdown();
}

async fn send_admin_post(
    addr: SocketAddr,
    path: &str,
    token: &str,
    body: &str,
) -> (u16, Vec<String>, String) {
    let mut socket = TcpStream::connect(addr).await.unwrap();

    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
