//! Prometheus Metrics and Production Observability for ChocoBase.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

#[derive(Default)]
pub struct MetricsRegistry {
    start_time: OnceLock<Instant>,
    http_requests_total: AtomicU64,
    http_requests_2xx: AtomicU64,
    http_requests_4xx: AtomicU64,
    http_requests_5xx: AtomicU64,
    sql_queries_total: AtomicU64,
    sql_queries_success: AtomicU64,
    sql_queries_failed: AtomicU64,
    auth_attempts_total: AtomicU64,
    auth_failures_total: AtomicU64,
    realtime_events_broadcast_total: AtomicU64,
}

impl MetricsRegistry {
    pub fn global() -> &'static Self {
        static REGISTRY: OnceLock<MetricsRegistry> = OnceLock::new();
        REGISTRY.get_or_init(|| {
            let reg = MetricsRegistry::default();
            let _ = reg.start_time.set(Instant::now());
            reg
        })
    }

    pub fn record_http_request(&self, status: u16) {
        self.http_requests_total.fetch_add(1, Ordering::Relaxed);
        match status {
            200..=299 => self.http_requests_2xx.fetch_add(1, Ordering::Relaxed),
            400..=499 => self.http_requests_4xx.fetch_add(1, Ordering::Relaxed),
            500..=599 => self.http_requests_5xx.fetch_add(1, Ordering::Relaxed),
            _ => 0,
        };
    }

    pub fn record_sql_query(&self, success: bool) {
        self.sql_queries_total.fetch_add(1, Ordering::Relaxed);
        if success {
            self.sql_queries_success.fetch_add(1, Ordering::Relaxed);
        } else {
            self.sql_queries_failed.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn record_auth_attempt(&self, success: bool) {
        self.auth_attempts_total.fetch_add(1, Ordering::Relaxed);
        if !success {
            self.auth_failures_total.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn record_realtime_broadcast(&self) {
        self.realtime_events_broadcast_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn render_prometheus(&self) -> String {
        let uptime = self
            .start_time
            .get()
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0);

        let mut out = String::new();
        out.push_str("# HELP chocobase_uptime_seconds Database engine uptime in seconds.\n");
        out.push_str("# TYPE chocobase_uptime_seconds gauge\n");
        out.push_str(&format!("chocobase_uptime_seconds {uptime}\n\n"));

        out.push_str(
            "# HELP chocobase_http_requests_total Total number of HTTP requests processed.\n",
        );
        out.push_str("# TYPE chocobase_http_requests_total counter\n");
        out.push_str(&format!(
            "chocobase_http_requests_total {}\n",
            self.http_requests_total.load(Ordering::Relaxed)
        ));
        out.push_str(&format!(
            "chocobase_http_requests_total{{status=\"2xx\"}} {}\n",
            self.http_requests_2xx.load(Ordering::Relaxed)
        ));
        out.push_str(&format!(
            "chocobase_http_requests_total{{status=\"4xx\"}} {}\n",
            self.http_requests_4xx.load(Ordering::Relaxed)
        ));
        out.push_str(&format!(
            "chocobase_http_requests_total{{status=\"5xx\"}} {}\n\n",
            self.http_requests_5xx.load(Ordering::Relaxed)
        ));

        out.push_str("# HELP chocobase_sql_queries_total Total number of executed SQL queries.\n");
        out.push_str("# TYPE chocobase_sql_queries_total counter\n");
        out.push_str(&format!(
            "chocobase_sql_queries_total {}\n",
            self.sql_queries_total.load(Ordering::Relaxed)
        ));
        out.push_str(&format!(
            "chocobase_sql_queries_total{{status=\"success\"}} {}\n",
            self.sql_queries_success.load(Ordering::Relaxed)
        ));
        out.push_str(&format!(
            "chocobase_sql_queries_total{{status=\"failed\"}} {}\n\n",
            self.sql_queries_failed.load(Ordering::Relaxed)
        ));

        out.push_str("# HELP chocobase_auth_attempts_total Total authentication attempts.\n");
        out.push_str("# TYPE chocobase_auth_attempts_total counter\n");
        out.push_str(&format!(
            "chocobase_auth_attempts_total {}\n",
            self.auth_attempts_total.load(Ordering::Relaxed)
        ));
        out.push_str(&format!(
            "chocobase_auth_failures_total {}\n\n",
            self.auth_failures_total.load(Ordering::Relaxed)
        ));

        out.push_str(
            "# HELP chocobase_realtime_events_broadcast_total Total realtime events dispatched.\n",
        );
        out.push_str("# TYPE chocobase_realtime_events_broadcast_total counter\n");
        out.push_str(&format!(
            "chocobase_realtime_events_broadcast_total {}\n",
            self.realtime_events_broadcast_total.load(Ordering::Relaxed)
        ));

        out
    }
}
