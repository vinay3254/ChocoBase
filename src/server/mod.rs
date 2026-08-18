//! Network server and protocol module for ChocoBase.

pub mod protocol;
pub mod session;
pub mod listener;
pub mod postgres_wire;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use tokio::sync::broadcast;

use crate::engine::SharedDatabase;
use crate::error::Result;

/// Configuration options for starting a ChocoBase server.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind_addr: SocketAddr,
    pub db_path: PathBuf,
}

impl ServerConfig {
    pub fn new(bind_addr: SocketAddr, db_path: impl AsRef<Path>) -> Self {
        Self {
            bind_addr,
            db_path: db_path.as_ref().to_path_buf(),
        }
    }
}

/// ChocoBase Network Server handle.
pub struct Server {
    config: ServerConfig,
    db: SharedDatabase,
    shutdown_tx: broadcast::Sender<()>,
}

impl Server {
    pub fn new(config: ServerConfig) -> Result<Self> {
        let is_existing_db = config.db_path.exists()
            && std::fs::metadata(&config.db_path)
                .map(|m| m.len() > 0)
                .unwrap_or(false);

        let db = if is_existing_db {
            SharedDatabase::open(&config.db_path)?
        } else {
            SharedDatabase::create(&config.db_path)?
        };
        let (shutdown_tx, _) = broadcast::channel(1);
        Ok(Self {
            config,
            db,
            shutdown_tx,
        })
    }

    /// Returns a cloned SharedDatabase handle sharing the same storage and lock manager.
    pub fn db(&self) -> SharedDatabase {
        self.db.clone()
    }

    /// Binds the listener immediately, spawns the server task in the background,
    /// and returns the running server handle and bound local socket address.
    pub async fn bind(config: ServerConfig) -> Result<(Self, SocketAddr)> {
        let is_existing_db = config.db_path.exists()
            && std::fs::metadata(&config.db_path)
                .map(|m| m.len() > 0)
                .unwrap_or(false);

        let db = if is_existing_db {
            SharedDatabase::open(&config.db_path)?
        } else {
            SharedDatabase::create(&config.db_path)?
        };
        let listener = tokio::net::TcpListener::bind(config.bind_addr)
            .await
            .map_err(crate::error::StorageError::Io)?;
        let local_addr = listener.local_addr().map_err(crate::error::StorageError::Io)?;
        let (shutdown_tx, _) = broadcast::channel(1);
        let shutdown_rx = shutdown_tx.subscribe();
        let session_db = db.clone();

        tokio::spawn(async move {
            let _ = listener::run_server_with_listener(listener, session_db, shutdown_rx).await;
        });

        Ok((
            Self {
                config,
                db,
                shutdown_tx,
            },
            local_addr,
        ))
    }

    /// Runs the server loop in the current task until shutdown.
    pub async fn run(&self) -> std::io::Result<()> {
        let shutdown_rx = self.shutdown_tx.subscribe();
        listener::run_server(self.config.bind_addr, self.db.clone(), shutdown_rx).await
    }

    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }
}
