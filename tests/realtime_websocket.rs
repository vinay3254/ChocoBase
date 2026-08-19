use dbengine::engine::SharedDatabase;
use dbengine::http::websocket::{decode_frame, generate_websocket_accept};
use dbengine::http::HttpServer;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_realtime_websocket_handshake_and_channel_join() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    let mut socket = TcpStream::connect(addr).await.unwrap();

    // 1. Send RFC 6455 WebSocket Upgrade Handshake
    let ws_key = "dGhlIHNhbXBsZSBub25jZQ==";
    let expected_accept = generate_websocket_accept(ws_key);

    let handshake_req = format!(
        "GET /v1/realtime/v1/websocket HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {ws_key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    socket.write_all(handshake_req.as_bytes()).await.unwrap();
    socket.flush().await.unwrap();

    // 2. Read 101 Switching Protocols response
    let mut resp_buf = [0u8; 1024];
    let n = socket.read(&mut resp_buf).await.unwrap();
    let resp_str = String::from_utf8_lossy(&resp_buf[..n]);
    assert!(resp_str.contains("101 Switching Protocols"));
    assert!(resp_str.contains(&format!("Sec-WebSocket-Accept: {expected_accept}")));

    // 3. Send masked WebSocket Text Frame: phx_join
    let join_payload = serde_json::json!({
        "topic": "room:general",
        "event": "phx_join",
        "payload": {},
        "ref": "100"
    });
    let join_bytes = join_payload.to_string().into_bytes();

    // Encode masked client frame
    let mut client_frame = Vec::new();
    client_frame.push(0x81); // FIN + Text
    client_frame.push(0x80 | (join_bytes.len() as u8)); // Masked + Length
    let mask = [0x12, 0x34, 0x56, 0x78];
    client_frame.extend_from_slice(&mask);
    for (i, b) in join_bytes.iter().enumerate() {
        client_frame.push(b ^ mask[i % 4]);
    }

    socket.write_all(&client_frame).await.unwrap();
    socket.flush().await.unwrap();

    // 4. Read server frame reply
    let mut server_frame_buf = [0u8; 1024];
    let frame_n = socket.read(&mut server_frame_buf).await.unwrap();
    let (opcode, payload, _) = decode_frame(&server_frame_buf[..frame_n]).expect("valid frame");
    assert_eq!(opcode, 1); // Text frame

    let reply_json: serde_json::Value =
        serde_json::from_slice(&payload).expect("valid JSON payload");
    assert_eq!(reply_json["topic"], "room:general");
    assert_eq!(reply_json["event"], "phx_reply");
    assert_eq!(reply_json["ref"], "100");
    assert_eq!(reply_json["payload"]["status"], "ok");

    server.shutdown();
}
