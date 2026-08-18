use dbengine::{ExecResult, SharedDatabase};
use tempfile::NamedTempFile;
use std::sync::{Arc, Barrier};
use std::thread;

#[test]
fn shared_database_serializes_writers_and_allows_concurrent_clients() {
    let file = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(file.path()).unwrap();
    db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, worker INTEGER NOT NULL)").unwrap();

    let workers = 4;
    let per_worker = 40;
    let barrier = Arc::new(Barrier::new(workers + 2));
    let mut handles = Vec::new();
    for worker in 0..workers {
        let session = db.clone();
        let start = barrier.clone();
        handles.push(thread::spawn(move || {
            start.wait();
            for offset in 0..per_worker {
                let id = worker * per_worker + offset;
                session.execute(&format!("INSERT INTO t (id, worker) VALUES ({id}, {worker})")).unwrap();
            }
        }));
    }

    let reader = db.clone();
    let start = barrier.clone();
    let reader_handle = thread::spawn(move || {
        start.wait();
        let mut observed = 0;
        for _ in 0..20 {
            if let ExecResult::Rows { rows, .. } = reader.execute("SELECT id FROM t").unwrap() {
                observed = observed.max(rows.len());
            }
            thread::yield_now();
        }
        observed
    });

    barrier.wait();
    for handle in handles { handle.join().unwrap(); }
    let observed = reader_handle.join().unwrap();
    assert!(observed > 0, "reader should make progress while writers run");

    match db.execute("SELECT id FROM t").unwrap() {
        ExecResult::Rows { rows, .. } => assert_eq!(rows.len(), workers * per_worker),
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn explicit_shared_transaction_holds_writer_lock_until_commit() {
    let file = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(file.path()).unwrap();
    db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)").unwrap();
    let tx = db.clone();
    tx.execute("BEGIN").unwrap();
    tx.execute("INSERT INTO t (id) VALUES (1)").unwrap();

    let other = db.clone();
    let handle = thread::spawn(move || other.execute("INSERT INTO t (id) VALUES (2)").unwrap());
    thread::sleep(std::time::Duration::from_millis(50));
    assert!(!handle.is_finished(), "writer must wait for the explicit transaction");
    tx.execute("COMMIT").unwrap();
    handle.join().unwrap();
}

#[test]
fn explicit_shared_transaction_rollback_releases_lock_and_discards_data() {
    let file = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(file.path()).unwrap();
    db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT)").unwrap();
    db.execute("INSERT INTO t (id, val) VALUES (1, 'initial')").unwrap();

    let tx = db.clone();
    tx.execute("BEGIN").unwrap();
    tx.execute("UPDATE t SET val = 'modified' WHERE id = 1").unwrap();

    let reader = db.clone();
    let handle = thread::spawn(move || {
        let res = reader.execute("SELECT val FROM t WHERE id = 1").unwrap();
        match res {
            ExecResult::Rows { rows, .. } => rows[0][0].clone(),
            other => panic!("unexpected result: {other:?}"),
        }
    });

    thread::sleep(std::time::Duration::from_millis(50));
    assert!(!handle.is_finished(), "reader must wait for the exclusive transaction");

    tx.execute("ROLLBACK").unwrap();
    let read_val = handle.join().unwrap();
    assert_eq!(read_val, dbengine::types::value::Value::Text("initial".into()));
}
