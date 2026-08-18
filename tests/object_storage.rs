use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use dbengine::{HttpServer, SharedDatabase};

async fn send_http_request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: Option<&str>,
    auth_header: Option<&str>,
) -> (u16, serde_json::Value, Vec<u8>) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let body_str = body.unwrap_or("");
    let auth_str = match auth_header {
        Some(token) => format!("Authorization: Bearer {token}\r\n"),
        None => String::new(),
    };
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\n{auth_str}Content-Length: {}\r\nConnection: close\r\n\r\n{body_str}",
        body_str.len()
    );

    stream.write_all(req.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();

    let resp_str = String::from_utf8_lossy(&buf);
    let mut lines = resp_str.lines();
    let status_line = lines.next().unwrap();
    let status_code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();

    let body_idx = resp_str.find("\r\n\r\n").unwrap() + 4;
    let raw_body = buf[body_idx..].to_vec();
    let json_body: serde_json::Value =
        serde_json::from_slice(&raw_body).unwrap_or(serde_json::Value::Null);

    (status_code, json_body, raw_body)
}

#[tokio::test]
async fn test_object_storage_buckets_and_objects_lifecycle() {
    let file = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(file.path()).unwrap();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (_server, bound_addr) = HttpServer::bind(addr, db).await.unwrap();

    // 1. Create a public bucket 'photos'
    let (code, res, _) = send_http_request(
        bound_addr,
        "POST",
        "/v1/storage/v1/bucket",
        Some(r#"{"id": "photos", "name": "photos", "public": true}"#),
        None,
    )
    .await;
    assert_eq!(code, 201);
    assert_eq!(res["name"], "photos");

    // 2. List buckets
    let (code, buckets, _) =
        send_http_request(bound_addr, "GET", "/v1/storage/v1/bucket", None, None).await;
    assert_eq!(code, 200);
    let arr = buckets.as_array().unwrap();
    assert!(arr.iter().any(|b| b["id"] == "photos"));

    // 3. Upload object 'avatar.png'
    let image_data = "PNG_FAKE_IMAGE_DATA_12345";
    let (code, res, _) = send_http_request(
        bound_addr,
        "POST",
        "/v1/storage/v1/object/photos/avatar.png",
        Some(image_data),
        None,
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(res["Key"], "photos/avatar.png");

    // 4. Download object 'avatar.png'
    let (code, _, raw_bytes) = send_http_request(
        bound_addr,
        "GET",
        "/v1/storage/v1/object/photos/avatar.png",
        None,
        None,
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(String::from_utf8_lossy(&raw_bytes), image_data);

    // 5. Delete object 'avatar.png'
    let (code, res, _) = send_http_request(
        bound_addr,
        "DELETE",
        "/v1/storage/v1/object/photos/avatar.png",
        None,
        None,
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(res["message"], "object deleted");

    // 6. Verify object is deleted
    let (code, _, _) = send_http_request(
        bound_addr,
        "GET",
        "/v1/storage/v1/object/photos/avatar.png",
        None,
        None,
    )
    .await;
    assert_eq!(code, 404);

    // 7. Delete bucket 'photos'
    let (code, res, _) = send_http_request(
        bound_addr,
        "DELETE",
        "/v1/storage/v1/bucket/photos",
        None,
        None,
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(res["message"], "bucket deleted");
}
