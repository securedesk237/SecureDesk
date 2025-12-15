//! SSO/OIDC Authentication Module
//!
//! Provides OAuth2/OpenID Connect authentication for enterprise single sign-on.
//! Supports multiple identity providers (Azure AD, Okta, Google Workspace, etc.)
//!
//! Flow:
//! 1. User initiates SSO login
//! 2. Application opens browser with authorization URL
//! 3. User authenticates with identity provider
//! 4. IdP redirects to local callback server
//! 5. Application exchanges code for tokens
//! 6. User identity is verified and session established

#![allow(dead_code)]

use anyhow::{Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;
#[allow(unused_imports)]
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener as AsyncTcpListener;

/// OIDC Provider configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcProvider {
    /// Provider name (e.g., "Azure AD", "Okta", "Google")
    pub name: String,
    /// Client ID from the identity provider
    pub client_id: String,
    /// Client secret (optional for public clients with PKCE)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    /// Authorization endpoint URL
    pub authorization_endpoint: String,
    /// Token endpoint URL
    pub token_endpoint: String,
    /// UserInfo endpoint URL (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub userinfo_endpoint: Option<String>,
    /// JWKS URI for token verification
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jwks_uri: Option<String>,
    /// Issuer URL for token validation
    pub issuer: String,
    /// Scopes to request
    #[serde(default = "default_scopes")]
    pub scopes: Vec<String>,
    /// Use PKCE (recommended for native apps)
    #[serde(default = "default_true")]
    pub use_pkce: bool,
}

fn default_scopes() -> Vec<String> {
    vec!["openid".to_string(), "profile".to_string(), "email".to_string()]
}

fn default_true() -> bool {
    true
}

/// Well-known OIDC provider presets
impl OidcProvider {
    /// Azure AD (Microsoft Entra ID) preset
    pub fn azure_ad(tenant_id: &str, client_id: &str) -> Self {
        Self {
            name: "Microsoft Azure AD".to_string(),
            client_id: client_id.to_string(),
            client_secret: None,
            authorization_endpoint: format!(
                "https://login.microsoftonline.com/{}/oauth2/v2.0/authorize",
                tenant_id
            ),
            token_endpoint: format!(
                "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
                tenant_id
            ),
            userinfo_endpoint: Some("https://graph.microsoft.com/oidc/userinfo".to_string()),
            jwks_uri: Some(format!(
                "https://login.microsoftonline.com/{}/discovery/v2.0/keys",
                tenant_id
            )),
            issuer: format!("https://login.microsoftonline.com/{}/v2.0", tenant_id),
            scopes: vec![
                "openid".to_string(),
                "profile".to_string(),
                "email".to_string(),
            ],
            use_pkce: true,
        }
    }

    /// Okta preset
    pub fn okta(domain: &str, client_id: &str) -> Self {
        Self {
            name: "Okta".to_string(),
            client_id: client_id.to_string(),
            client_secret: None,
            authorization_endpoint: format!("https://{}/oauth2/v1/authorize", domain),
            token_endpoint: format!("https://{}/oauth2/v1/token", domain),
            userinfo_endpoint: Some(format!("https://{}/oauth2/v1/userinfo", domain)),
            jwks_uri: Some(format!("https://{}/oauth2/v1/keys", domain)),
            issuer: format!("https://{}", domain),
            scopes: vec![
                "openid".to_string(),
                "profile".to_string(),
                "email".to_string(),
            ],
            use_pkce: true,
        }
    }

    /// Google Workspace preset
    pub fn google(client_id: &str, client_secret: &str) -> Self {
        Self {
            name: "Google".to_string(),
            client_id: client_id.to_string(),
            client_secret: Some(client_secret.to_string()),
            authorization_endpoint: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
            token_endpoint: "https://oauth2.googleapis.com/token".to_string(),
            userinfo_endpoint: Some("https://openidconnect.googleapis.com/v1/userinfo".to_string()),
            jwks_uri: Some("https://www.googleapis.com/oauth2/v3/certs".to_string()),
            issuer: "https://accounts.google.com".to_string(),
            scopes: vec![
                "openid".to_string(),
                "profile".to_string(),
                "email".to_string(),
            ],
            use_pkce: true,
        }
    }

    /// Generic OIDC provider from discovery URL
    pub async fn from_discovery(discovery_url: &str, client_id: &str) -> Result<Self> {
        // Fetch OpenID Connect discovery document
        let client = reqwest::Client::new();
        let response = client.get(discovery_url).send().await?;
        let discovery: OidcDiscovery = response.json().await?;

        Ok(Self {
            name: "Custom OIDC".to_string(),
            client_id: client_id.to_string(),
            client_secret: None,
            authorization_endpoint: discovery.authorization_endpoint,
            token_endpoint: discovery.token_endpoint,
            userinfo_endpoint: discovery.userinfo_endpoint,
            jwks_uri: discovery.jwks_uri,
            issuer: discovery.issuer,
            scopes: vec![
                "openid".to_string(),
                "profile".to_string(),
                "email".to_string(),
            ],
            use_pkce: true,
        })
    }
}

/// OIDC Discovery document
#[derive(Debug, Deserialize)]
struct OidcDiscovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    userinfo_endpoint: Option<String>,
    jwks_uri: Option<String>,
}

/// PKCE (Proof Key for Code Exchange) challenge
#[derive(Debug, Clone)]
pub struct PkceChallenge {
    pub code_verifier: String,
    pub code_challenge: String,
}

impl PkceChallenge {
    fn new() -> Self {
        // Generate 32 random bytes for code verifier
        let mut verifier_bytes = [0u8; 32];
        getrandom::getrandom(&mut verifier_bytes).expect("Failed to generate random bytes");
        let code_verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);

        // Create SHA256 hash of verifier for challenge
        let challenge_hash = blake3::hash(code_verifier.as_bytes());
        let code_challenge = URL_SAFE_NO_PAD.encode(challenge_hash.as_bytes());

        Self {
            code_verifier,
            code_challenge,
        }
    }
}

/// OAuth2 token response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

/// User info from OIDC provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    /// Subject (unique user identifier)
    pub sub: String,
    /// User's full name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// User's email address
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Whether email is verified
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_verified: Option<bool>,
    /// User's preferred username
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_username: Option<String>,
    /// User's picture URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    /// Additional claims
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// SSO session information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SsoSession {
    /// User information
    pub user: UserInfo,
    /// Access token
    pub access_token: String,
    /// Token expiration timestamp
    pub expires_at: u64,
    /// Refresh token (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// ID token (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    /// Provider name
    pub provider: String,
}

impl SsoSession {
    /// Check if the session is expired
    pub fn is_expired(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        now >= self.expires_at
    }

    /// Check if the session needs refresh (within 5 minutes of expiry)
    pub fn needs_refresh(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        now + 300 >= self.expires_at
    }
}

/// SSO Configuration stored on disk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SsoConfig {
    /// Configured OIDC providers
    pub providers: Vec<OidcProvider>,
    /// Currently active session
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_session: Option<SsoSession>,
    /// Require SSO for all connections
    #[serde(default)]
    pub require_sso: bool,
    /// Allowed email domains (empty = all allowed)
    #[serde(default)]
    pub allowed_domains: Vec<String>,
}

impl Default for SsoConfig {
    fn default() -> Self {
        Self {
            providers: Vec::new(),
            active_session: None,
            require_sso: false,
            allowed_domains: Vec::new(),
        }
    }
}

impl SsoConfig {
    /// Get the config file path
    fn config_path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .context("Failed to get config directory")?
            .join("SecureDesk");
        fs::create_dir_all(&config_dir)?;
        Ok(config_dir.join("sso.json"))
    }

    /// Load SSO configuration from disk
    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;
        if path.exists() {
            let data = fs::read_to_string(&path)?;
            Ok(serde_json::from_str(&data)?)
        } else {
            Ok(Self::default())
        }
    }

    /// Save SSO configuration to disk
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        let data = serde_json::to_string_pretty(self)?;
        fs::write(path, data)?;
        Ok(())
    }

    /// Add a provider
    pub fn add_provider(&mut self, provider: OidcProvider) -> Result<()> {
        // Remove existing provider with same name
        self.providers.retain(|p| p.name != provider.name);
        self.providers.push(provider);
        self.save()
    }

    /// Remove a provider by name
    pub fn remove_provider(&mut self, name: &str) -> Result<()> {
        self.providers.retain(|p| p.name != name);
        self.save()
    }

    /// Get provider by name
    pub fn get_provider(&self, name: &str) -> Option<&OidcProvider> {
        self.providers.iter().find(|p| p.name == name)
    }

    /// Set active session
    pub fn set_session(&mut self, session: SsoSession) -> Result<()> {
        self.active_session = Some(session);
        self.save()
    }

    /// Clear active session
    pub fn clear_session(&mut self) -> Result<()> {
        self.active_session = None;
        self.save()
    }

    /// Check if user's email domain is allowed
    pub fn is_domain_allowed(&self, email: &str) -> bool {
        if self.allowed_domains.is_empty() {
            return true;
        }
        if let Some(domain) = email.split('@').nth(1) {
            self.allowed_domains.iter().any(|d| d.eq_ignore_ascii_case(domain))
        } else {
            false
        }
    }
}

/// SSO Manager handles authentication flow
pub struct SsoManager {
    config: SsoConfig,
    http_client: reqwest::Client,
}

impl SsoManager {
    /// Create a new SSO manager
    pub fn new() -> Result<Self> {
        let config = SsoConfig::load().unwrap_or_default();
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;

        Ok(Self { config, http_client })
    }

    /// Get current configuration
    pub fn config(&self) -> &SsoConfig {
        &self.config
    }

    /// Get mutable configuration
    pub fn config_mut(&mut self) -> &mut SsoConfig {
        &mut self.config
    }

    /// Get current session if valid
    pub fn current_session(&self) -> Option<&SsoSession> {
        self.config.active_session.as_ref().filter(|s| !s.is_expired())
    }

    /// Check if user is authenticated
    pub fn is_authenticated(&self) -> bool {
        self.current_session().is_some()
    }

    /// Start SSO login flow
    /// Returns the authorization URL to open in browser
    pub fn start_login(&self, provider: &OidcProvider) -> Result<(String, String, Option<PkceChallenge>)> {
        // Find an available port for the callback server
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        drop(listener);

        let redirect_uri = format!("http://127.0.0.1:{}/callback", port);

        // Generate state for CSRF protection
        let mut state_bytes = [0u8; 16];
        getrandom::getrandom(&mut state_bytes)?;
        let state = URL_SAFE_NO_PAD.encode(state_bytes);

        // Build authorization URL
        let mut auth_url = format!(
            "{}?client_id={}&redirect_uri={}&response_type=code&state={}&scope={}",
            provider.authorization_endpoint,
            urlencoding::encode(&provider.client_id),
            urlencoding::encode(&redirect_uri),
            urlencoding::encode(&state),
            urlencoding::encode(&provider.scopes.join(" ")),
        );

        // Add PKCE challenge if enabled
        let pkce = if provider.use_pkce {
            let challenge = PkceChallenge::new();
            auth_url.push_str(&format!(
                "&code_challenge={}&code_challenge_method=S256",
                urlencoding::encode(&challenge.code_challenge)
            ));
            Some(challenge)
        } else {
            None
        };

        Ok((auth_url, redirect_uri, pkce))
    }

    /// Wait for OAuth callback and exchange code for tokens
    pub async fn wait_for_callback(
        &mut self,
        provider: &OidcProvider,
        redirect_uri: &str,
        expected_state: &str,
        pkce: Option<PkceChallenge>,
    ) -> Result<SsoSession> {
        // Parse port from redirect URI
        let port: u16 = redirect_uri
            .split(':')
            .last()
            .and_then(|s| s.split('/').next())
            .and_then(|s| s.parse().ok())
            .context("Invalid redirect URI")?;

        // Start callback server
        let listener = AsyncTcpListener::bind(format!("127.0.0.1:{}", port)).await?;

        // Wait for callback with timeout
        let callback_task = async {
            let (mut socket, _) = listener.accept().await?;
            let mut reader = BufReader::new(&mut socket);

            // Read HTTP request
            let mut request_line = String::new();
            reader.read_line(&mut request_line).await?;

            // Parse the request
            // GET /callback?code=xxx&state=yyy HTTP/1.1
            let parts: Vec<&str> = request_line.split_whitespace().collect();
            if parts.len() < 2 {
                anyhow::bail!("Invalid callback request");
            }

            let path = parts[1];
            let query_start = path.find('?').unwrap_or(path.len());
            let query = &path[query_start..].trim_start_matches('?');

            // Parse query parameters
            let params: HashMap<String, String> = query
                .split('&')
                .filter_map(|pair| {
                    let mut parts = pair.splitn(2, '=');
                    Some((
                        urlencoding::decode(parts.next()?).ok()?,
                        urlencoding::decode(parts.next().unwrap_or("")).ok()?,
                    ))
                })
                .collect();

            // Send response to browser
            let response_body = r#"<!DOCTYPE html>
<html>
<head><title>SecureDesk SSO</title></head>
<body style="font-family: system-ui; text-align: center; padding: 50px;">
<h1>Authentication Successful</h1>
<p>You can close this window and return to SecureDesk.</p>
<script>window.close();</script>
</body>
</html>"#;

            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );

            // Get the underlying socket for writing
            drop(reader);
            socket.write_all(response.as_bytes()).await?;
            socket.flush().await?;

            Ok::<_, anyhow::Error>(params)
        };

        // Run callback with timeout
        let params = tokio::time::timeout(Duration::from_secs(120), callback_task)
            .await
            .context("SSO callback timeout")??;

        // Verify state
        let state = params.get("state").context("Missing state parameter")?;
        if state != expected_state {
            anyhow::bail!("Invalid state parameter - possible CSRF attack");
        }

        // Check for error
        if let Some(error) = params.get("error") {
            let desc = params.get("error_description").map(|s| s.as_str()).unwrap_or("");
            anyhow::bail!("OAuth error: {} - {}", error, desc);
        }

        // Get authorization code
        let code = params.get("code").context("Missing authorization code")?;

        // Exchange code for tokens
        let tokens = self.exchange_code(provider, code, redirect_uri, pkce).await?;

        // Get user info
        let user = self.get_user_info(provider, &tokens.access_token).await?;

        // Check domain restriction
        if let Some(ref email) = user.email {
            if !self.config.is_domain_allowed(email) {
                anyhow::bail!("Email domain not allowed: {}", email);
            }
        }

        // Calculate expiration
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let expires_at = now + tokens.expires_in.unwrap_or(3600);

        // Create session
        let session = SsoSession {
            user,
            access_token: tokens.access_token,
            expires_at,
            refresh_token: tokens.refresh_token,
            id_token: tokens.id_token,
            provider: provider.name.clone(),
        };

        // Save session
        self.config.set_session(session.clone())?;

        Ok(session)
    }

    /// Exchange authorization code for tokens
    async fn exchange_code(
        &self,
        provider: &OidcProvider,
        code: &str,
        redirect_uri: &str,
        pkce: Option<PkceChallenge>,
    ) -> Result<TokenResponse> {
        let mut params = vec![
            ("grant_type", "authorization_code".to_string()),
            ("code", code.to_string()),
            ("redirect_uri", redirect_uri.to_string()),
            ("client_id", provider.client_id.clone()),
        ];

        // Add client secret if available
        if let Some(ref secret) = provider.client_secret {
            params.push(("client_secret", secret.clone()));
        }

        // Add PKCE verifier if used
        if let Some(pkce) = pkce {
            params.push(("code_verifier", pkce.code_verifier));
        }

        let response = self
            .http_client
            .post(&provider.token_endpoint)
            .form(&params)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Token exchange failed: {}", error_text);
        }

        Ok(response.json().await?)
    }

    /// Get user info from identity provider
    async fn get_user_info(&self, provider: &OidcProvider, access_token: &str) -> Result<UserInfo> {
        let userinfo_url = provider
            .userinfo_endpoint
            .as_ref()
            .context("Provider does not support userinfo endpoint")?;

        let response = self
            .http_client
            .get(userinfo_url)
            .bearer_auth(access_token)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to get user info: {}", error_text);
        }

        Ok(response.json().await?)
    }

    /// Refresh the access token using refresh token
    pub async fn refresh_session(&mut self) -> Result<SsoSession> {
        let session = self
            .config
            .active_session
            .as_ref()
            .context("No active session")?;

        let refresh_token = session
            .refresh_token
            .as_ref()
            .context("No refresh token available")?;

        let provider = self
            .config
            .get_provider(&session.provider)
            .context("Provider not found")?
            .clone();

        let mut params = vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh_token.clone()),
            ("client_id", provider.client_id.clone()),
        ];

        if let Some(ref secret) = provider.client_secret {
            params.push(("client_secret", secret.clone()));
        }

        let response = self
            .http_client
            .post(&provider.token_endpoint)
            .form(&params)
            .send()
            .await?;

        if !response.status().is_success() {
            // Clear invalid session
            self.config.clear_session()?;
            anyhow::bail!("Token refresh failed");
        }

        let tokens: TokenResponse = response.json().await?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let expires_at = now + tokens.expires_in.unwrap_or(3600);

        // Update session with new tokens
        let mut new_session = session.clone();
        new_session.access_token = tokens.access_token;
        new_session.expires_at = expires_at;
        if let Some(refresh) = tokens.refresh_token {
            new_session.refresh_token = Some(refresh);
        }
        if let Some(id) = tokens.id_token {
            new_session.id_token = Some(id);
        }

        self.config.set_session(new_session.clone())?;

        Ok(new_session)
    }

    /// Logout and clear session
    pub fn logout(&mut self) -> Result<()> {
        self.config.clear_session()
    }

    /// Configure a new provider
    pub fn add_provider(&mut self, provider: OidcProvider) -> Result<()> {
        self.config.add_provider(provider)
    }

    /// Remove a provider
    pub fn remove_provider(&mut self, name: &str) -> Result<()> {
        self.config.remove_provider(name)
    }

    /// List all providers
    pub fn list_providers(&self) -> &[OidcProvider] {
        &self.config.providers
    }

    /// Set allowed email domains
    pub fn set_allowed_domains(&mut self, domains: Vec<String>) -> Result<()> {
        self.config.allowed_domains = domains;
        self.config.save()
    }

    /// Set whether SSO is required
    pub fn set_require_sso(&mut self, required: bool) -> Result<()> {
        self.config.require_sso = required;
        self.config.save()
    }
}

/// Simplified SSO info for UI
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SsoInfo {
    pub is_authenticated: bool,
    pub user_name: Option<String>,
    pub user_email: Option<String>,
    pub provider: Option<String>,
    pub expires_at: Option<u64>,
    pub require_sso: bool,
    pub providers: Vec<String>,
}

impl SsoInfo {
    pub fn from_manager(manager: &SsoManager) -> Self {
        let session = manager.current_session();
        Self {
            is_authenticated: session.is_some(),
            user_name: session.and_then(|s| s.user.name.clone()),
            user_email: session.and_then(|s| s.user.email.clone()),
            provider: session.map(|s| s.provider.clone()),
            expires_at: session.map(|s| s.expires_at),
            require_sso: manager.config.require_sso,
            providers: manager.list_providers().iter().map(|p| p.name.clone()).collect(),
        }
    }
}

// URL encoding helper
mod urlencoding {
    pub fn encode(s: &str) -> String {
        let mut result = String::new();
        for c in s.bytes() {
            match c {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    result.push(c as char);
                }
                _ => {
                    result.push_str(&format!("%{:02X}", c));
                }
            }
        }
        result
    }

    pub fn decode(s: &str) -> Result<String, std::string::FromUtf8Error> {
        let mut result = Vec::new();
        let mut chars = s.bytes().peekable();

        while let Some(c) = chars.next() {
            if c == b'%' {
                let high = chars.next().unwrap_or(0);
                let low = chars.next().unwrap_or(0);
                let byte = u8::from_str_radix(
                    &format!("{}{}", high as char, low as char),
                    16,
                ).unwrap_or(0);
                result.push(byte);
            } else if c == b'+' {
                result.push(b' ');
            } else {
                result.push(c);
            }
        }

        String::from_utf8(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pkce_generation() {
        let pkce = PkceChallenge::new();
        assert!(!pkce.code_verifier.is_empty());
        assert!(!pkce.code_challenge.is_empty());
        assert_ne!(pkce.code_verifier, pkce.code_challenge);
    }

    #[test]
    fn test_azure_ad_preset() {
        let provider = OidcProvider::azure_ad("test-tenant", "test-client");
        assert!(provider.authorization_endpoint.contains("test-tenant"));
        assert_eq!(provider.client_id, "test-client");
    }

    #[test]
    fn test_okta_preset() {
        let provider = OidcProvider::okta("dev-12345.okta.com", "test-client");
        assert!(provider.authorization_endpoint.contains("dev-12345"));
        assert_eq!(provider.client_id, "test-client");
    }

    #[test]
    fn test_domain_restriction() {
        let mut config = SsoConfig::default();
        config.allowed_domains = vec!["example.com".to_string(), "company.org".to_string()];

        assert!(config.is_domain_allowed("user@example.com"));
        assert!(config.is_domain_allowed("user@company.org"));
        assert!(!config.is_domain_allowed("user@other.com"));

        // Empty allowed list allows all
        config.allowed_domains.clear();
        assert!(config.is_domain_allowed("user@any.com"));
    }

    #[test]
    fn test_urlencoding() {
        assert_eq!(urlencoding::encode("hello world"), "hello%20world");
        assert_eq!(urlencoding::decode("hello%20world").unwrap(), "hello world");
    }
}
