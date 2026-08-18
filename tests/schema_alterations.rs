use dbengine::engine::{Database, ExecResult};
use dbengine::types::value::Value;
use tempfile::NamedTempFile;

#[test]
fn test_alter_table_add_column_with_existing_data() {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();

    db.execute("CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT)")
        .unwrap();
    db.execute("INSERT INTO products (id, name) VALUES (1, 'Laptop'), (2, 'Phone')")
        .unwrap();

    // 1. Add column
    db.execute("ALTER TABLE products ADD COLUMN price FLOAT")
        .unwrap();

    // 2. Query existing rows (price should be NULL)
    let res = db
        .execute("SELECT id, name, price FROM products ORDER BY id ASC")
        .unwrap();
    match res {
        ExecResult::Rows { rows, .. } => {
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0][0], Value::Integer(1));
            assert_eq!(rows[0][1], Value::Text("Laptop".into()));
            assert_eq!(rows[0][2], Value::Null);
            assert_eq!(rows[1][0], Value::Integer(2));
            assert_eq!(rows[1][1], Value::Text("Phone".into()));
            assert_eq!(rows[1][2], Value::Null);
        }
        other => panic!("unexpected result: {other:?}"),
    }

    // 3. Insert new row with price
    db.execute("INSERT INTO products (id, name, price) VALUES (3, 'Monitor', 299.99)")
        .unwrap();

    let res = db
        .execute("SELECT price FROM products WHERE id = 3")
        .unwrap();
    match res {
        ExecResult::Rows { rows, .. } => {
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0][0], Value::Float(299.99));
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn test_alter_table_drop_column_and_rename_table() {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();

    db.execute("CREATE TABLE members (id INTEGER PRIMARY KEY, username TEXT, legacy_notes TEXT)")
        .unwrap();
    db.execute(
        "INSERT INTO members (id, username, legacy_notes) VALUES (10, 'alice', 'temporary note')",
    )
    .unwrap();

    // 1. Drop column
    db.execute("ALTER TABLE members DROP COLUMN legacy_notes")
        .unwrap();

    let res = db.execute("SELECT * FROM members").unwrap();
    match res {
        ExecResult::Rows { columns, rows } => {
            assert_eq!(columns, vec!["id", "username"]);
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0][0], Value::Integer(10));
            assert_eq!(rows[0][1], Value::Text("alice".into()));
        }
        other => panic!("unexpected result: {other:?}"),
    }

    // 2. Rename table
    db.execute("ALTER TABLE members RENAME TO accounts")
        .unwrap();

    let res = db
        .execute("SELECT username FROM accounts WHERE id = 10")
        .unwrap();
    match res {
        ExecResult::Rows { rows, .. } => {
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0][0], Value::Text("alice".into()));
        }
        other => panic!("unexpected result: {other:?}"),
    }

    // Old name must not exist
    assert!(db.execute("SELECT * FROM members").is_err());
}
