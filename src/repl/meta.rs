use crate::engine::Database;

pub fn dispatch(db: &mut Database, cmd: &str) -> String {
    let cmd = cmd.trim();
    if cmd == "tables" {
        return db.list_tables().join("\n");
    }
    if let Some(rest) = cmd.strip_prefix("schema") {
        return schema_text(db, rest.trim());
    }
    if let Some(rest) = cmd.strip_prefix("indexes") {
        return indexes_text(db, rest.trim());
    }
    if let Some(rest) = cmd.strip_prefix("btree") {
        return btree_text(db, rest.trim());
    }
    if cmd == "stats" {
        return stats_text(db);
    }
    format!("unknown command: .{cmd}")
}

fn schema_text(db: &mut Database, table: &str) -> String {
    match db.table_schema(table) {
        Some(schema) => {
            let cols: Vec<String> = schema
                .columns
                .iter()
                .map(|c| {
                    let ty = match c.ty {
                        crate::types::value::ColumnType::Integer => "INTEGER".to_string(),
                        crate::types::value::ColumnType::Float => "FLOAT".to_string(),
                        crate::types::value::ColumnType::Text => "TEXT".to_string(),
                        crate::types::value::ColumnType::Boolean => "BOOLEAN".to_string(),
                        crate::types::value::ColumnType::Json => "JSON".to_string(),
                        crate::types::value::ColumnType::Vector(dim) => format!("VECTOR({dim})"),
                    };
                    let mut parts = vec![c.name.clone(), ty];
                    if c.not_null {
                        parts.push("NOT NULL".to_string());
                    }
                    if c.is_primary_key {
                        parts.push("PRIMARY KEY".to_string());
                    }
                    parts.join(" ")
                })
                .collect();
            format!("CREATE TABLE {} ({})", schema.name, cols.join(", "))
        }
        None => format!("no such table: {table}"),
    }
}

fn indexes_text(db: &mut Database, table: &str) -> String {
    let indexes = db.list_indexes(table);
    if indexes.is_empty() {
        return "(no indexes)".to_string();
    }
    indexes
        .iter()
        .map(|i| format!("{} ON {} ({})", i.name, i.table, i.column))
        .collect::<Vec<_>>()
        .join("\n")
}

fn btree_text(db: &mut Database, table: &str) -> String {
    match db.dump_table_btree(table) {
        Some(text) => text,
        None => format!("no such table: {table}"),
    }
}

fn stats_text(db: &mut Database) -> String {
    let s = db.pager_stats();
    format!(
        "pages: {}\nfreelist: {}\ncached pages: {}\npages read since last statement: {}",
        s.page_count, s.freelist_head, s.cached_pages, s.pages_read
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn db_with_one_table() -> Database {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        std::mem::forget(file); // acceptable here: test runs and drops db within the same call, no cross-scope reopen
        db.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
            .unwrap();
        db
    }

    #[test]
    fn tables_lists_table_names() {
        let mut db = db_with_one_table();
        assert_eq!(dispatch(&mut db, "tables"), "users");
    }

    #[test]
    fn schema_shows_create_table_text() {
        let mut db = db_with_one_table();
        let out = dispatch(&mut db, "schema users");
        assert!(out.contains("CREATE TABLE users"));
        assert!(out.contains("id INTEGER"));
        assert!(out.contains("PRIMARY KEY"));
        assert!(out.contains("name TEXT"));
        assert!(out.contains("NOT NULL"));
    }

    #[test]
    fn schema_missing_table_reports_error() {
        let mut db = db_with_one_table();
        assert!(dispatch(&mut db, "schema nope").contains("no such table"));
    }

    #[test]
    fn indexes_lists_indexes_for_table() {
        let mut db = db_with_one_table();
        db.execute("CREATE INDEX idx_name ON users (name)").unwrap();
        let out = dispatch(&mut db, "indexes users");
        assert!(out.contains("idx_name"));
    }

    #[test]
    fn unknown_command_reports_error() {
        let mut db = db_with_one_table();
        assert!(dispatch(&mut db, "bogus").contains("unknown command"));
    }

    #[test]
    fn btree_dumps_table_structure() {
        let mut db = db_with_one_table();
        for i in 0..50 {
            db.execute(&format!(
                "INSERT INTO users (id, name) VALUES ({i}, 'n{i}')"
            ))
            .unwrap();
        }
        let out = dispatch(&mut db, "btree users");
        assert!(out.contains("leaf page"));
    }

    #[test]
    fn stats_reports_page_count_and_reads() {
        let mut db = db_with_one_table();
        let out = dispatch(&mut db, "stats");
        assert!(out.contains("pages:"));
        assert!(out.contains("freelist:"));
        assert!(out.contains("pages read since last statement:"));
    }
}
