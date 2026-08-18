use dbengine::engine::{Database, ExecResult};
use dbengine::types::value::Value;
use tempfile::NamedTempFile;

#[test]
fn test_insert_returning_star_and_specific_columns() {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();

    db.execute("CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT NOT NULL, price INTEGER NOT NULL)").unwrap();

    // 1. Insert single row with RETURNING *
    let res = db
        .execute("INSERT INTO products (id, name, price) VALUES (1, 'Laptop', 1200) RETURNING *")
        .unwrap();

    match res {
        ExecResult::Rows { columns, rows } => {
            assert_eq!(columns, vec!["id", "name", "price"]);
            assert_eq!(rows.len(), 1);
            assert_eq!(
                rows[0],
                vec![
                    Value::Integer(1),
                    Value::Text("Laptop".into()),
                    Value::Integer(1200)
                ]
            );
        }
        other => panic!("expected Rows, got {other:?}"),
    }

    // 2. Insert multiple rows with RETURNING specific columns
    let res = db
        .execute("INSERT INTO products (id, name, price) VALUES (2, 'Mouse', 25), (3, 'Keyboard', 75) RETURNING id, name")
        .unwrap();

    match res {
        ExecResult::Rows { columns, rows } => {
            assert_eq!(columns, vec!["id", "name"]);
            assert_eq!(rows.len(), 2);
            assert_eq!(
                rows[0],
                vec![Value::Integer(2), Value::Text("Mouse".into())]
            );
            assert_eq!(
                rows[1],
                vec![Value::Integer(3), Value::Text("Keyboard".into())]
            );
        }
        other => panic!("expected Rows, got {other:?}"),
    }
}

#[test]
fn test_update_returning_updated_values() {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();

    db.execute(
        "CREATE TABLE inventory (id INTEGER PRIMARY KEY, item TEXT NOT NULL, qty INTEGER NOT NULL)",
    )
    .unwrap();
    db.execute(
        "INSERT INTO inventory VALUES (1, 'Apples', 10), (2, 'Bananas', 20), (3, 'Cherries', 30)",
    )
    .unwrap();

    // Update with RETURNING *
    let res = db
        .execute("UPDATE inventory SET qty = 15 WHERE id = 1 RETURNING *")
        .unwrap();

    match res {
        ExecResult::Rows { columns, rows } => {
            assert_eq!(columns, vec!["id", "item", "qty"]);
            assert_eq!(rows.len(), 1);
            assert_eq!(
                rows[0],
                vec![
                    Value::Integer(1),
                    Value::Text("Apples".into()),
                    Value::Integer(15)
                ]
            );
        }
        other => panic!("expected Rows, got {other:?}"),
    }

    // Update multiple with specific RETURNING column
    let res = db
        .execute("UPDATE inventory SET qty = 50 WHERE qty >= 20 RETURNING item")
        .unwrap();

    match res {
        ExecResult::Rows { columns, rows } => {
            assert_eq!(columns, vec!["item"]);
            assert_eq!(rows.len(), 2);
        }
        other => panic!("expected Rows, got {other:?}"),
    }
}

#[test]
fn test_delete_returning_deleted_rows() {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();

    db.execute(
        "CREATE TABLE tasks (id INTEGER PRIMARY KEY, title TEXT NOT NULL, done BOOLEAN NOT NULL)",
    )
    .unwrap();
    db.execute(
        "INSERT INTO tasks VALUES (1, 'Task A', TRUE), (2, 'Task B', FALSE), (3, 'Task C', TRUE)",
    )
    .unwrap();

    // Delete with RETURNING id, title
    let res = db
        .execute("DELETE FROM tasks WHERE done = TRUE RETURNING id, title")
        .unwrap();

    match res {
        ExecResult::Rows { columns, rows } => {
            assert_eq!(columns, vec!["id", "title"]);
            assert_eq!(rows.len(), 2);
            assert_eq!(
                rows[0],
                vec![Value::Integer(1), Value::Text("Task A".into())]
            );
            assert_eq!(
                rows[1],
                vec![Value::Integer(3), Value::Text("Task C".into())]
            );
        }
        other => panic!("expected Rows, got {other:?}"),
    }

    // Ensure they were actually deleted
    let remaining = db.execute("SELECT COUNT(*) FROM tasks").unwrap();
    match remaining {
        ExecResult::Rows { rows, .. } => {
            assert_eq!(rows[0][0], Value::Integer(1));
        }
        other => panic!("expected Rows, got {other:?}"),
    }
}
