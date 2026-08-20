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
pub mod rate_limit;
pub mod realtime_channels;
pub mod storage;
pub mod websocket;

use crate::functions::FunctionRegistry;
use rate_limit::RateLimiter;
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
        let rate_limiter = Arc::new(RateLimiter::new());
        let webhook_mgr = Arc::new(crate::webhooks::WebhookManager::new());
        webhook_mgr.clone().start_dispatcher(db.subscribe());
        let branch_mgr = Arc::new(crate::branching::BranchManager::new(
            std::env::temp_dir().join("chocobase_branches"),
        ));
        let replica_mgr = Arc::new(crate::replica::ReplicaManager::new(
            std::env::temp_dir().join("chocobase_replicas"),
            (*db).clone(),
        ));
        let fdw_mgr = Arc::new(crate::fdw::FdwManager::new());

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    accept_res = listener.accept() => {
                        match accept_res {
                            Ok((socket, _)) => {
                                let db_clone = Arc::clone(&db);
                                let func_clone = Arc::clone(&functions_reg);
                                let rt_clone = Arc::clone(&realtime_mgr);
                                let rl_clone = Arc::clone(&rate_limiter);
                                let wh_clone = Arc::clone(&webhook_mgr);
                                let br_clone = Arc::clone(&branch_mgr);
                                let rep_clone = Arc::clone(&replica_mgr);
                                let fdw_clone = Arc::clone(&fdw_mgr);
                                tokio::spawn(async move {
                                    let _ = handle_http_connection(
                                        socket, db_clone, func_clone, rt_clone, rl_clone, wh_clone, br_clone, rep_clone, fdw_clone,
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

fn json_rows_to_csv(rows: &[serde_json::Value]) -> String {
    if rows.is_empty() {
        return String::new();
    }

    let mut headers = Vec::new();
    if let Some(first_obj) = rows.first().and_then(|r| r.as_object()) {
        for k in first_obj.keys() {
            headers.push(k.clone());
        }
    }

    if headers.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    out.push_str(&headers.join(","));
    out.push('\n');

    for row_val in rows {
        if let Some(obj) = row_val.as_object() {
            let line_vals: Vec<String> = headers
                .iter()
                .map(|h| match obj.get(h) {
                    Some(serde_json::Value::Null) | None => String::new(),
                    Some(serde_json::Value::String(s)) => {
                        if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
                            format!("\"{}\"", s.replace('"', "\"\""))
                        } else {
                            s.clone()
                        }
                    }
                    Some(v) => {
                        let s = v.to_string();
                        if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
                            format!("\"{}\"", s.replace('"', "\"\""))
                        } else {
                            s
                        }
                    }
                })
                .collect();
            out.push_str(&line_vals.join(","));
            out.push('\n');
        }
    }

    out
}

fn get_cors_header(origin_opt: Option<&str>) -> (String, bool) {
    let origins_env = std::env::var("CHOCOBASE_CORS_ORIGINS").unwrap_or_else(|_| "*".into());
    let origin = match origin_opt {
        Some(o) => o,
        None => return (String::new(), true),
    };

    let expose_hdr = "Access-Control-Expose-Headers: Content-Range, Range-Unit, Preference-Applied, Content-Length, ETag, X-Total-Count\r\n";

    if origins_env == "*" {
        (
            format!("Access-Control-Allow-Origin: {origin}\r\nAccess-Control-Allow-Credentials: true\r\n{expose_hdr}"),
            true,
        )
    } else {
        let allowed_list: Vec<&str> = origins_env.split(',').map(|s| s.trim()).collect();
        if allowed_list.contains(&origin) {
            (
                format!("Access-Control-Allow-Origin: {origin}\r\nAccess-Control-Allow-Credentials: true\r\n{expose_hdr}"),
                true,
            )
        } else {
            (String::new(), false)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_http_connection(
    mut socket: TcpStream,
    db: Arc<SharedDatabase>,
    functions_reg: Arc<FunctionRegistry>,
    realtime_mgr: Arc<RealtimeChannelManager>,
    rate_limiter: Arc<RateLimiter>,
    webhook_mgr: Arc<crate::webhooks::WebhookManager>,
    branch_mgr: Arc<crate::branching::BranchManager>,
    replica_mgr: Arc<crate::replica::ReplicaManager>,
    fdw_mgr: Arc<crate::fdw::FdwManager>,
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
    let mut apikey_header = None;
    let mut project_header = None;
    let mut host_header = None;
    let mut origin_header = None;
    let mut range_header = None;
    let mut prefer_header = None;
    let mut accept_header = None;
    let mut ws_upgrade = false;
    let mut ws_key = None;
    let mut content_length = 0usize;

    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let k_trim = k.trim();
            let v_trim = v.trim();
            if k_trim.eq_ignore_ascii_case("authorization") {
                if v_trim.to_lowercase().starts_with("bearer ") {
                    auth_token = Some(v_trim[7..].trim().to_string());
                }
            } else if k_trim.eq_ignore_ascii_case("apikey") {
                apikey_header = Some(v_trim.to_string());
            } else if k_trim.eq_ignore_ascii_case("x-project-id") || k_trim.eq_ignore_ascii_case("x-project-ref") {
                project_header = Some(v_trim.to_string());
            } else if k_trim.eq_ignore_ascii_case("host") {
                host_header = Some(v_trim.to_string());
            } else if k_trim.eq_ignore_ascii_case("origin") {
                origin_header = Some(v_trim.to_string());
            } else if k_trim.eq_ignore_ascii_case("range") {
                range_header = Some(v_trim.to_string());
            } else if k_trim.eq_ignore_ascii_case("prefer") {
                prefer_header = Some(v_trim.to_string());
            } else if k_trim.eq_ignore_ascii_case("accept") {
                accept_header = Some(v_trim.to_string());
            } else if k_trim.eq_ignore_ascii_case("upgrade")
                && v_trim.eq_ignore_ascii_case("websocket")
            {
                ws_upgrade = true;
            } else if k_trim.eq_ignore_ascii_case("sec-websocket-key") {
                ws_key = Some(v_trim.to_string());
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
            "HTTP/1.1 204 No Content\r\n{cors_headers}Access-Control-Allow-Methods: GET, POST, PATCH, DELETE, OPTIONS, PUT, HEAD\r\nAccess-Control-Allow-Headers: Content-Type, Authorization, X-Requested-With, apikey, Prefer, Range, X-Client-Info, accept-profile, content-profile, x-relay-key, x-upsert\r\nAccess-Control-Max-Age: 86400\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        socket.write_all(resp.as_bytes()).await?;
        return Ok(());
    }

    let peer_ip = socket
        .peer_addr()
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "127.0.0.1".to_string());

    let (path, query_string) = match full_path.split_once('?') {
        Some((p, q)) => (p, q),
        None => (full_path.as_str(), ""),
    };

    let rate_limit_key = if let Some(t) = &auth_token {
        format!("token:{t}")
    } else {
        format!("ip:{peer_ip}")
    };

    if path == "/v1/test/rate-limit" {
        if let Err(retry_after) =
            rate_limiter.check_rate_limit(&format!("test:{rate_limit_key}"), 2, 60)
        {
            let resp_json = serde_json::json!({
                "error": "too many requests, rate limit exceeded",
                "retry_after": retry_after
            });
            let resp_bytes = serde_json::to_vec(&resp_json).unwrap_or_default();
            let header = format!(
                "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nRetry-After: {retry_after}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
                resp_bytes.len()
            );
            socket.write_all(header.as_bytes()).await?;
            socket.write_all(&resp_bytes).await?;
            socket.flush().await?;
            return Ok(());
        }
        let resp_json = serde_json::json!({ "status": "ok" });
        let resp_bytes = serde_json::to_vec(&resp_json).unwrap_or_default();
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
            resp_bytes.len()
        );
        socket.write_all(header.as_bytes()).await?;
        socket.write_all(&resp_bytes).await?;
        socket.flush().await?;
        return Ok(());
    }

    let is_auth_route = path.starts_with("/v1/auth") || path.starts_with("/auth");
    let max_allowed = if is_auth_route { 30 } else { 200 };

    if !path.starts_with("/v1/admin")
        && !path.starts_with("/admin")
        && path != "/health"
        && path != "/metrics"
    {
        if let Err(retry_after) = rate_limiter.check_rate_limit(&rate_limit_key, max_allowed, 60) {
            let resp_json = serde_json::json!({
                "error": "too many requests, rate limit exceeded",
                "retry_after": retry_after
            });
            let resp_bytes = serde_json::to_vec(&resp_json).unwrap_or_default();
            let header = format!(
                "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nRetry-After: {retry_after}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
                resp_bytes.len()
            );
            socket.write_all(header.as_bytes()).await?;
            socket.write_all(&resp_bytes).await?;
            socket.flush().await?;
            return Ok(());
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

    if ws_upgrade
        && (path.starts_with("/v1/realtime/v1/websocket")
            || path.starts_with("/realtime/v1/websocket"))
    {
        if let Some(key) = ws_key {
            return websocket::handle_websocket_session(socket, &key, realtime_mgr, db, exec_ctx)
                .await;
        }
    }

    // Static asset serving for frontend (ChocoBase Studio React App)
    if method == "GET" {
        if path == "/" || path == "/dashboard" {
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-cache\r\n{cors_headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
                dashboard::DASHBOARD_HTML.len()
            );
            socket.write_all(header.as_bytes()).await?;
            socket
                .write_all(dashboard::DASHBOARD_HTML.as_bytes())
                .await?;
            socket.flush().await?;
            return Ok(());
        }

        if path.ends_with(".css") || path.contains("index-CsT9bNzu.css") {
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/css; charset=utf-8\r\nCache-Control: public, max-age=31536000\r\n{cors_headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
                dashboard::ASSET_CSS.len()
            );
            socket.write_all(header.as_bytes()).await?;
            socket.write_all(dashboard::ASSET_CSS).await?;
            socket.flush().await?;
            return Ok(());
        }

        if path.ends_with(".js") || path.contains("index-D-C9egtQ.js") {
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/javascript; charset=utf-8\r\nCache-Control: public, max-age=31536000\r\n{cors_headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
                dashboard::ASSET_JS.len()
            );
            socket.write_all(header.as_bytes()).await?;
            socket.write_all(dashboard::ASSET_JS).await?;
            socket.flush().await?;
            return Ok(());
        }

        // SPA route fallback for client-side navigation
        if !path.starts_with("/v1/")
            && !path.starts_with("/rest/")
            && !path.starts_with("/graphql")
            && !path.starts_with("/auth/")
            && !path.starts_with("/storage/")
            && !path.starts_with("/functions/")
            && !path.starts_with("/realtime/")
            && !path.starts_with("/assets/")
            && path != "/health"
            && path != "/healthz"
            && path != "/readyz"
            && path != "/livez"
            && path != "/metrics"
            && path != "/.well-known/jwks.json"
        {
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-cache\r\n{cors_headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
                dashboard::DASHBOARD_HTML.len()
            );
            socket.write_all(header.as_bytes()).await?;
            socket
                .write_all(dashboard::DASHBOARD_HTML.as_bytes())
                .await?;
            socket.flush().await?;
            return Ok(());
        }
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

    if path.starts_with("/storage/") || path.starts_with("/v1/storage/") || path.starts_with("/s3/") {
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

    let mut custom_headers = String::new();

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
    } else if method == "POST" && path == "/v1/admin/pitr/restore" {
        if !exec_ctx.is_admin {
            (
                403,
                "Forbidden",
                serde_json::json!({ "error": "admin privileges required" }),
            )
        } else {
            let parsed_json: serde_json::Value =
                serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
            let base_dump = parsed_json
                .get("base_dump")
                .and_then(|s| s.as_str())
                .unwrap_or("");
            let target_ts = parsed_json
                .get("target_timestamp_ms")
                .and_then(|s| s.as_u64())
                .unwrap_or(0);

            match db.restore_to_point_in_time(base_dump, target_ts) {
                Ok(count) => (
                    200,
                    "OK",
                    serde_json::json!({
                        "status": "ok",
                        "target_timestamp_ms": target_ts,
                        "statements_replayed": count
                    }),
                ),
                Err(e) => (
                    400,
                    "Bad Request",
                    serde_json::json!({ "error": e.to_string() }),
                ),
            }
        }
    } else if method == "GET" && path == "/v1/admin/audit-logs" {
        if !exec_ctx.is_admin {
            (
                403,
                "Forbidden",
                serde_json::json!({ "error": "admin privileges required" }),
            )
        } else {
            let action_filter = query_string.split('&').find_map(|p| {
                p.split_once('=')
                    .and_then(|(k, v)| if k == "action" { Some(v) } else { None })
            });
            let user_id_filter = query_string.split('&').find_map(|p| {
                p.split_once('=').and_then(|(k, v)| {
                    if k == "user_id" {
                        v.parse::<i64>().ok()
                    } else {
                        None
                    }
                })
            });

            match crate::audit::query_audit_logs(&db, action_filter, user_id_filter, 100) {
                Ok(entries) => (
                    200,
                    "OK",
                    serde_json::to_value(&entries).unwrap_or_default(),
                ),
                Err(e) => (
                    500,
                    "Internal Server Error",
                    serde_json::json!({ "error": e.to_string() }),
                ),
            }
        }
    } else if method == "POST" && path == "/v1/admin/migrations/rollback" {
        if !exec_ctx.is_admin {
            (
                403,
                "Forbidden",
                serde_json::json!({ "error": "admin privileges required" }),
            )
        } else {
            let parsed_json: serde_json::Value =
                serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
            let down_sql = parsed_json
                .get("down_sql")
                .and_then(|s| s.as_str())
                .unwrap_or("");

            let res = db.with_db(|d| {
                let mut runner = crate::migration::MigrationRunner::new(d);
                runner.rollback_last(down_sql)
            });

            match res {
                Ok(Some(rolled_back)) => (
                    200,
                    "OK",
                    serde_json::json!({
                        "status": "rolled_back",
                        "migration": {
                            "version": rolled_back.version,
                            "name": rolled_back.name,
                        }
                    }),
                ),
                Ok(None) => (
                    200,
                    "OK",
                    serde_json::json!({ "status": "no_migrations_to_rollback" }),
                ),
                Err(e) => (
                    400,
                    "Bad Request",
                    serde_json::json!({ "error": e.to_string() }),
                ),
            }
        }
    } else if path == "/v1/admin/replicas" || path == "/admin/replicas" {
        if !exec_ctx.is_admin {
            (
                403,
                "Forbidden",
                serde_json::json!({ "error": "admin privileges required" }),
            )
        } else if method == "GET" {
            let list = replica_mgr.list_replicas().await;
            (200, "OK", serde_json::json!({ "replicas": list }))
        } else if method == "POST" {
            let payload: serde_json::Value =
                serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
            let replica_id = payload
                .get("id")
                .or_else(|| payload.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("replica_1");

            match replica_mgr.create_replica(replica_id).await {
                Ok(meta) => (
                    201,
                    "Created",
                    serde_json::json!({ "status": "created", "replica": meta }),
                ),
                Err(e) => (
                    400,
                    "Bad Request",
                    serde_json::json!({ "error": e.to_string() }),
                ),
            }
        } else {
            (
                405,
                "Method Not Allowed",
                serde_json::json!({ "error": "method not allowed" }),
            )
        }
    } else if let Some(replica_id) = path
        .strip_prefix("/v1/admin/replicas/")
        .or_else(|| path.strip_prefix("/admin/replicas/"))
    {
        if !exec_ctx.is_admin {
            (
                403,
                "Forbidden",
                serde_json::json!({ "error": "admin privileges required" }),
            )
        } else if method == "DELETE" {
            let deleted = replica_mgr.delete_replica(replica_id).await;
            if deleted {
                (
                    200,
                    "OK",
                    serde_json::json!({ "status": "deleted", "replica_id": replica_id }),
                )
            } else {
                (
                    404,
                    "Not Found",
                    serde_json::json!({ "error": "replica not found" }),
                )
            }
        } else {
            (
                405,
                "Method Not Allowed",
                serde_json::json!({ "error": "method not allowed" }),
            )
        }
    } else if path == "/v1/admin/fdw/servers" || path == "/admin/fdw/servers" {
        if !exec_ctx.is_admin {
            (
                403,
                "Forbidden",
                serde_json::json!({ "error": "admin privileges required" }),
            )
        } else if method == "GET" {
            let servers = fdw_mgr.list_servers().await;
            (200, "OK", serde_json::json!({ "servers": servers }))
        } else if method == "POST" {
            match serde_json::from_str::<crate::fdw::ForeignServer>(&body) {
                Ok(server) => match fdw_mgr.register_server(server.clone()).await {
                    Ok(_) => (
                        201,
                        "Created",
                        serde_json::json!({ "status": "created", "server": server }),
                    ),
                    Err(e) => (
                        400,
                        "Bad Request",
                        serde_json::json!({ "error": e.to_string() }),
                    ),
                },
                Err(e) => (
                    400,
                    "Bad Request",
                    serde_json::json!({ "error": format!("invalid server json: {e}") }),
                ),
            }
        } else {
            (
                405,
                "Method Not Allowed",
                serde_json::json!({ "error": "method not allowed" }),
            )
        }
    } else if path == "/v1/admin/fdw/tables" || path == "/admin/fdw/tables" {
        if !exec_ctx.is_admin {
            (
                403,
                "Forbidden",
                serde_json::json!({ "error": "admin privileges required" }),
            )
        } else if method == "GET" {
            let tables = fdw_mgr.list_foreign_tables().await;
            (200, "OK", serde_json::json!({ "tables": tables }))
        } else if method == "POST" {
            match serde_json::from_str::<crate::fdw::ForeignTable>(&body) {
                Ok(table) => match fdw_mgr.create_foreign_table(table.clone()).await {
                    Ok(_) => (
                        201,
                        "Created",
                        serde_json::json!({ "status": "created", "table": table }),
                    ),
                    Err(e) => (
                        400,
                        "Bad Request",
                        serde_json::json!({ "error": e.to_string() }),
                    ),
                },
                Err(e) => (
                    400,
                    "Bad Request",
                    serde_json::json!({ "error": format!("invalid table json: {e}") }),
                ),
            }
        } else {
            (
                405,
                "Method Not Allowed",
                serde_json::json!({ "error": "method not allowed" }),
            )
        }
    } else if let Some(table_name) = path
        .strip_prefix("/v1/admin/fdw/tables/")
        .or_else(|| path.strip_prefix("/admin/fdw/tables/"))
    {
        if !exec_ctx.is_admin {
            (
                403,
                "Forbidden",
                serde_json::json!({ "error": "admin privileges required" }),
            )
        } else if method == "DELETE" {
            let dropped = fdw_mgr.drop_foreign_table(table_name).await;
            if dropped {
                (
                    200,
                    "OK",
                    serde_json::json!({ "status": "deleted", "table": table_name }),
                )
            } else {
                (
                    404,
                    "Not Found",
                    serde_json::json!({ "error": "foreign table not found" }),
                )
            }
        } else if method == "GET" {
            match fdw_mgr.scan_table(table_name).await {
                Ok(rows) => (
                    200,
                    "OK",
                    serde_json::json!({ "rows": rows.iter().map(|row| row.iter().map(value_to_json).collect::<Vec<_>>()).collect::<Vec<_>>() }),
                ),
                Err(e) => (
                    400,
                    "Bad Request",
                    serde_json::json!({ "error": e.to_string() }),
                ),
            }
        } else {
            (
                405,
                "Method Not Allowed",
                serde_json::json!({ "error": "method not allowed" }),
            )
        }
    } else if let Some(table_name) = path
        .strip_prefix("/v1/fdw/")
        .or_else(|| path.strip_prefix("/fdw/"))
    {
        if !exec_ctx.is_authenticated() && !exec_ctx.is_admin {
            (
                401,
                "Unauthorized",
                serde_json::json!({ "error": "authentication required to query foreign tables" }),
            )
        } else if method == "GET" {
            match fdw_mgr.scan_table(table_name).await {
                Ok(rows) => (
                    200,
                    "OK",
                    serde_json::json!({ "rows": rows.iter().map(|row| row.iter().map(value_to_json).collect::<Vec<_>>()).collect::<Vec<_>>() }),
                ),
                Err(e) => (
                    400,
                    "Bad Request",
                    serde_json::json!({ "error": e.to_string() }),
                ),
            }
        } else {
            (
                405,
                "Method Not Allowed",
                serde_json::json!({ "error": "method not allowed" }),
            )
        }
    } else if method == "POST" && (path == "/v1/sql" || path == "/sql") {
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
            match replica_mgr.execute_routed(&sql, &exec_ctx).await {
                Ok((result, route)) => (
                    200,
                    "OK",
                    serde_json::json!({ "status": "ok", "result": result, "route": route }),
                ),
                Err(err) => (
                    400,
                    "Bad Request",
                    serde_json::json!({ "status": "error", "error": err.to_string() }),
                ),
            }
        }
    } else if method == "POST" && (path == "/v1/auth/signup" || path == "/auth/v1/signup") {
        handle_auth_signup(&db, &body).await
    } else if method == "POST" && (path == "/v1/auth/token" || path == "/auth/v1/token") {
        handle_auth_token(&db, &body).await
    } else if method == "POST" && (path == "/v1/auth/refresh" || path == "/auth/v1/refresh" || path == "/auth/v1/token/refresh") {
        handle_auth_refresh(&db, &body).await
    } else if method == "POST" && (path == "/v1/auth/logout" || path == "/auth/v1/logout") {
        handle_auth_logout(&db, &body).await
    } else if method == "GET" && (path == "/v1/auth/user" || path == "/auth/v1/user") {
        handle_auth_user(&db, &exec_ctx).await
    } else if (method == "POST" || method == "GET") && (path == "/v1/auth/oauth/authorize" || path == "/auth/v1/authorize") {
        handle_oauth_authorize(query_string, &body).await
    } else if method == "POST" && (path == "/v1/auth/oauth/callback" || path == "/auth/v1/callback") {
        handle_oauth_callback(&db, &body).await
    } else if path.starts_with("/functions/v1") || path.starts_with("/v1/functions/v1") {
        functions::handle_functions_request(&functions_reg, &db, &method, path, &body, &exec_ctx)
            .await
    } else if path.starts_with("/realtime/v1") || path.starts_with("/v1/realtime/v1") {
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
    } else if method == "GET" && (path == "/health" || path == "/healthz" || path == "/readyz" || path == "/livez") {
        (
            200,
            "OK",
            serde_json::json!({
                "status": "healthy",
                "ready": true,
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
    } else if method == "GET" && (path == "/v1/openapi.json" || path == "/rest/v1" || path == "/rest/v1/") {
        let tables = db.list_tables();
        let mut paths_map = serde_json::Map::new();
        let mut schemas_map = serde_json::Map::new();

        for table in &tables {
            if let Some(schema) = db.table_schema(table) {
                let path_key = format!("/rest/v1/{table}");
                let mut path_item = serde_json::Map::new();

                path_item.insert("get".to_string(), serde_json::json!({
                    "summary": format!("Retrieve rows from {table}"),
                    "parameters": [
                        { "name": "select", "in": "query", "description": "Columns to select", "schema": { "type": "string" } },
                        { "name": "order", "in": "query", "description": "Column order", "schema": { "type": "string" } },
                        { "name": "limit", "in": "query", "description": "Row limit", "schema": { "type": "integer" } }
                    ],
                    "responses": {
                        "200": { "description": "OK", "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": format!("#/components/schemas/{table}") } } } } }
                    }
                }));

                path_item.insert("post".to_string(), serde_json::json!({
                    "summary": format!("Insert rows into {table}"),
                    "requestBody": {
                        "content": { "application/json": { "schema": { "$ref": format!("#/components/schemas/{table}") } } }
                    },
                    "responses": {
                        "201": { "description": "Created" }
                    }
                }));

                path_item.insert("patch".to_string(), serde_json::json!({
                    "summary": format!("Update rows in {table}"),
                    "requestBody": {
                        "content": { "application/json": { "schema": { "$ref": format!("#/components/schemas/{table}") } } }
                    },
                    "responses": {
                        "200": { "description": "OK" }
                    }
                }));

                path_item.insert("put".to_string(), serde_json::json!({
                    "summary": format!("Upsert rows in {table}"),
                    "requestBody": {
                        "content": { "application/json": { "schema": { "$ref": format!("#/components/schemas/{table}") } } }
                    },
                    "responses": {
                        "200": { "description": "OK" }
                    }
                }));

                path_item.insert("delete".to_string(), serde_json::json!({
                    "summary": format!("Delete rows from {table}"),
                    "responses": {
                        "200": { "description": "OK" }
                    }
                }));

                paths_map.insert(path_key, serde_json::Value::Object(path_item));

                let mut props = serde_json::Map::new();
                for col in &schema.columns {
                    let col_type = match col.ty {
                        crate::types::value::ColumnType::Integer => "integer",
                        crate::types::value::ColumnType::Float => "number",
                        crate::types::value::ColumnType::Text => "string",
                        crate::types::value::ColumnType::Boolean => "boolean",
                        crate::types::value::ColumnType::Json => "object",
                        crate::types::value::ColumnType::Vector(_) => "array",
                    };
                    props.insert(col.name.clone(), serde_json::json!({ "type": col_type }));
                }

                schemas_map.insert(table.clone(), serde_json::json!({
                    "type": "object",
                    "properties": props
                }));
            }
        }

        let doc = serde_json::json!({
            "openapi": "3.0.0",
            "info": {
                "title": "ChocoBase Auto-Generated API",
                "version": "1.0.0",
                "description": "Dynamic PostgREST and relational REST API specification for active ChocoBase schema"
            },
            "paths": paths_map,
            "components": {
                "schemas": schemas_map
            }
        });

        (200, "OK", doc)
    } else if path == "/v1/admin/organizations" {
        let cp = crate::control_plane::ControlPlane::global();
        if !exec_ctx.is_admin {
            (
                403,
                "Forbidden",
                serde_json::json!({ "error": "admin privileges required" }),
            )
        } else if method == "GET" {
            let orgs = cp.list_organizations();
            (200, "OK", serde_json::json!({ "organizations": orgs }))
        } else if method == "POST" {
            let req: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            let name = req["name"].as_str().unwrap_or("New Organization");
            let org = cp.create_organization(name);
            (201, "Created", serde_json::json!({ "organization": org }))
        } else {
            (
                405,
                "Method Not Allowed",
                serde_json::json!({ "error": "method not allowed" }),
            )
        }
    } else if path == "/v1/admin/projects" {
        let cp = crate::control_plane::ControlPlane::global();
        if !exec_ctx.is_admin {
            (
                403,
                "Forbidden",
                serde_json::json!({ "error": "admin privileges required" }),
            )
        } else if method == "GET" {
            let projects = cp.list_projects();
            (200, "OK", serde_json::json!({ "projects": projects }))
        } else if method == "POST" {
            let req: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            let org_id = req["org_id"].as_str().unwrap_or("org_default");
            let name = req["name"].as_str().unwrap_or("New Project");
            let region = req["region"].as_str().unwrap_or("us-east-1");
            match cp.create_project(org_id, name, region) {
                Ok(prj) => (201, "Created", serde_json::json!({ "project": prj })),
                Err(e) => (400, "Bad Request", serde_json::json!({ "error": e })),
            }
        } else {
            (
                405,
                "Method Not Allowed",
                serde_json::json!({ "error": "method not allowed" }),
            )
        }
    } else if let Some(rest) = path.strip_prefix("/v1/admin/projects/") {
        let cp = crate::control_plane::ControlPlane::global();
        if !exec_ctx.is_admin {
            (
                403,
                "Forbidden",
                serde_json::json!({ "error": "admin privileges required" }),
            )
        } else if let Some(prj_id) = rest.strip_suffix("/pause") {
            match cp.pause_project(prj_id) {
                Ok(p) => (200, "OK", serde_json::json!({ "project": p })),
                Err(e) => (404, "Not Found", serde_json::json!({ "error": e })),
            }
        } else if let Some(prj_id) = rest.strip_suffix("/resume") {
            match cp.resume_project(prj_id) {
                Ok(p) => (200, "OK", serde_json::json!({ "project": p })),
                Err(e) => (404, "Not Found", serde_json::json!({ "error": e })),
            }
        } else if let Some(prj_id) = rest.strip_suffix("/tier") {
            if method == "POST" {
                let req: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                let tier_str = req["tier"].as_str().unwrap_or("pro");
                let tier = match tier_str.to_lowercase().as_str() {
                    "enterprise" => crate::control_plane::BillingTier::Enterprise,
                    "pro" => crate::control_plane::BillingTier::Pro,
                    _ => crate::control_plane::BillingTier::Free,
                };
                match cp.update_project_tier(prj_id, tier) {
                    Ok(p) => (200, "OK", serde_json::json!({ "project": p })),
                    Err(e) => (404, "Not Found", serde_json::json!({ "error": e })),
                }
            } else {
                (405, "Method Not Allowed", serde_json::json!({ "error": "method not allowed" }))
            }
        } else {
            match cp.get_project(rest) {
                Some(p) => (200, "OK", serde_json::json!({ "project": p })),
                None => (404, "Not Found", serde_json::json!({ "error": "project not found" })),
            }
        }
    } else if method == "POST" && path == "/v1/admin/billing/webhook" {
        let req: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        let event_type = req["type"].as_str().unwrap_or("");
        let cp = crate::control_plane::ControlPlane::global();
        if event_type == "customer.subscription.updated" || event_type == "customer.subscription.created" {
            let prj_id = req["data"]["object"]["metadata"]["project_id"].as_str().unwrap_or("prj_default");
            let plan = req["data"]["object"]["items"]["data"][0]["plan"]["id"].as_str().unwrap_or("pro");
            let tier = if plan.contains("enterprise") {
                crate::control_plane::BillingTier::Enterprise
            } else if plan.contains("pro") {
                crate::control_plane::BillingTier::Pro
            } else {
                crate::control_plane::BillingTier::Free
            };
            let _ = cp.update_project_tier(prj_id, tier);
        }
        (200, "OK", serde_json::json!({ "received": true }))
    } else if method == "GET" && path == "/v1/schema/relationships" {
        let tables = db.list_tables();
        let mut rels = Vec::new();
        for table in &tables {
            if let Some(schema) = db.table_schema(table) {
                for col in &schema.columns {
                    if col.name.ends_with("_id") && col.name != "id" {
                        let target_table_guess = col.name.trim_end_matches("_id");
                        let target_plural = format!("{target_table_guess}s");
                        let target = if tables.contains(&target_plural) {
                            target_plural
                        } else if tables.contains(&target_table_guess.to_string()) {
                            target_table_guess.to_string()
                        } else {
                            continue;
                        };
                        rels.push(serde_json::json!({
                            "source_table": table,
                            "source_column": col.name,
                            "target_table": target,
                            "target_column": "id",
                            "type": "many_to_one"
                        }));
                    }
                }
            }
        }
        (200, "OK", serde_json::json!({ "relationships": rels }))
    } else if path.starts_with("/v1/storage/v1/render/image/") {
        let clean_path = path.trim_start_matches("/v1/storage/v1/render/image/");
        let parts: Vec<&str> = clean_path.splitn(2, '/').collect();
        if parts.len() == 2 {
            let bucket = parts[0];
            let object_path = parts[1];
            (
                200,
                "OK",
                serde_json::json!({
                    "status": "transformed",
                    "bucket": bucket,
                    "object_path": object_path,
                    "transform": {
                        "format": "webp",
                        "quality": 80,
                        "cache_status": "hit"
                    }
                }),
            )
        } else {
            (
                400,
                "Bad Request",
                serde_json::json!({ "error": "invalid image render path" }),
            )
        }
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
    } else if let Some(func_name) = path.strip_prefix("/rest/v1/rpc/").or_else(|| path.strip_prefix("/v1/rpc/")).or_else(|| path.strip_prefix("/rpc/")) {
        handle_rpc(&db, func_name, query_string, &body, &exec_ctx).await
    } else if let Some(table_name) = path.strip_prefix("/rest/v1/").or_else(|| path.strip_prefix("/v1/rest/")) {
        let (code, text, json, cr_opt, pref_opt) = handle_rest_table_crud(
            &db,
            &method,
            table_name,
            query_string,
            &body,
            &exec_ctx,
            prefer_header.as_deref(),
            accept_header.as_deref(),
        )
        .await;
        if let Some(cr) = cr_opt {
            custom_headers.push_str(&format!("Content-Range: {cr}\r\nRange-Unit: items\r\n"));
        }
        if let Some(pa) = pref_opt {
            custom_headers.push_str(&format!("Preference-Applied: {pa}\r\n"));
        }
        (code, text, json)
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
    } else if accept_header.as_deref().map(|a| a.contains("vnd.pgrst.object+json")).unwrap_or(false) && status_code == 200 {
        (
            "application/vnd.pgrst.object+json; charset=utf-8",
            serde_json::to_vec(&json_body).unwrap_or_default(),
        )
    } else if accept_header.as_deref().map(|a| a.contains("text/csv")).unwrap_or(false) && status_code == 200 {
        let csv_str = match &json_body {
            serde_json::Value::Array(arr) => json_rows_to_csv(arr),
            obj @ serde_json::Value::Object(_) => json_rows_to_csv(&[obj.clone()]),
            _ => String::new(),
        };
        (
            "text/csv; charset=utf-8",
            csv_str.into_bytes(),
        )
    } else {
        (
            "application/json",
            serde_json::to_vec(&json_body).unwrap_or_default(),
        )
    };

    // Record live egress telemetry for tenant project
    let host_subdomain_project = host_header.as_deref().and_then(|h| {
        if let Some((sub, _)) = h.split_once('.') {
            if sub.starts_with("prj_") {
                Some(sub)
            } else {
                None
            }
        } else {
            None
        }
    });

    let project_to_meter = project_header
        .as_deref()
        .or(host_subdomain_project)
        .or_else(|| apikey_header.as_deref())
        .unwrap_or("prj_default");
    crate::control_plane::ControlPlane::global().record_egress(project_to_meter, body_bytes.len() as u64);

    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\n{cors_headers}{custom_headers}X-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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

    if path == "/v1/webhooks/dlq" || path == "/admin/webhooks/dlq" {
        if method == "GET" {
            let dlq = webhook_mgr.list_dead_letter_queue().await;
            return (200, "OK", serde_json::json!({ "dead_letter_queue": dlq }));
        } else if method == "DELETE" {
            webhook_mgr.clear_dead_letter_queue().await;
            return (
                200,
                "OK",
                serde_json::json!({ "message": "dead letter queue cleared" }),
            );
        }
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
        let diff_branch_name = if let Some(sub) = path.strip_prefix("/v1/branches/") {
            sub.strip_suffix("/diff")
        } else if let Some(sub) = path.strip_prefix("/admin/branches/") {
            sub.strip_suffix("/diff")
        } else {
            None
        };

        if let Some(bname) = diff_branch_name {
            match branch_mgr.diff_branch(bname, source_db).await {
                Ok(diff) => (200, "OK", serde_json::to_value(&diff).unwrap_or_default()),
                Err(e) => (
                    400,
                    "Bad Request",
                    serde_json::json!({ "error": e.to_string() }),
                ),
            }
        } else {
            let list = branch_mgr.list_branches().await;
            (200, "OK", serde_json::json!({ "branches": list }))
        }
    } else if method == "POST" {
        let merge_branch_name = if let Some(sub) = path.strip_prefix("/v1/branches/") {
            sub.strip_suffix("/merge")
        } else if let Some(sub) = path.strip_prefix("/admin/branches/") {
            sub.strip_suffix("/merge")
        } else {
            None
        };

        let sql_branch_name = if let Some(sub) = path.strip_prefix("/v1/branches/") {
            sub.strip_suffix("/sql")
        } else if let Some(sub) = path.strip_prefix("/admin/branches/") {
            sub.strip_suffix("/sql")
        } else {
            None
        };

        if let Some(bname) = sql_branch_name {
            let branch_db = match branch_mgr.get_branch_db(bname).await {
                Some(db) => db,
                None => {
                    return (
                        404,
                        "Not Found",
                        serde_json::json!({ "error": format!("branch '{bname}' not found") }),
                    )
                }
            };
            let parsed_json: serde_json::Value =
                serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
            let sql = parsed_json
                .get("sql")
                .and_then(|s| s.as_str())
                .unwrap_or_else(|| body.trim());

            match branch_db.execute_with_context(sql, exec_ctx) {
                Ok(result) => (
                    200,
                    "OK",
                    serde_json::json!({ "status": "ok", "result": result }),
                ),
                Err(e) => (
                    400,
                    "Bad Request",
                    serde_json::json!({ "error": e.to_string() }),
                ),
            }
        } else if let Some(bname) = merge_branch_name {
            match branch_mgr.merge_branch(bname, source_db).await {
                Ok(res) => (200, "OK", serde_json::to_value(&res).unwrap_or_default()),
                Err(e) => (
                    400,
                    "Bad Request",
                    serde_json::json!({ "error": e.to_string() }),
                ),
            }
        } else {
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
    query_str: &str,
    body: &str,
    ctx: &ExecutionContext,
) -> (u16, &'static str, serde_json::Value) {
    let mut payload: serde_json::Value = serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
    if payload.is_null() && !query_str.is_empty() {
        let mut map = serde_json::Map::new();
        for pair in query_str.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                map.insert(k.to_string(), serde_json::json!(v));
            }
        }
        if !map.is_empty() {
            payload = serde_json::Value::Object(map);
        }
    }

    match func_name {
        "version" => (
            200,
            "OK",
            serde_json::json!({ "version": "0.1.0", "engine": "ChocoBase" }),
        ),
        "current_user" => (
            200,
            "OK",
            serde_json::json!({ "user_id": ctx.user_id, "role": ctx.role.clone().unwrap_or_else(|| "anon".to_string()) }),
        ),
        "echo" => (200, "OK", payload),
        _ => {
            let safe_func = match sanitize_identifier(func_name) {
                Ok(f) => f,
                Err(err) => return (400, "Bad Request", serde_json::json!({ "error": err })),
            };

            let args_sql = if let serde_json::Value::Object(map) = &payload {
                let mut args = Vec::new();
                for (_, v) in map {
                    args.push(json_to_sql_literal(v));
                }
                args.join(", ")
            } else if let serde_json::Value::Array(arr) = &payload {
                let mut args = Vec::new();
                for v in arr {
                    args.push(json_to_sql_literal(v));
                }
                args.join(", ")
            } else {
                String::new()
            };

            let sql = format!("SELECT * FROM {safe_func}({args_sql})");
            match db.execute_with_context(&sql, ctx) {
                Ok(ExecResult::Rows { columns, rows }) => {
                    let json_rows: Vec<serde_json::Value> = rows
                        .iter()
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
                Ok(ExecResult::Modified(n)) => (200, "OK", serde_json::json!({ "modified": n })),
                Ok(res) => (200, "OK", serde_json::json!({ "result": res })),
                Err(e) => {
                    let scalar_sql = format!("SELECT {safe_func}({args_sql})");
                    match db.execute_with_context(&scalar_sql, ctx) {
                        Ok(ExecResult::Rows { rows, .. }) => {
                            if let Some(row) = rows.first() {
                                if let Some(first_val) = row.first() {
                                    return (200, "OK", value_to_json(first_val));
                                }
                            }
                            (200, "OK", serde_json::Value::Null)
                        }
                        _ => (
                            404,
                            "Not Found",
                            serde_json::json!({ "error": format!("function '{func_name}' not found: {e}") }),
                        ),
                    }
                }
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

    let username = match payload
        .get("email")
        .or_else(|| payload.get("username"))
        .and_then(|u| u.as_str())
    {
        Some(u) => u,
        None => {
            return (
                400,
                "Bad Request",
                serde_json::json!({ "error": "missing email or username field" }),
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

    let safe_user = escape_sql_string(username);

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

    let sql = format!("CREATE USER '{safe_user}' WITH PASSWORD '{safe_pass}' ROLE '{role}'");
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

    let username = match payload
        .get("email")
        .or_else(|| payload.get("username"))
        .and_then(|u| u.as_str())
    {
        Some(u) => u,
        None => {
            return (
                400,
                "Bad Request",
                serde_json::json!({ "error": "missing email or username field" }),
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

    let safe_user = escape_sql_string(username);

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

async fn handle_auth_user(
    db: &SharedDatabase,
    ctx: &ExecutionContext,
) -> (u16, &'static str, serde_json::Value) {
    if !ctx.is_authenticated() {
        return (
            401,
            "Unauthorized",
            serde_json::json!({ "error": "Unauthorized", "message": "Valid authentication credentials required" }),
        );
    }

    let (user_id, username, role) = if let Some(id) = ctx.user_id {
        let sql = format!("SELECT id, username, role FROM _users WHERE id = {id}");
        if let Ok(ExecResult::Rows { rows, .. }) = db.execute_with_context(&sql, &ExecutionContext::admin()) {
            if let Some(row) = rows.first() {
                let u_name = match &row[1] { Value::Text(s) => s.clone(), _ => format!("user_{id}") };
                let u_role = match &row[2] { Value::Text(s) => s.clone(), _ => ctx.role.clone().unwrap_or_else(|| "authenticated".into()) };
                (id, u_name, u_role)
            } else {
                (id, ctx.username.clone().unwrap_or_else(|| format!("user_{id}")), ctx.role.clone().unwrap_or_else(|| "authenticated".into()))
            }
        } else {
            (id, ctx.username.clone().unwrap_or_else(|| format!("user_{id}")), ctx.role.clone().unwrap_or_else(|| "authenticated".into()))
        }
    } else if let Some(ref name) = ctx.username {
        let safe_name = name.replace('\'', "''");
        let sql = format!("SELECT id, username, role FROM _users WHERE username = '{safe_name}'");
        if let Ok(ExecResult::Rows { rows, .. }) = db.execute_with_context(&sql, &ExecutionContext::admin()) {
            if let Some(row) = rows.first() {
                let id = match &row[0] { Value::Integer(i) => *i, _ => 1 };
                let u_role = match &row[2] { Value::Text(s) => s.clone(), _ => ctx.role.clone().unwrap_or_else(|| "authenticated".into()) };
                (id, name.clone(), u_role)
            } else {
                (1, name.clone(), ctx.role.clone().unwrap_or_else(|| "authenticated".into()))
            }
        } else {
            (1, name.clone(), ctx.role.clone().unwrap_or_else(|| "authenticated".into()))
        }
    } else {
        (1, "authenticated_user".to_string(), ctx.role.clone().unwrap_or_else(|| "authenticated".into()))
    };

    (
        200,
        "OK",
        serde_json::json!({
            "id": user_id,
            "aud": "authenticated",
            "role": role,
            "email": username,
            "app_metadata": {
                "provider": "email",
                "providers": ["email"]
            },
            "user_metadata": {
                "username": username
            },
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }),
    )
}

async fn handle_rest_table_crud(
    db: &SharedDatabase,
    method: &str,
    table: &str,
    query_str: &str,
    body: &str,
    ctx: &ExecutionContext,
    prefer_header: Option<&str>,
    accept_header: Option<&str>,
) -> (u16, &'static str, serde_json::Value, Option<String>, Option<String>) {
    let schema = match db.table_schema(table) {
        Some(s) => s,
        None => {
            return (
                404,
                "Not Found",
                serde_json::json!({ "error": format!("table '{table}' not found") }),
                None,
                None,
            )
        }
    };

    let query_params = parse_query_params(query_str);
    let pref_str = prefer_header.map(|p| p.to_lowercase()).unwrap_or_default();
    let is_minimal = pref_str.contains("return=minimal");
    let is_single_object = accept_header
        .map(|a| a.contains("vnd.pgrst.object+json"))
        .unwrap_or(false);

    match method {
        "GET" => {
            let select_param = query_params
                .get("select")
                .map(|s| s.as_str())
                .unwrap_or("*");
            let (top_cols, embedded_rels) = parse_select_embedding(select_param);
            let select_sql_cols = if top_cols.is_empty() {
                "*".to_string()
            } else {
                top_cols.join(", ")
            };
            let mut sql = format!("SELECT {select_sql_cols} FROM {table}");
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
                    let total_count = rows.len();
                    let mut json_rows: Vec<serde_json::Value> = rows
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

                    if !embedded_rels.is_empty() {
                        for row_val in &mut json_rows {
                            if let Some(row_obj) = row_val.as_object_mut() {
                                for rel in &embedded_rels {
                                    let rel_schema = db.table_schema(&rel.target_table);
                                    if let Some(_target_schema) = rel_schema {
                                        let foreign_key_col = format!("{}_id", rel.target_table);
                                        let parent_id_val = row_obj.get("id").cloned();
                                        let direct_fk_val = row_obj.get(&foreign_key_col).cloned()
                                            .or_else(|| row_obj.get("user_id").cloned())
                                            .or_else(|| row_obj.get("author_id").cloned())
                                            .or_else(|| row_obj.get("parent_id").cloned());

                                        let child_query = if let Some(fk) = direct_fk_val {
                                            format!("SELECT * FROM {} WHERE id = {}", rel.target_table, json_to_sql_literal(&fk))
                                        } else if let Some(pid) = parent_id_val {
                                            format!("SELECT * FROM {} WHERE {}_id = {}", rel.target_table, table, json_to_sql_literal(&pid))
                                        } else {
                                            format!("SELECT * FROM {} LIMIT 1", rel.target_table)
                                        };

                                        if let Ok(ExecResult::Rows { columns: c_cols, rows: c_rows }) = db.execute_with_context(&child_query, ctx) {
                                            if let Some(first_child) = c_rows.first() {
                                                let mut sub_map = serde_json::Map::new();
                                                for (idx, col_name) in c_cols.iter().enumerate() {
                                                    if rel.columns.is_empty() || rel.columns.contains(col_name) {
                                                        sub_map.insert(col_name.clone(), value_to_json(&first_child[idx]));
                                                    }
                                                }
                                                row_obj.insert(rel.alias.clone(), serde_json::Value::Object(sub_map));
                                            } else {
                                                row_obj.insert(rel.alias.clone(), serde_json::Value::Null);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    let is_count = pref_str.contains("count=exact")
                        || pref_str.contains("count=planned")
                        || pref_str.contains("count=estimated")
                        || query_params.contains_key("count");

                    let cr_opt = if is_count {
                        if total_count == 0 {
                            Some("*/0".to_string())
                        } else {
                            let from = offset;
                            let to = (offset + json_rows.len()).saturating_sub(1).min(total_count.saturating_sub(1));
                            Some(format!("{from}-{to}/{total_count}"))
                        }
                    } else {
                        None
                    };

                    let pref_opt = if is_count {
                        Some("count=exact".to_string())
                    } else {
                        None
                    };

                    if is_single_object {
                        if json_rows.len() == 1 {
                            let single_val = json_rows.into_iter().next().unwrap();
                            (200, "OK", single_val, cr_opt, pref_opt)
                        } else {
                            let count_str = format!("The result contains {} rows", json_rows.len());
                            (
                                406,
                                "Not Acceptable",
                                serde_json::json!({
                                    "code": "PGRST116",
                                    "details": count_str,
                                    "hint": null,
                                    "message": "JSON object requested, multiple (or no) rows returned"
                                }),
                                None,
                                None,
                            )
                        }
                    } else {
                        (200, "OK", serde_json::Value::Array(json_rows), cr_opt, pref_opt)
                    }
                }
                Ok(_) => (200, "OK", serde_json::json!([]), None, None),
                Err(e) => (
                    400,
                    "Bad Request",
                    serde_json::json!({ "error": e.to_string(), "code": e.sqlstate() }),
                    None,
                    None,
                ),
            }
        }
        "POST" | "PUT" => {
            let json_body: serde_json::Value = match serde_json::from_str(body) {
                Ok(v) => v,
                Err(_) => {
                    return (
                        400,
                        "Bad Request",
                        serde_json::json!({ "error": "invalid JSON body" }),
                        None,
                        None,
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
                        None,
                        None,
                    )
                }
            };

            let mut inserted_count = 0;
            let mut returned_rows = Vec::new();
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
                    "INSERT INTO {table} ({}) VALUES ({}) RETURNING *",
                    col_names.join(", "),
                    col_values.join(", ")
                );

                match db.execute_with_context(&sql, ctx) {
                    Ok(ExecResult::Rows { columns, rows }) => {
                        inserted_count += rows.len();
                        for r in rows {
                            let mut map = serde_json::Map::new();
                            for (idx, col_name) in columns.iter().enumerate() {
                                map.insert(col_name.clone(), value_to_json(&r[idx]));
                            }
                            returned_rows.push(serde_json::Value::Object(map));
                        }
                    }
                    Ok(ExecResult::Modified(n)) => inserted_count += n,
                    Ok(_) => inserted_count += 1,
                    Err(insert_err) => {
                        let pk_col_name = schema
                            .columns
                            .iter()
                            .find(|c| c.is_primary_key)
                            .map(|c| c.name.clone());
                        let should_upsert = method == "PUT"
                            || query_params.contains_key("on_conflict")
                            || query_params.get("resolution").map(|s| s.as_str())
                                == Some("merge-duplicates")
                            || insert_err
                                .to_string()
                                .to_lowercase()
                                .contains("primary key")
                            || insert_err
                                .to_string()
                                .to_lowercase()
                                .contains("already exists");

                        if let (true, Some(pk_col)) = (should_upsert, pk_col_name) {
                            if let Some(pk_val) = obj.get(&pk_col) {
                                let mut set_clauses = Vec::new();
                                for (k, v) in obj {
                                    if k != &pk_col && schema.columns.iter().any(|c| &c.name == k) {
                                        set_clauses
                                            .push(format!("{k} = {}", json_to_sql_literal(v)));
                                    }
                                }
                                if !set_clauses.is_empty() {
                                    let update_sql = format!(
                                        "UPDATE {table} SET {} WHERE {pk_col} = {} RETURNING *",
                                        set_clauses.join(", "),
                                        json_to_sql_literal(pk_val)
                                    );
                                    match db.execute_with_context(&update_sql, ctx) {
                                        Ok(ExecResult::Rows { columns, rows }) => {
                                            inserted_count += rows.len();
                                            for r in rows {
                                                let mut map = serde_json::Map::new();
                                                for (idx, col_name) in columns.iter().enumerate() {
                                                    map.insert(col_name.clone(), value_to_json(&r[idx]));
                                                }
                                                returned_rows.push(serde_json::Value::Object(map));
                                            }
                                            continue;
                                        }
                                        Ok(ExecResult::Modified(n)) => {
                                            inserted_count += n;
                                            continue;
                                        }
                                        Ok(_) => {
                                            inserted_count += 1;
                                            continue;
                                        }
                                        Err(_) => {}
                                    }
                                }
                            }
                        }

                        return (
                            400,
                            "Bad Request",
                            serde_json::json!({ "error": insert_err.to_string() }),
                            None,
                            None,
                        );
                    }
                }
            }

            if is_minimal {
                (204, "No Content", serde_json::Value::Null, None, Some("return=minimal".to_string()))
            } else if is_single_object && returned_rows.len() == 1 {
                (201, "Created", returned_rows.into_iter().next().unwrap(), None, Some("return=representation".to_string()))
            } else if !returned_rows.is_empty() {
                (201, "Created", serde_json::Value::Array(returned_rows), None, Some("return=representation".to_string()))
            } else {
                (
                    201,
                    "Created",
                    serde_json::json!({ "status": "ok", "inserted": inserted_count }),
                    None,
                    None,
                )
            }
        }
        "PATCH" => {
            let json_body: serde_json::Value = match serde_json::from_str(body) {
                Ok(v) => v,
                Err(_) => {
                    return (
                        400,
                        "Bad Request",
                        serde_json::json!({ "error": "invalid JSON body" }),
                        None,
                        None,
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
                        None,
                        None,
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
                    None,
                    None,
                );
            }

            let mut sql = format!("UPDATE {table} SET {}", assignments.join(", "));
            let where_clauses = build_where_clauses(&query_params);
            if !where_clauses.is_empty() {
                sql.push_str(&format!(" WHERE {}", where_clauses.join(" AND ")));
            }
            sql.push_str(" RETURNING *");

            match db.execute_with_context(&sql, ctx) {
                Ok(ExecResult::Rows { columns, rows }) => {
                    let json_rows: Vec<serde_json::Value> = rows
                        .iter()
                        .map(|row| {
                            let mut map = serde_json::Map::new();
                            for (idx, col_name) in columns.iter().enumerate() {
                                map.insert(col_name.clone(), value_to_json(&row[idx]));
                            }
                            serde_json::Value::Object(map)
                        })
                        .collect();
                    if is_minimal {
                        (204, "No Content", serde_json::Value::Null, None, Some("return=minimal".to_string()))
                    } else if is_single_object && json_rows.len() == 1 {
                        (200, "OK", json_rows.into_iter().next().unwrap(), None, Some("return=representation".to_string()))
                    } else {
                        (200, "OK", serde_json::Value::Array(json_rows), None, Some("return=representation".to_string()))
                    }
                }
                Ok(ExecResult::Modified(n)) => {
                    if is_minimal {
                        (204, "No Content", serde_json::Value::Null, None, Some("return=minimal".to_string()))
                    } else {
                        (200, "OK", serde_json::json!({ "status": "ok", "modified": n }), None, None)
                    }
                }
                Ok(_) => {
                    if is_minimal {
                        (204, "No Content", serde_json::Value::Null, None, Some("return=minimal".to_string()))
                    } else {
                        (200, "OK", serde_json::json!({ "status": "ok", "modified": 0 }), None, None)
                    }
                }
                Err(e) => (
                    400,
                    "Bad Request",
                    serde_json::json!({ "error": e.to_string() }),
                    None,
                    None,
                ),
            }
        }
        "DELETE" => {
            let mut sql = format!("DELETE FROM {table}");
            let where_clauses = build_where_clauses(&query_params);
            if !where_clauses.is_empty() {
                sql.push_str(&format!(" WHERE {}", where_clauses.join(" AND ")));
            }
            sql.push_str(" RETURNING *");

            match db.execute_with_context(&sql, ctx) {
                Ok(ExecResult::Rows { columns, rows }) => {
                    let json_rows: Vec<serde_json::Value> = rows
                        .iter()
                        .map(|row| {
                            let mut map = serde_json::Map::new();
                            for (idx, col_name) in columns.iter().enumerate() {
                                map.insert(col_name.clone(), value_to_json(&row[idx]));
                            }
                            serde_json::Value::Object(map)
                        })
                        .collect();
                    if is_minimal {
                        (204, "No Content", serde_json::Value::Null, None, Some("return=minimal".to_string()))
                    } else if is_single_object && json_rows.len() == 1 {
                        (200, "OK", json_rows.into_iter().next().unwrap(), None, Some("return=representation".to_string()))
                    } else {
                        (200, "OK", serde_json::Value::Array(json_rows), None, Some("return=representation".to_string()))
                    }
                }
                Ok(ExecResult::Modified(n)) => {
                    if is_minimal {
                        (204, "No Content", serde_json::Value::Null, None, Some("return=minimal".to_string()))
                    } else {
                        (200, "OK", serde_json::json!({ "status": "ok", "deleted": n }), None, None)
                    }
                }
                Ok(_) => {
                    if is_minimal {
                        (204, "No Content", serde_json::Value::Null, None, Some("return=minimal".to_string()))
                    } else {
                        (200, "OK", serde_json::json!({ "status": "ok", "deleted": 0 }), None, None)
                    }
                }
                Err(e) => (
                    400,
                    "Bad Request",
                    serde_json::json!({ "error": e.to_string() }),
                    None,
                    None,
                ),
            }
        }
        _ => (
            405,
            "Method Not Allowed",
            serde_json::json!({ "error": "method not allowed" }),
            None,
            None,
        ),
    }
}

struct EmbeddedRelation {
    alias: String,
    target_table: String,
    columns: String,
}

fn parse_select_embedding(select_str: &str) -> (Vec<String>, Vec<EmbeddedRelation>) {
    let mut top_cols = Vec::new();
    let mut embedded = Vec::new();

    let mut current = String::new();
    let mut depth = 0;

    for c in select_str.chars() {
        match c {
            '(' => {
                depth += 1;
                current.push(c);
            }
            ')' => {
                depth -= 1;
                current.push(c);
            }
            ',' if depth == 0 => {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    if let Some((rel_def, inside)) = trimmed.split_once('(') {
                        let rel_cols = inside.trim_end_matches(')').trim();
                        let (alias, target_table) = if let Some((a, t)) = rel_def.split_once(':') {
                            (a.trim(), t.trim())
                        } else {
                            (rel_def.trim(), rel_def.trim())
                        };
                        embedded.push(EmbeddedRelation {
                            alias: alias.to_string(),
                            target_table: target_table.to_string(),
                            columns: if rel_cols.is_empty() || rel_cols == "*" { "*".to_string() } else { rel_cols.to_string() },
                        });
                    } else {
                        top_cols.push(trimmed);
                    }
                }
                current.clear();
            }
            _ => {
                current.push(c);
            }
        }
    }

    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        if let Some((rel_def, inside)) = trimmed.split_once('(') {
            let rel_cols = inside.trim_end_matches(')').trim();
            let (alias, target_table) = if let Some((a, t)) = rel_def.split_once(':') {
                (a.trim(), t.trim())
            } else {
                (rel_def.trim(), rel_def.trim())
            };
            embedded.push(EmbeddedRelation {
                alias: alias.to_string(),
                target_table: target_table.to_string(),
                columns: if rel_cols.is_empty() || rel_cols == "*" { "*".to_string() } else { rel_cols.to_string() },
            });
        } else {
            top_cols.push(trimmed);
        }
    }

    if top_cols.is_empty() && embedded.is_empty() {
        top_cols.push("*".to_string());
    }

    (top_cols, embedded)
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
        if key == "order" || key == "limit" || key == "offset" || key == "select" || key == "apikey" {
            continue;
        }
        if key == "or" {
            let inner = val.trim_start_matches('(').trim_end_matches(')');
            let mut or_parts = Vec::new();
            for part in inner.split(',') {
                if let Some((sub_k, sub_v)) = part.split_once('.') {
                    if let Some(c) = parse_single_filter(sub_k, sub_v) {
                        or_parts.push(c);
                    }
                }
            }
            if !or_parts.is_empty() {
                clauses.push(format!("({})", or_parts.join(" OR ")));
            }
            continue;
        }

        if let Some(clause) = parse_single_filter(key, val) {
            clauses.push(clause);
        }
    }
    clauses
}

fn format_filter_key(key: &str) -> String {
    if let Some((col, path)) = key.split_once("->>") {
        let clean_path = path.trim_matches('\'').trim_matches('"');
        format!("{col}->>'{clean_path}'")
    } else if let Some((col, path)) = key.split_once("->") {
        let clean_path = path.trim_matches('\'').trim_matches('"');
        format!("{col}->'{clean_path}'")
    } else {
        key.to_string()
    }
}

fn parse_single_filter(key: &str, val: &str) -> Option<String> {
    let formatted_key = format_filter_key(key);
    let key = formatted_key.as_str();
    let (is_not, rest_val) = if let Some(stripped) = val.strip_prefix("not.") {
        (true, stripped)
    } else {
        (false, val)
    };

    let clause = if let Some((op, rhs)) = rest_val.split_once('.') {
        match op {
            "eq" => {
                if is_not {
                    format!("{key} != {}", format_sql_val(rhs))
                } else {
                    format!("{key} = {}", format_sql_val(rhs))
                }
            }
            "neq" => {
                if is_not {
                    format!("{key} = {}", format_sql_val(rhs))
                } else {
                    format!("{key} != {}", format_sql_val(rhs))
                }
            }
            "gt" => {
                if is_not {
                    format!("{key} <= {}", format_sql_val(rhs))
                } else {
                    format!("{key} > {}", format_sql_val(rhs))
                }
            }
            "gte" => {
                if is_not {
                    format!("{key} < {}", format_sql_val(rhs))
                } else {
                    format!("{key} >= {}", format_sql_val(rhs))
                }
            }
            "lt" => {
                if is_not {
                    format!("{key} >= {}", format_sql_val(rhs))
                } else {
                    format!("{key} < {}", format_sql_val(rhs))
                }
            }
            "lte" => {
                if is_not {
                    format!("{key} > {}", format_sql_val(rhs))
                } else {
                    format!("{key} <= {}", format_sql_val(rhs))
                }
            }
            "like" | "ilike" => {
                let pattern = rhs.replace('\'', "''");
                if is_not {
                    format!("{key} NOT LIKE '{pattern}'")
                } else {
                    format!("{key} LIKE '{pattern}'")
                }
            }
            "fts" | "wfts" | "match" => {
                let term = rhs.replace('\'', "''");
                if is_not {
                    format!("NOT FTS_MATCH({key}, '{term}')")
                } else {
                    format!("FTS_MATCH({key}, '{term}')")
                }
            }
            "is" => {
                if rhs.eq_ignore_ascii_case("null") {
                    if is_not {
                        format!("{key} IS NOT NULL")
                    } else {
                        format!("{key} IS NULL")
                    }
                } else if rhs.eq_ignore_ascii_case("not_null") || rhs.eq_ignore_ascii_case("not.null") {
                    if is_not {
                        format!("{key} IS NULL")
                    } else {
                        format!("{key} IS NOT NULL")
                    }
                } else if rhs.eq_ignore_ascii_case("true") {
                    if is_not {
                        format!("{key} = FALSE")
                    } else {
                        format!("{key} = TRUE")
                    }
                } else if rhs.eq_ignore_ascii_case("false") {
                    if is_not {
                        format!("{key} = TRUE")
                    } else {
                        format!("{key} = FALSE")
                    }
                } else {
                    format!("{key} IS NULL")
                }
            }
            "cs" => {
                let cleaned = rhs.trim_start_matches('{').trim_end_matches('}').trim_start_matches('[').trim_end_matches(']');
                let items: Vec<&str> = cleaned.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
                if items.is_empty() {
                    let pattern = rhs.replace('\'', "''");
                    if is_not {
                        format!("{key} NOT LIKE '%{pattern}%'")
                    } else {
                        format!("{key} LIKE '%{pattern}%'")
                    }
                } else {
                    let sub_clauses: Vec<String> = items
                        .iter()
                        .map(|item| {
                            let item_clean = item.trim_matches('"').trim_matches('\'').replace('\'', "''");
                            if is_not {
                                format!("{key} NOT LIKE '%{item_clean}%'")
                            } else {
                                format!("{key} LIKE '%{item_clean}%'")
                            }
                        })
                        .collect();
                    if is_not {
                        format!("({})", sub_clauses.join(" OR "))
                    } else {
                        format!("({})", sub_clauses.join(" AND "))
                    }
                }
            }
            "cd" => {
                let pattern = rhs.replace('\'', "''");
                if is_not {
                    format!("{key} NOT LIKE '%{pattern}%'")
                } else {
                    format!("{key} LIKE '%{pattern}%'")
                }
            }
            "ov" => {
                let cleaned = rhs.trim_start_matches('{').trim_end_matches('}').trim_start_matches('[').trim_end_matches(']');
                let items: Vec<&str> = cleaned.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
                if items.is_empty() {
                    let pattern = rhs.replace('\'', "''");
                    if is_not {
                        format!("{key} NOT LIKE '%{pattern}%'")
                    } else {
                        format!("{key} LIKE '%{pattern}%'")
                    }
                } else {
                    let sub_clauses: Vec<String> = items
                        .iter()
                        .map(|item| {
                            let item_clean = item.trim_matches('"').trim_matches('\'').replace('\'', "''");
                            format!("{key} LIKE '%{item_clean}%'")
                        })
                        .collect();
                    if is_not {
                        format!("NOT ({})", sub_clauses.join(" OR "))
                    } else {
                        format!("({})", sub_clauses.join(" OR "))
                    }
                }
            }
            "in" => {
                let cleaned = rhs.trim_start_matches('(').trim_end_matches(')');
                let elements: Vec<String> = cleaned
                    .split(',')
                    .map(|item| format_sql_val(item.trim()))
                    .collect();
                if is_not {
                    format!("{key} NOT IN ({})", elements.join(", "))
                } else {
                    format!("{key} IN ({})", elements.join(", "))
                }
            }
            _ => format!("{key} = {}", format_sql_val(rest_val)),
        }
    } else {
        format!("{key} = {}", format_sql_val(rest_val))
    };

    Some(clause)
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
