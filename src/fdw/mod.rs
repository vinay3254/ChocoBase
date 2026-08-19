//! PostgreSQL Foreign Data Wrapper (FDW) and Federated Virtual Table Engine for ChocoBase.
//! Enables querying external HTTP JSON endpoints, remote databases, and virtual tables seamlessly via SQL.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::error::{DbError, Result};
use crate::sql::ast::ColumnDef;
use crate::types::value::{ColumnType, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FdwWrapperType {
    HttpJson,
    Csv,
    Mock,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForeignServer {
    pub name: String,
    pub wrapper_type: FdwWrapperType,
    pub options: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForeignTable {
    pub name: String,
    pub server_name: String,
    pub columns: Vec<ColumnDef>,
    pub options: HashMap<String, String>,
}

pub struct FdwManager {
    servers: Arc<RwLock<HashMap<String, ForeignServer>>>,
    tables: Arc<RwLock<HashMap<String, ForeignTable>>>,
}

impl Default for FdwManager {
    fn default() -> Self {
        Self::new()
    }
}

impl FdwManager {
    pub fn new() -> Self {
        Self {
            servers: Arc::new(RwLock::new(HashMap::new())),
            tables: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn register_server(&self, server: ForeignServer) -> Result<()> {
        let mut servers = self.servers.write().await;
        servers.insert(server.name.clone(), server);
        Ok(())
    }

    pub async fn get_server(&self, name: &str) -> Option<ForeignServer> {
        let servers = self.servers.read().await;
        servers.get(name).cloned()
    }

    pub async fn list_servers(&self) -> Vec<ForeignServer> {
        let servers = self.servers.read().await;
        servers.values().cloned().collect()
    }

    pub async fn create_foreign_table(&self, table: ForeignTable) -> Result<()> {
        let servers = self.servers.read().await;
        if !servers.contains_key(&table.server_name) {
            return Err(DbError::Plan(crate::error::PlanError::NoSuchTable(
                format!("foreign server '{}' does not exist", table.server_name),
            )));
        }
        drop(servers);

        let mut tables = self.tables.write().await;
        tables.insert(table.name.clone(), table);
        Ok(())
    }

    pub async fn get_foreign_table(&self, name: &str) -> Option<ForeignTable> {
        let tables = self.tables.read().await;
        tables.get(name).cloned()
    }

    pub async fn list_foreign_tables(&self) -> Vec<ForeignTable> {
        let tables = self.tables.read().await;
        tables.values().cloned().collect()
    }

    pub async fn drop_foreign_table(&self, name: &str) -> bool {
        let mut tables = self.tables.write().await;
        tables.remove(name).is_some()
    }

    /// Scans the foreign data source and returns rows conforming to the table schema.
    pub async fn scan_table(&self, table_name: &str) -> Result<Vec<Vec<Value>>> {
        let table = match self.get_foreign_table(table_name).await {
            Some(t) => t,
            None => {
                return Err(DbError::Plan(crate::error::PlanError::NoSuchTable(
                    format!("foreign table '{table_name}' not found"),
                )))
            }
        };

        let server = match self.get_server(&table.server_name).await {
            Some(s) => s,
            None => {
                return Err(DbError::Plan(crate::error::PlanError::NoSuchTable(
                    format!("foreign server '{}' not found", table.server_name),
                )))
            }
        };

        match server.wrapper_type {
            FdwWrapperType::HttpJson => {
                let base_url = server
                    .options
                    .get("url")
                    .or(server.options.get("base_url"))
                    .cloned()
                    .unwrap_or_default();
                let endpoint = table
                    .options
                    .get("endpoint")
                    .or(table.options.get("path"))
                    .cloned()
                    .unwrap_or_default();
                let full_url =
                    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
                        endpoint
                    } else {
                        format!("{base_url}{endpoint}")
                    };

                let json_data = fetch_http_json(&full_url).await?;
                convert_json_to_rows(&json_data, &table.columns)
            }
            FdwWrapperType::Csv => {
                let file_path = table
                    .options
                    .get("filename")
                    .or(table.options.get("path"))
                    .cloned()
                    .unwrap_or_default();
                let content = std::fs::read_to_string(&file_path)
                    .map_err(|e| DbError::Storage(crate::error::StorageError::Io(e)))?;
                convert_csv_to_rows(&content, &table.columns)
            }
            FdwWrapperType::Mock => {
                let mock_json = table
                    .options
                    .get("data")
                    .cloned()
                    .unwrap_or_else(|| "[]".to_string());
                let parsed: serde_json::Value =
                    serde_json::from_str(&mock_json).unwrap_or(serde_json::Value::Array(vec![]));
                convert_json_to_rows(&parsed, &table.columns)
            }
        }
    }
}

async fn fetch_http_json(url_str: &str) -> Result<serde_json::Value> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let trimmed = url_str.strip_prefix("http://").unwrap_or(url_str);
    let (host_port, path) = match trimmed.find('/') {
        Some(idx) => (&trimmed[..idx], &trimmed[idx..]),
        None => (trimmed, "/"),
    };

    let (host, port) = match host_port.split_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().unwrap_or(80)),
        None => (host_port, 80),
    };

    let mut socket = TcpStream::connect((host, port))
        .await
        .map_err(|e| DbError::Storage(crate::error::StorageError::Io(e)))?;

    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {host_port}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
    );
    socket
        .write_all(req.as_bytes())
        .await
        .map_err(|e| DbError::Storage(crate::error::StorageError::Io(e)))?;
    socket
        .flush()
        .await
        .map_err(|e| DbError::Storage(crate::error::StorageError::Io(e)))?;

    let mut res_bytes = Vec::new();
    socket
        .read_to_end(&mut res_bytes)
        .await
        .map_err(|e| DbError::Storage(crate::error::StorageError::Io(e)))?;

    let s = String::from_utf8_lossy(&res_bytes);
    let body_start = s.find("\r\n\r\n").map(|i| i + 4).unwrap_or(0);
    let body = &s[body_start..];

    serde_json::from_str(body).map_err(|e| {
        DbError::Exec(crate::error::ExecError::InvalidValue(format!(
            "invalid JSON response from FDW: {e}"
        )))
    })
}

fn convert_json_to_rows(
    json: &serde_json::Value,
    columns: &[ColumnDef],
) -> Result<Vec<Vec<Value>>> {
    let items = match json {
        serde_json::Value::Array(arr) => arr.as_slice(),
        obj @ serde_json::Value::Object(_) => std::slice::from_ref(obj),
        _ => return Ok(vec![]),
    };

    let mut rows = Vec::with_capacity(items.len());
    for item in items {
        if let Some(obj) = item.as_object() {
            let mut row = Vec::with_capacity(columns.len());
            for col in columns {
                let val = match obj.get(&col.name) {
                    Some(serde_json::Value::Null) | None => Value::Null,
                    Some(serde_json::Value::Bool(b)) => Value::Boolean(*b),
                    Some(serde_json::Value::Number(n)) => {
                        if let Some(i) = n.as_i64() {
                            Value::Integer(i)
                        } else if let Some(f) = n.as_f64() {
                            Value::Float(f)
                        } else {
                            Value::Null
                        }
                    }
                    Some(serde_json::Value::String(s)) => Value::Text(s.clone()),
                    Some(other) => Value::Json(other.to_string()),
                };
                row.push(val);
            }
            rows.push(row);
        }
    }
    Ok(rows)
}

fn convert_csv_to_rows(csv_content: &str, columns: &[ColumnDef]) -> Result<Vec<Vec<Value>>> {
    let mut lines = csv_content.lines();
    let header_line = match lines.next() {
        Some(h) => h,
        None => return Ok(vec![]),
    };

    let headers: Vec<&str> = header_line.split(',').map(|s| s.trim()).collect();
    let mut header_indices = HashMap::new();
    for (i, h) in headers.iter().enumerate() {
        header_indices.insert(*h, i);
    }

    let mut rows = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        let mut row = Vec::with_capacity(columns.len());
        for col in columns {
            let val = match header_indices.get(col.name.as_str()) {
                Some(&idx) if idx < fields.len() => {
                    let field = fields[idx];
                    match col.ty {
                        ColumnType::Integer => Value::Integer(field.parse().unwrap_or(0)),
                        ColumnType::Float => Value::Float(field.parse().unwrap_or(0.0)),
                        ColumnType::Boolean => {
                            Value::Boolean(field.eq_ignore_ascii_case("true") || field == "1")
                        }
                        _ => Value::Text(field.to_string()),
                    }
                }
                _ => Value::Null,
            };
            row.push(val);
        }
        rows.push(row);
    }
    Ok(rows)
}
