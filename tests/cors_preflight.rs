//! Integration test for CORS OPTIONS preflight and exposed headers.

use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_cors_options_and_headers() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    // 1. Send OPTIONS request with Origin and Access-Control-Request-Headers
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let req = "OPTIONS /rest/v1/notes HTTP/1.1\r\nHost: localhost\r\nOrigin: http://localhost:3000\r\nAccess-Control-Request-Method: GET\r\nAccess-Control-Request-Headers: apikey, authorization, prefer\r\nConnection: close\r\n\r\n";
    stream.write_all(req.as_bytes()).await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let resp = String::from_utf8_lossy(&buf);

    assert!(resp.starts_with("HTTP/1.1 204 No Content"));
    assert!(resp.contains("Access-Control-Allow-Origin: http://localhost:3000"));
    assert!(resp.contains("Access-Control-Allow-Headers:"));
    assert!(resp.contains("Prefer") || resp.contains("prefer"));
    assert!(resp.contains("Access-Control-Expose-Headers:"));

    server.shutdown();
}
