use std::net::SocketAddr;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use dbengine::auth::{
    issue_refresh_token, rotate_active_key, rotate_refresh_token, sign_jwt, verify_jwt,
    SessionClaims, DEFAULT_DEV_JWT_SECRET,
};
use dbengine::{HttpServer, SharedDatabase};

async fn send_http_post(addr: SocketAddr, path: &str, body: &str) -> (u16, serde_json::Value) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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

#[test]
fn test_rfc7519_jwt_validation_and_security_checks() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // 1. Valid token
    let valid_claims = SessionClaims::new(42, "tester", "user", now + 3600);
    let token = sign_jwt(&valid_claims, DEFAULT_DEV_JWT_SECRET);
    let verified = verify_jwt(&token, DEFAULT_DEV_JWT_SECRET).unwrap();
    assert_eq!(verified.sub, 42);
    assert_eq!(verified.username, "tester");

    // 2. Expired token
    let expired_claims = SessionClaims {
        iss: "chocobase-auth".into(),
        aud: "authenticated".into(),
        sub: 42,
        username: "tester".into(),
        role: "user".into(),
        exp: now - 100,
        iat: now - 200,
        nbf: now - 200,
        jti: None,
    };
    let expired_token = sign_jwt(&expired_claims, DEFAULT_DEV_JWT_SECRET);
    assert!(verify_jwt(&expired_token, DEFAULT_DEV_JWT_SECRET).is_err());

    // 3. nbf in future
    let nbf_future_claims = SessionClaims {
        iss: "chocobase-auth".into(),
        aud: "authenticated".into(),
        sub: 42,
        username: "tester".into(),
        role: "user".into(),
        exp: now + 3600,
        iat: now,
        nbf: now + 1000,
        jti: None,
    };
    let nbf_token = sign_jwt(&nbf_future_claims, DEFAULT_DEV_JWT_SECRET);
    assert!(verify_jwt(&nbf_token, DEFAULT_DEV_JWT_SECRET).is_err());

    // 4. Invalid signature / tampering
    let mut tampered_token = token.clone();
    tampered_token.push_str("tampered");
    assert!(verify_jwt(&tampered_token, DEFAULT_DEV_JWT_SECRET).is_err());

    // 5. Wrong issuer / audience
    let mut bad_iss = valid_claims.clone();
    bad_iss.iss = "evil-issuer".into();
    let bad_iss_token = sign_jwt(&bad_iss, DEFAULT_DEV_JWT_SECRET);
    assert!(verify_jwt(&bad_iss_token, DEFAULT_DEV_JWT_SECRET).is_err());
}

#[test]
fn test_jwt_key_rotation() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let claims = SessionClaims::new(100, "rotator", "admin", now + 3600);
    let initial_secret = DEFAULT_DEV_JWT_SECRET;
    let old_token = sign_jwt(&claims, initial_secret);

    // Rotate active signing key to k2
    let new_secret = b"chocobase-new-rotated-secret-key-32b!".to_vec();
    rotate_active_key("k2", new_secret.clone());

    // Old token should still be valid because KeyStore retains previous keys
    let verified_old = verify_jwt(&old_token, initial_secret).unwrap();
    assert_eq!(verified_old.sub, 100);

    // Newly signed tokens use the new key k2
    let new_claims = SessionClaims::new(101, "new_user", "user", now + 3600);
    let new_token = sign_jwt(&new_claims, &new_secret);
    let verified_new = verify_jwt(&new_token, &new_secret).unwrap();
    assert_eq!(verified_new.sub, 101);
}

#[test]
fn test_refresh_token_rotation_and_reuse_detection() {
    // 1. Issue initial token
    let (t1, _family) = issue_refresh_token(500, "dev", "user");
    assert!(t1.starts_with("rt_"));

    // 2. Rotate t1 -> get t2
    let (claims2, t2) = rotate_refresh_token(&t1).unwrap();
    assert_eq!(claims2.sub, 500);
    assert_ne!(t1, t2);

    // 3. Rotate t2 -> get t3
    let (claims3, t3) = rotate_refresh_token(&t2).unwrap();
    assert_eq!(claims3.sub, 500);
    assert_ne!(t2, t3);

    // 4. REUSE DETECTION: Attacker tries to replay t1 (already rotated)
    let reuse_err = rotate_refresh_token(&t1);
    assert!(reuse_err.is_err());
    assert!(reuse_err
        .unwrap_err()
        .to_string()
        .contains("reuse detected"));

    // 5. Because family was invalidated, legitimate t3 is now also revoked
    let t3_revoked_err = rotate_refresh_token(&t3);
    assert!(t3_revoked_err.is_err());
}

#[tokio::test]
async fn test_http_auth_refresh_and_logout_endpoints() {
    let file = NamedTempFile::new().unwrap();
    let db = SharedDatabase::create(file.path()).unwrap();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (_server, bound_addr) = HttpServer::bind(addr, db).await.unwrap();

    // 1. Sign up
    let signup_body = r#"{"username": "grace", "password": "grace_password_123"}"#;
    let (code, signup_res) = send_http_post(bound_addr, "/v1/auth/signup", signup_body).await;
    assert_eq!(code, 201);
    let rt1 = signup_res["refresh_token"].as_str().unwrap().to_string();

    // 2. Refresh token
    let refresh_body = format!(r#"{{"refresh_token": "{rt1}"}}"#);
    let (code, refresh_res) = send_http_post(bound_addr, "/v1/auth/refresh", &refresh_body).await;
    assert_eq!(code, 200);
    let rt2 = refresh_res["refresh_token"].as_str().unwrap().to_string();
    assert_ne!(rt1, rt2);

    // 3. Logout
    let logout_body = format!(r#"{{"refresh_token": "{rt2}"}}"#);
    let (code, logout_res) = send_http_post(bound_addr, "/v1/auth/logout", &logout_body).await;
    assert_eq!(code, 200);
    assert_eq!(logout_res["status"], "logged_out");

    // 4. Refresh after logout fails
    let (code, _) = send_http_post(bound_addr, "/v1/auth/refresh", &logout_body).await;
    assert_eq!(code, 401);
}
