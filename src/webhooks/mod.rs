//! Database Webhooks Manager and HTTP Event Dispatcher.

use crate::server::protocol::{ChangeAction, ChangeEvent};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::broadcast;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub id: String,
    pub table_name: String,
    pub events: Vec<String>, // e.g. ["INSERT", "UPDATE", "DELETE"] or ["*"]
    pub target_url: String,
    pub headers: HashMap<String, String>,
    pub active: bool,
    #[serde(default = "default_max_retries")]
    pub max_retries: usize,
}

fn default_max_retries() -> usize {
    3
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeadLetterEntry {
    pub id: String,
    pub webhook_id: String,
    pub target_url: String,
    pub payload: serde_json::Value,
    pub last_error: String,
    pub attempts: usize,
    pub failed_at: u64,
}

pub struct WebhookManager {
    configs: tokio::sync::RwLock<Vec<WebhookConfig>>,
    dlq: tokio::sync::RwLock<Vec<DeadLetterEntry>>,
}

impl Default for WebhookManager {
    fn default() -> Self {
        Self::new()
    }
}

impl WebhookManager {
    pub fn new() -> Self {
        Self {
            configs: tokio::sync::RwLock::new(Vec::new()),
            dlq: tokio::sync::RwLock::new(Vec::new()),
        }
    }

    pub async fn add_webhook(&self, config: WebhookConfig) {
        let mut cfgs = self.configs.write().await;
        cfgs.retain(|c| c.id != config.id);
        cfgs.push(config);
    }

    pub async fn remove_webhook(&self, id: &str) -> bool {
        let mut cfgs = self.configs.write().await;
        let prev_len = cfgs.len();
        cfgs.retain(|c| c.id != id);
        cfgs.len() < prev_len
    }

    pub async fn list_webhooks(&self) -> Vec<WebhookConfig> {
        self.configs.read().await.clone()
    }

    pub async fn list_dead_letter_queue(&self) -> Vec<DeadLetterEntry> {
        self.dlq.read().await.clone()
    }

    pub async fn clear_dead_letter_queue(&self) {
        self.dlq.write().await.clear();
    }

    pub fn start_dispatcher(self: Arc<Self>, mut change_rx: broadcast::Receiver<ChangeEvent>) {
        tokio::spawn(async move {
            while let Ok(event) = change_rx.recv().await {
                let configs = self.configs.read().await.clone();
                let action_str = match event.action {
                    ChangeAction::Insert => "INSERT",
                    ChangeAction::Update => "UPDATE",
                    ChangeAction::Delete => "DELETE",
                };

                for webhook in configs {
                    if !webhook.active {
                        continue;
                    }
                    if webhook.table_name != "*" && webhook.table_name != event.table {
                        continue;
                    }
                    if !webhook
                        .events
                        .iter()
                        .any(|e| e == "*" || e.eq_ignore_ascii_case(action_str))
                    {
                        continue;
                    }

                    let payload = serde_json::json!({
                        "type": action_str,
                        "table": event.table,
                        "timestamp": event.timestamp_ms,
                        "record": event.new_row,
                        "old_record": event.old_row,
                    });

                    let target_url = webhook.target_url.clone();
                    let headers = webhook.headers.clone();
                    let webhook_id = webhook.id.clone();
                    let max_retries = webhook.max_retries.max(1);
                    let mgr = Arc::clone(&self);

                    tokio::spawn(async move {
                        let payload_str = payload.to_string();
                        let result =
                            dispatch_with_retry(&target_url, &payload_str, &headers, max_retries)
                                .await;
                        if let Err(e) = result {
                            let failed_at = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();
                            let dlq_entry = DeadLetterEntry {
                                id: format!("dlq_{}_{failed_at}", webhook_id),
                                webhook_id,
                                target_url,
                                payload,
                                last_error: e.to_string(),
                                attempts: max_retries,
                                failed_at,
                            };
                            mgr.dlq.write().await.push(dlq_entry);
                        }
                    });
                }
            }
        });
    }
}

pub async fn dispatch_http_post(
    url_str: &str,
    body: &str,
    headers: &HashMap<String, String>,
) -> std::io::Result<()> {
    // Parse simple http://host:port/path URLs
    let trimmed = url_str.strip_prefix("http://").unwrap_or(url_str);
    let (host_port, path) = match trimmed.find('/') {
        Some(idx) => (&trimmed[..idx], &trimmed[idx..]),
        None => (trimmed, "/"),
    };

    let (host, port) = match host_port.split_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().unwrap_or(80)),
        None => (host_port, 80),
    };

    let host_header = host_port;
    let mut socket = TcpStream::connect((host, port)).await?;

    let mut custom_headers = String::new();
    for (k, v) in headers {
        custom_headers.push_str(&format!("{k}: {v}\r\n"));
    }

    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host_header}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nUser-Agent: ChocoBase-Webhook/1.0\r\nConnection: close\r\n{custom_headers}\r\n{body}",
        body.len()
    );

    socket.write_all(request.as_bytes()).await?;
    socket.flush().await?;

    // Read response with small buffer then close
    let mut buf = [0u8; 1024];
    let _ = socket.read(&mut buf).await;

    Ok(())
}

pub async fn dispatch_with_retry(
    url_str: &str,
    body: &str,
    headers: &HashMap<String, String>,
    max_retries: usize,
) -> std::io::Result<()> {
    let mut attempt = 0;
    let mut delay_ms = 25;

    loop {
        attempt += 1;
        match dispatch_http_post(url_str, body, headers).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                if attempt >= max_retries {
                    return Err(e);
                }
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                delay_ms = (delay_ms * 2).min(1000);
            }
        }
    }
}
