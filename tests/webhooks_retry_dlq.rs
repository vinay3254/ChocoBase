use dbengine::engine::SharedDatabase;
use dbengine::webhooks::{WebhookConfig, WebhookManager};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::NamedTempFile;

#[tokio::test]
async fn test_webhooks_exponential_retry_and_dead_letter_queue() {
    let tmp = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(tmp.path()).unwrap();

    let webhook_mgr = Arc::new(WebhookManager::new());
    webhook_mgr.clone().start_dispatcher(db.subscribe());

    // 1. Register Webhook pointing to an unreachable port with max_retries = 2
    let bad_webhook = WebhookConfig {
        id: "offline_sink".to_string(),
        table_name: "users".to_string(),
        events: vec!["INSERT".to_string()],
        target_url: "http://127.0.0.1:59999/dead_sink".to_string(),
        headers: HashMap::new(),
        active: true,
        max_retries: 2,
    };
    webhook_mgr.add_webhook(bad_webhook).await;

    // 2. Trigger change event via table mutation
    db.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
        .unwrap();
    db.execute("INSERT INTO users (id, name) VALUES (1, 'alice')")
        .unwrap();

    // 3. Poll DLQ for up to 5 seconds
    let start = std::time::Instant::now();
    let mut dlq_entries = Vec::new();
    while start.elapsed() < std::time::Duration::from_secs(5) {
        dlq_entries = webhook_mgr.list_dead_letter_queue().await;
        if !dlq_entries.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    assert_eq!(
        dlq_entries.len(),
        1,
        "failed webhook must land in dead-letter queue"
    );
    assert_eq!(dlq_entries[0].webhook_id, "offline_sink");
    assert_eq!(dlq_entries[0].attempts, 2);
    assert!(!dlq_entries[0].last_error.is_empty());

    // 4. Clear DLQ
    webhook_mgr.clear_dead_letter_queue().await;
    let empty_dlq = webhook_mgr.list_dead_letter_queue().await;
    assert_eq!(empty_dlq.len(), 0);
}
