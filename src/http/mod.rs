//! HTTP REST Gateway for ChocoBase.
//! Exposes JSON endpoints for SQL query execution, schema inspection, health checks, and metrics.

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;

use crate::engine::SharedDatabase;
use crate::error::Result;

pub mod dashboard;

pub struct HttpServer {
    shutdown_tx: broadcast::Sender<()>,
}

impl HttpServer {
    pub async fn bind(addr: SocketAddr, db: SharedDatabase) -> Result<(Self, SocketAddr)> {
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        let (shutdown_tx, _) = broadcast::channel(1);
        let mut shutdown_rx = shutdown_tx.subscribe();

        let db = Arc::new(db);

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    accept_res = listener.accept() => {
                        match accept_res {
                            Ok((socket, _)) => {
                                let db_clone = Arc::clone(&db);
                                tokio::spawn(async move {
                                    let _ = handle_http_connection(socket, db_clone).await;
                                });
                            }
                            Err(_) => break,
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        break;
                    }
                }
            }
        });

        Ok((Self { shutdown_tx }, local_addr))
    }

    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }
}

async fn handle_http_connection(mut socket: TcpStream, db: Arc<SharedDatabase>) -> std::io::Result<()> {
    let mut buf = vec![0u8; 8192];
    let n = socket.read(&mut buf).await?;
    if n == 0 {
        return Ok(());
    }

    let req_str = String::from_utf8_lossy(&buf[..n]);
    let mut lines = req_str.lines();
    let request_line = match lines.next() {
        Some(line) => line,
        None => return Ok(()),
    };

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let path = parts.next().unwrap_or("/");

    if method == "OPTIONS" {
        let resp = "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization\r\nContent-Length: 0\r\n\r\n";
        socket.write_all(resp.as_bytes()).await?;
        return Ok(());
    }

    // Extract body after \r\n\r\n
    let body = if let Some(idx) = req_str.find("\r\n\r\n") {
        &req_str[idx + 4..]
    } else {
        ""
    };

    if method == "GET" && (path == "/" || path == "/dashboard") {
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            dashboard::DASHBOARD_HTML.len()
        );
        socket.write_all(header.as_bytes()).await?;
        socket.write_all(dashboard::DASHBOARD_HTML.as_bytes()).await?;
        socket.flush().await?;
        return Ok(());
    }

    let (status_code, status_text, json_body) = if method == "GET" && path == "/v1/health" {
        (200, "OK", serde_json::json!({ "status": "healthy", "engine": "ChocoBase", "version": "0.1.0" }))
    } else if method == "GET" && path == "/v1/tables" {
        let tables = db.list_tables();
        (200, "OK", serde_json::json!({ "tables": tables }))
    } else if method == "GET" && path.starts_with("/v1/tables/") {
        let table_name = &path["/v1/tables/".len()..];
        match db.table_schema(table_name) {
            Some(schema) => (200, "OK", serde_json::json!({ "table": table_name, "schema": schema })),
            None => (404, "Not Found", serde_json::json!({ "error": format!("table '{}' not found", table_name) })),
        }
    } else if method == "GET" && path == "/v1/metrics" {
        let stats = db.pager_stats();
        (200, "OK", serde_json::json!({ "page_count": stats.page_count, "pages_read": stats.pages_read, "cached_pages": stats.cached_pages }))
    } else if method == "POST" && path == "/v1/sql" {
        let sql = if let Ok(parsed_json) = serde_json::from_str::<serde_json::Value>(body) {
            parsed_json.get("sql").and_then(|s| s.as_str()).map(|s| s.to_string()).unwrap_or_else(|| body.to_string())
        } else {
            body.trim().to_string()
        };

        if sql.is_empty() {
            (400, "Bad Request", serde_json::json!({ "error": "missing sql query in request body" }))
        } else {
            match db.execute(&sql) {
                Ok(result) => (200, "OK", serde_json::json!({ "status": "ok", "result": result })),
                Err(err) => (400, "Bad Request", serde_json::json!({ "status": "error", "error": err.to_string() })),
            }
        }
    } else {
        (404, "Not Found", serde_json::json!({ "error": format!("endpoint '{}' not found", path) }))
    };

    let body_bytes = serde_json::to_vec(&json_body).unwrap_or_default();
    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status_code,
        status_text,
        body_bytes.len()
    );

    socket.write_all(header.as_bytes()).await?;
    socket.write_all(&body_bytes).await?;
    socket.flush().await?;

    Ok(())
}
