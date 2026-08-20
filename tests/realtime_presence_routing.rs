//! Integration test for Realtime Presence & Broadcast via /realtime/v1/ route prefix.

use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_realtime_presence_and_broadcast_routing() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    // 1. Post presence state to /realtime/v1/presence/lobby
    let presence_payload = r#"{"key": "user_42", "state": {"online": true, "cursor": {"x": 100, "y": 200}}}"#;
    let res = send_post(addr, "/realtime/v1/presence/lobby", presence_payload).await;
    assert_eq!(res["status"], "tracked");
    assert_eq!(res["presence"]["key"], "user_42");

    // 2. Get presence state from /realtime/v1/presence/lobby
    let res_get = send_get(addr, "/realtime/v1/presence/lobby").await;
    let arr = res_get.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["key"], "user_42");
    assert_eq!(arr[0]["state"]["online"], true);

    // 3. Publish broadcast event to /realtime/v1/broadcast/lobby
    let broadcast_payload = r#"{"event": "cursor_move", "payload": {"x": 150, "y": 250}}"#;
    let res_bcast = send_post(addr, "/realtime/v1/broadcast/lobby", broadcast_payload).await;
    assert_eq!(res_bcast["status"], "published");
    assert_eq!(res_bcast["event"], "cursor_move");

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
    let body_str = resp.split("\r\n\r\n").nth(1).unwrap();
    serde_json::from_str(body_str).unwrap_or(serde_json::json!({}))
}

async fn send_get(addr: std::net::SocketAddr, path: &str) -> serde_json::Value {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let req = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let resp = String::from_utf8_lossy(&buf);
    let body_str = resp.split("\r\n\r\n").nth(1).unwrap();
    serde_json::from_str(body_str).unwrap_or(serde_json::json!([]))
}
