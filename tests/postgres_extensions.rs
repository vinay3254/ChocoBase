//! Automated tests for PostgreSQL native extensions:
//! pgcrypto, pgjwt, uuid-ossp, pgvector, pg_stat_statements, and pg_cron.

use dbengine::extensions::{
    get_registry, PgCrypto, PgJwt, PgStatStatements, PgVector, UuidOssp,
};

#[test]
fn test_pgcrypto_digest_and_hmac() {
    let message = b"Hello, PostgreSQL!";
    let sha256_digest = PgCrypto::digest(message, "sha256").unwrap();
    assert_eq!(sha256_digest.len(), 32);

    let sha512_digest = PgCrypto::digest(message, "sha512").unwrap();
    assert_eq!(sha512_digest.len(), 64);

    let key = b"secret-key-for-hmac";
    let hmac_res = PgCrypto::hmac(message, key, "sha256").unwrap();
    assert_eq!(hmac_res.len(), 32);

    let salt = PgCrypto::gen_salt("bf");
    assert!(salt.starts_with("bf_"));
    let hash = PgCrypto::crypt("super-secure-pass", &salt);
    assert!(hash.starts_with("$2a$10$"));
}

#[test]
fn test_pgjwt_sign_and_verify() {
    let payload = serde_json::json!({
        "sub": "1234567890",
        "name": "Vinay Kumar",
        "role": "authenticated",
        "iat": 1516239022
    });
    let secret = "chocobase-jwt-secret-key-32-chars-minimum";
    let token = PgJwt::sign(&payload, secret, "HS256").unwrap();
    assert!(token.contains('.'));

    let claims = PgJwt::verify(&token, secret, "HS256").unwrap();
    assert_eq!(claims["name"], "Vinay Kumar");
    assert_eq!(claims["role"], "authenticated");

    // Invalid secret verification fails
    assert!(PgJwt::verify(&token, "wrong-secret", "HS256").is_err());
}

#[test]
fn test_uuid_ossp_generation() {
    let v4 = UuidOssp::uuid_generate_v4();
    assert_eq!(v4.len(), 36);
    assert_eq!(&v4[14..15], "4"); // UUID v4 format

    let v1 = UuidOssp::uuid_generate_v1();
    assert_eq!(v1.len(), 36);

    let nil = UuidOssp::uuid_nil();
    assert_eq!(nil, "00000000-0000-0000-0000-000000000000");
}

#[test]
fn test_pgvector_similarity_operations() {
    let v1 = vec![1.0f32, 0.0, 0.0];
    let v2 = vec![0.0f32, 1.0, 0.0];
    let v3 = vec![1.0f32, 0.0, 0.0];

    // Cosine distance
    let dist_ortho = PgVector::cosine_distance(&v1, &v2).unwrap();
    assert!((dist_ortho - 1.0).abs() < 1e-4);

    let dist_identical = PgVector::cosine_distance(&v1, &v3).unwrap();
    assert!(dist_identical.abs() < 1e-4);

    // L2 distance
    let l2 = PgVector::l2_distance(&v1, &v2).unwrap();
    assert!((l2 - std::f32::consts::SQRT_2).abs() < 1e-4);

    // Inner product
    let ip = PgVector::inner_product(&v1, &v3).unwrap();
    assert_eq!(ip, -1.0);
}

#[test]
fn test_pg_stat_statements_profiling() {
    let tracker = PgStatStatements::global();
    tracker.reset();

    tracker.record("SELECT * FROM profiles WHERE id = 1", 2.5, 1);
    tracker.record("SELECT * FROM profiles WHERE id = 1", 3.5, 1);
    tracker.record("SELECT * FROM posts LIMIT 10", 12.0, 10);

    let stats = tracker.get_all();
    assert_eq!(stats.len(), 2);

    let profiles_stat = stats
        .iter()
        .find(|s| s.query.contains("profiles"))
        .expect("profiles query recorded");
    assert_eq!(profiles_stat.calls, 2);
    assert_eq!(profiles_stat.total_exec_time_ms, 6.0);
    assert_eq!(profiles_stat.mean_exec_time_ms, 3.0);
    assert_eq!(profiles_stat.rows_affected, 2);
}

#[test]
fn test_extension_catalog_registry() {
    let reg = get_registry().lock().unwrap();
    assert!(reg.contains_key("pgcrypto"));
    assert!(reg.contains_key("pgjwt"));
    assert!(reg.contains_key("uuid-ossp"));
    assert!(reg.contains_key("pgvector"));
    assert!(reg.contains_key("pg_stat_statements"));
    assert!(reg.contains_key("pg_cron"));
}
