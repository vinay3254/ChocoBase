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
        let webhook_mgr = Arc::new(crate::webhooks::WebhookManager::new());
        webhook_mgr.clone().start_dispatcher(db.subscribe());
        let branch_mgr = Arc::new(crate::branching::BranchManager::new(
            std::env::temp_dir().join("chocobase_branches"),
        ));

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    accept_res = listener.accept() => {
                        match accept_res {
                            Ok((socket, _)) => {
                                let db_clone = Arc::clone(&db);
                                let func_clone = Arc::clone(&functions_reg);
                                let rt_clone = Arc::clone(&realtime_mgr);
                                let wh_clone = Arc::clone(&webhook_mgr);
                                let br_clone = Arc::clone(&branch_mgr);
                                tokio::spawn(async move {
                                    let _ = handle_http_connection(
                                        socket, db_clone, func_clone, rt_clone, wh_clone, br_clone,
                                    )
                                    .await;
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

fn get_cors_header(origin_opt: Option<&str>) -> (String, bool) {
    let origins_env = std::env::var("CHOCOBASE_CORS_ORIGINS").unwrap_or_else(|_| "*".into());
    let origin = match origin_opt {
        Some(o) => o,
        None => return ("Access-Control-Allow-Origin: *\r\n".to_string(), true),
    };

    if origins_env == "*" {
        ("Access-Control-Allow-Origin: *\r\n".to_string(), true)
    } else {
        let allowed_list: Vec<&str> = origins_env.split(',').map(|s| s.trim()).collect();
        if allowed_list.contains(&origin) {
            (
                format!("Access-Control-Allow-Origin: {origin}\r\nAccess-Control-Allow-Credentials: true\r\n"),
                true,
            )
        } else {
            (String::new(), false)
        }
    }
}

async fn handle_http_connection(
    mut socket: TcpStream,
    db: Arc<SharedDatabase>,
    functions_reg: Arc<FunctionRegistry>,
    realtime_mgr: Arc<RealtimeChannelManager>,
    webhook_mgr: Arc<crate::webhooks::WebhookManager>,
    branch_mgr: Arc<crate::branching::BranchManager>,
) -> std::io::Result<()> {
    let mut header_buf = Vec::new();
    let mut temp_chunk = [0u8; 1024];
    let mut header_end_pos = None;

    while header_buf.len() < 32768 {
        let n = socket.read(&mut temp_chunk).await?;
        if n == 0 {
            return Ok(());
        }
        header_buf.extend_from_slice(&temp_chunk[..n]);
        if let Some(pos) = header_buf.windows(4).position(|w| w == b"\r\n\r\n") {
            header_end_pos = Some(pos);
            break;
        }
    }

    let header_pos = match header_end_pos {
        Some(p) => p,
        None => {
            let resp = "HTTP/1.1 431 Request Header Fields Too Large\r\nConnection: close\r\n\r\n";
            socket.write_all(resp.as_bytes()).await?;
            return Ok(());
        }
    };

    let header_str = String::from_utf8_lossy(&header_buf[..header_pos]);
    let mut lines = header_str.lines();
    let request_line = match lines.next() {
        Some(line) => line,
        None => return Ok(()),
    };

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_uppercase();
    let full_path = parts.next().unwrap_or("/").to_string();

    // Parse headers
    let mut auth_token = None;
    let mut origin_header = None;
    let mut range_header = None;
    let mut content_length = 0usize;

    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let k_trim = k.trim();
            let v_trim = v.trim();
            if k_trim.eq_ignore_ascii_case("authorization") {
                if v_trim.to_lowercase().starts_with("bearer ") {
                    auth_token = Some(v_trim[7..].trim().to_string());
                }
            } else if k_trim.eq_ignore_ascii_case("origin") {
                origin_header = Some(v_trim.to_string());
            } else if k_trim.eq_ignore_ascii_case("range") {
                range_header = Some(v_trim.to_string());
            } else if k_trim.eq_ignore_ascii_case("content-length") {
                content_length = v_trim.parse().unwrap_or(0);
            }
        }
    }

    const MAX_PAYLOAD_BYTES: usize = 10 * 1024 * 1024; // 10MB limit
    if content_length > MAX_PAYLOAD_BYTES {
        let resp_json = serde_json::json!({ "error": "payload too large (max 10MB)" });
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

    // Read remaining body bytes if needed
    let mut body_bytes = header_buf[header_pos + 4..].to_vec();
    while body_bytes.len() < content_length {
        let n = socket.read(&mut temp_chunk).await?;
        if n == 0 {
            break;
        }
        body_bytes.extend_from_slice(&temp_chunk[..n]);
    }
    let body = String::from_utf8_lossy(&body_bytes).to_string();

    let (cors_headers, origin_allowed) = get_cors_header(origin_header.as_deref());

    if method == "OPTIONS" {
        if !origin_allowed {
            let resp = "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            socket.write_all(resp.as_bytes()).await?;
            return Ok(());
        }
        let resp = format!(
            "HTTP/1.1 204 No Content\r\n{cors_headers}Access-Control-Allow-Methods: GET, POST, PATCH, DELETE, OPTIONS, PUT\r\nAccess-Control-Allow-Headers: Content-Type, Authorization, X-Requested-With, apikey\r\nAccess-Control-Max-Age: 86400\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        socket.write_all(resp.as_bytes()).await?;
        return Ok(());
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

    let (path, query_string) = match full_path.split_once('?') {
        Some((p, q)) => (p, q),
        None => (full_path.as_str(), ""),
    };

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

    if method == "GET" && path == "/v1/realtime/v1/stream" {
        let mut channel_name = "general";
        for pair in query_string.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                if k == "channel" {
                    channel_name = v;
                }
            }
        }

        if channel_name.starts_with("private:")
            && !exec_ctx.is_authenticated()
            && !exec_ctx.is_admin
        {
            let resp_json = serde_json::json!({ "error": "authentication required for private broadcast channel" });
            let resp_bytes = serde_json::to_vec(&resp_json).unwrap_or_default();
            let header = format!(
                "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\n{cors_headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
                resp_bytes.len()
            );
            socket.write_all(header.as_bytes()).await?;
            socket.write_all(&resp_bytes).await?;
            socket.flush().await?;
            return Ok(());
        }

        let init_header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n{cors_headers}\r\n"
        );
        socket.write_all(init_header.as_bytes()).await?;
        let init_event = format!(
            "event: connected\ndata: {{\"status\":\"subscribed\",\"channel\":\"{channel_name}\"}}\n\n"
        );
        socket.write_all(init_event.as_bytes()).await?;
        socket.flush().await?;

        let mut bcast_rx = realtime_mgr.subscribe(channel_name);
        let mut change_rx = db.subscribe();

        loop {
            tokio::select! {
                bcast_res = bcast_rx.recv() => {
                    match bcast_res {
                        Ok(msg) => {
                            let json_payload = serde_json::to_string(&msg).unwrap_or_default();
                            let sse_chunk = format!("event: broadcast\ndata: {json_payload}\n\n");
                            if socket.write_all(sse_chunk.as_bytes()).await.is_err() {
                                break;
                            }
                            let _ = socket.flush().await;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
                change_res = change_rx.recv() => {
                    match change_res {
                        Ok(change) => {
                            let json_payload = serde_json::to_string(&change).unwrap_or_default();
                            let sse_chunk = format!("event: change\ndata: {json_payload}\n\n");
                            if socket.write_all(sse_chunk.as_bytes()).await.is_err() {
                                break;
                            }
                            let _ = socket.flush().await;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
            }
        }
        return Ok(());
    }

    if path.starts_with("/v1/storage/") {
        let (status_code, status_text, json_body, binary_data) = storage::handle_storage_request(
            &db,
            &method,
            path,
            query_string,
            &body,
            &exec_ctx,
            range_header.as_deref(),
        )
        .await;

        if let Some((bytes, content_type, cr_opt, etag)) = binary_data {
            let cr_header = if let Some(cr) = cr_opt {
                format!("Content-Range: {cr}\r\n")
            } else {
                String::new()
            };
            let header = format!(
                "HTTP/1.1 {status_code} {status_text}\r\nContent-Type: {content_type}\r\nETag: {etag}\r\nAccept-Ranges: bytes\r\n{cr_header}{cors_headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            );
            socket.write_all(header.as_bytes()).await?;
            socket.write_all(&bytes).await?;
            socket.flush().await?;
            return Ok(());
        } else {
            let body_bytes = serde_json::to_vec(&json_body).unwrap_or_default();
            let header = format!(
                "HTTP/1.1 {status_code} {status_text}\r\nContent-Type: application/json\r\n{cors_headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
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
            let sql = if let Ok(parsed_json) = serde_json::from_str::<serde_json::Value>(&body) {
                parsed_json
                    .get("sql")
                    .and_then(|s| s.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| body.clone())
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
        let sql = if let Ok(parsed_json) = serde_json::from_str::<serde_json::Value>(&body) {
            parsed_json
                .get("sql")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| body.clone())
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
        handle_auth_signup(&db, &body).await
    } else if method == "POST" && path == "/v1/auth/token" {
        handle_auth_token(&db, &body).await
    } else if method == "POST" && path == "/v1/auth/refresh" {
        handle_auth_refresh(&db, &body).await
    } else if method == "POST" && path == "/v1/auth/logout" {
        handle_auth_logout(&db, &body).await
    } else if (method == "POST" || method == "GET") && path == "/v1/auth/oauth/authorize" {
        handle_oauth_authorize(query_string, &body).await
    } else if method == "POST" && path == "/v1/auth/oauth/callback" {
        handle_oauth_callback(&db, &body).await
    } else if path.starts_with("/v1/functions/v1") {
        functions::handle_functions_request(&functions_reg, &db, &method, path, &body, &exec_ctx)
            .await
    } else if path.starts_with("/v1/realtime/v1") {
        realtime_channels::handle_realtime_channel_request(
            &realtime_mgr,
            &method,
            path,
            &body,
            &exec_ctx,
        )
        .await
    } else if path.starts_with("/v1/webhooks") || path.starts_with("/admin/webhooks") {
        handle_webhooks_request(&webhook_mgr, &method, path, query_string, &body, &exec_ctx).await
    } else if method == "POST" && (path == "/v1/graphql" || path == "/graphql") {
        let gql_req: crate::graphql::GraphQLRequest = match serde_json::from_str(&body) {
            Ok(req) => req,
            Err(_) => crate::graphql::GraphQLRequest {
                query: body.clone(),
                variables: None,
                operation_name: None,
            },
        };
        let gql_resp = crate::graphql::execute_graphql(&db, &gql_req, &exec_ctx).await;
        (
            200,
            "OK",
            serde_json::to_value(&gql_resp).unwrap_or_else(|_| serde_json::json!({})),
        )
    } else if method == "GET" && path == "/health" {
        (
            200,
            "OK",
            serde_json::json!({
                "status": "healthy",
                "version": "0.1.0",
                "engine": "ChocoBase"
            }),
        )
    } else if method == "GET" && path == "/metrics" {
        let metrics_text = crate::metrics::MetricsRegistry::global().render_prometheus();
        (200, "OK", serde_json::Value::String(metrics_text))
    } else if method == "GET" && (path == "/.well-known/jwks.json" || path == "/v1/auth/keys") {
        (
            200,
            "OK",
            serde_json::json!({
                "keys": [
                    {
                        "kty": "oct",
                        "kid": "k1",
                        "alg": "HS256",
                        "use": "sig"
                    }
                ]
            }),
        )
    } else if path.starts_with("/v1/branches") || path.starts_with("/admin/branches") {
        handle_branches_request(
            &branch_mgr,
            &db,
            &method,
            path,
            query_string,
            &body,
            &exec_ctx,
        )
        .await
    } else if let Some(func_name) = path.strip_prefix("/v1/rpc/") {
        handle_rpc(&db, func_name, &body, &exec_ctx).await
    } else if let Some(table_name) = path.strip_prefix("/v1/rest/") {
        handle_rest_table_crud(&db, &method, table_name, query_string, &body, &exec_ctx).await
    } else {
        (
            404,
            "Not Found",
            serde_json::json!({ "error": format!("endpoint '{}' not found", path) }),
        )
    };

    crate::metrics::MetricsRegistry::global().record_http_request(status_code);

    let (content_type, body_bytes) = if path == "/metrics" {
        (
            "text/plain; version=0.0.4; charset=utf-8",
            match &json_body {
                serde_json::Value::String(s) => s.as_bytes().to_vec(),
                _ => vec![],
            },
        )
    } else {
        (
            "application/json",
            serde_json::to_vec(&json_body).unwrap_or_default(),
        )
    };

    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\n{cors_headers}X-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status_code,
        status_text,
        content_type,
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

async fn handle_webhooks_request(
    webhook_mgr: &crate::webhooks::WebhookManager,
    method: &str,
    path: &str,
    query_string: &str,
    body: &str,
    exec_ctx: &crate::auth::ExecutionContext,
) -> (u16, &'static str, serde_json::Value) {
    if !exec_ctx.is_authenticated() {
        return (
            401,
            "Unauthorized",
            serde_json::json!({ "error": "authentication required" }),
        );
    }

    if method == "GET" {
        let list = webhook_mgr.list_webhooks().await;
        (200, "OK", serde_json::json!({ "webhooks": list }))
    } else if method == "POST" {
        let cfg: crate::webhooks::WebhookConfig = match serde_json::from_str(body) {
            Ok(c) => c,
            Err(e) => {
                return (
                    400,
                    "Bad Request",
                    serde_json::json!({ "error": format!("invalid webhook config JSON: {e}") }),
                );
            }
        };
        webhook_mgr.add_webhook(cfg.clone()).await;
        (
            201,
            "Created",
            serde_json::json!({ "status": "created", "webhook": cfg }),
        )
    } else if method == "DELETE" {
        let id = if let Some(sub) = path.strip_prefix("/v1/webhooks/") {
            sub
        } else if let Some(sub) = path.strip_prefix("/admin/webhooks/") {
            sub
        } else if !query_string.is_empty() {
            query_string.strip_prefix("id=").unwrap_or(query_string)
        } else {
            ""
        };

        if id.is_empty() {
            return (
                400,
                "Bad Request",
                serde_json::json!({ "error": "missing webhook id" }),
            );
        }

        let removed = webhook_mgr.remove_webhook(id).await;
        if removed {
            (
                200,
                "OK",
                serde_json::json!({ "status": "deleted", "id": id }),
            )
        } else {
            (
                404,
                "Not Found",
                serde_json::json!({ "error": "webhook not found" }),
            )
        }
    } else {
        (
            405,
            "Method Not Allowed",
            serde_json::json!({ "error": format!("method '{method}' not allowed") }),
        )
    }
}

async fn handle_branches_request(
    branch_mgr: &crate::branching::BranchManager,
    source_db: &SharedDatabase,
    method: &str,
    path: &str,
    query_string: &str,
    body: &str,
    exec_ctx: &crate::auth::ExecutionContext,
) -> (u16, &'static str, serde_json::Value) {
    if !exec_ctx.is_authenticated() {
        return (
            401,
            "Unauthorized",
            serde_json::json!({ "error": "authentication required" }),
        );
    }

    if method == "GET" {
        let list = branch_mgr.list_branches().await;
        (200, "OK", serde_json::json!({ "branches": list }))
    } else if method == "POST" {
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
        let branch_name = match payload.get("name").and_then(|n| n.as_str()) {
            Some(n) => n,
            None => {
                return (
                    400,
                    "Bad Request",
                    serde_json::json!({ "error": "missing branch name" }),
                )
            }
        };

        match branch_mgr.create_branch(branch_name, source_db).await {
            Ok(meta) => (
                201,
                "Created",
                serde_json::json!({ "status": "created", "branch": meta }),
            ),
            Err(e) => (
                400,
                "Bad Request",
                serde_json::json!({ "error": e.to_string() }),
            ),
        }
    } else if method == "DELETE" {
        let name = if let Some(sub) = path.strip_prefix("/v1/branches/") {
            sub
        } else if let Some(sub) = path.strip_prefix("/admin/branches/") {
            sub
        } else if !query_string.is_empty() {
            query_string.strip_prefix("name=").unwrap_or(query_string)
        } else {
            ""
        };

        if name.is_empty() {
            return (
                400,
                "Bad Request",
                serde_json::json!({ "error": "missing branch name" }),
            );
        }

        let deleted = branch_mgr.delete_branch(name).await;
        if deleted {
            (
                200,
                "OK",
                serde_json::json!({ "status": "deleted", "branch": name }),
            )
        } else {
            (
                404,
                "Not Found",
                serde_json::json!({ "error": "branch not found" }),
            )
        }
    } else {
        (
            405,
            "Method Not Allowed",
            serde_json::json!({ "error": format!("method '{method}' not allowed") }),
        )
    }
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

    match crate::auth::rotate_refresh_token(refresh_token) {
        Ok((new_claims, new_refresh_token)) => {
            let secret = crate::auth::jwt_secret();
            let new_access_token = sign_jwt(&new_claims, &secret);
            (
                200,
                "OK",
                serde_json::json!({
                    "access_token": new_access_token,
                    "refresh_token": new_refresh_token,
                    "token_type": "bearer",
                    "user": {
                        "id": new_claims.sub,
                        "username": new_claims.username,
                        "role": new_claims.role
                    }
                }),
            )
        }
        Err(e) => (
            401,
            "Unauthorized",
            serde_json::json!({ "error": e.to_string() }),
        ),
    }
}

async fn handle_auth_logout(
    _db: &SharedDatabase,
    body: &str,
) -> (u16, &'static str, serde_json::Value) {
    let payload: serde_json::Value = serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
    if let Some(refresh_token) = payload.get("refresh_token").and_then(|r| r.as_str()) {
        crate::auth::revoke_refresh_token(refresh_token);
    }
    (200, "OK", serde_json::json!({ "status": "logged_out" }))
}

async fn handle_oauth_authorize(
    query_string: &str,
    body: &str,
) -> (u16, &'static str, serde_json::Value) {
    let mut provider = "google".to_string();
    let mut redirect_uri = "http://localhost:3000/callback".to_string();

    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(p) = parsed.get("provider").and_then(|v| v.as_str()) {
            provider = p.to_string();
        }
        if let Some(r) = parsed.get("redirect_uri").and_then(|v| v.as_str()) {
            redirect_uri = r.to_string();
        }
    } else if !query_string.is_empty() {
        for part in query_string.split('&') {
            if let Some((k, v)) = part.split_once('=') {
                if k == "provider" {
                    provider = v.to_string();
                } else if k == "redirect_uri" {
                    redirect_uri = v.to_string();
                }
            }
        }
    }

    match crate::auth::oauth::generate_authorize_url(&provider, &redirect_uri) {
        Ok(resp) => (
            200,
            "OK",
            serde_json::json!({
                "status": "ok",
                "provider": resp.provider,
                "url": resp.url,
                "state": resp.state,
            }),
        ),
        Err(err) => (400, "Bad Request", serde_json::json!({ "error": err })),
    }
}

async fn handle_oauth_callback(
    db: &SharedDatabase,
    body: &str,
) -> (u16, &'static str, serde_json::Value) {
    let req: crate::auth::oauth::OAuthCallbackRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(_) => {
            return (
                400,
                "Bad Request",
                serde_json::json!({ "error": "invalid OAuth callback JSON payload" }),
            );
        }
    };

    if req.code.is_empty() {
        return (
            400,
            "Bad Request",
            serde_json::json!({ "error": "missing authorization code" }),
        );
    }

    let username = req.username.unwrap_or_else(|| {
        if let Some(e) = &req.email {
            e.split('@').next().unwrap_or("oauth_user").to_string()
        } else {
            format!("{}_user", req.provider)
        }
    });

    let safe_user = match sanitize_identifier(&username) {
        Ok(u) => u,
        Err(_) => "oauth_user".to_string(),
    };

    // Ensure user exists in _users table
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
            // Auto-create user
            let random_pass = format!("oauth_pass_{}", req.code);
            let safe_pass = escape_sql_string(&crate::auth::hash_password(&random_pass));
            let insert_sql = format!(
                "INSERT INTO _users (username, password_hash, role) VALUES ('{safe_user}', '{safe_pass}', 'user')"
            );
            let _ = db.execute_with_context(&insert_sql, &ExecutionContext::admin());

            if let Ok(ExecResult::Rows { rows: r2, .. }) =
                db.execute_with_context(&select_sql, &ExecutionContext::admin())
            {
                r2.first()
                    .and_then(|r| match &r[0] {
                        Value::Integer(id) => Some(*id),
                        _ => None,
                    })
                    .unwrap_or(2)
            } else {
                2
            }
        }
    } else {
        1
    };

    let role = "user";
    let secret = crate::auth::jwt_secret();
    let exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        + 86400 * 7;
    let claims = SessionClaims::new(user_id, &safe_user, role, exp);
    let access_token = sign_jwt(&claims, &secret);
    let (refresh_token, _) = crate::auth::issue_refresh_token(user_id, &safe_user, role);

    (
        200,
        "OK",
        serde_json::json!({
            "status": "ok",
            "access_token": access_token,
            "refresh_token": refresh_token,
            "token_type": "bearer",
            "provider": req.provider,
            "user": {
                "id": user_id,
                "username": safe_user,
                "role": role,
            }
        }),
    )
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
            let (refresh, _) = crate::auth::issue_refresh_token(user_id, username, role);

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
                    let (refresh, _) = crate::auth::issue_refresh_token(user_id, username, role);

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
        Value::Float(f) => serde_json::json!(f),
        Value::Text(s) => serde_json::json!(s),
        Value::Boolean(b) => serde_json::json!(b),
        Value::Json(j) => serde_json::from_str(j).unwrap_or_else(|_| serde_json::json!(j)),
        Value::Vector(v) => serde_json::json!(v),
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
