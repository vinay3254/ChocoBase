//! Database Backup, Dump, and Point-in-Time Recovery Engine for ChocoBase.
//! Generates deterministic SQL DDL + DML dumps and transactional restoration.

use crate::engine::{Database, ExecResult};
use crate::error::Result;
use crate::migration::split_statements;
use crate::types::value::{ColumnType, Value};

/// Generates a complete, self-contained SQL dump of the database schema, indexes, and row data.
pub fn dump_database(db: &mut Database) -> Result<String> {
    let mut out = String::new();
    out.push_str("-- ChocoBase Database Dump\n");
    out.push_str("-- Format: SQL DDL + DML\n\n");

    let tables = db.list_tables();

    // 1. Dump Table DDL & Data
    for table_name in &tables {
        if table_name.starts_with('_') {
            continue;
        }

        let schema = match db.table_schema(table_name) {
            Some(s) => s,
            None => continue,
        };

        out.push_str(&format!("-- Table: {table_name}\n"));
        let mut col_defs = Vec::new();
        for col in &schema.columns {
            let type_str = match col.ty {
                ColumnType::Integer => "INTEGER".to_string(),
                ColumnType::Float => "FLOAT".to_string(),
                ColumnType::Text => "TEXT".to_string(),
                ColumnType::Boolean => "BOOLEAN".to_string(),
                ColumnType::Json => "JSON".to_string(),
                ColumnType::Vector(dim) => format!("VECTOR({dim})"),
            };
            let mut def = format!("{} {}", col.name, type_str);
            if col.is_primary_key {
                def.push_str(" PRIMARY KEY");
            } else if col.not_null {
                def.push_str(" NOT NULL");
            }
            col_defs.push(def);
        }

        out.push_str(&format!(
            "CREATE TABLE {table_name} ({});\n",
            col_defs.join(", ")
        ));

        // Dump Secondary Indexes
        let indexes = db.list_indexes(table_name);
        for idx in &indexes {
            if idx.name.starts_with("pk_") {
                continue; // Skip primary key indexes
            }
            out.push_str(&format!(
                "CREATE INDEX {} ON {table_name} ({});\n",
                idx.name, idx.column
            ));
        }

        // Dump Rows
        let query_sql = format!("SELECT * FROM {table_name}");
        if let Ok(ExecResult::Rows { columns, rows }) = db.execute(&query_sql) {
            for row in rows {
                let mut vals = Vec::new();
                for val in row {
                    match val {
                        Value::Integer(i) => vals.push(i.to_string()),
                        Value::Float(f) => vals.push(f.to_string()),
                        Value::Text(s) => vals.push(format!("'{}'", s.replace('\'', "''"))),
                        Value::Boolean(b) => {
                            vals.push(if b { "TRUE".into() } else { "FALSE".into() })
                        }
                        Value::Json(j) => vals.push(format!("'{}'", j.replace('\'', "''"))),
                        Value::Vector(vec) => {
                            let json = serde_json::to_string(&vec).unwrap_or_else(|_| "[]".into());
                            vals.push(format!("'{json}'"));
                        }
                        Value::Null => vals.push("NULL".into()),
                    }
                }
                out.push_str(&format!(
                    "INSERT INTO {table_name} ({}) VALUES ({});\n",
                    columns.join(", "),
                    vals.join(", ")
                ));
            }
        }
        out.push('\n');
    }

    Ok(out)
}

/// Restores a database from a SQL dump within an atomic transaction.
/// Returns the number of SQL statements successfully executed.
pub fn restore_database(db: &mut Database, sql: &str) -> Result<usize> {
    let stmts = split_statements(sql);
    if stmts.is_empty() {
        return Ok(0);
    }

    db.execute("BEGIN TRANSACTION")?;
    let mut count = 0;

    for stmt in stmts {
        let lines: Vec<&str> = stmt
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with("--"))
            .collect();
        let cleaned_stmt = lines.join(" ");
        if cleaned_stmt.is_empty() {
            continue;
        }

        if cleaned_stmt.to_uppercase().starts_with("CREATE TABLE ") {
            let parts: Vec<&str> = cleaned_stmt.split_whitespace().collect();
            if parts.len() >= 3 {
                let table_name = parts[2].trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
                if db.table_schema(table_name).is_some() {
                    let _ = db.execute(&format!("DROP TABLE {table_name}"));
                }
            }
        }

        if let Err(e) = db.execute(&cleaned_stmt) {
            let _ = db.execute("ROLLBACK");
            return Err(e);
        }
        count += 1;
    }

    db.execute("COMMIT")?;
    Ok(count)
}

/// Ensures the PITR WAL logging table exists.
pub fn ensure_pitr_tables(db: &mut Database) {
    let sql = "CREATE TABLE _pitr_wal_log (id INTEGER PRIMARY KEY, timestamp_ms INTEGER NOT NULL, sql TEXT NOT NULL)";
    let _ = db.execute(sql);
}

/// Logs a mutation statement with timestamp for Point-In-Time Recovery.
pub fn record_pitr_entry(db: &mut Database, timestamp_ms: u64, sql: &str) -> Result<()> {
    ensure_pitr_tables(db);
    let esc_sql = sql.replace('\'', "''");
    let insert = format!(
        "INSERT INTO _pitr_wal_log (id, timestamp_ms, sql) VALUES ({timestamp_ms}, {timestamp_ms}, '{esc_sql}')"
    );
    let _ = db.execute(&insert);
    Ok(())
}

/// Restores a base SQL dump and deterministically replays WAL statements up to target_timestamp_ms.
pub fn restore_to_point_in_time(
    db: &mut Database,
    base_dump_sql: &str,
    target_timestamp_ms: u64,
) -> Result<usize> {
    ensure_pitr_tables(db);
    let query = format!(
        "SELECT sql FROM _pitr_wal_log WHERE timestamp_ms <= {target_timestamp_ms} ORDER BY id ASC"
    );
    let wal_statements = match db.execute(&query) {
        Ok(ExecResult::Rows { rows, .. }) => rows
            .into_iter()
            .filter_map(|r| match r.into_iter().next() {
                Some(Value::Text(s)) => Some(s),
                _ => None,
            })
            .collect::<Vec<String>>(),
        _ => vec![],
    };

    let mut total = 0;
    if !base_dump_sql.is_empty() {
        total += restore_database(db, base_dump_sql)?;
    }

    if !wal_statements.is_empty() {
        db.execute("BEGIN TRANSACTION")?;
        for stmt in &wal_statements {
            if let Err(e) = db.execute(stmt) {
                let _ = db.execute("ROLLBACK");
                return Err(e);
            }
            total += 1;
        }
        db.execute("COMMIT")?;
    }

    Ok(total)
}
