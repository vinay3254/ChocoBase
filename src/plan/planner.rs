use crate::exec::filter::Filter;
use crate::exec::project::Project;
use crate::exec::scan::SeqScan;
use crate::exec::Operator;
use crate::sql::ast::Expr;
use crate::types::schema::TableSchema;

pub fn build_select_plan(schema: &TableSchema, where_clause: Option<Expr>, projection_indices: Vec<usize>) -> Box<dyn Operator> {
    let mut plan: Box<dyn Operator> = Box::new(SeqScan::new(schema.clone()));
    if let Some(predicate) = where_clause {
        plan = Box::new(Filter { input: plan, schema: schema.clone(), predicate });
    }
    Box::new(Project { input: plan, indices: projection_indices })
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
}
