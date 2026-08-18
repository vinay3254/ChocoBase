use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use dbengine::{Server, ServerConfig};

#[tokio::test]
async fn test_postgres_wire_protocol_handshake_and_query_execution() {
    let file = NamedTempFile::new().unwrap();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let config = ServerConfig::new(addr, file.path());
    let (_server, bound_addr) = Server::bind(config).await.unwrap();

    let mut stream = TcpStream::connect(bound_addr).await.unwrap();

    // 1. Send SSLRequest (length 8, code 80877103)
    let ssl_req = [0, 0, 0, 8, 0x04, 0xd2, 0x16, 0x2f];
    stream.write_all(&ssl_req).await.unwrap();
    stream.flush().await.unwrap();

    // Expect 'N'
    let mut ssl_resp = [0u8; 1];
    stream.read_exact(&mut ssl_resp).await.unwrap();
    assert_eq!(ssl_resp[0], b'N');

    // 2. Send StartupMessage (v3.0 = 196608, params: user\0postgres\0database\0test\0\0)
    let mut startup_payload = Vec::new();
    startup_payload.extend_from_slice(&(196608u32).to_be_bytes());
    startup_payload.extend_from_slice(b"user\0postgres\0database\0test\0\0");
    let startup_len = (startup_payload.len() + 4) as u32;

    stream.write_all(&startup_len.to_be_bytes()).await.unwrap();
    stream.write_all(&startup_payload).await.unwrap();
    stream.flush().await.unwrap();

    // Read AuthenticationOk ('R', len 8, code 0)
    let mut auth_hdr = [0u8; 5];
    stream.read_exact(&mut auth_hdr).await.unwrap();
    assert_eq!(auth_hdr[0], b'R');
    let mut auth_code = [0u8; 4];
    stream.read_exact(&mut auth_code).await.unwrap();
    assert_eq!(u32::from_be_bytes(auth_code), 0);

    // Read ParameterStatus messages until ReadyForQuery ('Z')
    loop {
        let mut msg_type = [0u8; 1];
        stream.read_exact(&mut msg_type).await.unwrap();
        let mut msg_len_buf = [0u8; 4];
        stream.read_exact(&mut msg_len_buf).await.unwrap();
        let msg_len = u32::from_be_bytes(msg_len_buf) as usize;
        let mut msg_body = vec![0u8; msg_len - 4];
        stream.read_exact(&mut msg_body).await.unwrap();

        if msg_type[0] == b'Z' {
            assert_eq!(msg_body[0], b'I');
            break;
        }
    }

    // 3. Send Simple Query: CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT)
    send_simple_query(&mut stream, "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT)").await;
    let (tag, rows) = read_query_response(&mut stream).await;
    assert_eq!(tag, "CREATE TABLE");
    assert_eq!(rows.len(), 0);

    // 4. Send Simple Query: INSERT INTO items (id, name) VALUES (1, 'Book')
    send_simple_query(&mut stream, "INSERT INTO items (id, name) VALUES (1, 'Book')").await;
    let (tag, rows) = read_query_response(&mut stream).await;
    assert_eq!(tag, "INSERT 0 1");
    assert_eq!(rows.len(), 0);

    // 5. Send Simple Query: SELECT * FROM items
    send_simple_query(&mut stream, "SELECT * FROM items").await;
    let (tag, rows) = read_query_response(&mut stream).await;
    assert_eq!(tag, "SELECT 1");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0], vec!["1", "Book"]);

    // 6. Terminate ('X', len 4)
    stream.write_all(b"X\x00\x00\x00\x04").await.unwrap();
}

async fn send_simple_query(stream: &mut TcpStream, sql: &str) {
    let mut payload = sql.as_bytes().to_vec();
    payload.push(0);
    let len = (payload.len() + 4) as u32;

    stream.write_all(b"Q").await.unwrap();
    stream.write_all(&len.to_be_bytes()).await.unwrap();
    stream.write_all(&payload).await.unwrap();
    stream.flush().await.unwrap();
}

async fn read_query_response(stream: &mut TcpStream) -> (String, Vec<Vec<String>>) {
    let mut command_tag = String::new();
    let mut rows = Vec::new();

    loop {
        let mut msg_type = [0u8; 1];
        stream.read_exact(&mut msg_type).await.unwrap();
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await.unwrap();
        let msg_len = u32::from_be_bytes(len_buf) as usize;
        let mut msg_body = vec![0u8; msg_len - 4];
        stream.read_exact(&mut msg_body).await.unwrap();

        match msg_type[0] {
            b'T' => {
                // RowDescription
            }
            b'D' => {
                // DataRow
                let col_count = u16::from_be_bytes([msg_body[0], msg_body[1]]) as usize;
                let mut cursor = 2;
                let mut row = Vec::new();
                for _ in 0..col_count {
                    let col_len = i32::from_be_bytes([
                        msg_body[cursor],
                        msg_body[cursor + 1],
                        msg_body[cursor + 2],
                        msg_body[cursor + 3],
                    ]);
                    cursor += 4;
                    if col_len == -1 {
                        row.push("NULL".to_string());
                    } else {
                        let len = col_len as usize;
                        let text = String::from_utf8_lossy(&msg_body[cursor..cursor + len]).to_string();
                        cursor += len;
                        row.push(text);
                    }
                }
                rows.push(row);
            }
            b'C' => {
                // CommandComplete
                command_tag = String::from_utf8_lossy(&msg_body).trim_matches('\0').to_string();
            }
            b'Z' => {
                // ReadyForQuery
                break;
            }
            b'E' => {
                // ErrorResponse
                panic!("received error response: {:?}", String::from_utf8_lossy(&msg_body));
            }
            _ => {}
        }
    }

    (command_tag, rows)
}
