//! HTTP request handler for Serverless Functions.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::auth::ExecutionContext;
use crate::engine::SharedDatabase;
use crate::functions::{FunctionMetadata, FunctionRegistry};

pub async fn handle_functions_request(
    registry: &FunctionRegistry,
    db: &SharedDatabase,
    method: &str,
    path: &str,
    body: &str,
    ctx: &ExecutionContext,
) -> (u16, &'static str, serde_json::Value) {
    let subpath = path.strip_prefix("/v1/functions/v1").unwrap_or(path);

    if (subpath.is_empty() || subpath == "/") && method == "GET" {
        let list = registry.list();
        return (200, "OK", serde_json::json!(list));
    }

    if (subpath == "/deploy" || subpath == "deploy") && method == "POST" {
        if !ctx.is_admin {
            return (
                403,
                "Forbidden",
                serde_json::json!({ "error": "admin privileges required to deploy functions" }),
            );
        }

        let payload: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(_) => {
                return (
                    400,
                    "Bad Request",
                    serde_json::json!({ "error": "invalid JSON payload" }),
                )
            }
        };

        let name = match payload.get("name").and_then(|v| v.as_str()) {
            Some(n) => n.to_string(),
            None => {
                return (
                    400,
                    "Bad Request",
                    serde_json::json!({ "error": "missing function name" }),
                )
            }
        };

        let script = payload
            .get("script")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let runtime = payload
            .get("runtime")
            .and_then(|v| v.as_str())
            .unwrap_or("default")
            .to_string();
        let verify_jwt = payload
            .get("verify_jwt")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let timeout_ms = payload
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(5000);

        let mut env_map = HashMap::new();
        if let Some(env_obj) = payload.get("env").and_then(|v| v.as_object()) {
            for (k, v) in env_obj {
                if let Some(val_str) = v.as_str() {
                    env_map.insert(k.clone(), val_str.to_string());
                }
            }
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let meta = FunctionMetadata {
            name: name.clone(),
            runtime,
            entrypoint: "index.ts".into(),
            timeout_ms,
            env: env_map,
            verify_jwt,
            created_at: now,
        };

        match registry.deploy(meta.clone(), script) {
            Ok(_) => (
                201,
                "Created",
                serde_json::json!({
                    "status": "deployed",
                    "function": meta
                }),
            ),
            Err(e) => (
                400,
                "Bad Request",
                serde_json::json!({ "error": e.to_string() }),
            ),
        }
    } else if method == "POST" {
        let func_name = subpath.trim_start_matches('/');
        let payload: serde_json::Value =
            serde_json::from_str(body).unwrap_or(serde_json::Value::Null);

        match registry.execute(func_name, &payload, ctx, db).await {
            Ok(output) => (200, "OK", output),
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
}
