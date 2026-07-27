use crate::exec::filter::Filter;
use crate::exec::project::Project;
use crate::exec::scan::{SeqScan, TableSeek};
use crate::exec::Operator;
use crate::sql::ast::{BinOp, Expr};
use crate::types::schema::TableSchema;
use crate::types::value::{encode_key, Value};

pub fn build_select_plan(schema: &TableSchema, where_clause: Option<Expr>, projection_indices: Vec<usize>) -> Box<dyn Operator> {
    let pk_col = schema.columns[schema.primary_key_index()].name.clone();
    let (pk_value, residual) = extract_pk_equality(where_clause, &pk_col);

    let mut plan: Box<dyn Operator> = match pk_value {
        Some(v) => Box::new(TableSeek::new(schema.clone(), encode_key(&v))),
        None => Box::new(SeqScan::new(schema.clone())),
    };
    if let Some(predicate) = residual {
        plan = Box::new(Filter { input: plan, schema: schema.clone(), predicate });
    }
    Box::new(Project { input: plan, indices: projection_indices })
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
fn extract_pk_equality(where_clause: Option<Expr>, pk_col: &str) -> (Option<Value>, Option<Expr>) {
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
        let mut plan = build_select_plan(&schema, Some(predicate), vec![1]); // project just "name"

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

        let predicate = Expr::BinaryOp {
            op: crate::sql::ast::BinOp::Eq,
            left: Box::new(Expr::Column("id".into())),
            right: Box::new(Expr::IntLiteral(2500)),
        };
        pager.reset_read_counter();
        let mut plan = build_select_plan(&schema, Some(predicate), vec![0]);
        let mut rows = Vec::new();
        while let Some(row) = plan.next(&mut pager).unwrap() {
            rows.push(row);
        }
        assert_eq!(rows, vec![vec![Value::Integer(2500)]]);
        assert!(
            pager.stats().pages_read < 20,
            "a PK-equality lookup on a 5000-row tree should touch only a handful of pages, touched {}",
            pager.stats().pages_read
        );
    }
}
