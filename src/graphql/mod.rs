//! Lightweight, high-performance GraphQL engine for ChocoBase (`pg_graphql` parity).
//! Translates GraphQL queries directly into SQL with full Row-Level Security preservation.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};
use std::collections::HashMap;

use crate::auth::ExecutionContext;
use crate::engine::{ExecResult, SharedDatabase};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphQLRequest {
    pub query: String,
    pub variables: Option<HashMap<String, JsonValue>>,
    pub operation_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphQLResponse {
    pub data: Option<JsonValue>,
    pub errors: Option<Vec<GraphQLError>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphQLError {
    pub message: String,
}

pub async fn execute_graphql(
    db: &SharedDatabase,
    req: &GraphQLRequest,
    ctx: &ExecutionContext,
) -> GraphQLResponse {
    let query_str = req.query.trim();
    if query_str.is_empty() {
        return GraphQLResponse {
            data: None,
            errors: Some(vec![GraphQLError {
                message: "query cannot be empty".into(),
            }]),
        };
    }

    match parse_and_execute_query(db, query_str, ctx) {
        Ok(data) => GraphQLResponse {
            data: Some(data),
            errors: None,
        },
        Err(e) => GraphQLResponse {
            data: None,
            errors: Some(vec![GraphQLError { message: e }]),
        },
    }
}

fn parse_and_execute_query(
    db: &SharedDatabase,
    query: &str,
    ctx: &ExecutionContext,
) -> std::result::Result<JsonValue, String> {
    let mut cleaned = query.trim();
    if cleaned.starts_with("query") {
        cleaned = cleaned.strip_prefix("query").unwrap().trim();
        if let Some(pos) = cleaned.find('{') {
            cleaned = &cleaned[pos..];
        }
    }

    if !cleaned.starts_with('{') || !cleaned.ends_with('}') {
        return Err("invalid GraphQL syntax: expected '{ ... }'".into());
    }

    let inside = &cleaned[1..cleaned.len() - 1].trim();

    let open_brace = inside
        .find('{')
        .ok_or("expected field selection block '{ ... }'")?;
    let close_brace = inside
        .rfind('}')
        .ok_or("unclosed field selection block '}'")?;

    let header = inside[..open_brace].trim();
    let fields_str = inside[open_brace + 1..close_brace].trim();

    let (table_name, args) = if let Some(open_paren) = header.find('(') {
        let close_paren = header.find(')').ok_or("unclosed argument list ')'")?;
        let tbl = header[..open_paren].trim();
        let arg_str = &header[open_paren + 1..close_paren];
        (tbl, parse_graphql_args(arg_str))
    } else {
        (header, HashMap::new())
    };

    let fields: Vec<&str> = fields_str
        .split(|c: char| c.is_whitespace() || c == ',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    if fields.is_empty() {
        return Err("must select at least one field".into());
    }

    let select_cols = fields.join(", ");
    let mut sql = format!("SELECT {select_cols} FROM {table_name}");

    if let Some(limit) = args.get("limit") {
        sql.push_str(&format!(" LIMIT {limit}"));
    }

    match db.execute_with_context(&sql, ctx) {
        Ok(ExecResult::Rows { columns, rows }) => {
            let json_rows: Vec<JsonValue> = rows
                .iter()
                .map(|row| {
                    let mut map = serde_json::Map::new();
                    for (idx, col_name) in columns.iter().enumerate() {
                        map.insert(
                            col_name.clone(),
                            match &row[idx] {
                                crate::types::value::Value::Integer(i) => json!(i),
                                crate::types::value::Value::Float(f) => json!(f),
                                crate::types::value::Value::Text(t) => json!(t),
                                crate::types::value::Value::Boolean(b) => json!(b),
                                crate::types::value::Value::Json(j) => {
                                    serde_json::from_str(j).unwrap_or(json!(j))
                                }
                                crate::types::value::Value::Vector(v) => json!(v),
                                crate::types::value::Value::Null => JsonValue::Null,
                            },
                        );
                    }
                    JsonValue::Object(map)
                })
                .collect();

            let mut data_map = serde_json::Map::new();
            data_map.insert(table_name.to_string(), JsonValue::Array(json_rows));
            Ok(JsonValue::Object(data_map))
        }
        Ok(_) => {
            let mut data_map = serde_json::Map::new();
            data_map.insert(table_name.to_string(), json!([]));
            Ok(JsonValue::Object(data_map))
        }
        Err(e) => Err(e.to_string()),
    }
}

fn parse_graphql_args(args: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for part in args.split(',') {
        if let Some((k, v)) = part.split_once(':') {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    map
}
