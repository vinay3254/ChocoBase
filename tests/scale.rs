use dbengine::types::value::Value;
use dbengine::{Database, ExecResult};
use tempfile::NamedTempFile;

#[test]
#[ignore]
fn hundred_thousand_rows_create_populate_query_update_delete_reopen() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path().to_path_buf();

    {
        let mut db = Database::create(&path).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val INTEGER NOT NULL)").unwrap();
        for i in 0..100_000i64 {
            db.execute(&format!("INSERT INTO t (id, val) VALUES ({i}, {})", i % 1000)).unwrap();
        }

        let result = db.execute("SELECT id FROM t WHERE id = 54321").unwrap();
        assert_eq!(result, ExecResult::Rows { columns: vec!["id".into()], rows: vec![vec![Value::Integer(54321)]] });

        assert_eq!(db.execute("UPDATE t SET val = 9999 WHERE id = 1").unwrap(), ExecResult::Modified(1));
        assert_eq!(db.execute("DELETE FROM t WHERE id = 2").unwrap(), ExecResult::Modified(1));

        // Tree height check, equivalent to what `.btree` shows interactively: dump
        // the table's B+Tree and confirm it went past a single level.
        let dump = db.dump_table_btree("t").unwrap();
        assert!(dump.lines().any(|l| l.trim_start().starts_with("internal")), "100k rows must produce a multi-level tree");
    }

    // Reopen from disk and verify the data, including the update and delete, survived.
    let mut db = Database::open(&path).unwrap();
    let result = db.execute("SELECT val FROM t WHERE id = 1").unwrap();
    assert_eq!(result, ExecResult::Rows { columns: vec!["val".into()], rows: vec![vec![Value::Integer(9999)]] });
    let result = db.execute("SELECT id FROM t WHERE id = 2").unwrap();
    assert_eq!(result, ExecResult::Rows { columns: vec!["id".into()], rows: vec![] });

    // Index page-read reduction: compare pages read for the same query before and
    // after CREATE INDEX, directly demonstrating the design spec's success criterion.
    db.reset_read_counter();
    db.execute("SELECT id FROM t WHERE val = 500").unwrap();
    let pages_without_index = db.pager_stats().pages_read;

    db.execute("CREATE INDEX idx_val ON t (val)").unwrap();

    db.reset_read_counter();
    db.execute("SELECT id FROM t WHERE val = 500").unwrap();
    let pages_with_index = db.pager_stats().pages_read;

    assert!(
        pages_with_index < pages_without_index,
        "indexed lookup ({pages_with_index} pages) should read fewer pages than the sequential scan ({pages_without_index} pages)"
    );
}
