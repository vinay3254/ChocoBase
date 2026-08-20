//! Integration test for PostgREST single row object representation (Accept: application/vnd.pgrst.object+json).

use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_single_object_representation() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    // Create table
    db.execute("CREATE TABLE profiles (id INTEGER PRIMARY KEY, username TEXT, score INTEGER)").unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    // 1. Insert 1 profile with Accept: application/vnd.pgrst.object+json
    let insert_body = r#"{"id": 1, "username": "satoshi", "score": 100}"#;
    let (resp1_hdrs, resp1_body) = send_req(addr, "POST", "/rest/v1/profiles", insert_body, Some("application/vnd.pgrst.object+json")).await;
    assert!(resp1_hdrs.contains("201 Created"));
    let inserted_obj: serde_json::Value = serde_json::from_str(&resp1_body).unwrap();
    assert!(inserted_obj.is_object());
    assert_eq!(inserted_obj["username"], "satoshi");

    // 2. Query 1 row with .single() (Accept: application/vnd.pgrst.object+json)
    let (resp2_hdrs, resp2_body) = send_req(addr, "GET", "/rest/v1/profiles?id=eq.1", "", Some("application/vnd.pgrst.object+json")).await;
    assert!(resp2_hdrs.contains("200 OK"));
    assert!(resp2_hdrs.contains("application/vnd.pgrst.object+json"));
    let profile_obj: serde_json::Value = serde_json::from_str(&resp2_body).unwrap();
    assert!(profile_obj.is_object());
    assert_eq!(profile_obj["score"], 100);

    // 3. Query 0 rows with single object -> 406 Not Acceptable (PGRST116)
    let (resp3_hdrs, resp3_body) = send_req(addr, "GET", "/rest/v1/profiles?id=eq.999", "", Some("application/vnd.pgrst.object+json")).await;
    assert!(resp3_hdrs.contains("406 Not Acceptable"));
    let err_obj: serde_json::Value = serde_json::from_str(&resp3_body).unwrap();
    assert_eq!(err_obj["code"], "PGRST116");

    server.shutdown();
}

async fn send_req(addr: std::net::SocketAddr, method: &str, path: &str, body: &str, accept: Option<&str>) -> (String, String) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let accept_hdr = if let Some(a) = accept {
        format!("Accept: {a}\r\n")
    } else {
        String::new()
    };
    let cl_hdr = if method == "POST" || method == "PATCH" || method == "PUT" {
        format!("Content-Length: {}\r\nContent-Type: application/json\r\n", body.len())
    } else {
        String::new()
    };
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\n{accept_hdr}{cl_hdr}Connection: close\r\n\r\n{body}"
    );
    stream.write_all(req.as_bytes()).await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let resp = String::from_utf8_lossy(&buf).to_string();
    let (headers, body_str) = resp.split_once("\r\n\r\n").unwrap_or((&resp, ""));
    (headers.to_string(), body_str.to_string())
}
