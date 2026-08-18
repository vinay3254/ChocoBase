//! PostgreSQL Wire Protocol v3 Server Handler for ChocoBase.
//! Enables standard PostgreSQL clients (psql, Node pg, Python psycopg, ORMs) to connect directly.

use std::collections::HashMap;
use std::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::auth::{verify_password, ExecutionContext};
use crate::engine::{ExecResult, SharedDatabase};
use crate::types::value::Value;

/// Entry point for PostgreSQL sessions when the listener has already consumed the first byte
/// for protocol detection. `prefix_byte` (always 0x00) is prepended to reconstruct the full
/// 4-byte startup message length before dispatching to the session body.
pub async fn handle_postgres_session_with_prefix(
    mut socket: tokio::net::TcpStream,
    db: SharedDatabase,
    prefix_byte: u8,
) -> io::Result<()> {
    let mut rest = [0u8; 3];
    socket.read_exact(&mut rest).await?;
    let len_buf = [prefix_byte, rest[0], rest[1], rest[2]];
    handle_postgres_session_with_lenbuf(&mut socket, db, len_buf).await
}

pub async fn handle_postgres_session(
    mut socket: tokio::net::TcpStream,
    db: SharedDatabase,
) -> io::Result<()> {
    let mut len_buf = [0u8; 4];
    socket.read_exact(&mut len_buf).await?;
    handle_postgres_session_with_lenbuf(&mut socket, db, len_buf).await
}

fn parse_startup_params(body: &[u8]) -> HashMap<String, String> {
    let mut params = HashMap::new();
    if body.len() <= 4 {
        return params;
    }
    let s = String::from_utf8_lossy(&body[4..]);
    let parts: Vec<&str> = s.split('\0').filter(|p| !p.is_empty()).collect();
    for chunk in parts.chunks(2) {
        if chunk.len() == 2 {
            params.insert(chunk[0].to_string(), chunk[1].to_string());
        }
    }
    params
}

async fn handle_postgres_session_with_lenbuf(
    socket: &mut tokio::net::TcpStream,
    db: SharedDatabase,
    first_len_buf: [u8; 4],
) -> io::Result<()> {
    let mut len_buf = first_len_buf;
    let mut packet_len = u32::from_be_bytes(len_buf) as usize;

    let mut body = vec![0u8; packet_len.saturating_sub(4)];
    socket.read_exact(&mut body).await?;

    if body.len() >= 4 {
        let code = u32::from_be_bytes([body[0], body[1], body[2], body[3]]);
        // SSL Request (80877103) -> Reply 'N' (SSL not supported in plaintext dev mode)
        if code == 80877103 {
            socket.write_all(b"N").await?;
            socket.flush().await?;

            // Read the real startup packet
            socket.read_exact(&mut len_buf).await?;
            packet_len = u32::from_be_bytes(len_buf) as usize;
            body = vec![0u8; packet_len.saturating_sub(4)];
            socket.read_exact(&mut body).await?;
        }
    }

    let params = parse_startup_params(&body);
    let username = match params.get("user") {
        Some(u) if !u.trim().is_empty() => u.trim().to_string(),
        _ => {
            write_error_response(
                socket,
                "28000",
                "no PostgreSQL user name specified in startup packet",
            )
            .await?;
            socket.flush().await?;
            return Ok(());
        }
    };

    // 1. Send AuthenticationCleartextPassword challenge ('R', len 8, code 3)
    socket
        .write_all(b"R\x00\x00\x00\x08\x00\x00\x00\x03")
        .await?;
    socket.flush().await?;

    // 2. Read PasswordMessage ('p')
    let mut pass_type = [0u8; 1];
    socket.read_exact(&mut pass_type).await?;
    if pass_type[0] != b'p' {
        write_error_response(socket, "28P01", "expected password message").await?;
        socket.flush().await?;
        return Ok(());
    }

    let mut pass_len_buf = [0u8; 4];
    socket.read_exact(&mut pass_len_buf).await?;
    let pass_len = u32::from_be_bytes(pass_len_buf) as usize;
    let mut pass_body = vec![0u8; pass_len.saturating_sub(4)];
    socket.read_exact(&mut pass_body).await?;

    let password = String::from_utf8_lossy(&pass_body)
        .trim_matches('\0')
        .to_string();

    // 3. Verify user credentials against _users table
    let safe_user = username.replace('\'', "''");
    let select_user_sql =
        format!("SELECT id, password_hash, role FROM _users WHERE username = '{safe_user}'");

    let auth_result = db.execute_with_context(&select_user_sql, &ExecutionContext::admin());
    let mut authenticated_ctx = None;

    if let Ok(ExecResult::Rows { rows, .. }) = auth_result {
        if let Some(r) = rows.first() {
            let user_id = match &r[0] {
                Value::Integer(id) => *id,
                _ => 1,
            };
            let hash = match &r[1] {
                Value::Text(h) => h.as_str(),
                _ => "",
            };
            let role = match &r[2] {
                Value::Text(role_str) => role_str.as_str(),
                _ => "user",
            };

            if verify_password(&password, hash) {
                authenticated_ctx = Some(ExecutionContext::authenticated(user_id, role));
            }
        }
    }

    let exec_ctx = match authenticated_ctx {
        Some(ctx) => ctx,
        None => {
            write_error_response(
                socket,
                "28P01",
                &format!("password authentication failed for user \"{}\"", username),
            )
            .await?;
            socket.flush().await?;
            return Ok(());
        }
    };

    // Send AuthenticationOk ('R', len 8, code 0)
    write_auth_ok(socket).await?;

    // Send standard ParameterStatus ('S')
    write_parameter_status(socket, "server_version", "15.0 (ChocoBase)").await?;
    write_parameter_status(socket, "client_encoding", "UTF8").await?;
    write_parameter_status(socket, "server_encoding", "UTF8").await?;
    write_parameter_status(socket, "DateStyle", "ISO, MDY").await?;
    write_parameter_status(socket, "standard_conforming_strings", "on").await?;

    // Send ReadyForQuery ('Z', len 5, 'I')
    write_ready_for_query(socket, b'I').await?;
    socket.flush().await?;

    // 4. Query execution loop using authenticated execution context
    loop {
        let mut msg_type_buf = [0u8; 1];
        match socket.read_exact(&mut msg_type_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e),
        }

        let msg_type = msg_type_buf[0];
        socket.read_exact(&mut len_buf).await?;
        let msg_len = u32::from_be_bytes(len_buf) as usize;
        let mut msg_body = vec![0u8; msg_len.saturating_sub(4)];
        socket.read_exact(&mut msg_body).await?;

        match msg_type {
            b'Q' => {
                // Simple Query
                let sql = String::from_utf8_lossy(&msg_body)
                    .trim_matches('\0')
                    .trim()
                    .to_string();

                if sql.is_empty() {
                    // EmptyQueryResponse ('I', len 4)
                    socket.write_all(b"I\x00\x00\x00\x04").await?;
                } else {
                    // Normalize standard pg driver discovery queries
                    let trimmed_sql = sql.trim_end_matches(';').trim();
                    if trimmed_sql.eq_ignore_ascii_case("SELECT 1")
                        || trimmed_sql.eq_ignore_ascii_case("SELECT 1 AS one")
                    {
                        write_row_description(socket, &["one".to_string()]).await?;
                        write_data_row(socket, &[Value::Integer(1)]).await?;
                        write_command_complete(socket, "SELECT 1").await?;
                    } else {
                        match db.execute_with_context(&sql, &exec_ctx) {
                            Ok(ExecResult::Rows { columns, rows }) => {
                                write_row_description(socket, &columns).await?;
                                for row in &rows {
                                    write_data_row(socket, row).await?;
                                }
                                write_command_complete(socket, &format!("SELECT {}", rows.len()))
                                    .await?;
                            }
                            Ok(ExecResult::Modified(count)) => {
                                let tag = if sql.trim_start().to_uppercase().starts_with("INSERT") {
                                    format!("INSERT 0 {count}")
                                } else if sql.trim_start().to_uppercase().starts_with("UPDATE") {
                                    format!("UPDATE {count}")
                                } else if sql.trim_start().to_uppercase().starts_with("DELETE") {
                                    format!("DELETE {count}")
                                } else {
                                    format!("SET {count}")
                                };
                                write_command_complete(socket, &tag).await?;
                            }
                            Ok(ExecResult::Ok) => {
                                let tag = if sql
                                    .trim_start()
                                    .to_uppercase()
                                    .starts_with("CREATE TABLE")
                                {
                                    "CREATE TABLE"
                                } else if sql.trim_start().to_uppercase().starts_with("DROP TABLE")
                                {
                                    "DROP TABLE"
                                } else if sql
                                    .trim_start()
                                    .to_uppercase()
                                    .starts_with("CREATE INDEX")
                                {
                                    "CREATE INDEX"
                                } else if sql.trim_start().to_uppercase().starts_with("BEGIN") {
                                    "BEGIN"
                                } else if sql.trim_start().to_uppercase().starts_with("COMMIT") {
                                    "COMMIT"
                                } else if sql.trim_start().to_uppercase().starts_with("ROLLBACK") {
                                    "ROLLBACK"
                                } else {
                                    "OK"
                                };
                                write_command_complete(socket, tag).await?;
                            }
                            Err(err) => {
                                write_error_response(socket, "42601", &err.to_string()).await?;
                            }
                        }
                    }
                }

                write_ready_for_query(socket, b'I').await?;
                socket.flush().await?;
            }
            b'X' => {
                // Terminate
                break;
            }
            _ => {
                // Unsupported message type - reply ReadyForQuery
                write_ready_for_query(socket, b'I').await?;
                socket.flush().await?;
            }
        }
    }

    Ok(())
}

async fn write_auth_ok(writer: &mut TcpStream) -> io::Result<()> {
    writer.write_all(b"R\x00\x00\x00\x08\x00\x00\x00\x00").await
}

async fn write_parameter_status(writer: &mut TcpStream, name: &str, val: &str) -> io::Result<()> {
    let mut payload = Vec::new();
    payload.extend_from_slice(name.as_bytes());
    payload.push(0);
    payload.extend_from_slice(val.as_bytes());
    payload.push(0);

    let len = (payload.len() + 4) as u32;
    writer.write_all(b"S").await?;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(&payload).await
}

async fn write_ready_for_query(writer: &mut TcpStream, status: u8) -> io::Result<()> {
    writer.write_all(&[b'Z', 0, 0, 0, 5, status]).await
}

async fn write_row_description(writer: &mut TcpStream, columns: &[String]) -> io::Result<()> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&(columns.len() as u16).to_be_bytes());

    for col in columns {
        payload.extend_from_slice(col.as_bytes());
        payload.push(0); // null-terminated
        payload.extend_from_slice(&0u32.to_be_bytes()); // table_oid
        payload.extend_from_slice(&0u16.to_be_bytes()); // col_attr
        payload.extend_from_slice(&25u32.to_be_bytes()); // type_oid (25 = text)
        payload.extend_from_slice(&(-1i16).to_be_bytes()); // type_len
        payload.extend_from_slice(&(-1i32).to_be_bytes()); // type_mod
        payload.extend_from_slice(&0u16.to_be_bytes()); // format_code (0 = text)
    }

    let len = (payload.len() + 4) as u32;
    writer.write_all(b"T").await?;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(&payload).await
}

async fn write_data_row(writer: &mut TcpStream, row: &[Value]) -> io::Result<()> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&(row.len() as u16).to_be_bytes());

    for val in row {
        match val {
            Value::Null => {
                payload.extend_from_slice(&(-1i32).to_be_bytes());
            }
            _ => {
                let s = match val {
                    Value::Integer(i) => i.to_string(),
                    Value::Text(t) | Value::Json(t) => t.clone(),
                    Value::Boolean(b) => b.to_string(),
                    Value::Null => String::new(),
                };
                let bytes = s.as_bytes();
                payload.extend_from_slice(&(bytes.len() as i32).to_be_bytes());
                payload.extend_from_slice(bytes);
            }
        }
    }

    let len = (payload.len() + 4) as u32;
    writer.write_all(b"D").await?;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(&payload).await
}

async fn write_command_complete(writer: &mut TcpStream, tag: &str) -> io::Result<()> {
    let mut payload = Vec::new();
    payload.extend_from_slice(tag.as_bytes());
    payload.push(0);

    let len = (payload.len() + 4) as u32;
    writer.write_all(b"C").await?;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(&payload).await
}

async fn write_error_response(writer: &mut TcpStream, code: &str, message: &str) -> io::Result<()> {
    let mut payload = Vec::new();
    payload.push(b'S');
    payload.extend_from_slice(b"ERROR\0");
    payload.push(b'C');
    payload.extend_from_slice(code.as_bytes());
    payload.push(0);
    payload.push(b'M');
    payload.extend_from_slice(message.as_bytes());
    payload.push(0);
    payload.push(0); // Terminator

    let len = (payload.len() + 4) as u32;
    writer.write_all(b"E").await?;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(&payload).await
}
