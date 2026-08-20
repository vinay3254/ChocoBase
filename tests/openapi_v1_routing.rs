//! Integration test for OpenAPI 3.0 specification generation via /rest/v1 and /v1/openapi.json.

use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_openapi_v1_specification() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    // Create table
    db.execute("CREATE TABLE products (id INTEGER PRIMARY KEY, title TEXT, price FLOAT)").unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    // 1. Query /rest/v1
    let spec1 = send_get(addr, "/rest/v1").await;
    assert_eq!(spec1["openapi"], "3.0.0");
    assert!(spec1["paths"]["/rest/v1/products"]["get"].is_object());
    assert!(spec1["paths"]["/rest/v1/products"]["post"].is_object());
    assert!(spec1["paths"]["/rest/v1/products"]["patch"].is_object());
    assert!(spec1["paths"]["/rest/v1/products"]["put"].is_object());
    assert!(spec1["paths"]["/rest/v1/products"]["delete"].is_object());

    // 2. Query /v1/openapi.json
    let spec2 = send_get(addr, "/v1/openapi.json").await;
    assert_eq!(spec2["openapi"], "3.0.0");
    assert_eq!(spec2["components"]["schemas"]["products"]["properties"]["title"]["type"], "string");
    assert_eq!(spec2["components"]["schemas"]["products"]["properties"]["price"]["type"], "number");

    server.shutdown();
}

async fn send_get(addr: std::net::SocketAddr, path: &str) -> serde_json::Value {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let req = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let resp = String::from_utf8_lossy(&buf);
    let body_str = resp.split("\r\n\r\n").nth(1).unwrap_or("{}");
    serde_json::from_str(body_str).unwrap_or(serde_json::json!({}))
}
