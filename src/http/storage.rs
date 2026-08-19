//! Supabase-Class Object Storage Engine for ChocoBase.
//! Provides bucket and object metadata management, binary storage, public/private access controls,
//! fine-grained per-user authorization, signed download URLs, and RLS integration.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::auth::ExecutionContext;
use crate::engine::{ExecResult, SharedDatabase};
use crate::types::value::Value;

type HmacSha256 = Hmac<Sha256>;

pub fn ensure_storage_tables(db: &SharedDatabase) {
    let buckets_sql = "CREATE TABLE _storage_buckets (id TEXT PRIMARY KEY, name TEXT NOT NULL, public BOOLEAN NOT NULL, created_at INTEGER NOT NULL)";
    let objects_sql = "CREATE TABLE _storage_objects (id TEXT PRIMARY KEY, bucket_id TEXT NOT NULL, name TEXT NOT NULL, owner_id INTEGER, content_type TEXT NOT NULL, size_bytes INTEGER NOT NULL, metadata JSON, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)";
    let lifecycle_sql = "CREATE TABLE _storage_lifecycle_rules (id TEXT PRIMARY KEY, bucket_id TEXT NOT NULL, prefix TEXT NOT NULL, expiry_days INTEGER NOT NULL, created_at INTEGER NOT NULL)";

    let _ = db.execute_with_context(buckets_sql, &ExecutionContext::admin());
    let _ = db.execute_with_context(objects_sql, &ExecutionContext::admin());
    let _ = db.execute_with_context(lifecycle_sql, &ExecutionContext::admin());
}

pub fn get_storage_root() -> PathBuf {
    let data_dir = std::env::var("CHOCOBASE_DATA_DIR").unwrap_or_else(|_| ".".into());
    Path::new(&data_dir).join("storage")
}

pub fn sanitize_object_path(path: &str) -> String {
    let cleaned = path
        .replace('\\', "/")
        .split('/')
        .filter(|s| !s.is_empty() && *s != "." && *s != "..")
        .collect::<Vec<&str>>()
        .join("/");
    cleaned
}

pub fn sign_download_token(bucket: &str, key: &str, expires_at: u64, secret: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC can take key of any size");
    let payload = format!("{bucket}:{key}:{expires_at}");
    mac.update(payload.as_bytes());
    let result = mac.finalize().into_bytes();
    result.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn verify_download_signature(
    bucket: &str,
    key: &str,
    expires_at: u64,
    token: &str,
    secret: &[u8],
) -> bool {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if expires_at <= now {
        return false;
    }

    let expected = sign_download_token(bucket, key, expires_at, secret);
    token.as_bytes().ct_eq(expected.as_bytes()).into()
}

fn get_object_owner(db: &SharedDatabase, obj_id: &str) -> Option<Option<i64>> {
    let esc_id = obj_id.replace('\'', "''");
    let sql = format!("SELECT owner_id FROM _storage_objects WHERE id = '{esc_id}'");
    if let Ok(ExecResult::Rows { rows, .. }) =
        db.execute_with_context(&sql, &ExecutionContext::admin())
    {
        if let Some(r) = rows.first() {
            return match &r[0] {
                Value::Integer(id) => Some(Some(*id)),
                Value::Null => Some(None),
                _ => Some(None),
            };
        }
    }
    None
}

pub async fn handle_storage_request(
    db: &SharedDatabase,
    method: &str,
    path: &str,
    query_str: &str,
    body: &str,
    ctx: &ExecutionContext,
    range_header: Option<&str>,
) -> (
    u16,
    &'static str,
    serde_json::Value,
    Option<(Vec<u8>, String, Option<String>, String)>,
) {
    ensure_storage_tables(db);

    let s3_path = path
        .strip_prefix("/v1/storage/s3/")
        .or_else(|| path.strip_prefix("/s3/"));
    if let Some(s3_subpath) = s3_path {
        if let Some((bucket_id, object_key)) = s3_subpath.split_once('/') {
            let bucket_id = sanitize_object_path(bucket_id);
            let object_key = sanitize_object_path(object_key);
            let obj_id = format!("{bucket_id}/{object_key}");

            match method {
                "PUT" => {
                    let file_path = get_storage_root().join(&bucket_id).join(&object_key);
                    if let Some(parent) = file_path.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    let bytes = body.as_bytes();
                    if fs::write(&file_path, bytes).is_err() {
                        return (
                            500,
                            "Internal Server Error",
                            serde_json::json!({ "error": "failed to write object" }),
                            None,
                        );
                    }
                    let mut hasher = Sha256::new();
                    hasher.update(bytes);
                    let etag = format!("\"{:x}\"", hasher.finalize());
                    let size_bytes = bytes.len() as i64;
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let esc_obj_id = obj_id.replace('\'', "''");
                    let esc_bucket_id = bucket_id.replace('\'', "''");
                    let esc_obj_key = object_key.replace('\'', "''");
                    let insert_sql = format!("INSERT INTO _storage_objects (id, bucket_id, name, owner_id, content_type, size_bytes, metadata, created_at, updated_at) VALUES ('{esc_obj_id}', '{esc_bucket_id}', '{esc_obj_key}', NULL, 'application/octet-stream', {size_bytes}, '{{}}', {now}, {now})");
                    let _ = db.execute_with_context(&insert_sql, &ExecutionContext::admin());
                    return (
                        200,
                        "OK",
                        serde_json::json!({ "ETag": etag, "Key": object_key, "Bucket": bucket_id }),
                        None,
                    );
                }
                "GET" => {
                    let file_path = get_storage_root().join(&bucket_id).join(&object_key);
                    if file_path.exists() {
                        if let Ok(bytes) = fs::read(&file_path) {
                            let mut hasher = Sha256::new();
                            hasher.update(&bytes);
                            let etag = format!("\"{:x}\"", hasher.finalize());
                            let (status_code, status_text, final_bytes, cr_opt) =
                                if let Some(range_str) = range_header {
                                    if let Some(range_val) = range_str.strip_prefix("bytes=") {
                                        let total_len = bytes.len();
                                        let parts: Vec<&str> = range_val.split('-').collect();
                                        let start = parts[0].parse::<usize>().unwrap_or(0);
                                        let end = if parts.len() > 1 && !parts[1].is_empty() {
                                            parts[1]
                                                .parse::<usize>()
                                                .unwrap_or(total_len.saturating_sub(1))
                                                .min(total_len.saturating_sub(1))
                                        } else {
                                            total_len.saturating_sub(1)
                                        };
                                        if start < total_len && start <= end {
                                            let slice = bytes[start..=end].to_vec();
                                            let cr = format!("bytes {start}-{end}/{total_len}");
                                            (206, "Partial Content", slice, Some(cr))
                                        } else {
                                            (200, "OK", bytes, None)
                                        }
                                    } else {
                                        (200, "OK", bytes, None)
                                    }
                                } else {
                                    (200, "OK", bytes, None)
                                };
                            return (
                                status_code,
                                status_text,
                                serde_json::Value::Null,
                                Some((
                                    final_bytes,
                                    "application/octet-stream".to_string(),
                                    cr_opt,
                                    etag,
                                )),
                            );
                        }
                    }
                    return (
                        404,
                        "Not Found",
                        serde_json::json!({ "error": "NoSuchKey" }),
                        None,
                    );
                }
                "DELETE" => {
                    let file_path = get_storage_root().join(&bucket_id).join(&object_key);
                    let _ = fs::remove_file(file_path);
                    let esc_obj_id = obj_id.replace('\'', "''");
                    let sql = format!("DELETE FROM _storage_objects WHERE id = '{esc_obj_id}'");
                    let _ = db.execute_with_context(&sql, &ExecutionContext::admin());
                    return (204, "No Content", serde_json::Value::Null, None);
                }
                _ => {
                    return (
                        405,
                        "Method Not Allowed",
                        serde_json::json!({ "error": "MethodNotAllowed" }),
                        None,
                    )
                }
            }
        } else {
            let bucket_id = sanitize_object_path(s3_subpath);
            if method == "GET" {
                let sql = format!("SELECT name, size_bytes, created_at FROM _storage_objects WHERE bucket_id = '{bucket_id}'");
                let contents = match db.execute_with_context(&sql, &ExecutionContext::admin()) {
                    Ok(ExecResult::Rows { rows, .. }) => rows
                        .iter()
                        .map(|r| {
                            serde_json::json!({
                                "Key": match &r[0] { Value::Text(s) => s, _ => "" },
                                "Size": match &r[1] { Value::Integer(i) => *i, _ => 0 },
                                "LastModified": match &r[2] { Value::Integer(i) => *i, _ => 0 }
                            })
                        })
                        .collect::<Vec<_>>(),
                    _ => vec![],
                };
                return (
                    200,
                    "OK",
                    serde_json::json!({
                        "Name": bucket_id,
                        "KeyCount": contents.len(),
                        "Contents": contents
                    }),
                    None,
                );
            }
        }
    }

    let subpath = path.strip_prefix("/v1/storage/v1").unwrap_or(path);

    if subpath.starts_with("/object/sign/") && method == "POST" {
        let sign_path = &subpath["/object/sign/".len()..];
        if let Some((bucket_id, object_key)) = sign_path.split_once('/') {
            let bucket_id = sanitize_object_path(bucket_id);
            let object_key = sanitize_object_path(object_key);
            let obj_id = format!("{bucket_id}/{object_key}");

            // Authorization to sign URL: must be owner, admin, or in public bucket
            let bucket_sql =
                format!("SELECT public FROM _storage_buckets WHERE id = '{bucket_id}'");
            let is_public_bucket =
                match db.execute_with_context(&bucket_sql, &ExecutionContext::admin()) {
                    Ok(ExecResult::Rows { rows, .. }) if !rows.is_empty() => {
                        matches!(&rows[0][0], Value::Boolean(true))
                    }
                    _ => false,
                };

            let owner_opt = get_object_owner(db, &obj_id);
            let is_owner = match (owner_opt, ctx.user_id) {
                (Some(Some(owner_id)), Some(caller_id)) => owner_id == caller_id,
                _ => false,
            };

            if !is_public_bucket && !ctx.is_admin && !is_owner {
                return (
                    403,
                    "Forbidden",
                    serde_json::json!({ "error": "cannot generate signed URL for private object not owned by caller" }),
                    None,
                );
            }

            let payload: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
            let expires_in = payload
                .get("expiresIn")
                .and_then(|v| v.as_u64())
                .unwrap_or(3600);

            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let expires_at = now + expires_in;

            let secret = crate::auth::jwt_secret();
            let token = sign_download_token(&bucket_id, &object_key, expires_at, &secret);
            let signed_url = format!(
                "/v1/storage/v1/object/{bucket_id}/{object_key}?token={token}&expires={expires_at}"
            );

            return (
                200,
                "OK",
                serde_json::json!({
                    "signedURL": signed_url,
                    "token": token,
                    "expiresAt": expires_at
                }),
                None,
            );
        }
    }

    if subpath == "/bucket" || subpath == "/bucket/" {
        match method {
            "GET" => {
                let sql = "SELECT id, name, public, created_at FROM _storage_buckets ORDER BY created_at ASC";
                match db.execute_with_context(sql, ctx) {
                    Ok(ExecResult::Rows { rows, .. }) => {
                        let list: Vec<serde_json::Value> = rows
                            .iter()
                            .map(|r| {
                                serde_json::json!({
                                    "id": match &r[0] { Value::Text(s) => s, _ => "" },
                                    "name": match &r[1] { Value::Text(s) => s, _ => "" },
                                    "public": match &r[2] { Value::Boolean(b) => *b, _ => false },
                                    "created_at": match &r[3] { Value::Integer(i) => *i, _ => 0 },
                                })
                            })
                            .collect();
                        (200, "OK", serde_json::Value::Array(list), None)
                    }
                    _ => (200, "OK", serde_json::json!([]), None),
                }
            }
            "POST" => {
                let payload: serde_json::Value = match serde_json::from_str(body) {
                    Ok(v) => v,
                    Err(_) => {
                        return (
                            400,
                            "Bad Request",
                            serde_json::json!({ "error": "invalid JSON body" }),
                            None,
                        )
                    }
                };

                let id = match payload
                    .get("id")
                    .or_else(|| payload.get("name"))
                    .and_then(|v| v.as_str())
                {
                    Some(s) => sanitize_object_path(s),
                    None => {
                        return (
                            400,
                            "Bad Request",
                            serde_json::json!({ "error": "missing bucket id/name" }),
                            None,
                        )
                    }
                };
                let name = payload.get("name").and_then(|v| v.as_str()).unwrap_or(&id);
                let is_public = payload
                    .get("public")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                let sql = format!(
                    "INSERT INTO _storage_buckets (id, name, public, created_at) VALUES ('{id}', '{}', {}, {now})",
                    name.replace('\'', "''"),
                    if is_public { "TRUE" } else { "FALSE" }
                );

                match db.execute_with_context(&sql, ctx) {
                    Ok(_) => {
                        let root = get_storage_root().join(&id);
                        let _ = fs::create_dir_all(&root);
                        (
                            201,
                            "Created",
                            serde_json::json!({ "name": id, "message": "bucket created successfully" }),
                            None,
                        )
                    }
                    Err(e) => (
                        400,
                        "Bad Request",
                        serde_json::json!({ "error": e.to_string() }),
                        None,
                    ),
                }
            }
            _ => (
                405,
                "Method Not Allowed",
                serde_json::json!({ "error": "method not allowed" }),
                None,
            ),
        }
    } else if let Some(stripped) = subpath.strip_prefix("/bucket/") {
        if let Some((bucket_id, "lifecycle")) = stripped.split_once('/') {
            let bucket_id = sanitize_object_path(bucket_id);
            if !ctx.is_admin {
                return (
                    403,
                    "Forbidden",
                    serde_json::json!({ "error": "admin privileges required for lifecycle rules" }),
                    None,
                );
            }
            match method {
                "POST" => {
                    let payload: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
                    let prefix = payload
                        .get("prefix")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let expiry_days = payload
                        .get("expiry_days")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(30);
                    let rule_id = format!("{bucket_id}_{prefix}_{expiry_days}");
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();

                    let esc_id = rule_id.replace('\'', "''");
                    let esc_bucket = bucket_id.replace('\'', "''");
                    let esc_prefix = prefix.replace('\'', "''");
                    let insert_sql = format!(
                        "INSERT INTO _storage_lifecycle_rules (id, bucket_id, prefix, expiry_days, created_at) VALUES ('{esc_id}', '{esc_bucket}', '{esc_prefix}', {expiry_days}, {now})"
                    );
                    match db.execute_with_context(&insert_sql, &ExecutionContext::admin()) {
                        Ok(_) => (
                            201,
                            "Created",
                            serde_json::json!({
                                "id": rule_id,
                                "bucket_id": bucket_id,
                                "prefix": prefix,
                                "expiry_days": expiry_days
                            }),
                            None,
                        ),
                        Err(e) => (
                            400,
                            "Bad Request",
                            serde_json::json!({ "error": e.to_string() }),
                            None,
                        ),
                    }
                }
                "GET" => {
                    let esc_bucket = bucket_id.replace('\'', "''");
                    let sql = format!(
                        "SELECT id, prefix, expiry_days, created_at FROM _storage_lifecycle_rules WHERE bucket_id = '{esc_bucket}'"
                    );
                    let rules = match db.execute_with_context(&sql, &ExecutionContext::admin()) {
                        Ok(ExecResult::Rows { rows, .. }) => rows
                            .iter()
                            .map(|r| {
                                serde_json::json!({
                                    "id": match &r[0] { Value::Text(s) => s, _ => "" },
                                    "prefix": match &r[1] { Value::Text(s) => s, _ => "" },
                                    "expiry_days": match &r[2] { Value::Integer(i) => *i, _ => 0 },
                                    "created_at": match &r[3] { Value::Integer(i) => *i, _ => 0 }
                                })
                            })
                            .collect::<Vec<_>>(),
                        _ => vec![],
                    };
                    (200, "OK", serde_json::Value::Array(rules), None)
                }
                "DELETE" => {
                    let esc_bucket = bucket_id.replace('\'', "''");
                    let sql = format!(
                        "DELETE FROM _storage_lifecycle_rules WHERE bucket_id = '{esc_bucket}'"
                    );
                    let _ = db.execute_with_context(&sql, &ExecutionContext::admin());
                    (
                        200,
                        "OK",
                        serde_json::json!({ "message": "lifecycle rules cleared" }),
                        None,
                    )
                }
                _ => (
                    405,
                    "Method Not Allowed",
                    serde_json::json!({ "error": "method not allowed" }),
                    None,
                ),
            }
        } else {
            let bucket_id = sanitize_object_path(stripped);
            match method {
                "GET" => {
                    let sql = format!("SELECT id, name, public, created_at FROM _storage_buckets WHERE id = '{bucket_id}'");
                    match db.execute_with_context(&sql, ctx) {
                        Ok(ExecResult::Rows { rows, .. }) if !rows.is_empty() => {
                            let r = &rows[0];
                            (
                                200,
                                "OK",
                                serde_json::json!({
                                    "id": match &r[0] { Value::Text(s) => s, _ => "" },
                                    "name": match &r[1] { Value::Text(s) => s, _ => "" },
                                    "public": match &r[2] { Value::Boolean(b) => *b, _ => false },
                                    "created_at": match &r[3] { Value::Integer(i) => *i, _ => 0 },
                                }),
                                None,
                            )
                        }
                        _ => (
                            404,
                            "Not Found",
                            serde_json::json!({ "error": "bucket not found" }),
                            None,
                        ),
                    }
                }
                "DELETE" => {
                    let sql = format!("DELETE FROM _storage_buckets WHERE id = '{bucket_id}'");
                    let _ = db.execute_with_context(&sql, ctx);
                    let root = get_storage_root().join(&bucket_id);
                    let _ = fs::remove_dir_all(&root);
                    (
                        200,
                        "OK",
                        serde_json::json!({ "message": "bucket deleted" }),
                        None,
                    )
                }
                _ => (
                    405,
                    "Method Not Allowed",
                    serde_json::json!({ "error": "method not allowed" }),
                    None,
                ),
            }
        }
    } else if let Some(stripped) = subpath.strip_prefix("/object/list/") {
        let bucket_id = sanitize_object_path(stripped);

        let bucket_sql = format!("SELECT public FROM _storage_buckets WHERE id = '{bucket_id}'");
        let is_public_bucket =
            match db.execute_with_context(&bucket_sql, &ExecutionContext::admin()) {
                Ok(ExecResult::Rows { rows, .. }) if !rows.is_empty() => {
                    matches!(&rows[0][0], Value::Boolean(true))
                }
                _ => false,
            };

        if !is_public_bucket && !ctx.is_authenticated() && !ctx.is_admin {
            return (
                401,
                "Unauthorized",
                serde_json::json!({ "error": "access denied to private bucket" }),
                None,
            );
        }

        let sql = if ctx.is_admin || is_public_bucket {
            format!("SELECT id, name, owner_id, size_bytes, created_at FROM _storage_objects WHERE bucket_id = '{bucket_id}'")
        } else {
            let caller_id = ctx.user_id.unwrap_or(0);
            format!("SELECT id, name, owner_id, size_bytes, created_at FROM _storage_objects WHERE bucket_id = '{bucket_id}' AND owner_id = {caller_id}")
        };

        match db.execute_with_context(&sql, &ExecutionContext::admin()) {
            Ok(ExecResult::Rows { rows, .. }) => {
                let list: Vec<serde_json::Value> = rows
                    .iter()
                    .map(|r| {
                        serde_json::json!({
                            "name": match &r[1] { Value::Text(s) => s, _ => "" },
                            "id": match &r[0] { Value::Text(s) => s, _ => "" },
                            "owner_id": match &r[2] { Value::Integer(i) => serde_json::Value::Number((*i).into()), _ => serde_json::Value::Null },
                            "metadata": {
                                "size": match &r[3] { Value::Integer(i) => *i, _ => 0 }
                            },
                            "created_at": match &r[4] { Value::Integer(i) => *i, _ => 0 },
                        })
                    })
                    .collect();
                (200, "OK", serde_json::Value::Array(list), None)
            }
            _ => (200, "OK", serde_json::json!([]), None),
        }
    } else if let Some(obj_subpath) = subpath.strip_prefix("/object/") {
        let clean_subpath = obj_subpath.strip_prefix("public/").unwrap_or(obj_subpath);

        if let Some((bucket_id, object_key)) = clean_subpath.split_once('/') {
            let bucket_id = sanitize_object_path(bucket_id);
            let object_key = sanitize_object_path(object_key);
            let obj_id = format!("{bucket_id}/{object_key}");

            match method {
                "GET" => {
                    // Check query params for signed token
                    let mut token_opt = None;
                    let mut expires_opt = None;
                    for pair in query_str.split('&') {
                        if let Some((k, v)) = pair.split_once('=') {
                            if k == "token" {
                                token_opt = Some(v);
                            } else if k == "expires" {
                                expires_opt = v.parse::<u64>().ok();
                            }
                        }
                    }

                    let is_valid_signed_request =
                        if let (Some(token), Some(expires_at)) = (token_opt, expires_opt) {
                            let secret = crate::auth::jwt_secret();
                            verify_download_signature(
                                &bucket_id,
                                &object_key,
                                expires_at,
                                token,
                                &secret,
                            )
                        } else {
                            false
                        };

                    // Check if bucket is public
                    let bucket_sql =
                        format!("SELECT public FROM _storage_buckets WHERE id = '{bucket_id}'");
                    let is_public_bucket =
                        match db.execute_with_context(&bucket_sql, &ExecutionContext::admin()) {
                            Ok(ExecResult::Rows { rows, .. }) if !rows.is_empty() => {
                                matches!(&rows[0][0], Value::Boolean(true))
                            }
                            _ => false,
                        };

                    let owner_opt = get_object_owner(db, &obj_id);
                    let is_owner = match (owner_opt, ctx.user_id) {
                        (Some(Some(owner_id)), Some(caller_id)) => owner_id == caller_id,
                        _ => false,
                    };

                    if !is_public_bucket && !ctx.is_admin && !is_valid_signed_request && !is_owner {
                        if !ctx.is_authenticated() {
                            return (
                                401,
                                "Unauthorized",
                                serde_json::json!({ "error": "access denied to private object" }),
                                None,
                            );
                        } else {
                            return (
                                403,
                                "Forbidden",
                                serde_json::json!({ "error": "access denied to private object owned by another user" }),
                                None,
                            );
                        }
                    }

                    let file_path = get_storage_root().join(&bucket_id).join(&object_key);
                    if file_path.exists() {
                        if let Ok(bytes) = fs::read(&file_path) {
                            let content_type = if object_key.ends_with(".png") {
                                "image/png"
                            } else if object_key.ends_with(".jpg") || object_key.ends_with(".jpeg")
                            {
                                "image/jpeg"
                            } else if object_key.ends_with(".json") {
                                "application/json"
                            } else if object_key.ends_with(".txt") {
                                "text/plain"
                            } else {
                                "application/octet-stream"
                            };

                            let mut hasher = Sha256::new();
                            hasher.update(&bytes);
                            let etag = format!("\"{:x}\"", hasher.finalize());

                            let (status_code, status_text, final_bytes, cr_opt) =
                                if let Some(range_str) = range_header {
                                    if let Some(range_val) = range_str.strip_prefix("bytes=") {
                                        let total_len = bytes.len();
                                        let parts: Vec<&str> = range_val.split('-').collect();
                                        let start = parts[0].parse::<usize>().unwrap_or(0);
                                        let end = if parts.len() > 1 && !parts[1].is_empty() {
                                            parts[1]
                                                .parse::<usize>()
                                                .unwrap_or(total_len.saturating_sub(1))
                                                .min(total_len.saturating_sub(1))
                                        } else {
                                            total_len.saturating_sub(1)
                                        };
                                        if start < total_len && start <= end {
                                            let slice = bytes[start..=end].to_vec();
                                            let cr = format!("bytes {start}-{end}/{total_len}");
                                            (206, "Partial Content", slice, Some(cr))
                                        } else {
                                            (200, "OK", bytes, None)
                                        }
                                    } else {
                                        (200, "OK", bytes, None)
                                    }
                                } else {
                                    (200, "OK", bytes, None)
                                };

                            return (
                                status_code,
                                status_text,
                                serde_json::Value::Null,
                                Some((final_bytes, content_type.to_string(), cr_opt, etag)),
                            );
                        }
                    }
                    (
                        404,
                        "Not Found",
                        serde_json::json!({ "error": "object not found" }),
                        None,
                    )
                }
                "POST" => {
                    // Check bucket existence and privacy
                    let bucket_sql =
                        format!("SELECT public FROM _storage_buckets WHERE id = '{bucket_id}'");
                    let is_public_bucket =
                        match db.execute_with_context(&bucket_sql, &ExecutionContext::admin()) {
                            Ok(ExecResult::Rows { rows, .. }) if !rows.is_empty() => {
                                matches!(&rows[0][0], Value::Boolean(true))
                            }
                            _ => false,
                        };

                    if !is_public_bucket && !ctx.is_authenticated() && !ctx.is_admin {
                        return (
                            401,
                            "Unauthorized",
                            serde_json::json!({ "error": "authentication required to upload to private bucket" }),
                            None,
                        );
                    }

                    // If object already exists, check ownership
                    let owner_opt = get_object_owner(db, &obj_id);
                    if let Some(Some(existing_owner)) = owner_opt {
                        if !ctx.is_admin && ctx.user_id != Some(existing_owner) {
                            return (
                                403,
                                "Forbidden",
                                serde_json::json!({ "error": "cannot overwrite object owned by another user" }),
                                None,
                            );
                        }
                    }

                    let file_path = get_storage_root().join(&bucket_id).join(&object_key);
                    if let Some(parent) = file_path.parent() {
                        let _ = fs::create_dir_all(parent);
                    }

                    let bytes = body.as_bytes();
                    if fs::write(&file_path, bytes).is_err() {
                        return (
                            500,
                            "Internal Server Error",
                            serde_json::json!({ "error": "failed to write object to disk" }),
                            None,
                        );
                    }

                    let size_bytes = bytes.len() as i64;
                    let mut hasher = Sha256::new();
                    hasher.update(bytes);
                    let etag_hex = format!("{:x}", hasher.finalize());

                    let owner_id = ctx
                        .user_id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "NULL".to_string());
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();

                    let esc_obj_id = obj_id.replace('\'', "''");
                    let esc_bucket_id = bucket_id.replace('\'', "''");
                    let esc_obj_key = object_key.replace('\'', "''");
                    let insert_sql = format!(
                        "INSERT INTO _storage_objects (id, bucket_id, name, owner_id, content_type, size_bytes, metadata, created_at, updated_at) VALUES ('{esc_obj_id}', '{esc_bucket_id}', '{esc_obj_key}', {owner_id}, 'application/octet-stream', {size_bytes}, '{{}}', {now}, {now})"
                    );
                    let _ = db.execute_with_context(&insert_sql, &ExecutionContext::admin());

                    (
                        200,
                        "OK",
                        serde_json::json!({
                            "Key": format!("{bucket_id}/{object_key}"),
                            "Id": obj_id,
                            "size": size_bytes,
                            "etag": etag_hex,
                            "checksum_sha256": etag_hex
                        }),
                        None,
                    )
                }
                "DELETE" => {
                    let bucket_sql =
                        format!("SELECT public FROM _storage_buckets WHERE id = '{bucket_id}'");
                    let is_public_bucket =
                        match db.execute_with_context(&bucket_sql, &ExecutionContext::admin()) {
                            Ok(ExecResult::Rows { rows, .. }) if !rows.is_empty() => {
                                matches!(&rows[0][0], Value::Boolean(true))
                            }
                            _ => false,
                        };

                    let owner_opt = get_object_owner(db, &obj_id);
                    let is_owner = match (owner_opt, ctx.user_id) {
                        (Some(Some(owner_id)), Some(caller_id)) => owner_id == caller_id,
                        (Some(None), _) if is_public_bucket => true,
                        _ => false,
                    };

                    if !ctx.is_admin && !is_owner {
                        return (
                            403,
                            "Forbidden",
                            serde_json::json!({ "error": "cannot delete object owned by another user" }),
                            None,
                        );
                    }

                    let file_path = get_storage_root().join(&bucket_id).join(&object_key);
                    let _ = fs::remove_file(file_path);
                    let esc_obj_id = obj_id.replace('\'', "''");
                    let sql = format!("DELETE FROM _storage_objects WHERE id = '{esc_obj_id}'");
                    let _ = db.execute_with_context(&sql, &ExecutionContext::admin());
                    (
                        200,
                        "OK",
                        serde_json::json!({ "message": "object deleted" }),
                        None,
                    )
                }
                _ => (
                    405,
                    "Method Not Allowed",
                    serde_json::json!({ "error": "method not allowed" }),
                    None,
                ),
            }
        } else {
            (
                400,
                "Bad Request",
                serde_json::json!({ "error": "missing bucket or object key" }),
                None,
            )
        }
    } else {
        (
            404,
            "Not Found",
            serde_json::json!({ "error": "storage endpoint not found" }),
            None,
        )
    }
}

pub fn cleanup_expired_objects(db: &SharedDatabase) -> usize {
    ensure_storage_tables(db);
    let rules_sql = "SELECT bucket_id, prefix, expiry_days FROM _storage_lifecycle_rules";
    let rules = match db.execute_with_context(rules_sql, &ExecutionContext::admin()) {
        Ok(ExecResult::Rows { rows, .. }) => rows,
        _ => return 0,
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut deleted_count = 0;

    for rule in rules {
        let bucket_id = match &rule[0] {
            Value::Text(s) => s.as_str(),
            _ => continue,
        };
        let prefix = match &rule[1] {
            Value::Text(s) => s.as_str(),
            _ => "",
        };
        let expiry_days = match &rule[2] {
            Value::Integer(i) => *i as u64,
            _ => continue,
        };

        let cutoff = now.saturating_sub(expiry_days * 86400);

        let esc_bucket = bucket_id.replace('\'', "''");
        let obj_sql = format!(
            "SELECT id, name FROM _storage_objects WHERE bucket_id = '{esc_bucket}' AND created_at < {cutoff}"
        );

        if let Ok(ExecResult::Rows { rows, .. }) =
            db.execute_with_context(&obj_sql, &ExecutionContext::admin())
        {
            for obj in rows {
                let obj_id = match &obj[0] {
                    Value::Text(s) => s.as_str(),
                    _ => continue,
                };
                let name = match &obj[1] {
                    Value::Text(s) => s.as_str(),
                    _ => continue,
                };

                if name.starts_with(prefix) {
                    let file_path = get_storage_root().join(bucket_id).join(name);
                    let _ = fs::remove_file(file_path);
                    let esc_obj_id = obj_id.replace('\'', "''");
                    let del_sql = format!("DELETE FROM _storage_objects WHERE id = '{esc_obj_id}'");
                    let _ = db.execute_with_context(&del_sql, &ExecutionContext::admin());
                    deleted_count += 1;
                }
            }
        }
    }

    deleted_count
}
