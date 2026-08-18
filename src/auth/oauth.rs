//! OAuth2 and Social Login Provider Integration for ChocoBase.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OAuthProvider {
    Google,
    GitHub,
    Apple,
    Discord,
    Custom,
}

impl std::str::FromStr for OAuthProvider {
    type Err = &'static str;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "google" => Ok(Self::Google),
            "github" => Ok(Self::GitHub),
            "apple" => Ok(Self::Apple),
            "discord" => Ok(Self::Discord),
            "custom" => Ok(Self::Custom),
            _ => Err("unsupported oauth provider"),
        }
    }
}

impl OAuthProvider {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::GitHub => "github",
            Self::Apple => "apple",
            Self::Discord => "discord",
            Self::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthAuthorizeResponse {
    pub provider: String,
    pub url: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthCallbackRequest {
    pub provider: String,
    pub code: String,
    pub state: Option<String>,
    pub email: Option<String>,
    pub username: Option<String>,
}

pub fn generate_authorize_url(
    provider_str: &str,
    redirect_uri: &str,
) -> Result<OAuthAuthorizeResponse, &'static str> {
    let provider: OAuthProvider = provider_str.parse()?;

    let state = format!(
        "st_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );

    let auth_endpoint = match provider {
        OAuthProvider::Google => "https://accounts.google.com/o/oauth2/v2/auth",
        OAuthProvider::GitHub => "https://github.com/login/oauth/authorize",
        OAuthProvider::Apple => "https://appleid.apple.com/auth/authorize",
        OAuthProvider::Discord => "https://discord.com/api/oauth2/authorize",
        OAuthProvider::Custom => "https://auth.example.com/oauth/authorize",
    };

    let url = format!(
        "{auth_endpoint}?client_id=chocobase_client&redirect_uri={redirect_uri}&response_type=code&state={state}&scope=email+profile"
    );

    Ok(OAuthAuthorizeResponse {
        provider: provider.name().to_string(),
        url,
        state,
    })
}
