use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use dbengine::{dump_database, restore_database, Database, HttpServer, SharedDatabase};

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

#[test]
fn test_database_dump_and_restore_cycle() {
    let file1 = NamedTempFile::new().unwrap();
    let mut db1 = Database::create(file1.path()).unwrap();

    // 1. Create schema and populate records
    db1.execute("CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT NOT NULL, price INTEGER NOT NULL, in_stock BOOLEAN)")
        .unwrap();
    db1.execute("CREATE INDEX idx_price ON products (price)")
        .unwrap();

    db1.execute("BEGIN TRANSACTION").unwrap();
    db1.execute(
        "INSERT INTO products (id, name, price, in_stock) VALUES (1, 'Laptop', 1200, TRUE)",
    )
    .unwrap();
    db1.execute("INSERT INTO products (id, name, price, in_stock) VALUES (2, 'Mouse', 25, TRUE)")
        .unwrap();
    db1.execute(
        "INSERT INTO products (id, name, price, in_stock) VALUES (3, 'Monitor', 300, FALSE)",
    )
    .unwrap();
    db1.execute("COMMIT").unwrap();

    // 2. Generate SQL Dump
    let dump_sql = dump_database(&mut db1).unwrap();
    assert!(dump_sql.contains("CREATE TABLE products"));
    assert!(dump_sql.contains("CREATE INDEX idx_price ON products (price)"));
    assert!(dump_sql.contains("INSERT INTO products"));

    // 3. Create fresh database and restore
    let file2 = NamedTempFile::new().unwrap();
    let mut db2 = Database::create(file2.path()).unwrap();

    let executed_count = restore_database(&mut db2, &dump_sql).unwrap();
    assert!(executed_count >= 5);

    // 4. Verify data matches exactly
    let res1 = db1
        .execute("SELECT id, name, price, in_stock FROM products ORDER BY id")
        .unwrap();
    let res2 = db2
        .execute("SELECT id, name, price, in_stock FROM products ORDER BY id")
        .unwrap();
    assert_eq!(res1, res2);
}

#[tokio::test]
async fn test_http_admin_dump_and_restore_endpoints() {
    let file = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(file.path()).unwrap();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (_server, bound_addr) = HttpServer::bind(addr, db).await.unwrap();

    // 1. Setup table
    let (code, _) = send_http_request(
        bound_addr,
        "POST",
        "/v1/sql",
        None,
        Some(r#"{"sql": "CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT, balance INTEGER)"}"#),
    )
    .await;
    assert_eq!(code, 200);

    let (code, _) = send_http_request(
        bound_addr,
        "POST",
        "/v1/sql",
        None,
        Some(r#"{"sql": "INSERT INTO customers (id, name, balance) VALUES (1, 'Alice', 500), (2, 'Bob', 250)"}"#),
    )
    .await;
    assert_eq!(code, 200);

    // 2. Unauthenticated dump should be forbidden (403)
    let (code, _) = send_http_request(bound_addr, "GET", "/v1/admin/dump", None, None).await;
    assert_eq!(code, 403);

    // 3. Authenticated admin dump should succeed (200)
    // Create admin user
    let (code, signup_res) = send_http_request(
        bound_addr,
        "POST",
        "/v1/auth/signup",
        None,
        Some(r#"{"username": "sysadmin", "password": "rootpassword"}"#),
    )
    .await;
    assert_eq!(code, 201);
    let _token = signup_res["access_token"].as_str().unwrap();

    // Set sysadmin to admin role directly
    let (code, _) = send_http_request(
        bound_addr,
        "POST",
        "/v1/sql",
        None,
        Some(r#"{"sql": "UPDATE _users SET role = 'admin' WHERE username = 'sysadmin'"}"#),
    )
    .await;
    assert_eq!(code, 200);

    // Re-login to get admin claims
    let (code, login_res) = send_http_request(
        bound_addr,
        "POST",
        "/v1/auth/token",
        None,
        Some(r#"{"username": "sysadmin", "password": "rootpassword"}"#),
    )
    .await;
    assert_eq!(code, 200);
    let admin_token = login_res["access_token"].as_str().unwrap();

    let (code, dump_res) = send_http_request(
        bound_addr,
        "GET",
        "/v1/admin/dump",
        Some(&format!("Bearer {admin_token}")),
        None,
    )
    .await;
    assert_eq!(code, 200);
    let dump_content = dump_res["dump"].as_str().unwrap();
    assert!(dump_content.contains("CREATE TABLE customers"));
    assert!(dump_content.contains("INSERT INTO customers"));
}
