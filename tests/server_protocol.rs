use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::net::TcpStream;

use dbengine::server::protocol::{read_response, write_request, Request, Response};
use dbengine::types::value::Value;
use dbengine::{ExecResult, Server, ServerConfig};

#[tokio::test]
async fn server_accepts_tcp_connections_and_executes_queries() {
    let file = NamedTempFile::new().unwrap();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let config = ServerConfig::new(addr, file.path());
    let (_server, bound_addr) = Server::bind(config).await.unwrap();

    let mut stream = TcpStream::connect(bound_addr).await.unwrap();
    let mut buf = Vec::new();

    // 1. Ping
    write_request(&mut stream, &Request::Ping).await.unwrap();
    let resp = read_response(&mut stream, &mut buf).await.unwrap().unwrap();
    assert_eq!(resp, Response::Pong);

    // 2. Create Table
    write_request(
        &mut stream,
        &Request::Query {
            sql: "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT)".into(),
        },
    )
    .await
    .unwrap();
    let resp = read_response(&mut stream, &mut buf).await.unwrap().unwrap();
    assert_eq!(resp, Response::Result(ExecResult::Ok));

    // 3. Insert Row
    write_request(
        &mut stream,
        &Request::Query {
            sql: "INSERT INTO items (id, name) VALUES (1, 'hammer')".into(),
        },
    )
    .await
    .unwrap();
    let resp = read_response(&mut stream, &mut buf).await.unwrap().unwrap();
    assert_eq!(resp, Response::Result(ExecResult::Modified(1)));

    // 4. Select Row
    write_request(
        &mut stream,
        &Request::Query {
            sql: "SELECT name FROM items WHERE id = 1".into(),
        },
    )
    .await
    .unwrap();
    let resp = read_response(&mut stream, &mut buf).await.unwrap().unwrap();
    match resp {
        Response::Result(ExecResult::Rows { columns, rows }) => {
            assert_eq!(columns, vec!["name".to_string()]);
            assert_eq!(rows, vec![vec![Value::Text("hammer".into())]]);
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn server_handles_multiple_concurrent_tcp_clients() {
    let file = NamedTempFile::new().unwrap();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let config = ServerConfig::new(addr, file.path());
    let (_server, bound_addr) = Server::bind(config).await.unwrap();

    // Initial setup
    {
        let mut stream = TcpStream::connect(bound_addr).await.unwrap();
        let mut buf = Vec::new();
        write_request(
            &mut stream,
            &Request::Query {
                sql: "CREATE TABLE metrics (id INTEGER PRIMARY KEY, val INTEGER)".into(),
            },
        )
        .await
        .unwrap();
        let _ = read_response(&mut stream, &mut buf).await.unwrap();
    }

    let clients = 4;
    let ops_per_client = 25;
    let mut tasks = Vec::new();

    for client_id in 0..clients {
        let server_addr = bound_addr;
        tasks.push(tokio::spawn(async move {
            let mut stream = TcpStream::connect(server_addr).await.unwrap();
            let mut buf = Vec::new();
            for i in 0..ops_per_client {
                let id = client_id * ops_per_client + i;
                let sql = format!("INSERT INTO metrics (id, val) VALUES ({id}, {i})");
                write_request(&mut stream, &Request::Query { sql }).await.unwrap();
                let resp = read_response(&mut stream, &mut buf).await.unwrap().unwrap();
                assert_eq!(resp, Response::Result(ExecResult::Modified(1)));
            }
        }));
    }

    for task in tasks {
        task.await.unwrap();
    }

    // Verify total count
    let mut verify_stream = TcpStream::connect(bound_addr).await.unwrap();
    let mut buf = Vec::new();
    write_request(
        &mut verify_stream,
        &Request::Query {
            sql: "SELECT id FROM metrics".into(),
        },
    )
    .await
    .unwrap();
    let resp = read_response(&mut verify_stream, &mut buf).await.unwrap().unwrap();
    match resp {
        Response::Result(ExecResult::Rows { rows, .. }) => {
            assert_eq!(rows.len(), clients * ops_per_client);
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn server_cleans_up_locks_when_client_disconnects_abruptly() {
    let file = NamedTempFile::new().unwrap();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let config = ServerConfig::new(addr, file.path());
    let (_server, bound_addr) = Server::bind(config).await.unwrap();

    // Client 1: Start transaction, insert row, then disconnect abruptly
    {
        let mut stream = TcpStream::connect(bound_addr).await.unwrap();
        let mut buf = Vec::new();
        write_request(
            &mut stream,
            &Request::Query {
                sql: "CREATE TABLE t (id INTEGER PRIMARY KEY)".into(),
            },
        )
        .await
        .unwrap();
        let _ = read_response(&mut stream, &mut buf).await.unwrap();

        write_request(&mut stream, &Request::Query { sql: "BEGIN".into() }).await.unwrap();
        let _ = read_response(&mut stream, &mut buf).await.unwrap();

        write_request(
            &mut stream,
            &Request::Query {
                sql: "INSERT INTO t (id) VALUES (1)".into(),
            },
        )
        .await
        .unwrap();
        let _ = read_response(&mut stream, &mut buf).await.unwrap();

        // Drop socket abruptly
        drop(stream);
    }

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Client 2: Connect fresh and verify database is unlocked and can proceed
    let mut stream2 = TcpStream::connect(bound_addr).await.unwrap();
    let mut buf2 = Vec::new();
    write_request(
        &mut stream2,
        &Request::Query {
            sql: "INSERT INTO t (id) VALUES (2)".into(),
        },
    )
    .await
    .unwrap();
    let resp = read_response(&mut stream2, &mut buf2).await.unwrap().unwrap();
    assert_eq!(resp, Response::Result(ExecResult::Modified(1)));
}
