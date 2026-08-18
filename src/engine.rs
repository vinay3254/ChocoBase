use std::path::Path;
use std::collections::HashMap;

use crate::btree::node::LeafNode;
use crate::catalog::Catalog;
use crate::error::{DbError, PlanError, Result};
use crate::exec::Operator;
use crate::sql::ast::{ColumnDef, Statement, Expr, SelectColumns, SelectItem};
use crate::storage::pager::Pager;
use crate::types::schema::{Column, TableSchema, IndexSchema};
use crate::types::value::{ColumnType, Value};

use crate::storage::lock::LockFile;
use crate::storage::lock_manager::{LockManager, LockToken};
use std::sync::{Arc, Mutex};

pub struct Database {
    pager: Pager,
    catalog: Catalog,
    _lock: LockFile,
    pub change_tx: Option<tokio::sync::broadcast::Sender<crate::server::protocol::ChangeEvent>>,
}

/// Thread-safe facade for sharing one embedded database between client threads.
/// Each facade instance represents a logical session; clones share storage but
/// maintain independent explicit transaction state.
pub struct SharedDatabase {
    db: Arc<Mutex<Database>>,
    locks: Arc<LockManager>,
    transaction: Mutex<Option<LockToken>>,
    change_tx: tokio::sync::broadcast::Sender<crate::server::protocol::ChangeEvent>,
}

impl Clone for SharedDatabase {
    fn clone(&self) -> Self {
        Self {
            db: Arc::clone(&self.db),
            locks: Arc::clone(&self.locks),
            transaction: Mutex::new(None),
            change_tx: self.change_tx.clone(),
        }
    }
}

impl SharedDatabase {
    pub fn create(path: &Path) -> Result<Self> {
        let (tx, _) = tokio::sync::broadcast::channel(1024);
        let mut db = Database::create(path)?;
        db.change_tx = Some(tx.clone());
        Ok(Self {
            db: Arc::new(Mutex::new(db)),
            locks: LockManager::new(),
            transaction: Mutex::new(None),
            change_tx: tx,
        })
    }

    pub fn open(path: &Path) -> Result<Self> {
        let (tx, _) = tokio::sync::broadcast::channel(1024);
        let mut db = Database::open(path)?;
        db.change_tx = Some(tx.clone());
        Ok(Self {
            db: Arc::new(Mutex::new(db)),
            locks: LockManager::new(),
            transaction: Mutex::new(None),
            change_tx: tx,
        })
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<crate::server::protocol::ChangeEvent> {
        self.change_tx.subscribe()
    }

    pub fn execute(&self, sql: &str) -> Result<ExecResult> {
        let stmt = crate::sql::parser::parse(sql)?;
        match stmt {
            Statement::Begin => {
                let token = self.locks.begin();
                token.exclusive("database");
                let mut db = self.db.lock().unwrap();
                let result = db.execute(sql);
                if result.is_ok() {
                    *self.transaction.lock().unwrap() = Some(token);
                }
                result
            }
            Statement::Commit | Statement::Rollback => {
                let token = self.transaction.lock().unwrap().take();
                let mut db = self.db.lock().unwrap();
                let result = db.execute(sql);
                drop(token);
                result
            }
            Statement::Select { .. } => {
                if self.transaction.lock().unwrap().is_some() {
                    self.db.lock().unwrap().execute(sql)
                } else {
                    let token = self.locks.begin();
                    token.shared("database");
                    self.db.lock().unwrap().execute(sql)
                }
            }
            _ => {
                if self.transaction.lock().unwrap().is_some() {
                    self.db.lock().unwrap().execute(sql)
                } else {
                    let token = self.locks.begin();
                    token.exclusive("database");
                    self.db.lock().unwrap().execute(sql)
                }
            }
        }
    }

    pub fn list_tables(&self) -> Vec<String> {
        self.db.lock().unwrap().list_tables()
    }

    pub fn table_schema(&self, name: &str) -> Option<TableSchema> {
        self.db.lock().unwrap().table_schema(name)
    }

    pub fn list_indexes(&self, table: &str) -> Vec<IndexSchema> {
        self.db.lock().unwrap().list_indexes(table)
    }

    pub fn pager_stats(&self) -> crate::storage::pager::PagerStats {
        self.db.lock().unwrap().pager_stats()
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ExecResult {
    Rows { columns: Vec<String>, rows: Vec<Vec<Value>> },
    Modified(usize),
    Ok,
}

impl Database {
    pub fn create(path: &Path) -> Result<Self> {
        let lock = LockFile::acquire(path)?;
        let mut pager = Pager::create(path)?;
        let catalog = Catalog::bootstrap(&mut pager)?;
        pager.flush()?;
        Ok(Database { pager, catalog, _lock: lock, change_tx: None })
    }

    pub fn open(path: &Path) -> Result<Self> {
        let lock = LockFile::acquire(path)?;
        let mut pager = Pager::open(path)?;
        let catalog = Catalog::bootstrap(&mut pager)?;
        pager.flush()?;
        Ok(Database { pager, catalog, _lock: lock, change_tx: None })
    }

    pub fn execute(&mut self, sql: &str) -> Result<ExecResult> {
        let stmt = crate::sql::parser::parse(sql)?;
        match stmt {
            Statement::Begin => {
                if self.pager.in_transaction() {
                    return Err(DbError::Plan(PlanError::NestedTransactionNotSupported));
                }
                self.pager.begin_transaction()?;
                Ok(ExecResult::Ok)
            }
            Statement::Commit => {
                if !self.pager.in_transaction() {
                    return Err(DbError::Plan(PlanError::NoTransactionInProgress));
                }
                self.pager.commit_transaction()?;
                Ok(ExecResult::Ok)
            }
            Statement::Rollback => {
                if !self.pager.in_transaction() {
                    return Err(DbError::Plan(PlanError::NoTransactionInProgress));
                }
                self.pager.rollback_transaction()?;
                self.catalog = Catalog::bootstrap(&mut self.pager)?;
                Ok(ExecResult::Ok)
            }
            Statement::Select {
                columns,
                table,
                table_ref,
                where_clause,
                group_by,
                having,
                order_by,
                limit,
            } => {
                self.execute_select(columns, &table, table_ref, where_clause, group_by, having, order_by, limit)
            }
            mutating_stmt => {
                if self.pager.in_transaction() {
                    self.execute_mutating(mutating_stmt)
                } else {
                    self.pager.begin_transaction()?;
                    match self.execute_mutating(mutating_stmt) {
                        Ok(res) => {
                            self.pager.commit_transaction()?;
                            Ok(res)
                        }
                        Err(e) => {
                            let _ = self.pager.rollback_transaction();
                            let _ = Catalog::bootstrap(&mut self.pager).map(|cat| self.catalog = cat);
                            Err(e)
                        }
                    }
                }
            }
        }
    }

    fn execute_mutating(&mut self, stmt: Statement) -> Result<ExecResult> {
        match stmt {
            Statement::CreateTable { name, columns } => self.execute_create_table(name, columns),
            Statement::DropTable { name } => self.execute_drop_table(&name),
            Statement::CreateIndex { name, table, column } => self.execute_create_index(&name, &table, &column),
            Statement::DropIndex { name } => self.execute_drop_index(&name),
            Statement::Insert { table, columns, rows } => self.execute_insert(&table, columns, rows),
            Statement::Delete { table, where_clause } => self.execute_delete(&table, where_clause),
            Statement::Update { table, assignments, where_clause } => self.execute_update(&table, assignments, where_clause),
            _ => unreachable!(),
        }
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

        let mut table_root = schema.root_page;
        let mut index_roots: HashMap<String, u32> =
            indexes.iter().map(|i| (i.name.clone(), i.root_page)).collect();
        let mut count = 0usize;

        for expr_row in &rows {
            let mut full_row = vec![Value::Null; schema.columns.len()];
            match &columns {
                Some(col_names) => {
                    if col_names.len() != expr_row.len() {
                        return Err(DbError::Plan(PlanError::ColumnCountMismatch {
                            expected: col_names.len(),
                            found: expr_row.len(),
                        }));
                    }
                    for (cname, expr) in col_names.iter().zip(expr_row.iter()) {
                        let idx = schema
                            .column_index(cname)
                            .ok_or_else(|| PlanError::NoSuchColumn(cname.clone()))?;
                        full_row[idx] = literal_to_value_typed(expr, Some(&schema.columns[idx].ty))?;
                    }
                }
                None => {
                    if expr_row.len() != schema.columns.len() {
                        return Err(DbError::Plan(PlanError::ColumnCountMismatch {
                            expected: schema.columns.len(),
                            found: expr_row.len(),
                        }));
                    }
                    for (i, expr) in expr_row.iter().enumerate() {
                        full_row[i] = literal_to_value_typed(expr, Some(&schema.columns[i].ty))?;
                    }
                }
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
            if let Some(tx) = &self.change_tx {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let _ = tx.send(crate::server::protocol::ChangeEvent {
                    table: table.to_string(),
                    action: crate::server::protocol::ChangeAction::Insert,
                    old_row: None,
                    new_row: Some(full_row.clone()),
                    timestamp_ms: now,
                });
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
        for (col_name, expr) in &assignments {
            let idx = schema
                .column_index(col_name)
                .ok_or_else(|| PlanError::NoSuchColumn(col_name.clone()))?;
            assignment_indices.push((idx, expr.clone()));
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
            if let Some(tx) = &self.change_tx {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let _ = tx.send(crate::server::protocol::ChangeEvent {
                    table: table.to_string(),
                    action: crate::server::protocol::ChangeAction::Update,
                    old_row: Some(old_row.clone()),
                    new_row: Some(new_row.clone()),
                    timestamp_ms: now,
                });
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
        table_ref: Option<crate::sql::ast::TableRef>,
        where_clause: Option<Expr>,
        _group_by: Option<Vec<Expr>>,
        _having: Option<Expr>,
        order_by: Option<(String, bool)>,
        limit: Option<i64>,
    ) -> Result<ExecResult> {
        let is_join = match &table_ref {
            Some(crate::sql::ast::TableRef::Join { .. }) => true,
            _ => false,
        };

        if is_join {
            let tref = table_ref.unwrap();
            let (mut plan, schema) = crate::plan::planner::build_table_ref_plan(&mut self.catalog, &mut self.pager, &tref)?;
            if let Some(predicate) = where_clause {
                plan = Box::new(crate::exec::filter::Filter { input: plan, schema: schema.clone(), predicate });
            }

            let (out_names, indices) = match &columns {
                SelectColumns::All => (
                    schema.columns.iter().map(|c| c.name.clone()).collect(),
                    (0..schema.columns.len()).collect(),
                ),
                SelectColumns::List(names) => {
                    let mut idxs = Vec::new();
                    for n in names {
                        let idx = schema.column_index(n).ok_or_else(|| PlanError::NoSuchColumn(n.clone()))?;
                        idxs.push(idx);
                    }
                    (names.clone(), idxs)
                }
                SelectColumns::Items(items) => {
                    let mut names = Vec::new();
                    let mut exprs = Vec::new();
                    for item in items {
                        match item {
                            SelectItem::All => {
                                for col in &schema.columns {
                                    names.push(col.name.clone());
                                    exprs.push(Expr::Column(col.name.clone()));
                                }
                            }
                            SelectItem::Expr { expr, alias } => {
                                let name = alias.clone().unwrap_or_else(|| match expr {
                                    Expr::Column(c) => c.clone(),
                                    Expr::QualifiedColumn { table, column } => format!("{table}.{column}"),
                                    Expr::JsonExtract { path, .. } => path.clone(),
                                    _ => "expr".into(),
                                });
                                names.push(name);
                                exprs.push(expr.clone());
                            }
                        }
                    }

                    if let Some((col, desc)) = order_by {
                        let idx = schema.column_index(&col).ok_or_else(|| PlanError::NoSuchColumn(col))?;
                        plan = Box::new(crate::exec::sort::Sort::new(plan, idx, desc));
                    }
                    plan = Box::new(crate::exec::project::ProjectExpr { input: plan, schema: schema.clone(), exprs });
                    if let Some(n) = limit {
                        plan = Box::new(crate::exec::limit::Limit::new(plan, n));
                    }

                    let mut rows = Vec::new();
                    while let Some(row) = plan.next(&mut self.pager)? {
                        rows.push(row);
                    }
                    return Ok(ExecResult::Rows { columns: names, rows });
                }
            };

            if let Some((col, desc)) = order_by {
                let idx = schema.column_index(&col).ok_or_else(|| PlanError::NoSuchColumn(col))?;
                plan = Box::new(crate::exec::sort::Sort::new(plan, idx, desc));
            }
            plan = Box::new(crate::exec::project::Project { input: plan, indices });
            if let Some(n) = limit {
                plan = Box::new(crate::exec::limit::Limit::new(plan, n));
            }

            let mut rows = Vec::new();
            while let Some(row) = plan.next(&mut self.pager)? {
                rows.push(row);
            }
            return Ok(ExecResult::Rows { columns: out_names, rows });
        }

        let schema = self
            .catalog
            .get_table(&mut self.pager, table)?
            .ok_or_else(|| PlanError::NoSuchTable(table.to_string()))?;
        let indexes = self.catalog.list_indexes_for_table(&mut self.pager, table)?;

        let has_group_by = _group_by.is_some();
        let has_agg = match &columns {
            SelectColumns::Items(items) => items.iter().any(|item| match item {
                SelectItem::Expr { expr: Expr::Aggregate(_), .. } => true,
                _ => false,
            }),
            _ => false,
        };

        if has_group_by || has_agg {
            let mut aggregates = Vec::new();
            let mut out_names = Vec::new();
            let mut group_exprs = _group_by.clone().unwrap_or_default();

            if let SelectColumns::Items(items) = &columns {
                for item in items {
                    match item {
                        SelectItem::Expr { expr: Expr::Aggregate(func), alias } => {
                            aggregates.push(func.clone());
                            let name = alias.clone().unwrap_or_else(|| match func {
                                crate::sql::ast::AggregateFunc::CountStar => "count(*)".into(),
                                crate::sql::ast::AggregateFunc::Count(_) => "count".into(),
                                crate::sql::ast::AggregateFunc::Sum(_) => "sum".into(),
                                crate::sql::ast::AggregateFunc::Avg(_) => "avg".into(),
                                crate::sql::ast::AggregateFunc::Min(_) => "min".into(),
                                crate::sql::ast::AggregateFunc::Max(_) => "max".into(),
                            });
                            out_names.push(name);
                        }
                        SelectItem::Expr { expr, alias } => {
                            let name = alias.clone().unwrap_or_else(|| match expr {
                                Expr::Column(c) => c.clone(),
                                Expr::QualifiedColumn { column, .. } => column.clone(),
                                _ => "expr".into(),
                            });
                            out_names.push(name);
                            if !has_group_by && !group_exprs.contains(expr) {
                                group_exprs.push(expr.clone());
                            }
                        }
                        SelectItem::All => return Err(DbError::Plan(PlanError::InvalidExpression("cannot use SELECT * with aggregations".into()))),
                    }
                }
            }

            let seq_scan: Box<dyn Operator> = Box::new(crate::exec::scan::SeqScan::new(schema.clone()));
            let scan_plan = if let Some(predicate) = where_clause {
                Box::new(crate::exec::filter::Filter { input: seq_scan, schema: schema.clone(), predicate })
            } else {
                seq_scan
            };

            let mut plan: Box<dyn Operator> = Box::new(crate::exec::aggregate::AggregateOperator::new(
                scan_plan,
                schema.clone(),
                if group_exprs.is_empty() { None } else { Some(group_exprs) },
                aggregates,
                _having,
            ));

            if let Some(n) = limit {
                plan = Box::new(crate::exec::limit::Limit::new(plan, n));
            }

            let mut rows = Vec::new();
            while let Some(row) = plan.next(&mut self.pager)? {
                rows.push(row);
            }
            return Ok(ExecResult::Rows { columns: out_names, rows });
        }

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
            SelectColumns::Items(items) => {
                let mut names = Vec::new();
                let mut exprs = Vec::new();
                for item in items {
                    match item {
                        SelectItem::All => {
                            for col in &schema.columns {
                                names.push(col.name.clone());
                                exprs.push(Expr::Column(col.name.clone()));
                            }
                        }
                        SelectItem::Expr { expr, alias } => {
                            let name = alias.clone().unwrap_or_else(|| match expr {
                                Expr::Column(c) => c.clone(),
                                Expr::QualifiedColumn { column, .. } => column.clone(),
                                Expr::JsonExtract { path, .. } => path.clone(),
                                _ => "expr".into(),
                            });
                            names.push(name);
                            exprs.push(expr.clone());
                        }
                    }
                }

                let all_columns: Vec<usize> = (0..schema.columns.len()).collect();
                let mut plan = crate::plan::planner::build_select_plan(&schema, &indexes, where_clause, all_columns, order_by, None)?;
                plan = Box::new(crate::exec::project::ProjectExpr { input: plan, schema: schema.clone(), exprs });
                if let Some(n) = limit {
                    plan = Box::new(crate::exec::limit::Limit::new(plan, n));
                }
                let mut rows = Vec::new();
                while let Some(row) = plan.next(&mut self.pager)? {
                    rows.push(row);
                }
                return Ok(ExecResult::Rows { columns: names, rows });
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
            if let Some(tx) = &self.change_tx {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let _ = tx.send(crate::server::protocol::ChangeEvent {
                    table: table.to_string(),
                    action: crate::server::protocol::ChangeAction::Delete,
                    old_row: Some(row.clone()),
                    new_row: None,
                    timestamp_ms: now,
                });
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

    pub fn dump_table_btree(&mut self, table: &str) -> Option<String> {
        let schema = self.table_schema(table)?;
        let mut bt = crate::btree::tree::BTree::new(&mut self.pager, schema.root_page);
        Some(bt.dump())
    }

    pub fn pager_stats(&self) -> crate::storage::pager::PagerStats {
        self.pager.stats()
    }

    pub fn reset_read_counter(&mut self) {
        self.pager.reset_read_counter();
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        if self.pager.in_transaction() {
            let _ = self.pager.rollback_transaction();
        }
    }
}

#[allow(dead_code)]
fn literal_to_value(expr: &Expr) -> Result<Value> {
    literal_to_value_typed(expr, None)
}

fn literal_to_value_typed(expr: &Expr, target_type: Option<&ColumnType>) -> Result<Value> {
    match expr {
        Expr::IntLiteral(n) => Ok(Value::Integer(*n)),
        Expr::StringLiteral(s) => match target_type {
            Some(ColumnType::Json) => Ok(Value::Json(s.clone())),
            _ => Ok(Value::Text(s.clone())),
        },
        Expr::BoolLiteral(b) => Ok(Value::Boolean(*b)),
        Expr::Null => Ok(Value::Null),
        other => Err(DbError::Exec(crate::error::ExecError::InvalidValue(format!(
            "expected a literal value in statement, found {other:?}"
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

    #[test]
    fn transaction_commit_persists_changes() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path();
        {
            let mut db = Database::create(path).unwrap();
            db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT)").unwrap();
            db.execute("BEGIN").unwrap();
            db.execute("INSERT INTO t (id, val) VALUES (1, 'committed')").unwrap();
            db.execute("COMMIT").unwrap();
        }
        let mut db = Database::open(path).unwrap();
        let res = db.execute("SELECT val FROM t WHERE id = 1").unwrap();
        match res {
            ExecResult::Rows { rows, .. } => assert_eq!(rows, vec![vec![Value::Text("committed".into())]]),
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[test]
    fn transaction_rollback_discards_mutations() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path();
        let mut db = Database::create(path).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT)").unwrap();
        db.execute("INSERT INTO t (id, val) VALUES (1, 'initial')").unwrap();

        db.execute("BEGIN").unwrap();
        db.execute("UPDATE t SET val = 'modified' WHERE id = 1").unwrap();
        db.execute("INSERT INTO t (id, val) VALUES (2, 'new')").unwrap();
        db.execute("ROLLBACK").unwrap();

        let res = db.execute("SELECT val FROM t WHERE id = 1").unwrap();
        match res {
            ExecResult::Rows { rows, .. } => assert_eq!(rows, vec![vec![Value::Text("initial".into())]]),
            other => panic!("unexpected result: {other:?}"),
        }
        let res2 = db.execute("SELECT * FROM t WHERE id = 2").unwrap();
        match res2 {
            ExecResult::Rows { rows, .. } => assert!(rows.is_empty()),
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[test]
    fn transaction_rollback_reverts_ddl_and_catalog() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path();
        let mut db = Database::create(path).unwrap();
        db.execute("CREATE TABLE t1 (id INTEGER PRIMARY KEY)").unwrap();

        db.execute("BEGIN").unwrap();
        db.execute("CREATE TABLE t2 (id INTEGER PRIMARY KEY)").unwrap();
        db.execute("DROP TABLE t1").unwrap();
        db.execute("ROLLBACK").unwrap();

        assert_eq!(db.list_tables(), vec!["t1".to_string()]);
    }

    #[test]
    fn nested_begin_and_naked_commit_error() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        assert!(matches!(db.execute("COMMIT").unwrap_err(), DbError::Plan(PlanError::NoTransactionInProgress)));
        assert!(matches!(db.execute("ROLLBACK").unwrap_err(), DbError::Plan(PlanError::NoTransactionInProgress)));

        db.execute("BEGIN").unwrap();
        assert!(matches!(db.execute("BEGIN").unwrap_err(), DbError::Plan(PlanError::NestedTransactionNotSupported)));
        db.execute("ROLLBACK").unwrap();
    }

    #[test]
    fn lock_file_reclaims_dead_pid() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path();
        {
            let _db = Database::create(path).unwrap();
        }

        // Simulate a stale lock file written by a non-existent dead PID
        let lock_path = crate::storage::lock::lock_path_for(path);
        std::fs::write(&lock_path, "99999999").unwrap();

        // Database::open must detect that PID 99999999 is dead and successfully reclaim the lock
        let mut db = Database::open(path).unwrap();
        assert_eq!(db.execute("SELECT 1 FROM non_existent").is_err(), true);

        // Verify the lock file now contains the current process's PID
        let lock_contents = std::fs::read_to_string(&lock_path).unwrap();
        assert_eq!(lock_contents.trim(), std::process::id().to_string());
    }

    #[test]
    fn active_lock_prevents_second_open() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path();
        {
            let _db1 = Database::create(path).unwrap();
        }

        // Spawn a background dummy child process so we have a guaranteed live foreign PID
        let mut child = if cfg!(windows) {
            std::process::Command::new("powershell")
                .arg("-Command")
                .arg("Start-Sleep -Seconds 5")
                .spawn()
                .unwrap()
        } else {
            std::process::Command::new("sleep")
                .arg("5")
                .spawn()
                .unwrap()
        };
        let foreign_pid = child.id();

        // Write foreign alive PID to lock file
        let lock_path = crate::storage::lock::lock_path_for(path);
        std::fs::write(&lock_path, foreign_pid.to_string()).unwrap();

        // Attempting to open the database simultaneously must fail with DatabaseLocked
        match Database::open(path) {
            Err(DbError::Storage(crate::error::StorageError::DatabaseLocked(_))) => (),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("expected DatabaseLocked, got error: {e:?}");
            }
            Ok(_) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("expected DatabaseLocked, got Ok");
            }
        }

        let _ = child.kill();
        let _ = child.wait();
    }
}
