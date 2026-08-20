//! Extended integration tests for advanced PostgREST filter operators (not, fts, or).

use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_advanced_postgrest_filters() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    // 1. Create articles table and seed data
    db.execute("CREATE TABLE articles (id INTEGER PRIMARY KEY, title TEXT NOT NULL, status TEXT NOT NULL, rating INTEGER NOT NULL)").unwrap();
    db.execute("INSERT INTO articles (id, title, status, rating) VALUES (1, 'Rust Engine', 'published', 5)").unwrap();
    db.execute("INSERT INTO articles (id, title, status, rating) VALUES (2, 'Postgres Wire', 'draft', 3)").unwrap();
    db.execute("INSERT INTO articles (id, title, status, rating) VALUES (3, 'Old Archive', 'archived', 1)").unwrap();
    db.execute("INSERT INTO articles (id, title, status, rating) VALUES (4, 'Realtime Broadcast', 'published', 4)").unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    // Test A: Negation filter: status=not.eq.archived
    let res = get_json(addr, "/rest/v1/articles?status=not.eq.archived&order=id.asc").await;
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0]["id"], 1);
    assert_eq!(arr[1]["id"], 2);
    assert_eq!(arr[2]["id"], 4);

    // Test B: Logical OR filter: or=(status.eq.draft,rating.gte.5)
    let res_or = get_json(addr, "/rest/v1/articles?or=(status.eq.draft,rating.gte.5)&order=id.asc").await;
    let arr_or = res_or.as_array().unwrap();
    assert_eq!(arr_or.len(), 2);
    assert_eq!(arr_or[0]["id"], 1);
    assert_eq!(arr_or[1]["id"], 2);

    // Test C: Numeric negation: rating=not.lt.4
    let res_num = get_json(addr, "/rest/v1/articles?rating=not.lt.4&order=id.asc").await;
    let arr_num = res_num.as_array().unwrap();
    assert_eq!(arr_num.len(), 2);
    assert_eq!(arr_num[0]["id"], 1);
    assert_eq!(arr_num[1]["id"], 4);

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
