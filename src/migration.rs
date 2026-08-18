//! Schema Migration Engine for ChocoBase.
//! Tracks, versions, and transactionalizes database schema migrations via the `_migrations` system table.

use std::time::{SystemTime, UNIX_EPOCH};
use crate::engine::{Database, ExecResult};
use crate::error::Result;
use crate::types::value::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct Migration {
    pub version: i64,
    pub name: String,
    pub sql: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppliedMigration {
    pub id: i64,
    pub version: i64,
    pub name: String,
    pub applied_at: u64,
}

pub struct MigrationRunner<'a> {
    db: &'a mut Database,
}

impl<'a> MigrationRunner<'a> {
    pub fn new(db: &'a mut Database) -> Self {
        Self { db }
    }

    pub fn ensure_migrations_table(&mut self) -> Result<()> {
        let sql = "CREATE TABLE _migrations (id INTEGER PRIMARY KEY, version INTEGER NOT NULL, name TEXT NOT NULL, applied_at INTEGER NOT NULL)";
        // If it doesn't exist, create it
        if self.db.table_schema("_migrations").is_none() {
            self.db.execute(sql)?;
        }
        Ok(())
    }

    pub fn get_applied_migrations(&mut self) -> Result<Vec<AppliedMigration>> {
        self.ensure_migrations_table()?;
        let res = self.db.execute("SELECT id, version, name, applied_at FROM _migrations ORDER BY version ASC")?;
        let mut list = Vec::new();
        if let ExecResult::Rows { rows, .. } = res {
            for r in rows {
                let id = match r[0] { Value::Integer(i) => i, _ => 0 };
                let version = match r[1] { Value::Integer(v) => v, _ => 0 };
                let name = match &r[2] { Value::Text(s) => s.clone(), _ => String::new() };
                let applied_at = match r[3] { Value::Integer(t) => t as u64, _ => 0 };
                list.push(AppliedMigration { id, version, name, applied_at });
            }
        }
        Ok(list)
    }

    pub fn apply_all(&mut self, migrations: &[Migration]) -> Result<Vec<AppliedMigration>> {
        self.ensure_migrations_table()?;
        let applied = self.get_applied_migrations()?;
        let applied_versions: Vec<i64> = applied.iter().map(|m| m.version).collect();

        let mut newly_applied = Vec::new();
        let mut max_id = applied.iter().map(|m| m.id).max().unwrap_or(0);

        let mut sorted = migrations.to_vec();
        sorted.sort_by_key(|m| m.version);

        for m in sorted {
            if applied_versions.contains(&m.version) {
                continue;
            }

            // Run migration inside transaction
            self.db.execute("BEGIN TRANSACTION")?;
            for stmt in m.sql.split(';') {
                let s = stmt.trim();
                if !s.is_empty() {
                    if let Err(e) = self.db.execute(s) {
                        let _ = self.db.execute("ROLLBACK");
                        return Err(e);
                    }
                }
            }

            max_id += 1;
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
            let record_sql = format!(
                "INSERT INTO _migrations (id, version, name, applied_at) VALUES ({max_id}, {}, '{}', {now})",
                m.version,
                m.name.replace('\'', "''")
            );

            if let Err(e) = self.db.execute(&record_sql) {
                let _ = self.db.execute("ROLLBACK");
                return Err(e);
            }

            self.db.execute("COMMIT")?;

            newly_applied.push(AppliedMigration {
                id: max_id,
                version: m.version,
                name: m.name.clone(),
                applied_at: now,
            });
        }

        Ok(newly_applied)
    }
}
