use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use dbengine::{HttpServer, SharedDatabase};

async fn send_http_request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: Option<&str>,
    auth_header: Option<&str>,
) -> (u16, serde_json::Value) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let body_str = body.unwrap_or("");
    let auth_str = match auth_header {
        Some(token) => format!("Authorization: Bearer {token}\r\n"),
        None => String::new(),
    };
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\n{auth_str}Content-Length: {}\r\nConnection: close\r\n\r\n{body_str}",
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
    let json_body: serde_json::Value = serde_json::from_str(&resp_str[body_idx..]).unwrap();

    (status_code, json_body)
}

#[tokio::test]
async fn http_gateway_end_to_end_rest_api() {
    let file = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(file.path()).unwrap();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (_server, bound_addr) = HttpServer::bind(addr, db).await.unwrap();

    // 1. Health check
    let (code, health) = send_http_request(bound_addr, "GET", "/v1/health", None, None).await;
    assert_eq!(code, 200);
    assert_eq!(health["status"], "healthy");
    assert_eq!(health["engine"], "ChocoBase");

    // 2. Create table via POST /v1/sql
    let (code, res) = send_http_request(
        bound_addr,
        "POST",
        "/v1/sql",
        Some(r#"{"sql": "CREATE TABLE products (id INTEGER PRIMARY KEY, title TEXT, price INTEGER)"}"#),
        None,
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(res["status"], "ok");

    // 3. Insert row via POST /v1/sql
    let (code, res) = send_http_request(
        bound_addr,
        "POST",
        "/v1/sql",
        Some(r#"{"sql": "INSERT INTO products (id, title, price) VALUES (1, 'Keyboard', 99), (2, 'Mouse', 49)"}"#),
        None,
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(res["status"], "ok");

    // 4. Query rows via POST /v1/sql
    let (code, res) = send_http_request(
        bound_addr,
        "POST",
        "/v1/sql",
        Some(r#"{"sql": "SELECT title, price FROM products ORDER BY price DESC"}"#),
        None,
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(res["status"], "ok");
    assert_eq!(res["result"]["Rows"]["rows"].as_array().unwrap().len(), 2);

    // 5. List tables via GET /v1/tables
    let (code, res) = send_http_request(bound_addr, "GET", "/v1/tables", None, None).await;
    assert_eq!(code, 200);
    let tables = res["tables"].as_array().unwrap();
    assert!(tables.iter().any(|t| t == "products"));

    // 6. Inspect schema via GET /v1/tables/products
    let (code, res) = send_http_request(bound_addr, "GET", "/v1/tables/products", None, None).await;
    assert_eq!(code, 200);
    assert_eq!(res["table"], "products");

    // 7. Metrics via GET /v1/metrics
    let (code, metrics) = send_http_request(bound_addr, "GET", "/v1/metrics", None, None).await;
    assert_eq!(code, 200);
    assert!(metrics["page_count"].as_u64().unwrap() > 0);

    // 8. Auth Signup & Token endpoints
    // Public signup requesting admin role must be rejected with 400
    let (code_admin_attempt, _) = send_http_request(
        bound_addr,
        "POST",
        "/v1/auth/signup",
        Some(r#"{"username": "hacker", "password": "securepassword", "role": "admin"}"#),
        None,
    )
    .await;
    assert_eq!(code_admin_attempt, 400);

    // Standard public signup
    let (code, signup) = send_http_request(
        bound_addr,
        "POST",
        "/v1/auth/signup",
        Some(r#"{"username": "developer1", "password": "securepassword"}"#),
        None,
    )
    .await;
    assert_eq!(code, 201);
    let token = signup["access_token"].as_str().unwrap();
    assert!(!token.is_empty());

    let (code, auth_res) = send_http_request(
        bound_addr,
        "POST",
        "/v1/auth/token",
        Some(r#"{"username": "developer1", "password": "securepassword"}"#),
        None,
    )
    .await;
    assert_eq!(code, 200);
    let token2 = auth_res["access_token"].as_str().unwrap();
    assert!(!token2.is_empty());

    // 9. Auto-generated REST Table CRUD APIs (/v1/rest/products)
    // Insert via POST /v1/rest/products
    let (code, rest_insert) = send_http_request(
        bound_addr,
        "POST",
        "/v1/rest/products",
        Some(r#"{"id": 3, "title": "Monitor", "price": 299}"#),
        Some(token2),
    )
    .await;
    assert_eq!(code, 201);
    assert_eq!(rest_insert["inserted"], 1);

    // Query via GET /v1/rest/products with filter & order & limit
    let (code, rest_get) = send_http_request(
        bound_addr,
        "GET",
        "/v1/rest/products?order=price.desc&limit=1",
        None,
        Some(token2),
    )
    .await;
    assert_eq!(code, 200);
    let arr = rest_get.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["title"], "Monitor");
    assert_eq!(arr[0]["price"], 299);

    // Update via PATCH /v1/rest/products?id=eq.3
    let (code, rest_patch) = send_http_request(
        bound_addr,
        "PATCH",
        "/v1/rest/products?id=eq.3",
        Some(r#"{"price": 249}"#),
        Some(token2),
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(rest_patch["modified"], 1);

    // Delete via DELETE /v1/rest/products?id=eq.3
    let (code, rest_delete) = send_http_request(
        bound_addr,
        "DELETE",
        "/v1/rest/products?id=eq.3",
        None,
        Some(token2),
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(rest_delete["deleted"], 1);

    // Verify deletion
    let (code, rest_get_after) = send_http_request(
        bound_addr,
        "GET",
        "/v1/rest/products?id=eq.3",
        None,
        Some(token2),
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(rest_get_after.as_array().unwrap().len(), 0);

    // 10. Refresh token via POST /v1/auth/refresh
    let refresh_token = signup["refresh_token"].as_str().unwrap();
    let (code, refresh_res) = send_http_request(
        bound_addr,
        "POST",
        "/v1/auth/refresh",
        Some(&format!(r#"{{"refresh_token": "{refresh_token}"}}"#)),
        None,
    )
    .await;
    assert_eq!(code, 200);
    assert!(refresh_res["access_token"].is_string());

    // 11. RPC endpoint via POST /v1/rpc/version
    let (code, rpc_res) =
        send_http_request(bound_addr, "POST", "/v1/rpc/version", None, None).await;
    assert_eq!(code, 200);
    assert_eq!(rpc_res["version"], "0.1.0");

    // 12. EXPLAIN query execution via POST /v1/sql
    let (code, explain_res) = send_http_request(
        bound_addr,
        "POST",
        "/v1/sql",
        Some(r#"{"sql": "EXPLAIN SELECT * FROM products WHERE id = 1"}"#),
        None,
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(explain_res["status"], "ok");

    // 13. Admin Studio Dashboard via GET /dashboard
    let mut stream = TcpStream::connect(bound_addr).await.unwrap();
    stream
        .write_all(b"GET /dashboard HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let resp = String::from_utf8_lossy(&buf);
    assert!(resp.starts_with("HTTP/1.1 200 OK"));
    assert!(resp.contains("ChocoBase Studio"));
}
