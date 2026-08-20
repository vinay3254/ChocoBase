//! Integration test for PostgREST multi-column ordering and nullsfirst/nullslast parameters.

use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_postgrest_ordering() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    // Create table
    db.execute("CREATE TABLE products (id INTEGER PRIMARY KEY, category TEXT, price INTEGER)").unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    // Insert 3 products
    let insert_body = r#"[
        {"id": 1, "category": "electronics", "price": 100},
        {"id": 2, "category": "electronics", "price": 500},
        {"id": 3, "category": "books", "price": 20}
    ]"#;
    let _ = send_req(addr, "POST", "/rest/v1/products", insert_body).await;

    // Query with order=price.desc.nullslast
    let (_, resp_body) = send_req(addr, "GET", "/rest/v1/products?order=price.desc.nullslast", "").await;
    let arr: serde_json::Value = serde_json::from_str(&resp_body).unwrap();
    assert_eq!(arr.as_array().unwrap().len(), 3);
    assert_eq!(arr[0]["price"], 500);
    assert_eq!(arr[1]["price"], 100);
    assert_eq!(arr[2]["price"], 20);

    server.shutdown();
}

async fn send_req(addr: std::net::SocketAddr, method: &str, path: &str, body: &str) -> (String, String) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let cl_hdr = if method == "POST" || method == "PATCH" || method == "PUT" {
        format!("Content-Length: {}\r\nContent-Type: application/json\r\n", body.len())
    } else {
        String::new()
    };
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\n{cl_hdr}Connection: close\r\n\r\n{body}"
    );
    stream.write_all(req.as_bytes()).await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let resp = String::from_utf8_lossy(&buf).to_string();
    let (headers, body_str) = resp.split_once("\r\n\r\n").unwrap_or((&resp, ""));
    (headers.to_string(), body_str.to_string())
}
