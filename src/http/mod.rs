//! HTTP REST Gateway for ChocoBase.
//! Exposes JSON endpoints for SQL query execution, auto-generated table REST CRUD APIs,
//! authentication (signup/token), schema inspection, health checks, and dashboard.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;

use crate::auth::{sign_jwt, verify_jwt, verify_password, ExecutionContext, SessionClaims};
use crate::engine::{ExecResult, SharedDatabase};
use crate::error::Result;
use crate::types::value::Value;

pub mod dashboard;
pub mod functions;
pub mod realtime_channels;
pub mod storage;

use crate::functions::FunctionRegistry;
use realtime_channels::RealtimeChannelManager;

pub struct HttpServer {
    shutdown_tx: broadcast::Sender<()>,
}

impl HttpServer {
    pub async fn bind(addr: SocketAddr, db: SharedDatabase) -> Result<(Self, SocketAddr)> {
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        let (shutdown_tx, _) = broadcast::channel(1);
        let mut shutdown_rx = shutdown_tx.subscribe();

        let db = Arc::new(db);
        storage::ensure_storage_tables(&db);

        let functions_reg = Arc::new(FunctionRegistry::new());
        let realtime_mgr = Arc::new(RealtimeChannelManager::new());

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    accept_res = listener.accept() => {
                        match accept_res {
                            Ok((socket, _)) => {
                                let db_clone = Arc::clone(&db);
                                let func_clone = Arc::clone(&functions_reg);
                                let rt_clone = Arc::clone(&realtime_mgr);
                                tokio::spawn(async move {
                                    let _ = handle_http_connection(socket, db_clone, func_clone, rt_clone).await;
                                });
                            }
                            Err(_) => break,
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        break;
                    }
                }
            }
        });

        Ok((Self { shutdown_tx }, local_addr))
    }

    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }
}

async fn handle_http_connection(
    mut socket: TcpStream,
    db: Arc<SharedDatabase>,
    functions_reg: Arc<FunctionRegistry>,
    realtime_mgr: Arc<RealtimeChannelManager>,
) -> std::io::Result<()> {
    let mut buf = vec![0u8; 16384];
    let n = socket.read(&mut buf).await?;
    if n == 0 {
        return Ok(());
    }

    let req_str = String::from_utf8_lossy(&buf[..n]);
    let mut lines = req_str.lines();
    let request_line = match lines.next() {
        Some(line) => line,
        None => return Ok(()),
    };

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_uppercase();
    let full_path = parts.next().unwrap_or("/").to_string();

    // Parse headers
    let mut auth_token = None;
    for line in lines.by_ref() {
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("authorization") {
                let v = v.trim();
                if v.to_lowercase().starts_with("bearer ") {
                    auth_token = Some(v[7..].trim().to_string());
                }
            }
        }
    }

    let secret = crate::auth::jwt_secret();
    // Determine execution context from Authorization header (fail-closed: default to anonymous)
    let exec_ctx = if let Some(token) = auth_token {
        if let Ok(svc_key) = std::env::var("CHOCOBASE_SERVICE_ROLE_KEY") {
            if !svc_key.is_empty() && token == svc_key {
                ExecutionContext::admin()
            } else {
                match verify_jwt(&token, &secret) {
                    Ok(claims) => ExecutionContext::from_claims(&claims),
                    Err(_) => ExecutionContext::anonymous(),
                }
            }
        } else {
            match verify_jwt(&token, &secret) {
                Ok(claims) => ExecutionContext::from_claims(&claims),
                Err(_) => ExecutionContext::anonymous(),
            }
        }
    } else {
        ExecutionContext::anonymous()
    };

    if method == "OPTIONS" {
        let resp = "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, PATCH, DELETE, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization\r\nContent-Length: 0\r\n\r\n";
        socket.write_all(resp.as_bytes()).await?;
        return Ok(());
    }

    let (path, query_string) = match full_path.split_once('?') {
        Some((p, q)) => (p, q),
        None => (full_path.as_str(), ""),
    };

    // Extract body after \r\n\r\n
    let body = if let Some(idx) = req_str.find("\r\n\r\n") {
        &req_str[idx + 4..]
    } else {
        ""
    };

    const MAX_PAYLOAD_BYTES: usize = 10 * 1024 * 1024; // 10MB limit
    if body.len() > MAX_PAYLOAD_BYTES {
        let resp_json = serde_json::json!({ "error": "payload too large" });
        let resp_bytes = serde_json::to_vec(&resp_json).unwrap_or_default();
        let header = format!(
            "HTTP/1.1 413 Payload Too Large\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            resp_bytes.len()
        );
        socket.write_all(header.as_bytes()).await?;
        socket.write_all(&resp_bytes).await?;
        socket.flush().await?;
        return Ok(());
    }

    if method == "GET" && (path == "/" || path == "/dashboard") {
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            dashboard::DASHBOARD_HTML.len()
        );
        socket.write_all(header.as_bytes()).await?;
        socket
            .write_all(dashboard::DASHBOARD_HTML.as_bytes())
            .await?;
        socket.flush().await?;
        return Ok(());
    }

    if path.starts_with("/v1/storage/") {
        let (status_code, status_text, json_body, binary_data) =
            storage::handle_storage_request(&db, &method, path, query_string, body, &exec_ctx)
                .await;

        if let Some((bytes, content_type)) = binary_data {
            let header = format!(
                "HTTP/1.1 {status_code} {status_text}\r\nContent-Type: {content_type}\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            );
            socket.write_all(header.as_bytes()).await?;
            socket.write_all(&bytes).await?;
            socket.flush().await?;
            return Ok(());
        } else {
            let body_bytes = serde_json::to_vec(&json_body).unwrap_or_default();
            let header = format!(
                "HTTP/1.1 {status_code} {status_text}\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body_bytes.len()
            );
            socket.write_all(header.as_bytes()).await?;
            socket.write_all(&body_bytes).await?;
            socket.flush().await?;
            return Ok(());
        }
    }

    let (status_code, status_text, json_body) = if method == "GET" && path == "/v1/health" {
        (
            200,
            "OK",
            serde_json::json!({ "status": "healthy", "engine": "ChocoBase", "version": "0.1.0" }),
        )
    } else if method == "GET" && path == "/v1/tables" {
        let tables = db.list_tables();
        (200, "OK", serde_json::json!({ "tables": tables }))
    } else if method == "GET" && path.starts_with("/v1/tables/") {
        let table_name = &path["/v1/tables/".len()..];
        match db.table_schema(table_name) {
            Some(schema) => (
                200,
                "OK",
                serde_json::json!({ "table": table_name, "schema": schema }),
            ),
            None => (
                404,
                "Not Found",
                serde_json::json!({ "error": format!("table '{}' not found", table_name) }),
            ),
        }
    } else if method == "GET" && path == "/v1/metrics" {
        let stats = db.pager_stats();
        (
            200,
            "OK",
            serde_json::json!({ "page_count": stats.page_count, "pages_read": stats.pages_read, "cached_pages": stats.cached_pages }),
        )
    } else if method == "GET" && path == "/v1/admin/dump" {
        if !exec_ctx.is_admin {
            (
                403,
                "Forbidden",
                serde_json::json!({ "error": "admin privileges required" }),
            )
        } else {
            match db.dump_sql() {
                Ok(dump_sql) => (
                    200,
                    "OK",
                    serde_json::json!({ "status": "ok", "dump": dump_sql }),
                ),
                Err(e) => (
                    500,
                    "Internal Server Error",
                    serde_json::json!({ "error": e.to_string() }),
                ),
            }
        }
    } else if method == "POST" && path == "/v1/admin/restore" {
        if !exec_ctx.is_admin {
            (
                403,
                "Forbidden",
                serde_json::json!({ "error": "admin privileges required" }),
            )
        } else {
            let sql = if let Ok(parsed_json) = serde_json::from_str::<serde_json::Value>(body) {
                parsed_json
                    .get("sql")
                    .and_then(|s| s.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| body.to_string())
            } else {
                body.trim().to_string()
            };
            match db.restore_from_sql(&sql) {
                Ok(count) => (
                    200,
                    "OK",
                    serde_json::json!({ "status": "ok", "statements_executed": count }),
                ),
                Err(e) => (
                    400,
                    "Bad Request",
                    serde_json::json!({ "error": e.to_string() }),
                ),
            }
        }
    } else if method == "POST" && path == "/v1/sql" {
        let sql = if let Ok(parsed_json) = serde_json::from_str::<serde_json::Value>(body) {
            parsed_json
                .get("sql")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| body.to_string())
        } else {
            body.trim().to_string()
        };

        if sql.is_empty() {
            (
                400,
                "Bad Request",
                serde_json::json!({ "error": "missing sql query in request body" }),
            )
        } else {
            match db.execute_with_context(&sql, &exec_ctx) {
                Ok(result) => (
                    200,
                    "OK",
                    serde_json::json!({ "status": "ok", "result": result }),
                ),
                Err(err) => (
                    400,
                    "Bad Request",
                    serde_json::json!({ "status": "error", "error": err.to_string() }),
                ),
            }
        }
    } else if method == "POST" && path == "/v1/auth/signup" {
        handle_auth_signup(&db, body).await
    } else if method == "POST" && path == "/v1/auth/token" {
        handle_auth_token(&db, body).await
    } else if method == "POST" && path == "/v1/auth/refresh" {
        handle_auth_refresh(&db, body).await
    } else if path.starts_with("/v1/functions/v1") {
        functions::handle_functions_request(&functions_reg, &db, &method, path, body, &exec_ctx)
            .await
    } else if path.starts_with("/v1/realtime/v1") {
        realtime_channels::handle_realtime_channel_request(
            &realtime_mgr,
            &method,
            path,
            body,
            &exec_ctx,
        )
        .await
    } else if path.starts_with("/v1/rpc/") {
        let func_name = &path["/v1/rpc/".len()..];
        handle_rpc(&db, func_name, body, &exec_ctx).await
    } else if path.starts_with("/v1/rest/") {
        let table_name = &path["/v1/rest/".len()..];
        handle_rest_table_crud(&db, &method, table_name, query_string, body, &exec_ctx).await
    } else {
        (
            404,
            "Not Found",
            serde_json::json!({ "error": format!("endpoint '{}' not found", path) }),
        )
    };

    let body_bytes = serde_json::to_vec(&json_body).unwrap_or_default();
    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, PATCH, DELETE, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status_code,
        status_text,
        body_bytes.len()
    );

    socket.write_all(header.as_bytes()).await?;
    socket.write_all(&body_bytes).await?;
    socket.flush().await?;

    Ok(())
}

fn sanitize_identifier(s: &str) -> std::result::Result<String, &'static str> {
    if s.is_empty() || s.len() > 64 {
        return Err("identifier length must be between 1 and 64 characters");
    }
    if !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("identifier must contain only alphanumeric characters and underscores");
    }
    Ok(s.to_string())
}

fn escape_sql_string(s: &str) -> String {
    s.replace('\'', "''")
}

async fn handle_auth_refresh(
    _db: &SharedDatabase,
    body: &str,
) -> (u16, &'static str, serde_json::Value) {
    let payload: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => {
            return (
                400,
                "Bad Request",
                serde_json::json!({ "error": "invalid JSON body" }),
            )
        }
    };

    let refresh_token = match payload.get("refresh_token").and_then(|r| r.as_str()) {
        Some(t) => t,
        None => {
            return (
                400,
                "Bad Request",
                serde_json::json!({ "error": "missing refresh_token" }),
            )
        }
    };

    let secret = crate::auth::jwt_secret();
    match verify_jwt(refresh_token, &secret) {
        Ok(claims) => {
            let exp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                + 86400 * 7;
            let new_claims = SessionClaims::new(claims.sub, &claims.username, &claims.role, exp);
            let token = sign_jwt(&new_claims, &secret);
            let refresh = sign_jwt(&new_claims, &secret);

            (
                200,
                "OK",
                serde_json::json!({
                    "access_token": token,
                    "refresh_token": refresh,
                    "token_type": "bearer",
                    "expires_in": 86400 * 7,
                    "user": {
                        "id": claims.sub,
                        "username": claims.username,
                        "role": claims.role,
                    }
                }),
            )
        }
        Err(_) => (
            401,
            "Unauthorized",
            serde_json::json!({ "error": "invalid or expired refresh token" }),
        ),
    }
}

async fn handle_rpc(
    db: &SharedDatabase,
    func_name: &str,
    body: &str,
    ctx: &ExecutionContext,
) -> (u16, &'static str, serde_json::Value) {
    let payload: serde_json::Value = serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
    match func_name {
        "version" => (
            200,
            "OK",
            serde_json::json!({ "version": "0.1.0", "engine": "ChocoBase" }),
        ),
        "current_user" => (
            200,
            "OK",
            serde_json::json!({ "user_id": ctx.user_id, "role": ctx.role }),
        ),
        "echo" => (200, "OK", payload),
        _ => {
            // Check if there's a stored SQL statement / function
            let safe_func = match sanitize_identifier(func_name) {
                Ok(f) => f,
                Err(err) => return (400, "Bad Request", serde_json::json!({ "error": err })),
            };
            let sql = format!("SELECT * FROM {safe_func}()");
            match db.execute_with_context(&sql, ctx) {
                Ok(res) => (200, "OK", serde_json::json!({ "result": res })),
                Err(e) => (
                    404,
                    "Not Found",
                    serde_json::json!({ "error": format!("function '{func_name}' not found: {e}") }),
                ),
            }
        }
    }
}

async fn handle_auth_signup(
    db: &SharedDatabase,
    body: &str,
) -> (u16, &'static str, serde_json::Value) {
    let payload: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => {
            return (
                400,
                "Bad Request",
                serde_json::json!({ "error": "invalid JSON body" }),
            )
        }
    };

    let username = match payload.get("username").and_then(|u| u.as_str()) {
        Some(u) => u,
        None => {
            return (
                400,
                "Bad Request",
                serde_json::json!({ "error": "missing username field" }),
            )
        }
    };

    let password = match payload.get("password").and_then(|p| p.as_str()) {
        Some(p) => p,
        None => {
            return (
                400,
                "Bad Request",
                serde_json::json!({ "error": "missing password field" }),
            )
        }
    };

    let safe_user = match sanitize_identifier(username) {
        Ok(u) => u,
        Err(err) => return (400, "Bad Request", serde_json::json!({ "error": err })),
    };

    if let Some(r) = payload.get("role").and_then(|r| r.as_str()) {
        if r.eq_ignore_ascii_case("admin") || r.eq_ignore_ascii_case("service_role") {
            return (
                400,
                "Bad Request",
                serde_json::json!({ "error": "cannot request administrative role during public signup" }),
            );
        }
    }
    let role = "user";
    let safe_pass = escape_sql_string(password);

    let sql = format!("CREATE USER {safe_user} WITH PASSWORD '{safe_pass}' ROLE '{role}'");
    match db.execute_with_context(&sql, &ExecutionContext::admin()) {
        Ok(_) => {
            // Lookup created user ID
            let select_sql = format!("SELECT id, role FROM _users WHERE username = '{safe_user}'");
            let user_id = if let Ok(ExecResult::Rows { rows, .. }) =
                db.execute_with_context(&select_sql, &ExecutionContext::admin())
            {
                if let Some(row) = rows.first() {
                    match &row[0] {
                        Value::Integer(id) => *id,
                        _ => 1,
                    }
                } else {
                    1
                }
            } else {
                1
            };

            let secret = crate::auth::jwt_secret();
            let exp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                + 86400 * 7;
            let claims = SessionClaims::new(user_id, username, role, exp);
            let token = sign_jwt(&claims, &secret);
            let refresh = sign_jwt(&claims, &secret);

            (
                201,
                "Created",
                serde_json::json!({
                    "status": "ok",
                    "access_token": token,
                    "refresh_token": refresh,
                    "token_type": "bearer",
                    "user": {
                        "id": user_id,
                        "username": username,
                        "role": role,
                    }
                }),
            )
        }
        Err(e) => (
            400,
            "Bad Request",
            serde_json::json!({ "error": e.to_string() }),
        ),
    }
}

async fn handle_auth_token(
    db: &SharedDatabase,
    body: &str,
) -> (u16, &'static str, serde_json::Value) {
    let payload: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => {
            return (
                400,
                "Bad Request",
                serde_json::json!({ "error": "invalid JSON body" }),
            )
        }
    };

    let username = match payload.get("username").and_then(|u| u.as_str()) {
        Some(u) => u,
        None => {
            return (
                400,
                "Bad Request",
                serde_json::json!({ "error": "missing username field" }),
            )
        }
    };

    let password = match payload.get("password").and_then(|p| p.as_str()) {
        Some(p) => p,
        None => {
            return (
                400,
                "Bad Request",
                serde_json::json!({ "error": "missing password field" }),
            )
        }
    };

    let safe_user = match sanitize_identifier(username) {
        Ok(u) => u,
        Err(err) => return (400, "Bad Request", serde_json::json!({ "error": err })),
    };

    let sql = format!("SELECT id, password_hash, role FROM _users WHERE username = '{safe_user}'");
    match db.execute_with_context(&sql, &ExecutionContext::admin()) {
        Ok(ExecResult::Rows { rows, .. }) => {
            if let Some(row) = rows.first() {
                let user_id = match &row[0] {
                    Value::Integer(id) => *id,
                    _ => 0,
                };
                let hash = match &row[1] {
                    Value::Text(h) => h.as_str(),
                    _ => "",
                };
                let role = match &row[2] {
                    Value::Text(r) => r.as_str(),
                    _ => "user",
                };

                if verify_password(password, hash) {
                    let secret = crate::auth::jwt_secret();
                    let exp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                        + 86400 * 7;
                    let claims = SessionClaims::new(user_id, username, role, exp);
                    let token = sign_jwt(&claims, &secret);
                    let refresh = sign_jwt(&claims, &secret);

                    (
                        200,
                        "OK",
                        serde_json::json!({
                            "access_token": token,
                            "refresh_token": refresh,
                            "token_type": "bearer",
                            "user": {
                                "id": user_id,
                                "username": username,
                                "role": role,
                            }
                        }),
                    )
                } else {
                    (
                        401,
                        "Unauthorized",
                        serde_json::json!({ "error": "invalid credentials" }),
                    )
                }
            } else {
                (
                    401,
                    "Unauthorized",
                    serde_json::json!({ "error": "user not found" }),
                )
            }
        }
        _ => (
            401,
            "Unauthorized",
            serde_json::json!({ "error": "authentication failed" }),
        ),
    }
}

async fn handle_rest_table_crud(
    db: &SharedDatabase,
    method: &str,
    table: &str,
    query_str: &str,
    body: &str,
    ctx: &ExecutionContext,
) -> (u16, &'static str, serde_json::Value) {
    let schema = match db.table_schema(table) {
        Some(s) => s,
        None => {
            return (
                404,
                "Not Found",
                serde_json::json!({ "error": format!("table '{table}' not found") }),
            )
        }
    };

    let query_params = parse_query_params(query_str);

    match method {
        "GET" => {
            let select_cols = query_params
                .get("select")
                .map(|s| s.as_str())
                .unwrap_or("*");
            let mut sql = format!("SELECT {select_cols} FROM {table}");
            let where_clauses = build_where_clauses(&query_params);
            if !where_clauses.is_empty() {
                sql.push_str(&format!(" WHERE {}", where_clauses.join(" AND ")));
            }

            if let Some(order) = query_params.get("order") {
                if let Some((col, dir)) = order.split_once('.') {
                    let dir_sql = if dir.eq_ignore_ascii_case("desc") {
                        "DESC"
                    } else {
                        "ASC"
                    };
                    sql.push_str(&format!(" ORDER BY {col} {dir_sql}"));
                } else {
                    sql.push_str(&format!(" ORDER BY {order} ASC"));
                }
            }

            let offset = query_params
                .get("offset")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(0);
            let limit = query_params
                .get("limit")
                .and_then(|s| s.parse::<usize>().ok());

            match db.execute_with_context(&sql, ctx) {
                Ok(ExecResult::Rows { columns, rows }) => {
                    let json_rows: Vec<serde_json::Value> = rows
                        .iter()
                        .skip(offset)
                        .take(limit.unwrap_or(usize::MAX))
                        .map(|row| {
                            let mut map = serde_json::Map::new();
                            for (idx, col_name) in columns.iter().enumerate() {
                                map.insert(col_name.clone(), value_to_json(&row[idx]));
                            }
                            serde_json::Value::Object(map)
                        })
                        .collect();
                    (200, "OK", serde_json::Value::Array(json_rows))
                }
                Ok(_) => (200, "OK", serde_json::json!([])),
                Err(e) => (
                    400,
                    "Bad Request",
                    serde_json::json!({ "error": e.to_string(), "code": e.sqlstate() }),
                ),
            }
        }
        "POST" => {
            let json_body: serde_json::Value = match serde_json::from_str(body) {
                Ok(v) => v,
                Err(_) => {
                    return (
                        400,
                        "Bad Request",
                        serde_json::json!({ "error": "invalid JSON body" }),
                    )
                }
            };

            let rows_to_insert = match json_body {
                serde_json::Value::Array(arr) => arr,
                obj @ serde_json::Value::Object(_) => vec![obj],
                _ => {
                    return (
                        400,
                        "Bad Request",
                        serde_json::json!({ "error": "body must be JSON object or array" }),
                    )
                }
            };

            let mut inserted_count = 0;
            for row_val in rows_to_insert {
                let obj = match row_val.as_object() {
                    Some(o) => o,
                    None => continue,
                };

                let mut col_names = Vec::new();
                let mut col_values = Vec::new();

                for col in &schema.columns {
                    if let Some(val) = obj.get(&col.name) {
                        col_names.push(col.name.clone());
                        col_values.push(json_to_sql_literal(val));
                    }
                }

                if col_names.is_empty() {
                    continue;
                }

                let sql = format!(
                    "INSERT INTO {table} ({}) VALUES ({})",
                    col_names.join(", "),
                    col_values.join(", ")
                );

                match db.execute_with_context(&sql, ctx) {
                    Ok(ExecResult::Modified(n)) => inserted_count += n,
                    Ok(_) => inserted_count += 1,
                    Err(e) => {
                        return (
                            400,
                            "Bad Request",
                            serde_json::json!({ "error": e.to_string() }),
                        )
                    }
                }
            }

            (
                201,
                "Created",
                serde_json::json!({ "status": "ok", "inserted": inserted_count }),
            )
        }
        "PATCH" => {
            let json_body: serde_json::Value = match serde_json::from_str(body) {
                Ok(v) => v,
                Err(_) => {
                    return (
                        400,
                        "Bad Request",
                        serde_json::json!({ "error": "invalid JSON body" }),
                    )
                }
            };

            let obj = match json_body.as_object() {
                Some(o) => o,
                None => {
                    return (
                        400,
                        "Bad Request",
                        serde_json::json!({ "error": "body must be JSON object" }),
                    )
                }
            };

            let mut assignments = Vec::new();
            for (k, v) in obj {
                assignments.push(format!("{k} = {}", json_to_sql_literal(v)));
            }

            if assignments.is_empty() {
                return (
                    400,
                    "Bad Request",
                    serde_json::json!({ "error": "no fields provided to update" }),
                );
            }

            let mut sql = format!("UPDATE {table} SET {}", assignments.join(", "));
            let where_clauses = build_where_clauses(&query_params);
            if !where_clauses.is_empty() {
                sql.push_str(&format!(" WHERE {}", where_clauses.join(" AND ")));
            }

            match db.execute_with_context(&sql, ctx) {
                Ok(ExecResult::Modified(n)) => (
                    200,
                    "OK",
                    serde_json::json!({ "status": "ok", "modified": n }),
                ),
                Ok(_) => (
                    200,
                    "OK",
                    serde_json::json!({ "status": "ok", "modified": 0 }),
                ),
                Err(e) => (
                    400,
                    "Bad Request",
                    serde_json::json!({ "error": e.to_string() }),
                ),
            }
        }
        "DELETE" => {
            let mut sql = format!("DELETE FROM {table}");
            let where_clauses = build_where_clauses(&query_params);
            if !where_clauses.is_empty() {
                sql.push_str(&format!(" WHERE {}", where_clauses.join(" AND ")));
            }

            match db.execute_with_context(&sql, ctx) {
                Ok(ExecResult::Modified(n)) => (
                    200,
                    "OK",
                    serde_json::json!({ "status": "ok", "deleted": n }),
                ),
                Ok(_) => (
                    200,
                    "OK",
                    serde_json::json!({ "status": "ok", "deleted": 0 }),
                ),
                Err(e) => (
                    400,
                    "Bad Request",
                    serde_json::json!({ "error": e.to_string() }),
                ),
            }
        }
        _ => (
            405,
            "Method Not Allowed",
            serde_json::json!({ "error": format!("method '{method}' not allowed") }),
        ),
    }
}

fn parse_query_params(query: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        if let Some((k, v)) = pair.split_once('=') {
            map.insert(k.to_string(), v.to_string());
        }
    }
    map
}

fn build_where_clauses(params: &HashMap<String, String>) -> Vec<String> {
    let mut clauses = Vec::new();
    for (key, val) in params {
        if key == "order" || key == "limit" || key == "offset" || key == "select" {
            continue;
        }
        if let Some((op, rhs)) = val.split_once('.') {
            match op {
                "eq" => clauses.push(format!("{key} = {}", format_sql_val(rhs))),
                "neq" => clauses.push(format!("{key} != {}", format_sql_val(rhs))),
                "gt" => clauses.push(format!("{key} > {}", format_sql_val(rhs))),
                "gte" => clauses.push(format!("{key} >= {}", format_sql_val(rhs))),
                "lt" => clauses.push(format!("{key} < {}", format_sql_val(rhs))),
                "lte" => clauses.push(format!("{key} <= {}", format_sql_val(rhs))),
                "like" | "ilike" => {
                    clauses.push(format!("{key} LIKE '{}'", rhs.replace('\'', "''")))
                }
                "is" => {
                    if rhs.eq_ignore_ascii_case("null") {
                        clauses.push(format!("{key} IS NULL"));
                    } else if rhs.eq_ignore_ascii_case("not_null")
                        || rhs.eq_ignore_ascii_case("not.null")
                    {
                        clauses.push(format!("{key} IS NOT NULL"));
                    }
                }
                "in" => {
                    let cleaned = rhs.trim_start_matches('(').trim_end_matches(')');
                    let elements: Vec<String> = cleaned
                        .split(',')
                        .map(|item| format_sql_val(item.trim()))
                        .collect();
                    clauses.push(format!("{key} IN ({})", elements.join(", ")));
                }
                _ => clauses.push(format!("{key} = {}", format_sql_val(rhs))),
            }
        } else {
            clauses.push(format!("{key} = {}", format_sql_val(val)));
        }
    }
    clauses
}

fn format_sql_val(s: &str) -> String {
    if let Ok(i) = s.parse::<i64>() {
        i.to_string()
    } else if s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("false") {
        s.to_uppercase()
    } else {
        format!("'{}'", s.replace('\'', "''"))
    }
}

fn value_to_json(val: &Value) -> serde_json::Value {
    match val {
        Value::Integer(i) => serde_json::json!(i),
        Value::Text(s) => serde_json::json!(s),
        Value::Boolean(b) => serde_json::json!(b),
        Value::Json(j) => serde_json::from_str(j).unwrap_or_else(|_| serde_json::json!(j)),
        Value::Null => serde_json::Value::Null,
    }
}

fn json_to_sql_literal(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::Null => "NULL".to_string(),
        serde_json::Value::Bool(b) => {
            if *b {
                "TRUE".to_string()
            } else {
                "FALSE".to_string()
            }
        }
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => format!("'{}'", s.replace('\'', "''")),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            let serialized = serde_json::to_string(val).unwrap_or_default();
            format!("'{}'", serialized.replace('\'', "''"))
        }
    }
}
