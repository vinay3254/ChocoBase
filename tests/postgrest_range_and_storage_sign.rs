//! Integration test for PostgREST Range header pagination and Supabase Storage signed URLs.

use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_postgrest_range_header_and_storage_sign() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    // 1. PostgREST Range Header test
    db.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT)").unwrap();
    db.execute("INSERT INTO items VALUES (1, 'Item 1'), (2, 'Item 2'), (3, 'Item 3'), (4, 'Item 4'), (5, 'Item 5')").unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db.clone())
        .await
        .unwrap();

    // Request Range: 0-1 (should return 2 items, 206 Partial Content, Content-Range: 0-1/5)
    let (h1, b1) = send_req_range(addr, "GET", "/rest/v1/items", "", Some("0-1")).await;
    assert!(h1.contains("206 Partial Content"));
    assert!(h1.contains("Content-Range: 0-1/5"));
    let arr1: serde_json::Value = serde_json::from_str(&b1).unwrap();
    assert_eq!(arr1.as_array().unwrap().len(), 2);

    // Request Range: items=2-4 (should return 3 items, 206 Partial Content, Content-Range: 2-4/5)
    let (h2, b2) = send_req_range(addr, "GET", "/rest/v1/items", "", Some("items=2-4")).await;
    assert!(h2.contains("206 Partial Content"));
    assert!(h2.contains("Content-Range: 2-4/5"));
    let arr2: serde_json::Value = serde_json::from_str(&b2).unwrap();
    assert_eq!(arr2.as_array().unwrap().len(), 3);

    // 2. Storage Presigned URL test
    // Create public bucket
    let _ = send_req_range(addr, "POST", "/v1/storage/v1/bucket", r#"{"id": "docs", "name": "docs", "public": true}"#, None).await;
    // Upload object
    let _ = send_req_range(addr, "POST", "/v1/storage/v1/object/docs/readme.txt", "ChocoBase Storage Test", None).await;
    // Create signed URL for object
    let (h_sign, b_sign) = send_req_range(addr, "POST", "/v1/storage/v1/object/sign/docs/readme.txt", r#"{"expiresIn": 60}"#, None).await;
    assert!(h_sign.contains("200 OK"));
    let sign_obj: serde_json::Value = serde_json::from_str(&b_sign).unwrap();
    assert!(sign_obj.get("signedURL").is_some());
    let signed_url = sign_obj["signedURL"].as_str().unwrap();

    // Fetch via signed URL
    let (h_get, b_get) = send_req_range(addr, "GET", signed_url, "", None).await;
    assert!(h_get.contains("200 OK"));
    assert_eq!(b_get, "ChocoBase Storage Test");

    server.shutdown();
}

async fn send_req_range(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    body: &str,
    range: Option<&str>,
) -> (String, String) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let range_hdr = if let Some(r) = range {
        format!("Range: {r}\r\n")
    } else {
        String::new()
    };
    let cl_hdr = if method == "POST" || method == "PATCH" || method == "PUT" {
        format!("Content-Length: {}\r\nContent-Type: application/json\r\n", body.len())
    } else {
        String::new()
    };
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\n{range_hdr}{cl_hdr}Connection: close\r\n\r\n{body}"
    );
    stream.write_all(req.as_bytes()).await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let resp = String::from_utf8_lossy(&buf).to_string();
    let (headers, body_str) = resp.split_once("\r\n\r\n").unwrap_or((&resp, ""));
    (headers.to_string(), body_str.to_string())
}
