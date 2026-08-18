use crate::error::ExecError;
use crate::exec::Operator;
use crate::storage::pager::Pager;
use crate::types::value::Value;

pub struct Project {
    pub input: Box<dyn Operator>,
    pub indices: Vec<usize>,
}

impl Operator for Project {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError> {
        match self.input.next(pager)? {
            Some(row) => Ok(Some(self.indices.iter().map(|&i| row[i].clone()).collect())),
            None => Ok(None),
        }
    }
}

pub struct ProjectExpr {
    pub input: Box<dyn Operator>,
    pub schema: crate::types::schema::TableSchema,
    pub exprs: Vec<crate::sql::ast::Expr>,
    pub context: crate::auth::ExecutionContext,
}

impl Operator for ProjectExpr {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError> {
        match self.input.next(pager)? {
            Some(row) => {
                let mut out = Vec::with_capacity(self.exprs.len());
                for expr in &self.exprs {
                    let val = crate::plan::expr::eval_with_context(
                        expr,
                        &self.schema,
                        &row,
                        &self.context,
                    )
                    .map_err(|e| ExecError::InvalidValue(e.to_string()))?;
                    out.push(val);
                }
                Ok(Some(out))
            }
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(Vec<Vec<Value>>);
    impl Operator for Fixed {
        fn next(&mut self, _pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError> {
            Ok(self.0.pop())
        }
    }

    #[test]
    fn projects_selected_columns_in_order() {
        use tempfile::NamedTempFile;
        let file = NamedTempFile::new().unwrap();
        let mut pager = crate::storage::pager::Pager::create(file.path()).unwrap();

        let input = Fixed(vec![vec![
            Value::Integer(1),
            Value::Text("a".into()),
            Value::Boolean(true),
        ]]);
        let mut project = Project {
            input: Box::new(input),
            indices: vec![2, 0],
        };
        let row = project.next(&mut pager).unwrap().unwrap();
        assert_eq!(row, vec![Value::Boolean(true), Value::Integer(1)]);
    }
}
