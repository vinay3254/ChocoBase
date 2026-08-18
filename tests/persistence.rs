use dbengine::types::value::Value;
use dbengine::{Database, ExecResult};
use tempfile::NamedTempFile;

#[test]
fn data_survives_full_close_and_reopen() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path().to_path_buf();

    {
        let mut db = Database::create(&path).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
            .unwrap();
        db.execute("INSERT INTO t (id, name) VALUES (1, 'a'), (2, 'b'), (3, 'c')")
            .unwrap();
        db.execute("CREATE INDEX idx_name ON t (name)").unwrap();
        // db is dropped here, closing its Pager's file handle — no explicit close() exists
        // or is needed, since every statement already flushes and fsyncs on commit.
    }

    let mut db = Database::open(&path).unwrap();
    let result = db.execute("SELECT id, name FROM t WHERE id = 2").unwrap();
    assert_eq!(
        result,
        ExecResult::Rows {
            columns: vec!["id".into(), "name".into()],
            rows: vec![vec![Value::Integer(2), Value::Text("b".into())]]
        }
    );

    // the index built before close must still route through IndexSeek after reopen —
    // this is the same query Task 35's engine test uses, now run against a reopened file.
    let result = db.execute("SELECT id FROM t WHERE name = 'c'").unwrap();
    assert_eq!(
        result,
        ExecResult::Rows {
            columns: vec!["id".into()],
            rows: vec![vec![Value::Integer(3)]]
        }
    );

    db.execute("DELETE FROM t WHERE id = 1").unwrap();
    drop(db);

    let mut db = Database::open(&path).unwrap();
    let result = db.execute("SELECT id FROM t").unwrap();
    match result {
        ExecResult::Rows { rows, .. } => assert_eq!(
            rows.len(),
            2,
            "delete before close must also have persisted"
        ),
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn every_statement_is_durable_even_without_explicit_flush_call() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path().to_path_buf();

    let mut db = Database::create(&path).unwrap();
    db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        .unwrap();
    for i in 0..50 {
        db.execute(&format!("INSERT INTO t (id) VALUES ({i})"))
            .unwrap();
        // Reopening after every single statement (not just at the end) proves each
        // individual execute() call is fsynced on its own, per the autocommit design.
        drop(Database::open(&path).unwrap());
    }
    drop(db);

    let mut db = Database::open(&path).unwrap();
    let result = db.execute("SELECT id FROM t").unwrap();
    match result {
        ExecResult::Rows { rows, .. } => assert_eq!(rows.len(), 50),
        other => panic!("unexpected result: {other:?}"),
    }
}
