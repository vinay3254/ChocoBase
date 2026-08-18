//! Authentication, password hashing, standard RFC 7519 JWT management,
//! and secure opaque refresh token family lifecycle for ChocoBase.

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};
use subtle::ConstantTimeEq;

use crate::error::{DbError, Result};

pub const DEFAULT_JWT_ISSUER: &str = "chocobase-auth";
pub const DEFAULT_JWT_AUDIENCE: &str = "authenticated";
pub const DEFAULT_DEV_JWT_SECRET: &[u8] = b"chocobase-development-secret-key-32b!";

type HmacSha256 = Hmac<Sha256>;

/// In-memory Key Store supporting key rotation with Key IDs (kid).
pub struct KeyStore {
    active_kid: String,
    keys: HashMap<String, Vec<u8>>,
}

impl Default for KeyStore {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyStore {
    pub fn new() -> Self {
        let mut keys = HashMap::new();
        let active_kid = "k1".to_string();

        let secret = if let Ok(s) = std::env::var("CHOCOBASE_JWT_SECRET") {
            if !s.is_empty() {
                s.into_bytes()
            } else {
                DEFAULT_DEV_JWT_SECRET.to_vec()
            }
        } else if std::env::var("CHOCOBASE_ENV").unwrap_or_default() == "production" {
            panic!("CHOCOBASE_JWT_SECRET must be set in production mode!");
        } else {
            DEFAULT_DEV_JWT_SECRET.to_vec()
        };

        keys.insert(active_kid.clone(), secret);
        Self { active_kid, keys }
    }

    pub fn active_key(&self) -> (&str, &[u8]) {
        let key = self.keys.get(&self.active_kid).expect("active key exists");
        (&self.active_kid, key.as_slice())
    }

    pub fn get_key(&self, kid: Option<&str>) -> Option<&[u8]> {
        match kid {
            Some(k) => self.keys.get(k).map(|v| v.as_slice()),
            None => self.keys.get(&self.active_kid).map(|v| v.as_slice()),
        }
    }

    pub fn rotate_key(&mut self, new_kid: &str, new_secret: Vec<u8>) {
        self.keys.insert(new_kid.to_string(), new_secret);
        self.active_kid = new_kid.to_string();
    }
}

fn get_key_store() -> &'static RwLock<KeyStore> {
    static KEY_STORE: std::sync::OnceLock<RwLock<KeyStore>> = std::sync::OnceLock::new();
    KEY_STORE.get_or_init(|| RwLock::new(KeyStore::new()))
}

pub fn jwt_secret() -> Vec<u8> {
    let store = get_key_store().read().unwrap();
    store.active_key().1.to_vec()
}

pub fn rotate_active_key(new_kid: &str, new_secret: Vec<u8>) {
    let mut store = get_key_store().write().unwrap();
    store.rotate_key(new_kid, new_secret);
}

/// JWT Header representing RFC 7519 Jose Header.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JwtHeader {
    pub alg: String,
    pub typ: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kid: Option<String>,
}

/// Standard RFC 7519 JWT Claims + application metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionClaims {
    pub iss: String,      // Issuer
    pub aud: String,      // Audience
    pub sub: i64,         // Subject (User ID)
    pub username: String, // Username / Email
    pub role: String,     // Role (admin, user, member, etc.)
    pub exp: u64,         // Expiration unix timestamp
    pub iat: u64,         // Issued at unix timestamp
    pub nbf: u64,         // Not before unix timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jti: Option<String>, // Unique JWT ID
}

impl SessionClaims {
    pub fn new(user_id: i64, username: &str, role: &str, exp: u64) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            iss: DEFAULT_JWT_ISSUER.to_string(),
            aud: DEFAULT_JWT_AUDIENCE.to_string(),
            sub: user_id,
            username: username.to_string(),
            role: role.to_string(),
            exp,
            iat: now,
            nbf: now,
            jti: Some(generate_random_hex(16)),
        }
    }

    pub fn user_id(&self) -> i64 {
        self.sub
    }
}

/// Execution context carrying authenticated identity into query planning and RLS evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionContext {
    pub user_id: Option<i64>,
    pub role: Option<String>,
    pub is_admin: bool,
}

impl ExecutionContext {
    pub fn anonymous() -> Self {
        Self {
            user_id: None,
            role: None,
            is_admin: false,
        }
    }

    pub fn admin() -> Self {
        Self {
            user_id: Some(0),
            role: Some("admin".into()),
            is_admin: true,
        }
    }

    pub fn authenticated(user_id: i64, role: &str) -> Self {
        let is_admin = role == "admin";
        Self {
            user_id: Some(user_id),
            role: Some(role.to_string()),
            is_admin,
        }
    }

    pub fn from_claims(claims: &SessionClaims) -> Self {
        let is_admin = claims.role == "admin";
        Self {
            user_id: Some(claims.sub),
            role: Some(claims.role.clone()),
            is_admin,
        }
    }

    pub fn is_authenticated(&self) -> bool {
        self.user_id.is_some()
    }
}

/// Computes a secure password hash using Argon2id with random salt.
pub fn hash_password(password: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    match argon2.hash_password(password.as_bytes(), &salt) {
        Ok(hash) => hash.to_string(),
        Err(e) => panic!("Argon2id password hashing failed: {e}"),
    }
}

/// Verifies a password against an Argon2id stored hash using constant-time comparison.
pub fn verify_password(password: &str, stored_hash: &str) -> bool {
    let parsed_hash = match PasswordHash::new(stored_hash) {
        Ok(h) => h,
        Err(_) => return false,
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok()
}

/// Encodes and signs a standard RFC 7519 JWT using HMAC-SHA256 and the active Key ID.
pub fn sign_jwt(claims: &SessionClaims, secret: &[u8]) -> String {
    let store = get_key_store().read().unwrap();
    let (kid, _) = store.active_key();

    let header = JwtHeader {
        alg: "HS256".to_string(),
        typ: "JWT".to_string(),
        kid: Some(kid.to_string()),
    };

    let header_encoded = base64_url_encode(&serde_json::to_vec(&header).unwrap_or_default());
    let payload_encoded = base64_url_encode(&serde_json::to_vec(claims).unwrap_or_default());
    let to_sign = format!("{header_encoded}.{payload_encoded}");

    let signature_bytes = compute_hmac_signature(to_sign.as_bytes(), secret);
    let signature = base64_url_encode(&signature_bytes);
    format!("{to_sign}.{signature}")
}

/// Decodes and verifies a JWT token with full RFC 7519 checks:
/// signature, algorithm (rejects alg=none), expiration, not-before, issuer, and audience.
pub fn verify_jwt(token: &str, fallback_secret: &[u8]) -> Result<SessionClaims> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(DbError::Exec(crate::error::ExecError::InvalidValue(
            "invalid JWT structure (expected header.payload.signature)".into(),
        )));
    }

    // 1. Decode & validate Jose Header
    let header_bytes = base64_url_decode(parts[0]).map_err(|_| {
        DbError::Exec(crate::error::ExecError::InvalidValue(
            "invalid JWT base64 header".into(),
        ))
    })?;
    let header: JwtHeader = serde_json::from_slice(&header_bytes).map_err(|_| {
        DbError::Exec(crate::error::ExecError::InvalidValue(
            "invalid JWT header json".into(),
        ))
    })?;

    // Fail-closed algorithm verification: only HS256 allowed (reject alg=none)
    if header.alg != "HS256" {
        return Err(DbError::Exec(crate::error::ExecError::InvalidValue(
            format!("unsupported or forbidden JWT algorithm: {}", header.alg),
        )));
    }

    // 2. Resolve secret by kid or fallback
    let store = get_key_store().read().unwrap();
    let secret = if let Some(kid) = header.kid.as_deref() {
        if let Some(k) = store.get_key(Some(kid)) {
            if fallback_secret != DEFAULT_DEV_JWT_SECRET && fallback_secret != k {
                fallback_secret
            } else {
                k
            }
        } else {
            fallback_secret
        }
    } else {
        fallback_secret
    };

    // 3. Constant-time signature verification
    let to_sign = format!("{}.{}", parts[0], parts[1]);
    let expected_sig_bytes = compute_hmac_signature(to_sign.as_bytes(), secret);
    let expected_sig = base64_url_encode(&expected_sig_bytes);

    let is_valid = expected_sig
        .as_bytes()
        .ct_eq(parts[2].as_bytes())
        .unwrap_u8()
        == 1;

    if !is_valid {
        let fallback_sig_bytes = compute_hmac_signature(to_sign.as_bytes(), fallback_secret);
        let fallback_sig = base64_url_encode(&fallback_sig_bytes);
        if fallback_sig
            .as_bytes()
            .ct_eq(parts[2].as_bytes())
            .unwrap_u8()
            != 1
        {
            return Err(DbError::Exec(crate::error::ExecError::InvalidValue(
                "invalid JWT signature".into(),
            )));
        }
    }

    // 4. Decode claims
    let payload_bytes = base64_url_decode(parts[1]).map_err(|_| {
        DbError::Exec(crate::error::ExecError::InvalidValue(
            "invalid JWT base64 payload".into(),
        ))
    })?;
    let claims: SessionClaims = serde_json::from_slice(&payload_bytes).map_err(|_| {
        DbError::Exec(crate::error::ExecError::InvalidValue(
            "invalid JWT claims json".into(),
        ))
    })?;

    // 5. Standard RFC 7519 time & scope checks
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Expired check
    if claims.exp < now {
        return Err(DbError::Exec(crate::error::ExecError::InvalidValue(
            "JWT token has expired".into(),
        )));
    }

    // Not Before check (allow 5s clock skew)
    if claims.nbf > now.saturating_add(5) {
        return Err(DbError::Exec(crate::error::ExecError::InvalidValue(
            "JWT token not yet valid (nbf in future)".into(),
        )));
    }

    // Issued At future check (allow 60s clock skew)
    if claims.iat > now.saturating_add(60) {
        return Err(DbError::Exec(crate::error::ExecError::InvalidValue(
            "JWT token issued in the future (iat in future)".into(),
        )));
    }

    // Issuer check
    if !claims.iss.is_empty() && claims.iss != DEFAULT_JWT_ISSUER {
        return Err(DbError::Exec(crate::error::ExecError::InvalidValue(
            format!(
                "invalid JWT issuer: expected {}, got {}",
                DEFAULT_JWT_ISSUER, claims.iss
            ),
        )));
    }

    // Audience check
    if !claims.aud.is_empty() && claims.aud != DEFAULT_JWT_AUDIENCE {
        return Err(DbError::Exec(crate::error::ExecError::InvalidValue(
            format!(
                "invalid JWT audience: expected {}, got {}",
                DEFAULT_JWT_AUDIENCE, claims.aud
            ),
        )));
    }

    Ok(claims)
}

/// Generates a cryptographically secure opaque refresh token (`rt_<hex>`).
pub fn generate_opaque_refresh_token() -> String {
    format!("rt_{}", generate_random_hex(32))
}

/// Generates a unique token family ID (`tf_<hex>`).
pub fn generate_token_family_id() -> String {
    format!("tf_{}", generate_random_hex(16))
}

/// Computes SHA-256 hash of a string (used for storing refresh tokens safely).
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(&hasher.finalize())
}

#[derive(Debug, Clone)]
pub struct RefreshTokenRecord {
    pub user_id: i64,
    pub username: String,
    pub role: String,
    pub family_id: String,
    pub expires_at: u64,
    pub revoked: bool,
}

pub struct RefreshSessionStore {
    tokens: HashMap<String, RefreshTokenRecord>,
    families: HashMap<String, Vec<String>>,
}

impl Default for RefreshSessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl RefreshSessionStore {
    pub fn new() -> Self {
        Self {
            tokens: HashMap::new(),
            families: HashMap::new(),
        }
    }

    pub fn issue_token(&mut self, user_id: i64, username: &str, role: &str) -> (String, String) {
        let raw_token = generate_opaque_refresh_token();
        let family_id = generate_token_family_id();
        let token_hash = hash_token(&raw_token);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let expires_at = now + 86400 * 30; // 30 days

        let record = RefreshTokenRecord {
            user_id,
            username: username.to_string(),
            role: role.to_string(),
            family_id: family_id.clone(),
            expires_at,
            revoked: false,
        };

        self.tokens.insert(token_hash.clone(), record);
        self.families
            .entry(family_id.clone())
            .or_default()
            .push(token_hash);

        (raw_token, family_id)
    }

    pub fn rotate_token(&mut self, raw_token: &str) -> Result<(SessionClaims, String)> {
        let token_hash = hash_token(raw_token);
        let record = match self.tokens.get_mut(&token_hash) {
            Some(r) => r,
            None => {
                return Err(DbError::Exec(crate::error::ExecError::InvalidValue(
                    "invalid refresh token".into(),
                )))
            }
        };

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        if record.expires_at < now {
            return Err(DbError::Exec(crate::error::ExecError::InvalidValue(
                "refresh token has expired".into(),
            )));
        }

        // REUSE DETECTION: If an already-revoked token is used, invalidate entire family!
        if record.revoked {
            let family_id = record.family_id.clone();
            self.revoke_family(&family_id);
            return Err(DbError::Exec(crate::error::ExecError::InvalidValue(
                "refresh token reuse detected; session family has been revoked".into(),
            )));
        }

        // Mark current token as revoked
        record.revoked = true;
        let user_id = record.user_id;
        let username = record.username.clone();
        let role = record.role.clone();
        let family_id = record.family_id.clone();

        // Issue replacement token under same family
        let new_raw_token = generate_opaque_refresh_token();
        let new_token_hash = hash_token(&new_raw_token);
        let new_expires_at = now + 86400 * 30;

        let new_record = RefreshTokenRecord {
            user_id,
            username: username.clone(),
            role: role.clone(),
            family_id: family_id.clone(),
            expires_at: new_expires_at,
            revoked: false,
        };

        self.tokens.insert(new_token_hash.clone(), new_record);
        if let Some(list) = self.families.get_mut(&family_id) {
            list.push(new_token_hash);
        }

        let exp = now + 86400 * 7;
        let claims = SessionClaims::new(user_id, &username, &role, exp);
        Ok((claims, new_raw_token))
    }

    pub fn revoke_family(&mut self, family_id: &str) {
        if let Some(token_hashes) = self.families.remove(family_id) {
            for h in token_hashes {
                self.tokens.remove(&h);
            }
        }
    }

    pub fn revoke_token(&mut self, raw_token: &str) -> bool {
        let token_hash = hash_token(raw_token);
        if let Some(record) = self.tokens.get_mut(&token_hash) {
            record.revoked = true;
            true
        } else {
            false
        }
    }
}

pub fn issue_refresh_token(user_id: i64, username: &str, role: &str) -> (String, String) {
    let mut store = get_session_store().write().unwrap();
    store.issue_token(user_id, username, role)
}

pub fn rotate_refresh_token(raw_token: &str) -> Result<(SessionClaims, String)> {
    let mut store = get_session_store().write().unwrap();
    store.rotate_token(raw_token)
}

pub fn revoke_refresh_token(raw_token: &str) -> bool {
    let mut store = get_session_store().write().unwrap();
    store.revoke_token(raw_token)
}

fn get_session_store() -> &'static RwLock<RefreshSessionStore> {
    static SESSIONS_STORE: std::sync::OnceLock<RwLock<RefreshSessionStore>> =
        std::sync::OnceLock::new();
    SESSIONS_STORE.get_or_init(|| RwLock::new(RefreshSessionStore::new()))
}

/// Helper to generate random hex strings using OsRng
fn generate_random_hex(bytes_count: usize) -> String {
    let mut bytes = vec![0u8; bytes_count];
    let _ = getrandom::getrandom(&mut bytes);
    hex::encode(&bytes)
}

fn compute_hmac_signature(data: &[u8], secret: &[u8]) -> Vec<u8> {
    let mut mac =
        <HmacSha256 as Mac>::new_from_slice(secret).expect("HMAC can take key of any size");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn base64_url_encode(input: &[u8]) -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    let mut i = 0;
    while i < input.len() {
        let b0 = input[i] as u32;
        let b1 = if i + 1 < input.len() {
            input[i + 1] as u32
        } else {
            0
        };
        let b2 = if i + 2 < input.len() {
            input[i + 2] as u32
        } else {
            0
        };

        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(CHARSET[((triple >> 18) & 0x3F) as usize] as char);
        out.push(CHARSET[((triple >> 12) & 0x3F) as usize] as char);
        if i + 1 < input.len() {
            out.push(CHARSET[((triple >> 6) & 0x3F) as usize] as char);
        }
        if i + 2 < input.len() {
            out.push(CHARSET[(triple & 0x3F) as usize] as char);
        }
        i += 3;
    }
    out
}

fn base64_url_decode(input: &str) -> std::result::Result<Vec<u8>, ()> {
    fn decode_char(c: char) -> Option<u8> {
        match c {
            'A'..='Z' => Some(c as u8 - b'A'),
            'a'..='z' => Some(c as u8 - b'a' + 26),
            '0'..='9' => Some(c as u8 - b'0' + 52),
            '-' => Some(62),
            '_' => Some(63),
            _ => None,
        }
    }

    let chars: Vec<char> = input.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c0 = decode_char(chars[i]).ok_or(())? as u32;
        let c1 = if i + 1 < chars.len() {
            decode_char(chars[i + 1]).ok_or(())? as u32
        } else {
            0
        };
        let c2 = if i + 2 < chars.len() {
            decode_char(chars[i + 2]).ok_or(())? as u32
        } else {
            0
        };
        let c3 = if i + 3 < chars.len() {
            decode_char(chars[i + 3]).ok_or(())? as u32
        } else {
            0
        };

        let triple = (c0 << 18) | (c1 << 12) | (c2 << 6) | c3;

        out.push(((triple >> 16) & 0xFF) as u8);
        if i + 2 < chars.len() {
            out.push(((triple >> 8) & 0xFF) as u8);
        }
        if i + 3 < chars.len() {
            out.push((triple & 0xFF) as u8);
        }
        i += 4;
    }
    Ok(out)
}

mod hex {
    pub fn encode(data: &[u8]) -> String {
        let mut s = String::with_capacity(data.len() * 2);
        for &b in data {
            s.push_str(&format!("{b:02x}"));
        }
        s
    }
}
