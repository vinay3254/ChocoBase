use std::fs;
use tempfile::{tempdir, NamedTempFile};

use dbengine::{
    dump_database, restore_database, Database, ExecResult, Migration, MigrationRunner, Value,
};

#[test]
fn test_cli_migration_runner_directory_workflow() {
    let dir = tempdir().unwrap();
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();

    // Create migration files in directory
    let m1 = "CREATE TABLE users_v1 (id INTEGER PRIMARY KEY, name TEXT NOT NULL);";
    let m2 = "CREATE TABLE orders_v1 (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL, amount INTEGER NOT NULL);";
    let m3 = "CREATE INDEX idx_orders_user ON orders_v1 (user_id);";

    fs::write(dir.path().join("001_create_users.sql"), m1).unwrap();
    fs::write(dir.path().join("002_create_orders.sql"), m2).unwrap();
    fs::write(dir.path().join("003_add_index.sql"), m3).unwrap();

    // Read and run migrations
    let mut migrations = Vec::new();
    for entry in fs::read_dir(dir.path()).unwrap().flatten() {
        let p = entry.path();
        if p.extension().map(|e| e == "sql").unwrap_or(false) {
            if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                if let Some((v_str, desc)) = stem.split_once('_') {
                    if let Ok(v) = v_str.parse::<i64>() {
                        let sql = fs::read_to_string(&p).unwrap();
                        migrations.push(Migration {
                            version: v,
                            name: desc.to_string(),
                            sql,
                        });
                    }
                }
            }
        }
    }
    migrations.sort_by_key(|m| m.version);

    let mut runner = MigrationRunner::new(&mut db);
    let applied = runner.apply_all(&migrations).unwrap();
    assert_eq!(applied.len(), 3);

    // Verify tables exist
    let tables = db.list_tables();
    assert!(tables.contains(&"users_v1".to_string()));
    assert!(tables.contains(&"orders_v1".to_string()));
}

#[test]
fn test_cli_dump_and_restore_file_workflow() {
    let src_file = NamedTempFile::new().unwrap();
    let mut src_db = Database::create(src_file.path()).unwrap();

    src_db
        .execute("CREATE TABLE inventory (id INTEGER PRIMARY KEY, item TEXT NOT NULL, qty INTEGER NOT NULL)")
        .unwrap();
    src_db.execute("BEGIN TRANSACTION").unwrap();
    src_db
        .execute(
            "INSERT INTO inventory (id, item, qty) VALUES (1, 'Widget', 100), (2, 'Gadget', 50)",
        )
        .unwrap();
    src_db.execute("COMMIT").unwrap();

    // Dump SQL to string (mimicking file write)
    let dump_sql = dump_database(&mut src_db).unwrap();

    // Restore into new database
    let dst_file = NamedTempFile::new().unwrap();
    let mut dst_db = Database::create(dst_file.path()).unwrap();

    let restored_count = restore_database(&mut dst_db, &dump_sql).unwrap();
    assert!(restored_count >= 2);

    let res = dst_db
        .execute("SELECT id, item, qty FROM inventory ORDER BY id")
        .unwrap();
    if let ExecResult::Rows { rows, .. } = res {
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][1], Value::Text("Widget".into()));
        assert_eq!(rows[1][1], Value::Text("Gadget".into()));
    } else {
        panic!("expected rows");
    }
}

#[test]
fn test_cli_seed_sql_file_workflow() {
    let dir = tempdir().unwrap();
    let db_file = dir.path().join("seeded.db");
    let seed_file = dir.path().join("seed.sql");

    let seed_sql = "CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT NOT NULL);\n\
                    INSERT INTO customers (id, name) VALUES (1, 'Alice');\n\
                    INSERT INTO customers (id, name) VALUES (2, 'Bob');";
    fs::write(&seed_file, seed_sql).unwrap();

    let mut db = Database::create(&db_file).unwrap();
    let content = fs::read_to_string(&seed_file).unwrap();
    for stmt in content.split(';') {
        let s = stmt.trim();
        if !s.is_empty() {
            let _ = db.execute(&format!("{s};"));
        }
    }

    let res = db.execute("SELECT id, name FROM customers ORDER BY id ASC").unwrap();
    match res {
        ExecResult::Rows { rows, .. } => {
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0][1], Value::Text("Alice".to_string()));
            assert_eq!(rows[1][1], Value::Text("Bob".to_string()));
        }
        other => panic!("expected rows, got {:?}", other),
    }
}
