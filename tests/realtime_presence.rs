use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use dbengine::{HttpServer, SharedDatabase};

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

#[tokio::test]
async fn test_realtime_broadcast_and_presence_channels() {
    let file = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(file.path()).unwrap();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (_server, bound_addr) = HttpServer::bind(addr, db).await.unwrap();

    // 1. Broadcast an event to room channel
    let broadcast_body = r#"{
        "event": "user_typing",
        "payload": { "username": "alice", "active": true }
    }"#;
    let (code, broadcast_res) = send_http_request(
        bound_addr,
        "POST",
        "/v1/realtime/v1/broadcast/general-chat",
        None,
        Some(broadcast_body),
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(broadcast_res["status"], "published");
    assert_eq!(broadcast_res["channel"], "general-chat");
    assert_eq!(broadcast_res["event"], "user_typing");
    assert_eq!(broadcast_res["payload"]["username"], "alice");

    // 2. Track presence for User 1 (Alice)
    let presence_alice = r#"{
        "key": "alice_session_1",
        "state": { "status": "online", "cursor": { "x": 100, "y": 250 } }
    }"#;
    let (code, track_res) = send_http_request(
        bound_addr,
        "POST",
        "/v1/realtime/v1/presence/design-canvas",
        None,
        Some(presence_alice),
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(track_res["status"], "tracked");
    assert_eq!(track_res["presence"]["key"], "alice_session_1");

    // 3. Track presence for User 2 (Bob)
    let presence_bob = r#"{
        "key": "bob_session_2",
        "state": { "status": "idle", "cursor": { "x": 500, "y": 600 } }
    }"#;
    let (code, _) = send_http_request(
        bound_addr,
        "POST",
        "/v1/realtime/v1/presence/design-canvas",
        None,
        Some(presence_bob),
    )
    .await;
    assert_eq!(code, 200);

    // 4. Query active channel presence list
    let (code, presence_list) = send_http_request(
        bound_addr,
        "GET",
        "/v1/realtime/v1/presence/design-canvas",
        None,
        None,
    )
    .await;
    assert_eq!(code, 200);
    let arr = presence_list.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert!(arr.iter().any(|p| p["key"] == "alice_session_1"));
    assert!(arr.iter().any(|p| p["key"] == "bob_session_2"));

    // 5. Untrack / Leave presence (Alice leaves)
    let (code, untrack_res) = send_http_request(
        bound_addr,
        "DELETE",
        "/v1/realtime/v1/presence/design-canvas",
        None,
        Some(r#"{"key": "alice_session_1"}"#),
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(untrack_res["status"], "untracked");
    assert_eq!(untrack_res["removed"], true);

    // 6. Verify only Bob remains
    let (code, presence_list2) = send_http_request(
        bound_addr,
        "GET",
        "/v1/realtime/v1/presence/design-canvas",
        None,
        None,
    )
    .await;
    assert_eq!(code, 200);
    let arr2 = presence_list2.as_array().unwrap();
    assert_eq!(arr2.len(), 1);
    assert_eq!(arr2[0]["key"], "bob_session_2");
}
