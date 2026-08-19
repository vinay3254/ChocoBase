//! Automated integration tests for High Availability, Streaming Replication,
//! Read Replica Query Routing, and Automated Standby Promotion.

use dbengine::auth::ExecutionContext;
use dbengine::engine::{ExecResult, SharedDatabase};
use dbengine::replica::ReplicaManager;
use tempfile::tempdir;

#[tokio::test]
async fn test_replica_provisioning_and_failover() {
    let dir = tempdir().unwrap();
    let primary_path = dir.path().join("primary.db");
    let primary = SharedDatabase::create(&primary_path).unwrap();

    let admin = ExecutionContext::admin();
    primary
        .execute_with_context(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, email TEXT NOT NULL);",
            &admin,
        )
        .unwrap();

    primary
        .execute_with_context(
            "INSERT INTO users (id, name, email) VALUES (1, 'Vinay', 'vinay@example.com');",
            &admin,
        )
        .unwrap();

    let replica_mgr = ReplicaManager::new(dir.path().join("replicas"), primary.clone());

    // 1. Provision Standby Replica
    let meta = replica_mgr.create_replica("standby_node_1").await.unwrap();
    assert_eq!(meta.id, "standby_node_1");
    assert_eq!(meta.status, "online");

    // 2. Continuous Replication Sync: Write to primary
    primary
        .execute_with_context(
            "INSERT INTO users (id, name, email) VALUES (2, 'Alice', 'alice@example.com');",
            &admin,
        )
        .unwrap();

    // Give asynchronous replication task a moment to apply changefeed event
    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;

    // 3. Test routed execution
    let (res_read, target) = replica_mgr
        .execute_routed("SELECT * FROM users ORDER BY id ASC;", &admin)
        .await
        .unwrap();
    assert_eq!(target, "replica");
    if let ExecResult::Rows { rows, .. } = res_read {
        assert_eq!(rows.len(), 2);
    } else {
        panic!("expected rows from routed select");
    }

    // 4. Standby Promotion / Automated Failover
    // Simulate primary termination and promote standby_node_1 to become the new primary
    let promoted_primary = replica_mgr
        .promote_to_primary("standby_node_1")
        .await
        .unwrap();

    // Verify writes succeed directly on promoted standby
    promoted_primary
        .execute_with_context(
            "INSERT INTO users (id, name, email) VALUES (3, 'Bob', 'bob@example.com');",
            &admin,
        )
        .unwrap();

    let query_res = promoted_primary
        .execute_with_context("SELECT * FROM users WHERE id = 3;", &admin)
        .unwrap();
    if let ExecResult::Rows { rows, .. } = query_res {
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][1], dbengine::Value::Text("Bob".into()));
    } else {
        panic!("expected row 3 on promoted standby");
    }
}
