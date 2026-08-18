//! Relational Aggregation and Grouping execution operator.

use std::collections::HashMap;

use crate::error::ExecError;
use crate::exec::Operator;
use crate::sql::ast::{AggregateFunc, Expr};
use crate::storage::pager::Pager;
use crate::types::schema::TableSchema;
use crate::types::value::{sql_cmp, Value};

/// Aggregate accumulator state.
#[derive(Debug, Clone)]
enum Accumulator {
    CountStar(i64),
    Count(i64),
    Sum(i64),
    Avg { sum: i64, count: i64 },
    Min(Option<Value>),
    Max(Option<Value>),
}

impl Accumulator {
    fn new(func: &AggregateFunc) -> Self {
        match func {
            AggregateFunc::CountStar => Accumulator::CountStar(0),
            AggregateFunc::Count(_) => Accumulator::Count(0),
            AggregateFunc::Sum(_) => Accumulator::Sum(0),
            AggregateFunc::Avg(_) => Accumulator::Avg { sum: 0, count: 0 },
            AggregateFunc::Min(_) => Accumulator::Min(None),
            AggregateFunc::Max(_) => Accumulator::Max(None),
        }
    }

    fn update(&mut self, val: Option<&Value>) {
        match self {
            Accumulator::CountStar(c) => *c += 1,
            Accumulator::Count(c) => {
                if let Some(v) = val {
                    if !matches!(v, Value::Null) {
                        *c += 1;
                    }
                }
            }
            Accumulator::Sum(s) => {
                if let Some(Value::Integer(n)) = val {
                    *s += n;
                }
            }
            Accumulator::Avg { sum, count } => {
                if let Some(Value::Integer(n)) = val {
                    *sum += n;
                    *count += 1;
                }
            }
            Accumulator::Min(cur) => {
                if let Some(v) = val {
                    if !matches!(v, Value::Null) {
                        match cur {
                            None => *cur = Some(v.clone()),
                            Some(existing) => {
                                if sql_cmp(v, existing) == std::cmp::Ordering::Less {
                                    *cur = Some(v.clone());
                                }
                            }
                        }
                    }
                }
            }
            Accumulator::Max(cur) => {
                if let Some(v) = val {
                    if !matches!(v, Value::Null) {
                        match cur {
                            None => *cur = Some(v.clone()),
                            Some(existing) => {
                                if sql_cmp(v, existing) == std::cmp::Ordering::Greater {
                                    *cur = Some(v.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn finalize(&self) -> Value {
        match self {
            Accumulator::CountStar(c) => Value::Integer(*c),
            Accumulator::Count(c) => Value::Integer(*c),
            Accumulator::Sum(s) => Value::Integer(*s),
            Accumulator::Avg { sum, count } => {
                if *count == 0 {
                    Value::Null
                } else {
                    Value::Integer(sum / count)
                }
            }
            Accumulator::Min(v) => v.clone().unwrap_or(Value::Null),
            Accumulator::Max(v) => v.clone().unwrap_or(Value::Null),
        }
    }
}

/// Aggregation execution operator.
pub struct AggregateOperator {
    child: Box<dyn Operator>,
    group_by: Option<Vec<Expr>>,
    aggregates: Vec<AggregateFunc>,
    having: Option<Expr>,
    input_schema: TableSchema,

    output_rows: Option<Vec<Vec<Value>>>,
    cursor: usize,
}

impl AggregateOperator {
    pub fn new(
        child: Box<dyn Operator>,
        input_schema: TableSchema,
        group_by: Option<Vec<Expr>>,
        aggregates: Vec<AggregateFunc>,
        having: Option<Expr>,
    ) -> Self {
        Self {
            child,
            group_by,
            aggregates,
            having,
            input_schema,
            output_rows: None,
            cursor: 0,
        }
    }

    fn compute_aggregates(&mut self, pager: &mut Pager) -> Result<(), ExecError> {
        let mut groups: HashMap<Vec<Value>, Vec<Accumulator>> = HashMap::new();
        let mut group_order: Vec<Vec<Value>> = Vec::new();
        let mut saw_rows = false;

        while let Some(row) = self.child.next(pager)? {
            saw_rows = true;
            let group_key: Vec<Value> = match &self.group_by {
                Some(exprs) => {
                    let mut key = Vec::with_capacity(exprs.len());
                    for expr in exprs {
                        key.push(crate::plan::expr::eval(expr, &self.input_schema, &row).map_err(|e| ExecError::InvalidValue(e.to_string()))?);
                    }
                    key
                }
                None => Vec::new(),
            };

            let accumulators = groups.entry(group_key.clone()).or_insert_with(|| {
                group_order.push(group_key);
                self.aggregates.iter().map(Accumulator::new).collect()
            });

            for (acc, func) in accumulators.iter_mut().zip(&self.aggregates) {
                match func {
                    AggregateFunc::CountStar => acc.update(None),
                    AggregateFunc::Count(e)
                    | AggregateFunc::Sum(e)
                    | AggregateFunc::Avg(e)
                    | AggregateFunc::Min(e)
                    | AggregateFunc::Max(e) => {
                        let val = crate::plan::expr::eval(&e, &self.input_schema, &row).map_err(|err| ExecError::InvalidValue(err.to_string()))?;
                        acc.update(Some(&val));
                    }
                }
            }
        }

        // If no rows were seen and there is no GROUP BY, produce single row with default aggregates (e.g. COUNT(*) = 0)
        if !saw_rows && self.group_by.is_none() {
            let empty_key = Vec::new();
            let default_accs: Vec<Accumulator> = self.aggregates.iter().map(Accumulator::new).collect();
            groups.insert(empty_key.clone(), default_accs);
            group_order.push(empty_key);
        }

        let mut final_rows = Vec::new();
        for key in group_order {
            let accs = &groups[&key];
            let mut out_row = key.clone();
            for acc in accs {
                out_row.push(acc.finalize());
            }

            // Evaluate optional HAVING filter
            let include = match &self.having {
                Some(cond) => {
                    match crate::plan::expr::eval(cond, &self.input_schema, &out_row) {
                        Ok(v) => crate::plan::expr::is_truthy(&v),
                        Err(_) => true,
                    }
                }
                None => true,
            };

            if include {
                final_rows.push(out_row);
            }
        }

        self.output_rows = Some(final_rows);
        Ok(())
    }
}

impl Operator for AggregateOperator {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError> {
        if self.output_rows.is_none() {
            self.compute_aggregates(pager)?;
        }

        let rows = self.output_rows.as_ref().unwrap();
        if self.cursor < rows.len() {
            let row = rows[self.cursor].clone();
            self.cursor += 1;
            Ok(Some(row))
        } else {
            Ok(None)
        }
    }
}
