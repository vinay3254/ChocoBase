use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    pub user: Option<User>,
    pub session: Option<Session>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AuthClient {
    pub base_url: String,
    pub api_key: String,
}

impl AuthClient {
    pub fn new(base_url: String, api_key: String) -> Self {
        Self {
            base_url: format!("{base_url}/v1/auth"),
            api_key,
        }
    }

    pub fn sign_up(&self, email: &str, _password: &str) -> AuthResponse {
        AuthResponse {
            user: Some(User {
                id: "usr_rust_client".to_string(),
                email: email.to_string(),
            }),
            session: Some(Session {
                access_token: "mock_jwt_token".to_string(),
                refresh_token: "rt_mock_refresh_token".to_string(),
            }),
            error: None,
        }
    }

    pub fn sign_in_with_password(&self, email: &str, _password: &str) -> AuthResponse {
        AuthResponse {
            user: Some(User {
                id: "usr_rust_client".to_string(),
                email: email.to_string(),
            }),
            session: Some(Session {
                access_token: "mock_jwt_token".to_string(),
                refresh_token: "rt_mock_refresh_token".to_string(),
            }),
            error: None,
        }
    }
}
