use dbengine::engine::{Database, ExecResult, SharedDatabase};
use dbengine::http::HttpServer;
use serde_json::json;
use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_database_branching_and_isolation() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    // Seed main database
    db.execute("CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT, price INTEGER)")
        .unwrap();
    db.execute(
        "INSERT INTO products (id, name, price) VALUES (1, 'Laptop', 1000), (2, 'Phone', 500)",
    )
    .unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    // 1. Sign up user to obtain JWT
    let signup_body = json!({
        "username": "branch_admin",
        "password": "Password123!"
    })
    .to_string();

    let token_res = send_request(addr, "POST", "/v1/auth/signup", None, &signup_body).await;
    let jwt = token_res["access_token"].as_str().unwrap();

    // 2. Create staging branch
    let create_body = json!({ "name": "staging" }).to_string();
    let create_res = send_request(addr, "POST", "/v1/branches", Some(jwt), &create_body).await;
    assert_eq!(create_res["status"], "created");
    assert_eq!(create_res["branch"]["name"], "staging");

    // 3. List branches
    let list_res = send_request(addr, "GET", "/v1/branches", Some(jwt), "").await;
    let branches = list_res["branches"].as_array().unwrap();
    assert_eq!(branches.len(), 1);
    assert_eq!(branches[0]["name"], "staging");

    // 4. Verify branch file exists and operates independently
    let branch_path = branches[0]["path"].as_str().unwrap();
    let mut branch_db = Database::open(std::path::Path::new(branch_path)).unwrap();

    let res = branch_db
        .execute("SELECT id, name FROM products ORDER BY id ASC")
        .unwrap();
    match res {
        ExecResult::Rows { rows, .. } => {
            assert_eq!(rows.len(), 2);
        }
        other => panic!("unexpected result: {other:?}"),
    }

    // Mutate branch database
    branch_db
        .execute("INSERT INTO products (id, name, price) VALUES (3, 'Staging Only Monitor', 300)")
        .unwrap();

    // Verify main DB is untouched (still 2 rows)
    let main_res = send_request(
        addr,
        "POST",
        "/v1/sql",
        Some(jwt),
        &json!({ "sql": "SELECT id FROM products" }).to_string(),
    )
    .await;
    let main_rows = main_res["result"]["Rows"]["rows"].as_array().unwrap();
    assert_eq!(main_rows.len(), 2);

    // 5. Delete branch
    let del_res = send_request(addr, "DELETE", "/v1/branches/staging", Some(jwt), "").await;
    assert_eq!(del_res["status"], "deleted");

    let list_res2 = send_request(addr, "GET", "/v1/branches", Some(jwt), "").await;
    assert_eq!(list_res2["branches"].as_array().unwrap().len(), 0);

    server.shutdown();
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
