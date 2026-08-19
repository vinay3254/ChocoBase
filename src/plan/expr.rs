use crate::error::PlanError;
use crate::sql::ast::{BinOp, Expr};
use crate::types::schema::TableSchema;
use crate::types::value::{sql_cmp, Value};

pub fn eval(expr: &Expr, schema: &TableSchema, row: &[Value]) -> Result<Value, PlanError> {
    eval_with_context(
        expr,
        schema,
        row,
        &crate::auth::ExecutionContext::anonymous(),
    )
}

pub fn eval_with_context(
    expr: &Expr,
    schema: &TableSchema,
    row: &[Value],
    ctx: &crate::auth::ExecutionContext,
) -> Result<Value, PlanError> {
    match expr {
        Expr::AuthUid => match ctx.user_id {
            Some(uid) => Ok(Value::Integer(uid)),
            None => Ok(Value::Null),
        },
        Expr::Column(name) => {
            let idx = schema
                .column_index(name)
                .ok_or_else(|| PlanError::NoSuchColumn(name.clone()))?;
            Ok(row[idx].clone())
        }
        Expr::QualifiedColumn { table, column } => {
            let qual = format!("{table}.{column}");
            let idx = schema
                .column_index(&qual)
                .or_else(|| schema.column_index(column))
                .ok_or(PlanError::NoSuchColumn(qual))?;
            Ok(row[idx].clone())
        }
        Expr::IntLiteral(i) => Ok(Value::Integer(*i)),
        Expr::StringLiteral(s) => Ok(Value::Text(s.clone())),
        Expr::BoolLiteral(b) => Ok(Value::Boolean(*b)),
        Expr::Null => Ok(Value::Null),
        Expr::IsNull { expr, negated } => {
            let v = eval_with_context(expr, schema, row, ctx)?;
            let is_null = matches!(v, Value::Null);
            Ok(Value::Boolean(is_null != *negated))
        }
        Expr::BinaryOp { op, left, right } => {
            let l = eval_with_context(left, schema, row, ctx)?;
            let r = eval_with_context(right, schema, row, ctx)?;
            Ok(eval_binop(*op, &l, &r))
        }
        Expr::InList {
            expr,
            list,
            negated,
        } => {
            let target = eval_with_context(expr, schema, row, ctx)?;
            if matches!(target, Value::Null) {
                return Ok(Value::Null);
            }
            let mut found = false;
            for item in list {
                let candidate = eval_with_context(item, schema, row, ctx)?;
                if target == candidate {
                    found = true;
                    break;
                }
            }
            Ok(Value::Boolean(if *negated { !found } else { found }))
        }
        Expr::Like {
            expr,
            pattern,
            negated,
        } => {
            let val = eval_with_context(expr, schema, row, ctx)?;
            let text = match &val {
                Value::Text(s) => s.as_str(),
                Value::Null => return Ok(Value::Null),
                _ => "",
            };
            let text_chars: Vec<char> = text.chars().collect();
            let pat_chars: Vec<char> = pattern.chars().collect();
            let is_match = like_match_chars(&text_chars, &pat_chars, 0, 0);
            Ok(Value::Boolean(if *negated { !is_match } else { is_match }))
        }
        Expr::Aggregate(_) => Err(PlanError::InvalidExpression(
            "aggregate functions cannot be evaluated directly in row context".into(),
        )),
        Expr::InSubquery { .. } | Expr::Exists { .. } => Err(PlanError::InvalidExpression(
            "subqueries must be resolved before row-level evaluation".into(),
        )),
        Expr::JsonExtract {
            expr,
            path,
            as_text,
        } => {
            let val = eval_with_context(expr, schema, row, ctx)?;
            let json_str = match &val {
                Value::Json(s) | Value::Text(s) => s.as_str(),
                Value::Null => return Ok(Value::Null),
                _ => return Ok(Value::Null),
            };

            let parsed: serde_json::Value = match serde_json::from_str(json_str) {
                Ok(v) => v,
                Err(_) => return Ok(Value::Null),
            };

            let normalized_path = path
                .strip_prefix("$.")
                .unwrap_or(path.strip_prefix('$').unwrap_or(path.as_str()));
            let mut current = &parsed;
            if !normalized_path.is_empty() {
                for segment in normalized_path.split('.') {
                    match current {
                        serde_json::Value::Object(map) => {
                            if let Some(next) = map.get(segment) {
                                current = next;
                            } else {
                                return Ok(Value::Null);
                            }
                        }
                        serde_json::Value::Array(arr) => {
                            if let Ok(idx) = segment.parse::<usize>() {
                                if let Some(next) = arr.get(idx) {
                                    current = next;
                                } else {
                                    return Ok(Value::Null);
                                }
                            } else {
                                return Ok(Value::Null);
                            }
                        }
                        _ => return Ok(Value::Null),
                    }
                }
            }

            match current {
                serde_json::Value::Null => Ok(Value::Null),
                serde_json::Value::Bool(b) => Ok(Value::Boolean(*b)),
                serde_json::Value::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        Ok(Value::Integer(i))
                    } else if *as_text {
                        Ok(Value::Text(n.to_string()))
                    } else {
                        Ok(Value::Json(n.to_string()))
                    }
                }
                serde_json::Value::String(s) => Ok(Value::Text(s.clone())),
                serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                    let serialized = serde_json::to_string(current).unwrap_or_default();
                    if *as_text {
                        Ok(Value::Text(serialized))
                    } else {
                        Ok(Value::Json(serialized))
                    }
                }
            }
        }
        Expr::FloatLiteral(f) => Ok(Value::Float(*f)),
        Expr::VectorDistance {
            metric,
            left,
            right,
        } => {
            let l_val = eval_with_context(left, schema, row, ctx)?;
            let r_val = eval_with_context(right, schema, row, ctx)?;
            let l_vec = match parse_vector(&l_val) {
                Some(v) => v,
                None => return Ok(Value::Null),
            };
            let r_vec = match parse_vector(&r_val) {
                Some(v) => v,
                None => return Ok(Value::Null),
            };
            let dist = match metric {
                crate::sql::ast::VectorMetric::Cosine => cosine_distance(&l_vec, &r_vec),
                crate::sql::ast::VectorMetric::L2 => l2_distance(&l_vec, &r_vec),
                crate::sql::ast::VectorMetric::InnerProduct => inner_product(&l_vec, &r_vec),
            };
            Ok(Value::Float(dist))
        }
        Expr::FtsMatch { expr, query } => {
            let val = eval_with_context(expr, schema, row, ctx)?;
            let text = match &val {
                Value::Text(s) | Value::Json(s) => s.as_str(),
                _ => return Ok(Value::Boolean(false)),
            };
            Ok(Value::Boolean(fts_matches(text, query)))
        }
        Expr::FtsRank { expr, query } => {
            let val = eval_with_context(expr, schema, row, ctx)?;
            let text = match &val {
                Value::Text(s) | Value::Json(s) => s.as_str(),
                _ => return Ok(Value::Float(0.0)),
            };
            Ok(Value::Float(fts_rank(text, query)))
        }
        Expr::FtsSnippet { expr, query } => {
            let val = eval_with_context(expr, schema, row, ctx)?;
            let text = match &val {
                Value::Text(s) | Value::Json(s) => s.as_str(),
                _ => return Ok(Value::Text(String::new())),
            };
            Ok(Value::Text(fts_snippet(text, query)))
        }
    }
}

pub fn fts_snippet(doc: &str, query: &str) -> String {
    let query_tokens: Vec<String> = tokenize_words(query)
        .into_iter()
        .map(|q| q.trim_end_matches(":*").trim_end_matches('*').to_string())
        .filter(|q| !q.is_empty())
        .collect();

    if query_tokens.is_empty() || doc.is_empty() {
        return doc.to_string();
    }

    let words: Vec<&str> = doc.split_whitespace().collect();
    if words.is_empty() {
        return doc.to_string();
    }

    let mut first_match_idx = None;
    for (idx, w) in words.iter().enumerate() {
        let clean = w
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase();
        if query_tokens
            .iter()
            .any(|q| clean == *q || (clean.len() >= 3 && clean.contains(q)))
        {
            first_match_idx = Some(idx);
            break;
        }
    }

    let match_idx = first_match_idx.unwrap_or(0);
    let start_idx = match_idx.saturating_sub(4);
    let end_idx = (match_idx + 5).min(words.len());

    let mut parts: Vec<String> = Vec::new();
    if start_idx > 0 {
        parts.push("...".to_string());
    }

    for &w in &words[start_idx..end_idx] {
        let clean = w
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase();
        if query_tokens
            .iter()
            .any(|q| clean == *q || (clean.len() >= 3 && clean.contains(q)))
        {
            parts.push(format!("<b>{w}</b>"));
        } else {
            parts.push(w.to_string());
        }
    }

    if end_idx < words.len() {
        parts.push("...".to_string());
    }

    parts.join(" ")
}

pub fn tokenize_words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric() && c != '*' && c != ':')
        .map(|w| w.trim().to_lowercase())
        .filter(|w| !w.is_empty())
        .collect()
}

pub fn fts_matches(doc: &str, query: &str) -> bool {
    let doc_tokens = tokenize_words(doc);
    let query_tokens = tokenize_words(query);
    if query_tokens.is_empty() {
        return false;
    }

    for q in &query_tokens {
        let is_prefix = q.ends_with(":*") || q.ends_with('*');
        let clean_q = q.trim_end_matches(":*").trim_end_matches('*');
        if clean_q.is_empty() {
            continue;
        }
        let matched = if is_prefix {
            doc_tokens.iter().any(|d| d.starts_with(clean_q))
        } else {
            doc_tokens.iter().any(|d| d == clean_q)
        };
        if !matched {
            return false;
        }
    }
    true
}

pub fn fts_rank(doc: &str, query: &str) -> f64 {
    let doc_tokens = tokenize_words(doc);
    let query_tokens = tokenize_words(query);
    if doc_tokens.is_empty() || query_tokens.is_empty() {
        return 0.0;
    }

    let mut score = 0.0f64;
    for q in &query_tokens {
        let is_prefix = q.ends_with(":*") || q.ends_with('*');
        let clean_q = q.trim_end_matches(":*").trim_end_matches('*');
        if clean_q.is_empty() {
            continue;
        }
        let count = if is_prefix {
            doc_tokens.iter().filter(|d| d.starts_with(clean_q)).count()
        } else {
            doc_tokens.iter().filter(|d| *d == clean_q).count()
        };

        if count > 0 {
            let tf = (count as f64) / (count as f64 + 1.2);
            score += tf;
        }
    }

    let length_norm = 1.0 / ((doc_tokens.len() as f64).sqrt() + 1.0);
    score * (1.0 + length_norm)
}

pub fn parse_vector(v: &Value) -> Option<Vec<f32>> {
    match v {
        Value::Vector(vec) => Some(vec.clone()),
        Value::Text(s) | Value::Json(s) => serde_json::from_str::<Vec<f32>>(s).ok(),
        _ => None,
    }
}

pub fn cosine_distance(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 1.0;
    }
    let mut dot = 0.0f64;
    let mut norm_a = 0.0f64;
    let mut norm_b = 0.0f64;
    for (x, y) in a.iter().zip(b.iter()) {
        let x64 = *x as f64;
        let y64 = *y as f64;
        dot += x64 * y64;
        norm_a += x64 * x64;
        norm_b += y64 * y64;
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        return 1.0;
    }
    1.0 - (dot / (norm_a.sqrt() * norm_b.sqrt()))
}

pub fn l2_distance(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() {
        return f64::MAX;
    }
    let mut sum = 0.0f64;
    for (x, y) in a.iter().zip(b.iter()) {
        let diff = (*x as f64) - (*y as f64);
        sum += diff * diff;
    }
    sum.sqrt()
}

pub fn inner_product(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f64;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += (*x as f64) * (*y as f64);
    }
    dot
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

fn like_match_chars(text: &[char], pat: &[char], t_idx: usize, p_idx: usize) -> bool {
    if p_idx == pat.len() {
        return t_idx == text.len();
    }
    if pat[p_idx] == '%' {
        if p_idx + 1 == pat.len() {
            return true;
        }
        for next_t in t_idx..=text.len() {
            if like_match_chars(text, pat, next_t, p_idx + 1) {
                return true;
            }
        }
        return false;
    }
    if t_idx < text.len() && (pat[p_idx] == '_' || pat[p_idx] == text[t_idx]) {
        return like_match_chars(text, pat, t_idx + 1, p_idx + 1);
    }
    false
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
                Column {
                    name: "id".into(),
                    ty: ColumnType::Integer,
                    not_null: true,
                    is_primary_key: true,
                },
                Column {
                    name: "name".into(),
                    ty: ColumnType::Text,
                    not_null: false,
                    is_primary_key: false,
                },
            ],
            root_page: 0,
            rls_enabled: false,
        }
    }

    #[test]
    fn evaluates_column_reference() {
        let row = vec![Value::Integer(7), Value::Text("x".into())];
        assert_eq!(
            eval(&Expr::Column("id".into()), &schema(), &row).unwrap(),
            Value::Integer(7)
        );
    }

    #[test]
    fn unknown_column_errors() {
        let row = vec![Value::Integer(7), Value::Text("x".into())];
        assert!(matches!(
            eval(&Expr::Column("nope".into()), &schema(), &row),
            Err(PlanError::NoSuchColumn(_))
        ));
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
        let is_null = Expr::IsNull {
            expr: Box::new(Expr::Column("name".into())),
            negated: false,
        };
        let is_not_null = Expr::IsNull {
            expr: Box::new(Expr::Column("name".into())),
            negated: true,
        };
        assert_eq!(
            eval(&is_null, &schema(), &row).unwrap(),
            Value::Boolean(true)
        );
        assert_eq!(
            eval(&is_not_null, &schema(), &row).unwrap(),
            Value::Boolean(false)
        );
    }

    #[test]
    fn and_or_short_circuit_semantics() {
        let row = vec![Value::Integer(7), Value::Text("x".into())];
        let and_expr = Expr::BinaryOp {
            op: BinOp::And,
            left: Box::new(Expr::BoolLiteral(true)),
            right: Box::new(Expr::BoolLiteral(false)),
        };
        assert_eq!(
            eval(&and_expr, &schema(), &row).unwrap(),
            Value::Boolean(false)
        );
    }
}
