use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;
use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_graphql_query_execution_and_field_selection() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    // Setup initial data
    db.execute("CREATE TABLE posts (id INTEGER PRIMARY KEY, title TEXT, author TEXT)")
        .unwrap();
    db.execute("INSERT INTO posts (id, title, author) VALUES (1, 'Hello World', 'Alice')")
        .unwrap();
    db.execute("INSERT INTO posts (id, title, author) VALUES (2, 'Second Post', 'Bob')")
        .unwrap();

    let (server, addr) = HttpServer::bind("127.0.0.1:0".parse().unwrap(), db)
        .await
        .unwrap();

    // 1. Basic GraphQL Query: { posts { id, title, author } }
    let gql_query = serde_json::json!({
        "query": "{ posts { id, title, author } }"
    });
    let (status, body) = send_http_post(addr, "/v1/graphql", &gql_query.to_string()).await;
    assert_eq!(status, 200);

    let resp_json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(resp_json.get("errors").unwrap().is_null());
    let posts = resp_json["data"]["posts"].as_array().unwrap();
    assert_eq!(posts.len(), 2);
    assert_eq!(posts[0]["id"], 1);
    assert_eq!(posts[0]["title"], "Hello World");
    assert_eq!(posts[0]["author"], "Alice");
    assert_eq!(posts[1]["id"], 2);
    assert_eq!(posts[1]["title"], "Second Post");

    // 2. Query with Limit Argument: { posts(limit: 1) { id, title } }
    let gql_limit_query = serde_json::json!({
        "query": "query { posts(limit: 1) { id, title } }"
    });
    let (status_lim, body_lim) =
        send_http_post(addr, "/v1/graphql", &gql_limit_query.to_string()).await;
    assert_eq!(status_lim, 200);

    let resp_lim: serde_json::Value = serde_json::from_str(&body_lim).unwrap();
    let posts_lim = resp_lim["data"]["posts"].as_array().unwrap();
    assert_eq!(posts_lim.len(), 1);
    assert_eq!(posts_lim[0]["id"], 1);
    assert!(posts_lim[0].get("author").is_none());

    server.shutdown();
}

async fn send_http_post(addr: SocketAddr, path: &str, body: &str) -> (u16, String) {
    let mut socket = TcpStream::connect(addr).await.unwrap();

    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );

    socket.write_all(req.as_bytes()).await.unwrap();
    socket.flush().await.unwrap();

    let mut response_buf = Vec::new();
    socket.read_to_end(&mut response_buf).await.unwrap();

    let s = String::from_utf8_lossy(&response_buf);
    let status_line = s.lines().next().unwrap_or("");
    let status_code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(500);

    let body_start = s.find("\r\n\r\n").map(|i| i + 4).unwrap_or(s.len());
    let resp_body = s[body_start..].to_string();

    (status_code, resp_body)
}
