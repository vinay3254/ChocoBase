use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_storage_image_transformation_and_rendering_pipeline() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db.clone())
        .await
        .unwrap();

    // 1. Create public bucket 'media'
    let bucket_payload = serde_json::json!({
        "name": "media",
        "public": true
    });
    let (b_status, _, _) = send_http_req(
        addr,
        "POST",
        "/v1/storage/v1/bucket",
        None,
        &bucket_payload.to_string(),
    )
    .await;
    assert_eq!(b_status, 201);

    // 2. Upload image file to /v1/storage/v1/object/media/avatars/user1.jpg
    let image_data = "FAKE_JPEG_BINARY_DATA_IMAGE_CONTENT_SAMPLE";
    let (up_status, _, _) = send_http_req(
        addr,
        "POST",
        "/v1/storage/v1/object/media/avatars/user1.jpg",
        None,
        image_data,
    )
    .await;
    assert_eq!(up_status, 200);

    // 3. Query Image Transformation endpoint with format=webp
    let (render_status, headers_webp, body_webp) = send_http_req(
        addr,
        "GET",
        "/v1/storage/v1/render/image/public/media/avatars/user1.jpg?width=200&height=200&format=webp&quality=85",
        None,
        "",
    )
    .await;
    assert_eq!(render_status, 200);
    assert_eq!(body_webp, image_data);

    let has_webp_content_type = headers_webp
        .iter()
        .any(|h| h.to_lowercase().starts_with("content-type: image/webp"));
    assert!(
        has_webp_content_type,
        "expected image/webp content-type header"
    );

    let has_etag = headers_webp
        .iter()
        .any(|h| h.to_lowercase().starts_with("etag:"));
    assert!(has_etag, "expected etag header");

    // 4. Query Image Transformation endpoint with format=png
    let (render_png_status, headers_png, body_png) = send_http_req(
        addr,
        "GET",
        "/v1/storage/v1/render/image/public/media/avatars/user1.jpg?format=png",
        None,
        "",
    )
    .await;
    assert_eq!(render_png_status, 200);
    assert_eq!(body_png, image_data);

    let has_png_content_type = headers_png
        .iter()
        .any(|h| h.to_lowercase().starts_with("content-type: image/png"));
    assert!(
        has_png_content_type,
        "expected image/png content-type header"
    );

    server.shutdown();
}

async fn send_http_req(
    addr: SocketAddr,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: &str,
) -> (u16, Vec<String>, String) {
    let mut socket = TcpStream::connect(addr).await.unwrap();

    let auth_hdr = match token {
        Some(t) => format!("Authorization: Bearer {t}\r\n"),
        None => String::new(),
    };

    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/octet-stream\r\n{auth_hdr}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
