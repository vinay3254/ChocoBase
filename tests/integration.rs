use dbengine::{Database, ExecResult};
use dbengine::types::value::Value;
use tempfile::NamedTempFile;

#[test]
fn create_insert_select_end_to_end() {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();

    db.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, active BOOLEAN)")
        .unwrap();

    db.execute("INSERT INTO users (id, name, active) VALUES (1, 'Ada', TRUE), (2, 'Bea', FALSE), (3, 'Cy', TRUE)")
        .unwrap();

    let result = db.execute("SELECT name FROM users WHERE active = TRUE ORDER BY id").unwrap();
    match result {
        ExecResult::Rows { columns, rows } => {
            assert_eq!(columns, vec!["name".to_string()]);
            assert_eq!(rows, vec![vec![Value::Text("Ada".into())], vec![Value::Text("Cy".into())]]);
        }
        other => panic!("unexpected result: {other:?}"),
    }
}
