use dbengine::Database;
use proptest::prelude::*;
use tempfile::NamedTempFile;

// Returns the Database together with the NamedTempFile that backs it — the
// temp file must stay alive (not be dropped/deleted) for as long as the
// Database's Pager holds an open file handle to it, so the caller must keep
// both bindings in scope for the same duration rather than discarding the
// NamedTempFile immediately.
fn make_db_with_rows(rows: &[(i64, i64)]) -> (Database, NamedTempFile) {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();
    db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, score INTEGER NOT NULL)")
        .unwrap();
    for (id, score) in rows {
        db.execute(&format!("INSERT INTO t (id, score) VALUES ({id}, {score})"))
            .unwrap();
    }
    (db, file)
}

proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig {
        cases: 32,
        ..proptest::prelude::ProptestConfig::default()
    })]
    #[test]
    fn indexed_and_unindexed_queries_agree(
        rows in prop::collection::vec((0i64..500, 0i64..20), 1..100),
        target in 0i64..20,
    ) {
        // de-duplicate ids: the primary key must be unique per row.
        let mut seen = std::collections::HashSet::new();
        let unique_rows: Vec<(i64, i64)> = rows.into_iter().filter(|(id, _)| seen.insert(*id)).collect();

        let query = format!("SELECT id FROM t WHERE score = {target} ORDER BY id");

        let (mut without_index, _file1) = make_db_with_rows(&unique_rows);
        let without_result = without_index.execute(&query).unwrap();

        let (mut with_index, _file2) = make_db_with_rows(&unique_rows);
        with_index.execute("CREATE INDEX idx_score ON t (score)").unwrap();
        let with_result = with_index.execute(&query).unwrap();

        prop_assert_eq!(without_result, with_result);
    }
}
