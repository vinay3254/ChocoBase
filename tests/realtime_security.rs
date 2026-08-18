use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use dbengine::auth::{sign_jwt, SessionClaims, DEFAULT_DEV_JWT_SECRET};
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
    let status_code: u16 = resp_str
        .lines()
        .next()
        .unwrap()
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
async fn test_realtime_presence_ownership_and_channel_security() {
    let file = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(file.path()).unwrap();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (_server, bound_addr) = HttpServer::bind(addr, db).await.unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // User A (Alice, ID 100)
    let claims_alice = SessionClaims::new(100, "alice", "user", now + 3600);
    let token_alice = format!("Bearer {}", sign_jwt(&claims_alice, DEFAULT_DEV_JWT_SECRET));

    // User B (Bob, ID 200)
    let claims_bob = SessionClaims::new(200, "bob", "user", now + 3600);
    let token_bob = format!("Bearer {}", sign_jwt(&claims_bob, DEFAULT_DEV_JWT_SECRET));

    // 1. Private broadcast channel requires authentication
    let (code, _) = send_http_request(
        bound_addr,
        "POST",
        "/v1/realtime/v1/broadcast/private:executive-room",
        None,
        Some(r#"{"event":"leak","payload":"secret"}"#),
    )
    .await;
    assert_eq!(code, 401);

    // With Alice's token, private broadcast succeeds
    let (code, bcast_res) = send_http_request(
        bound_addr,
        "POST",
        "/v1/realtime/v1/broadcast/private:executive-room",
        Some(&token_alice),
        Some(r#"{"event":"announcement","payload":{"title":"Q3 Goals"}}"#),
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(bcast_res["status"], "published");

    // 2. Alice tracks presence in room "design-board"
    let alice_presence = r#"{"key": "alice_cursor", "state": {"x": 20, "y": 40}}"#;
    let (code, track_res) = send_http_request(
        bound_addr,
        "POST",
        "/v1/realtime/v1/presence/design-board",
        Some(&token_alice),
        Some(alice_presence),
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(track_res["status"], "tracked");

    // 3. Bob tracks his own presence in the same room
    let bob_presence = r#"{"key": "bob_cursor", "state": {"x": 300, "y": 450}}"#;
    let (code, _) = send_http_request(
        bound_addr,
        "POST",
        "/v1/realtime/v1/presence/design-board",
        Some(&token_bob),
        Some(bob_presence),
    )
    .await;
    assert_eq!(code, 200);

    // 4. CROSS-USER PRESENCE TAMPERING: Bob attempts to delete Alice's presence key -> 403 Forbidden!
    let (code, _) = send_http_request(
        bound_addr,
        "DELETE",
        "/v1/realtime/v1/presence/design-board",
        Some(&token_bob),
        Some(r#"{"key": "alice_cursor"}"#),
    )
    .await;
    assert_eq!(code, 403);

    // 5. Alice deletes her own presence key -> 200 OK
    let (code, untrack_res) = send_http_request(
        bound_addr,
        "DELETE",
        "/v1/realtime/v1/presence/design-board",
        Some(&token_alice),
        Some(r#"{"key": "alice_cursor"}"#),
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(untrack_res["status"], "untracked");

    // 6. Verify only Bob remains in presence list
    let (code, list_res) = send_http_request(
        bound_addr,
        "GET",
        "/v1/realtime/v1/presence/design-board",
        None,
        None,
    )
    .await;
    assert_eq!(code, 200);
    let presences = list_res.as_array().unwrap();
    assert_eq!(presences.len(), 1);
    assert_eq!(presences[0]["key"], "bob_cursor");
}
