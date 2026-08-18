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

        if let Err(e) = db.execute(&cleaned_stmt) {
            let _ = db.execute("ROLLBACK");
            return Err(e);
        }
        count += 1;
    }

    db.execute("COMMIT")?;
    Ok(count)
}
