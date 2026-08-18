use dbengine::auth::{
    hash_password, sign_jwt, verify_jwt, verify_password, ExecutionContext, SessionClaims,
};
use dbengine::engine::{Database, ExecResult};
use dbengine::types::value::Value;
use tempfile::NamedTempFile;

#[test]
fn test_password_hashing_and_verification() {
    let password = "my_secure_password_!@#";
    let hash = hash_password(password);
    assert_ne!(hash, password);
    assert!(verify_password(password, &hash));
    assert!(!verify_password("wrong_password", &hash));
}

#[test]
fn test_jwt_token_signing_and_verification() {
    let secret = b"production-jwt-secret-key-123456";
    let claims = SessionClaims::new(42, "alice", "developer", 1893456000);

    let token = sign_jwt(&claims, secret);
    let verified = verify_jwt(&token, secret).expect("verification should succeed");
    assert_eq!(verified.user_id(), 42);
    assert_eq!(verified.username, "alice");
    assert_eq!(verified.role, "developer");

    // Invalid secret
    assert!(verify_jwt(&token, b"wrong-secret").is_err());
}

#[test]
fn test_user_creation_in_database() {
    let temp_file = NamedTempFile::new().unwrap();
    let mut db = Database::create(temp_file.path()).unwrap();

    let res = db
        .execute("CREATE USER alice WITH PASSWORD 'password123' ROLE 'member'")
        .unwrap();
    assert_eq!(res, ExecResult::Modified(1));

    // Duplicate username fails
    let dup_res = db.execute("CREATE USER alice WITH PASSWORD 'diffpwd'");
    assert!(dup_res.is_err());

    // Second user succeeds
    let res2 = db
        .execute("CREATE USER bob WITH PASSWORD 'secret'")
        .unwrap();
    assert_eq!(res2, ExecResult::Modified(1));

    // Verify _users table query
    let rows = db
        .execute("SELECT id, username, role FROM _users ORDER BY id ASC")
        .unwrap();
    if let ExecResult::Rows { rows, .. } = rows {
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0][1], Value::Text("postgres".into()));
        assert_eq!(rows[0][2], Value::Text("admin".into()));
        assert_eq!(rows[1][1], Value::Text("alice".into()));
        assert_eq!(rows[1][2], Value::Text("member".into()));
        assert_eq!(rows[2][1], Value::Text("bob".into()));
        assert_eq!(rows[2][2], Value::Text("user".into()));
    } else {
        panic!("expected rows");
    }
}

#[test]
fn test_row_level_security_multi_tenant_isolation() {
    let temp_file = NamedTempFile::new().unwrap();
    let mut db = Database::create(temp_file.path()).unwrap();

    // 1. Create table and enable RLS
    db.execute("CREATE TABLE notes (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL, content TEXT NOT NULL)").unwrap();
    db.execute("ALTER TABLE notes ENABLE ROW LEVEL SECURITY")
        .unwrap();

    // 2. Create policy using auth.uid()
    db.execute("CREATE POLICY user_isolation ON notes FOR ALL USING (user_id = auth.uid()) WITH CHECK (user_id = auth.uid())").unwrap();

    let claims_alice = SessionClaims::new(1, "alice", "user", 0);
    let ctx_alice = ExecutionContext::from_claims(&claims_alice);

    let claims_bob = SessionClaims::new(2, "bob", "user", 0);
    let ctx_bob = ExecutionContext::from_claims(&claims_bob);

    let ctx_anon = ExecutionContext::anonymous();
    let ctx_admin = ExecutionContext::admin();

    // 3. Alice inserts her own note -> SUCCESS
    let res = db.execute_with_context(
        "INSERT INTO notes (id, user_id, content) VALUES (1, 1, 'Alice secret note')",
        &ctx_alice,
    );
    assert!(res.is_ok());

    // 4. Alice tries to insert note for Bob (user_id = 2) -> REJECTED by WITH CHECK
    let bad_insert = db.execute_with_context(
        "INSERT INTO notes (id, user_id, content) VALUES (2, 2, 'Spoofed note by Alice')",
        &ctx_alice,
    );
    assert!(bad_insert.is_err());

    // 5. Bob inserts his own note -> SUCCESS
    let res_bob = db.execute_with_context(
        "INSERT INTO notes (id, user_id, content) VALUES (2, 2, 'Bob secret note')",
        &ctx_bob,
    );
    assert!(res_bob.is_ok());

    // 6. Alice queries notes -> sees ONLY Alice's note
    let alice_notes = db
        .execute_with_context("SELECT id, content FROM notes", &ctx_alice)
        .unwrap();
    if let ExecResult::Rows { rows, .. } = alice_notes {
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0], Value::Integer(1));
        assert_eq!(rows[0][1], Value::Text("Alice secret note".into()));
    } else {
        panic!("expected rows");
    }

    // 7. Bob queries notes -> sees ONLY Bob's note
    let bob_notes = db
        .execute_with_context("SELECT id, content FROM notes", &ctx_bob)
        .unwrap();
    if let ExecResult::Rows { rows, .. } = bob_notes {
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0], Value::Integer(2));
        assert_eq!(rows[0][1], Value::Text("Bob secret note".into()));
    } else {
        panic!("expected rows");
    }

    // 8. Anonymous queries notes -> default deny returns 0 rows
    let anon_notes = db
        .execute_with_context("SELECT id, content FROM notes", &ctx_anon)
        .unwrap();
    if let ExecResult::Rows { rows, .. } = anon_notes {
        assert_eq!(rows.len(), 0);
    } else {
        panic!("expected rows");
    }

    // 9. Admin queries notes -> bypasses RLS, sees ALL notes
    let admin_notes = db
        .execute_with_context("SELECT id, content FROM notes ORDER BY id ASC", &ctx_admin)
        .unwrap();
    if let ExecResult::Rows { rows, .. } = admin_notes {
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0], Value::Integer(1));
        assert_eq!(rows[1][0], Value::Integer(2));
    } else {
        panic!("expected rows");
    }

    // 10. Alice attempts to update Bob's note -> 0 rows modified
    let update_res = db
        .execute_with_context(
            "UPDATE notes SET content = 'hacked' WHERE id = 2",
            &ctx_alice,
        )
        .unwrap();
    assert_eq!(update_res, ExecResult::Modified(0));

    // 11. Alice updates her own note -> 1 row modified
    let update_own = db
        .execute_with_context(
            "UPDATE notes SET content = 'Alice updated' WHERE id = 1",
            &ctx_alice,
        )
        .unwrap();
    assert_eq!(update_own, ExecResult::Modified(1));

    // 12. Alice attempts to delete Bob's note -> 0 rows modified
    let delete_res = db
        .execute_with_context("DELETE FROM notes WHERE id = 2", &ctx_alice)
        .unwrap();
    assert_eq!(delete_res, ExecResult::Modified(0));

    // 13. Alice deletes her own note -> 1 row modified
    let delete_own = db
        .execute_with_context("DELETE FROM notes WHERE id = 1", &ctx_alice)
        .unwrap();
    assert_eq!(delete_own, ExecResult::Modified(1));
}
