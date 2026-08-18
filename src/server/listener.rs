//! Asynchronous TCP connection listener and server loop for ChocoBase.

use std::io;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio::sync::broadcast;

use crate::engine::SharedDatabase;
use crate::server::session::handle_session;

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
                            let _ = handle_session(socket, session_db).await;
                        });
                    }
                    Err(e) => {
                        eprintln!("error accepting client connection: {e}");
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                // Graceful shutdown triggered
                break;
            }
        }
    }

    Ok(())
}
