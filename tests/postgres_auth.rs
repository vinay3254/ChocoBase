use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use dbengine::server::protocol::{write_request, Request, Response};
use dbengine::{Database, Server, ServerConfig};

async fn connect_and_start_postgres(addr: SocketAddr, username: &str) -> (TcpStream, u8, u32) {
    let mut stream = TcpStream::connect(addr).await.unwrap();

    let mut startup_payload = Vec::new();
    startup_payload.extend_from_slice(&(196608u32).to_be_bytes());
    if !username.is_empty() {
        startup_payload
            .extend_from_slice(format!("user\0{username}\0database\0test\0\0").as_bytes());
    } else {
        startup_payload.extend_from_slice(b"\0\0");
    }
    let startup_len = (startup_payload.len() + 4) as u32;

    stream.write_all(&startup_len.to_be_bytes()).await.unwrap();
    stream.write_all(&startup_payload).await.unwrap();
    stream.flush().await.unwrap();

    let mut msg_type = [0u8; 1];
    stream.read_exact(&mut msg_type).await.unwrap();

    let mut msg_len_buf = [0u8; 4];
    stream.read_exact(&mut msg_len_buf).await.unwrap();
    let msg_len = u32::from_be_bytes(msg_len_buf);

    (stream, msg_type[0], msg_len)
}

#[tokio::test]
async fn test_postgres_wire_authentication_success_and_failures() {
    let file = NamedTempFile::new().unwrap();
    {
        // Seed a standard user and an admin user
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE USER alice WITH PASSWORD 'alice_secret_123' ROLE 'user'")
            .unwrap();
    }

    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let config = ServerConfig::new(addr, file.path());
    let (_server, bound_addr) = Server::bind(config).await.unwrap();

    // 1. Test missing user in startup packet -> ErrorResponse 'E' (code 28000)
    let (_stream1, msg_type1, _) = connect_and_start_postgres(bound_addr, "").await;
    assert_eq!(msg_type1, b'E');

    // 2. Test unknown user -> Cleartext challenge, but invalid response -> ErrorResponse 'E' (code 28P01)
    let (mut stream2, msg_type2, _) = connect_and_start_postgres(bound_addr, "bob").await;
    assert_eq!(msg_type2, b'R'); // sends challenge
    let mut code_buf = [0u8; 4];
    stream2.read_exact(&mut code_buf).await.unwrap();
    assert_eq!(u32::from_be_bytes(code_buf), 3);

    // Send password for bob
    let pass_payload = b"somepassword\0";
    let pass_len = (pass_payload.len() + 4) as u32;
    stream2.write_all(b"p").await.unwrap();
    stream2.write_all(&pass_len.to_be_bytes()).await.unwrap();
    stream2.write_all(pass_payload).await.unwrap();
    stream2.flush().await.unwrap();

    let mut err_type = [0u8; 1];
    stream2.read_exact(&mut err_type).await.unwrap();
    assert_eq!(err_type[0], b'E'); // Authentication error

    // 3. Test known user alice with WRONG password -> ErrorResponse 'E'
    let (mut stream3, msg_type3, _) = connect_and_start_postgres(bound_addr, "alice").await;
    assert_eq!(msg_type3, b'R');
    let mut code_buf = [0u8; 4];
    stream3.read_exact(&mut code_buf).await.unwrap();
    assert_eq!(u32::from_be_bytes(code_buf), 3);

    let wrong_pass = b"wrong_password\0";
    let pass_len = (wrong_pass.len() + 4) as u32;
    stream3.write_all(b"p").await.unwrap();
    stream3.write_all(&pass_len.to_be_bytes()).await.unwrap();
    stream3.write_all(wrong_pass).await.unwrap();
    stream3.flush().await.unwrap();

    let mut err_type3 = [0u8; 1];
    stream3.read_exact(&mut err_type3).await.unwrap();
    assert_eq!(err_type3[0], b'E');

    // 4. Test known user alice with CORRECT password -> AuthenticationOk ('R', code 0)
    let (mut stream4, msg_type4, _) = connect_and_start_postgres(bound_addr, "alice").await;
    assert_eq!(msg_type4, b'R');
    let mut code_buf = [0u8; 4];
    stream4.read_exact(&mut code_buf).await.unwrap();
    assert_eq!(u32::from_be_bytes(code_buf), 3);

    let right_pass = b"alice_secret_123\0";
    let pass_len = (right_pass.len() + 4) as u32;
    stream4.write_all(b"p").await.unwrap();
    stream4.write_all(&pass_len.to_be_bytes()).await.unwrap();
    stream4.write_all(right_pass).await.unwrap();
    stream4.flush().await.unwrap();

    let mut auth_ok_type = [0u8; 1];
    stream4.read_exact(&mut auth_ok_type).await.unwrap();
    assert_eq!(auth_ok_type[0], b'R');
    let mut auth_ok_len = [0u8; 4];
    stream4.read_exact(&mut auth_ok_len).await.unwrap();
    let mut auth_ok_code = [0u8; 4];
    stream4.read_exact(&mut auth_ok_code).await.unwrap();
    assert_eq!(u32::from_be_bytes(auth_ok_code), 0); // Auth OK!
}

#[tokio::test]
async fn test_tcp_json_protocol_session_authentication() {
    let file = NamedTempFile::new().unwrap();
    {
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE USER charlie WITH PASSWORD 'charlie_pass' ROLE 'user'")
            .unwrap();
        db.execute("CREATE TABLE notes (id INTEGER PRIMARY KEY, author TEXT, content TEXT)")
            .unwrap();
        db.execute(
            "INSERT INTO notes (id, author, content) VALUES (1, 'charlie', 'My Secret Note')",
        )
        .unwrap();
    }

    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let config = ServerConfig::new(addr, file.path());
    let (_server, bound_addr) = Server::bind(config).await.unwrap();

    let mut stream = TcpStream::connect(bound_addr).await.unwrap();

    // 1. Authenticate with wrong password -> Response::Error
    let auth_req_wrong = Request::Auth {
        username: "charlie".to_string(),
        password: "bad_password".to_string(),
    };
    write_request(&mut stream, &auth_req_wrong).await.unwrap();

    let mut line_buf = [0u8; 1024];
    let n = stream.read(&mut line_buf).await.unwrap();
    let resp1: Response = serde_json::from_slice(&line_buf[..n.saturating_sub(1)]).unwrap();
    assert!(matches!(resp1, Response::Error(_)));

    // 2. Authenticate with correct password -> Response::AuthOk
    let auth_req_right = Request::Auth {
        username: "charlie".to_string(),
        password: "charlie_pass".to_string(),
    };
    write_request(&mut stream, &auth_req_right).await.unwrap();

    let n = stream.read(&mut line_buf).await.unwrap();
    let resp2: Response = serde_json::from_slice(&line_buf[..n.saturating_sub(1)]).unwrap();
    assert!(matches!(resp2, Response::AuthOk { .. }));

    // 3. Query as authenticated charlie
    let query_req = Request::Query {
        sql: "SELECT id, author, content FROM notes".to_string(),
    };
    write_request(&mut stream, &query_req).await.unwrap();

    let n = stream.read(&mut line_buf).await.unwrap();
    let resp3: Response = serde_json::from_slice(&line_buf[..n.saturating_sub(1)]).unwrap();
    assert!(matches!(resp3, Response::Result(_)));
}
