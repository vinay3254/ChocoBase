use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_postgrest_upsert_and_merge_duplicates() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    db.execute("CREATE TABLE profiles (id INTEGER PRIMARY KEY, username TEXT NOT NULL, rating INTEGER NOT NULL)").unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db.clone())
        .await
        .unwrap();

    // 1. Initial Insert via POST
    let initial_row = serde_json::json!({
        "id": 1,
        "username": "alice",
        "rating": 100
    });
    let (status1, _, _) =
        send_http_req(addr, "POST", "/v1/rest/profiles", &initial_row.to_string()).await;
    assert_eq!(status1, 201);

    // 2. Upsert via POST with resolution=merge-duplicates query param
    let update_row = serde_json::json!({
        "id": 1,
        "username": "alice_prime",
        "rating": 150
    });
    let (status2, _, _) = send_http_req(
        addr,
        "POST",
        "/v1/rest/profiles?resolution=merge-duplicates",
        &update_row.to_string(),
    )
    .await;
    assert_eq!(status2, 201);

    // 3. Upsert via PUT method
    let put_row = serde_json::json!({
        "id": 1,
        "username": "alice_final",
        "rating": 200
    });
    let (status3, _, _) =
        send_http_req(addr, "PUT", "/v1/rest/profiles", &put_row.to_string()).await;
    assert_eq!(status3, 201);

    // 4. Verify Single Updated Record
    let (status_get, _, get_body) = send_http_req(addr, "GET", "/v1/rest/profiles", "").await;
    assert_eq!(status_get, 200);
    let arr: Vec<serde_json::Value> = serde_json::from_str(&get_body).unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], 1);
    assert_eq!(arr[0]["username"], "alice_final");
    assert_eq!(arr[0]["rating"], 200);

    // 5. Bulk Upsert / Insert
    let bulk = serde_json::json!([
        { "id": 1, "username": "alice_v2", "rating": 300 },
        { "id": 2, "username": "bob", "rating": 400 }
    ]);
    let (status_bulk, _, _) = send_http_req(
        addr,
        "POST",
        "/v1/rest/profiles?resolution=merge-duplicates",
        &bulk.to_string(),
    )
    .await;
    assert_eq!(status_bulk, 201);

    let (_, _, final_body) = send_http_req(addr, "GET", "/v1/rest/profiles?order=id.asc", "").await;
    let final_arr: Vec<serde_json::Value> = serde_json::from_str(&final_body).unwrap();
    assert_eq!(final_arr.len(), 2);
    assert_eq!(final_arr[0]["id"], 1);
    assert_eq!(final_arr[0]["username"], "alice_v2");
    assert_eq!(final_arr[0]["rating"], 300);
    assert_eq!(final_arr[1]["id"], 2);
    assert_eq!(final_arr[1]["username"], "bob");
    assert_eq!(final_arr[1]["rating"], 400);

    server.shutdown();
}

async fn send_http_req(
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: &str,
) -> (u16, Vec<String>, String) {
    let mut socket = TcpStream::connect(addr).await.unwrap();

    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );

    socket.write_all(req.as_bytes()).await.unwrap();
    socket.flush().await.unwrap();

    let mut response_buf = Vec::new();
    socket.read_to_end(&mut response_buf).await.unwrap();

    let s = String::from_utf8_lossy(&response_buf);
    let mut header_lines = Vec::new();
    let mut lines = s.lines();

    let status_line = lines.next().unwrap_or("");
    let status_code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(500);

    for line in lines.by_ref() {
        if line.is_empty() {
            break;
        }
        header_lines.push(line.to_string());
    }

    let body_start = s.find("\r\n\r\n").map(|i| i + 4).unwrap_or(s.len());
    let resp_body = s[body_start..].to_string();

    (status_code, header_lines, resp_body)
}
