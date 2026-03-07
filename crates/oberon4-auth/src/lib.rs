use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use oberon4_core::{
    AccountState, OAuthProvider, OAuthProviderKind, OberonError, SecretStore, StoredAccount,
};

pub const OPENAI_CLIENT_ID: &str = "openai-desktop";
pub const OPENAI_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
pub const OPENAI_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const OPENAI_SCOPE: &str = "openid profile email offline_access";

pub const ANTHROPIC_CLIENT_ID: &str = "anthropic-console";
pub const ANTHROPIC_AUTHORIZE_URL: &str = "https://console.anthropic.com/v1/oauth/authorize";
pub const ANTHROPIC_TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";
pub const ANTHROPIC_SCOPE: &str = "org:create_api_key user:profile user:inference";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthProviderConfig {
    pub provider: OAuthProviderKind,
    pub client_id: String,
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
            authorize_url: ANTHROPIC_AUTHORIZE_URL.to_string(),
            token_url: ANTHROPIC_TOKEN_URL.to_string(),
            redirect_base: "http://localhost:1455".to_string(),
            scope: ANTHROPIC_SCOPE.to_string(),
            state_ttl: Duration::from_secs(600),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingOAuthFlow {
    pub flow_id: String,
    pub provider: OAuthProviderKind,
    pub oauth_state: String,
    pub code_verifier: String,
    pub redirect_uri: String,
    pub expires_at_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthStart {
    pub flow_id: String,
    pub provider: OAuthProviderKind,
    pub authorization_url: String,
    pub redirect_uri: String,
    pub expires_at_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthCallbackPayload {
    pub state: String,
    pub code: String,
    pub return_to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenBundle {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at_unix: u64,
    pub account_id: String,
    pub label: String,
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

#[derive(Debug, Default)]
pub struct AccountRegistry {
    accounts: Vec<StoredAccount>,
    active_accounts: BTreeMap<OAuthProviderKind, String>,
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
            "{}?client_id={}&response_type=code&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256{}",
            self.config.authorize_url,
            url_encode(&self.config.client_id),
            url_encode(&redirect_uri),
            url_encode(&self.config.scope),
            url_encode(&oauth_state),
            url_encode(&code_challenge),
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

    fn start_connect_flow(&self) -> Result<String, OberonError> {
        let (start, _) = self.inner.start_flow("/auth/callback", None, now_unix())?;
        Ok(start.authorization_url)
    }

    fn exchange_callback(&self, code: &str, state: &str) -> Result<StoredAccount, OberonError> {
        Ok(StoredAccount {
            id: format!("openai-{state}"),
            provider: OAuthProviderKind::OpenAi,
            label: format!("OpenAI {code}"),
            state: AccountState::Active,
        })
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

impl AnthropicOAuthAdapter {
    pub fn flow_manager(&self) -> &OAuthFlowManager {
        &self.inner
    }
}

impl OAuthProvider for AnthropicOAuthAdapter {
    fn provider_kind(&self) -> OAuthProviderKind {
        OAuthProviderKind::Anthropic
    }

    fn start_connect_flow(&self) -> Result<String, OberonError> {
        let (start, _) = self.inner.start_flow("/callback", None, now_unix())?;
        Ok(start.authorization_url)
    }

    fn exchange_callback(&self, code: &str, state: &str) -> Result<StoredAccount, OberonError> {
        Ok(StoredAccount {
            id: format!("anthropic-{state}"),
            provider: OAuthProviderKind::Anthropic,
            label: format!("Anthropic {code}"),
            state: AccountState::Active,
        })
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
    let provider_id = match provider {
        OAuthProviderKind::OpenAi => "openai",
        OAuthProviderKind::Anthropic => "anthropic",
    };
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

#[cfg(test)]
mod tests {
    use super::{
        AccountRegistry, AnthropicOAuthAdapter, OAuthCallbackPayload, OAuthFlowManager,
        OAuthProviderConfig, OAuthTokenExchange, OpenAiOAuthAdapter, TokenBundle,
        store_provider_token,
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
        assert!(start.authorization_url.contains("code_challenge="));
        assert!(start.authorization_url.contains("state="));
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
    fn openai_and_anthropic_adapters_report_correct_provider() {
        let openai = OpenAiOAuthAdapter::default();
        let anthropic = AnthropicOAuthAdapter::default();

        assert_eq!(openai.provider_kind(), OAuthProviderKind::OpenAi);
        assert_eq!(anthropic.provider_kind(), OAuthProviderKind::Anthropic);
        assert!(
            openai
                .start_connect_flow()
                .unwrap()
                .contains("auth.openai.com")
        );
        assert!(
            anthropic
                .start_connect_flow()
                .unwrap()
                .contains("console.anthropic.com")
        );
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
}
