//! Realtime Broadcast and Presence Channel Engine for ChocoBase.
//! Provides topic-based live message distribution across async subscribers,
//! and secure, authenticated multi-user room presence state tracking.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::auth::ExecutionContext;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PresenceEntry {
    pub key: String,
    pub user_id: Option<i64>,
    pub role: String,
    pub state: serde_json::Value,
    pub updated_at: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BroadcastMessage {
    pub channel: String,
    pub event: String,
    pub payload: serde_json::Value,
    pub sender_id: Option<i64>,
    pub sender_role: String,
    pub timestamp: u64,
}

#[derive(Clone, Default)]
pub struct RealtimeChannelManager {
    // channel_name -> (user_key -> PresenceEntry)
    presence: Arc<RwLock<HashMap<String, HashMap<String, PresenceEntry>>>>,
    // channel_name -> broadcast sender
    broadcast_channels:
        Arc<RwLock<HashMap<String, tokio::sync::broadcast::Sender<BroadcastMessage>>>>,
}

impl RealtimeChannelManager {
    pub fn new() -> Self {
        Self {
            presence: Arc::new(RwLock::new(HashMap::new())),
            broadcast_channels: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Subscribe to live broadcast messages on a specific channel topic.
    pub fn subscribe(&self, channel: &str) -> tokio::sync::broadcast::Receiver<BroadcastMessage> {
        let mut map = self.broadcast_channels.write().unwrap();
        let tx = map.entry(channel.to_string()).or_insert_with(|| {
            let (tx, _) = tokio::sync::broadcast::channel(512);
            tx
        });
        tx.subscribe()
    }

    /// Publishes a message to all active subscribers on a channel. Returns number of active receivers.
    pub fn publish(&self, msg: BroadcastMessage) -> usize {
        let map = self.broadcast_channels.read().unwrap();
        if let Some(tx) = map.get(&msg.channel) {
            tx.send(msg).unwrap_or(0)
        } else {
            0
        }
    }

    pub fn track_presence(
        &self,
        channel: &str,
        key: &str,
        state: serde_json::Value,
        ctx: &ExecutionContext,
    ) -> PresenceEntry {
        let mut map = self.presence.write().unwrap();
        let channel_presence = map.entry(channel.to_string()).or_default();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let entry = PresenceEntry {
            key: key.to_string(),
            user_id: ctx.user_id,
            role: ctx.role.clone().unwrap_or_else(|| "anon".into()),
            state,
            updated_at: now,
        };

        channel_presence.insert(key.to_string(), entry.clone());
        entry
    }

    pub fn get_presence(&self, channel: &str) -> Vec<PresenceEntry> {
        let map = self.presence.read().unwrap();
        if let Some(channel_presence) = map.get(channel) {
            channel_presence.values().cloned().collect()
        } else {
            Vec::new()
        }
    }

    pub fn untrack_presence(
        &self,
        channel: &str,
        key: &str,
        ctx: &ExecutionContext,
    ) -> Result<bool, &'static str> {
        let mut map = self.presence.write().unwrap();
        if let Some(channel_presence) = map.get_mut(channel) {
            if let Some(entry) = channel_presence.get(key) {
                // Enforce presence key ownership
                if !ctx.is_admin {
                    if let (Some(owner_id), Some(caller_id)) = (entry.user_id, ctx.user_id) {
                        if owner_id != caller_id {
                            return Err("cannot untrack presence key owned by another user");
                        }
                    } else if entry.user_id.is_some() && ctx.user_id.is_none() {
                        return Err(
                            "authentication required to untrack authenticated presence key",
                        );
                    }
                }
                Ok(channel_presence.remove(key).is_some())
            } else {
                Ok(false)
            }
        } else {
            Ok(false)
        }
    }
}

pub async fn handle_realtime_channel_request(
    manager: &RealtimeChannelManager,
    method: &str,
    path: &str,
    body: &str,
    ctx: &ExecutionContext,
) -> (u16, &'static str, serde_json::Value) {
    let subpath = path.strip_prefix("/v1/realtime/v1").unwrap_or(path);

    if subpath.starts_with("/broadcast/") && method == "POST" {
        let channel = subpath["/broadcast/".len()..].trim_matches('/');
        if channel.starts_with("private:") && !ctx.is_authenticated() && !ctx.is_admin {
            return (
                401,
                "Unauthorized",
                serde_json::json!({ "error": "authentication required for private broadcast channel" }),
            );
        }

        let payload: serde_json::Value =
            serde_json::from_str(body).unwrap_or_else(|_| serde_json::json!({ "raw": body }));

        let event_name = payload
            .get("event")
            .and_then(|v| v.as_str())
            .unwrap_or("broadcast");
        let event_payload = payload.get("payload").unwrap_or(&payload).clone();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let bcast_msg = BroadcastMessage {
            channel: channel.to_string(),
            event: event_name.to_string(),
            payload: event_payload.clone(),
            sender_id: ctx.user_id,
            sender_role: ctx.role.clone().unwrap_or_else(|| "anon".into()),
            timestamp: now,
        };

        let delivered_count = manager.publish(bcast_msg);

        (
            200,
            "OK",
            serde_json::json!({
                "status": "published",
                "channel": channel,
                "event": event_name,
                "payload": event_payload,
                "delivered_to": delivered_count,
                "sender": {
                    "user_id": ctx.user_id,
                    "role": ctx.role
                },
                "timestamp": now
            }),
        )
    } else if let Some(stripped) = subpath.strip_prefix("/presence/") {
        let channel = stripped.trim_matches('/');

        match method {
            "GET" => {
                let state_list = manager.get_presence(channel);
                (200, "OK", serde_json::json!(state_list))
            }
            "POST" => {
                let payload: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
                let key = payload
                    .get("key")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        ctx.user_id
                            .map(|id| format!("user_{id}"))
                            .unwrap_or_else(|| "anon".into())
                    });
                let state = payload
                    .get("state")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));

                let entry = manager.track_presence(channel, &key, state, ctx);
                (
                    200,
                    "OK",
                    serde_json::json!({
                        "status": "tracked",
                        "presence": entry
                    }),
                )
            }
            "DELETE" => {
                let payload: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
                let key = payload
                    .get("key")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        ctx.user_id
                            .map(|id| format!("user_{id}"))
                            .unwrap_or_else(|| "anon".into())
                    });

                match manager.untrack_presence(channel, &key, ctx) {
                    Ok(removed) => (
                        200,
                        "OK",
                        serde_json::json!({
                            "status": "untracked",
                            "removed": removed
                        }),
                    ),
                    Err(err) => (403, "Forbidden", serde_json::json!({ "error": err })),
                }
            }
            _ => (
                405,
                "Method Not Allowed",
                serde_json::json!({ "error": "method not allowed" }),
            ),
        }
    } else {
        (
            404,
            "Not Found",
            serde_json::json!({ "error": "realtime channel route not found" }),
        )
    }
}
