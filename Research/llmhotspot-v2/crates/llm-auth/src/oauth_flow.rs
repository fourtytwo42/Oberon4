use crate::{
    claims::{openai_chatgpt_account_id, openai_email_from_access_token},
    AuthError,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use llm_storage::{StoredTokenRecord, TokenStore};
use oauth2::{
    basic::BasicClient, AuthUrl, ClientId, CsrfToken, PkceCodeChallenge, RedirectUrl, Scope,
    TokenUrl,
};
use rand::{rngs::OsRng, RngCore};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const DEFAULT_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const DEFAULT_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
pub const DEFAULT_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const DEFAULT_REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
pub const DEFAULT_SCOPE: &str = "openid profile email offline_access";
pub const DEFAULT_JWT_CLAIM_PATH: &str = "https://api.openai.com/auth";

#[derive(Debug, Clone)]
pub struct OpenAiOAuthConfig {
    pub client_id: String,
    pub authorize_url: String,
    pub token_url: String,
    pub redirect_uri: String,
    pub scope: String,
    pub jwt_claim_path: String,
    pub state_ttl: Duration,
}

impl Default for OpenAiOAuthConfig {
    fn default() -> Self {
        Self {
            client_id: DEFAULT_CLIENT_ID.to_string(),
            authorize_url: DEFAULT_AUTHORIZE_URL.to_string(),
            token_url: DEFAULT_TOKEN_URL.to_string(),
            redirect_uri: DEFAULT_REDIRECT_URI.to_string(),
            scope: DEFAULT_SCOPE.to_string(),
            jwt_claim_path: DEFAULT_JWT_CLAIM_PATH.to_string(),
            state_ttl: Duration::from_secs(600),
        }
    }
}

#[derive(Debug, Clone)]
pub struct OAuthStateRecord {
    pub redirect_uri: String,
    pub code_verifier: String,
    pub expires_unix: u64,
}

#[derive(Debug, Clone)]
pub struct InMemoryOAuthStateStore {
    ttl: Duration,
    inner: Arc<Mutex<HashMap<String, OAuthStateRecord>>>,
}

impl InMemoryOAuthStateStore {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn set(&self, key: String, value: OAuthStateRecord) {
        let mut guard = self.inner.lock().expect("oauth state mutex poisoned");
        prune(&mut guard);
        guard.insert(key, value);
    }

    pub fn consume(&self, key: &str) -> Option<OAuthStateRecord> {
        let mut guard = self.inner.lock().expect("oauth state mutex poisoned");
        prune(&mut guard);
        guard.remove(key)
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }
}

fn prune(map: &mut HashMap<String, OAuthStateRecord>) {
    let now = now_unix();
    map.retain(|_, v| v.expires_unix > now);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhamUsageWindow {
    pub used: u64,
    pub limit: u64,
    pub reset_unix: u64,
    pub percent_used: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhamUsage {
    pub five_hour: Option<WhamUsageWindow>,
    pub weekly: Option<WhamUsageWindow>,
    pub fetched_unix: u64,
    pub plan_type: Option<String>,
    pub credits_balance: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct WhamCredits {
    #[serde(default)]
    balance: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct WhamResponse {
    #[serde(default)]
    rate_limit: Option<WhamRateLimit>,
    #[serde(default)]
    plan_type: Option<String>,
    #[serde(default)]
    credits: Option<WhamCredits>,
}

#[derive(Debug, Deserialize)]
struct WhamRateLimit {
    #[serde(default)]
    primary_window: Option<WhamRateWindow>,
    #[serde(default)]
    secondary_window: Option<WhamRateWindow>,
}

#[derive(Debug, Deserialize)]
struct WhamRateWindow {
    #[serde(default)]
    used_percent: Option<f64>,
    #[serde(default)]
    reset_at: Option<u64>,
    #[serde(default)]
    used: Option<u64>,
    #[serde(default)]
    limit: Option<u64>,
}

fn wham_window(rate: &WhamRateWindow) -> Option<WhamUsageWindow> {
    let percent = rate.used_percent.unwrap_or(0.0).clamp(0.0, 100.0);
    let limit = rate.limit.unwrap_or(100);
    let used = rate
        .used
        .unwrap_or_else(|| ((percent / 100.0) * limit as f64).round() as u64);
    Some(WhamUsageWindow {
        used,
        limit,
        reset_unix: rate.reset_at.unwrap_or(0),
        percent_used: percent,
    })
}

#[derive(Debug, Clone)]
pub struct OAuthStart {
    pub authorization_url: String,
    pub state: String,
    pub redirect_uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBundle {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
    pub expires_at_unix: u64,
    pub account_id: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    expires_in: u64,
}

#[derive(Debug, Clone)]
pub struct OpenAiChatGptOAuthClient {
    pub config: OpenAiOAuthConfig,
    pub state_store: InMemoryOAuthStateStore,
    http: Client,
}

impl Default for OpenAiChatGptOAuthClient {
    fn default() -> Self {
        let config = OpenAiOAuthConfig::default();
        let state_store = InMemoryOAuthStateStore::new(config.state_ttl);
        Self {
            config,
            state_store,
            http: Client::new(),
        }
    }
}

impl OpenAiChatGptOAuthClient {
    pub fn new(config: OpenAiOAuthConfig) -> Self {
        let state_store = InMemoryOAuthStateStore::new(config.state_ttl);
        Self {
            config,
            state_store,
            http: Client::new(),
        }
    }

    pub fn start_flow(&self, redirect_uri: Option<&str>) -> Result<OAuthStart, AuthError> {
        let redirect_uri = redirect_uri
            .unwrap_or(&self.config.redirect_uri)
            .to_string();
        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        let expires_unix = now_unix().saturating_add(self.state_store.ttl().as_secs());

        let auth_url = AuthUrl::new(self.config.authorize_url.clone())
            .map_err(|e| AuthError::InvalidConfig(e.to_string()))?;
        let token_url = TokenUrl::new(self.config.token_url.clone())
            .map_err(|e| AuthError::InvalidConfig(e.to_string()))?;
        let redirect = RedirectUrl::new(redirect_uri.clone())
            .map_err(|e| AuthError::InvalidConfig(e.to_string()))?;
        let oauth_client = BasicClient::new(ClientId::new(self.config.client_id.clone()))
            .set_auth_uri(auth_url)
            .set_token_uri(token_url)
            .set_redirect_uri(redirect);

        let mut auth_req = oauth_client
            .authorize_url(CsrfToken::new_random)
            .set_pkce_challenge(pkce_challenge);
        for scope in self.config.scope.split_whitespace() {
            auth_req = auth_req.add_scope(Scope::new(scope.to_string()));
        }
        let (mut authorization_url, csrf_state) = auth_req.url();
        authorization_url
            .query_pairs_mut()
            .append_pair("id_token_add_organizations", "true")
            .append_pair("codex_cli_simplified_flow", "true")
            .append_pair("originator", "pi");
        let state = csrf_state.secret().to_string();

        self.state_store.set(
            state.clone(),
            OAuthStateRecord {
                redirect_uri: redirect_uri.clone(),
                code_verifier: pkce_verifier.secret().to_string(),
                expires_unix,
            },
        );

        Ok(OAuthStart {
            authorization_url: authorization_url.to_string(),
            state,
            redirect_uri,
        })
    }

    pub async fn exchange_code(
        &self,
        state: &str,
        code: &str,
        redirect_uri: Option<&str>,
    ) -> Result<TokenBundle, AuthError> {
        if code.trim().is_empty() {
            return Err(AuthError::CodeMissing);
        }
        let Some(flow_state) = self.state_store.consume(state) else {
            return Err(AuthError::InvalidState);
        };

        if let Some(uri) = redirect_uri {
            if !uri.trim().is_empty() && uri != flow_state.redirect_uri {
                return Err(AuthError::RedirectMismatch);
            }
        }

        let response = self
            .http
            .post(&self.config.token_url)
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", self.config.client_id.as_str()),
                ("code", code.trim()),
                ("code_verifier", flow_state.code_verifier.as_str()),
                ("redirect_uri", flow_state.redirect_uri.as_str()),
            ])
            .send()
            .await
            .map_err(AuthError::from)?;

        if !response.status().is_success() {
            return Err(AuthError::TokenExchangeFailed {
                status: response.status().as_u16(),
                body: response.text().await.unwrap_or_default(),
            });
        }

        let token: TokenResponse = response.json().await.map_err(AuthError::from)?;
        token_bundle_from_response(token, &self.config.jwt_claim_path, None)
    }

    pub async fn refresh(&self, refresh_token: &str) -> Result<TokenBundle, AuthError> {
        if refresh_token.trim().is_empty() {
            return Err(AuthError::RefreshMissing);
        }

        let response = self
            .http
            .post(&self.config.token_url)
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", self.config.client_id.as_str()),
                ("refresh_token", refresh_token),
            ])
            .send()
            .await
            .map_err(AuthError::from)?;

        if !response.status().is_success() {
            return Err(AuthError::RefreshFailed {
                status: response.status().as_u16(),
                body: response.text().await.unwrap_or_default(),
            });
        }

        let token: TokenResponse = response.json().await.map_err(AuthError::from)?;
        token_bundle_from_response(token, &self.config.jwt_claim_path, Some(refresh_token))
    }

    pub async fn fetch_wham_usage(
        &self,
        access_token: &str,
        account_id: Option<&str>,
    ) -> Result<WhamUsage, AuthError> {
        let fetched_unix = now_unix();
        let mut req = self
            .http
            .get("https://chatgpt.com/backend-api/wham/usage")
            .bearer_auth(access_token)
            .header("user-agent", "llmhotspot/0.1")
            .header("accept", "application/json");
        if let Some(id) = account_id {
            req = req.header("chatgpt-account-id", id);
        }
        let resp = req.send().await.map_err(AuthError::from)?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(AuthError::Http(format!("wham usage {status}: {body}")));
        }
        let body: WhamResponse = resp.json().await.map_err(AuthError::from)?;
        let five_hour = body
            .rate_limit
            .as_ref()
            .and_then(|r| r.primary_window.as_ref())
            .and_then(wham_window);
        let weekly = body
            .rate_limit
            .as_ref()
            .and_then(|r| r.secondary_window.as_ref())
            .and_then(wham_window);
        let plan_type = body.plan_type.clone();
        let credits_balance = body.credits.as_ref().and_then(|c| {
            c.balance.as_ref().and_then(|b| match b {
                serde_json::Value::Number(n) => n.as_f64(),
                serde_json::Value::String(s) => s.parse::<f64>().ok(),
                _ => None,
            })
        });
        Ok(WhamUsage {
            five_hour,
            weekly,
            fetched_unix,
            plan_type,
            credits_balance,
        })
    }

    pub fn save_refresh_token<T: TokenStore>(
        &self,
        store: &T,
        workspace_id: &str,
        connection_id: &str,
        token: &TokenBundle,
    ) -> Result<(), AuthError> {
        let mut rec = StoredTokenRecord::new(workspace_id, connection_id, &token.refresh_token);
        rec.account_id = token.account_id.clone();
        store
            .save(workspace_id, connection_id, &rec)
            .map_err(|e| AuthError::Storage(e.to_string()))
    }
}

fn token_bundle_from_response(
    token: TokenResponse,
    claim_path: &str,
    fallback_refresh_token: Option<&str>,
) -> Result<TokenBundle, AuthError> {
    let refresh_token = token
        .refresh_token
        .filter(|v| !v.trim().is_empty())
        .or_else(|| fallback_refresh_token.map(ToString::to_string))
        .ok_or_else(|| AuthError::TokenPayloadInvalid("missing refresh_token".to_string()))?;

    let access = token.access_token;
    Ok(TokenBundle {
        account_id: openai_chatgpt_account_id(&access, claim_path),
        email: openai_email_from_access_token(&access),
        expires_at_unix: now_unix().saturating_add(token.expires_in.saturating_sub(60)),
        expires_in: token.expires_in,
        access_token: access,
        refresh_token,
    })
}

pub fn random_urlsafe_token(byte_len: usize) -> String {
    let mut bytes = vec![0_u8; byte_len];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn pkce_challenge_s256(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
