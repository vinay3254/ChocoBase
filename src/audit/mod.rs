//! Audit Logging & Compliance Security Engine for ChocoBase.
//! Tracks administrative, authentication, database mutation, and object storage events
//! in an immutable compliance ledger with structured JSON metadata.

use crate::auth::ExecutionContext;
use crate::engine::{ExecResult, SharedDatabase};
use crate::error::Result;
use crate::types::value::Value;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuditLogEntry {
    pub id: i64,
    pub user_id: Option<i64>,
    pub action: String,
    pub target: String,
    pub ip_address: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: u64,
}

pub fn ensure_audit_tables(db: &SharedDatabase) {
    let sql = "CREATE TABLE _audit_logs (id INTEGER PRIMARY KEY, user_id INTEGER, action TEXT NOT NULL, target TEXT NOT NULL, ip_address TEXT, metadata JSON, created_at INTEGER NOT NULL)";
    let _ = db.execute_with_context(sql, &ExecutionContext::admin());
}

pub fn record_audit_log(
    db: &SharedDatabase,
    user_id: Option<i64>,
    action: &str,
    target: &str,
    ip_address: Option<&str>,
    metadata: serde_json::Value,
) -> Result<()> {
    ensure_audit_tables(db);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let esc_action = action.replace('\'', "''");
    let esc_target = target.replace('\'', "''");
    let esc_meta = metadata.to_string().replace('\'', "''");
    let user_str = match user_id {
        Some(id) => id.to_string(),
        None => "NULL".to_string(),
    };
    let ip_str = match ip_address {
        Some(ip) => format!("'{}'", ip.replace('\'', "''")),
        None => "NULL".to_string(),
    };

    // Calculate next ID
    let next_id_sql = "SELECT id FROM _audit_logs ORDER BY id DESC";
    let next_id = match db.execute_with_context(next_id_sql, &ExecutionContext::admin()) {
        Ok(ExecResult::Rows { rows, .. }) if !rows.is_empty() => match &rows[0][0] {
            Value::Integer(i) => i + 1,
            _ => 1,
        },
        _ => 1,
    };

    let insert_sql = format!(
        "INSERT INTO _audit_logs (id, user_id, action, target, ip_address, metadata, created_at) VALUES ({next_id}, {user_str}, '{esc_action}', '{esc_target}', {ip_str}, '{esc_meta}', {now})"
    );
    db.execute_with_context(&insert_sql, &ExecutionContext::admin())?;
    Ok(())
}

pub fn query_audit_logs(
    db: &SharedDatabase,
    action_filter: Option<&str>,
    user_id_filter: Option<i64>,
    limit: usize,
) -> Result<Vec<AuditLogEntry>> {
    ensure_audit_tables(db);
    let mut where_clauses = Vec::new();
    if let Some(act) = action_filter {
        let esc = act.replace('\'', "''");
        where_clauses.push(format!("action = '{esc}'"));
    }
    if let Some(uid) = user_id_filter {
        where_clauses.push(format!("user_id = {uid}"));
    }

    let where_str = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_clauses.join(" AND "))
    };

    let sql = format!("SELECT id, user_id, action, target, ip_address, metadata, created_at FROM _audit_logs {where_str} ORDER BY id DESC");
    let mut entries = Vec::new();

    if let Ok(ExecResult::Rows { rows, .. }) =
        db.execute_with_context(&sql, &ExecutionContext::admin())
    {
        for r in rows.into_iter().take(limit) {
            let id = match &r[0] {
                Value::Integer(i) => *i,
                _ => continue,
            };
            let user_id = match &r[1] {
                Value::Integer(i) => Some(*i),
                _ => None,
            };
            let action = match &r[2] {
                Value::Text(s) => s.clone(),
                _ => String::new(),
            };
            let target = match &r[3] {
                Value::Text(s) => s.clone(),
                _ => String::new(),
            };
            let ip_address = match &r[4] {
                Value::Text(s) => Some(s.clone()),
                _ => None,
            };
            let metadata = match &r[5] {
                Value::Json(j) | Value::Text(j) => {
                    serde_json::from_str(j).unwrap_or(serde_json::Value::Null)
                }
                _ => serde_json::Value::Null,
            };
            let created_at = match &r[6] {
                Value::Integer(i) => *i as u64,
                _ => 0,
            };

            entries.push(AuditLogEntry {
                id,
                user_id,
                action,
                target,
                ip_address,
                metadata,
                created_at,
            });
        }
    }

    Ok(entries)
}
