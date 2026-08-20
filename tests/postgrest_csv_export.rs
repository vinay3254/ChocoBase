//! Integration test for PostgREST CSV export (Accept: text/csv).

use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_csv_export() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    // Create table
    db.execute("CREATE TABLE inventory (id INTEGER PRIMARY KEY, sku TEXT, quantity INTEGER)").unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    // 1. Insert 2 rows
    let insert_body = r#"[{"id": 1, "sku": "ITEM,WITH,COMMAS", "quantity": 50}, {"id": 2, "sku": "NORMAL_SKU", "quantity": 100}]"#;
    let _ = send_req(addr, "POST", "/rest/v1/inventory", insert_body, None).await;

    // 2. Query with Accept: text/csv
    let (resp_hdrs, resp_body) = send_req(addr, "GET", "/rest/v1/inventory", "", Some("text/csv")).await;
    assert!(resp_hdrs.contains("200 OK"));
    assert!(resp_hdrs.contains("text/csv"));
    assert!(resp_body.contains("id,sku,quantity") || resp_body.contains("id") && resp_body.contains("sku"));
    assert!(resp_body.contains("\"ITEM,WITH,COMMAS\""));
    assert!(resp_body.contains("NORMAL_SKU"));

    server.shutdown();
}

async fn send_req(addr: std::net::SocketAddr, method: &str, path: &str, body: &str, accept: Option<&str>) -> (String, String) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let accept_hdr = if let Some(a) = accept {
        format!("Accept: {a}\r\n")
    } else {
        String::new()
    };
    let cl_hdr = if method == "POST" || method == "PATCH" || method == "PUT" {
        format!("Content-Length: {}\r\nContent-Type: application/json\r\n", body.len())
    } else {
        String::new()
    };
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\n{accept_hdr}{cl_hdr}Connection: close\r\n\r\n{body}"
    );
    stream.write_all(req.as_bytes()).await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let resp = String::from_utf8_lossy(&buf).to_string();
    let (headers, body_str) = resp.split_once("\r\n\r\n").unwrap_or((&resp, ""));
    (headers.to_string(), body_str.to_string())
}
