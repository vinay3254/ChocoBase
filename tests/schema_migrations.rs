use dbengine::engine::Database;
use dbengine::migration::{Migration, MigrationRunner};
use tempfile::NamedTempFile;

#[test]
fn test_schema_migrations_apply_in_order_and_skip_duplicates() {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();

    let mut runner = MigrationRunner::new(&mut db);

    let m1 = Migration {
        version: 1,
        name: "create_users_table".into(),
        sql: "CREATE TABLE app_users (id INTEGER PRIMARY KEY, email TEXT NOT NULL, is_active BOOLEAN NOT NULL)".into(),
    };

    let m2 = Migration {
        version: 2,
        name: "create_posts_table".into(),
        sql: "CREATE TABLE posts (id INTEGER PRIMARY KEY, author_id INTEGER NOT NULL, title TEXT NOT NULL)".into(),
    };

    let applied = runner
        .apply_all(&[m1.clone(), m2.clone()])
        .expect("migrations should succeed");
    assert_eq!(applied.len(), 2);
    assert_eq!(applied[0].version, 1);
    assert_eq!(applied[1].version, 2);

    // Re-running apply_all skips already applied migrations
    let re_applied = runner
        .apply_all(&[m1.clone(), m2.clone()])
        .expect("re-apply should succeed");
    assert_eq!(re_applied.len(), 0);

    // Query applied migrations from DB
    let history = runner.get_applied_migrations().expect("fetch history");
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].name, "create_users_table");
    assert_eq!(history[1].name, "create_posts_table");

    // Add 3rd migration
    let m3 = Migration {
        version: 3,
        name: "add_post_author_index".into(),
        sql: "CREATE INDEX idx_post_author ON posts (author_id)".into(),
    };

    let applied_m3 = runner.apply_all(&[m1, m2, m3]).expect("migration 3");
    assert_eq!(applied_m3.len(), 1);
    assert_eq!(applied_m3[0].version, 3);
}

#[test]
fn test_schema_migration_rollback_on_failure() {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();

    let mut runner = MigrationRunner::new(&mut db);

    let m1 = Migration {
        version: 1,
        name: "valid_migration".into(),
        sql: "CREATE TABLE accounts (id INTEGER PRIMARY KEY, balance INTEGER NOT NULL)".into(),
    };

    let m2_failing = Migration {
        version: 2,
        name: "failing_migration".into(),
        sql: "CREATE TABLE orders (id INTEGER NOT NULL); BAD SYNTAX ERROR".into(),
    };

    runner.apply_all(&[m1]).expect("migration 1 should succeed");

    let fail_res = runner.apply_all(&[m2_failing]);
    assert!(fail_res.is_err());

    // Failed migration 2 is rolled back and not recorded in history
    let history = runner.get_applied_migrations().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].version, 1);

    // Table created by failing migration was rolled back
    assert!(db.table_schema("orders").is_none());
}

#[test]
fn test_schema_migration_handles_semicolons_in_string_literals() {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();
    let mut runner = MigrationRunner::new(&mut db);

    let m1 = Migration {
        version: 1,
        name: "migration_with_semicolons_in_strings".into(),
        sql: "CREATE TABLE messages (id INTEGER PRIMARY KEY, content TEXT NOT NULL); INSERT INTO messages (id, content) VALUES (1, 'hello; world; test'); INSERT INTO messages (id, content) VALUES (2, 'second; message')".into(),
    };

    let applied = runner
        .apply_all(&[m1])
        .expect("migration with semicolons in strings should succeed");
    assert_eq!(applied.len(), 1);

    let res = db
        .execute("SELECT content FROM messages WHERE id = 1")
        .unwrap();
    if let dbengine::engine::ExecResult::Rows { rows, .. } = res {
        assert_eq!(
            rows[0][0],
            dbengine::types::value::Value::Text("hello; world; test".into())
        );
    } else {
        panic!("expected rows");
    }
}
