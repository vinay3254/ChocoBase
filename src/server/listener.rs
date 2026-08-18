//! Asynchronous TCP connection listener and server loop for ChocoBase.

use std::io;
use std::net::SocketAddr;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::sync::broadcast;

use crate::engine::SharedDatabase;
use crate::server::session::handle_session_with_prefix;

/// Binds to `addr` and serves client connections until a shutdown signal is received.
pub async fn run_server(
    addr: SocketAddr,
    db: SharedDatabase,
    shutdown_rx: broadcast::Receiver<()>,
) -> io::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    run_server_with_listener(listener, db, shutdown_rx).await
}

/// Serves client connections from an already-bound `listener` until shutdown.
///
/// Protocol detection: reads the first byte with a 5-second timeout. A leading `{` indicates the
/// ChocoBase JSON protocol; anything else is treated as PostgreSQL wire v3. The consumed first byte
/// is passed back to the session handler so it can prepend it to its read buffer.
pub async fn run_server_with_listener(
    listener: TcpListener,
    db: SharedDatabase,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> io::Result<()> {
    loop {
        tokio::select! {
            accept_res = listener.accept() => {
                match accept_res {
                    Ok((socket, _peer_addr)) => {
                        let session_db = db.clone();
                        tokio::spawn(async move {
                            // Read the first byte to dispatch the protocol. Using read_exact
                            // instead of peek avoids a Windows/tokio bug where peek() blocks
                            // indefinitely when client and server share the same I/O thread.
                            let mut socket = socket;
                            let mut first = [0u8; 1];
                            let read_result = tokio::time::timeout(
                                std::time::Duration::from_secs(5),
                                socket.read_exact(&mut first),
                            ).await;

                            match read_result {
                                Ok(Ok(_)) => {
                                    // PostgreSQL wire startup messages begin with 0x00
                                    // (first byte of a 4-byte big-endian message length).
                                    // JSON messages (ChocoBase protocol) always start with
                                    // a printable ASCII character (" { [ t f n 0-9).
                                    if first[0] == 0x00 {
                                        let _ = crate::server::postgres_wire::handle_postgres_session_with_prefix(socket, session_db, first[0]).await;
                                    } else {
                                        let _ = handle_session_with_prefix(socket, session_db, first[0]).await;
                                    }
                                }
                                _ => { /* timeout or read error — drop connection silently */ }
                            }
                        });
                    }
                    Err(e) => {
                        eprintln!("error accepting client connection: {e}");
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                break;
            }
        }
    }

    Ok(())
}
