use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::net::TcpStream;

use dbengine::server::protocol::{read_response, write_request, ChangeAction, Request, Response};
use dbengine::types::value::Value;
use dbengine::{ExecResult, Server, ServerConfig};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_realtime_changefeed_cross_tenant_rls_isolation() {
    let file = NamedTempFile::new().unwrap();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let config = ServerConfig::new(addr, file.path());
    let (_server, bound_addr) = Server::bind(config).await.unwrap();

    // 1. Initial Setup: Create users & RLS notes table
    {
        let mut stream = TcpStream::connect(bound_addr).await.unwrap();
        let mut buf = Vec::new();

        write_request(
            &mut stream,
            &Request::Query {
                sql: "CREATE USER alice WITH PASSWORD 'password123' ROLE 'user'".into(),
            },
        )
        .await
        .unwrap();
        let _ = read_response(&mut stream, &mut buf).await.unwrap();

        write_request(
            &mut stream,
            &Request::Query {
                sql: "CREATE USER bob WITH PASSWORD 'password123' ROLE 'user'".into(),
            },
        )
        .await
        .unwrap();
        let _ = read_response(&mut stream, &mut buf).await.unwrap();

        write_request(
            &mut stream,
            &Request::Query {
                sql: "CREATE TABLE notes (id INTEGER PRIMARY KEY, user_id INTEGER, content TEXT)"
                    .into(),
            },
        )
        .await
        .unwrap();
        let _ = read_response(&mut stream, &mut buf).await.unwrap();

        write_request(
            &mut stream,
            &Request::Query {
                sql: "ALTER TABLE notes ENABLE ROW LEVEL SECURITY".into(),
            },
        )
        .await
        .unwrap();
        let _ = read_response(&mut stream, &mut buf).await.unwrap();

        write_request(
            &mut stream,
            &Request::Query {
                sql: "CREATE POLICY user_isolation ON notes FOR ALL USING (user_id = auth.uid())"
                    .into(),
            },
        )
        .await
        .unwrap();
        let _ = read_response(&mut stream, &mut buf).await.unwrap();
    }

    // 2. Connect Alice & subscribe to notes
    let mut alice_stream = TcpStream::connect(bound_addr).await.unwrap();
    let mut alice_buf = Vec::new();
    write_request(
        &mut alice_stream,
        &Request::Auth {
            username: "alice".into(),
            password: "password123".into(),
        },
    )
    .await
    .unwrap();
    let auth_resp = read_response(&mut alice_stream, &mut alice_buf)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(auth_resp, Response::AuthOk { .. }));

    write_request(
        &mut alice_stream,
        &Request::Subscribe {
            table: Some("notes".into()),
        },
    )
    .await
    .unwrap();
    let sub_resp = read_response(&mut alice_stream, &mut alice_buf)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sub_resp, Response::Subscribed);

    // 3. Connect Bob & subscribe to notes
    let mut bob_stream = TcpStream::connect(bound_addr).await.unwrap();
    let mut bob_buf = Vec::new();
    write_request(
        &mut bob_stream,
        &Request::Auth {
            username: "bob".into(),
            password: "password123".into(),
        },
    )
    .await
    .unwrap();
    let auth_resp_b = read_response(&mut bob_stream, &mut bob_buf)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(auth_resp_b, Response::AuthOk { .. }));

    write_request(
        &mut bob_stream,
        &Request::Subscribe {
            table: Some("notes".into()),
        },
    )
    .await
    .unwrap();
    let sub_resp_b = read_response(&mut bob_stream, &mut bob_buf)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sub_resp_b, Response::Subscribed);

    // 4. Alice inserts her own note (user_id = 2, since 1 is postgres admin, 2 is alice, 3 is bob)
    // Query alice's user_id
    write_request(
        &mut alice_stream,
        &Request::Query {
            sql: "INSERT INTO notes (id, user_id, content) VALUES (101, 2, 'Alice Note')".into(),
        },
    )
    .await
    .unwrap();
    let insert_res = read_response(&mut alice_stream, &mut alice_buf)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(insert_res, Response::Result(ExecResult::Modified(1)));

    // Alice MUST receive the Event
    let event_res = read_response(&mut alice_stream, &mut alice_buf)
        .await
        .unwrap()
        .unwrap();
    match event_res {
        Response::Event(evt) => {
            assert_eq!(evt.table, "notes");
            assert_eq!(evt.action, ChangeAction::Insert);
            assert_eq!(evt.new_row.unwrap()[2], Value::Text("Alice Note".into()));
        }
        other => panic!("expected Event, got {other:?}"),
    }

    // 5. Bob MUST NOT receive Alice's note (check non-blocking / timeout)
    let bob_rx = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        read_response(&mut bob_stream, &mut bob_buf),
    )
    .await;
    // Timeout elapsed without receiving any event
    assert!(
        bob_rx.is_err(),
        "Bob leaked Alice's private realtime event!"
    );
}
