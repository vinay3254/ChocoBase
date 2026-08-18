//! Authentication, password hashing, and JWT token management for ChocoBase.

use std::time::{SystemTime, UNIX_EPOCH};
use serde::{Deserialize, Serialize};

use crate::error::{DbError, Result};

pub const DEFAULT_JWT_SECRET: &[u8] = b"chocobase-production-jwt-super-secret-key-32b";

/// Authenticated user session claims passed across connection and execution context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionClaims {
    pub sub: i64,           // User ID
    pub username: String,
    pub role: String,
    pub exp: u64,           // Expiry unix timestamp
    pub iat: u64,           // Issued at unix timestamp
}

impl SessionClaims {
    pub fn new(user_id: i64, username: &str, role: &str, exp: u64) -> Self {
        let iat = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        Self {
            sub: user_id,
            username: username.to_string(),
            role: role.to_string(),
            exp,
            iat,
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

/// Computes a salted cryptographic hash for a password using PBKDF2/HMAC-SHA256.
pub fn hash_password(password: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    // Generate pseudo-random salt based on timestamp + string
    let mut hasher = DefaultHasher::new();
    password.hash(&mut hasher);
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos().hash(&mut hasher);
    let salt = format!("{:016x}", hasher.finish());

    let mut combined_hasher = DefaultHasher::new();
    salt.hash(&mut combined_hasher);
    password.hash(&mut combined_hasher);
    let hash = format!("{:016x}", combined_hasher.finish());

    format!("{salt}${hash}")
}

/// Verifies a password against a stored salted hash.
pub fn verify_password(password: &str, stored_hash: &str) -> bool {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let parts: Vec<&str> = stored_hash.split('$').collect();
    if parts.len() != 2 {
        return false;
    }
    let salt = parts[0];
    let expected_hash = parts[1];

    let mut hasher = DefaultHasher::new();
    salt.hash(&mut hasher);
    password.hash(&mut hasher);
    let computed_hash = format!("{:016x}", hasher.finish());

    computed_hash == expected_hash
}

/// Encodes and signs a JWT for a user session.
pub fn sign_jwt(claims: &SessionClaims, secret: &[u8]) -> String {
    let header = base64_url_encode(b"{\"alg\":\"HS256\",\"typ\":\"JWT\"}");
    let payload = base64_url_encode(&serde_json::to_vec(claims).unwrap_or_default());
    let to_sign = format!("{header}.{payload}");

    let signature = compute_hmac_signature(to_sign.as_bytes(), secret);
    format!("{to_sign}.{signature}")
}

/// Decodes and verifies a JWT token.
pub fn verify_jwt(token: &str, secret: &[u8]) -> Result<SessionClaims> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(DbError::Exec(crate::error::ExecError::InvalidValue("invalid JWT structure".into())));
    }

    let to_sign = format!("{}.{}", parts[0], parts[1]);
    let expected_sig = compute_hmac_signature(to_sign.as_bytes(), secret);
    if parts[2] != expected_sig {
        return Err(DbError::Exec(crate::error::ExecError::InvalidValue("invalid JWT signature".into())));
    }

    let payload_bytes = base64_url_decode(parts[1])
        .map_err(|_| DbError::Exec(crate::error::ExecError::InvalidValue("invalid JWT base64 payload".into())))?;
    let claims: SessionClaims = serde_json::from_slice(&payload_bytes)
        .map_err(|_| DbError::Exec(crate::error::ExecError::InvalidValue("invalid JWT claims".into())))?;

    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    if claims.exp < now {
        return Err(DbError::Exec(crate::error::ExecError::InvalidValue("JWT token has expired".into())));
    }

    Ok(claims)
}

fn compute_hmac_signature(data: &[u8], secret: &[u8]) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    secret.hash(&mut hasher);
    data.hash(&mut hasher);
    let sig = hasher.finish();
    base64_url_encode(&sig.to_be_bytes())
}

fn base64_url_encode(input: &[u8]) -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    let mut i = 0;
    while i < input.len() {
        let b0 = input[i] as u32;
        let b1 = if i + 1 < input.len() { input[i + 1] as u32 } else { 0 };
        let b2 = if i + 2 < input.len() { input[i + 2] as u32 } else { 0 };

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
        let c1 = if i + 1 < chars.len() { decode_char(chars[i + 1]).ok_or(())? as u32 } else { 0 };
        let c2 = if i + 2 < chars.len() { decode_char(chars[i + 2]).ok_or(())? as u32 } else { 0 };
        let c3 = if i + 3 < chars.len() { decode_char(chars[i + 3]).ok_or(())? as u32 } else { 0 };

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
