//! Realtime Broadcast and Presence Channel Engine for ChocoBase.

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

#[derive(Clone, Default)]
pub struct RealtimeChannelManager {
    // channel_name -> (user_key -> PresenceEntry)
    presence: Arc<RwLock<HashMap<String, HashMap<String, PresenceEntry>>>>,
}

impl RealtimeChannelManager {
    pub fn new() -> Self {
        Self {
            presence: Arc::new(RwLock::new(HashMap::new())),
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

    pub fn untrack_presence(&self, channel: &str, key: &str) -> bool {
        let mut map = self.presence.write().unwrap();
        if let Some(channel_presence) = map.get_mut(channel) {
            channel_presence.remove(key).is_some()
        } else {
            false
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
        let payload: serde_json::Value =
            serde_json::from_str(body).unwrap_or_else(|_| serde_json::json!({ "raw": body }));

        let event_name = payload
            .get("event")
            .and_then(|v| v.as_str())
            .unwrap_or("broadcast");
        let event_payload = payload.get("payload").unwrap_or(&payload);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        (
            200,
            "OK",
            serde_json::json!({
                "status": "published",
                "channel": channel,
                "event": event_name,
                "payload": event_payload,
                "sender": {
                    "user_id": ctx.user_id,
                    "role": ctx.role
                },
                "timestamp": now
            }),
        )
    } else if subpath.starts_with("/presence/") {
        let channel = subpath["/presence/".len()..].trim_matches('/');

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
                let removed = manager.untrack_presence(channel, &key);
                (
                    200,
                    "OK",
                    serde_json::json!({
                        "status": "untracked",
                        "removed": removed
                    }),
                )
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
