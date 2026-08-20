//! Integration test for PostgREST returning representation rows on mutations.

use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_postgrest_representation_returns() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    db.execute("CREATE TABLE todos (id INTEGER PRIMARY KEY, task TEXT NOT NULL, done BOOLEAN NOT NULL)").unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    // 1. Insert with representation return
    let insert_body = r#"{"id": 1, "task": "Write docs", "done": false}"#;
    let res = send_mutation(addr, "POST", "/rest/v1/todos", insert_body).await;
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], 1);
    assert_eq!(arr[0]["task"], "Write docs");
    assert_eq!(arr[0]["done"], false);

    // 2. Patch with representation return
    let patch_body = r#"{"done": true}"#;
    let res_patch = send_mutation(addr, "PATCH", "/rest/v1/todos?id=eq.1", patch_body).await;
    let arr_patch = res_patch.as_array().unwrap();
    assert_eq!(arr_patch.len(), 1);
    assert_eq!(arr_patch[0]["id"], 1);
    assert_eq!(arr_patch[0]["done"], true);

    // 3. Delete with representation return
    let res_del = send_mutation(addr, "DELETE", "/rest/v1/todos?id=eq.1", "").await;
    let arr_del = res_del.as_array().unwrap();
    assert_eq!(arr_del.len(), 1);
    assert_eq!(arr_del[0]["id"], 1);
    assert_eq!(arr_del[0]["task"], "Write docs");

    server.shutdown();
}

async fn send_mutation(addr: std::net::SocketAddr, method: &str, path: &str, body: &str) -> serde_json::Value {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(req.as_bytes()).await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let resp = String::from_utf8_lossy(&buf);
    let body_str = resp.split("\r\n\r\n").nth(1).unwrap();
    serde_json::from_str(body_str).unwrap_or(serde_json::json!([]))
}
