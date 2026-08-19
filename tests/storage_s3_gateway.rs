use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_s3_storage_gateway_lifecycle() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    // 1. S3 PutObject: PUT /v1/storage/s3/assets/app_logo.png
    let payload = "PNG_MAGIC_HEADER_BYTES_123456";
    let (put_status, _, put_body) = send_s3_req(
        addr,
        "PUT",
        "/v1/storage/s3/assets/app_logo.png",
        None,
        payload,
    )
    .await;
    assert_eq!(put_status, 200);
    let put_json: serde_json::Value = serde_json::from_str(&put_body).unwrap();
    assert!(put_json.get("ETag").is_some());
    assert_eq!(put_json["Key"], "app_logo.png");

    // 2. S3 ListObjects: GET /v1/storage/s3/assets
    let (list_status, _, list_body) =
        send_s3_req(addr, "GET", "/v1/storage/s3/assets", None, "").await;
    assert_eq!(list_status, 200);
    let list_json: serde_json::Value = serde_json::from_str(&list_body).unwrap();
    assert_eq!(list_json["Name"], "assets");
    assert_eq!(list_json["KeyCount"], 1);

    // 3. S3 GetObject with Range: GET /v1/storage/s3/assets/app_logo.png (bytes=0-8)
    let (get_status, get_headers, get_body) = send_s3_req(
        addr,
        "GET",
        "/v1/storage/s3/assets/app_logo.png",
        Some("bytes=0-8"),
        "",
    )
    .await;
    assert_eq!(get_status, 206);
    assert_eq!(get_body, "PNG_MAGIC");
    assert!(get_headers.iter().any(|h| h.contains("Content-Range:")));

    // 4. S3 DeleteObject: DELETE /v1/storage/s3/assets/app_logo.png
    let (del_status, _, _) = send_s3_req(
        addr,
        "DELETE",
        "/v1/storage/s3/assets/app_logo.png",
        None,
        "",
    )
    .await;
    assert_eq!(del_status, 204);

    // Verify gone
    let (after_status, _, _) =
        send_s3_req(addr, "GET", "/v1/storage/s3/assets/app_logo.png", None, "").await;
    assert_eq!(after_status, 404);

    server.shutdown();
}

async fn send_s3_req(
    addr: SocketAddr,
    method: &str,
    path: &str,
    range: Option<&str>,
    body: &str,
) -> (u16, Vec<String>, String) {
    let mut socket = TcpStream::connect(addr).await.unwrap();

    let range_hdr = if let Some(r) = range {
        format!("Range: {r}\r\n")
    } else {
        String::new()
    };

    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/octet-stream\r\n{range_hdr}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
