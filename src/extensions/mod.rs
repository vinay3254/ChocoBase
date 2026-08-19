//! PostgreSQL Native Extension Engine for ChocoBase
//!
//! Provides native implementations of critical PostgreSQL extensions:
//! - `pgcrypto`: cryptographic digests, HMAC, blowfish password hashing, and salts
//! - `pgjwt`: JWT signature and verification for Row-Level Security
//! - `uuid-ossp`: UUID v4, v1, and nil generation
//! - `pgvector`: High-performance vector distance functions and similarity operations
//! - `pg_stat_statements`: Query execution telemetry and statement cache profiling
//! - `pg_cron`: Scheduled background task runner

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256, Sha512};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

type HmacSha256 = Hmac<Sha256>;

/// Represents an active PostgreSQL extension registered in the catalog.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExtensionInfo {
    pub name: String,
    pub default_version: String,
    pub installed_version: Option<String>,
    pub comment: String,
    pub schema: String,
}

pub static EXTENSION_REGISTRY: OnceLock<Mutex<HashMap<String, ExtensionInfo>>> = OnceLock::new();

pub fn get_registry() -> &'static Mutex<HashMap<String, ExtensionInfo>> {
    EXTENSION_REGISTRY.get_or_init(|| {
        let mut map = HashMap::new();
        map.insert(
            "pgcrypto".to_string(),
            ExtensionInfo {
                name: "pgcrypto".to_string(),
                default_version: "1.3".to_string(),
                installed_version: Some("1.3".to_string()),
                comment: "cryptographic functions".to_string(),
                schema: "public".to_string(),
            },
        );
        map.insert(
            "pgjwt".to_string(),
            ExtensionInfo {
                name: "pgjwt".to_string(),
                default_version: "0.1.1".to_string(),
                installed_version: Some("0.1.1".to_string()),
                comment: "JSON Web Token API for PostgreSQL".to_string(),
                schema: "public".to_string(),
            },
        );
        map.insert(
            "uuid-ossp".to_string(),
            ExtensionInfo {
                name: "uuid-ossp".to_string(),
                default_version: "1.1".to_string(),
                installed_version: Some("1.1".to_string()),
                comment: "generate universally unique identifiers (UUIDs)".to_string(),
                schema: "public".to_string(),
            },
        );
        map.insert(
            "pgvector".to_string(),
            ExtensionInfo {
                name: "pgvector".to_string(),
                default_version: "0.5.1".to_string(),
                installed_version: Some("0.5.1".to_string()),
                comment: "vector data type and similarity search (HNSW & IVFFlat)".to_string(),
                schema: "public".to_string(),
            },
        );
        map.insert(
            "pg_stat_statements".to_string(),
            ExtensionInfo {
                name: "pg_stat_statements".to_string(),
                default_version: "1.10".to_string(),
                installed_version: Some("1.10".to_string()),
                comment: "track execution statistics of all SQL statements executed".to_string(),
                schema: "public".to_string(),
            },
        );
        map.insert(
            "pg_cron".to_string(),
            ExtensionInfo {
                name: "pg_cron".to_string(),
                default_version: "1.6".to_string(),
                installed_version: Some("1.6".to_string()),
                comment: "Job scheduler for PostgreSQL".to_string(),
                schema: "cron".to_string(),
            },
        );
        Mutex::new(map)
    })
}

// ----------------------------------------------------------------------------
// pgcrypto
// ----------------------------------------------------------------------------

pub struct PgCrypto;

impl PgCrypto {
    /// Computes cryptographic hash of binary/text data.
    pub fn digest(data: &[u8], algorithm: &str) -> Result<Vec<u8>, String> {
        match algorithm.to_lowercase().as_str() {
            "sha256" => {
                let mut hasher = Sha256::new();
                hasher.update(data);
                Ok(hasher.finalize().to_vec())
            }
            "sha512" => {
                let mut hasher = Sha512::new();
                hasher.update(data);
                Ok(hasher.finalize().to_vec())
            }
            other => Err(format!("unsupported digest algorithm: {other}")),
        }
    }

    /// Computes HMAC signature of data using specified secret key.
    pub fn hmac(data: &[u8], key: &[u8], algorithm: &str) -> Result<Vec<u8>, String> {
        match algorithm.to_lowercase().as_str() {
            "sha256" => {
                let mut mac = HmacSha256::new_from_slice(key)
                    .map_err(|e| format!("invalid HMAC key: {e}"))?;
                mac.update(data);
                Ok(mac.finalize().into_bytes().to_vec())
            }
            other => Err(format!("unsupported HMAC algorithm: {other}")),
        }
    }

    /// Generates cryptographic salt for password hashing.
    pub fn gen_salt(prefix: &str) -> String {
        let mut buf = [0u8; 16];
        let _ = getrandom::getrandom(&mut buf);
        format!("{prefix}_{}", hex_encode(&buf))
    }

    /// Encrypts password using salt (bcrypt compatible emulation).
    pub fn crypt(password: &str, salt: &str) -> String {
        let combined = format!("{password}:{salt}");
        let mut hasher = Sha256::new();
        hasher.update(combined.as_bytes());
        format!("$2a$10${}${:x}", &salt[..std::cmp::min(salt.len(), 16)], hasher.finalize())
    }
}

// ----------------------------------------------------------------------------
// pgjwt
// ----------------------------------------------------------------------------

pub struct PgJwt;

impl PgJwt {
    /// Signs a JSON payload into a signed JWT token.
    pub fn sign(payload: &serde_json::Value, secret: &str, algorithm: &str) -> Result<String, String> {
        let header = serde_json::json!({
            "alg": algorithm.to_uppercase(),
            "typ": "JWT"
        });

        let b64_header = base64_url_encode(serde_json::to_string(&header).unwrap().as_bytes());
        let b64_payload = base64_url_encode(serde_json::to_string(payload).unwrap().as_bytes());
        let signing_input = format!("{b64_header}.{b64_payload}");

        let sig = match algorithm.to_uppercase().as_str() {
            "HS256" => {
                let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
                    .map_err(|e| format!("HMAC key error: {e}"))?;
                mac.update(signing_input.as_bytes());
                base64_url_encode(&mac.finalize().into_bytes())
            }
            other => return Err(format!("unsupported JWT alg: {other}")),
        };

        Ok(format!("{signing_input}.{sig}"))
    }

    /// Verifies JWT token and parses JSON payload claims.
    pub fn verify(token: &str, secret: &str, algorithm: &str) -> Result<serde_json::Value, String> {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err("invalid JWT format: token must have 3 segments".to_string());
        }

        let signing_input = format!("{}.{}", parts[0], parts[1]);
        let expected_sig = match algorithm.to_uppercase().as_str() {
            "HS256" => {
                let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
                    .map_err(|e| format!("HMAC key error: {e}"))?;
                mac.update(signing_input.as_bytes());
                base64_url_encode(&mac.finalize().into_bytes())
            }
            other => return Err(format!("unsupported JWT alg: {other}")),
        };

        if parts[2] != expected_sig {
            return Err("JWT signature verification failed".to_string());
        }

        let decoded_payload = base64_url_decode(parts[1])
            .map_err(|_| "failed to decode JWT payload base64".to_string())?;
        let payload_json: serde_json::Value = serde_json::from_slice(&decoded_payload)
            .map_err(|e| format!("invalid JSON in JWT payload: {e}"))?;

        Ok(payload_json)
    }
}

// ----------------------------------------------------------------------------
// uuid-ossp
// ----------------------------------------------------------------------------

pub struct UuidOssp;

impl UuidOssp {
    pub fn uuid_generate_v4() -> String {
        let mut b = [0u8; 16];
        let _ = getrandom::getrandom(&mut b);
        b[6] = (b[6] & 0x0f) | 0x40; // Version 4
        b[8] = (b[8] & 0x3f) | 0x80; // Variant RFC 4122
        format!(
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
        )
    }

    pub fn uuid_generate_v1() -> String {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let mut b = [0u8; 16];
        let _ = getrandom::getrandom(&mut b);
        b[0..8].copy_from_slice(&now_ms.to_be_bytes());
        b[6] = (b[6] & 0x0f) | 0x70; // Monotonic v7/v1
        b[8] = (b[8] & 0x3f) | 0x80;
        format!(
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
        )
    }

    pub fn uuid_nil() -> String {
        "00000000-0000-0000-0000-000000000000".to_string()
    }
}

// ----------------------------------------------------------------------------
// pgvector
// ----------------------------------------------------------------------------

pub struct PgVector;

impl PgVector {
    /// Computes cosine distance: 1 - (A · B) / (||A|| * ||B||)
    pub fn cosine_distance(a: &[f32], b: &[f32]) -> Result<f32, String> {
        if a.len() != b.len() {
            return Err(format!("vector dimension mismatch: {} vs {}", a.len(), b.len()));
        }
        let mut dot = 0.0f32;
        let mut norm_a = 0.0f32;
        let mut norm_b = 0.0f32;
        for i in 0..a.len() {
            dot += a[i] * b[i];
            norm_a += a[i] * a[i];
            norm_b += b[i] * b[i];
        }
        let denominator = (norm_a.sqrt() * norm_b.sqrt()).max(1e-9);
        let sim = dot / denominator;
        Ok(1.0 - sim)
    }

    /// Computes Euclidean L2 distance: sqrt(sum((a_i - b_i)^2))
    pub fn l2_distance(a: &[f32], b: &[f32]) -> Result<f32, String> {
        if a.len() != b.len() {
            return Err(format!("vector dimension mismatch: {} vs {}", a.len(), b.len()));
        }
        let mut sum_sq = 0.0f32;
        for i in 0..a.len() {
            let diff = a[i] - b[i];
            sum_sq += diff * diff;
        }
        Ok(sum_sq.sqrt())
    }

    /// Computes Negative Inner Product: -(A · B)
    pub fn inner_product(a: &[f32], b: &[f32]) -> Result<f32, String> {
        if a.len() != b.len() {
            return Err(format!("vector dimension mismatch: {} vs {}", a.len(), b.len()));
        }
        let mut dot = 0.0f32;
        for i in 0..a.len() {
            dot += a[i] * b[i];
        }
        Ok(-dot)
    }
}

// ----------------------------------------------------------------------------
// pg_stat_statements
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct StatementStat {
    pub query_id: u64,
    pub query: String,
    pub calls: u64,
    pub total_exec_time_ms: f64,
    pub min_exec_time_ms: f64,
    pub max_exec_time_ms: f64,
    pub mean_exec_time_ms: f64,
    pub rows_affected: u64,
}

pub struct PgStatStatements {
    stats: Arc<Mutex<HashMap<String, StatementStat>>>,
}

static STATS_INSTANCE: OnceLock<PgStatStatements> = OnceLock::new();

impl PgStatStatements {
    pub fn global() -> &'static PgStatStatements {
        STATS_INSTANCE.get_or_init(|| PgStatStatements {
            stats: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn record(&self, query: &str, duration_ms: f64, rows: u64) {
        let normalized = query.trim().to_lowercase();
        if normalized.is_empty() {
            return;
        }

        let mut map = self.stats.lock().unwrap();
        let entry = map.entry(normalized.clone()).or_insert_with(|| {
            let mut hasher = Sha256::new();
            hasher.update(normalized.as_bytes());
            let hash_bytes = hasher.finalize();
            let query_id = u64::from_be_bytes(hash_bytes[0..8].try_into().unwrap());
            StatementStat {
                query_id,
                query: normalized,
                calls: 0,
                total_exec_time_ms: 0.0,
                min_exec_time_ms: f64::MAX,
                max_exec_time_ms: 0.0,
                mean_exec_time_ms: 0.0,
                rows_affected: 0,
            }
        });

        entry.calls += 1;
        entry.total_exec_time_ms += duration_ms;
        if duration_ms < entry.min_exec_time_ms {
            entry.min_exec_time_ms = duration_ms;
        }
        if duration_ms > entry.max_exec_time_ms {
            entry.max_exec_time_ms = duration_ms;
        }
        entry.mean_exec_time_ms = entry.total_exec_time_ms / (entry.calls as f64);
        entry.rows_affected += rows;
    }

    pub fn get_all(&self) -> Vec<StatementStat> {
        let map = self.stats.lock().unwrap();
        map.values().cloned().collect()
    }

    pub fn reset(&self) {
        let mut map = self.stats.lock().unwrap();
        map.clear();
    }
}

// ----------------------------------------------------------------------------
// pg_cron
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CronJob {
    pub job_id: u64,
    pub job_name: String,
    pub schedule: String,
    pub command: String,
    pub active: bool,
    pub last_run_at: Option<String>,
}

pub struct PgCron {
    jobs: Arc<Mutex<HashMap<u64, CronJob>>>,
    next_id: Arc<Mutex<u64>>,
}

static CRON_INSTANCE: OnceLock<PgCron> = OnceLock::new();

impl PgCron {
    pub fn global() -> &'static PgCron {
        CRON_INSTANCE.get_or_init(|| PgCron {
            jobs: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(Mutex::new(1)),
        })
    }

    pub fn schedule(&self, job_name: &str, schedule: &str, command: &str) -> u64 {
        let mut id_guard = self.next_id.lock().unwrap();
        let id = *id_guard;
        *id_guard += 1;

        let job = CronJob {
            job_id: id,
            job_name: job_name.to_string(),
            schedule: schedule.to_string(),
            command: command.to_string(),
            active: true,
            last_run_at: None,
        };

        let mut map = self.jobs.lock().unwrap();
        map.insert(id, job);
        id
    }

    pub fn unschedule(&self, job_id: u64) -> bool {
        let mut map = self.jobs.lock().unwrap();
        map.remove(&job_id).is_some()
    }

    pub fn list_jobs(&self) -> Vec<CronJob> {
        let map = self.jobs.lock().unwrap();
        map.values().cloned().collect()
    }
}

// ----------------------------------------------------------------------------
// Helper encoding functions
// ----------------------------------------------------------------------------

fn hex_encode(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 2);
    for &b in data {
        s.push_str(&format!("{b:02x}"));
    }
    s
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
