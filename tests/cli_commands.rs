//! Automated integration tests for ChocoBase CLI subcommands:
//! init, migrate, dump, restore, status.

use dbengine::engine::Database;
use dbengine::migration::{load_from_dir, MigrationRunner};
use tempfile::tempdir;

#[test]
fn test_cli_migrations_and_dump_restore_cycle() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("cli_test.db");
    let migrations_dir = dir.path().join("migrations");
    std::fs::create_dir_all(&migrations_dir).unwrap();

    // 1. Create migration files: 001_create_users.sql, 002_create_posts.sql
    let m1_content = "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, email TEXT NOT NULL);";
    let m2_content = "CREATE TABLE posts (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL, title TEXT NOT NULL);";
    std::fs::write(migrations_dir.join("001_create_users.sql"), m1_content).unwrap();
    std::fs::write(migrations_dir.join("002_create_posts.sql"), m2_content).unwrap();

    // 2. Load and apply migrations
    let mut db = Database::create(&db_path).unwrap();
    let migrations = load_from_dir(&migrations_dir).unwrap();
    assert_eq!(migrations.len(), 2);

    let mut runner = MigrationRunner::new(&mut db);
    let applied = runner.apply_all(&migrations).unwrap();
    assert_eq!(applied.len(), 2);
    assert_eq!(applied[0].version, 1);
    assert_eq!(applied[1].version, 2);

    // 3. Insert some rows
    db.execute("INSERT INTO users (id, name, email) VALUES (1, 'Alice', 'alice@test.com');")
        .unwrap();
    db.execute("INSERT INTO posts (id, user_id, title) VALUES (10, 1, 'First Post');")
        .unwrap();

    // 4. Dump database to SQL
    let dump_sql = dbengine::backup::dump_database(&mut db).unwrap();
    assert!(dump_sql.contains("CREATE TABLE users"));
    assert!(dump_sql.contains("CREATE TABLE posts"));
    assert!(dump_sql.contains("INSERT INTO users"));
    assert!(dump_sql.contains("INSERT INTO posts"));

    // 5. Restore into a brand-new database instance
    let restore_db_path = dir.path().join("restored_cli.db");
    let mut restored_db = Database::create(&restore_db_path).unwrap();
    let count = dbengine::backup::restore_database(&mut restored_db, &dump_sql).unwrap();
    assert!(count >= 4);

    // 6. Verify restored database has identical tables and rows
    let tables = restored_db.list_tables();
    assert!(tables.contains(&"users".to_string()));
    assert!(tables.contains(&"posts".to_string()));

    let select_res = restored_db.execute("SELECT name FROM users WHERE id = 1;").unwrap();
    if let dbengine::engine::ExecResult::Rows { rows, .. } = select_res {
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0], dbengine::Value::Text("Alice".into()));
    } else {
        panic!("expected row result");
    }
}
