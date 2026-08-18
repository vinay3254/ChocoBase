use tempfile::NamedTempFile;

use dbengine::types::value::Value;
use dbengine::{Database, ExecResult};

#[test]
fn json_column_creation_insertion_and_arrow_extraction() {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();

    db.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, profile JSON)")
        .unwrap();

    db.execute(
        "INSERT INTO users (id, profile) VALUES (1, '{\"name\": \"Alice\", \"age\": 30, \"role\": \"admin\"}'), (2, '{\"name\": \"Bob\", \"age\": 25, \"role\": \"user\"}')"
    ).unwrap();

    // Query with arrow extract ->>
    let res = db
        .execute("SELECT profile->>'name', profile->>'role' FROM users WHERE id = 1")
        .unwrap();
    match res {
        ExecResult::Rows { rows, .. } => {
            assert_eq!(
                rows,
                vec![vec![
                    Value::Text("Alice".into()),
                    Value::Text("admin".into())
                ]]
            );
        }
        other => panic!("unexpected result: {other:?}"),
    }

    // Filter in WHERE using JSON field
    let res = db
        .execute("SELECT id, profile->>'name' FROM users WHERE profile->>'role' = 'admin'")
        .unwrap();
    match res {
        ExecResult::Rows { rows, .. } => {
            assert_eq!(
                rows,
                vec![vec![Value::Integer(1), Value::Text("Alice".into())]]
            );
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn json_extract_nested_properties_and_arrays() {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();

    db.execute("CREATE TABLE events (id INTEGER PRIMARY KEY, payload JSON)")
        .unwrap();

    db.execute(
        "INSERT INTO events (id, payload) VALUES (10, '{\"metadata\": {\"ip\": \"127.0.0.1\", \"tags\": [\"auth\", \"login\"]}}')"
    ).unwrap();

    let res = db
        .execute(
            "SELECT payload->'metadata.ip', payload->'metadata.tags.0' FROM events WHERE id = 10",
        )
        .unwrap();
    match res {
        ExecResult::Rows { rows, .. } => {
            assert_eq!(
                rows,
                vec![vec![
                    Value::Text("127.0.0.1".into()),
                    Value::Text("auth".into())
                ]]
            );
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn invalid_json_rejected_on_insert() {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();

    db.execute("CREATE TABLE docs (id INTEGER PRIMARY KEY, data JSON)")
        .unwrap();

    // Inserting malformed JSON syntax must be rejected
    let res = db.execute("INSERT INTO docs (id, data) VALUES (1, '{unquoted_key: 123')");
    assert!(
        res.is_err(),
        "malformed JSON syntax must be rejected on insert"
    );
}
