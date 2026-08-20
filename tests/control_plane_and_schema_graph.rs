//! Automated integration tests for Control Plane, Multi-Tenant Fleet Provisioning,
//! Schema Relationship Graph, and Dynamic Storage Image Transformations.

use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use dbengine::control_plane::{ControlPlane, ProjectStatus};
use dbengine::engine::SharedDatabase;
use dbengine::http::HttpServer;

async fn send_http_req(
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: &str,
    auth_header: &str,
) -> (u16, serde_json::Value) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let auth = if !auth_header.is_empty() {
        format!("Authorization: {auth_header}\r\n")
    } else {
        String::new()
    };

    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );

    stream.write_all(req.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();

    let resp_str = String::from_utf8_lossy(&buf);
    let status_code: u16 = resp_str
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();

    let body_idx = resp_str.find("\r\n\r\n").unwrap() + 4;
    let json_body: serde_json::Value =
        serde_json::from_str(&resp_str[body_idx..]).unwrap_or(serde_json::Value::Null);

    (status_code, json_body)
}

#[tokio::test]
async fn test_control_plane_fleet_and_tenant_lifecycle() {
    let cp = ControlPlane::global();

    // 1. Create Organization
    let org = cp.create_organization("Acme Corp");
    assert!(org.id.starts_with("org_"));
    assert_eq!(org.name, "Acme Corp");

    // 2. Create Projects under Organization
    let project = cp.create_project(&org.id, "Staging App", "eu-central-1").unwrap();
    assert!(project.id.starts_with("prj_"));
    assert_eq!(project.name, "Staging App");
    assert_eq!(project.status, ProjectStatus::Active);
    assert_eq!(project.quota.max_storage_mb, 1024);

    // 3. Pause & Resume Project
    let paused = cp.pause_project(&project.id).unwrap();
    assert_eq!(paused.status, ProjectStatus::Paused);

    let resumed = cp.resume_project(&project.id).unwrap();
    assert_eq!(resumed.status, ProjectStatus::Active);

    // 4. Record Usage
    cp.record_egress(&project.id, 50000);
    let updated = cp.get_project(&project.id).unwrap();
    assert_eq!(updated.usage.egress_bytes, 50000);
}

#[tokio::test]
async fn test_http_control_plane_and_schema_endpoints() {
    let file = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(file.path()).unwrap();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (_server, bound_addr) = HttpServer::bind(addr, db.clone()).await.unwrap();

    // Create tables with foreign key relation
    let admin = dbengine::auth::ExecutionContext::admin();
    db.execute_with_context(
        "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT NOT NULL);",
        &admin,
    )
    .unwrap();
    db.execute_with_context(
        "CREATE TABLE posts (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL, title TEXT NOT NULL);",
        &admin,
    )
    .unwrap();

    // 1. Test Schema Relationships Discovery
    let (code, schema_res) = send_http_req(
        bound_addr,
        "GET",
        "/v1/schema/relationships",
        "",
        "",
    )
    .await;
    assert_eq!(code, 200);
    let rels = schema_res["relationships"].as_array().unwrap();
    assert!(!rels.is_empty());
    let post_rel = rels.iter().find(|r| r["source_table"] == "posts").unwrap();
    assert_eq!(post_rel["source_column"], "user_id");
    assert_eq!(post_rel["target_table"], "users");

    // 2. Test Control Plane Project API (Admin authentication)
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let secret = dbengine::auth::jwt_secret();
    let claims = dbengine::auth::SessionClaims::new(1, "admin@chocobase.internal", "service_role", now + 3600);
    let admin_jwt = dbengine::auth::sign_jwt(&claims, &secret);
    let (code, prj_list) = send_http_req(
        bound_addr,
        "GET",
        "/v1/admin/projects",
        "",
        &format!("Bearer {admin_jwt}"),
    )
    .await;
    assert_eq!(code, 200);
    assert!(!prj_list["projects"].as_array().unwrap().is_empty());

    // 3. Test Dynamic Image Transformation API
    let (code, img_res) = send_http_req(
        bound_addr,
        "GET",
        "/v1/storage/v1/render/image/avatars/user_123.png?width=200&height=200",
        "",
        "",
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(img_res["status"], "transformed");
    assert_eq!(img_res["transform"]["format"], "webp");
}
