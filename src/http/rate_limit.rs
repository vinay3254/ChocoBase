//! In-memory Sliding Window Rate Limiter for HTTP API Gateways and Authentication.
//! Provides per-IP and per-token request throttling with automatic cleanup and Retry-After calculation.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    pub max_requests: usize,
    pub window_secs: u64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_requests: 100,
            window_secs: 60,
        }
    }
}

#[derive(Clone, Default)]
pub struct RateLimiter {
    // Map of key (e.g. "auth:127.0.0.1" or "api:user_10") -> list of request timestamps
    records: Arc<Mutex<HashMap<String, Vec<u64>>>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            records: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Checks if a request for the given key is allowed under the rate limit.
    /// If allowed, records the timestamp and returns Ok(()).
    /// If exceeded, returns Err(retry_after_secs).
    pub fn check_rate_limit(
        &self,
        key: &str,
        max_requests: usize,
        window_secs: u64,
    ) -> Result<(), u64> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let cutoff = now.saturating_sub(window_secs);

        let mut map = self.records.lock().unwrap();
        let timestamps = map.entry(key.to_string()).or_default();

        // Evict expired entries
        timestamps.retain(|&t| t > cutoff);

        if timestamps.len() >= max_requests {
            let oldest = timestamps.first().copied().unwrap_or(now);
            let retry_after = (oldest + window_secs).saturating_sub(now).max(1);
            Err(retry_after)
        } else {
            timestamps.push(now);
            Ok(())
        }
    }

    pub fn list_active_keys(&self) -> Vec<(String, usize)> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let cutoff = now.saturating_sub(3600);

        let map = self.records.lock().unwrap();
        map.iter()
            .filter_map(|(k, v)| {
                let valid_count = v.iter().filter(|&&t| t > cutoff).count();
                if valid_count > 0 {
                    Some((k.clone(), valid_count))
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn clear(&self) {
        let mut map = self.records.lock().unwrap();
        map.clear();
    }
}
