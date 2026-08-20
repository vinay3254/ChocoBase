//! Integration test for Storage API via /storage/v1/ route prefix.

use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_storage_v1_routing() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    // 1. Create a bucket at /storage/v1/bucket
    let create_payload = r#"{"id": "avatars", "name": "avatars", "public": true}"#;
    let res = send_post(addr, "/storage/v1/bucket", create_payload).await;
    assert_eq!(res["name"], "avatars");

    // 2. List buckets at /storage/v1/bucket
    let res_list = send_get(addr, "/storage/v1/bucket").await;
    let arr = res_list.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "avatars");

    // 3. Upload object at /storage/v1/object/avatars/user1.png
    let upload_bytes = b"fake-png-binary-data";
    let res_upload = send_binary_post(addr, "/storage/v1/object/avatars/user1.png", upload_bytes, "image/png").await;
    assert_eq!(res_upload["Key"], "avatars/user1.png");

    server.shutdown();
}

async fn send_post(addr: std::net::SocketAddr, path: &str, body: &str) -> serde_json::Value {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(req.as_bytes()).await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let resp = String::from_utf8_lossy(&buf);
    let body_str = resp.split("\r\n\r\n").nth(1).unwrap_or("{}");
    serde_json::from_str(body_str).unwrap_or(serde_json::json!({}))
}

async fn send_binary_post(addr: std::net::SocketAddr, path: &str, body: &[u8], content_type: &str) -> serde_json::Value {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(req.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let resp = String::from_utf8_lossy(&buf);
    let body_str = resp.split("\r\n\r\n").nth(1).unwrap_or("{}");
    serde_json::from_str(body_str).unwrap_or(serde_json::json!({}))
}

async fn send_get(addr: std::net::SocketAddr, path: &str) -> serde_json::Value {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let req = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let resp = String::from_utf8_lossy(&buf);
    let body_str = resp.split("\r\n\r\n").nth(1).unwrap_or("[]");
    serde_json::from_str(body_str).unwrap_or(serde_json::json!([]))
}
