use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_storage_resumable_chunked_upload_lifecycle() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db.clone())
        .await
        .unwrap();

    // 1. Create public bucket 'assets'
    let bucket_payload = serde_json::json!({
        "name": "assets",
        "public": true
    });
    let (b_status, _, _) = send_http_req(
        addr,
        "POST",
        "/v1/storage/v1/bucket",
        &bucket_payload.to_string(),
    )
    .await;
    assert_eq!(b_status, 201);

    // 2. Initialize Resumable Upload Session for 15 bytes total
    let init_payload = serde_json::json!({
        "bucket_id": "assets",
        "object_name": "clips/intro.mp4",
        "content_type": "video/mp4",
        "total_size": 15
    });
    let (init_status, _, init_body) = send_http_req(
        addr,
        "POST",
        "/v1/storage/v1/upload/resumable",
        &init_payload.to_string(),
    )
    .await;
    assert_eq!(init_status, 201);
    let init_json: serde_json::Value = serde_json::from_str(&init_body).unwrap();
    let session_id = init_json["session_id"].as_str().expect("session_id");

    // 3. Upload first chunk (7 bytes)
    let chunk1 = "CHUNK_1";
    let patch1_url = format!("/v1/storage/v1/upload/resumable/{session_id}");
    let (patch1_status, _, patch1_body) = send_http_req(addr, "PATCH", &patch1_url, chunk1).await;
    assert_eq!(patch1_status, 200);
    let patch1_json: serde_json::Value = serde_json::from_str(&patch1_body).unwrap();
    assert_eq!(patch1_json["status"], "in_progress");
    assert_eq!(patch1_json["uploaded_offset"], 7);

    // 4. Query session status via GET
    let (get_status, _, get_body) = send_http_req(addr, "GET", &patch1_url, "").await;
    assert_eq!(get_status, 200);
    let get_json: serde_json::Value = serde_json::from_str(&get_body).unwrap();
    assert_eq!(get_json["uploaded_offset"], 7);
    assert_eq!(get_json["total_size"], 15);

    // 5. Upload final chunk (8 bytes) -> completes upload
    let chunk2 = "CHUNK_2_";
    let (patch2_status, _, patch2_body) = send_http_req(addr, "PATCH", &patch1_url, chunk2).await;
    assert_eq!(patch2_status, 200);
    let patch2_json: serde_json::Value = serde_json::from_str(&patch2_body).unwrap();
    assert_eq!(patch2_json["status"], "completed");
    assert_eq!(patch2_json["size_bytes"], 15);
    assert!(patch2_json["etag"].as_str().is_some());

    // 6. Download completed object and verify full content
    let (dl_status, _, dl_body) = send_http_req(
        addr,
        "GET",
        "/v1/storage/v1/object/assets/clips/intro.mp4",
        "",
    )
    .await;
    assert_eq!(dl_status, 200);
    assert_eq!(dl_body, "CHUNK_1CHUNK_2_");

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
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
