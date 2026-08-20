//! Integration test for PostgREST JSON arrow operators (-> and ->>).

use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_postgrest_json_arrow_filtering() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    // 1. Create table with JSON column
    db.execute("CREATE TABLE accounts (id INTEGER PRIMARY KEY, metadata JSON NOT NULL)").unwrap();
    db.execute("INSERT INTO accounts (id, metadata) VALUES (1, '{\"role\": \"admin\", \"plan\": \"enterprise\", \"tier\": 1}')").unwrap();
    db.execute("INSERT INTO accounts (id, metadata) VALUES (2, '{\"role\": \"user\", \"plan\": \"pro\", \"tier\": 2}')").unwrap();
    db.execute("INSERT INTO accounts (id, metadata) VALUES (3, '{\"role\": \"guest\", \"plan\": \"free\", \"tier\": 3}')").unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    // 2. Query with metadata->>role=eq.admin
    let res = get_json(addr, "/rest/v1/accounts?metadata->>role=eq.admin").await;
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], 1);

    // 3. Query with metadata->>plan=not.eq.free
    let res_not = get_json(addr, "/rest/v1/accounts?metadata->>plan=not.eq.free&order=id.asc").await;
    let arr_not = res_not.as_array().unwrap();
    assert_eq!(arr_not.len(), 2);
    assert_eq!(arr_not[0]["id"], 1);
    assert_eq!(arr_not[1]["id"], 2);

    server.shutdown();
}

async fn get_json(addr: std::net::SocketAddr, path: &str) -> serde_json::Value {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let req = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let resp = String::from_utf8_lossy(&buf);
    let body = resp.split("\r\n\r\n").nth(1).unwrap();
    serde_json::from_str(body).unwrap_or(serde_json::json!([]))
}
