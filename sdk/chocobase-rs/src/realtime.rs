#[derive(Debug, Clone)]
pub struct RealtimeChannel {
    pub topic: String,
}

impl RealtimeChannel {
    pub fn on(&self, _event: &str) -> &Self {
        self
    }

    pub fn subscribe(&self) -> &Self {
        self
    }
}

#[derive(Debug, Clone)]
pub struct RealtimeClient {
    pub base_url: String,
    pub api_key: String,
}

impl RealtimeClient {
    pub fn new(base_url: String, api_key: String) -> Self {
        let ws_url = if let Some(stripped) = base_url.strip_prefix("http://") {
            format!("ws://{stripped}")
        } else if let Some(stripped) = base_url.strip_prefix("https://") {
            format!("wss://{stripped}")
        } else {
            base_url
        };

        Self {
            base_url: format!("{ws_url}/v1/realtime"),
            api_key,
        }
    }

    pub fn channel(&self, topic: &str) -> RealtimeChannel {
        RealtimeChannel {
            topic: topic.to_string(),
        }
    }
}
