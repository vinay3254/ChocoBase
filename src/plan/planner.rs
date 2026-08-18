use crate::error::PlanError;
use crate::exec::filter::Filter;
use crate::exec::limit::Limit;
use crate::exec::project::Project;
use crate::exec::scan::{IndexSeek, SeqScan, TableSeek};
use crate::exec::sort::Sort;
use crate::exec::Operator;
use crate::sql::ast::{BinOp, Expr};
use crate::types::schema::{IndexSchema, TableSchema};
use crate::types::value::{encode_key, Value};

pub fn build_select_plan(
    schema: &TableSchema,
    indexes: &[IndexSchema],
    where_clause: Option<Expr>,
    projection_indices: Vec<usize>,
    order_by: Option<(String, bool)>,
    limit: Option<i64>,
) -> Result<Box<dyn Operator>, PlanError> {
    build_select_plan_with_context(
        schema,
        indexes,
        where_clause,
        projection_indices,
        order_by,
        limit,
        &crate::auth::ExecutionContext::anonymous(),
    )
}

pub fn build_select_plan_with_context(
    schema: &TableSchema,
    indexes: &[IndexSchema],
    where_clause: Option<Expr>,
    projection_indices: Vec<usize>,
    order_by: Option<(String, bool)>,
    limit: Option<i64>,
    context: &crate::auth::ExecutionContext,
) -> Result<Box<dyn Operator>, PlanError> {
    let pk_col = schema.columns[schema.primary_key_index()].name.clone();
    let (pk_value, residual) = extract_pk_equality(where_clause, &pk_col);

    let (mut plan, residual): (Box<dyn Operator>, Option<Expr>) = if let Some(v) = pk_value {
        (Box::new(TableSeek::new(schema.clone(), encode_key(&v))), residual)
    } else if let Some((idx_schema, value, residual2)) = find_index_equality(residual.clone(), indexes) {
        let prefix = encode_key(&value);
        (Box::new(IndexSeek::new(schema.clone(), idx_schema.root_page, prefix)), residual2)
    } else {
        (Box::new(SeqScan::new(schema.clone())), residual)
    };

    if let Some(predicate) = residual {
        plan = Box::new(Filter { input: plan, schema: schema.clone(), predicate, context: context.clone() });
    }
    if let Some((col, desc)) = order_by {
        let idx = schema.column_index(&col).ok_or_else(|| PlanError::NoSuchColumn(col))?;
        plan = Box::new(Sort::new(plan, idx, desc));
    }
    plan = Box::new(Project { input: plan, indices: projection_indices });
    if let Some(n) = limit {
        plan = Box::new(Limit::new(plan, n));
    }
    Ok(plan)
}

pub fn build_table_ref_plan(
    catalog: &mut crate::catalog::Catalog,
    pager: &mut crate::storage::pager::Pager,
    table_ref: &crate::sql::ast::TableRef,
) -> Result<(Box<dyn Operator>, TableSchema), PlanError> {
    match table_ref {
        crate::sql::ast::TableRef::Table { name, alias } => {
            let base_schema = catalog
                .get_table(pager, name)
                .map_err(|e| PlanError::NoSuchTable(e.to_string()))?
                .ok_or_else(|| PlanError::NoSuchTable(name.clone()))?;

            let prefix = alias.as_deref().unwrap_or(name);
            let mut cols = Vec::new();
            for c in &base_schema.columns {
                let mut qual_col = c.clone();
                qual_col.name = format!("{prefix}.{}", c.name);
                cols.push(qual_col);
            }

            let schema = TableSchema {
                name: prefix.to_string(),
                columns: cols,
                root_page: base_schema.root_page,
                rls_enabled: base_schema.rls_enabled,
            };
            let plan: Box<dyn Operator> = Box::new(SeqScan::new(base_schema));
            Ok((plan, schema))
        }
        crate::sql::ast::TableRef::Join { left, right, join_type, condition } => {
            let (left_plan, left_schema) = build_table_ref_plan(catalog, pager, left)?;
            let (right_plan, right_schema) = build_table_ref_plan(catalog, pager, right)?;

            let mut combined_cols = left_schema.columns.clone();
            combined_cols.extend(right_schema.columns.clone());

            let combined_schema = TableSchema {
                name: format!("{}_join_{}", left_schema.name, right_schema.name),
                columns: combined_cols,
                root_page: 0,
                rls_enabled: false,
            };

            let join_op = Box::new(crate::exec::join::NestedLoopJoin::new(
                left_plan,
                right_plan,
                right_schema.columns.len(),
                *join_type,
                condition.clone(),
                combined_schema.clone(),
            ));

            Ok((join_op, combined_schema))
        }
    }
}

pub(crate) fn find_index_equality(where_clause: Option<Expr>, indexes: &[IndexSchema]) -> Option<(IndexSchema, Value, Option<Expr>)> {
    let expr = where_clause?;
    if contains_or(&expr) {
        return None;
    }
    let mut conjuncts = Vec::new();
    split_and_conjuncts(expr, &mut conjuncts);

    let mut matched: Option<(IndexSchema, Value)> = None;
    let mut remaining = Vec::new();
    for c in conjuncts {
        if matched.is_none() {
            if let Expr::BinaryOp { op: BinOp::Eq, left, right } = &c {
                let candidate = match (left.as_ref(), right.as_ref()) {
                    (Expr::Column(name), lit) => indexes
                        .iter()
                        .find(|i| &i.column == name)
                        .and_then(|i| literal_value(lit).map(|v| (i.clone(), v))),
                    (lit, Expr::Column(name)) => indexes
                        .iter()
                        .find(|i| &i.column == name)
                        .and_then(|i| literal_value(lit).map(|v| (i.clone(), v))),
                    _ => None,
                };
                if let Some(pair) = candidate {
                    matched = Some(pair);
                    continue;
                }
            }
        }
        remaining.push(c);
    }
    matched.map(|(idx, v)| (idx, v, rebuild_and(remaining)))
}

fn split_and_conjuncts(expr: Expr, out: &mut Vec<Expr>) {
    match expr {
        Expr::BinaryOp { op: BinOp::And, left, right } => {
            split_and_conjuncts(*left, out);
            split_and_conjuncts(*right, out);
        }
        other => out.push(other),
    }
}

fn rebuild_and(mut conjuncts: Vec<Expr>) -> Option<Expr> {
    if conjuncts.is_empty() {
        return None;
    }
    let mut acc = conjuncts.remove(0);
    for c in conjuncts {
        acc = Expr::BinaryOp { op: BinOp::And, left: Box::new(acc), right: Box::new(c) };
    }
    Some(acc)
}

fn contains_or(expr: &Expr) -> bool {
    match expr {
        Expr::BinaryOp { op: BinOp::Or, .. } => true,
        Expr::BinaryOp { left, right, .. } => contains_or(left) || contains_or(right),
        _ => false,
    }
}

fn literal_value(expr: &Expr) -> Option<Value> {
    match expr {
        Expr::IntLiteral(n) => Some(Value::Integer(*n)),
        Expr::StringLiteral(s) => Some(Value::Text(s.clone())),
        Expr::BoolLiteral(b) => Some(Value::Boolean(*b)),
        _ => None,
    }
}

/// Splits `where_clause` into (an optional PK-equality literal to seek on, the
/// remaining predicate to apply as a residual filter). Only a top-level chain of
/// `AND`s is analyzed; any `OR` anywhere disables the optimization entirely and
/// the whole expression is returned as the residual filter.
pub(crate) fn extract_pk_equality(where_clause: Option<Expr>, pk_col: &str) -> (Option<Value>, Option<Expr>) {
    let Some(expr) = where_clause else {
        return (None, None);
    };
    if contains_or(&expr) {
        return (None, Some(expr));
    }
    let mut conjuncts = Vec::new();
    split_and_conjuncts(expr, &mut conjuncts);

    let mut pk_value = None;
    let mut remaining = Vec::new();
    for c in conjuncts {
        if pk_value.is_none() {
            if let Expr::BinaryOp { op: BinOp::Eq, left, right } = &c {
                let matched = match (left.as_ref(), right.as_ref()) {
                    (Expr::Column(name), lit) if name == pk_col => literal_value(lit),
                    (lit, Expr::Column(name)) if name == pk_col => literal_value(lit),
                    _ => None,
                };
                if let Some(v) = matched {
                    pk_value = Some(v);
                    continue;
                }
            }
        }
        remaining.push(c);
    }
    (pk_value, rebuild_and(remaining))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::btree::node::LeafNode;
    use crate::storage::pager::Pager;
    use crate::types::schema::Column;
    use crate::types::value::{ColumnType, Value};
    use tempfile::NamedTempFile;

    #[test]
    fn builds_a_plan_that_scans_filters_and_projects() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let initial_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(initial_root).unwrap());

        let mut schema = TableSchema {
            name: "t".into(),
            columns: vec![
                Column { name: "id".into(), ty: ColumnType::Integer, not_null: true, is_primary_key: true },
                Column { name: "name".into(), ty: ColumnType::Text, not_null: true, is_primary_key: false },
            ],
            root_page: initial_root,
            rls_enabled: false,
        };

        let final_root = {
            let mut bt = crate::btree::tree::BTree::new(&mut pager, initial_root);
            for (id, name) in [(1, "a"), (2, "b"), (3, "c")] {
                let row = vec![Value::Integer(id), Value::Text(name.into())];
                bt.insert(&crate::types::value::encode_key(&Value::Integer(id)), &crate::types::row::encode_row(&schema, &row)).unwrap();
            }
            bt.root()
        };
        schema.root_page = final_root;

        let predicate = crate::sql::ast::Expr::BinaryOp {
            op: crate::sql::ast::BinOp::Gt,
            left: Box::new(Expr::Column("id".into())),
            right: Box::new(Expr::IntLiteral(1)),
        };
        let mut plan = build_select_plan(&schema, &[], Some(predicate), vec![1], None, None).unwrap(); // project just "name"

        let mut rows = Vec::new();
        while let Some(row) = plan.next(&mut pager).unwrap() {
            rows.push(row);
        }
        assert_eq!(rows, vec![vec![Value::Text("b".into())], vec![Value::Text("c".into())]]);
    }

    #[test]
    fn pk_equality_predicate_uses_table_seek_and_touches_few_pages() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let initial_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(initial_root).unwrap());

        let mut schema = TableSchema {
            name: "t".into(),
            columns: vec![Column { name: "id".into(), ty: ColumnType::Integer, not_null: true, is_primary_key: true }],
            root_page: initial_root,
            rls_enabled: false,
        };
        let final_root = {
            let mut bt = crate::btree::tree::BTree::new(&mut pager, initial_root);
            for i in 0..5000i64 {
                let row = vec![Value::Integer(i)];
                bt.insert(&crate::types::value::encode_key(&Value::Integer(i)), &crate::types::row::encode_row(&schema, &row)).unwrap();
            }
            bt.root()
        };
        schema.root_page = final_root;
        pager.flush().unwrap();
        // Force a genuinely cold cache before measuring: the pager's LRU cache
        // holds 256 pages, and the tree built above easily fits, so every page
        // touched during setup is still resident. Without reopening, pages_read
        // would report 0 for ANY query afterward (TableSeek or a full SeqScan
        // alike), making the assertion below vacuously true regardless of which
        // operator actually ran -- proving nothing about page efficiency.
        drop(pager);
        let mut pager = Pager::open(file.path()).unwrap();

        let predicate = Expr::BinaryOp {
            op: crate::sql::ast::BinOp::Eq,
            left: Box::new(Expr::Column("id".into())),
            right: Box::new(Expr::IntLiteral(2500)),
        };
        pager.reset_read_counter();
        let mut plan = build_select_plan(&schema, &[], Some(predicate), vec![0], None, None).unwrap();
        let mut rows = Vec::new();
        while let Some(row) = plan.next(&mut pager).unwrap() {
            rows.push(row);
        }
        assert_eq!(rows, vec![vec![Value::Integer(2500)]]);
        let table_seek_pages = pager.stats().pages_read;
        assert!(
            table_seek_pages < 20,
            "a PK-equality lookup on a 5000-row tree should touch only a handful of pages, touched {table_seek_pages}"
        );

        // Directly prove the optimization's value, not just an absolute threshold:
        // reopen again (cold cache) and run the SAME predicate through a hand-built
        // SeqScan + Filter plan (Task 28's pre-TableSeek behavior), which must walk
        // the entire leaf chain and therefore read strictly more pages.
        drop(pager);
        let mut pager = Pager::open(file.path()).unwrap();
        pager.reset_read_counter();
        let seq_scan: Box<dyn crate::exec::Operator> = Box::new(SeqScan::new(schema.clone()));
        let mut seq_plan = Filter {
            input: seq_scan,
            schema: schema.clone(),
            predicate: predicate_for_seq_scan(),
            context: crate::auth::ExecutionContext::anonymous(),
        };
        let mut seq_rows = Vec::new();
        while let Some(row) = seq_plan.next(&mut pager).unwrap() {
            seq_rows.push(row);
        }
        assert_eq!(seq_rows, vec![vec![Value::Integer(2500)]]);
        let seq_scan_pages = pager.stats().pages_read;
        assert!(
            table_seek_pages < seq_scan_pages,
            "TableSeek ({table_seek_pages} pages) should read strictly fewer pages than a full SeqScan ({seq_scan_pages} pages) for the same predicate"
        );
    }

    fn predicate_for_seq_scan() -> Expr {
        Expr::BinaryOp {
            op: crate::sql::ast::BinOp::Eq,
            left: Box::new(Expr::Column("id".into())),
            right: Box::new(Expr::IntLiteral(2500)),
        }
    }
}
