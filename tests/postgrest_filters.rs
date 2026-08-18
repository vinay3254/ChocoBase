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
) -> (u16, serde_json::Value) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let body_str = body.unwrap_or("");
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body_str}",
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
    let json_body: serde_json::Value = serde_json::from_str(&resp_str[body_idx..]).unwrap();

    (status_code, json_body)
}

#[tokio::test]
async fn test_postgrest_filters_and_pagination() {
    let file = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(file.path()).unwrap();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (_server, bound_addr) = HttpServer::bind(addr, db).await.unwrap();

    // 1. Setup table and seed test data
    let (code, _) = send_http_request(
        bound_addr,
        "POST",
        "/v1/sql",
        Some(r#"{"sql": "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT, price INTEGER, category TEXT)"}"#),
    )
    .await;
    assert_eq!(code, 200);

    let seed_sql = r#"{"sql": "INSERT INTO items (id, name, price, category) VALUES (1, 'Apple', 10, 'Fruit'), (2, 'Banana', 20, 'Fruit'), (3, 'Carrot', 15, 'Vegetable'), (4, 'Donut', 30, 'Bakery'), (5, 'Eggplant', 25, 'Vegetable')"}"#;
    let (code, _) = send_http_request(bound_addr, "POST", "/v1/sql", Some(seed_sql)).await;
    assert_eq!(code, 200);

    // 2. Filter: eq (Fruit)
    let (code, res) =
        send_http_request(bound_addr, "GET", "/v1/rest/items?category=eq.Fruit", None).await;
    assert_eq!(code, 200);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert!(arr.iter().all(|item| item["category"] == "Fruit"));

    // 3. Filter: gt (price > 15)
    let (code, res) =
        send_http_request(bound_addr, "GET", "/v1/rest/items?price=gt.15", None).await;
    assert_eq!(code, 200);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 3); // Banana(20), Donut(30), Eggplant(25)

    // 4. Filter: lte (price <= 15)
    let (code, res) =
        send_http_request(bound_addr, "GET", "/v1/rest/items?price=lte.15", None).await;
    assert_eq!(code, 200);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2); // Apple(10), Carrot(15)

    // 5. Filter: like (name LIKE 'B%')
    let (code, res) =
        send_http_request(bound_addr, "GET", "/v1/rest/items?name=like.B%", None).await;
    assert_eq!(code, 200);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["name"], "Banana");

    // 6. Filter: in (id IN (1, 3, 5))
    let (code, res) =
        send_http_request(bound_addr, "GET", "/v1/rest/items?id=in.(1,3,5)", None).await;
    assert_eq!(code, 200);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 3);

    // 7. Projection & Order: select=name,price & order=price.desc
    let (code, res) = send_http_request(
        bound_addr,
        "GET",
        "/v1/rest/items?select=name,price&order=price.desc",
        None,
    )
    .await;
    assert_eq!(code, 200);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 5);
    assert_eq!(arr[0]["name"], "Donut"); // highest price 30
    assert_eq!(arr[0]["price"], 30);
    assert_eq!(arr[4]["name"], "Apple"); // lowest price 10
    assert_eq!(arr[4]["price"], 10);

    // 8. Pagination: limit=2 & offset=1 with order=price.asc
    let (code, res) = send_http_request(
        bound_addr,
        "GET",
        "/v1/rest/items?order=price.asc&limit=2&offset=1",
        None,
    )
    .await;
    assert_eq!(code, 200);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["name"], "Carrot"); // price 15 (offset 1 skipped Apple at 10)
    assert_eq!(arr[1]["name"], "Banana"); // price 20
}
