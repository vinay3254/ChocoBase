use dbengine::engine::{Database, ExecResult};
use dbengine::types::value::Value;
use tempfile::NamedTempFile;

#[test]
fn test_in_subquery_and_not_in_subquery() {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();

    db.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, active BOOLEAN)")
        .unwrap();
    db.execute("CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER, amount INTEGER)")
        .unwrap();

    db.execute("INSERT INTO users (id, name, active) VALUES (1, 'Alice', true), (2, 'Bob', false), (3, 'Charlie', true)")
        .unwrap();
    db.execute("INSERT INTO orders (id, user_id, amount) VALUES (101, 1, 50), (102, 2, 120), (103, 3, 300), (104, 1, 80)")
        .unwrap();

    // 1. SELECT WHERE user_id IN (SELECT id FROM users WHERE active = true)
    let res = db
        .execute("SELECT id, amount FROM orders WHERE user_id IN (SELECT id FROM users WHERE active = true) ORDER BY id ASC")
        .unwrap();

    match res {
        ExecResult::Rows { rows, .. } => {
            // Orders for active users (Alice: 101, 104; Charlie: 103)
            assert_eq!(rows.len(), 3);
            assert_eq!(rows[0][0], Value::Integer(101));
            assert_eq!(rows[1][0], Value::Integer(103));
            assert_eq!(rows[2][0], Value::Integer(104));
        }
        other => panic!("unexpected result: {other:?}"),
    }

    // 2. SELECT WHERE user_id NOT IN (SELECT id FROM users WHERE active = true)
    let res = db
        .execute("SELECT id, amount FROM orders WHERE user_id NOT IN (SELECT id FROM users WHERE active = true)")
        .unwrap();

    match res {
        ExecResult::Rows { rows, .. } => {
            // Only Bob's order (102)
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0][0], Value::Integer(102));
            assert_eq!(rows[0][1], Value::Integer(120));
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn test_exists_and_not_exists_subqueries() {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();

    db.execute("CREATE TABLE accounts (id INTEGER PRIMARY KEY, status TEXT)")
        .unwrap();
    db.execute("CREATE TABLE log_entries (id INTEGER PRIMARY KEY, msg TEXT)")
        .unwrap();

    db.execute("INSERT INTO accounts (id, status) VALUES (1, 'premium'), (2, 'trial')")
        .unwrap();
    db.execute("INSERT INTO log_entries (id, msg) VALUES (10, 'system booted')")
        .unwrap();

    // 1. WHERE EXISTS (SELECT * FROM accounts WHERE status = 'premium') -> returns all rows
    let res = db
        .execute("SELECT id, msg FROM log_entries WHERE EXISTS (SELECT id FROM accounts WHERE status = 'premium')")
        .unwrap();

    match res {
        ExecResult::Rows { rows, .. } => {
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0][0], Value::Integer(10));
        }
        other => panic!("unexpected result: {other:?}"),
    }

    // 2. WHERE EXISTS (SELECT * FROM accounts WHERE status = 'banned') -> returns 0 rows
    let res = db
        .execute("SELECT id, msg FROM log_entries WHERE EXISTS (SELECT id FROM accounts WHERE status = 'banned')")
        .unwrap();

    match res {
        ExecResult::Rows { rows, .. } => {
            assert_eq!(rows.len(), 0);
        }
        other => panic!("unexpected result: {other:?}"),
    }

    // 3. WHERE NOT EXISTS (SELECT * FROM accounts WHERE status = 'banned') -> returns all rows
    let res = db
        .execute("SELECT id, msg FROM log_entries WHERE NOT EXISTS (SELECT id FROM accounts WHERE status = 'banned')")
        .unwrap();

    match res {
        ExecResult::Rows { rows, .. } => {
            assert_eq!(rows.len(), 1);
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn test_subqueries_in_mutations_update_and_delete() {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();

    db.execute("CREATE TABLE employees (id INTEGER PRIMARY KEY, dept TEXT, bonus INTEGER)")
        .unwrap();
    db.execute("CREATE TABLE top_departments (name TEXT PRIMARY KEY)")
        .unwrap();

    db.execute("INSERT INTO employees (id, dept, bonus) VALUES (1, 'Engineering', 0), (2, 'Sales', 0), (3, 'HR', 0)")
        .unwrap();
    db.execute("INSERT INTO top_departments (name) VALUES ('Engineering'), ('Sales')")
        .unwrap();

    // 1. UPDATE employees SET bonus = 1000 WHERE dept IN (SELECT name FROM top_departments)
    let res = db
        .execute(
            "UPDATE employees SET bonus = 1000 WHERE dept IN (SELECT name FROM top_departments)",
        )
        .unwrap();
    assert_eq!(res, ExecResult::Modified(2));

    // Verify bonuses updated
    let res = db
        .execute("SELECT id, bonus FROM employees ORDER BY id ASC")
        .unwrap();
    match res {
        ExecResult::Rows { rows, .. } => {
            assert_eq!(rows[0][1], Value::Integer(1000));
            assert_eq!(rows[1][1], Value::Integer(1000));
            assert_eq!(rows[2][1], Value::Integer(0));
        }
        other => panic!("unexpected result: {other:?}"),
    }

    // 2. DELETE FROM employees WHERE dept NOT IN (SELECT name FROM top_departments)
    let res = db
        .execute("DELETE FROM employees WHERE dept NOT IN (SELECT name FROM top_departments)")
        .unwrap();
    assert_eq!(res, ExecResult::Modified(1));

    // Verify remaining count
    let res = db.execute("SELECT id FROM employees").unwrap();
    match res {
        ExecResult::Rows { rows, .. } => {
            assert_eq!(rows.len(), 2);
        }
        other => panic!("unexpected result: {other:?}"),
    }
}
