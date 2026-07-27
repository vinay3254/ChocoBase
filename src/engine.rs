use std::path::Path;

use crate::btree::node::LeafNode;
use crate::catalog::Catalog;
use crate::error::{DbError, PlanError, Result};
use crate::sql::ast::{ColumnDef, Statement};
use crate::storage::pager::Pager;
use crate::types::schema::{Column, TableSchema};
use crate::types::value::Value;

pub struct Database {
    pager: Pager,
    catalog: Catalog,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExecResult {
    Rows { columns: Vec<String>, rows: Vec<Vec<Value>> },
    Modified(usize),
    Ok,
}

impl Database {
    pub fn create(path: &Path) -> Result<Self> {
        let mut pager = Pager::create(path)?;
        let catalog = Catalog::bootstrap(&mut pager)?;
        Ok(Database { pager, catalog })
    }

    pub fn open(path: &Path) -> Result<Self> {
        let mut pager = Pager::open(path)?;
        let catalog = Catalog::bootstrap(&mut pager)?;
        Ok(Database { pager, catalog })
    }

    pub fn execute(&mut self, sql: &str) -> Result<ExecResult> {
        let stmt = crate::sql::parser::parse(sql)?;
        let result = match stmt {
            Statement::CreateTable { name, columns } => self.execute_create_table(name, columns)?,
            Statement::DropTable { name } => self.execute_drop_table(&name)?,
            other => {
                return Err(DbError::Plan(PlanError::InvalidSchema(format!(
                    "statement not yet supported: {other:?}"
                ))))
            }
        };
        self.pager.flush()?;
        Ok(result)
    }

    fn execute_create_table(&mut self, name: String, columns: Vec<ColumnDef>) -> Result<ExecResult> {
        let pk_count = columns.iter().filter(|c| c.primary_key).count();
        if pk_count != 1 {
            return Err(DbError::Plan(PlanError::InvalidSchema(format!(
                "table {name} must declare exactly one PRIMARY KEY column"
            ))));
        }
        let root = self.pager.allocate_page()?;
        LeafNode { entries: vec![], next_leaf: 0 }.encode(self.pager.get_page_mut(root)?);
        let cols = columns
            .into_iter()
            .map(|c| Column {
                name: c.name,
                ty: c.ty,
                not_null: c.not_null || c.primary_key,
                is_primary_key: c.primary_key,
            })
            .collect();
        let schema = TableSchema { name, columns: cols, root_page: root };
        self.catalog.create_table(&mut self.pager, &schema)?;
        Ok(ExecResult::Ok)
    }

    fn execute_drop_table(&mut self, name: &str) -> Result<ExecResult> {
        self.catalog.drop_table(&mut self.pager, name)?;
        Ok(ExecResult::Ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn create_table_then_drop_table() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        assert_eq!(
            db.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)").unwrap(),
            ExecResult::Ok
        );
        assert_eq!(db.execute("DROP TABLE users").unwrap(), ExecResult::Ok);
    }

    #[test]
    fn create_table_without_primary_key_errors() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        let err = db.execute("CREATE TABLE t (a INTEGER)").unwrap_err();
        assert!(matches!(err, DbError::Plan(PlanError::InvalidSchema(_))));
    }

    #[test]
    fn create_duplicate_table_errors() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)").unwrap();
        let err = db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)").unwrap_err();
        assert!(matches!(err, DbError::Plan(PlanError::TableAlreadyExists(_))));
    }

    #[test]
    fn reopening_preserves_schema() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path();
        {
            let mut db = Database::create(path).unwrap();
            db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)").unwrap();
        }
        let mut db = Database::open(path).unwrap();
        db.execute("DROP TABLE t").unwrap(); // succeeds only if the schema survived reopen
    }
}
