//! Relational Join execution operators.

use crate::error::ExecError;
use crate::exec::Operator;
use crate::sql::ast::{Expr, JoinType};
use crate::storage::pager::Pager;
use crate::types::schema::TableSchema;
use crate::types::value::Value;

/// Nested Loop Join operator for executing inner, left, and cross joins between relations.
pub struct NestedLoopJoin {
    left: Box<dyn Operator>,
    right_rows: Option<Vec<Vec<Value>>>,
    right_operator: Option<Box<dyn Operator>>,
    right_schema_len: usize,
    join_type: JoinType,
    condition: Option<Expr>,
    combined_schema: TableSchema,

    current_left: Option<Vec<Value>>,
    right_cursor: usize,
    matched_left: bool,
}

impl NestedLoopJoin {
    pub fn new(
        left: Box<dyn Operator>,
        right: Box<dyn Operator>,
        right_schema_len: usize,
        join_type: JoinType,
        condition: Option<Expr>,
        combined_schema: TableSchema,
    ) -> Self {
        Self {
            left,
            right_rows: None,
            right_operator: Some(right),
            right_schema_len,
            join_type,
            condition,
            combined_schema,
            current_left: None,
            right_cursor: 0,
            matched_left: false,
        }
    }

    fn materialize_right_if_needed(&mut self, pager: &mut Pager) -> Result<(), ExecError> {
        if self.right_rows.is_none() {
            if let Some(mut right_op) = self.right_operator.take() {
                let mut rows = Vec::new();
                while let Some(row) = right_op.next(pager)? {
                    rows.push(row);
                }
                self.right_rows = Some(rows);
            } else {
                self.right_rows = Some(Vec::new());
            }
        }
        Ok(())
    }
}

impl Operator for NestedLoopJoin {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError> {
        self.materialize_right_if_needed(pager)?;
        let right_rows = self.right_rows.as_ref().unwrap();

        loop {
            if self.current_left.is_none() {
                match self.left.next(pager)? {
                    Some(l_row) => {
                        self.current_left = Some(l_row);
                        self.right_cursor = 0;
                        self.matched_left = false;
                    }
                    None => return Ok(None),
                }
            }

            let l_row = self.current_left.as_ref().unwrap();

            while self.right_cursor < right_rows.len() {
                let r_row = &right_rows[self.right_cursor];
                self.right_cursor += 1;

                let mut combined = l_row.clone();
                combined.extend(r_row.iter().cloned());

                let is_match = match &self.condition {
                    Some(cond) => {
                        match crate::plan::expr::eval(cond, &self.combined_schema, &combined) {
                            Ok(v) => crate::plan::expr::is_truthy(&v),
                            Err(_) => false,
                        }
                    }
                    None => true,
                };

                if is_match {
                    self.matched_left = true;
                    return Ok(Some(combined));
                }
            }

            // Handle LEFT join with non-matching left row
            if self.join_type == JoinType::Left && !self.matched_left {
                let mut combined = l_row.clone();
                combined.extend(vec![Value::Null; self.right_schema_len]);
                self.current_left = None;
                return Ok(Some(combined));
            }

            self.current_left = None;
        }
    }
}
