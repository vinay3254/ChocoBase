use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_storage_range_requests_and_etags() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    // 1. Create public bucket 'media'
    let bucket_req = serde_json::json!({ "id": "media", "name": "media", "public": true });
    let (b_status, _, _) = send_http_req(
        addr,
        "POST",
        "/v1/storage/v1/bucket",
        None,
        &bucket_req.to_string(),
    )
    .await;
    assert_eq!(b_status, 201);

    // 2. Upload sample file (16 bytes: 0123456789ABCDEF)
    let file_content = "0123456789ABCDEF";
    let (up_status, _, up_body) = send_http_req(
        addr,
        "POST",
        "/v1/storage/v1/object/media/sample.txt",
        None,
        file_content,
    )
    .await;
    assert_eq!(up_status, 200);
    let up_json: serde_json::Value = serde_json::from_str(&up_body).unwrap();
    assert_eq!(up_json["size"], 16);
    assert!(up_json.get("etag").is_some());
    assert!(up_json.get("checksum_sha256").is_some());

    // 3. Full Download: 200 OK with ETag & Accept-Ranges
    let (full_status, full_headers, full_body) = send_http_req(
        addr,
        "GET",
        "/v1/storage/v1/object/public/media/sample.txt",
        None,
        "",
    )
    .await;
    assert_eq!(full_status, 200);
    assert_eq!(full_body, file_content);
    assert!(full_headers.iter().any(|h| h.contains("ETag:")));
    assert!(full_headers
        .iter()
        .any(|h| h.contains("Accept-Ranges: bytes")));

    // 4. Partial Range Download: bytes=0-4 (first 5 bytes)
    let (r1_status, r1_headers, r1_body) = send_http_req(
        addr,
        "GET",
        "/v1/storage/v1/object/public/media/sample.txt",
        Some("bytes=0-4"),
        "",
    )
    .await;
    assert_eq!(r1_status, 206);
    assert_eq!(r1_body, "01234");
    assert!(r1_headers
        .iter()
        .any(|h| h.contains("Content-Range: bytes 0-4/16")));

    // 5. Partial Range Download: bytes=10-15 (last 6 bytes)
    let (r2_status, r2_headers, r2_body) = send_http_req(
        addr,
        "GET",
        "/v1/storage/v1/object/public/media/sample.txt",
        Some("bytes=10-15"),
        "",
    )
    .await;
    assert_eq!(r2_status, 206);
    assert_eq!(r2_body, "ABCDEF");
    assert!(r2_headers
        .iter()
        .any(|h| h.contains("Content-Range: bytes 10-15/16")));

    server.shutdown();
}

async fn send_http_req(
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
