//! Integration test for PostgREST RPC endpoints via /rest/v1/rpc/ and /rpc/.

use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_rpc_v1_routing() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    // 1. Call built-in version rpc via /rest/v1/rpc/version
    let res_ver = send_post(addr, "/rest/v1/rpc/version", "{}").await;
    assert_eq!(res_ver["engine"], "ChocoBase");
    assert_eq!(res_ver["version"], "0.1.0");

    // 2. Call echo rpc via /rpc/echo with JSON payload
    let echo_payload = r#"{"hello": "world", "num": 42}"#;
    let res_echo = send_post(addr, "/rpc/echo", echo_payload).await;
    assert_eq!(res_echo["hello"], "world");
    assert_eq!(res_echo["num"], 42);

    // 3. Call current_user rpc via /rest/v1/rpc/current_user
    let res_curr = send_post(addr, "/rest/v1/rpc/current_user", "{}").await;
    assert_eq!(res_curr["role"], "anon");

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
