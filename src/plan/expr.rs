use crate::error::PlanError;
use crate::sql::ast::{BinOp, Expr};
use crate::types::schema::TableSchema;
use crate::types::value::{sql_cmp, Value};

pub fn eval(expr: &Expr, schema: &TableSchema, row: &[Value]) -> Result<Value, PlanError> {
    match expr {
        Expr::Column(name) => {
            let idx = schema.column_index(name).ok_or_else(|| PlanError::NoSuchColumn(name.clone()))?;
            Ok(row[idx].clone())
        }
        Expr::IntLiteral(i) => Ok(Value::Integer(*i)),
        Expr::StringLiteral(s) => Ok(Value::Text(s.clone())),
        Expr::BoolLiteral(b) => Ok(Value::Boolean(*b)),
        Expr::Null => Ok(Value::Null),
        Expr::IsNull { expr, negated } => {
            let v = eval(expr, schema, row)?;
            let is_null = matches!(v, Value::Null);
            Ok(Value::Boolean(is_null != *negated))
        }
        Expr::BinaryOp { op, left, right } => {
            let l = eval(left, schema, row)?;
            let r = eval(right, schema, row)?;
            Ok(eval_binop(*op, &l, &r))
        }
    }
}

fn eval_binop(op: BinOp, l: &Value, r: &Value) -> Value {
    match op {
        BinOp::And => Value::Boolean(is_truthy(l) && is_truthy(r)),
        BinOp::Or => Value::Boolean(is_truthy(l) || is_truthy(r)),
        _ => {
            if matches!(l, Value::Null) || matches!(r, Value::Null) {
                return Value::Boolean(false);
            }
            let ord = sql_cmp(l, r);
            let result = match op {
                BinOp::Eq => ord == std::cmp::Ordering::Equal,
                BinOp::NotEq => ord != std::cmp::Ordering::Equal,
                BinOp::Lt => ord == std::cmp::Ordering::Less,
                BinOp::LtEq => ord != std::cmp::Ordering::Greater,
                BinOp::Gt => ord == std::cmp::Ordering::Greater,
                BinOp::GtEq => ord != std::cmp::Ordering::Less,
                BinOp::And | BinOp::Or => unreachable!(),
            };
            Value::Boolean(result)
        }
    }
}

pub fn is_truthy(v: &Value) -> bool {
    matches!(v, Value::Boolean(true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::schema::Column;
    use crate::types::value::ColumnType;

    fn schema() -> TableSchema {
        TableSchema {
            name: "t".into(),
            columns: vec![
                Column { name: "id".into(), ty: ColumnType::Integer, not_null: true, is_primary_key: true },
                Column { name: "name".into(), ty: ColumnType::Text, not_null: false, is_primary_key: false },
            ],
            root_page: 0,
        }
    }

    #[test]
    fn evaluates_column_reference() {
        let row = vec![Value::Integer(7), Value::Text("x".into())];
        assert_eq!(eval(&Expr::Column("id".into()), &schema(), &row).unwrap(), Value::Integer(7));
    }

    #[test]
    fn unknown_column_errors() {
        let row = vec![Value::Integer(7), Value::Text("x".into())];
        assert!(matches!(eval(&Expr::Column("nope".into()), &schema(), &row), Err(PlanError::NoSuchColumn(_))));
    }

    #[test]
    fn equality_comparison() {
        let row = vec![Value::Integer(7), Value::Text("x".into())];
        let expr = Expr::BinaryOp {
            op: BinOp::Eq,
            left: Box::new(Expr::Column("id".into())),
            right: Box::new(Expr::IntLiteral(7)),
        };
        assert_eq!(eval(&expr, &schema(), &row).unwrap(), Value::Boolean(true));
    }

    #[test]
    fn comparison_against_null_is_false_not_panic() {
        let row = vec![Value::Integer(7), Value::Null];
        let expr = Expr::BinaryOp {
            op: BinOp::Eq,
            left: Box::new(Expr::Column("name".into())),
            right: Box::new(Expr::StringLiteral("x".into())),
        };
        assert_eq!(eval(&expr, &schema(), &row).unwrap(), Value::Boolean(false));
    }

    #[test]
    fn is_null_and_is_not_null() {
        let row = vec![Value::Integer(7), Value::Null];
        let is_null = Expr::IsNull { expr: Box::new(Expr::Column("name".into())), negated: false };
        let is_not_null = Expr::IsNull { expr: Box::new(Expr::Column("name".into())), negated: true };
        assert_eq!(eval(&is_null, &schema(), &row).unwrap(), Value::Boolean(true));
        assert_eq!(eval(&is_not_null, &schema(), &row).unwrap(), Value::Boolean(false));
    }

    #[test]
    fn and_or_short_circuit_semantics() {
        let row = vec![Value::Integer(7), Value::Text("x".into())];
        let and_expr = Expr::BinaryOp {
            op: BinOp::And,
            left: Box::new(Expr::BoolLiteral(true)),
            right: Box::new(Expr::BoolLiteral(false)),
        };
        assert_eq!(eval(&and_expr, &schema(), &row).unwrap(), Value::Boolean(false));
    }
}
