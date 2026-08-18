//! Client session connection handler for ChocoBase.

use std::io;
use tokio::net::TcpStream;

use crate::engine::SharedDatabase;
use crate::server::protocol::{read_request, write_response, Request, Response};

/// Handles an incoming TCP connection session.
/// Automatically releases any active transaction locks when the client disconnects.
pub async fn handle_session(mut socket: TcpStream, db: SharedDatabase) -> io::Result<()> {
    let (mut reader, mut writer) = socket.split();
    let mut buffer = Vec::new();
    let mut subscription: Option<(tokio::sync::broadcast::Receiver<crate::server::protocol::ChangeEvent>, Option<String>)> = None;

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

    Ok(())
}
