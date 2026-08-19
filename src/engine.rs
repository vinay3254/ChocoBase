use std::collections::HashMap;
use std::path::Path;

use crate::btree::node::LeafNode;
use crate::catalog::Catalog;
use crate::error::{DbError, ExecError, PlanError, Result};
use crate::exec::Operator;
use crate::sql::ast::{BinOp, ColumnDef, Expr, SelectColumns, SelectItem, Statement};
use crate::storage::pager::Pager;
use crate::types::schema::{Column, IndexSchema, TableSchema};
use crate::types::value::{ColumnType, Value};

use crate::auth::ExecutionContext;
use crate::storage::lock::LockFile;
use crate::storage::lock_manager::{LockManager, LockToken};
use std::sync::{Arc, Mutex};

pub struct Database {
    pager: Pager,
    catalog: Catalog,
    _lock: LockFile,
    pub change_tx: Option<tokio::sync::broadcast::Sender<crate::server::protocol::ChangeEvent>>,
    transaction_events: Vec<crate::server::protocol::ChangeEvent>,
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

    pub fn subscribe(
        &self,
    ) -> tokio::sync::broadcast::Receiver<crate::server::protocol::ChangeEvent> {
        self.change_tx.subscribe()
    }

    pub fn with_db<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&mut Database) -> Result<R>,
    {
        let mut db = self.db.lock().unwrap();
        f(&mut db)
    }

    /// Explicitly rolls back any in-flight transaction held by this session upon client disconnection.
    pub fn rollback_on_disconnect(&self) {
        let token = self.transaction.lock().unwrap().take();
        if token.is_some() {
            let mut db = self.db.lock().unwrap();
            let _ = db.execute("ROLLBACK");
            drop(token);
        }
    }

    pub fn execute(&self, sql: &str) -> Result<ExecResult> {
        self.execute_with_context(sql, &crate::auth::ExecutionContext::admin())
    }

    pub fn execute_with_context(
        &self,
        sql: &str,
        ctx: &crate::auth::ExecutionContext,
    ) -> Result<ExecResult> {
        let stmt = crate::sql::parser::parse(sql)?;
        match stmt {
            Statement::Begin => {
                let token = self.locks.begin();
                token.exclusive("database");
                let mut db = self.db.lock().unwrap();
                let result = db.execute_with_context(sql, ctx);
                if result.is_ok() {
                    *self.transaction.lock().unwrap() = Some(token);
                }
                result
            }
            Statement::Commit | Statement::Rollback => {
                let token = self.transaction.lock().unwrap().take();
                let mut db = self.db.lock().unwrap();
                let result = db.execute_with_context(sql, ctx);
                drop(token);
                result
            }
            Statement::Select { .. } => {
                if self.transaction.lock().unwrap().is_some() {
                    self.db.lock().unwrap().execute_with_context(sql, ctx)
                } else {
                    let token = self.locks.begin();
                    token.shared("database");
                    self.db.lock().unwrap().execute_with_context(sql, ctx)
                }
            }
            _ => {
                if self.transaction.lock().unwrap().is_some() {
                    self.db.lock().unwrap().execute_with_context(sql, ctx)
                } else {
                    let token = self.locks.begin();
                    token.exclusive("database");
                    self.db.lock().unwrap().execute_with_context(sql, ctx)
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

    pub fn dump_sql(&self) -> Result<String> {
        let token = self.locks.begin();
        token.shared("database");
        let mut db = self.db.lock().unwrap();
        crate::backup::dump_database(&mut db)
    }

    pub fn restore_from_sql(&self, sql: &str) -> Result<usize> {
        let token = self.locks.begin();
        token.exclusive("database");
        let mut db = self.db.lock().unwrap();
        crate::backup::restore_database(&mut db, sql)
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ExecResult {
    Rows {
        columns: Vec<String>,
        rows: Vec<Vec<Value>>,
    },
    Modified(usize),
    Ok,
}

impl Database {
    pub fn create(path: &Path) -> Result<Self> {
        let lock = LockFile::acquire(path)?;
        let mut pager = Pager::create(path)?;
        let catalog = Catalog::bootstrap(&mut pager)?;
        pager.flush()?;
        let mut db = Database {
            pager,
            catalog,
            _lock: lock,
            change_tx: None,
            transaction_events: Vec::new(),
        };
        db.ensure_auth_table()?;
        Ok(db)
    }

    pub fn open(path: &Path) -> Result<Self> {
        let lock = LockFile::acquire(path)?;
        let mut pager = Pager::open(path)?;
        let catalog = Catalog::bootstrap(&mut pager)?;
        pager.flush()?;
        let mut db = Database {
            pager,
            catalog,
            _lock: lock,
            change_tx: None,
            transaction_events: Vec::new(),
        };
        db.ensure_auth_table()?;
        Ok(db)
    }

    pub fn ensure_auth_table(&mut self) -> Result<()> {
        if self.catalog.get_table(&mut self.pager, "_users")?.is_none() {
            let cols = vec![
                crate::types::schema::Column {
                    name: "id".into(),
                    ty: crate::types::value::ColumnType::Integer,
                    not_null: true,
                    is_primary_key: true,
                },
                crate::types::schema::Column {
                    name: "username".into(),
                    ty: crate::types::value::ColumnType::Text,
                    not_null: true,
                    is_primary_key: false,
                },
                crate::types::schema::Column {
                    name: "password_hash".into(),
                    ty: crate::types::value::ColumnType::Text,
                    not_null: true,
                    is_primary_key: false,
                },
                crate::types::schema::Column {
                    name: "role".into(),
                    ty: crate::types::value::ColumnType::Text,
                    not_null: true,
                    is_primary_key: false,
                },
            ];
            let root = self.pager.allocate_page()?;
            LeafNode {
                entries: vec![],
                next_leaf: 0,
            }
            .encode(self.pager.get_page_mut(root)?);
            let schema = TableSchema {
                name: "_users".into(),
                columns: cols,
                root_page: root,
                rls_enabled: false,
            };
            self.catalog.create_table(&mut self.pager, &schema)?;

            // Seed default postgres administrator user
            let default_pass = std::env::var("CHOCOBASE_POSTGRES_PASSWORD")
                .unwrap_or_else(|_| "postgres".to_string());
            let hash = crate::auth::hash_password(&default_pass);
            let row = vec![
                Value::Integer(1),
                Value::Text("postgres".into()),
                Value::Text(hash),
                Value::Text("admin".into()),
            ];
            let (new_root, _) =
                crate::exec::mutate::insert_row(&mut self.pager, &schema, &[], &row)?;
            if new_root != schema.root_page {
                self.catalog
                    .update_table_root(&mut self.pager, "_users", new_root)?;
            }
        }
        Ok(())
    }

    pub fn execute(&mut self, sql: &str) -> Result<ExecResult> {
        self.execute_with_context(sql, &crate::auth::ExecutionContext::admin())
    }

    pub fn execute_with_context(
        &mut self,
        sql: &str,
        ctx: &crate::auth::ExecutionContext,
    ) -> Result<ExecResult> {
        let stmt = crate::sql::parser::parse(sql)?;
        self.execute_statement_with_context(stmt, ctx)
    }

    pub fn execute_statement_with_context(
        &mut self,
        stmt: Statement,
        ctx: &crate::auth::ExecutionContext,
    ) -> Result<ExecResult> {
        match stmt {
            Statement::Begin => {
                if self.pager.in_transaction() {
                    return Err(DbError::Plan(PlanError::NestedTransactionNotSupported));
                }
                self.transaction_events.clear();
                self.pager.begin_transaction()?;
                Ok(ExecResult::Ok)
            }
            Statement::Commit => {
                if !self.pager.in_transaction() {
                    return Err(DbError::Plan(PlanError::NoTransactionInProgress));
                }
                self.pager.commit_transaction()?;
                if let Some(tx) = &self.change_tx {
                    for event in self.transaction_events.drain(..) {
                        let _ = tx.send(event);
                    }
                }
                Ok(ExecResult::Ok)
            }
            Statement::Rollback => {
                if !self.pager.in_transaction() {
                    return Err(DbError::Plan(PlanError::NoTransactionInProgress));
                }
                self.transaction_events.clear();
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
            } => self.execute_select(
                columns,
                &table,
                table_ref,
                where_clause,
                group_by,
                having,
                order_by,
                limit,
                ctx,
            ),
            Statement::Explain(inner_stmt) => {
                let plan_lines = self.explain_statement(&inner_stmt, ctx)?;
                let rows = plan_lines
                    .into_iter()
                    .map(|line| vec![Value::Text(line)])
                    .collect();
                Ok(ExecResult::Rows {
                    columns: vec!["QUERY PLAN".to_string()],
                    rows,
                })
            }
            mutating_stmt => {
                if self.pager.in_transaction() {
                    self.execute_mutating(mutating_stmt, ctx)
                } else {
                    self.pager.begin_transaction()?;
                    match self.execute_mutating(mutating_stmt, ctx) {
                        Ok(res) => {
                            self.pager.commit_transaction()?;
                            if let Some(tx) = &self.change_tx {
                                for event in self.transaction_events.drain(..) {
                                    let _ = tx.send(event);
                                }
                            }
                            Ok(res)
                        }
                        Err(e) => {
                            self.transaction_events.clear();
                            let _ = self.pager.rollback_transaction();
                            let _ =
                                Catalog::bootstrap(&mut self.pager).map(|cat| self.catalog = cat);
                            Err(e)
                        }
                    }
                }
            }
        }
    }

    fn apply_rls_filter(
        &mut self,
        table: &str,
        cmd: crate::types::schema::PolicyCmd,
        user_where: Option<Expr>,
        ctx: &crate::auth::ExecutionContext,
    ) -> Result<Option<Expr>> {
        let schema = match self.catalog.get_table(&mut self.pager, table)? {
            Some(s) => s,
            None => return Ok(user_where),
        };
        if !schema.rls_enabled || ctx.is_admin {
            return Ok(user_where);
        }

        let policies = self
            .catalog
            .list_policies_for_table(&mut self.pager, table)?;
        let matching: Vec<&crate::types::schema::PolicySchema> = policies
            .iter()
            .filter(|p| p.cmd == cmd || p.cmd == crate::types::schema::PolicyCmd::All)
            .collect();

        if matching.is_empty() {
            // Default deny: condition that is always false
            return Ok(Some(Expr::BinaryOp {
                op: BinOp::Eq,
                left: Box::new(Expr::IntLiteral(1)),
                right: Box::new(Expr::IntLiteral(0)),
            }));
        }

        let mut combined_using: Option<Expr> = None;
        for p in matching {
            if let Some(using) = &p.using_expr {
                combined_using = match combined_using {
                    Some(prev) => Some(Expr::BinaryOp {
                        op: BinOp::Or,
                        left: Box::new(prev),
                        right: Box::new(using.clone()),
                    }),
                    None => Some(using.clone()),
                };
            }
        }

        match (user_where, combined_using) {
            (Some(w), Some(p)) => Ok(Some(Expr::BinaryOp {
                op: BinOp::And,
                left: Box::new(w),
                right: Box::new(p),
            })),
            (None, Some(p)) => Ok(Some(p)),
            (Some(w), None) => Ok(Some(w)),
            (None, None) => Ok(None),
        }
    }

    pub(crate) fn resolve_subqueries_in_expr(
        &mut self,
        expr: &Expr,
        ctx: &crate::auth::ExecutionContext,
    ) -> Result<Expr> {
        match expr {
            Expr::InSubquery {
                expr: inner_expr,
                subquery,
                negated,
            } => {
                let resolved_inner = self.resolve_subqueries_in_expr(inner_expr, ctx)?;
                let res = self.execute_statement_with_context(*subquery.clone(), ctx)?;
                let mut list = Vec::new();
                if let ExecResult::Rows { rows, .. } = res {
                    for row in rows {
                        if let Some(first_val) = row.into_iter().next() {
                            let expr_lit = match first_val {
                                Value::Integer(i) => Expr::IntLiteral(i),
                                Value::Float(f) => Expr::FloatLiteral(f),
                                Value::Text(s) => Expr::StringLiteral(s),
                                Value::Boolean(b) => Expr::BoolLiteral(b),
                                Value::Json(j) => Expr::StringLiteral(j),
                                Value::Vector(v) => Expr::StringLiteral(
                                    serde_json::to_string(&v).unwrap_or_default(),
                                ),
                                Value::Null => Expr::Null,
                            };
                            list.push(expr_lit);
                        }
                    }
                }
                Ok(Expr::InList {
                    expr: Box::new(resolved_inner),
                    list,
                    negated: *negated,
                })
            }
            Expr::Exists { subquery, negated } => {
                let res = self.execute_statement_with_context(*subquery.clone(), ctx)?;
                let has_rows = match res {
                    ExecResult::Rows { rows, .. } => !rows.is_empty(),
                    _ => false,
                };
                let result_bool = if *negated { !has_rows } else { has_rows };
                Ok(Expr::BoolLiteral(result_bool))
            }
            Expr::BinaryOp { op, left, right } => {
                let resolved_left = self.resolve_subqueries_in_expr(left, ctx)?;
                let resolved_right = self.resolve_subqueries_in_expr(right, ctx)?;
                Ok(Expr::BinaryOp {
                    op: *op,
                    left: Box::new(resolved_left),
                    right: Box::new(resolved_right),
                })
            }
            Expr::IsNull {
                expr: inner,
                negated,
            } => {
                let resolved_inner = self.resolve_subqueries_in_expr(inner, ctx)?;
                Ok(Expr::IsNull {
                    expr: Box::new(resolved_inner),
                    negated: *negated,
                })
            }
            Expr::InList {
                expr: inner,
                list,
                negated,
            } => {
                let resolved_inner = self.resolve_subqueries_in_expr(inner, ctx)?;
                let mut resolved_list = Vec::with_capacity(list.len());
                for item in list {
                    resolved_list.push(self.resolve_subqueries_in_expr(item, ctx)?);
                }
                Ok(Expr::InList {
                    expr: Box::new(resolved_inner),
                    list: resolved_list,
                    negated: *negated,
                })
            }
            Expr::Like {
                expr: inner,
                pattern,
                negated,
            } => {
                let resolved_inner = self.resolve_subqueries_in_expr(inner, ctx)?;
                Ok(Expr::Like {
                    expr: Box::new(resolved_inner),
                    pattern: pattern.clone(),
                    negated: *negated,
                })
            }
            Expr::VectorDistance {
                metric,
                left,
                right,
            } => {
                let resolved_left = self.resolve_subqueries_in_expr(left, ctx)?;
                let resolved_right = self.resolve_subqueries_in_expr(right, ctx)?;
                Ok(Expr::VectorDistance {
                    metric: *metric,
                    left: Box::new(resolved_left),
                    right: Box::new(resolved_right),
                })
            }
            Expr::JsonExtract {
                expr: inner,
                path,
                as_text,
            } => {
                let resolved_inner = self.resolve_subqueries_in_expr(inner, ctx)?;
                Ok(Expr::JsonExtract {
                    expr: Box::new(resolved_inner),
                    path: path.clone(),
                    as_text: *as_text,
                })
            }
            Expr::FtsMatch { expr: inner, query } => {
                let resolved_inner = self.resolve_subqueries_in_expr(inner, ctx)?;
                Ok(Expr::FtsMatch {
                    expr: Box::new(resolved_inner),
                    query: query.clone(),
                })
            }
            Expr::FtsRank { expr: inner, query } => {
                let resolved_inner = self.resolve_subqueries_in_expr(inner, ctx)?;
                Ok(Expr::FtsRank {
                    expr: Box::new(resolved_inner),
                    query: query.clone(),
                })
            }
            other => Ok(other.clone()),
        }
    }

    fn execute_mutating(
        &mut self,
        stmt: Statement,
        ctx: &crate::auth::ExecutionContext,
    ) -> Result<ExecResult> {
        match stmt {
            Statement::CreateTable { name, columns } => self.execute_create_table(name, columns),
            Statement::DropTable { name } => self.execute_drop_table(&name),
            Statement::CreateIndex {
                name,
                table,
                column,
            } => self.execute_create_index(&name, &table, &column),
            Statement::DropIndex { name } => self.execute_drop_index(&name),
            Statement::Insert {
                table,
                columns,
                rows,
                returning,
            } => self.execute_insert(&table, columns, rows, returning, ctx),
            Statement::Delete {
                table,
                where_clause,
                returning,
            } => self.execute_delete(&table, where_clause, returning, ctx),
            Statement::Update {
                table,
                assignments,
                where_clause,
                returning,
            } => self.execute_update(&table, assignments, where_clause, returning, ctx),
            Statement::CreateUser {
                username,
                password,
                role,
            } => self.execute_create_user(username, password, role),
            Statement::AlterTableRls { table, enabled } => {
                self.execute_alter_table_rls(&table, enabled)
            }
            Statement::AlterTableAddColumn { table, column } => {
                self.execute_alter_table_add_column(&table, column)
            }
            Statement::AlterTableDropColumn { table, column } => {
                self.execute_alter_table_drop_column(&table, &column)
            }
            Statement::AlterTableRename { table, new_name } => {
                self.execute_alter_table_rename(&table, &new_name)
            }
            Statement::CreatePolicy {
                name,
                table,
                cmd,
                using_expr,
                with_check,
            } => {
                self.catalog.create_policy(
                    &mut self.pager,
                    &crate::types::schema::PolicySchema {
                        name,
                        table,
                        cmd,
                        using_expr,
                        with_check,
                    },
                )?;
                Ok(ExecResult::Ok)
            }
            Statement::DropPolicy { name, .. } => {
                self.catalog.drop_policy(&mut self.pager, &name)?;
                Ok(ExecResult::Ok)
            }
            _ => unreachable!(),
        }
    }

    fn execute_create_user(
        &mut self,
        username: String,
        password: String,
        role: Option<String>,
    ) -> Result<ExecResult> {
        self.ensure_auth_table()?;
        let role = role.unwrap_or_else(|| "user".into());
        let hash = crate::auth::hash_password(&password);

        let mut max_id = 0i64;
        let schema = self.catalog.get_table(&mut self.pager, "_users")?.unwrap();
        let mut scan = crate::exec::scan::SeqScan::new(schema.clone());
        while let Some(row) = scan.next(&mut self.pager)? {
            if let Value::Text(existing) = &row[1] {
                if existing == &username {
                    return Err(DbError::Exec(crate::error::ExecError::InvalidValue(
                        format!("user '{username}' already exists"),
                    )));
                }
            }
            if let Value::Integer(id) = row[0] {
                if id > max_id {
                    max_id = id;
                }
            }
        }

        let new_id = max_id + 1;
        let row = vec![
            Value::Integer(new_id),
            Value::Text(username),
            Value::Text(hash),
            Value::Text(role),
        ];
        let (new_root, _) = crate::exec::mutate::insert_row(&mut self.pager, &schema, &[], &row)?;
        if new_root != schema.root_page {
            self.catalog
                .update_table_root(&mut self.pager, "_users", new_root)?;
        }
        Ok(ExecResult::Modified(1))
    }

    fn execute_alter_table_rls(&mut self, table: &str, enabled: bool) -> Result<ExecResult> {
        self.catalog
            .set_table_rls(&mut self.pager, table, enabled)?;
        Ok(ExecResult::Ok)
    }

    fn execute_alter_table_add_column(
        &mut self,
        table_name: &str,
        col_def: crate::sql::ast::ColumnDef,
    ) -> Result<ExecResult> {
        let mut schema = self
            .catalog
            .get_table(&mut self.pager, table_name)?
            .ok_or_else(|| PlanError::NoSuchTable(table_name.to_string()))?;

        if schema.columns.iter().any(|c| c.name == col_def.name) {
            return Err(DbError::Plan(PlanError::NoSuchColumn(format!(
                "column '{}' already exists in table '{}'",
                col_def.name, table_name
            ))));
        }

        let new_col = crate::types::schema::Column {
            name: col_def.name,
            ty: col_def.ty,
            not_null: col_def.not_null,
            is_primary_key: col_def.primary_key,
        };

        // Scan all existing rows, append Value::Null, and re-write
        let indexes = self
            .catalog
            .list_indexes_for_table(&mut self.pager, table_name)?;
        let all_cols: Vec<usize> = (0..schema.columns.len()).collect();
        let mut plan = crate::plan::planner::build_select_plan_with_context(
            &schema,
            &indexes,
            None,
            all_cols,
            None,
            None,
            &crate::auth::ExecutionContext::admin(),
        )?;

        let mut existing_rows = Vec::new();
        while let Some(mut row) = plan.next(&mut self.pager)? {
            row.push(Value::Null);
            existing_rows.push(row);
        }

        // Allocate new root leaf page for migrated table
        let new_root = self.pager.allocate_page()?;
        crate::btree::node::LeafNode {
            entries: vec![],
            next_leaf: 0,
        }
        .encode(self.pager.get_page_mut(new_root)?);

        schema.columns.push(new_col);
        schema.root_page = new_root;

        let mut table_root = new_root;
        let mut index_roots: HashMap<String, u32> = indexes
            .iter()
            .map(|i| (i.name.clone(), i.root_page))
            .collect();

        for row in existing_rows {
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

            let (next_table_root, next_index_roots) = crate::exec::mutate::insert_row(
                &mut self.pager,
                &schema_for_write,
                &indexes_for_write,
                &row,
            )?;
            table_root = next_table_root;
            for (name, root) in next_index_roots {
                index_roots.insert(name, root);
            }
        }

        schema.root_page = table_root;
        self.catalog.update_table_schema(&mut self.pager, &schema)?;
        for idx in &indexes {
            let new_idx_root = index_roots[&idx.name];
            if new_idx_root != idx.root_page {
                self.catalog
                    .update_index_root(&mut self.pager, &idx.name, new_idx_root)?;
            }
        }

        Ok(ExecResult::Ok)
    }

    fn execute_alter_table_drop_column(
        &mut self,
        table_name: &str,
        col_name: &str,
    ) -> Result<ExecResult> {
        let mut schema = self
            .catalog
            .get_table(&mut self.pager, table_name)?
            .ok_or_else(|| PlanError::NoSuchTable(table_name.to_string()))?;

        let col_idx = schema
            .column_index(col_name)
            .ok_or_else(|| PlanError::NoSuchColumn(col_name.to_string()))?;

        if schema.columns[col_idx].is_primary_key {
            return Err(DbError::Plan(PlanError::CannotUpdatePrimaryKey));
        }

        // Drop any indexes on this column
        let indexes = self
            .catalog
            .list_indexes_for_table(&mut self.pager, table_name)?;
        for idx in &indexes {
            if idx.column == col_name {
                self.catalog.drop_index(&mut self.pager, &idx.name)?;
            }
        }

        let remaining_indexes = self
            .catalog
            .list_indexes_for_table(&mut self.pager, table_name)?;

        // Scan all existing rows, remove column at col_idx, and re-write
        let all_cols: Vec<usize> = (0..schema.columns.len()).collect();
        let mut plan = crate::plan::planner::build_select_plan_with_context(
            &schema,
            &indexes,
            None,
            all_cols,
            None,
            None,
            &crate::auth::ExecutionContext::admin(),
        )?;

        let mut existing_rows = Vec::new();
        while let Some(mut row) = plan.next(&mut self.pager)? {
            row.remove(col_idx);
            existing_rows.push(row);
        }

        // Allocate new root leaf page for table
        let new_root = self.pager.allocate_page()?;
        crate::btree::node::LeafNode {
            entries: vec![],
            next_leaf: 0,
        }
        .encode(self.pager.get_page_mut(new_root)?);

        schema.columns.remove(col_idx);
        schema.root_page = new_root;

        let mut table_root = new_root;
        let mut index_roots: HashMap<String, u32> = remaining_indexes
            .iter()
            .map(|i| (i.name.clone(), i.root_page))
            .collect();

        for row in existing_rows {
            let mut schema_for_write = schema.clone();
            schema_for_write.root_page = table_root;
            let indexes_for_write: Vec<IndexSchema> = remaining_indexes
                .iter()
                .cloned()
                .map(|mut idx| {
                    idx.root_page = index_roots[&idx.name];
                    idx
                })
                .collect();

            let (next_table_root, next_index_roots) = crate::exec::mutate::insert_row(
                &mut self.pager,
                &schema_for_write,
                &indexes_for_write,
                &row,
            )?;
            table_root = next_table_root;
            for (name, root) in next_index_roots {
                index_roots.insert(name, root);
            }
        }

        schema.root_page = table_root;
        self.catalog.update_table_schema(&mut self.pager, &schema)?;
        for idx in &remaining_indexes {
            let new_idx_root = index_roots[&idx.name];
            if new_idx_root != idx.root_page {
                self.catalog
                    .update_index_root(&mut self.pager, &idx.name, new_idx_root)?;
            }
        }

        Ok(ExecResult::Ok)
    }

    fn execute_alter_table_rename(&mut self, old_name: &str, new_name: &str) -> Result<ExecResult> {
        self.catalog
            .rename_table(&mut self.pager, old_name, new_name)?;
        Ok(ExecResult::Ok)
    }

    fn execute_create_table(
        &mut self,
        name: String,
        columns: Vec<ColumnDef>,
    ) -> Result<ExecResult> {
        let pk_count = columns.iter().filter(|c| c.primary_key).count();
        if pk_count != 1 {
            return Err(DbError::Plan(PlanError::InvalidSchema(format!(
                "table {name} must declare exactly one PRIMARY KEY column"
            ))));
        }
        let root = self.pager.allocate_page()?;
        LeafNode {
            entries: vec![],
            next_leaf: 0,
        }
        .encode(self.pager.get_page_mut(root)?);
        let cols = columns
            .into_iter()
            .map(|c| Column {
                name: c.name,
                ty: c.ty,
                not_null: c.not_null || c.primary_key,
                is_primary_key: c.primary_key,
            })
            .collect();
        let schema = TableSchema {
            name,
            columns: cols,
            root_page: root,
            rls_enabled: false,
        };
        self.catalog.create_table(&mut self.pager, &schema)?;
        Ok(ExecResult::Ok)
    }

    fn execute_drop_table(&mut self, name: &str) -> Result<ExecResult> {
        self.catalog.drop_table(&mut self.pager, name)?;
        Ok(ExecResult::Ok)
    }

    fn execute_create_index(
        &mut self,
        name: &str,
        table: &str,
        column: &str,
    ) -> Result<ExecResult> {
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
        LeafNode {
            entries: vec![],
            next_leaf: 0,
        }
        .encode(self.pager.get_page_mut(initial_index_root)?);

        let pk_idx = schema.primary_key_index();
        let mut scan = crate::exec::scan::SeqScan::new(schema.clone());
        let mut current_root = initial_index_root;
        while let Some(row) = scan.next(&mut self.pager)? {
            let idx_key = crate::types::value::encode_composite_key(&[
                row[col_idx].clone(),
                row[pk_idx].clone(),
            ]);
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

    fn project_returning_rows(
        schema: &TableSchema,
        rows: Vec<Vec<Value>>,
        returning: &SelectColumns,
        ctx: &crate::auth::ExecutionContext,
    ) -> Result<ExecResult> {
        match returning {
            SelectColumns::All => {
                let names = schema.columns.iter().map(|c| c.name.clone()).collect();
                Ok(ExecResult::Rows {
                    columns: names,
                    rows,
                })
            }
            SelectColumns::List(col_names) => {
                let mut indices = Vec::new();
                for c in col_names {
                    let idx = schema
                        .column_index(c)
                        .ok_or(PlanError::NoSuchColumn(c.clone()))?;
                    indices.push(idx);
                }
                let projected_rows = rows
                    .into_iter()
                    .map(|r| indices.iter().map(|&i| r[i].clone()).collect())
                    .collect();
                Ok(ExecResult::Rows {
                    columns: col_names.clone(),
                    rows: projected_rows,
                })
            }
            SelectColumns::Items(items) => {
                let mut names = Vec::new();
                for (i, item) in items.iter().enumerate() {
                    match item {
                        crate::sql::ast::SelectItem::All => {
                            names.extend(schema.columns.iter().map(|c| c.name.clone()))
                        }
                        crate::sql::ast::SelectItem::Expr { alias, expr } => {
                            let name = alias.clone().unwrap_or_else(|| match expr {
                                Expr::Column(c) => c.clone(),
                                _ => format!("col_{i}"),
                            });
                            names.push(name);
                        }
                    }
                }
                let mut projected_rows = Vec::new();
                for r in &rows {
                    let mut out_row = Vec::new();
                    for item in items {
                        match item {
                            crate::sql::ast::SelectItem::All => out_row.extend(r.clone()),
                            crate::sql::ast::SelectItem::Expr { expr, .. } => {
                                let val =
                                    crate::plan::expr::eval_with_context(expr, schema, r, ctx)
                                        .map_err(DbError::Plan)?;
                                out_row.push(val);
                            }
                        }
                    }
                    projected_rows.push(out_row);
                }
                Ok(ExecResult::Rows {
                    columns: names,
                    rows: projected_rows,
                })
            }
        }
    }

    fn execute_insert(
        &mut self,
        table: &str,
        columns: Option<Vec<String>>,
        rows: Vec<Vec<Expr>>,
        returning: Option<SelectColumns>,
        ctx: &crate::auth::ExecutionContext,
    ) -> Result<ExecResult> {
        let schema = self
            .catalog
            .get_table(&mut self.pager, table)?
            .ok_or_else(|| PlanError::NoSuchTable(table.to_string()))?;
        let indexes = self
            .catalog
            .list_indexes_for_table(&mut self.pager, table)?;

        let policies = if schema.rls_enabled && !ctx.is_admin {
            let list = self
                .catalog
                .list_policies_for_table(&mut self.pager, table)?;
            let matching: Vec<crate::types::schema::PolicySchema> = list
                .into_iter()
                .filter(|p| {
                    p.cmd == crate::types::schema::PolicyCmd::Insert
                        || p.cmd == crate::types::schema::PolicyCmd::All
                })
                .collect();
            if matching.is_empty() {
                return Err(DbError::Exec(ExecError::InvalidValue(
                    "RLS check failed: no insert policy on table".into(),
                )));
            }
            Some(matching)
        } else {
            None
        };

        let mut table_root = schema.root_page;
        let mut index_roots: HashMap<String, u32> = indexes
            .iter()
            .map(|i| (i.name.clone(), i.root_page))
            .collect();
        let mut count = 0usize;
        let mut affected_rows = Vec::new();

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
                        full_row[idx] =
                            literal_to_value_typed(expr, Some(&schema.columns[idx].ty))?;
                    }
                }
                None => {
                    if expr_row.len() != schema.columns.len() {
                        return Err(DbError::Plan(PlanError::ColumnCountMismatch {
                            expected: schema.columns.len(),
                            found: expr_row.len(),
                        }));
                    }
                    for (idx, expr) in expr_row.iter().enumerate() {
                        full_row[idx] =
                            literal_to_value_typed(expr, Some(&schema.columns[idx].ty))?;
                    }
                }
            }

            for (idx, col) in schema.columns.iter().enumerate() {
                if col.not_null && matches!(full_row[idx], Value::Null) {
                    return Err(DbError::Exec(ExecError::NotNullViolation(col.name.clone())));
                }
            }

            if let Some(pols) = &policies {
                let mut passed = false;
                for pol in pols {
                    let check = pol.with_check.as_ref().or(pol.using_expr.as_ref());
                    if let Some(expr) = check {
                        let res =
                            crate::plan::expr::eval_with_context(expr, &schema, &full_row, ctx)
                                .map_err(DbError::Plan)?;
                        if crate::plan::expr::is_truthy(&res) {
                            passed = true;
                            break;
                        }
                    } else {
                        passed = true;
                        break;
                    }
                }
                if !passed {
                    return Err(DbError::Exec(ExecError::InvalidValue(
                        "row violates row-level security policy".into(),
                    )));
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

            let (new_table_root, new_index_roots) = crate::exec::mutate::insert_row(
                &mut self.pager,
                &schema_for_write,
                &indexes_for_write,
                &full_row,
            )?;
            table_root = new_table_root;
            for (name, root) in new_index_roots {
                index_roots.insert(name, root);
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let event = crate::server::protocol::ChangeEvent {
                table: table.to_string(),
                action: crate::server::protocol::ChangeAction::Insert,
                old_row: None,
                new_row: Some(full_row.clone()),
                timestamp_ms: now,
            };
            if self.pager.in_transaction() {
                self.transaction_events.push(event);
            } else if let Some(tx) = &self.change_tx {
                let _ = tx.send(event);
            }
            affected_rows.push(full_row);
            count += 1;
        }

        if table_root != schema.root_page {
            self.catalog
                .update_table_root(&mut self.pager, table, table_root)?;
        }
        for idx in &indexes {
            let new_root = index_roots[&idx.name];
            if new_root != idx.root_page {
                self.catalog
                    .update_index_root(&mut self.pager, &idx.name, new_root)?;
            }
        }

        if let Some(ret_cols) = &returning {
            Self::project_returning_rows(&schema, affected_rows, ret_cols, ctx)
        } else {
            Ok(ExecResult::Modified(count))
        }
    }

    fn execute_update(
        &mut self,
        table: &str,
        assignments: Vec<(String, Expr)>,
        where_clause: Option<Expr>,
        returning: Option<SelectColumns>,
        ctx: &crate::auth::ExecutionContext,
    ) -> Result<ExecResult> {
        let where_clause = self.apply_rls_filter(
            table,
            crate::types::schema::PolicyCmd::Update,
            where_clause,
            ctx,
        )?;
        let where_clause = if let Some(w) = where_clause {
            Some(self.resolve_subqueries_in_expr(&w, ctx)?)
        } else {
            None
        };
        let schema = self
            .catalog
            .get_table(&mut self.pager, table)?
            .ok_or_else(|| PlanError::NoSuchTable(table.to_string()))?;
        let indexes = self
            .catalog
            .list_indexes_for_table(&mut self.pager, table)?;

        let mut assignment_indices = Vec::new();
        for (col_name, expr) in &assignments {
            let idx = schema
                .column_index(col_name)
                .ok_or_else(|| PlanError::NoSuchColumn(col_name.clone()))?;
            if schema.columns[idx].is_primary_key {
                return Err(DbError::Plan(PlanError::CannotUpdatePrimaryKey));
            }
            assignment_indices.push((idx, expr.clone()));
        }

        let all_columns: Vec<usize> = (0..schema.columns.len()).collect();
        let mut plan = crate::plan::planner::build_select_plan_with_context(
            &schema,
            &indexes,
            where_clause,
            all_columns,
            None,
            None,
            ctx,
        )?;
        let mut old_rows = Vec::new();
        while let Some(row) = plan.next(&mut self.pager)? {
            old_rows.push(row);
        }

        let mut table_root = schema.root_page;
        let mut index_roots: HashMap<String, u32> = indexes
            .iter()
            .map(|i| (i.name.clone(), i.root_page))
            .collect();
        let mut count = 0usize;
        let mut affected_rows = Vec::new();

        for old_row in &old_rows {
            let mut new_row = old_row.clone();
            for (idx, expr) in &assignment_indices {
                new_row[*idx] = crate::plan::expr::eval_with_context(expr, &schema, old_row, ctx)
                    .map_err(DbError::Plan)?;
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
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let event = crate::server::protocol::ChangeEvent {
                table: table.to_string(),
                action: crate::server::protocol::ChangeAction::Update,
                old_row: Some(old_row.clone()),
                new_row: Some(new_row.clone()),
                timestamp_ms: now,
            };
            if self.pager.in_transaction() {
                self.transaction_events.push(event);
            } else if let Some(tx) = &self.change_tx {
                let _ = tx.send(event);
            }
            affected_rows.push(new_row);
            count += 1;
        }

        if table_root != schema.root_page {
            self.catalog
                .update_table_root(&mut self.pager, table, table_root)?;
        }
        for idx in &indexes {
            let new_root = index_roots[&idx.name];
            if new_root != idx.root_page {
                self.catalog
                    .update_index_root(&mut self.pager, &idx.name, new_root)?;
            }
        }

        if let Some(ret_cols) = &returning {
            Self::project_returning_rows(&schema, affected_rows, ret_cols, ctx)
        } else {
            Ok(ExecResult::Modified(count))
        }
    }

    #[allow(clippy::too_many_arguments)]
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
        ctx: &crate::auth::ExecutionContext,
    ) -> Result<ExecResult> {
        let is_join = matches!(&table_ref, Some(crate::sql::ast::TableRef::Join { .. }));

        let has_group_by = _group_by.is_some();
        let has_agg = match &columns {
            SelectColumns::Items(items) => items.iter().any(|item| {
                matches!(
                    item,
                    SelectItem::Expr {
                        expr: Expr::Aggregate(_),
                        ..
                    }
                )
            }),
            _ => false,
        };

        let where_clause = if !is_join {
            self.apply_rls_filter(
                table,
                crate::types::schema::PolicyCmd::Select,
                where_clause,
                ctx,
            )?
        } else {
            where_clause
        };
        let where_clause = if let Some(w) = where_clause {
            Some(self.resolve_subqueries_in_expr(&w, ctx)?)
        } else {
            None
        };

        if is_join {
            let tref = table_ref.unwrap();
            let (mut plan, schema) = crate::plan::planner::build_table_ref_plan(
                &mut self.catalog,
                &mut self.pager,
                &tref,
            )?;
            if let Some(predicate) = where_clause {
                plan = Box::new(crate::exec::filter::Filter {
                    input: plan,
                    schema: schema.clone(),
                    predicate,
                    context: ctx.clone(),
                });
            }

            if has_group_by || has_agg {
                let mut aggregates = Vec::new();
                let mut out_names = Vec::new();
                let mut group_exprs = _group_by.clone().unwrap_or_default();

                if let SelectColumns::Items(items) = &columns {
                    for item in items {
                        match item {
                            SelectItem::Expr {
                                expr: Expr::Aggregate(func),
                                alias,
                            } => {
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
                                    Expr::QualifiedColumn { table, column } => {
                                        format!("{table}.{column}")
                                    }
                                    _ => "expr".into(),
                                });
                                out_names.push(name);
                                if !has_group_by && !group_exprs.contains(expr) {
                                    group_exprs.push(expr.clone());
                                }
                            }
                            SelectItem::All => {
                                return Err(DbError::Plan(PlanError::InvalidExpression(
                                    "cannot use SELECT * with aggregations".into(),
                                )))
                            }
                        }
                    }
                }

                let mut agg_plan: Box<dyn Operator> =
                    Box::new(crate::exec::aggregate::AggregateOperator::new(
                        plan,
                        schema.clone(),
                        if group_exprs.is_empty() {
                            None
                        } else {
                            Some(group_exprs)
                        },
                        aggregates,
                        _having,
                    ));

                if let Some(n) = limit {
                    agg_plan = Box::new(crate::exec::limit::Limit::new(agg_plan, n));
                }

                let mut rows = Vec::new();
                while let Some(row) = agg_plan.next(&mut self.pager)? {
                    rows.push(row);
                }
                return Ok(ExecResult::Rows {
                    columns: out_names,
                    rows,
                });
            }

            let (out_names, indices) = match &columns {
                SelectColumns::All => (
                    schema.columns.iter().map(|c| c.name.clone()).collect(),
                    (0..schema.columns.len()).collect(),
                ),
                SelectColumns::List(names) => {
                    let mut idxs = Vec::new();
                    for n in names {
                        let idx = schema
                            .column_index(n)
                            .ok_or_else(|| PlanError::NoSuchColumn(n.clone()))?;
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
                                    Expr::QualifiedColumn { table, column } => {
                                        format!("{table}.{column}")
                                    }
                                    Expr::JsonExtract { path, .. } => path.clone(),
                                    _ => "expr".into(),
                                });
                                names.push(name);
                                exprs.push(expr.clone());
                            }
                        }
                    }

                    let (plan_order_by, post_sort) = if let Some((ref col_name, desc)) = order_by {
                        if schema.column_index(col_name).is_some() {
                            (Some((col_name.clone(), desc)), None)
                        } else if let Some(proj_idx) = names.iter().position(|n| n == col_name) {
                            (None, Some((proj_idx, desc)))
                        } else {
                            (Some((col_name.clone(), desc)), None)
                        }
                    } else {
                        (None, None)
                    };

                    if let Some((col, desc)) = plan_order_by {
                        let idx = schema
                            .column_index(&col)
                            .ok_or(PlanError::NoSuchColumn(col))?;
                        plan = Box::new(crate::exec::sort::Sort::new(plan, idx, desc));
                    }
                    plan = Box::new(crate::exec::project::ProjectExpr {
                        input: plan,
                        schema: schema.clone(),
                        exprs,
                        context: ctx.clone(),
                    });
                    if let Some((idx, desc)) = post_sort {
                        plan = Box::new(crate::exec::sort::Sort::new(plan, idx, desc));
                    }
                    if let Some(n) = limit {
                        plan = Box::new(crate::exec::limit::Limit::new(plan, n));
                    }

                    let mut rows = Vec::new();
                    while let Some(row) = plan.next(&mut self.pager)? {
                        rows.push(row);
                    }
                    return Ok(ExecResult::Rows {
                        columns: names,
                        rows,
                    });
                }
            };

            if let Some((col, desc)) = order_by {
                let idx = schema
                    .column_index(&col)
                    .ok_or(PlanError::NoSuchColumn(col))?;
                plan = Box::new(crate::exec::sort::Sort::new(plan, idx, desc));
            }
            plan = Box::new(crate::exec::project::Project {
                input: plan,
                indices,
            });
            if let Some(n) = limit {
                plan = Box::new(crate::exec::limit::Limit::new(plan, n));
            }

            let mut rows = Vec::new();
            while let Some(row) = plan.next(&mut self.pager)? {
                rows.push(row);
            }
            return Ok(ExecResult::Rows {
                columns: out_names,
                rows,
            });
        }

        let schema = self
            .catalog
            .get_table(&mut self.pager, table)?
            .ok_or_else(|| PlanError::NoSuchTable(table.to_string()))?;
        let indexes = self
            .catalog
            .list_indexes_for_table(&mut self.pager, table)?;

        if has_group_by || has_agg {
            let mut aggregates = Vec::new();
            let mut out_names = Vec::new();
            let mut group_exprs = _group_by.clone().unwrap_or_default();

            if let SelectColumns::Items(items) = &columns {
                for item in items {
                    match item {
                        SelectItem::Expr {
                            expr: Expr::Aggregate(func),
                            alias,
                        } => {
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
                        SelectItem::All => {
                            return Err(DbError::Plan(PlanError::InvalidExpression(
                                "cannot use SELECT * with aggregations".into(),
                            )))
                        }
                    }
                }
            }

            let seq_scan: Box<dyn Operator> =
                Box::new(crate::exec::scan::SeqScan::new(schema.clone()));
            let scan_plan = if let Some(predicate) = where_clause {
                Box::new(crate::exec::filter::Filter {
                    input: seq_scan,
                    schema: schema.clone(),
                    predicate,
                    context: ctx.clone(),
                })
            } else {
                seq_scan
            };

            let mut plan: Box<dyn Operator> =
                Box::new(crate::exec::aggregate::AggregateOperator::new(
                    scan_plan,
                    schema.clone(),
                    if group_exprs.is_empty() {
                        None
                    } else {
                        Some(group_exprs)
                    },
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
            return Ok(ExecResult::Rows {
                columns: out_names,
                rows,
            });
        }

        let (out_names, indices): (Vec<String>, Vec<usize>) = match &columns {
            SelectColumns::All => (
                schema.columns.iter().map(|c| c.name.clone()).collect(),
                (0..schema.columns.len()).collect(),
            ),
            SelectColumns::List(names) => {
                let mut idxs = Vec::new();
                for n in names {
                    idxs.push(
                        schema
                            .column_index(n)
                            .ok_or_else(|| PlanError::NoSuchColumn(n.clone()))?,
                    );
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

                let (plan_order_by, post_sort) = if let Some((ref col_name, desc)) = order_by {
                    if schema.column_index(col_name).is_some() {
                        (Some((col_name.clone(), desc)), None)
                    } else if let Some(proj_idx) = names.iter().position(|n| n == col_name) {
                        (None, Some((proj_idx, desc)))
                    } else {
                        (Some((col_name.clone(), desc)), None)
                    }
                } else {
                    (None, None)
                };

                let all_columns: Vec<usize> = (0..schema.columns.len()).collect();
                let mut plan = crate::plan::planner::build_select_plan_with_context(
                    &schema,
                    &indexes,
                    where_clause,
                    all_columns,
                    plan_order_by,
                    None,
                    ctx,
                )?;
                plan = Box::new(crate::exec::project::ProjectExpr {
                    input: plan,
                    schema: schema.clone(),
                    exprs,
                    context: ctx.clone(),
                });
                if let Some((idx, desc)) = post_sort {
                    plan = Box::new(crate::exec::sort::Sort::new(plan, idx, desc));
                }
                if let Some(n) = limit {
                    plan = Box::new(crate::exec::limit::Limit::new(plan, n));
                }
                let mut rows = Vec::new();
                while let Some(row) = plan.next(&mut self.pager)? {
                    rows.push(row);
                }
                return Ok(ExecResult::Rows {
                    columns: names,
                    rows,
                });
            }
        };

        let mut plan = crate::plan::planner::build_select_plan_with_context(
            &schema,
            &indexes,
            where_clause,
            indices,
            order_by,
            limit,
            ctx,
        )?;
        let mut rows = Vec::new();
        while let Some(row) = plan.next(&mut self.pager)? {
            rows.push(row);
        }
        Ok(ExecResult::Rows {
            columns: out_names,
            rows,
        })
    }

    fn execute_delete(
        &mut self,
        table: &str,
        where_clause: Option<Expr>,
        returning: Option<SelectColumns>,
        ctx: &crate::auth::ExecutionContext,
    ) -> Result<ExecResult> {
        let where_clause = self.apply_rls_filter(
            table,
            crate::types::schema::PolicyCmd::Delete,
            where_clause,
            ctx,
        )?;
        let where_clause = if let Some(w) = where_clause {
            Some(self.resolve_subqueries_in_expr(&w, ctx)?)
        } else {
            None
        };
        let schema = self
            .catalog
            .get_table(&mut self.pager, table)?
            .ok_or_else(|| PlanError::NoSuchTable(table.to_string()))?;
        let indexes = self
            .catalog
            .list_indexes_for_table(&mut self.pager, table)?;

        let all_columns: Vec<usize> = (0..schema.columns.len()).collect();
        let mut plan = crate::plan::planner::build_select_plan_with_context(
            &schema,
            &indexes,
            where_clause,
            all_columns,
            None,
            None,
            ctx,
        )?;
        let mut rows_to_delete = Vec::new();
        while let Some(row) = plan.next(&mut self.pager)? {
            rows_to_delete.push(row);
        }

        let mut table_root = schema.root_page;
        let mut index_roots: HashMap<String, u32> = indexes
            .iter()
            .map(|i| (i.name.clone(), i.root_page))
            .collect();
        let mut count = 0usize;
        let mut affected_rows = Vec::new();

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

            let (new_table_root, new_index_roots) = crate::exec::mutate::delete_row(
                &mut self.pager,
                &schema_for_write,
                &indexes_for_write,
                row,
            )?;
            table_root = new_table_root;
            for (name, root) in new_index_roots {
                index_roots.insert(name, root);
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let event = crate::server::protocol::ChangeEvent {
                table: table.to_string(),
                action: crate::server::protocol::ChangeAction::Delete,
                old_row: Some(row.clone()),
                new_row: None,
                timestamp_ms: now,
            };
            if self.pager.in_transaction() {
                self.transaction_events.push(event);
            } else if let Some(tx) = &self.change_tx {
                let _ = tx.send(event);
            }
            affected_rows.push(row.clone());
            count += 1;
        }

        if table_root != schema.root_page {
            self.catalog
                .update_table_root(&mut self.pager, table, table_root)?;
        }
        for idx in &indexes {
            let new_root = index_roots[&idx.name];
            if new_root != idx.root_page {
                self.catalog
                    .update_index_root(&mut self.pager, &idx.name, new_root)?;
            }
        }

        if let Some(ret_cols) = &returning {
            Self::project_returning_rows(&schema, affected_rows, ret_cols, ctx)
        } else {
            Ok(ExecResult::Modified(count))
        }
    }

    pub fn list_tables(&mut self) -> Vec<String> {
        let tables = self
            .catalog
            .list_tables(&mut self.pager)
            .unwrap_or_default();
        tables.into_iter().filter(|t| !t.starts_with('_')).collect()
    }

    pub fn table_schema(&mut self, name: &str) -> Option<TableSchema> {
        self.catalog.get_table(&mut self.pager, name).ok().flatten()
    }

    pub fn list_indexes(&mut self, table: &str) -> Vec<crate::types::schema::IndexSchema> {
        self.catalog
            .list_indexes_for_table(&mut self.pager, table)
            .unwrap_or_default()
    }

    pub fn dump_table_btree(&mut self, table: &str) -> Option<String> {
        let schema = self.table_schema(table)?;
        let mut bt = crate::btree::tree::BTree::new(&mut self.pager, schema.root_page);
        Some(bt.dump())
    }

    pub fn pager_stats(&self) -> crate::storage::pager::PagerStats {
        self.pager.stats()
    }

    pub fn explain_statement(
        &mut self,
        stmt: &Statement,
        _ctx: &ExecutionContext,
    ) -> Result<Vec<String>> {
        match stmt {
            Statement::Select {
                table,
                where_clause,
                order_by,
                limit,
                table_ref,
                ..
            } => {
                let mut lines = Vec::new();
                let is_join = matches!(table_ref, Some(crate::sql::ast::TableRef::Join { .. }));
                if is_join {
                    lines.push(format!("-> Join Execution: {table_ref:?}"));
                } else if let Some(schema) = self.catalog.get_table(&mut self.pager, table)? {
                    let indexes = self
                        .catalog
                        .list_indexes_for_table(&mut self.pager, table)?;
                    let pk_col = &schema.columns[schema.primary_key_index()].name;
                    let (pk_val, residual) =
                        crate::plan::planner::extract_pk_equality(where_clause.clone(), pk_col);
                    if let Some(v) = pk_val {
                        lines.push(format!(
                            "-> TableSeek on {table} (cost=1.0..1.2 rows=1 width={})",
                            schema.columns.len()
                        ));
                        lines.push(format!("   Index Cond: ({pk_col} = {v:?})"));
                    } else if let Some((idx_schema, val, _)) =
                        crate::plan::planner::find_index_equality(residual, &indexes)
                    {
                        lines.push(format!(
                            "-> IndexSeek on {} using {} (cost=1.0..4.5 rows=10 width={})",
                            table,
                            idx_schema.name,
                            schema.columns.len()
                        ));
                        lines.push(format!("   Index Cond: ({} = {val:?})", idx_schema.column));
                    } else {
                        lines.push(format!(
                            "-> SeqScan on {table} (cost=0.0..25.0 rows=100 width={})",
                            schema.columns.len()
                        ));
                    }
                    if let Some(pred) = where_clause {
                        lines.push(format!("   Filter: {pred:?}"));
                    }
                } else {
                    lines.push(format!("-> Scan on {table}"));
                }
                if let Some((col, desc)) = order_by {
                    lines.push(format!(
                        "-> Sort by {col} {}",
                        if *desc { "DESC" } else { "ASC" }
                    ));
                }
                if let Some(n) = limit {
                    lines.push(format!("-> Limit {n}"));
                }
                Ok(lines)
            }
            Statement::Insert { table, rows, .. } => Ok(vec![format!(
                "-> Insert into {table} (rows={})",
                rows.len()
            )]),
            Statement::Update {
                table,
                where_clause,
                ..
            } => Ok(vec![format!(
                "-> Update on {table} filter={where_clause:?}"
            )]),
            Statement::Delete {
                table,
                where_clause,
                ..
            } => Ok(vec![format!(
                "-> Delete on {table} filter={where_clause:?}"
            )]),
            other => Ok(vec![format!("-> Execute statement: {other:?}")]),
        }
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
        Expr::IntLiteral(n) => match target_type {
            Some(ColumnType::Float) => Ok(Value::Float(*n as f64)),
            _ => Ok(Value::Integer(*n)),
        },
        Expr::FloatLiteral(f) => Ok(Value::Float(*f)),
        Expr::StringLiteral(s) => match target_type {
            Some(ColumnType::Json) => {
                serde_json::from_str::<serde_json::Value>(s).map_err(|e| {
                    DbError::Exec(crate::error::ExecError::InvalidValue(format!(
                        "invalid JSON payload: {e}"
                    )))
                })?;
                Ok(Value::Json(s.clone()))
            }
            Some(ColumnType::Vector(dim)) => {
                let vec = serde_json::from_str::<Vec<f32>>(s).map_err(|e| {
                    DbError::Exec(crate::error::ExecError::InvalidValue(format!(
                        "invalid vector payload: {e}"
                    )))
                })?;
                if *dim > 0 && vec.len() != *dim {
                    return Err(DbError::Exec(crate::error::ExecError::InvalidValue(
                        format!(
                            "vector dimension mismatch: expected {dim}, found {}",
                            vec.len()
                        ),
                    )));
                }
                Ok(Value::Vector(vec))
            }
            _ => Ok(Value::Text(s.clone())),
        },
        Expr::BoolLiteral(b) => Ok(Value::Boolean(*b)),
        Expr::Null => Ok(Value::Null),
        other => Err(DbError::Exec(crate::error::ExecError::InvalidValue(
            format!("expected a literal value in statement, found {other:?}"),
        ))),
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
            db.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
                .unwrap(),
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
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
            .unwrap();
        let err = db
            .execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
            .unwrap_err();
        assert!(matches!(
            err,
            DbError::Plan(PlanError::TableAlreadyExists(_))
        ));
    }

    #[test]
    fn reopening_preserves_schema() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path();
        {
            let mut db = Database::create(path).unwrap();
            db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
                .unwrap();
        }
        let mut db = Database::open(path).unwrap();
        db.execute("DROP TABLE t").unwrap(); // succeeds only if the schema survived reopen
    }

    #[test]
    fn insert_then_reinsert_same_pk_errors() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)")
            .unwrap();
        assert_eq!(
            db.execute("INSERT INTO t (id, name) VALUES (1, 'a')")
                .unwrap(),
            ExecResult::Modified(1)
        );
        assert!(db
            .execute("INSERT INTO t (id, name) VALUES (1, 'b')")
            .is_err());
    }

    #[test]
    fn insert_many_rows_forces_table_split_and_still_works() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
            .unwrap();
        for i in 0..500 {
            let sql = format!("INSERT INTO t (id) VALUES ({i})");
            assert_eq!(db.execute(&sql).unwrap(), ExecResult::Modified(1));
        }
    }

    #[test]
    fn select_with_where_and_projection() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)")
            .unwrap();
        db.execute("INSERT INTO t (id, name) VALUES (1, 'a'), (2, 'b'), (3, 'c')")
            .unwrap();

        let result = db.execute("SELECT name FROM t WHERE id > 1").unwrap();
        match result {
            ExecResult::Rows { columns, rows } => {
                assert_eq!(columns, vec!["name".to_string()]);
                assert_eq!(
                    rows,
                    vec![vec![Value::Text("b".into())], vec![Value::Text("c".into())]]
                );
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[test]
    fn select_with_order_by_and_limit() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, score INTEGER)")
            .unwrap();
        db.execute("INSERT INTO t (id, score) VALUES (1, 30), (2, 10), (3, 20)")
            .unwrap();

        let result = db
            .execute("SELECT id FROM t ORDER BY score LIMIT 2")
            .unwrap();
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
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
            .unwrap();
        db.execute("INSERT INTO t (id) VALUES (1), (2), (3)")
            .unwrap();

        assert_eq!(
            db.execute("DELETE FROM t WHERE id = 2").unwrap(),
            ExecResult::Modified(1)
        );

        let result = db.execute("SELECT id FROM t").unwrap();
        match result {
            ExecResult::Rows { rows, .. } => {
                let mut remaining: Vec<i64> = rows
                    .iter()
                    .map(|r| match &r[0] {
                        Value::Integer(n) => *n,
                        _ => unreachable!(),
                    })
                    .collect();
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
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)")
            .unwrap();
        db.execute("INSERT INTO t (id, name) VALUES (1, 'a'), (2, 'b')")
            .unwrap();

        assert_eq!(
            db.execute("UPDATE t SET name = 'z' WHERE id = 1").unwrap(),
            ExecResult::Modified(1)
        );

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
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
            .unwrap();
        db.execute("INSERT INTO t (id, name) VALUES (1, 'a'), (2, 'b')")
            .unwrap();

        assert_eq!(
            db.execute("CREATE INDEX idx_name ON t (name)").unwrap(),
            ExecResult::Ok
        );
        assert_eq!(db.execute("DROP INDEX idx_name").unwrap(), ExecResult::Ok);
    }

    #[test]
    fn create_index_on_nullable_column_errors() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)")
            .unwrap();
        let err = db.execute("CREATE INDEX idx_name ON t (name)").unwrap_err();
        assert!(matches!(err, DbError::Plan(PlanError::InvalidSchema(_))));
    }

    #[test]
    fn select_on_indexed_column_uses_index_and_returns_correct_rows() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
            .unwrap();
        db.execute("INSERT INTO t (id, name) VALUES (1, 'a'), (2, 'b'), (3, 'a')")
            .unwrap();
        db.execute("CREATE INDEX idx_name ON t (name)").unwrap();

        let result = db.execute("SELECT id FROM t WHERE name = 'a'").unwrap();
        match result {
            ExecResult::Rows { rows, .. } => {
                let mut ids: Vec<i64> = rows
                    .iter()
                    .map(|r| match &r[0] {
                        Value::Integer(n) => *n,
                        _ => unreachable!(),
                    })
                    .collect();
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
            db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT)")
                .unwrap();
            db.execute("BEGIN").unwrap();
            db.execute("INSERT INTO t (id, val) VALUES (1, 'committed')")
                .unwrap();
            db.execute("COMMIT").unwrap();
        }
        let mut db = Database::open(path).unwrap();
        let res = db.execute("SELECT val FROM t WHERE id = 1").unwrap();
        match res {
            ExecResult::Rows { rows, .. } => {
                assert_eq!(rows, vec![vec![Value::Text("committed".into())]])
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[test]
    fn transaction_rollback_discards_mutations() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path();
        let mut db = Database::create(path).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT)")
            .unwrap();
        db.execute("INSERT INTO t (id, val) VALUES (1, 'initial')")
            .unwrap();

        db.execute("BEGIN").unwrap();
        db.execute("UPDATE t SET val = 'modified' WHERE id = 1")
            .unwrap();
        db.execute("INSERT INTO t (id, val) VALUES (2, 'new')")
            .unwrap();
        db.execute("ROLLBACK").unwrap();

        let res = db.execute("SELECT val FROM t WHERE id = 1").unwrap();
        match res {
            ExecResult::Rows { rows, .. } => {
                assert_eq!(rows, vec![vec![Value::Text("initial".into())]])
            }
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
        db.execute("CREATE TABLE t1 (id INTEGER PRIMARY KEY)")
            .unwrap();

        db.execute("BEGIN").unwrap();
        db.execute("CREATE TABLE t2 (id INTEGER PRIMARY KEY)")
            .unwrap();
        db.execute("DROP TABLE t1").unwrap();
        db.execute("ROLLBACK").unwrap();

        assert_eq!(db.list_tables(), vec!["t1".to_string()]);
    }

    #[test]
    fn nested_begin_and_naked_commit_error() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        assert!(matches!(
            db.execute("COMMIT").unwrap_err(),
            DbError::Plan(PlanError::NoTransactionInProgress)
        ));
        assert!(matches!(
            db.execute("ROLLBACK").unwrap_err(),
            DbError::Plan(PlanError::NoTransactionInProgress)
        ));

        db.execute("BEGIN").unwrap();
        assert!(matches!(
            db.execute("BEGIN").unwrap_err(),
            DbError::Plan(PlanError::NestedTransactionNotSupported)
        ));
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
        assert!(db.execute("SELECT 1 FROM non_existent").is_err());

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
