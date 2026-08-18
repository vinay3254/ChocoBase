use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use dbengine::{HttpServer, SharedDatabase};

async fn send_raw_http(addr: SocketAddr, req: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(req.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();

    let resp_str = String::from_utf8_lossy(&buf).to_string();
    let status_code: u16 = resp_str
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();

    (status_code, resp_str)
}

#[tokio::test]
async fn test_realtime_sse_broadcast_and_change_stream() {
    let file = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(file.path()).unwrap();

    db.execute("CREATE TABLE live_notes (id INTEGER PRIMARY KEY, content TEXT NOT NULL)")
        .unwrap();

    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (_server, bound_addr) = HttpServer::bind(addr, db.clone()).await.unwrap();

    // 1. Connect to SSE stream on channel=roomA
    let stream = TcpStream::connect(bound_addr).await.unwrap();
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    let req = "GET /v1/realtime/v1/stream?channel=roomA HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\n\r\n";
    write_half.write_all(req.as_bytes()).await.unwrap();
    write_half.flush().await.unwrap();

    // Read HTTP response headers
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    assert!(line.contains("200 OK"), "Expected 200 OK, got: {line}");

    loop {
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        if line == "\r\n" || line.is_empty() {
            break;
        }
    }

    // Read initial event
    let mut event_line = String::new();
    reader.read_line(&mut event_line).await.unwrap();
    assert!(event_line.contains("event: connected"), "got: {event_line}");
    let mut data_line = String::new();
    reader.read_line(&mut data_line).await.unwrap();
    assert!(data_line.contains("roomA"), "got: {data_line}");

    // Consume trailing newline
    let mut blank = String::new();
    reader.read_line(&mut blank).await.unwrap();

    // 2. Publish a broadcast message via HTTP POST
    let bcast_body = serde_json::json!({
        "event": "greeting",
        "payload": { "msg": "Hello World from SSE!" }
    })
    .to_string();
    let pub_req = format!(
        "POST /v1/realtime/v1/broadcast/roomA HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        bcast_body.len(),
        bcast_body
    );
    let (pub_status, _) = send_raw_http(bound_addr, &pub_req).await;
    assert_eq!(pub_status, 200);

    // Verify receipt of broadcast event on SSE stream
    event_line.clear();
    reader.read_line(&mut event_line).await.unwrap();
    assert!(event_line.contains("event: broadcast"), "got: {event_line}");
    data_line.clear();
    reader.read_line(&mut data_line).await.unwrap();
    assert!(
        data_line.contains("Hello World from SSE!"),
        "got: {data_line}"
    );

    // Consume trailing newline
    blank.clear();
    reader.read_line(&mut blank).await.unwrap();

    // 3. Perform a database mutation and verify changefeed event over SSE stream
    db.execute("INSERT INTO live_notes VALUES (1, 'Realtime note')")
        .unwrap();

    event_line.clear();
    reader.read_line(&mut event_line).await.unwrap();
    assert!(event_line.contains("event: change"), "got: {event_line}");
    data_line.clear();
    reader.read_line(&mut data_line).await.unwrap();
    assert!(data_line.contains("live_notes"), "got: {data_line}");
    assert!(data_line.contains("Realtime note"), "got: {data_line}");
}

#[tokio::test]
async fn test_realtime_sse_private_channel_unauthorized() {
    let file = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(file.path()).unwrap();

    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (_server, bound_addr) = HttpServer::bind(addr, db).await.unwrap();

    let req = "GET /v1/realtime/v1/stream?channel=private:secret HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    let (status, _) = send_raw_http(bound_addr, req).await;
    assert_eq!(status, 401);
}
