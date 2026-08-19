//! Client session connection handler for ChocoBase.

use std::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::auth::{verify_jwt, verify_password, ExecutionContext};
use crate::engine::{ExecResult, SharedDatabase};
use crate::server::protocol::{read_request, write_response, Request, Response};
use crate::types::value::Value;

/// Handles an incoming TCP connection session, with the first byte already consumed by
/// the protocol dispatcher. `prefix_byte` is prepended to the session's read buffer so
/// the JSON framing parser sees the complete message.
pub async fn handle_session_with_prefix(
    socket: TcpStream,
    db: SharedDatabase,
    prefix_byte: u8,
) -> io::Result<()> {
    let (reader, writer) = socket.into_split();
    // Pre-populate the protocol read buffer with the already-consumed first byte.
    // This is simpler and avoids Chain<Cursor, OwnedReadHalf> type complexity.
    let initial_buffer = vec![prefix_byte];
    run_session(reader, writer, db, initial_buffer).await
}

/// Handles an incoming TCP connection session directly from a raw TcpStream.
/// The caller guarantees the stream starts with `{` (JSON protocol).
#[allow(dead_code)]
pub async fn handle_session(socket: TcpStream, db: SharedDatabase) -> io::Result<()> {
    let (reader, writer) = socket.into_split();
    run_session(reader, writer, db, Vec::new()).await
}

/// Core session loop: reads requests from `reader`, executes them, writes responses to `writer`.
/// `initial_buffer` pre-populates the protocol read buffer (e.g. to re-inject a consumed first byte).
async fn run_session<R, W>(
    mut reader: R,
    mut writer: W,
    db: SharedDatabase,
    mut buffer: Vec<u8>,
) -> io::Result<()>
where
    R: AsyncReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    let mut subscription: Option<(
        tokio::sync::broadcast::Receiver<crate::server::protocol::ChangeEvent>,
        Option<String>,
    )> = None;

    let mut exec_ctx = ExecutionContext::anonymous();

    loop {
        if let Some((rx, table_filter)) = &mut subscription {
            tokio::select! {
                req_res = read_request(&mut reader, &mut buffer) => {
                    match req_res {
                        Ok(Some(Request::Auth { username, password })) => {
                            let resp = handle_auth(&db, &username, &password, &mut exec_ctx);
                            write_response(&mut writer, &resp).await?;
                        }
                        Ok(Some(Request::Token { token })) => {
                            let resp = handle_token_auth(&token, &mut exec_ctx);
                            write_response(&mut writer, &resp).await?;
                        }
                        Ok(Some(Request::Query { sql })) => {
                            let response = match db.execute_with_context(&sql, &exec_ctx) {
                                Ok(result) => Response::Result(result),
                                Err(err) => Response::Error(err.to_string()),
                            };
                            write_response(&mut writer, &response).await?;
                        }
                        Ok(Some(Request::Ping)) => {
                            write_response(&mut writer, &Response::Pong).await?;
                        }
                        Ok(Some(Request::Subscribe { table })) => {
                            *table_filter = table;
                            write_response(&mut writer, &Response::Subscribed).await?;
                        }
                        Ok(Some(Request::Unsubscribe)) => {
                            subscription = None;
                            write_response(&mut writer, &Response::Unsubscribed).await?;
                        }
                        Ok(None) => break,
                        Err(e) => {
                            let _ = write_response(&mut writer, &Response::Error(e.to_string())).await;
                            break;
                        }
                    }
                }
                event_res = rx.recv() => {
                    match event_res {
                        Ok(event) => {
                            let table_matches = match table_filter {
                                Some(tbl) => &event.table == tbl,
                                None => true,
                            };
                            if table_matches && check_event_rls(&db, &event, &exec_ctx) {
                                write_response(&mut writer, &Response::Event(event)).await?;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            subscription = None;
                        }
                    }
                }
            }
        } else {
            match read_request(&mut reader, &mut buffer).await {
                Ok(Some(Request::Auth { username, password })) => {
                    let resp = handle_auth(&db, &username, &password, &mut exec_ctx);
                    write_response(&mut writer, &resp).await?;
                }
                Ok(Some(Request::Token { token })) => {
                    let resp = handle_token_auth(&token, &mut exec_ctx);
                    write_response(&mut writer, &resp).await?;
                }
                Ok(Some(Request::Query { sql })) => {
                    let response = match db.execute_with_context(&sql, &exec_ctx) {
                        Ok(result) => Response::Result(result),
                        Err(err) => Response::Error(err.to_string()),
                    };
                    write_response(&mut writer, &response).await?;
                }
                Ok(Some(Request::Ping)) => {
                    write_response(&mut writer, &Response::Pong).await?;
                }
                Ok(Some(Request::Subscribe { table })) => {
                    subscription = Some((db.subscribe(), table));
                    write_response(&mut writer, &Response::Subscribed).await?;
                }
                Ok(Some(Request::Unsubscribe)) => {
                    write_response(&mut writer, &Response::Unsubscribed).await?;
                }
                Ok(None) => break,
                Err(e) => {
                    let _ = write_response(&mut writer, &Response::Error(e.to_string())).await;
                    break;
                }
            }
        }
    }

    // Cleanly roll back any active transaction left open by this disconnected client.
    db.rollback_on_disconnect();

    Ok(())
}

fn handle_auth(
    db: &SharedDatabase,
    username: &str,
    password: &str,
    ctx: &mut ExecutionContext,
) -> Response {
    let safe_user = username.replace('\'', "''");
    let sql = format!("SELECT id, password_hash, role FROM _users WHERE username = '{safe_user}'");
    match db.execute_with_context(&sql, &ExecutionContext::admin()) {
        Ok(ExecResult::Rows { rows, .. }) if !rows.is_empty() => {
            let row = &rows[0];
            let user_id = match &row[0] {
                Value::Integer(id) => *id,
                _ => 1,
            };
            let hash = match &row[1] {
                Value::Text(h) => h.as_str(),
                _ => "",
            };
            let role = match &row[2] {
                Value::Text(r) => r.clone(),
                _ => "user".to_string(),
            };

            if verify_password(password, hash) {
                *ctx = ExecutionContext::authenticated(user_id, &role);
                Response::AuthOk {
                    user_id,
                    username: username.to_string(),
                    role,
                }
            } else {
                Response::Error("invalid credentials".to_string())
            }
        }
        _ => Response::Error("invalid credentials".to_string()),
    }
}

fn handle_token_auth(token: &str, ctx: &mut ExecutionContext) -> Response {
    let secret = crate::auth::jwt_secret();
    match verify_jwt(token, &secret) {
        Ok(claims) => {
            let user_id = claims.sub;
            let username = claims.username.clone();
            let role = claims.role.clone();
            *ctx = ExecutionContext::from_claims(&claims);
            Response::AuthOk {
                user_id,
                username,
                role,
            }
        }
        Err(_) => Response::Error("invalid or expired token".to_string()),
    }
}

fn check_event_rls(
    db: &SharedDatabase,
    event: &crate::server::protocol::ChangeEvent,
    ctx: &ExecutionContext,
) -> bool {
    if ctx.is_admin {
        return true;
    }
    let schema = match db.table_schema(&event.table) {
        Some(s) => s,
        None => return true,
    };
    if !schema.rls_enabled {
        return true;
    }
    if !ctx.is_authenticated() {
        return false;
    }

    let row = event.new_row.as_ref().or(event.old_row.as_ref());
    if let Some(r) = row {
        if let Some(user_idx) = schema.column_index("user_id") {
            if let Some(Value::Integer(uid)) = r.get(user_idx) {
                if Some(*uid) != ctx.user_id {
                    return false;
                }
            }
        }
    }

    true
}
