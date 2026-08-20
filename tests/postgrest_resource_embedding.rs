//! Integration test for PostgREST nested resource embedding (select=*,users(...)).

use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_postgrest_nested_resource_embedding() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    // 1. Create tables: users and posts
    db.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, email TEXT NOT NULL)").unwrap();
    db.execute("CREATE TABLE posts (id INTEGER PRIMARY KEY, title TEXT NOT NULL, user_id INTEGER NOT NULL)").unwrap();

    // 2. Insert records
    db.execute("INSERT INTO users (id, name, email) VALUES (1, 'Alice', 'alice@example.com'), (2, 'Bob', 'bob@example.com')").unwrap();
    db.execute("INSERT INTO posts (id, title, user_id) VALUES (10, 'First Post', 1), (20, 'Second Post', 2)").unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    // 3. Query with resource embedding: select=id,title,user_id,author:users(id,name,email)
    let res = get_json(addr, "/rest/v1/posts?select=id,title,user_id,author:users(id,name,email)&order=id.asc").await;
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2);

    assert_eq!(arr[0]["id"], 10);
    assert_eq!(arr[0]["title"], "First Post");
    assert_eq!(arr[0]["author"]["name"], "Alice");
    assert_eq!(arr[0]["author"]["email"], "alice@example.com");

    assert_eq!(arr[1]["id"], 20);
    assert_eq!(arr[1]["title"], "Second Post");
    assert_eq!(arr[1]["author"]["name"], "Bob");
    assert_eq!(arr[1]["author"]["email"], "bob@example.com");

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
