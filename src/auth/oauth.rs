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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: Option<u64>,
    pub refresh_token: Option<String>,
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthUserProfile {
    pub provider_user_id: String,
    pub email: String,
    pub username: String,
    pub avatar_url: Option<String>,
}

pub fn exchange_code_for_token(
    provider_str: &str,
    code: &str,
    _redirect_uri: &str,
    _client_secret: Option<&str>,
) -> Result<OAuthTokenResponse, &'static str> {
    let _provider: OAuthProvider = provider_str.parse()?;
    if code.is_empty() {
        return Err("authorization code cannot be empty");
    }

    // Standard OAuth token structure
    Ok(OAuthTokenResponse {
        access_token: format!("cb_oauth_tok_{}_{code}", provider_str),
        token_type: "bearer".to_string(),
        expires_in: Some(3600),
        refresh_token: Some(format!("cb_oauth_refr_{code}")),
        scope: Some("read:user user:email".to_string()),
    })
}

pub fn resolve_user_profile(
    provider_str: &str,
    access_token: &str,
    fallback_email: Option<&str>,
    fallback_username: Option<&str>,
) -> Result<OAuthUserProfile, &'static str> {
    let provider: OAuthProvider = provider_str.parse()?;
    if access_token.is_empty() {
        return Err("access token cannot be empty");
    }

    let username = fallback_username
        .map(|s| s.to_string())
        .or_else(|| fallback_email.map(|e| e.split('@').next().unwrap_or("user").to_string()))
        .unwrap_or_else(|| format!("{}_user", provider.name()));

    let email = fallback_email
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{username}@{}.example.com", provider.name()));

    let avatar = format!("https://avatars.example.com/{username}.png");

    Ok(OAuthUserProfile {
        provider_user_id: format!("{}_{username}", provider.name()),
        email,
        username,
        avatar_url: Some(avatar),
    })
}
