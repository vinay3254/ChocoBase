use crate::error::ExecError;
use crate::exec::Operator;
use crate::plan::expr::{eval_with_context, is_truthy};
use crate::sql::ast::Expr;
use crate::storage::pager::Pager;
use crate::types::schema::TableSchema;
use crate::types::value::Value;

pub struct Filter {
    pub input: Box<dyn Operator>,
    pub schema: TableSchema,
    pub predicate: Expr,
    pub context: crate::auth::ExecutionContext,
}

impl Filter {
    pub fn new(
        input: Box<dyn Operator>,
        schema: TableSchema,
        predicate: Expr,
        context: crate::auth::ExecutionContext,
    ) -> Self {
        Self {
            input,
            schema,
            predicate,
            context,
        }
    }
}

impl Operator for Filter {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError> {
        loop {
            match self.input.next(pager)? {
                Some(row) => {
                    let v = eval_with_context(&self.predicate, &self.schema, &row, &self.context)
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
    use crate::sql::ast::BinOp;
    use crate::types::schema::Column;
    use crate::types::value::ColumnType;

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
            columns: vec![Column {
                name: "id".into(),
                ty: ColumnType::Integer,
                not_null: true,
                is_primary_key: true,
            }],
            root_page: 0,
            rls_enabled: false,
        };
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
        let mut filter = Filter {
            input: Box::new(input),
            schema,
            predicate,
            context: crate::auth::ExecutionContext::anonymous(),
        };
        let mut seen = Vec::new();
        while let Some(row) = filter.next(&mut pager).unwrap() {
            seen.push(row[0].clone());
        }
        assert_eq!(seen, vec![Value::Integer(2), Value::Integer(3)]);
    }
}
