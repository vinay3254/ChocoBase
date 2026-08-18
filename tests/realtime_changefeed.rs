use std::net::SocketAddr;
use std::time::Duration;
use tempfile::NamedTempFile;
use tokio::net::TcpStream;
use tokio::time::timeout;

use dbengine::server::protocol::{read_response, write_request, ChangeAction, Request, Response};
use dbengine::server::{Server, ServerConfig};
use dbengine::types::value::Value;

#[tokio::test]
async fn realtime_changefeed_streams_insert_update_delete_events() {
    let file = NamedTempFile::new().unwrap();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let config = ServerConfig::new(addr, file.path());
    let (_server, bound_addr) = Server::bind(config).await.unwrap();

    // Client 1: Subscriber
    let mut sub_socket = TcpStream::connect(bound_addr).await.unwrap();
    let (mut sub_reader, mut sub_writer) = sub_socket.split();
    let mut sub_buf = Vec::new();

    write_request(&mut sub_writer, &Request::Subscribe { table: Some("customers".into()) }).await.unwrap();
    let resp = read_response(&mut sub_reader, &mut sub_buf).await.unwrap().unwrap();
    assert_eq!(resp, Response::Subscribed);

    // Client 2: Mutator
    let mut mut_socket = TcpStream::connect(bound_addr).await.unwrap();
    let (mut mut_reader, mut mut_writer) = mut_socket.split();
    let mut mut_buf = Vec::new();

    // Create table
    write_request(&mut mut_writer, &Request::Query { sql: "CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT)".into() }).await.unwrap();
    let _ = read_response(&mut mut_reader, &mut mut_buf).await.unwrap();

    // 1. Insert
    write_request(&mut mut_writer, &Request::Query { sql: "INSERT INTO customers (id, name) VALUES (1, 'Alice')".into() }).await.unwrap();
    let _ = read_response(&mut mut_reader, &mut mut_buf).await.unwrap();

    let insert_event = timeout(Duration::from_secs(2), read_response(&mut sub_reader, &mut sub_buf)).await.unwrap().unwrap().unwrap();
    match insert_event {
        Response::Event(evt) => {
            assert_eq!(evt.table, "customers");
            assert_eq!(evt.action, ChangeAction::Insert);
            assert_eq!(evt.old_row, None);
            assert_eq!(evt.new_row, Some(vec![Value::Integer(1), Value::Text("Alice".into())]));
        }
        other => panic!("expected ChangeEvent, got {other:?}"),
    }

    // 2. Update
    write_request(&mut mut_writer, &Request::Query { sql: "UPDATE customers SET name = 'Alicia' WHERE id = 1".into() }).await.unwrap();
    let _ = read_response(&mut mut_reader, &mut mut_buf).await.unwrap();

    let update_event = timeout(Duration::from_secs(2), read_response(&mut sub_reader, &mut sub_buf)).await.unwrap().unwrap().unwrap();
    match update_event {
        Response::Event(evt) => {
            assert_eq!(evt.table, "customers");
            assert_eq!(evt.action, ChangeAction::Update);
            assert_eq!(evt.old_row, Some(vec![Value::Integer(1), Value::Text("Alice".into())]));
            assert_eq!(evt.new_row, Some(vec![Value::Integer(1), Value::Text("Alicia".into())]));
        }
        other => panic!("expected ChangeEvent, got {other:?}"),
    }

    // 3. Delete
    write_request(&mut mut_writer, &Request::Query { sql: "DELETE FROM customers WHERE id = 1".into() }).await.unwrap();
    let _ = read_response(&mut mut_reader, &mut mut_buf).await.unwrap();

    let delete_event = timeout(Duration::from_secs(2), read_response(&mut sub_reader, &mut sub_buf)).await.unwrap().unwrap().unwrap();
    match delete_event {
        Response::Event(evt) => {
            assert_eq!(evt.table, "customers");
            assert_eq!(evt.action, ChangeAction::Delete);
            assert_eq!(evt.old_row, Some(vec![Value::Integer(1), Value::Text("Alicia".into())]));
            assert_eq!(evt.new_row, None);
        }
        other => panic!("expected ChangeEvent, got {other:?}"),
    }
}

#[tokio::test]
async fn realtime_changefeed_respects_table_subscription_filters() {
    let file = NamedTempFile::new().unwrap();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let config = ServerConfig::new(addr, file.path());
    let (_server, bound_addr) = Server::bind(config).await.unwrap();

    // Client Orders: Subscribes only to "orders"
    let mut orders_socket = TcpStream::connect(bound_addr).await.unwrap();
    let (mut orders_reader, mut orders_writer) = orders_socket.split();
    let mut orders_buf = Vec::new();
    write_request(&mut orders_writer, &Request::Subscribe { table: Some("orders".into()) }).await.unwrap();
    assert_eq!(read_response(&mut orders_reader, &mut orders_buf).await.unwrap().unwrap(), Response::Subscribed);

    // Client All: Subscribes to all tables
    let mut all_socket = TcpStream::connect(bound_addr).await.unwrap();
    let (mut all_reader, mut all_writer) = all_socket.split();
    let mut all_buf = Vec::new();
    write_request(&mut all_writer, &Request::Subscribe { table: None }).await.unwrap();
    assert_eq!(read_response(&mut all_reader, &mut all_buf).await.unwrap().unwrap(), Response::Subscribed);

    // Client Mutator: Creates "items" and inserts row
    let mut mut_socket = TcpStream::connect(bound_addr).await.unwrap();
    let (mut mut_reader, mut mut_writer) = mut_socket.split();
    let mut mut_buf = Vec::new();

    write_request(&mut mut_writer, &Request::Query { sql: "CREATE TABLE items (id INTEGER PRIMARY KEY, title TEXT)".into() }).await.unwrap();
    let _ = read_response(&mut mut_reader, &mut mut_buf).await.unwrap();

    write_request(&mut mut_writer, &Request::Query { sql: "INSERT INTO items (id, title) VALUES (10, 'Book')".into() }).await.unwrap();
    let _ = read_response(&mut mut_reader, &mut mut_buf).await.unwrap();

    // Client All receives event for "items"
    let all_event = timeout(Duration::from_secs(2), read_response(&mut all_reader, &mut all_buf)).await.unwrap().unwrap().unwrap();
    match all_event {
        Response::Event(evt) => {
            assert_eq!(evt.table, "items");
            assert_eq!(evt.action, ChangeAction::Insert);
        }
        other => panic!("expected ChangeEvent on Client All, got {other:?}"),
    }

    // Client Orders receives NOTHING (times out cleanly)
    let orders_event = timeout(Duration::from_millis(200), read_response(&mut orders_reader, &mut orders_buf)).await;
    assert!(orders_event.is_err(), "Orders subscriber should not receive items event");
}

#[tokio::test]
async fn realtime_changefeed_buffers_events_until_commit_and_discards_on_rollback() {
    let file = NamedTempFile::new().unwrap();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let config = ServerConfig::new(addr, file.path());
    let (_server, bound_addr) = Server::bind(config).await.unwrap();

    // Subscriber
    let mut sub_socket = TcpStream::connect(bound_addr).await.unwrap();
    let (mut sub_reader, mut sub_writer) = sub_socket.split();
    let mut sub_buf = Vec::new();
    write_request(&mut sub_writer, &Request::Subscribe { table: Some("accounts".into()) }).await.unwrap();
    assert_eq!(read_response(&mut sub_reader, &mut sub_buf).await.unwrap().unwrap(), Response::Subscribed);

    // Mutator
    let mut mut_socket = TcpStream::connect(bound_addr).await.unwrap();
    let (mut mut_reader, mut mut_writer) = mut_socket.split();
    let mut mut_buf = Vec::new();

    write_request(&mut mut_writer, &Request::Query { sql: "CREATE TABLE accounts (id INTEGER PRIMARY KEY, balance INTEGER)".into() }).await.unwrap();
    let _ = read_response(&mut mut_reader, &mut mut_buf).await.unwrap();

    // 1. Transaction that ROLLS BACK
    write_request(&mut mut_writer, &Request::Query { sql: "BEGIN".into() }).await.unwrap();
    let _ = read_response(&mut mut_reader, &mut mut_buf).await.unwrap();

    write_request(&mut mut_writer, &Request::Query { sql: "INSERT INTO accounts (id, balance) VALUES (1, 500)".into() }).await.unwrap();
    let _ = read_response(&mut mut_reader, &mut mut_buf).await.unwrap();

    write_request(&mut mut_writer, &Request::Query { sql: "ROLLBACK".into() }).await.unwrap();
    let _ = read_response(&mut mut_reader, &mut mut_buf).await.unwrap();

    // Verify NO event was received during or after rollback
    let rolled_back_event = timeout(Duration::from_millis(200), read_response(&mut sub_reader, &mut sub_buf)).await;
    assert!(rolled_back_event.is_err(), "Rolled back transaction must NOT emit change events");

    // 2. Transaction that COMMITS
    write_request(&mut mut_writer, &Request::Query { sql: "BEGIN".into() }).await.unwrap();
    let _ = read_response(&mut mut_reader, &mut mut_buf).await.unwrap();

    write_request(&mut mut_writer, &Request::Query { sql: "INSERT INTO accounts (id, balance) VALUES (2, 1000)".into() }).await.unwrap();
    let _ = read_response(&mut mut_reader, &mut mut_buf).await.unwrap();

    // Still no event before COMMIT
    let before_commit = timeout(Duration::from_millis(100), read_response(&mut sub_reader, &mut sub_buf)).await;
    assert!(before_commit.is_err(), "Events must not emit prior to COMMIT");

    write_request(&mut mut_writer, &Request::Query { sql: "COMMIT".into() }).await.unwrap();
    let _ = read_response(&mut mut_reader, &mut mut_buf).await.unwrap();

    // Now event arrives!
    let commit_event = timeout(Duration::from_secs(2), read_response(&mut sub_reader, &mut sub_buf)).await.unwrap().unwrap().unwrap();
    match commit_event {
        Response::Event(evt) => {
            assert_eq!(evt.table, "accounts");
            assert_eq!(evt.action, ChangeAction::Insert);
            assert_eq!(evt.new_row, Some(vec![Value::Integer(2), Value::Integer(1000)]));
        }
        other => panic!("expected ChangeEvent after commit, got {other:?}"),
    }
}
