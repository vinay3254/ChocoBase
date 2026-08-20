//! Integration test for PostgREST HEAD requests with count headers.

use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_postgrest_head_requests() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    // Create table
    db.execute("CREATE TABLE posts (id INTEGER PRIMARY KEY, title TEXT)").unwrap();
    db.execute("INSERT INTO posts VALUES (1, 'Post 1'), (2, 'Post 2'), (3, 'Post 3')").unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    // HEAD /rest/v1/posts with Prefer: count=exact
    let (resp_hdrs, resp_body) = send_req(addr, "HEAD", "/rest/v1/posts", "", Some("count=exact")).await;
    assert!(resp_hdrs.contains("200 OK"));
    assert!(resp_hdrs.contains("Content-Range: 0-2/3"));
    assert!(resp_hdrs.contains("Range-Unit: items"));
    assert!(resp_hdrs.contains("Preference-Applied: count=exact"));
    assert!(resp_body.is_empty(), "HEAD request body must be empty");

    // HEAD /health
    let (h_hdrs, h_body) = send_req(addr, "HEAD", "/health", "", None).await;
    assert!(h_hdrs.contains("200 OK"));
    assert!(h_body.is_empty(), "HEAD /health body must be empty");

    server.shutdown();
}

async fn send_req(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    body: &str,
    prefer: Option<&str>,
) -> (String, String) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let pref_hdr = if let Some(p) = prefer {
        format!("Prefer: {p}\r\n")
    } else {
        String::new()
    };
    let cl_hdr = if method == "POST" || method == "PATCH" || method == "PUT" {
        format!("Content-Length: {}\r\nContent-Type: application/json\r\n", body.len())
    } else {
        String::new()
    };
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\n{pref_hdr}{cl_hdr}Connection: close\r\n\r\n{body}"
    );
    stream.write_all(req.as_bytes()).await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let resp = String::from_utf8_lossy(&buf).to_string();
    let (headers, body_str) = resp.split_once("\r\n\r\n").unwrap_or((&resp, ""));
    (headers.to_string(), body_str.to_string())
}
