//! Automated integration tests for Bulk Data COPY IN / COPY OUT Protocol
//! and Schema-to-Code Type Generation.

use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::net::TcpStream;

use dbengine::server::protocol::{read_response, write_request, Request, Response};
use dbengine::server::{Server, ServerConfig};
use dbengine::Value;

#[tokio::test]
async fn test_bulk_copy_in_and_copy_out_wire_protocol() {
    let file = NamedTempFile::new().unwrap();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let config = ServerConfig::new(addr, file.path());
    let (server, bound_addr) = Server::bind(config).await.unwrap();

    let mut stream = TcpStream::connect(bound_addr).await.unwrap();
    let mut buffer = Vec::new();

    // 1. Create table
    let create_sql = "CREATE TABLE metrics (id INTEGER PRIMARY KEY, metric_name TEXT NOT NULL, val FLOAT NOT NULL);";
    write_request(&mut stream, &Request::Query { sql: create_sql.to_string() }).await.unwrap();
    let resp = read_response(&mut stream, &mut buffer).await.unwrap().unwrap();
    assert!(matches!(resp, Response::Result(_)));

    // 2. High-throughput Bulk COPY IN (50 rows in a single atomic transmission)
    let mut bulk_rows = Vec::new();
    for i in 1..=50 {
        bulk_rows.push(vec![
            Value::Integer(i),
            Value::Text(format!("cpu_load_{i}")),
            Value::Float(0.12 * i as f64),
        ]);
    }

    write_request(
        &mut stream,
        &Request::CopyIn {
            table: "metrics".to_string(),
            rows: bulk_rows,
        },
    )
    .await
    .unwrap();

    let copy_in_resp = read_response(&mut stream, &mut buffer).await.unwrap().unwrap();
    match copy_in_resp {
        Response::CopyDone { rows_copied } => assert_eq!(rows_copied, 50),
        other => panic!("expected CopyDone response, got {:?}", other),
    }

    // 3. High-throughput Bulk COPY OUT
    write_request(&mut stream, &Request::CopyOut { table: "metrics".to_string() }).await.unwrap();
    let copy_out_resp = read_response(&mut stream, &mut buffer).await.unwrap().unwrap();
    match copy_out_resp {
        Response::CopyData { rows } => {
            assert_eq!(rows.len(), 50);
            assert_eq!(rows[0][0], Value::Integer(1));
            assert_eq!(rows[49][0], Value::Integer(50));
        }
        other => panic!("expected CopyData response, got {:?}", other),
    }

    server.shutdown();
}
