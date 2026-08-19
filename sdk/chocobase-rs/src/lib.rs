//! Official Rust client SDK for the ChocoBase platform.

pub mod auth;
pub mod functions;
pub mod postgrest;
pub mod realtime;
pub mod storage;

pub use auth::{AuthClient, AuthResponse, Session, User};
pub use functions::FunctionsClient;
pub use postgrest::{PostgrestClient, QueryBuilder};
pub use realtime::{RealtimeChannel, RealtimeClient};
pub use storage::{BucketClient, StorageClient};

#[derive(Debug, Clone)]
pub struct ChocoClient {
    pub base_url: String,
    pub api_key: String,
    pub auth: AuthClient,
    pub postgrest: PostgrestClient,
    pub storage: StorageClient,
    pub functions: FunctionsClient,
    pub realtime: RealtimeClient,
}

impl ChocoClient {
    pub fn new(base_url: &str, api_key: &str) -> Self {
        let clean_url = base_url.trim_end_matches('/').to_string();
        Self {
            base_url: clean_url.clone(),
            api_key: api_key.to_string(),
            auth: AuthClient::new(clean_url.clone(), api_key.to_string()),
            postgrest: PostgrestClient::new(clean_url.clone(), api_key.to_string()),
            storage: StorageClient::new(clean_url.clone(), api_key.to_string()),
            functions: FunctionsClient::new(clean_url.clone(), api_key.to_string()),
            realtime: RealtimeClient::new(clean_url, api_key.to_string()),
        }
    }

    pub fn from(&self, table: &str) -> QueryBuilder {
        self.postgrest.from(table)
    }

    pub fn table(&self, table: &str) -> QueryBuilder {
        self.from(table)
    }
}

pub fn create_client(base_url: &str, api_key: &str) -> ChocoClient {
    ChocoClient::new(base_url, api_key)
}
