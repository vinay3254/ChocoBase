use crate::error::ExecError;
use crate::exec::Operator;
use crate::plan::expr::{eval, is_truthy};
use crate::sql::ast::Expr;
use crate::storage::pager::Pager;
use crate::types::schema::TableSchema;
use crate::types::value::Value;

pub struct Filter {
    pub input: Box<dyn Operator>,
    pub schema: TableSchema,
    pub predicate: Expr,
}

impl Operator for Filter {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError> {
        loop {
            match self.input.next(pager)? {
                Some(row) => {
                    let v = eval(&self.predicate, &self.schema, &row)
                        .map_err(|e| ExecError::InvalidValue(e.to_string()))?;
                    if is_truthy(&v) {
                        return Ok(Some(row));
                    }
                }
                None => return Ok(None),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::scan::SeqScan;
    use crate::sql::ast::BinOp;
    use crate::types::schema::Column;
    use crate::types::value::ColumnType;

    // Filter is exercised end-to-end (with a real SeqScan feeding it) in Task 29's
    // integration test; this unit test only checks the truthiness/loop logic using
    // a trivial in-memory Operator stub so it doesn't need a Pager at all.
    struct Fixed(Vec<Vec<Value>>);
    impl Operator for Fixed {
        fn next(&mut self, _pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError> {
            Ok(self.0.pop())
        }
    }

    #[test]
    fn filters_out_non_matching_rows() {
        use tempfile::NamedTempFile;
        let file = NamedTempFile::new().unwrap();
        let mut pager = crate::storage::pager::Pager::create(file.path()).unwrap();

        let schema = TableSchema {
            name: "t".into(),
            columns: vec![Column { name: "id".into(), ty: ColumnType::Integer, not_null: true, is_primary_key: true }],
            root_page: 0,
        };
        // `Fixed` yields via Vec::pop(), which returns the LAST element first: this
        // list is consumed in the order 1, 2, 3 (not the 3, 2, 1 it's written in).
        let input = Fixed(vec![
            vec![Value::Integer(3)],
            vec![Value::Integer(2)],
            vec![Value::Integer(1)],
        ]);
        let predicate = Expr::BinaryOp {
            op: BinOp::Gt,
            left: Box::new(Expr::Column("id".into())),
            right: Box::new(Expr::IntLiteral(1)),
        };
        let mut filter = Filter { input: Box::new(input), schema, predicate };
        let mut seen = Vec::new();
        while let Some(row) = filter.next(&mut pager).unwrap() {
            seen.push(row[0].clone());
        }
        // Stream order is [1, 2, 3]; id > 1 excludes 1 and keeps 2 and 3.
        assert_eq!(seen, vec![Value::Integer(2), Value::Integer(3)]);
    }
}
