//! Integration tests for dynamic OpenAPI 3.0 specification generation endpoint.

use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_openapi_spec_generation() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    // Create tables
    db.execute("CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT NOT NULL, price FLOAT NOT NULL, in_stock BOOLEAN NOT NULL)").unwrap();
    db.execute("CREATE TABLE categories (id INTEGER PRIMARY KEY, title TEXT NOT NULL)").unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    let mut stream = TcpStream::connect(addr).await.unwrap();
    let req = "GET /v1/openapi.json HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    stream.write_all(req.as_bytes()).await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let response = String::from_utf8_lossy(&buf);

    assert!(response.contains("HTTP/1.1 200 OK"));
    let body = response.split("\r\n\r\n").nth(1).unwrap();
    let json: serde_json::Value = serde_json::from_str(body).unwrap();

    assert_eq!(json["openapi"], "3.0.0");
    assert_eq!(json["info"]["title"], "ChocoBase Auto-Generated API");

    // Check paths
    assert!(json["paths"]["/rest/v1/products"]["get"].is_object());
    assert!(json["paths"]["/rest/v1/categories"]["post"].is_object());

    // Check schemas
    assert_eq!(json["components"]["schemas"]["products"]["properties"]["name"]["type"], "string");
    assert_eq!(json["components"]["schemas"]["products"]["properties"]["price"]["type"], "number");
    assert_eq!(json["components"]["schemas"]["products"]["properties"]["in_stock"]["type"], "boolean");

    server.shutdown();
}
