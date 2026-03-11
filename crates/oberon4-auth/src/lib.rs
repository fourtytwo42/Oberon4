use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use oberon4_core::{
    AccountState, OAuthCallbackPayload, OAuthProvider, OAuthProviderKind, OAuthStart, OberonError,
    PendingOAuthFlow, SecretStore, StoredAccount, TokenBundle,
};

pub const OPENAI_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const OPENAI_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
pub const OPENAI_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const OPENAI_SCOPE: &str = "openid profile email offline_access";

pub const ANTHROPIC_CLIENT_ID: &str = "anthropic-console";
pub const ANTHROPIC_AUTHORIZE_URL: &str = "https://console.anthropic.com/v1/oauth/authorize";
pub const ANTHROPIC_TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";
pub const ANTHROPIC_SCOPE: &str = "org:create_api_key user:profile user:inference";

pub const GITHUB_CLIENT_ID_ENV: &str = "OBERON4_GITHUB_CLIENT_ID";
pub const GITHUB_CLIENT_SECRET_ENV: &str = "OBERON4_GITHUB_CLIENT_SECRET";
pub const GITHUB_AUTHORIZE_URL: &str = "https://github.com/login/oauth/authorize";
pub const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
pub const GITHUB_SCOPE: &str = "repo read:user user:email";
pub const GITHUB_CLIENT_ID_STORE_KEY: &str = "oauth.github.app.client_id";
pub const GITHUB_CLIENT_SECRET_STORE_KEY: &str = "oauth.github.app.client_secret";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthProviderConfig {
    pub provider: OAuthProviderKind,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub authorize_url: String,
    pub token_url: String,
    pub redirect_base: String,
    pub scope: String,
    pub state_ttl: Duration,
}

impl OAuthProviderConfig {
    pub fn openai() -> Self {
        Self {
            provider: OAuthProviderKind::OpenAi,
            client_id: OPENAI_CLIENT_ID.to_string(),
            client_secret: None,
            authorize_url: OPENAI_AUTHORIZE_URL.to_string(),
            token_url: OPENAI_TOKEN_URL.to_string(),
            redirect_base: "http://localhost:1455".to_string(),
            scope: OPENAI_SCOPE.to_string(),
            state_ttl: Duration::from_secs(600),
        }
    }

    pub fn anthropic() -> Self {
        Self {
            provider: OAuthProviderKind::Anthropic,
            client_id: ANTHROPIC_CLIENT_ID.to_string(),
            client_secret: None,
            authorize_url: ANTHROPIC_AUTHORIZE_URL.to_string(),
            token_url: ANTHROPIC_TOKEN_URL.to_string(),
            redirect_base: "http://localhost:1455".to_string(),
            scope: ANTHROPIC_SCOPE.to_string(),
            state_ttl: Duration::from_secs(600),
        }
    }

    pub fn github() -> Result<Self, OberonError> {
        let store = oberon4_core::FileSecretStore::default();
        let client_id = load_github_app_secret(
            &store,
            GITHUB_CLIENT_ID_ENV,
            GITHUB_CLIENT_ID_STORE_KEY,
            "client id",
        )?;
        let client_secret = load_github_app_secret(
            &store,
            GITHUB_CLIENT_SECRET_ENV,
            GITHUB_CLIENT_SECRET_STORE_KEY,
            "client secret",
        )?;
        Ok(Self {
            provider: OAuthProviderKind::Github,
            client_id,
            client_secret: Some(client_secret),
            authorize_url: GITHUB_AUTHORIZE_URL.to_string(),
            token_url: GITHUB_TOKEN_URL.to_string(),
            redirect_base: "http://localhost:1455".to_string(),
            scope: GITHUB_SCOPE.to_string(),
            state_ttl: Duration::from_secs(600),
        })
    }

    pub fn github_from_store<S: SecretStore>(store: &S) -> Result<Self, OberonError> {
        let client_id = store.get_secret(GITHUB_CLIENT_ID_STORE_KEY)?;
        let client_secret = store.get_secret(GITHUB_CLIENT_SECRET_STORE_KEY)?;
        Ok(Self {
            provider: OAuthProviderKind::Github,
            client_id,
            client_secret: Some(client_secret),
            authorize_url: GITHUB_AUTHORIZE_URL.to_string(),
            token_url: GITHUB_TOKEN_URL.to_string(),
            redirect_base: "http://localhost:1455".to_string(),
            scope: GITHUB_SCOPE.to_string(),
            state_ttl: Duration::from_secs(600),
        })
    }
}

pub trait OAuthTokenExchange {
    fn exchange_code(
        &self,
        provider: &OAuthProviderConfig,
        code: &str,
        code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<TokenBundle, OberonError>;
}

#[derive(Debug, Clone, Default)]
pub struct ReqwestOAuthTokenExchange;

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    account_id: Option<String>,
    #[serde(default)]
    label: Option<String>,
}

impl OAuthTokenExchange for ReqwestOAuthTokenExchange {
    fn exchange_code(
        &self,
        provider: &OAuthProviderConfig,
        code: &str,
        code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<TokenBundle, OberonError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|err| OberonError::Storage(format!("oauth client build failed: {err}")))?;

        let mut params = vec![
            ("grant_type", "authorization_code"),
            ("client_id", provider.client_id.as_str()),
            ("code", code),
            ("code_verifier", code_verifier),
            ("redirect_uri", redirect_uri),
        ];
        if let Some(client_secret) = provider.client_secret.as_deref() {
            params.push(("client_secret", client_secret));
        }

        let response = client
            .post(&provider.token_url)
            .header("Accept", "application/json")
            .form(&params)
            .send()
            .map_err(|err| OberonError::Storage(format!("oauth token request failed: {err}")))?;

        let status = response.status();
        let body = response
            .text()
            .map_err(|err| OberonError::Storage(format!("oauth token read failed: {err}")))?;

        if !status.is_success() {
            return Err(OberonError::Validation(format!(
                "oauth token endpoint returned HTTP {}",
                status.as_u16()
            )));
        }

        let token: OAuthTokenResponse = serde_json::from_str(&body).map_err(|err| {
            OberonError::Validation(format!("oauth token response invalid: {err}"))
        })?;

        let expires_at_unix = now_unix().saturating_add(token.expires_in.unwrap_or(3600));
        let provider_name = provider_name(provider.provider);

        Ok(TokenBundle {
            access_token: token.access_token,
            refresh_token: token.refresh_token,
            expires_at_unix,
            account_id: token
                .account_id
                .unwrap_or_else(|| format!("{provider_name}-{}", code.trim())),
            label: token
                .label
                .unwrap_or_else(|| format!("{} Account", provider_label(provider.provider))),
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountRegistry {
    accounts: Vec<StoredAccount>,
    active_accounts: BTreeMap<OAuthProviderKind, String>,
}

pub trait AccountMetadataStore {
    fn load_registry(&self) -> Result<AccountRegistry, OberonError>;
    fn save_registry(&mut self, registry: &AccountRegistry) -> Result<(), OberonError>;
}

#[derive(Debug, Default)]
pub struct InMemoryAccountMetadataStore {
    registry: AccountRegistry,
}

#[derive(Debug, Clone)]
pub struct FileAccountMetadataStore {
    path: PathBuf,
}

impl Default for FileAccountMetadataStore {
    fn default() -> Self {
        Self::new(oberon4_core::default_data_dir().join("accounts.json"))
    }
}

impl FileAccountMetadataStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl AccountMetadataStore for InMemoryAccountMetadataStore {
    fn load_registry(&self) -> Result<AccountRegistry, OberonError> {
        Ok(self.registry.clone())
    }

    fn save_registry(&mut self, registry: &AccountRegistry) -> Result<(), OberonError> {
        self.registry = registry.clone();
        Ok(())
    }
}

impl AccountMetadataStore for FileAccountMetadataStore {
    fn load_registry(&self) -> Result<AccountRegistry, OberonError> {
        if !self.path.exists() {
            return Ok(AccountRegistry::default());
        }
        let raw = fs::read(&self.path)
            .map_err(|err| OberonError::Storage(format!("account store read failed: {err}")))?;
        serde_json::from_slice(&raw)
            .map_err(|err| OberonError::Validation(format!("account store decode failed: {err}")))
    }

    fn save_registry(&mut self, registry: &AccountRegistry) -> Result<(), OberonError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                OberonError::Storage(format!("account store parent create failed: {err}"))
            })?;
        }
        let encoded = serde_json::to_vec_pretty(registry)
            .map_err(|err| OberonError::Storage(format!("account store encode failed: {err}")))?;
        fs::write(&self.path, encoded)
            .map_err(|err| OberonError::Storage(format!("account store write failed: {err}")))
    }
}

impl AccountRegistry {
    pub fn list(&self) -> Vec<StoredAccount> {
        self.accounts.clone()
    }

    pub fn connect<S: SecretStore>(
        &mut self,
        provider: OAuthProviderKind,
        token: &TokenBundle,
        store: &mut S,
    ) -> Result<StoredAccount, OberonError> {
        let account = StoredAccount {
            id: token.account_id.clone(),
            provider,
            label: token.label.clone(),
            state: AccountState::Active,
        };
        save_token_bundle(store, provider, token)?;
        self.upsert_account(account.clone());
        self.active_accounts.insert(provider, account.id.clone());
        Ok(account)
    }

    pub fn activate(
        &mut self,
        provider: OAuthProviderKind,
        account_id: &str,
    ) -> Result<(), OberonError> {
        let account = self
            .accounts
            .iter_mut()
            .find(|account| account.provider == provider && account.id == account_id)
            .ok_or_else(|| OberonError::NotFound(format!("account:{provider:?}:{account_id}")))?;
        account.state = AccountState::Active;
        self.active_accounts
            .insert(provider, account_id.to_string());
        Ok(())
    }

    pub fn disable(
        &mut self,
        provider: OAuthProviderKind,
        account_id: &str,
    ) -> Result<(), OberonError> {
        let account = self
            .accounts
            .iter_mut()
            .find(|account| account.provider == provider && account.id == account_id)
            .ok_or_else(|| OberonError::NotFound(format!("account:{provider:?}:{account_id}")))?;
        account.state = AccountState::Disabled;
        if self
            .active_accounts
            .get(&provider)
            .is_some_and(|id| id == account_id)
        {
            self.active_accounts.remove(&provider);
        }
        Ok(())
    }

    pub fn disconnect(
        &mut self,
        provider: OAuthProviderKind,
        account_id: &str,
    ) -> Result<(), OberonError> {
        let before = self.accounts.len();
        self.accounts
            .retain(|account| !(account.provider == provider && account.id == account_id));
        if self.accounts.len() == before {
            return Err(OberonError::NotFound(format!(
                "account:{provider:?}:{account_id}"
            )));
        }
        if self
            .active_accounts
            .get(&provider)
            .is_some_and(|id| id == account_id)
        {
            self.active_accounts.remove(&provider);
        }
        Ok(())
    }

    pub fn active_account_id(&self, provider: OAuthProviderKind) -> Option<&str> {
        self.active_accounts.get(&provider).map(String::as_str)
    }

    fn upsert_account(&mut self, account: StoredAccount) {
        if let Some(existing) = self
            .accounts
            .iter_mut()
            .find(|existing| existing.provider == account.provider && existing.id == account.id)
        {
            *existing = account;
        } else {
            self.accounts.push(account);
        }
    }
}

#[derive(Debug, Clone)]
pub struct OAuthFlowManager {
    config: OAuthProviderConfig,
}

impl OAuthFlowManager {
    pub fn new(config: OAuthProviderConfig) -> Self {
        Self { config }
    }

    pub fn provider(&self) -> OAuthProviderKind {
        self.config.provider
    }

    pub fn start_flow(
        &self,
        callback_path: &str,
        return_to: Option<&str>,
        now_unix: u64,
    ) -> Result<(OAuthStart, PendingOAuthFlow), OberonError> {
        let flow_id = random_url_safe(18);
        let oauth_state = random_url_safe(24);
        let code_verifier = random_url_safe(32);
        let code_challenge = pkce_challenge(&code_verifier);
        let callback_path = normalize_callback_path(callback_path);
        let redirect_uri = format!("{}{}", self.config.redirect_base, callback_path);
        let expires_at_unix = now_unix.saturating_add(self.config.state_ttl.as_secs());
        let return_to = sanitize_return_to(return_to)?;
        let authorization_url = format!(
            "{}?client_id={}&response_type=code&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256{}{}",
            self.config.authorize_url,
            url_encode(&self.config.client_id),
            url_encode(&redirect_uri),
            url_encode(&self.config.scope),
            url_encode(&oauth_state),
            url_encode(&code_challenge),
            provider_authorization_extras(self.config.provider),
            if let Some(return_to) = return_to.as_deref() {
                format!("&return_to={}", url_encode(return_to))
            } else {
                String::new()
            }
        );

        Ok((
            OAuthStart {
                flow_id: flow_id.clone(),
                provider: self.config.provider,
                authorization_url,
                redirect_uri: redirect_uri.clone(),
                expires_at_unix,
            },
            PendingOAuthFlow {
                flow_id,
                provider: self.config.provider,
                oauth_state,
                code_verifier,
                redirect_uri,
                expires_at_unix,
            },
        ))
    }

    pub fn complete_flow<S: SecretStore, X: OAuthTokenExchange>(
        &self,
        flow: &PendingOAuthFlow,
        callback: OAuthCallbackPayload,
        exchange: &X,
        registry: &mut AccountRegistry,
        store: &mut S,
        now_unix: u64,
    ) -> Result<StoredAccount, OberonError> {
        if flow.provider != self.config.provider {
            return Err(OberonError::Validation(
                "oauth flow provider mismatch".to_string(),
            ));
        }
        if now_unix > flow.expires_at_unix {
            return Err(OberonError::Validation("oauth state expired".to_string()));
        }
        if callback.state.trim().is_empty() || callback.code.trim().is_empty() {
            return Err(OberonError::Validation(
                "oauth callback missing required fields".to_string(),
            ));
        }
        if callback.state != flow.oauth_state {
            return Err(OberonError::Validation(
                "oauth callback state mismatch".to_string(),
            ));
        }
        let _ = sanitize_return_to(callback.return_to.as_deref())?;

        let token = exchange.exchange_code(
            &self.config,
            &callback.code,
            &flow.code_verifier,
            &flow.redirect_uri,
        )?;
        registry.connect(self.config.provider, &token, store)
    }

    pub fn complete_flow_bundle<X: OAuthTokenExchange>(
        &self,
        flow: &PendingOAuthFlow,
        callback: OAuthCallbackPayload,
        exchange: &X,
        now_unix: u64,
    ) -> Result<TokenBundle, OberonError> {
        if flow.provider != self.config.provider {
            return Err(OberonError::Validation(
                "oauth flow provider mismatch".to_string(),
            ));
        }
        if now_unix > flow.expires_at_unix {
            return Err(OberonError::Validation("oauth state expired".to_string()));
        }
        if callback.state.trim().is_empty() || callback.code.trim().is_empty() {
            return Err(OberonError::Validation(
                "oauth callback missing required fields".to_string(),
            ));
        }
        if callback.state != flow.oauth_state {
            return Err(OberonError::Validation(
                "oauth callback state mismatch".to_string(),
            ));
        }
        let _ = sanitize_return_to(callback.return_to.as_deref())?;

        exchange.exchange_code(
            &self.config,
            &callback.code,
            &flow.code_verifier,
            &flow.redirect_uri,
        )
    }
}

#[derive(Debug, Clone)]
pub struct OpenAiOAuthAdapter {
    inner: OAuthFlowManager,
}

impl Default for OpenAiOAuthAdapter {
    fn default() -> Self {
        Self {
            inner: OAuthFlowManager::new(OAuthProviderConfig::openai()),
        }
    }
}

impl OpenAiOAuthAdapter {
    pub fn flow_manager(&self) -> &OAuthFlowManager {
        &self.inner
    }
}

impl OAuthProvider for OpenAiOAuthAdapter {
    fn provider_kind(&self) -> OAuthProviderKind {
        OAuthProviderKind::OpenAi
    }

    fn start_connect_flow(&self) -> Result<(OAuthStart, PendingOAuthFlow), OberonError> {
        self.inner.start_flow("/auth/callback", None, now_unix())
    }

    fn exchange_callback(
        &self,
        flow: &PendingOAuthFlow,
        callback: OAuthCallbackPayload,
    ) -> Result<TokenBundle, OberonError> {
        self.inner
            .complete_flow_bundle(flow, callback, &ReqwestOAuthTokenExchange, now_unix())
    }
}

#[derive(Debug, Clone)]
pub struct AnthropicOAuthAdapter {
    inner: OAuthFlowManager,
}

impl Default for AnthropicOAuthAdapter {
    fn default() -> Self {
        Self {
            inner: OAuthFlowManager::new(OAuthProviderConfig::anthropic()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GithubOAuthAdapter {
    inner: Option<OAuthFlowManager>,
}

impl Default for GithubOAuthAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl GithubOAuthAdapter {
    pub fn new() -> Self {
        Self {
            inner: OAuthProviderConfig::github()
                .ok()
                .map(OAuthFlowManager::new),
        }
    }

    pub fn from_config(config: OAuthProviderConfig) -> Self {
        Self {
            inner: Some(OAuthFlowManager::new(config)),
        }
    }

    pub fn flow_manager(&self) -> Option<&OAuthFlowManager> {
        self.inner.as_ref()
    }
}

impl OAuthProvider for GithubOAuthAdapter {
    fn provider_kind(&self) -> OAuthProviderKind {
        OAuthProviderKind::Github
    }

    fn start_connect_flow(&self) -> Result<(OAuthStart, PendingOAuthFlow), OberonError> {
        self.inner
            .as_ref()
            .ok_or_else(|| {
                OberonError::Validation(
                    "github oauth requires app credentials in env or host secret store".to_string(),
                )
            })?
            .start_flow("/auth/callback", None, now_unix())
    }

    fn exchange_callback(
        &self,
        flow: &PendingOAuthFlow,
        callback: OAuthCallbackPayload,
    ) -> Result<TokenBundle, OberonError> {
        self.inner
            .as_ref()
            .ok_or_else(|| {
                OberonError::Validation(
                    "github oauth requires app credentials in env or host secret store".to_string(),
                )
            })?
            .complete_flow_bundle(flow, callback, &ReqwestOAuthTokenExchange, now_unix())
    }
}

impl AnthropicOAuthAdapter {
    pub fn flow_manager(&self) -> &OAuthFlowManager {
        &self.inner
    }
}

impl OAuthProvider for AnthropicOAuthAdapter {
    fn provider_kind(&self) -> OAuthProviderKind {
        OAuthProviderKind::Anthropic
    }

    fn start_connect_flow(&self) -> Result<(OAuthStart, PendingOAuthFlow), OberonError> {
        self.inner.start_flow("/callback", None, now_unix())
    }

    fn exchange_callback(
        &self,
        flow: &PendingOAuthFlow,
        callback: OAuthCallbackPayload,
    ) -> Result<TokenBundle, OberonError> {
        self.inner
            .complete_flow_bundle(flow, callback, &ReqwestOAuthTokenExchange, now_unix())
    }
}

pub fn store_provider_token<S: SecretStore>(
    store: &mut S,
    provider: &str,
    token: &str,
) -> Result<(), OberonError> {
    store.set_secret(&format!("provider.{provider}.token"), token)
}

fn save_token_bundle<S: SecretStore>(
    store: &mut S,
    provider: OAuthProviderKind,
    token: &TokenBundle,
) -> Result<(), OberonError> {
    let provider_id = provider_name(provider);
    store.set_secret(
        &format!("oauth.{provider_id}.{}.access_token", token.account_id),
        &token.access_token,
    )?;
    if let Some(refresh_token) = token.refresh_token.as_deref() {
        store.set_secret(
            &format!("oauth.{provider_id}.{}.refresh_token", token.account_id),
            refresh_token,
        )?;
    }
    Ok(())
}

fn random_url_safe(bytes: usize) -> String {
    let mut buffer = vec![0_u8; bytes];
    OsRng.fill_bytes(&mut buffer);
    URL_SAFE_NO_PAD.encode(buffer)
}

fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

fn normalize_callback_path(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

fn sanitize_return_to(value: Option<&str>) -> Result<Option<String>, OberonError> {
    let Some(raw) = value.map(str::trim) else {
        return Ok(None);
    };
    if raw.is_empty() {
        return Ok(None);
    }
    let lower = raw.to_ascii_lowercase();
    let valid_scheme = lower.starts_with("http://") || lower.starts_with("https://");
    let localhost = lower.contains("://localhost") || lower.contains("://127.0.0.1");
    if valid_scheme && localhost {
        Ok(Some(raw.to_string()))
    } else {
        Err(OberonError::Validation(
            "oauth return_to must target localhost over http or https".to_string(),
        ))
    }
}

fn url_encode(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn provider_name(provider: OAuthProviderKind) -> &'static str {
    match provider {
        OAuthProviderKind::OpenAi => "openai",
        OAuthProviderKind::Anthropic => "anthropic",
        OAuthProviderKind::Github => "github",
    }
}

fn provider_label(provider: OAuthProviderKind) -> &'static str {
    match provider {
        OAuthProviderKind::OpenAi => "OpenAI",
        OAuthProviderKind::Anthropic => "Anthropic",
        OAuthProviderKind::Github => "GitHub",
    }
}

fn provider_authorization_extras(provider: OAuthProviderKind) -> &'static str {
    match provider {
        OAuthProviderKind::OpenAi => {
            "&id_token_add_organizations=true&codex_cli_simplified_flow=true&originator=pi"
        }
        OAuthProviderKind::Anthropic => "",
        OAuthProviderKind::Github => "",
    }
}

fn load_github_app_secret<S: SecretStore>(
    store: &S,
    env_key: &str,
    store_key: &str,
    field_label: &str,
) -> Result<String, OberonError> {
    if let Ok(value) = std::env::var(env_key) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    match store.get_secret(store_key) {
        Ok(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(OberonError::Validation(format!(
            "github oauth requires {field_label} in {env_key} or host secret store key {store_key}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::{Read, Write},
        net::TcpListener,
        thread,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        AccountMetadataStore, AccountRegistry, AnthropicOAuthAdapter, FileAccountMetadataStore,
        GithubOAuthAdapter, InMemoryAccountMetadataStore, OAuthCallbackPayload, OAuthFlowManager,
        OAuthProviderConfig, OAuthTokenExchange, OpenAiOAuthAdapter, ReqwestOAuthTokenExchange,
        TokenBundle, store_provider_token,
    };
    use oberon4_core::{InMemorySecretStore, OAuthProvider, OAuthProviderKind, SecretStore};

    struct FakeExchange {
        bundle: TokenBundle,
    }

    impl OAuthTokenExchange for FakeExchange {
        fn exchange_code(
            &self,
            _provider: &super::OAuthProviderConfig,
            _code: &str,
            _code_verifier: &str,
            _redirect_uri: &str,
        ) -> Result<TokenBundle, oberon4_core::OberonError> {
            Ok(self.bundle.clone())
        }
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("oberon4-accounts-{name}-{nonce}.json"))
    }

    fn github_test_config() -> OAuthProviderConfig {
        OAuthProviderConfig {
            provider: OAuthProviderKind::Github,
            client_id: "github-client-id".to_string(),
            client_secret: Some("github-client-secret".to_string()),
            authorize_url: super::GITHUB_AUTHORIZE_URL.to_string(),
            token_url: super::GITHUB_TOKEN_URL.to_string(),
            redirect_base: "http://localhost:1455".to_string(),
            scope: super::GITHUB_SCOPE.to_string(),
            state_ttl: std::time::Duration::from_secs(600),
        }
    }

    #[test]
    fn auth_uses_secret_store_abstraction() {
        let mut store = InMemorySecretStore::default();
        store_provider_token(&mut store, "openai", "secret-token").expect("token should store");
        assert_eq!(
            store.get_secret("provider.openai.token").unwrap(),
            "secret-token"
        );
    }

    #[test]
    fn openai_flow_start_includes_pkce_state_and_local_redirect() {
        let manager = OAuthFlowManager::new(OAuthProviderConfig::openai());
        let (start, flow) = manager
            .start_flow("/auth/callback", Some("http://localhost:3000"), 100)
            .unwrap();

        assert_eq!(start.provider, OAuthProviderKind::OpenAi);
        assert!(
            start
                .authorization_url
                .contains(&format!("client_id={}", super::OPENAI_CLIENT_ID))
        );
        assert!(start.authorization_url.contains("code_challenge="));
        assert!(start.authorization_url.contains("state="));
        assert!(
            start
                .authorization_url
                .contains("id_token_add_organizations=true")
        );
        assert!(
            start
                .authorization_url
                .contains("codex_cli_simplified_flow=true")
        );
        assert!(start.authorization_url.contains("originator=pi"));
        assert!(
            start
                .authorization_url
                .contains("return_to=http%3A%2F%2Flocalhost%3A3000")
        );
        assert_eq!(flow.redirect_uri, "http://localhost:1455/auth/callback");
        assert_eq!(flow.expires_at_unix, 700);
    }

    #[test]
    fn anthropic_flow_start_uses_anthropic_urls() {
        let manager = OAuthFlowManager::new(OAuthProviderConfig::anthropic());
        let (start, _) = manager.start_flow("/callback", None, 50).unwrap();

        assert_eq!(start.provider, OAuthProviderKind::Anthropic);
        assert!(
            start
                .authorization_url
                .starts_with(super::ANTHROPIC_AUTHORIZE_URL)
        );
        assert!(
            start
                .authorization_url
                .contains("scope=org%3Acreate_api_key")
        );
    }

    #[test]
    fn github_flow_start_uses_github_urls() {
        let manager = OAuthFlowManager::new(github_test_config());
        let (start, _) = manager.start_flow("/auth/callback", None, 75).unwrap();

        assert_eq!(start.provider, OAuthProviderKind::Github);
        assert!(
            start
                .authorization_url
                .starts_with(super::GITHUB_AUTHORIZE_URL)
        );
        assert!(
            start
                .authorization_url
                .contains("client_id=github-client-id")
        );
        assert!(start.authorization_url.contains("scope=repo%20read%3Auser"));
    }

    #[test]
    fn oauth_adapters_report_correct_provider() {
        let openai = OpenAiOAuthAdapter::default();
        let anthropic = AnthropicOAuthAdapter::default();
        let github = GithubOAuthAdapter::from_config(github_test_config());

        assert_eq!(openai.provider_kind(), OAuthProviderKind::OpenAi);
        assert_eq!(anthropic.provider_kind(), OAuthProviderKind::Anthropic);
        assert_eq!(github.provider_kind(), OAuthProviderKind::Github);
        let (openai_start, _) = openai.start_connect_flow().unwrap();
        let (anthropic_start, _) = anthropic.start_connect_flow().unwrap();
        let (github_start, _) = github.start_connect_flow().unwrap();
        assert!(openai_start.authorization_url.contains("auth.openai.com"));
        assert!(
            anthropic_start
                .authorization_url
                .contains("console.anthropic.com")
        );
        assert!(
            github_start
                .authorization_url
                .contains("github.com/login/oauth/authorize")
        );
    }

    #[test]
    fn github_config_falls_back_to_host_secret_store() {
        let secret_path = temp_path("github-oauth-secrets");
        let mut store = oberon4_core::FileSecretStore::new(&secret_path);
        store
            .set_secret(super::GITHUB_CLIENT_ID_STORE_KEY, "github-client-id")
            .unwrap();
        store
            .set_secret(
                super::GITHUB_CLIENT_SECRET_STORE_KEY,
                "github-client-secret",
            )
            .unwrap();

        let config = OAuthProviderConfig::github_from_store(&store).unwrap();
        assert_eq!(config.client_id, "github-client-id");
        assert_eq!(
            config.client_secret.as_deref(),
            Some("github-client-secret")
        );

        let _ = std::fs::remove_file(secret_path);
    }

    #[test]
    fn completing_openai_flow_persists_tokens_and_registers_account() {
        let manager = OAuthFlowManager::new(OAuthProviderConfig::openai());
        let (_start, flow) = manager.start_flow("/auth/callback", None, 100).unwrap();
        let exchange = FakeExchange {
            bundle: TokenBundle {
                access_token: "access-openai".to_string(),
                refresh_token: Some("refresh-openai".to_string()),
                expires_at_unix: 999,
                account_id: "acct-openai".to_string(),
                label: "OpenAI Primary".to_string(),
            },
        };
        let mut registry = AccountRegistry::default();
        let mut store = InMemorySecretStore::default();

        let account = manager
            .complete_flow(
                &flow,
                OAuthCallbackPayload {
                    state: flow.oauth_state.clone(),
                    code: "code-1".to_string(),
                    return_to: Some("http://localhost:3000".to_string()),
                },
                &exchange,
                &mut registry,
                &mut store,
                200,
            )
            .unwrap();

        assert_eq!(account.id, "acct-openai");
        assert_eq!(
            store
                .get_secret("oauth.openai.acct-openai.access_token")
                .unwrap(),
            "access-openai"
        );
        assert_eq!(
            registry.active_account_id(OAuthProviderKind::OpenAi),
            Some("acct-openai")
        );
    }

    #[test]
    fn completing_anthropic_flow_registers_account() {
        let manager = OAuthFlowManager::new(OAuthProviderConfig::anthropic());
        let (_start, flow) = manager.start_flow("/callback", None, 100).unwrap();
        let exchange = FakeExchange {
            bundle: TokenBundle {
                access_token: "access-anthropic".to_string(),
                refresh_token: None,
                expires_at_unix: 999,
                account_id: "acct-anthropic".to_string(),
                label: "Anthropic Primary".to_string(),
            },
        };
        let mut registry = AccountRegistry::default();
        let mut store = InMemorySecretStore::default();

        let account = manager
            .complete_flow(
                &flow,
                OAuthCallbackPayload {
                    state: flow.oauth_state.clone(),
                    code: "code-2".to_string(),
                    return_to: Some("https://localhost:3000".to_string()),
                },
                &exchange,
                &mut registry,
                &mut store,
                300,
            )
            .unwrap();

        assert_eq!(account.provider, OAuthProviderKind::Anthropic);
        assert_eq!(
            store
                .get_secret("oauth.anthropic.acct-anthropic.access_token")
                .unwrap(),
            "access-anthropic"
        );
    }

    #[test]
    fn callback_state_mismatch_is_rejected() {
        let manager = OAuthFlowManager::new(OAuthProviderConfig::openai());
        let (_start, flow) = manager.start_flow("/auth/callback", None, 100).unwrap();
        let exchange = FakeExchange {
            bundle: TokenBundle {
                access_token: "secret-access".to_string(),
                refresh_token: Some("secret-refresh".to_string()),
                expires_at_unix: 999,
                account_id: "acct-openai".to_string(),
                label: "OpenAI Primary".to_string(),
            },
        };
        let mut registry = AccountRegistry::default();
        let mut store = InMemorySecretStore::default();

        let err = manager
            .complete_flow(
                &flow,
                OAuthCallbackPayload {
                    state: "wrong-state".to_string(),
                    code: "code".to_string(),
                    return_to: None,
                },
                &exchange,
                &mut registry,
                &mut store,
                200,
            )
            .expect_err("state mismatch should fail");

        assert_eq!(
            err.to_string(),
            "validation failed: oauth callback state mismatch"
        );
        assert!(!err.to_string().contains("secret-access"));
        assert!(!err.to_string().contains("secret-refresh"));
    }

    #[test]
    fn expired_state_is_rejected() {
        let manager = OAuthFlowManager::new(OAuthProviderConfig::openai());
        let (_start, flow) = manager.start_flow("/auth/callback", None, 100).unwrap();
        let exchange = FakeExchange {
            bundle: TokenBundle {
                access_token: "access".to_string(),
                refresh_token: None,
                expires_at_unix: 999,
                account_id: "acct-openai".to_string(),
                label: "OpenAI Primary".to_string(),
            },
        };
        let mut registry = AccountRegistry::default();
        let mut store = InMemorySecretStore::default();

        let err = manager
            .complete_flow(
                &flow,
                OAuthCallbackPayload {
                    state: flow.oauth_state.clone(),
                    code: "code".to_string(),
                    return_to: None,
                },
                &exchange,
                &mut registry,
                &mut store,
                701,
            )
            .expect_err("expired state should fail");

        assert_eq!(err.to_string(), "validation failed: oauth state expired");
    }

    #[test]
    fn non_local_return_to_is_rejected() {
        let manager = OAuthFlowManager::new(OAuthProviderConfig::openai());
        let err = manager
            .start_flow("/auth/callback", Some("https://example.com/callback"), 100)
            .expect_err("non-local return_to should fail");

        assert_eq!(
            err.to_string(),
            "validation failed: oauth return_to must target localhost over http or https"
        );
    }

    #[test]
    fn account_registry_supports_activate_disable_and_disconnect() {
        let mut registry = AccountRegistry::default();
        let mut store = InMemorySecretStore::default();
        let bundle = TokenBundle {
            access_token: "access-openai".to_string(),
            refresh_token: None,
            expires_at_unix: 999,
            account_id: "acct-openai".to_string(),
            label: "OpenAI Primary".to_string(),
        };

        registry
            .connect(OAuthProviderKind::OpenAi, &bundle, &mut store)
            .unwrap();
        registry
            .disable(OAuthProviderKind::OpenAi, "acct-openai")
            .unwrap();
        assert_eq!(registry.active_account_id(OAuthProviderKind::OpenAi), None);

        registry
            .activate(OAuthProviderKind::OpenAi, "acct-openai")
            .unwrap();
        assert_eq!(
            registry.active_account_id(OAuthProviderKind::OpenAi),
            Some("acct-openai")
        );

        registry
            .disconnect(OAuthProviderKind::OpenAi, "acct-openai")
            .unwrap();
        assert!(registry.list().is_empty());
    }

    #[test]
    fn in_memory_account_store_round_trips_registry() {
        let mut store = InMemoryAccountMetadataStore::default();
        let mut registry = AccountRegistry::default();
        let mut secrets = InMemorySecretStore::default();
        let bundle = TokenBundle {
            access_token: "access-openai".to_string(),
            refresh_token: Some("refresh-openai".to_string()),
            expires_at_unix: 999,
            account_id: "acct-openai".to_string(),
            label: "OpenAI Primary".to_string(),
        };

        registry
            .connect(OAuthProviderKind::OpenAi, &bundle, &mut secrets)
            .unwrap();
        store.save_registry(&registry).unwrap();

        let loaded = store.load_registry().unwrap();
        assert_eq!(loaded.list(), registry.list());
        assert_eq!(
            loaded.active_account_id(OAuthProviderKind::OpenAi),
            Some("acct-openai")
        );
    }

    #[test]
    fn file_account_store_round_trips_registry() {
        let path = temp_path("file");
        let mut store = FileAccountMetadataStore::new(&path);
        let mut registry = AccountRegistry::default();
        let mut secrets = InMemorySecretStore::default();
        let bundle = TokenBundle {
            access_token: "access-anthropic".to_string(),
            refresh_token: None,
            expires_at_unix: 999,
            account_id: "acct-anthropic".to_string(),
            label: "Anthropic Primary".to_string(),
        };

        registry
            .connect(OAuthProviderKind::Anthropic, &bundle, &mut secrets)
            .unwrap();
        store.save_registry(&registry).unwrap();

        let loaded = store.load_registry().unwrap();
        assert_eq!(loaded.list(), registry.list());
        assert_eq!(
            loaded.active_account_id(OAuthProviderKind::Anthropic),
            Some("acct-anthropic")
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn reqwest_exchange_posts_openai_form_to_local_server() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let addr = listener.local_addr().expect("local addr");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept should succeed");
            let mut buffer = [0_u8; 4096];
            let bytes_read = stream.read(&mut buffer).expect("request should read");
            let request = String::from_utf8_lossy(&buffer[..bytes_read]).to_string();
            assert!(request.starts_with("POST /token HTTP/1.1"));
            assert!(request.contains("grant_type=authorization_code"));
            assert!(request.contains(&format!("client_id={}", super::OPENAI_CLIENT_ID)));
            assert!(request.contains("code=code-123"));
            assert!(request.contains("code_verifier=verifier-123"));

            let body = r#"{"access_token":"access-openai","refresh_token":"refresh-openai","expires_in":1200,"account_id":"acct-openai","label":"OpenAI Primary"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("response should write");
        });

        let provider = OAuthProviderConfig {
            token_url: format!("http://{}/token", addr),
            ..OAuthProviderConfig::openai()
        };
        let exchange = ReqwestOAuthTokenExchange;

        let token = exchange
            .exchange_code(
                &provider,
                "code-123",
                "verifier-123",
                "http://localhost:1455/auth/callback",
            )
            .expect("real exchange should succeed");

        server.join().expect("server thread should finish");
        assert_eq!(token.access_token, "access-openai");
        assert_eq!(token.refresh_token.as_deref(), Some("refresh-openai"));
        assert_eq!(token.account_id, "acct-openai");
    }

    #[test]
    fn reqwest_exchange_posts_anthropic_form_to_local_server() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let addr = listener.local_addr().expect("local addr");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept should succeed");
            let mut buffer = [0_u8; 4096];
            let bytes_read = stream.read(&mut buffer).expect("request should read");
            let request = String::from_utf8_lossy(&buffer[..bytes_read]).to_string();
            assert!(request.starts_with("POST /token HTTP/1.1"));
            assert!(request.contains("client_id=anthropic-console"));
            assert!(request.contains("code=anthropic-code"));

            let body = r#"{"access_token":"access-anthropic","expires_in":900,"account_id":"acct-anthropic","label":"Anthropic Primary"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("response should write");
        });

        let provider = OAuthProviderConfig {
            token_url: format!("http://{}/token", addr),
            ..OAuthProviderConfig::anthropic()
        };
        let exchange = ReqwestOAuthTokenExchange;

        let token = exchange
            .exchange_code(
                &provider,
                "anthropic-code",
                "anthropic-verifier",
                "http://localhost:1455/callback",
            )
            .expect("real exchange should succeed");

        server.join().expect("server thread should finish");
        assert_eq!(token.access_token, "access-anthropic");
        assert!(token.refresh_token.is_none());
        assert_eq!(token.account_id, "acct-anthropic");
    }
}
