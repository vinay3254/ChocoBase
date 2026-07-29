use std::path::Path;
use std::collections::HashMap;

use crate::btree::node::LeafNode;
use crate::catalog::Catalog;
use crate::error::{DbError, PlanError, Result};
use crate::exec::Operator;
use crate::sql::ast::{ColumnDef, Statement, Expr, SelectColumns};
use crate::storage::pager::Pager;
use crate::types::schema::{Column, TableSchema, IndexSchema};
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
            Statement::CreateIndex { name, table, column } => self.execute_create_index(&name, &table, &column)?,
            Statement::DropIndex { name } => self.execute_drop_index(&name)?,
            Statement::Insert { table, columns, rows } => self.execute_insert(&table, columns, rows)?,
            Statement::Select { columns, table, where_clause, order_by, limit } => {
                self.execute_select(columns, &table, where_clause, order_by, limit)?
            }
            Statement::Delete { table, where_clause } => self.execute_delete(&table, where_clause)?,
            Statement::Update { table, assignments, where_clause } => self.execute_update(&table, assignments, where_clause)?,
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

    fn execute_create_index(&mut self, name: &str, table: &str, column: &str) -> Result<ExecResult> {
        let schema = self
            .catalog
            .get_table(&mut self.pager, table)?
            .ok_or_else(|| PlanError::NoSuchTable(table.to_string()))?;
        let col_idx = schema
            .column_index(column)
            .ok_or_else(|| PlanError::NoSuchColumn(column.to_string()))?;
        if !schema.columns[col_idx].not_null {
            return Err(DbError::Plan(PlanError::InvalidSchema(format!(
                "indexed column {column} must be NOT NULL"
            ))));
        }

        let initial_index_root = self.pager.allocate_page()?;
        LeafNode { entries: vec![], next_leaf: 0 }.encode(self.pager.get_page_mut(initial_index_root)?);

        let pk_idx = schema.primary_key_index();
        let mut scan = crate::exec::scan::SeqScan::new(schema.clone());
        let mut current_root = initial_index_root;
        while let Some(row) = scan.next(&mut self.pager)? {
            let idx_key = crate::types::value::encode_composite_key(&[row[col_idx].clone(), row[pk_idx].clone()]);
            let mut ibt = crate::btree::tree::BTree::new(&mut self.pager, current_root);
            ibt.insert(&idx_key, &[])?;
            current_root = ibt.root();
        }

        let idx_schema = crate::types::schema::IndexSchema {
            name: name.to_string(),
            table: table.to_string(),
            column: column.to_string(),
            root_page: current_root,
        };
        self.catalog.create_index(&mut self.pager, &idx_schema)?;
        Ok(ExecResult::Ok)
    }

    fn execute_drop_index(&mut self, name: &str) -> Result<ExecResult> {
        self.catalog.drop_index(&mut self.pager, name)?;
        Ok(ExecResult::Ok)
    }

    fn execute_insert(
        &mut self,
        table: &str,
        columns: Option<Vec<String>>,
        rows: Vec<Vec<Expr>>,
    ) -> Result<ExecResult> {
        let schema = self
            .catalog
            .get_table(&mut self.pager, table)?
            .ok_or_else(|| PlanError::NoSuchTable(table.to_string()))?;
        let indexes = self.catalog.list_indexes_for_table(&mut self.pager, table)?;

        let target_indices: Vec<usize> = match &columns {
            Some(names) => {
                let mut idxs = Vec::new();
                for n in names {
                    idxs.push(schema.column_index(n).ok_or_else(|| PlanError::NoSuchColumn(n.clone()))?);
                }
                idxs
            }
            None => (0..schema.columns.len()).collect(),
        };

        let mut count = 0usize;
        let mut table_root = schema.root_page;
        let mut index_roots: HashMap<String, u32> =
            indexes.iter().map(|i| (i.name.clone(), i.root_page)).collect();

        for row_exprs in rows {
            if row_exprs.len() != target_indices.len() {
                return Err(DbError::Exec(crate::error::ExecError::InvalidValue(
                    "value count does not match column count".into(),
                )));
            }
            let mut full_row = vec![Value::Null; schema.columns.len()];
            for (expr, &col_idx) in row_exprs.iter().zip(target_indices.iter()) {
                full_row[col_idx] = literal_to_value(expr)?;
            }

            let mut schema_for_write = schema.clone();
            schema_for_write.root_page = table_root;
            let indexes_for_write: Vec<IndexSchema> = indexes
                .iter()
                .cloned()
                .map(|mut idx| {
                    idx.root_page = index_roots[&idx.name];
                    idx
                })
                .collect();

            let (new_table_root, new_index_roots) =
                crate::exec::mutate::insert_row(&mut self.pager, &schema_for_write, &indexes_for_write, &full_row)?;
            table_root = new_table_root;
            for (name, root) in new_index_roots {
                index_roots.insert(name, root);
            }
            count += 1;
        }

        if table_root != schema.root_page {
            self.catalog.update_table_root(&mut self.pager, table, table_root)?;
        }
        for idx in &indexes {
            let new_root = index_roots[&idx.name];
            if new_root != idx.root_page {
                self.catalog.update_index_root(&mut self.pager, &idx.name, new_root)?;
            }
        }

        Ok(ExecResult::Modified(count))
    }

    fn execute_select(
        &mut self,
        columns: SelectColumns,
        table: &str,
        where_clause: Option<Expr>,
        order_by: Option<(String, bool)>,
        limit: Option<i64>,
    ) -> Result<ExecResult> {
        let schema = self
            .catalog
            .get_table(&mut self.pager, table)?
            .ok_or_else(|| PlanError::NoSuchTable(table.to_string()))?;
        let indexes = self.catalog.list_indexes_for_table(&mut self.pager, table)?;

        let (out_names, indices): (Vec<String>, Vec<usize>) = match &columns {
            SelectColumns::All => (
                schema.columns.iter().map(|c| c.name.clone()).collect(),
                (0..schema.columns.len()).collect(),
            ),
            SelectColumns::List(names) => {
                let mut idxs = Vec::new();
                for n in names {
                    idxs.push(schema.column_index(n).ok_or_else(|| PlanError::NoSuchColumn(n.clone()))?);
                }
                (names.clone(), idxs)
            }
        };

        let mut plan = crate::plan::planner::build_select_plan(&schema, &indexes, where_clause, indices, order_by, limit)?;
        let mut rows = Vec::new();
        while let Some(row) = plan.next(&mut self.pager)? {
            rows.push(row);
        }
        Ok(ExecResult::Rows { columns: out_names, rows })
    }

    fn execute_delete(&mut self, table: &str, where_clause: Option<Expr>) -> Result<ExecResult> {
        let schema = self
            .catalog
            .get_table(&mut self.pager, table)?
            .ok_or_else(|| PlanError::NoSuchTable(table.to_string()))?;
        let indexes = self.catalog.list_indexes_for_table(&mut self.pager, table)?;

        let all_columns: Vec<usize> = (0..schema.columns.len()).collect();
        let mut plan = crate::plan::planner::build_select_plan(&schema, &indexes, where_clause, all_columns, None, None)?;
        let mut rows_to_delete = Vec::new();
        while let Some(row) = plan.next(&mut self.pager)? {
            rows_to_delete.push(row);
        }

        let mut table_root = schema.root_page;
        let mut index_roots: HashMap<String, u32> =
            indexes.iter().map(|i| (i.name.clone(), i.root_page)).collect();
        let mut count = 0usize;

        for row in &rows_to_delete {
            let mut schema_for_write = schema.clone();
            schema_for_write.root_page = table_root;
            let indexes_for_write: Vec<IndexSchema> = indexes
                .iter()
                .cloned()
                .map(|mut idx| {
                    idx.root_page = index_roots[&idx.name];
                    idx
                })
                .collect();

            let (new_table_root, new_index_roots) =
                crate::exec::mutate::delete_row(&mut self.pager, &schema_for_write, &indexes_for_write, row)?;
            table_root = new_table_root;
            for (name, root) in new_index_roots {
                index_roots.insert(name, root);
            }
            count += 1;
        }

        if table_root != schema.root_page {
            self.catalog.update_table_root(&mut self.pager, table, table_root)?;
        }
        for idx in &indexes {
            let new_root = index_roots[&idx.name];
            if new_root != idx.root_page {
                self.catalog.update_index_root(&mut self.pager, &idx.name, new_root)?;
            }
        }

        Ok(ExecResult::Modified(count))
    }

    fn execute_update(
        &mut self,
        table: &str,
        assignments: Vec<(String, Expr)>,
        where_clause: Option<Expr>,
    ) -> Result<ExecResult> {
        let schema = self
            .catalog
            .get_table(&mut self.pager, table)?
            .ok_or_else(|| PlanError::NoSuchTable(table.to_string()))?;
        let indexes = self.catalog.list_indexes_for_table(&mut self.pager, table)?;

        let mut assignment_indices = Vec::new();
        for (name, expr) in assignments {
            let idx = schema.column_index(&name).ok_or_else(|| PlanError::NoSuchColumn(name.clone()))?;
            assignment_indices.push((idx, expr));
        }

        let all_columns: Vec<usize> = (0..schema.columns.len()).collect();
        let mut plan = crate::plan::planner::build_select_plan(&schema, &indexes, where_clause, all_columns, None, None)?;
        let mut old_rows = Vec::new();
        while let Some(row) = plan.next(&mut self.pager)? {
            old_rows.push(row);
        }

        let mut table_root = schema.root_page;
        let mut index_roots: HashMap<String, u32> =
            indexes.iter().map(|i| (i.name.clone(), i.root_page)).collect();
        let mut count = 0usize;

        for old_row in &old_rows {
            let mut new_row = old_row.clone();
            for (idx, expr) in &assignment_indices {
                new_row[*idx] =
                    crate::plan::expr::eval(expr, &schema, old_row).map_err(DbError::Plan)?;
            }

            let mut schema_for_write = schema.clone();
            schema_for_write.root_page = table_root;
            let indexes_for_write: Vec<IndexSchema> = indexes
                .iter()
                .cloned()
                .map(|mut idx| {
                    idx.root_page = index_roots[&idx.name];
                    idx
                })
                .collect();

            let (new_table_root, new_index_roots) = crate::exec::mutate::update_row(
                &mut self.pager,
                &schema_for_write,
                &indexes_for_write,
                old_row,
                &new_row,
            )?;
            table_root = new_table_root;
            for (name, root) in new_index_roots {
                index_roots.insert(name, root);
            }
            count += 1;
        }

        if table_root != schema.root_page {
            self.catalog.update_table_root(&mut self.pager, table, table_root)?;
        }
        for idx in &indexes {
            let new_root = index_roots[&idx.name];
            if new_root != idx.root_page {
                self.catalog.update_index_root(&mut self.pager, &idx.name, new_root)?;
            }
        }

        Ok(ExecResult::Modified(count))
    }

    pub fn list_tables(&mut self) -> Vec<String> {
        self.catalog.list_tables(&mut self.pager).unwrap_or_default()
    }

    pub fn table_schema(&mut self, name: &str) -> Option<TableSchema> {
        self.catalog.get_table(&mut self.pager, name).ok().flatten()
    }

    pub fn list_indexes(&mut self, table: &str) -> Vec<crate::types::schema::IndexSchema> {
        self.catalog.list_indexes_for_table(&mut self.pager, table).unwrap_or_default()
    }
}

fn literal_to_value(expr: &Expr) -> Result<Value> {
    match expr {
        Expr::IntLiteral(n) => Ok(Value::Integer(*n)),
        Expr::StringLiteral(s) => Ok(Value::Text(s.clone())),
        Expr::BoolLiteral(b) => Ok(Value::Boolean(*b)),
        Expr::Null => Ok(Value::Null),
        other => Err(DbError::Exec(crate::error::ExecError::InvalidValue(format!(
            "expected a literal value in INSERT, found {other:?}"
        )))),
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

    #[test]
    fn insert_then_reinsert_same_pk_errors() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)").unwrap();
        assert_eq!(
            db.execute("INSERT INTO t (id, name) VALUES (1, 'a')").unwrap(),
            ExecResult::Modified(1)
        );
        assert!(db.execute("INSERT INTO t (id, name) VALUES (1, 'b')").is_err());
    }

    #[test]
    fn insert_many_rows_forces_table_split_and_still_works() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)").unwrap();
        for i in 0..500 {
            let sql = format!("INSERT INTO t (id) VALUES ({i})");
            assert_eq!(db.execute(&sql).unwrap(), ExecResult::Modified(1));
        }
    }

    #[test]
    fn select_with_where_and_projection() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)").unwrap();
        db.execute("INSERT INTO t (id, name) VALUES (1, 'a'), (2, 'b'), (3, 'c')").unwrap();

        let result = db.execute("SELECT name FROM t WHERE id > 1").unwrap();
        match result {
            ExecResult::Rows { columns, rows } => {
                assert_eq!(columns, vec!["name".to_string()]);
                assert_eq!(rows, vec![vec![Value::Text("b".into())], vec![Value::Text("c".into())]]);
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[test]
    fn select_with_order_by_and_limit() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, score INTEGER)").unwrap();
        db.execute("INSERT INTO t (id, score) VALUES (1, 30), (2, 10), (3, 20)").unwrap();

        let result = db.execute("SELECT id FROM t ORDER BY score LIMIT 2").unwrap();
        match result {
            ExecResult::Rows { rows, .. } => {
                assert_eq!(rows, vec![vec![Value::Integer(2)], vec![Value::Integer(3)]]);
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[test]
    fn delete_removes_matching_rows_only() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)").unwrap();
        db.execute("INSERT INTO t (id) VALUES (1), (2), (3)").unwrap();

        assert_eq!(db.execute("DELETE FROM t WHERE id = 2").unwrap(), ExecResult::Modified(1));

        let result = db.execute("SELECT id FROM t").unwrap();
        match result {
            ExecResult::Rows { rows, .. } => {
                let mut remaining: Vec<i64> = rows.iter().map(|r| match &r[0] { Value::Integer(n) => *n, _ => unreachable!() }).collect();
                remaining.sort();
                assert_eq!(remaining, vec![1, 3]);
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[test]
    fn update_changes_matching_rows_only() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)").unwrap();
        db.execute("INSERT INTO t (id, name) VALUES (1, 'a'), (2, 'b')").unwrap();

        assert_eq!(db.execute("UPDATE t SET name = 'z' WHERE id = 1").unwrap(), ExecResult::Modified(1));

        let result = db.execute("SELECT id, name FROM t WHERE id = 1").unwrap();
        match result {
            ExecResult::Rows { rows, .. } => {
                assert_eq!(rows, vec![vec![Value::Integer(1), Value::Text("z".into())]]);
            }
            other => panic!("unexpected result: {other:?}"),
        }
        let unchanged = db.execute("SELECT name FROM t WHERE id = 2").unwrap();
        match unchanged {
            ExecResult::Rows { rows, .. } => assert_eq!(rows, vec![vec![Value::Text("b".into())]]),
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[test]
    fn create_index_on_existing_rows_then_drop_it() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT NOT NULL)").unwrap();
        db.execute("INSERT INTO t (id, name) VALUES (1, 'a'), (2, 'b')").unwrap();

        assert_eq!(db.execute("CREATE INDEX idx_name ON t (name)").unwrap(), ExecResult::Ok);
        assert_eq!(db.execute("DROP INDEX idx_name").unwrap(), ExecResult::Ok);
    }

    #[test]
    fn create_index_on_nullable_column_errors() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)").unwrap();
        let err = db.execute("CREATE INDEX idx_name ON t (name)").unwrap_err();
        assert!(matches!(err, DbError::Plan(PlanError::InvalidSchema(_))));
    }

    #[test]
    fn select_on_indexed_column_uses_index_and_returns_correct_rows() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT NOT NULL)").unwrap();
        db.execute("INSERT INTO t (id, name) VALUES (1, 'a'), (2, 'b'), (3, 'a')").unwrap();
        db.execute("CREATE INDEX idx_name ON t (name)").unwrap();

        let result = db.execute("SELECT id FROM t WHERE name = 'a'").unwrap();
        match result {
            ExecResult::Rows { rows, .. } => {
                let mut ids: Vec<i64> = rows.iter().map(|r| match &r[0] { Value::Integer(n) => *n, _ => unreachable!() }).collect();
                ids.sort();
                assert_eq!(ids, vec![1, 3]);
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }
}
