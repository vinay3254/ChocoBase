use std::process::Command;
use tempfile::NamedTempFile;

use dbengine::engine::{Database, ExecResult};
use dbengine::types::value::Value;

#[test]
fn child_process_killed_mid_transaction_is_recovered_on_reopen() {
    let file = NamedTempFile::new().unwrap();
    let db_path = file.path().to_str().unwrap().to_string();

    // 1. Initial setup in parent process: create table and insert 1 row
    {
        let mut db = Database::create(std::path::Path::new(&db_path)).unwrap();
        db.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
            .unwrap();
        db.execute("INSERT INTO users (id, name) VALUES (1, 'alice')")
            .unwrap();
    }

    // 2. Spawn a child CLI process that begins a transaction, modifies rows, and waits
    let mut child = Command::new(env!("CARGO_BIN_EXE_dbengine"))
        .arg(&db_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn child dbengine process");

    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().expect("failed to open child stdin");
        writeln!(stdin, "BEGIN;").unwrap();
        writeln!(stdin, "UPDATE users SET name = 'bob' WHERE id = 1;").unwrap();
        writeln!(stdin, "INSERT INTO users (id, name) VALUES (2, 'charlie');").unwrap();
        stdin.flush().unwrap();
    }

    // Give the child process time to execute and write journal pre-images
    std::thread::sleep(std::time::Duration::from_millis(500));

    // 3. Forcibly terminate (SIGKILL) the child process mid-transaction
    child.kill().expect("failed to kill child process");
    let _ = child.wait();

    // 4. Open the database in the parent process: startup recovery must detect and undo the transaction
    let mut db =
        Database::open(std::path::Path::new(&db_path)).expect("Database::open failed after crash");

    // 5. Verify the pre-transaction state was restored: id=1 is 'alice', id=2 does not exist
    let res = db.execute("SELECT name FROM users WHERE id = 1").unwrap();
    match res {
        ExecResult::Rows { rows, .. } => assert_eq!(rows, vec![vec![Value::Text("alice".into())]]),
        other => panic!("unexpected result: {other:?}"),
    }

    let res2 = db.execute("SELECT * FROM users WHERE id = 2").unwrap();
    match res2 {
        ExecResult::Rows { rows, .. } => assert!(rows.is_empty()),
        other => panic!("unexpected result: {other:?}"),
    }
}
