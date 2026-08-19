use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use dbengine::{Server, ServerConfig};

#[tokio::test]
async fn test_postgres_wire_copy_in_and_out_lifecycle() {
    let file = NamedTempFile::new().unwrap();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let config = ServerConfig::new(addr, file.path());
    let (_server, bound_addr) = Server::bind(config).await.unwrap();

    let mut stream = TcpStream::connect(bound_addr).await.unwrap();

    // 1. SSL Negation ('N')
    let ssl_req = [0, 0, 0, 8, 0x04, 0xd2, 0x16, 0x2f];
    stream.write_all(&ssl_req).await.unwrap();
    let mut ssl_resp = [0u8; 1];
    stream.read_exact(&mut ssl_resp).await.unwrap();
    assert_eq!(ssl_resp[0], b'N');

    // 2. StartupMessage (Protocol 3.0, user=postgres, database=test)
    let mut startup_payload = Vec::new();
    startup_payload.extend_from_slice(&(196608u32).to_be_bytes());
    startup_payload.extend_from_slice(b"user\0postgres\0database\0test\0\0");
    let startup_len = (startup_payload.len() + 4) as u32;

    stream.write_all(&startup_len.to_be_bytes()).await.unwrap();
    stream.write_all(&startup_payload).await.unwrap();
    stream.flush().await.unwrap();

    // Read AuthenticationCleartextPassword challenge ('R')
    let mut auth_hdr = [0u8; 5];
    stream.read_exact(&mut auth_hdr).await.unwrap();
    assert_eq!(auth_hdr[0], b'R');
    let mut auth_code = [0u8; 4];
    stream.read_exact(&mut auth_code).await.unwrap();

    // Send PasswordMessage ('p', password: "postgres\0")
    let pass_payload = b"postgres\0";
    let pass_len = (pass_payload.len() + 4) as u32;
    stream.write_all(b"p").await.unwrap();
    stream.write_all(&pass_len.to_be_bytes()).await.unwrap();
    stream.write_all(pass_payload).await.unwrap();
    stream.flush().await.unwrap();

    // Read AuthenticationOk ('R')
    let mut auth_ok_hdr = [0u8; 5];
    stream.read_exact(&mut auth_ok_hdr).await.unwrap();
    assert_eq!(auth_ok_hdr[0], b'R');
    let mut auth_ok_code = [0u8; 4];
    stream.read_exact(&mut auth_ok_code).await.unwrap();
    assert_eq!(u32::from_be_bytes(auth_ok_code), 0);

    // Consume ParameterStatuses until ReadyForQuery ('Z')
    loop {
        let mut msg_type = [0u8; 1];
        stream.read_exact(&mut msg_type).await.unwrap();
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await.unwrap();
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut body = vec![0u8; len - 4];
        stream.read_exact(&mut body).await.unwrap();

        if msg_type[0] == b'Z' {
            break;
        }
    }

    // 3. Create Table via Simple Query ('Q')
    send_query(
        &mut stream,
        "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT, price FLOAT)",
    )
    .await;
    read_until_ready(&mut stream).await;

    // 4. Send COPY items (id, name, price) FROM STDIN
    let copy_sql = "COPY items (id, name, price) FROM STDIN\0";
    let q_len = (copy_sql.len() + 4) as u32;
    stream.write_all(b"Q").await.unwrap();
    stream.write_all(&q_len.to_be_bytes()).await.unwrap();
    stream.write_all(copy_sql.as_bytes()).await.unwrap();

    // Expect CopyInResponse ('G')
    let mut g_type = [0u8; 1];
    stream.read_exact(&mut g_type).await.unwrap();
    assert_eq!(g_type[0], b'G', "expected CopyInResponse ('G')");
    let mut g_len_buf = [0u8; 4];
    stream.read_exact(&mut g_len_buf).await.unwrap();
    let g_len = u32::from_be_bytes(g_len_buf) as usize;
    let mut g_body = vec![0u8; g_len - 4];
    stream.read_exact(&mut g_body).await.unwrap();

    // Send CopyData ('d') with 2 rows
    let data_payload = "1\tMechanical Keyboard\t89.99\n2\tWireless Mouse\t34.50\n";
    let d_len = (data_payload.len() + 4) as u32;
    stream.write_all(b"d").await.unwrap();
    stream.write_all(&d_len.to_be_bytes()).await.unwrap();
    stream.write_all(data_payload.as_bytes()).await.unwrap();

    // Send CopyDone ('c')
    stream.write_all(b"c\x00\x00\x00\x04").await.unwrap();

    // Read CommandComplete ('C') and ReadyForQuery ('Z')
    let (tag, _) = read_until_ready(&mut stream).await;
    assert_eq!(tag, "COPY 2");

    // 5. Send COPY items TO STDOUT
    let copy_out_sql = "COPY items TO STDOUT\0";
    let q_out_len = (copy_out_sql.len() + 4) as u32;
    stream.write_all(b"Q").await.unwrap();
    stream.write_all(&q_out_len.to_be_bytes()).await.unwrap();
    stream.write_all(copy_out_sql.as_bytes()).await.unwrap();

    // Expect CopyOutResponse ('H'), CopyData ('d'), CopyDone ('c'), CommandComplete ('C'), ReadyForQuery ('Z')
    let (out_tag, out_data) = read_copy_out(&mut stream).await;
    assert_eq!(out_tag, "COPY 2");
    assert!(out_data.contains("Mechanical Keyboard"));
    assert!(out_data.contains("Wireless Mouse"));
}

async fn send_query(stream: &mut TcpStream, sql: &str) {
    let null_sql = format!("{sql}\0");
    let len = (null_sql.len() + 4) as u32;
    stream.write_all(b"Q").await.unwrap();
    stream.write_all(&len.to_be_bytes()).await.unwrap();
    stream.write_all(null_sql.as_bytes()).await.unwrap();
}

async fn read_until_ready(stream: &mut TcpStream) -> (String, Vec<u8>) {
    let mut last_tag = String::new();
    let mut data = Vec::new();
    loop {
        let mut msg_type = [0u8; 1];
        stream.read_exact(&mut msg_type).await.unwrap();
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await.unwrap();
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut body = vec![0u8; len - 4];
        stream.read_exact(&mut body).await.unwrap();

        if msg_type[0] == b'C' {
            last_tag = String::from_utf8_lossy(&body)
                .trim_matches('\0')
                .to_string();
        } else if msg_type[0] == b'd' {
            data.extend_from_slice(&body);
        } else if msg_type[0] == b'Z' {
            break;
        }
    }
    (last_tag, data)
}

async fn read_copy_out(stream: &mut TcpStream) -> (String, String) {
    let mut last_tag = String::new();
    let mut rows_text = String::new();
    loop {
        let mut msg_type = [0u8; 1];
        stream.read_exact(&mut msg_type).await.unwrap();
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await.unwrap();
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut body = vec![0u8; len - 4];
        stream.read_exact(&mut body).await.unwrap();

        if msg_type[0] == b'd' {
            rows_text.push_str(&String::from_utf8_lossy(&body));
        } else if msg_type[0] == b'C' {
            last_tag = String::from_utf8_lossy(&body)
                .trim_matches('\0')
                .to_string();
        } else if msg_type[0] == b'Z' {
            break;
        }
    }
    (last_tag, rows_text)
}
