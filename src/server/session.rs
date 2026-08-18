//! Client session connection handler for ChocoBase.

use std::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::engine::SharedDatabase;
use crate::server::protocol::{read_request, write_response, Request, Response};

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

    loop {
        if let Some((rx, table_filter)) = &mut subscription {
            tokio::select! {
                req_res = read_request(&mut reader, &mut buffer) => {
                    match req_res {
                        Ok(Some(Request::Query { sql })) => {
                            let response = match db.execute(&sql) {
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
                            let matches = match table_filter {
                                Some(tbl) => &event.table == tbl,
                                None => true,
                            };
                            if matches {
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
                Ok(Some(Request::Query { sql })) => {
                    let response = match db.execute(&sql) {
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
