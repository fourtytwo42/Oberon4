use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    Json,
};
use llm_auth::OAuthCallbackPayload;
use llm_foundation::contracts::ErrorCode;
use llm_storage::{OAuthAccountRecord, SqliteStore, StoredTokenRecord};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use url::Url;

use super::AppState;

// ─── Structs / Enums ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BrowserFlowState {
    Pending,
    Completed,
    Failed,
    Expired,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct BrowserFlowStatus {
    pub(crate) flow_id: String,
    pub(crate) state: BrowserFlowState,
    pub(crate) expires_at_unix: u64,
    pub(crate) authorization_url: Option<String>,
    pub(crate) account_id: Option<String>,
    pub(crate) error: Option<String>,
}

#[derive(Debug)]
pub(crate) struct BrowserFlowInner {
    pub(crate) flow_id: String,
    pub(crate) provider_id: String,
    pub(crate) expires_at_unix: u64,
    pub(crate) status: BrowserFlowStatus,
    pub(crate) oauth_state: String,
    pub(crate) redirect_uri: String,
    pub(crate) callback_payload: Option<OAuthCallbackPayload>,
    pub(crate) callback_server: Option<llm_auth::OAuthCallbackServer>,
    pub(crate) return_to: Option<String>,
    pub(crate) display_name_hint: Option<String>,
}

#[derive(Clone, Default)]
pub(crate) struct BrowserFlowStore {
    pub(crate) inner: Arc<std::sync::Mutex<HashMap<String, BrowserFlowInner>>>,
}

#[derive(Debug, Clone)]
pub(crate) struct AnthropicPendingFlow {
    pub(crate) flow_id: String,
    pub(crate) code_verifier: String,
    pub(crate) oauth_state: String,
    pub(crate) expires_at_unix: u64,
    pub(crate) display_name_hint: Option<String>,
    /// None means use the legacy ANTHROPIC_REDIRECT_URI constant.
    pub(crate) redirect_uri: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct GooglePendingFlow {
    pub(crate) flow_id: String,
    pub(crate) code_verifier: String,
    pub(crate) oauth_state: String,
    pub(crate) redirect_uri: String,
    pub(crate) expires_at_unix: u64,
    pub(crate) display_name_hint: Option<String>,
}

#[derive(Clone, Default)]
pub(crate) struct AnthropicFlowStore {
    pub(crate) inner: Arc<std::sync::Mutex<HashMap<String, AnthropicPendingFlow>>>,
}

#[derive(Clone, Default)]
pub(crate) struct GoogleFlowStore {
    pub(crate) inner: Arc<std::sync::Mutex<HashMap<String, GooglePendingFlow>>>,
}

#[derive(Clone, Default)]
pub(crate) struct GoogleThoughtSignatureStore {
    pub(crate) inner: Arc<std::sync::Mutex<HashMap<String, String>>>,
}

// ─── impl blocks ──────────────────────────────────────────────────────────────

impl AnthropicFlowStore {
    pub(crate) fn insert(&self, flow: AnthropicPendingFlow) {
        let mut m = self.inner.lock().expect("anthropic flow mutex poisoned");
        let now = super::now_unix();
        m.retain(|_, v| v.expires_at_unix > now);
        m.insert(flow.flow_id.clone(), flow);
    }
    pub(crate) fn take(&self, flow_id: &str) -> Option<AnthropicPendingFlow> {
        self.inner
            .lock()
            .expect("anthropic flow mutex poisoned")
            .remove(flow_id)
    }
}

impl GoogleThoughtSignatureStore {
    pub(crate) fn fingerprint(name: &str, args: &Value) -> String {
        let args_json = serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string());
        format!("fp:{name}:{args_json}")
    }

    pub(crate) fn put(&self, call_id: &str, name: &str, args: &Value, signature: &str) {
        if signature.trim().is_empty() {
            return;
        }
        let mut m = self.inner.lock().expect("google signature mutex poisoned");
        // Bound growth for long-running sessions.
        if m.len() > 4096 {
            m.clear();
        }
        if !call_id.trim().is_empty() {
            m.insert(format!("call:{call_id}"), signature.to_string());
        }
        if !name.trim().is_empty() {
            m.insert(Self::fingerprint(name, args), signature.to_string());
        }
    }

    pub(crate) fn get(&self, call_id: &str) -> Option<String> {
        self.inner
            .lock()
            .expect("google signature mutex poisoned")
            .get(&format!("call:{call_id}"))
            .cloned()
    }

    pub(crate) fn get_for_call_or_fingerprint(
        &self,
        call_id: &str,
        name: &str,
        args: &Value,
    ) -> Option<String> {
        if let Some(sig) = self.get(call_id) {
            return Some(sig);
        }
        self.inner
            .lock()
            .expect("google signature mutex poisoned")
            .get(&Self::fingerprint(name, args))
            .cloned()
    }
}

impl BrowserFlowStore {
    pub(crate) fn insert(&self, flow: BrowserFlowInner) {
        self.inner
            .lock()
            .expect("browser flow mutex poisoned")
            .insert(flow.flow_id.clone(), flow);
    }

    pub(crate) fn get(&self, flow_id: &str) -> Option<BrowserFlowStatus> {
        self.inner
            .lock()
            .expect("browser flow mutex poisoned")
            .get(flow_id)
            .map(|v| v.status.clone())
    }

    pub(crate) fn refresh(&self, flow_id: &str, now: u64) {
        let mut guard = self.inner.lock().expect("browser flow mutex poisoned");
        let Some(flow) = guard.get_mut(flow_id) else {
            return;
        };
        if flow.expires_at_unix <= now {
            flow.status.state = BrowserFlowState::Expired;
            flow.status.authorization_url = None;
            flow.status
                .error
                .get_or_insert("oauth flow expired".to_string());
            flow.callback_server = None;
            return;
        }
        if flow.callback_payload.is_some() {
            return;
        }
        let Some(server) = flow.callback_server.as_mut() else {
            return;
        };
        match server.callback_rx.try_recv() {
            Ok(result) => {
                flow.callback_server = None;
                match result {
                    Ok(payload) => {
                        flow.callback_payload = Some(payload);
                    }
                    Err(err) => {
                        flow.status.state = BrowserFlowState::Failed;
                        flow.status.error = Some(err.to_string());
                        flow.status.authorization_url = None;
                    }
                }
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                flow.callback_server = None;
                flow.status.state = BrowserFlowState::Failed;
                flow.status.error = Some("callback listener closed".to_string());
                flow.status.authorization_url = None;
            }
        }
    }

    pub(crate) fn take_callback(
        &self,
        flow_id: &str,
    ) -> Option<(OAuthCallbackPayload, String, Option<String>, String, String)> {
        let mut guard = self.inner.lock().expect("browser flow mutex poisoned");
        let flow = guard.get_mut(flow_id)?;
        let payload = flow.callback_payload.take()?;
        Some((
            payload,
            flow.redirect_uri.clone(),
            flow.display_name_hint.clone(),
            flow.provider_id.clone(),
            flow.flow_id.clone(),
        ))
    }

    pub(crate) fn set_result(&self, flow_id: &str, result: Result<String, String>) {
        let mut guard = self.inner.lock().expect("browser flow mutex poisoned");
        let Some(flow) = guard.get_mut(flow_id) else {
            return;
        };
        flow.callback_server = None;
        match result {
            Ok(account_id) => {
                flow.status.state = BrowserFlowState::Completed;
                flow.status.account_id = Some(account_id);
                flow.status.authorization_url = None;
                flow.status.error = None;
            }
            Err(err) => {
                flow.status.state = BrowserFlowState::Failed;
                flow.status.error = Some(err);
                flow.status.authorization_url = None;
            }
        }
    }

    /// If a pending flow already exists, extend its expiry and update metadata.
    pub(crate) fn extend_pending(
        &self,
        extend_secs: u64,
        display_name: Option<String>,
        return_to: Option<String>,
    ) -> Option<(String, String, u64, String)> {
        let mut guard = self.inner.lock().expect("browser flow mutex poisoned");
        let flow = guard
            .values_mut()
            .find(|f| matches!(f.status.state, BrowserFlowState::Pending))?;
        let new_expires = super::now_unix().saturating_add(extend_secs);
        flow.expires_at_unix = new_expires;
        flow.status.expires_at_unix = new_expires;
        if let Some(name) = display_name.filter(|s| !s.is_empty()) {
            flow.display_name_hint = Some(name);
        }
        if return_to.is_some() {
            flow.return_to = return_to;
        }
        let auth_url = flow.status.authorization_url.clone()?;
        Some((
            flow.flow_id.clone(),
            auth_url,
            new_expires,
            flow.redirect_uri.clone(),
        ))
    }

    /// Find a pending flow by OAuth `state` and attach callback payload or error.
    pub(crate) fn set_callback_for_state(
        &self,
        oauth_state: &str,
        payload: Result<OAuthCallbackPayload, String>,
    ) -> Option<BrowserFlowStatus> {
        let mut guard = self.inner.lock().expect("browser flow mutex poisoned");
        let flow = guard.values_mut().find(|f| {
            matches!(f.status.state, BrowserFlowState::Pending) && f.oauth_state == oauth_state
        })?;
        match payload {
            Ok(v) => {
                flow.callback_payload = Some(v);
            }
            Err(err) => {
                flow.status.state = BrowserFlowState::Failed;
                flow.status.error = Some(err);
                flow.status.authorization_url = None;
            }
        }
        Some(flow.status.clone())
    }

    pub(crate) fn return_to_for_state(&self, oauth_state: &str) -> Option<String> {
        self.inner
            .lock()
            .expect("browser flow mutex poisoned")
            .values()
            .find(|f| f.oauth_state == oauth_state)
            .and_then(|f| f.return_to.clone())
    }
}

impl GoogleFlowStore {
    pub(crate) fn insert(&self, flow: GooglePendingFlow) {
        let mut m = self.inner.lock().expect("google flow mutex poisoned");
        let now = super::now_unix();
        m.retain(|_, v| v.expires_at_unix > now);
        m.insert(flow.flow_id.clone(), flow);
    }
    pub(crate) fn take(&self, flow_id: &str) -> Option<GooglePendingFlow> {
        self.inner
            .lock()
            .expect("google flow mutex poisoned")
            .remove(flow_id)
    }
}

// ─── OAuth Error Classification ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OAuthErrorClass {
    StateMismatch,
    InvalidGrant,
    TokenExpired,
    RateLimited,
    UpstreamTransient,
    UpstreamPermanent,
    Storage,
}

impl OAuthErrorClass {
    pub(crate) fn as_tag(self) -> &'static str {
        match self {
            Self::StateMismatch => "state_mismatch",
            Self::InvalidGrant => "invalid_grant",
            Self::TokenExpired => "token_expired",
            Self::RateLimited => "rate_limited",
            Self::UpstreamTransient => "upstream_transient",
            Self::UpstreamPermanent => "upstream_permanent",
            Self::Storage => "storage",
        }
    }
}

// ─── Constants ────────────────────────────────────────────────────────────────

pub(crate) const ANTHROPIC_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
pub(crate) const ANTHROPIC_AUTH_URL: &str = "https://claude.ai/oauth/authorize";
pub(crate) const ANTHROPIC_TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";
pub(crate) const ANTHROPIC_REDIRECT_URI: &str = "https://console.anthropic.com/oauth/code/callback";
pub(crate) const ANTHROPIC_SCOPE: &str =
    "user:profile user:inference user:sessions:claude_code user:mcp_servers";
pub(crate) const ANTHROPIC_UPSTREAM: &str = "https://api.anthropic.com";
pub(crate) const ANTHROPIC_OAUTH_BETA_HEADER: &str = "claude-code-20250219,oauth-2025-04-20";
pub(crate) const GOOGLE_PROVIDER_ID: &str = "google";
pub(crate) const OPENAI_PROVIDER_ID: &str = "openai";
pub(crate) const OPENAI_API_PROVIDER_ID: &str = "openai_api";
pub(crate) const ANTHROPIC_PROVIDER_ID: &str = "anthropic";
pub(crate) const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub(crate) const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub(crate) const GOOGLE_USERINFO_URL: &str =
    "https://www.googleapis.com/oauth2/v1/userinfo?alt=json";
pub(crate) const GOOGLE_CODE_ASSIST_DAILY: &str =
    "https://daily-cloudcode-pa.sandbox.googleapis.com";
pub(crate) const GOOGLE_CODE_ASSIST_AUTOPUSH: &str =
    "https://autopush-cloudcode-pa.sandbox.googleapis.com";
pub(crate) const GOOGLE_CODE_ASSIST_PROD: &str = "https://cloudcode-pa.googleapis.com";
pub(crate) const GOOGLE_ANTIGRAVITY_CLIENT_ID: &str =
    "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
pub(crate) const GOOGLE_ANTIGRAVITY_CLIENT_SECRET: &str = "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";
pub(crate) const GOOGLE_API_CLIENT: &str = "google-cloud-sdk vscode_cloudshelleditor/0.1";
pub(crate) const GOOGLE_CLIENT_METADATA: &str =
    "{\"ideType\":\"IDE_UNSPECIFIED\",\"platform\":\"PLATFORM_UNSPECIFIED\",\"pluginType\":\"GEMINI\"}";
// Cold-cache fallback only — live list is fetched from api.anthropic.com/v1/models
// and cached in the store on each UI load via anthropic_oauth_models.
pub(crate) const ANTHROPIC_KNOWN_MODELS: &[&str] = &[
    "claude-opus-4-6",
    "claude-sonnet-4-6",
    "claude-opus-4-5",
    "claude-sonnet-4-5",
    "claude-haiku-4-5",
    "claude-haiku-4-5-20251001",
    "claude-3-7-sonnet-20250219",
    "claude-3-5-sonnet-20241022",
    "claude-3-5-haiku-20241022",
    "claude-3-opus-20240229",
];
pub(crate) const GOOGLE_KNOWN_MODELS: &[&str] = &[
    "gemini-2.5-flash",
    "gemini-2.5-flash-lite",
    "gemini-3-pro-preview",
    "gemini-3-pro-high",
    "gemini-3-pro-low",
    "gemini-3-flash",
    "gemini-3-flash-preview",
];
pub(crate) const GOOGLE_ANTIGRAVITY_BASE_SYSTEM_INSTRUCTION: &str = "You are Antigravity, a powerful agentic AI coding assistant designed by the Google Deepmind team working on Advanced Agentic Coding.You are pair programming with a USER to solve their coding task. The task may require creating a new codebase, modifying or debugging an existing codebase, or simply answering a question.**Absolute paths only****Proactiveness**";
pub(crate) const GOOGLE_VERIFY_CACHE_TTL_SECS: u64 = 600;
pub(crate) const GOOGLE_VERIFY_MAX_CONCURRENCY: usize = 4;
pub(crate) const GOOGLE_VERIFY_EXTRA_MODELS: &[&str] = &[
    "gemini-3-flash-preview",
    "gemini-3.1-pro-preview",
    "gemini-3-pro-high",
    "gemini-3-pro-low",
    "gemini-3.1-pro-high",
    "gemini-3.1-pro-low",
    "gemini-claude-sonnet-4-6-thinking",
    "gemini-claude-opus-4-6-thinking",
    "gpt-oss-120b-medium",
    "gemini-gpt-oss-120b-medium",
];

// ─── Request / Response DTOs ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct AnthropicStartRequest {
    pub(crate) display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AnthropicCompleteRequest {
    pub(crate) flow_id: String,
    pub(crate) code: String,
    pub(crate) state: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GoogleStartRequest {
    pub(crate) display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GoogleCompleteRequest {
    pub(crate) flow_id: String,
    pub(crate) code: String,
    pub(crate) state: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GoogleTokenResponse {
    pub(crate) access_token: Option<String>,
    pub(crate) refresh_token: Option<String>,
    pub(crate) expires_in: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OAuthBrowserStartRequest {
    pub(crate) callback_path: Option<String>,
    pub(crate) display_name: Option<String>,
    pub(crate) provider: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OAuthBrowserCallbackQuery {
    #[serde(default)]
    pub(crate) state: String,
    #[serde(default)]
    pub(crate) code: String,
    #[serde(default)]
    pub(crate) error: Option<String>,
    #[serde(default)]
    pub(crate) return_to: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GoogleModelVerifyQuery {
    pub(crate) refresh: Option<String>,
    pub(crate) models: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct GoogleModelVerifyResult {
    pub(crate) requested_model: String,
    pub(crate) resolved_model: String,
    pub(crate) status: String,
    pub(crate) available: bool,
    pub(crate) http_status: Option<u16>,
    pub(crate) endpoint: Option<String>,
    pub(crate) reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct GoogleModelVerifyCache {
    pub(crate) verified_at_unix: u64,
    pub(crate) ttl_seconds: u64,
    pub(crate) data: Vec<GoogleModelVerifyResult>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct GoogleProjectContext {
    pub(crate) project_id: String,
    pub(crate) managed_project_id: String,
    pub(crate) effective_project_id: String,
    pub(crate) tier_id: Option<String>,
    pub(crate) paid_tier_id: Option<String>,
    pub(crate) plan_type: Option<String>,
}

// ─── Pure Helper Functions ────────────────────────────────────────────────────

pub(crate) fn is_anthropic_oauth_token(token: &str) -> bool {
    token.contains("sk-ant-oat")
}

pub(crate) fn base64url_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 2 < bytes.len() {
        let b0 = bytes[i] as u32;
        let b1 = bytes[i + 1] as u32;
        let b2 = bytes[i + 2] as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(TABLE[((n >> 6) & 63) as usize] as char);
        out.push(TABLE[(n & 63) as usize] as char);
        i += 3;
    }
    let rem = bytes.len() - i;
    if rem == 2 {
        let b0 = bytes[i] as u32;
        let b1 = bytes[i + 1] as u32;
        let n = (b0 << 16) | (b1 << 8);
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(TABLE[((n >> 6) & 63) as usize] as char);
    } else if rem == 1 {
        let b0 = bytes[i] as u32;
        let n = b0 << 16;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
    }
    out
}

pub(crate) fn pkce_pair() -> (String, String) {
    // verifier: two UUID4s concatenated without hyphens = 64 hex chars (satisfies PKCE 43-128 char req)
    let verifier = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    // challenge: base64url(SHA256(verifier)) — no padding
    let digest = Sha256::digest(verifier.as_bytes());
    let challenge = base64url_encode(digest.as_slice());
    (verifier, challenge)
}

pub(crate) fn oauth_error_message(class: OAuthErrorClass, message: impl AsRef<str>) -> String {
    format!(
        "{} (oauth_error_class={})",
        message.as_ref(),
        class.as_tag()
    )
}

pub(crate) fn classify_oauth_http_error(status: StatusCode, body_text: &str) -> OAuthErrorClass {
    let lower = body_text.to_ascii_lowercase();
    if status == StatusCode::TOO_MANY_REQUESTS {
        return OAuthErrorClass::RateLimited;
    }
    if lower.contains("invalid_grant") || lower.contains("invalid grant") {
        return OAuthErrorClass::InvalidGrant;
    }
    if lower.contains("expired") && lower.contains("token") {
        return OAuthErrorClass::TokenExpired;
    }
    if status.is_server_error() {
        return OAuthErrorClass::UpstreamTransient;
    }
    OAuthErrorClass::UpstreamPermanent
}

pub(crate) fn oauth_retryable(class: OAuthErrorClass) -> bool {
    matches!(
        class,
        OAuthErrorClass::RateLimited
            | OAuthErrorClass::UpstreamTransient
            | OAuthErrorClass::TokenExpired
    )
}

pub(crate) fn oauth_backoff_delay_ms(attempt: u32) -> u64 {
    let exp = match attempt {
        0 => 150_u64,
        1 => 350_u64,
        _ => 700_u64,
    };
    let jitter = (super::now_unix().wrapping_mul(37) % 60) + 1;
    exp + jitter
}

pub(crate) async fn oauth_refresh_lock(
    state: Option<&AppState>,
    key: &str,
) -> Option<Arc<AsyncMutex<()>>> {
    let state = state?;
    let mut guard = state.oauth_refresh_locks.lock().await;
    Some(
        guard
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone(),
    )
}

pub(crate) fn redact_sensitive_text(input: &str) -> String {
    let mut out = input.to_string();
    for key in ["access_token", "refresh_token", "id_token", "authorization"] {
        for quote in ['"', '\''] {
            let marker = format!("{quote}{key}{quote}:");
            loop {
                let Some(start) = out.find(&marker) else {
                    break;
                };
                let tail = &out[start + marker.len()..];
                let Some(value_start_rel) = tail.find(quote) else {
                    break;
                };
                let value_start = start + marker.len() + value_start_rel + 1;
                let Some(value_end_rel) = out[value_start..].find(quote) else {
                    break;
                };
                let value_end = value_start + value_end_rel;
                out.replace_range(value_start..value_end, "***");
            }
        }
        let marker = format!("{key}=");
        loop {
            let Some(start) = out.to_ascii_lowercase().find(&marker) else {
                break;
            };
            let value_start = start + marker.len();
            let value_end = out[value_start..]
                .find(|c: char| c == '&' || c.is_whitespace() || c == '"')
                .map(|idx| value_start + idx)
                .unwrap_or_else(|| out.len());
            out.replace_range(value_start..value_end, "***");
        }
    }

    loop {
        let lower = out.to_ascii_lowercase();
        let Some(pos) = lower.find("bearer ") else {
            break;
        };
        let token_start = pos + 7;
        let token_end = out[token_start..]
            .find(|c: char| c.is_whitespace() || c == '"' || c == ',' || c == '}')
            .map(|idx| token_start + idx)
            .unwrap_or_else(|| out.len());
        out.replace_range(token_start..token_end, "***");
    }

    out
}

pub(crate) fn google_oauth_client_id() -> Option<String> {
    std::env::var("LLMHOTSPOT_GOOGLE_OAUTH_CLIENT_ID")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| Some(GOOGLE_ANTIGRAVITY_CLIENT_ID.to_string()))
}

pub(crate) fn google_oauth_client_secret() -> Option<String> {
    std::env::var("LLMHOTSPOT_GOOGLE_OAUTH_CLIENT_SECRET")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| Some(GOOGLE_ANTIGRAVITY_CLIENT_SECRET.to_string()))
}

pub(crate) fn google_oauth_redirect_uri() -> String {
    std::env::var("LLMHOTSPOT_GOOGLE_REDIRECT_URI")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "http://localhost:1455/auth/callback".to_string())
}

pub(crate) fn google_oauth_scope() -> String {
    std::env::var("LLMHOTSPOT_GOOGLE_SCOPE")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            "https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile https://www.googleapis.com/auth/cclog https://www.googleapis.com/auth/experimentsandconfigs".to_string()
        })
}

pub(crate) fn google_endpoint_chain() -> Vec<String> {
    vec![
        std::env::var("LLMHOTSPOT_GOOGLE_ENDPOINT_DAILY")
            .unwrap_or_else(|_| GOOGLE_CODE_ASSIST_DAILY.to_string()),
        std::env::var("LLMHOTSPOT_GOOGLE_ENDPOINT_AUTOPUSH")
            .unwrap_or_else(|_| GOOGLE_CODE_ASSIST_AUTOPUSH.to_string()),
        std::env::var("LLMHOTSPOT_GOOGLE_ENDPOINT_PROD")
            .unwrap_or_else(|_| GOOGLE_CODE_ASSIST_PROD.to_string()),
    ]
}

pub(crate) fn google_headers(access_token: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    if let Ok(v) = format!("Bearer {access_token}").parse() {
        h.insert(header::AUTHORIZATION, v);
    }
    h.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
    h.insert(
        "user-agent",
        "antigravity/1.15.8 linux/amd64".parse().unwrap(),
    );
    h.insert("x-goog-api-client", GOOGLE_API_CLIENT.parse().unwrap());
    h.insert("client-metadata", GOOGLE_CLIENT_METADATA.parse().unwrap());
    h
}

pub(crate) fn oauth_upstream_locked() -> bool {
    // Safety default: ChatGPT OAuth traffic must stay on Codex transport semantics.
    // See docs/OAUTH_CODEX_ENDPOINT_RULE.md.
    std::env::var("LLMHOTSPOT_OAUTH_UPSTREAM_LOCKED")
        .map(|v| v != "0")
        .unwrap_or(true)
}

// ─── Google Helper Functions ──────────────────────────────────────────────────

pub(crate) fn parse_google_project_id(body: &Value) -> String {
    body.get("cloudaicompanionProject")
        .and_then(Value::as_str)
        .or_else(|| {
            body.get("cloudaicompanionProject")
                .and_then(|v| v.get("id"))
                .and_then(Value::as_str)
        })
        .unwrap_or("")
        .to_string()
}

pub(crate) fn infer_google_plan(
    tier_id: Option<&str>,
    paid_tier_id: Option<&str>,
) -> Option<String> {
    let is_paid_tier = |id: &str| {
        let low = id.to_ascii_lowercase();
        !(low.contains("free") || low.contains("zero") || low == "legacy-tier")
    };
    if let Some(pid) = paid_tier_id {
        if is_paid_tier(pid) {
            return Some("paid".to_string());
        }
    }
    tier_id.map(|id| {
        if is_paid_tier(id) {
            "paid".to_string()
        } else {
            "free".to_string()
        }
    })
}

pub(crate) fn cached_verify_usable(cached: &GoogleModelVerifyCache) -> bool {
    if cached.data.is_empty() {
        return false;
    }
    // Ignore known-bad cache entries produced by older verifier payload shape.
    let all_legacy_invalid = cached.data.iter().all(|r| {
        r.status == "invalid_request"
            && r.reason
                .as_deref()
                .map(|s| {
                    s.contains("Unknown name \"contents\"")
                        || s.contains("Unknown name \"generationConfig\"")
                })
                .unwrap_or(false)
    });
    !all_legacy_invalid
}

pub(crate) fn parse_csv_models(raw: Option<&str>) -> Option<Vec<String>> {
    let raw = raw?;
    let out: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

pub(crate) fn parse_boolish(raw: Option<&str>) -> bool {
    matches!(
        raw.unwrap_or("").trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "t" | "yes" | "y" | "on"
    )
}

pub(crate) async fn probe_google_model(
    http: reqwest::Client,
    access_token: String,
    endpoints: Vec<String>,
    project_id: String,
    requested_model: String,
) -> GoogleModelVerifyResult {
    let resolved_model = super::resolve_google_model(&requested_model);
    let body = json!({
        "project": project_id,
        "model": resolved_model,
        "userAgent": "antigravity",
        "requestType": "agent",
        "requestId": uuid::Uuid::new_v4().to_string(),
        "request": {
            "sessionId": uuid::Uuid::new_v4().to_string(),
            "contents": [{"role":"user","parts":[{"text":"Reply with exactly: OK"}]}],
            "generationConfig": {"maxOutputTokens": 8, "thinkingConfig": {"includeThoughts": false}}
        }
    });

    let mut last_status: Option<u16> = None;
    let mut last_reason: Option<String> = None;
    let mut last_endpoint: Option<String> = None;

    for base in endpoints {
        let url = format!("{}/v1internal:generateContent", base.trim_end_matches('/'));
        let resp = match http
            .post(&url)
            .headers(google_headers(&access_token))
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                last_status = None;
                last_reason = Some(e.to_string());
                last_endpoint = Some(url.clone());
                continue;
            }
        };
        let status = resp.status();
        let status_u16 = status.as_u16();
        let v: Value = resp.json().await.unwrap_or_default();
        if status.is_success() {
            return GoogleModelVerifyResult {
                requested_model,
                resolved_model,
                status: "available".to_string(),
                available: true,
                http_status: Some(status_u16),
                endpoint: Some(url),
                reason: None,
            };
        }
        let reason = v
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
            .unwrap_or_else(|| {
                if status == StatusCode::NOT_FOUND {
                    "Requested entity was not found."
                } else if status == StatusCode::TOO_MANY_REQUESTS {
                    "Rate limited."
                } else {
                    "upstream failure"
                }
            })
            .to_string();
        last_status = Some(status_u16);
        last_reason = Some(reason);
        last_endpoint = Some(url);
    }

    let (status, available) = match last_status {
        Some(429) => ("rate_limited".to_string(), true),
        Some(404) => ("unavailable".to_string(), false),
        Some(401 | 403) => ("unauthorized".to_string(), false),
        Some(400) => ("invalid_request".to_string(), false),
        Some(_) => ("error".to_string(), false),
        None => ("error".to_string(), false),
    };

    GoogleModelVerifyResult {
        requested_model,
        resolved_model,
        status,
        available,
        http_status: last_status,
        endpoint: last_endpoint,
        reason: last_reason,
    }
}

// ─── Token Refresh Functions ──────────────────────────────────────────────────

pub(crate) async fn get_anthropic_access_token(
    state: Option<&AppState>,
    store: &SqliteStore,
    account_id: &str,
) -> Result<String, String> {
    let lock = oauth_refresh_lock(state, &format!("anthropic:{account_id}")).await;
    let _guard = if let Some(lock) = lock.as_ref() {
        Some(lock.lock().await)
    } else {
        None
    };

    let key = format!("anthropic_token_{account_id}");
    let raw = store.get_app_state(&key).await.unwrap_or_default();
    let info: Value = serde_json::from_str(&raw).unwrap_or_default();

    let access = info
        .get("access_token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if let Some(stored_acct) = info.get("account_id").and_then(Value::as_str) {
        if !stored_acct.is_empty() && stored_acct != account_id {
            return Err(oauth_error_message(
                OAuthErrorClass::Storage,
                "stored token account mismatch",
            ));
        }
    }
    if let Some(stored_provider) = info.get("provider_id").and_then(Value::as_str) {
        if !stored_provider.is_empty() && !stored_provider.eq_ignore_ascii_case("anthropic") {
            return Err(oauth_error_message(
                OAuthErrorClass::Storage,
                "stored token provider mismatch",
            ));
        }
    }
    let expires = info
        .get("access_expires_at")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    if !access.is_empty() && super::now_unix().saturating_add(60) < expires {
        return Ok(access);
    }

    let refresh = info
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if refresh.is_empty() {
        if !access.is_empty() {
            // Legacy tokens can miss refresh token metadata; try access token.
            return Ok(access);
        }
        return Err(oauth_error_message(
            OAuthErrorClass::InvalidGrant,
            "no refresh token stored",
        ));
    }

    let http = super::http_default_for(state);
    for attempt in 0..=2_u32 {
        let resp = match http
            .post(ANTHROPIC_TOKEN_URL)
            .header("anthropic-beta", "oauth-2025-04-20")
            .json(&serde_json::json!({
                "grant_type": "refresh_token",
                "client_id": ANTHROPIC_CLIENT_ID,
                "refresh_token": refresh,
            }))
            .send()
            .await
        {
            Ok(v) => v,
            Err(e) => {
                if attempt < 2 {
                    tokio::time::sleep(Duration::from_millis(oauth_backoff_delay_ms(attempt)))
                        .await;
                    continue;
                }
                return Err(oauth_error_message(
                    OAuthErrorClass::UpstreamTransient,
                    format!("refresh failed: {e}"),
                ));
            }
        };

        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        let body: Value = serde_json::from_str(&body_text).unwrap_or_default();

        if !status.is_success() {
            let class = classify_oauth_http_error(status, &body_text);
            if oauth_retryable(class) && attempt < 2 {
                tokio::time::sleep(Duration::from_millis(oauth_backoff_delay_ms(attempt))).await;
                continue;
            }
            let msg = body
                .get("error_description")
                .or_else(|| body.get("error"))
                .and_then(|v| v.as_str())
                .unwrap_or("refresh failed")
                .to_string();
            return Err(oauth_error_message(class, msg));
        }

        let new_access = body
            .get("access_token")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let new_expires_in = body
            .get("expires_in")
            .and_then(|v| v.as_u64())
            .unwrap_or(3600);
        let new_refresh = body
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .unwrap_or(refresh.as_str())
            .to_string();

        if new_access.is_empty() {
            return Err(oauth_error_message(
                OAuthErrorClass::UpstreamPermanent,
                "no access token in refresh response",
            ));
        }

        let new_expires = super::now_unix().saturating_add(new_expires_in);
        let new_token_info = serde_json::to_string(&json!({
            "access_token": new_access,
            "refresh_token": new_refresh,
            "access_expires_at": new_expires,
            "account_id": account_id,
            "provider_id": "anthropic",
        }))
        .unwrap_or_default();
        let _ = store.set_app_state(&key, &new_token_info).await;
        return Ok(new_access);
    }

    Err(oauth_error_message(
        OAuthErrorClass::UpstreamTransient,
        "refresh failed after retries",
    ))
}

pub(crate) async fn get_google_access_token(
    state: Option<&AppState>,
    store: &SqliteStore,
    account_id: &str,
) -> Result<String, String> {
    let lock = oauth_refresh_lock(state, &format!("google:{account_id}")).await;
    let _guard = if let Some(lock) = lock.as_ref() {
        Some(lock.lock().await)
    } else {
        None
    };

    let key = format!("google_token_{account_id}");
    let raw = store.get_app_state(&key).await.unwrap_or_default();
    let info: Value = serde_json::from_str(&raw).unwrap_or_default();
    let access = info
        .get("access_token")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if let Some(stored_acct) = info.get("account_id").and_then(Value::as_str) {
        if !stored_acct.is_empty() && stored_acct != account_id {
            return Err(oauth_error_message(
                OAuthErrorClass::Storage,
                "stored token account mismatch",
            ));
        }
    }
    if let Some(stored_provider) = info.get("provider_id").and_then(Value::as_str) {
        if !stored_provider.is_empty() && !stored_provider.eq_ignore_ascii_case("google") {
            return Err(oauth_error_message(
                OAuthErrorClass::Storage,
                "stored token provider mismatch",
            ));
        }
    }
    let expires = info
        .get("access_expires_at")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if !access.is_empty() && super::now_unix().saturating_add(60) < expires {
        return Ok(access);
    }
    let refresh = info
        .get("refresh_token")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if refresh.is_empty() {
        return Err(oauth_error_message(
            OAuthErrorClass::InvalidGrant,
            "no refresh token stored",
        ));
    }
    let Some(client_id) = google_oauth_client_id() else {
        return Err(oauth_error_message(
            OAuthErrorClass::UpstreamPermanent,
            "missing LLMHOTSPOT_GOOGLE_OAUTH_CLIENT_ID",
        ));
    };
    let mut form = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh.clone()),
        ("client_id", client_id),
    ];
    if let Some(secret) = google_oauth_client_secret() {
        form.push(("client_secret", secret));
    }
    let http = super::http_default_for(state);
    for attempt in 0..=2_u32 {
        let resp = match http.post(GOOGLE_TOKEN_URL).form(&form).send().await {
            Ok(v) => v,
            Err(e) => {
                if attempt < 2 {
                    tokio::time::sleep(Duration::from_millis(oauth_backoff_delay_ms(attempt)))
                        .await;
                    continue;
                }
                return Err(oauth_error_message(
                    OAuthErrorClass::UpstreamTransient,
                    format!("refresh failed: {e}"),
                ));
            }
        };
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        let body: Value = serde_json::from_str(&body_text).unwrap_or_default();
        if !status.is_success() {
            let class = classify_oauth_http_error(status, &body_text);
            if oauth_retryable(class) && attempt < 2 {
                tokio::time::sleep(Duration::from_millis(oauth_backoff_delay_ms(attempt))).await;
                continue;
            }
            return Err(oauth_error_message(
                class,
                body.get("error_description")
                    .or_else(|| body.get("error"))
                    .and_then(Value::as_str)
                    .unwrap_or("refresh failed"),
            ));
        }
        let new_access = body
            .get("access_token")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let new_expires_in = body
            .get("expires_in")
            .and_then(Value::as_u64)
            .unwrap_or(3600);
        let new_refresh = body
            .get("refresh_token")
            .and_then(Value::as_str)
            .unwrap_or(refresh.as_str())
            .to_string();
        if new_access.is_empty() {
            return Err(oauth_error_message(
                OAuthErrorClass::UpstreamPermanent,
                "no access token in refresh response",
            ));
        }
        let new_expires = super::now_unix().saturating_add(new_expires_in);
        let new_token_info = serde_json::to_string(&json!({
            "access_token": new_access,
            "refresh_token": new_refresh,
            "access_expires_at": new_expires,
            "project_id": info.get("project_id").and_then(Value::as_str).unwrap_or(""),
            "account_id": account_id,
            "provider_id": "google",
        }))
        .unwrap_or_default();
        let _ = store.set_app_state(&key, &new_token_info).await;
        return Ok(new_access);
    }

    Err(oauth_error_message(
        OAuthErrorClass::UpstreamTransient,
        "refresh failed after retries",
    ))
}

pub(crate) async fn resolve_google_project_context(
    store: &SqliteStore,
    access_token: &str,
    account_id: &str,
) -> GoogleProjectContext {
    let token_key = format!("google_token_{account_id}");
    let token_raw = store.get_app_state(&token_key).await.unwrap_or_default();
    let mut token_info: Value = serde_json::from_str(&token_raw).unwrap_or_default();
    let project_id = token_info
        .get("project_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let managed_project_id = token_info
        .get("managed_project_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let mut out = GoogleProjectContext {
        project_id: project_id.clone(),
        managed_project_id: managed_project_id.clone(),
        effective_project_id: if !managed_project_id.is_empty() {
            managed_project_id
        } else {
            project_id
        },
        tier_id: token_info
            .get("tier_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        paid_tier_id: token_info
            .get("paid_tier_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        plan_type: token_info
            .get("plan_type")
            .and_then(Value::as_str)
            .map(str::to_string),
    };
    if !out.effective_project_id.is_empty() && out.plan_type.is_some() {
        return out;
    }

    let http = super::http_default_for(None);
    for base in google_endpoint_chain() {
        let url = format!("{}/v1internal:loadCodeAssist", base.trim_end_matches('/'));
        let mut variants: Vec<Value> = vec![
            json!({
                "metadata": {
                    "ideType": "IDE_UNSPECIFIED",
                    "platform": "PLATFORM_UNSPECIFIED",
                    "pluginType": "ANTIGRAVITY"
                }
            }),
            json!({
                "metadata": {
                    "ideType": "IDE_UNSPECIFIED",
                    "platform": "PLATFORM_UNSPECIFIED",
                    "pluginType": "GEMINI"
                }
            }),
        ];
        if !out.effective_project_id.is_empty() {
            variants.push(json!({
                "cloudaicompanionProject": out.effective_project_id,
                "metadata": {
                    "ideType": "IDE_UNSPECIFIED",
                    "platform": "PLATFORM_UNSPECIFIED",
                    "pluginType": "ANTIGRAVITY",
                    "duetProject": out.effective_project_id
                }
            }));
            variants.push(json!({
                "cloudaicompanionProject": out.effective_project_id,
                "metadata": {
                    "ideType": "IDE_UNSPECIFIED",
                    "platform": "PLATFORM_UNSPECIFIED",
                    "pluginType": "GEMINI",
                    "duetProject": out.effective_project_id
                }
            }));
        }
        for req_body in variants {
            let resp = match http
                .post(&url)
                .headers(google_headers(access_token))
                .json(&req_body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(_) => continue,
            };
            if !resp.status().is_success() {
                continue;
            }
            let body: Value = resp.json().await.unwrap_or_default();
            let discovered = parse_google_project_id(&body);
            if !discovered.is_empty() {
                out.managed_project_id = discovered;
            }
            out.tier_id = body
                .get("currentTier")
                .and_then(|v| v.get("id"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    body.get("allowedTiers")
                        .and_then(Value::as_array)
                        .and_then(|arr| {
                            arr.iter()
                                .find(|t| t.get("isDefault").and_then(Value::as_bool) == Some(true))
                                .and_then(|t| t.get("id").and_then(Value::as_str))
                                .map(str::to_string)
                        })
                })
                .or(out.tier_id.clone());
            out.paid_tier_id = body
                .get("paidTier")
                .and_then(|v| v.get("id"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .or(out.paid_tier_id.clone());
            out.plan_type = infer_google_plan(out.tier_id.as_deref(), out.paid_tier_id.as_deref());
            out.effective_project_id = if !out.managed_project_id.is_empty() {
                out.managed_project_id.clone()
            } else {
                out.project_id.clone()
            };
            if !out.effective_project_id.is_empty() {
                token_info["project_id"] = Value::String(out.project_id.clone());
                token_info["managed_project_id"] = Value::String(out.managed_project_id.clone());
                token_info["effective_project_id"] =
                    Value::String(out.effective_project_id.clone());
                if let Some(v) = out.tier_id.as_ref() {
                    token_info["tier_id"] = Value::String(v.clone());
                }
                if let Some(v) = out.paid_tier_id.as_ref() {
                    token_info["paid_tier_id"] = Value::String(v.clone());
                }
                if let Some(v) = out.plan_type.as_ref() {
                    token_info["plan_type"] = Value::String(v.clone());
                }
                if let Ok(encoded) = serde_json::to_string(&token_info) {
                    let _ = store.set_app_state(&token_key, &encoded).await;
                }
                return out;
            }
        }
    }
    out
}

// ─── OAuth Handler Functions ──────────────────────────────────────────────────

pub(crate) async fn anthropic_oauth_start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<AnthropicStartRequest>,
) -> impl IntoResponse {
    let Some(store) = state.store else {
        return super::missing_store();
    };
    if !super::is_admin_authorized(&headers, &store).await {
        return super::unauthorized_admin();
    }

    let (code_verifier, code_challenge) = pkce_pair();
    let oauth_state = uuid::Uuid::new_v4().to_string();
    let flow_id = uuid::Uuid::new_v4().to_string();
    let expires_at_unix = super::now_unix().saturating_add(600);

    let mut auth_url = match url::Url::parse(ANTHROPIC_AUTH_URL) {
        Ok(u) => u,
        Err(e) => {
            return super::error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                &format!("invalid auth url: {e}"),
            )
        }
    };
    auth_url
        .query_pairs_mut()
        .append_pair("client_id", ANTHROPIC_CLIENT_ID)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", ANTHROPIC_REDIRECT_URI)
        .append_pair("scope", ANTHROPIC_SCOPE)
        .append_pair("state", &oauth_state)
        .append_pair("code_challenge", &code_challenge)
        .append_pair("code_challenge_method", "S256");

    state.anthropic_flows.insert(AnthropicPendingFlow {
        flow_id: flow_id.clone(),
        code_verifier,
        oauth_state,
        expires_at_unix,
        display_name_hint: req.display_name.filter(|s| !s.is_empty()),
        redirect_uri: None,
    });

    (
        StatusCode::OK,
        Json(json!({
            "flow_id": flow_id,
            "authorization_url": auth_url.to_string(),
            "redirect_uri": ANTHROPIC_REDIRECT_URI,
            "expires_at_unix": expires_at_unix,
        })),
    )
        .into_response()
}

pub(crate) async fn anthropic_oauth_complete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<AnthropicCompleteRequest>,
) -> impl IntoResponse {
    let Some(store) = state.store.clone() else {
        return super::missing_store();
    };
    if !super::is_admin_authorized(&headers, &store).await {
        return super::unauthorized_admin();
    }
    match anthropic_oauth_complete_inner(&state, &store, req).await {
        Ok(account_id) => (
            StatusCode::OK,
            Json(json!({ "ok": true, "account_id": account_id })),
        )
            .into_response(),
        Err(err) => {
            let status = if err.contains("oauth_error_class=state_mismatch")
                || err.contains("oauth_error_class=invalid_grant")
            {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::BAD_GATEWAY
            };
            super::error_response(status, ErrorCode::AuthError, &err)
        }
    }
}

pub(crate) async fn anthropic_oauth_complete_inner(
    state: &AppState,
    store: &SqliteStore,
    req: AnthropicCompleteRequest,
) -> Result<String, String> {
    let flow = match state.anthropic_flows.take(&req.flow_id) {
        Some(f) => f,
        None => {
            return Err(oauth_error_message(
                OAuthErrorClass::UpstreamPermanent,
                "unknown or expired flow_id",
            ))
        }
    };

    if super::now_unix() > flow.expires_at_unix {
        return Err(oauth_error_message(
            OAuthErrorClass::StateMismatch,
            "flow expired - please start a new authorization",
        ));
    }
    if req.state.as_deref().unwrap_or("").trim() != flow.oauth_state {
        return Err(oauth_error_message(
            OAuthErrorClass::StateMismatch,
            "state mismatch",
        ));
    }

    let redirect_uri = flow
        .redirect_uri
        .as_deref()
        .unwrap_or(ANTHROPIC_REDIRECT_URI)
        .to_string();

    let http = state.http_default().clone();
    let mut body = Value::Null;
    let mut success = false;
    for attempt in 0..=2_u32 {
        let resp = match http
            .post(ANTHROPIC_TOKEN_URL)
            .header("anthropic-beta", "oauth-2025-04-20")
            .json(&serde_json::json!({
                "grant_type": "authorization_code",
                "client_id": ANTHROPIC_CLIENT_ID,
                "code": req.code,
                "state": req.state,
                "redirect_uri": redirect_uri,
                "code_verifier": flow.code_verifier,
            }))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                if attempt < 2 {
                    tokio::time::sleep(Duration::from_millis(oauth_backoff_delay_ms(attempt)))
                        .await;
                    continue;
                }
                return Err(oauth_error_message(
                    OAuthErrorClass::UpstreamTransient,
                    format!("token exchange failed: {e}"),
                ));
            }
        };

        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            body = serde_json::from_str(&body_text).unwrap_or_default();
            success = true;
            break;
        }

        let class = classify_oauth_http_error(status, &body_text);
        if oauth_retryable(class) && attempt < 2 {
            tokio::time::sleep(Duration::from_millis(oauth_backoff_delay_ms(attempt))).await;
            continue;
        }

        let json_body: Value = serde_json::from_str(&body_text).unwrap_or_default();
        let msg = json_body
            .get("error_description")
            .and_then(|v| v.as_str())
            .or_else(|| {
                json_body.get("error").and_then(|e| {
                    e.as_str()
                        .or_else(|| e.get("message").and_then(|m| m.as_str()))
                })
            })
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                let snippet = &body_text[..body_text.len().min(300)];
                format!(
                    "token exchange failed (HTTP {}): {}",
                    status.as_u16(),
                    snippet
                )
            });
        return Err(oauth_error_message(class, msg));
    }
    if !success {
        return Err(oauth_error_message(
            OAuthErrorClass::UpstreamTransient,
            "token exchange failed after retries",
        ));
    }

    let access_token = body
        .get("access_token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let refresh_token = body
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let expires_in = body
        .get("expires_in")
        .and_then(|v| v.as_u64())
        .unwrap_or(3600);

    if access_token.is_empty() {
        return Err(oauth_error_message(
            OAuthErrorClass::UpstreamPermanent,
            "no access token in response",
        ));
    }

    let account_id = format!("anthropic_{}", uuid::Uuid::new_v4().simple());
    let display_name = flow
        .display_name_hint
        .unwrap_or_else(|| "Anthropic Account".to_string());
    let email = format!("{account_id}@unknown.local");
    let priority = store
        .list_oauth_accounts()
        .await
        .map(|v| v.len() as i64)
        .unwrap_or(0);

    let row = OAuthAccountRecord {
        account_id: account_id.clone(),
        provider_id: "anthropic".to_string(),
        display_name,
        email,
        priority,
        enabled: true,
        last_used_unix: None,
        last_error: None,
        upstream_base_url: Some(ANTHROPIC_UPSTREAM.to_string()),
        chat_path: None,
        responses_path: None,
        org_id: None,
        project_id: None,
        extra_headers_json: None,
    };

    if let Err(e) = store.upsert_oauth_account(&row).await {
        return Err(oauth_error_message(
            OAuthErrorClass::Storage,
            format!("storage error: {e}"),
        ));
    }

    let access_expires = super::now_unix().saturating_add(expires_in);
    let token_info = serde_json::to_string(&json!({
        "access_token": access_token,
        "refresh_token": refresh_token,
        "access_expires_at": access_expires,
        "account_id": account_id,
        "provider_id": "anthropic",
    }))
    .unwrap_or_default();
    let _ = store
        .set_app_state(&format!("anthropic_token_{account_id}"), &token_info)
        .await;

    // Warm Anthropic model cache immediately after successful OAuth completion so
    // the console and /v1/models reflect fresh account models without waiting for
    // the periodic UI refresh interval.
    let _ = refresh_anthropic_models_cache(state, store, &account_id, true).await;

    Ok(account_id)
}

pub(crate) async fn anthropic_oauth_usage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(account_id): Path<String>,
) -> impl IntoResponse {
    let Some(store) = state.store.clone() else {
        return super::missing_store();
    };
    if !super::is_admin_authorized(&headers, &store).await {
        return super::unauthorized_admin();
    }

    let access_token = match get_anthropic_access_token(Some(&state), &store, &account_id).await {
        Ok(t) => t,
        Err(e) => {
            return super::error_response(
                StatusCode::BAD_GATEWAY,
                ErrorCode::Internal,
                &format!("auth failed: {e}"),
            )
        }
    };

    let http = state.http_oauth_short().clone();
    let resp = match http
        .get("https://api.anthropic.com/api/oauth/usage")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("anthropic-version", "2023-06-01")
        .header("anthropic-beta", "oauth-2025-04-20")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return super::error_response(
                StatusCode::BAD_GATEWAY,
                ErrorCode::Internal,
                &format!("usage fetch failed: {e}"),
            )
        }
    };

    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or_default();

    if !status.is_success() {
        let msg = body
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("usage fetch failed");
        return super::error_response(StatusCode::BAD_GATEWAY, ErrorCode::Internal, msg);
    }

    (StatusCode::OK, Json(body)).into_response()
}

pub(crate) async fn anthropic_oauth_models(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(account_id): Path<String>,
) -> impl IntoResponse {
    let Some(store) = state.store.clone() else {
        return super::missing_store();
    };
    if !super::is_admin_authorized(&headers, &store).await {
        return super::unauthorized_admin();
    }

    match refresh_anthropic_models_cache(&state, &store, &account_id, false).await {
        Ok(body) => (StatusCode::OK, Json(body)).into_response(),
        Err(e) => super::error_response(
            StatusCode::BAD_GATEWAY,
            ErrorCode::Internal,
            &format!("models fetch failed: {e}"),
        ),
    }
}

async fn refresh_anthropic_models_cache(
    state: &AppState,
    store: &SqliteStore,
    account_id: &str,
    force_refresh: bool,
) -> Result<Value, String> {
    let now = super::now_unix();
    let cache_key = super::cached_models_key("anthropic", account_id);
    let cached = super::load_models_cache(store, &cache_key, now).await;
    if !force_refresh {
        if let Some(env) = cached.as_ref() {
            return Ok(env.data.clone());
        }
    }

    let access_token = get_anthropic_access_token(Some(state), store, account_id).await?;

    let http = state.http_oauth_short().clone();
    let mut req = http
        .get("https://api.anthropic.com/v1/models")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("anthropic-version", "2023-06-01");
    if let Some(env) = cached.as_ref() {
        if let Some(etag) = env.etag.as_ref() {
            req = req.header("if-none-match", etag);
        }
    }
    if is_anthropic_oauth_token(&access_token) {
        req = req.header("anthropic-beta", ANTHROPIC_OAUTH_BETA_HEADER);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;

    let status = resp.status();
    if status == StatusCode::NOT_MODIFIED {
        if let Some(env) = cached {
            super::upsert_models_cache(store, &cache_key, &env.data, env.etag.clone(), now).await;
            return Ok(env.data);
        }
    }
    let etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body: Value = resp.json().await.unwrap_or_default();
    if !status.is_success() {
        let msg = body
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|v| v.as_str())
            .unwrap_or("models request failed");
        return Err(msg.to_string());
    }

    super::upsert_models_cache(store, &cache_key, &body, etag, now).await;
    Ok(body)
}

pub(crate) async fn google_oauth_start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<GoogleStartRequest>,
) -> impl IntoResponse {
    let Some(store) = state.store else {
        return super::missing_store();
    };
    if !super::is_admin_authorized(&headers, &store).await {
        return super::unauthorized_admin();
    }
    let Some(client_id) = google_oauth_client_id() else {
        return super::error_response(
            StatusCode::BAD_REQUEST,
            ErrorCode::AuthError,
            "missing LLMHOTSPOT_GOOGLE_OAUTH_CLIENT_ID",
        );
    };
    let (code_verifier, code_challenge) = pkce_pair();
    let oauth_state = uuid::Uuid::new_v4().to_string();
    let flow_id = uuid::Uuid::new_v4().to_string();
    let expires_at_unix = super::now_unix().saturating_add(600);
    let redirect_uri = google_oauth_redirect_uri();
    let scope = google_oauth_scope();

    let mut auth_url = match url::Url::parse(GOOGLE_AUTH_URL) {
        Ok(u) => u,
        Err(e) => {
            return super::error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                &format!("invalid auth url: {e}"),
            )
        }
    };
    auth_url
        .query_pairs_mut()
        .append_pair("client_id", &client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("scope", &scope)
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("state", &oauth_state)
        .append_pair("code_challenge", &code_challenge)
        .append_pair("code_challenge_method", "S256");

    state.google_flows.insert(GooglePendingFlow {
        flow_id: flow_id.clone(),
        code_verifier,
        oauth_state,
        redirect_uri: redirect_uri.clone(),
        expires_at_unix,
        display_name_hint: req.display_name.filter(|s| !s.is_empty()),
    });

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "flow_id": flow_id,
            "authorization_url": auth_url.to_string(),
            "expires_at_unix": expires_at_unix
        })),
    )
        .into_response()
}

pub(crate) async fn google_oauth_complete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<GoogleCompleteRequest>,
) -> impl IntoResponse {
    let Some(store) = state.store.clone() else {
        return super::missing_store();
    };
    if !super::is_admin_authorized(&headers, &store).await {
        return super::unauthorized_admin();
    }
    match google_oauth_complete_inner(&state, &store, req).await {
        Ok(account_id) => (
            StatusCode::OK,
            Json(json!({"ok": true, "account_id": account_id})),
        )
            .into_response(),
        Err(err) => {
            let status = if err.contains("oauth_error_class=state_mismatch")
                || err.contains("oauth_error_class=invalid_grant")
            {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::BAD_GATEWAY
            };
            super::error_response(status, ErrorCode::AuthError, &err)
        }
    }
}

pub(crate) async fn google_oauth_complete_inner(
    state: &AppState,
    store: &SqliteStore,
    req: GoogleCompleteRequest,
) -> Result<String, String> {
    let flow = match state.google_flows.take(&req.flow_id) {
        Some(f) => f,
        None => {
            return Err(oauth_error_message(
                OAuthErrorClass::UpstreamPermanent,
                "unknown or expired flow_id",
            ))
        }
    };
    if super::now_unix() > flow.expires_at_unix {
        return Err(oauth_error_message(
            OAuthErrorClass::StateMismatch,
            "flow expired - please start a new authorization",
        ));
    }
    if req.state.as_deref().unwrap_or("").trim() != flow.oauth_state {
        return Err(oauth_error_message(
            OAuthErrorClass::StateMismatch,
            "state mismatch",
        ));
    }
    let Some(client_id) = google_oauth_client_id() else {
        return Err(oauth_error_message(
            OAuthErrorClass::UpstreamPermanent,
            "missing LLMHOTSPOT_GOOGLE_OAUTH_CLIENT_ID",
        ));
    };
    let redirect_uri = flow.redirect_uri.clone();
    let mut form = vec![
        ("client_id", client_id),
        ("code", req.code.clone()),
        ("grant_type", "authorization_code".to_string()),
        ("redirect_uri", redirect_uri),
        ("code_verifier", flow.code_verifier),
    ];
    if let Some(secret) = google_oauth_client_secret() {
        form.push(("client_secret", secret));
    }

    let http = super::http_default_for(Some(state));
    let mut token_body = Value::Null;
    let mut exchange_success = false;
    for attempt in 0..=2_u32 {
        let token_resp = match http.post(GOOGLE_TOKEN_URL).form(&form).send().await {
            Ok(v) => v,
            Err(e) => {
                if attempt < 2 {
                    tokio::time::sleep(Duration::from_millis(oauth_backoff_delay_ms(attempt)))
                        .await;
                    continue;
                }
                return Err(oauth_error_message(
                    OAuthErrorClass::UpstreamTransient,
                    format!("token exchange failed: {e}"),
                ));
            }
        };
        let status = token_resp.status();
        let body_text = token_resp.text().await.unwrap_or_default();
        let parsed: Value = serde_json::from_str(&body_text).unwrap_or_default();
        if status.is_success() {
            token_body = parsed;
            exchange_success = true;
            break;
        }
        let class = classify_oauth_http_error(status, &body_text);
        if oauth_retryable(class) && attempt < 2 {
            tokio::time::sleep(Duration::from_millis(oauth_backoff_delay_ms(attempt))).await;
            continue;
        }
        let msg = parsed
            .get("error_description")
            .or_else(|| parsed.get("error"))
            .and_then(Value::as_str)
            .unwrap_or("token exchange failed")
            .to_string();
        return Err(oauth_error_message(class, msg));
    }
    if !exchange_success {
        return Err(oauth_error_message(
            OAuthErrorClass::UpstreamTransient,
            "token exchange failed after retries",
        ));
    }

    let token: GoogleTokenResponse =
        serde_json::from_value(token_body.clone()).unwrap_or(GoogleTokenResponse {
            access_token: None,
            refresh_token: None,
            expires_in: Some(3600),
        });
    let access_token = token.access_token.unwrap_or_default();
    let refresh_token = token.refresh_token.unwrap_or_default();
    let expires_in = token.expires_in.unwrap_or(3600);
    if access_token.is_empty() {
        return Err(oauth_error_message(
            OAuthErrorClass::UpstreamPermanent,
            "no access token in response",
        ));
    }
    let has_refresh = !refresh_token.is_empty();

    let userinfo_resp = http
        .get(GOOGLE_USERINFO_URL)
        .header("Authorization", format!("Bearer {access_token}"))
        .send()
        .await
        .ok();
    let userinfo: Value = match userinfo_resp {
        Some(resp) => resp.json::<Value>().await.unwrap_or_default(),
        None => Value::Null,
    };
    let email = userinfo
        .get("email")
        .and_then(Value::as_str)
        .unwrap_or("unknown@google.local")
        .to_string();

    let account_id = format!("google_{}", uuid::Uuid::new_v4().simple());
    let display_name = flow
        .display_name_hint
        .unwrap_or_else(|| "Google Account".to_string());
    let priority = store
        .list_oauth_accounts()
        .await
        .map(|v| v.len() as i64)
        .unwrap_or(0);

    let row = OAuthAccountRecord {
        account_id: account_id.clone(),
        provider_id: GOOGLE_PROVIDER_ID.to_string(),
        display_name,
        email,
        priority,
        enabled: true,
        last_used_unix: None,
        last_error: None,
        upstream_base_url: Some(GOOGLE_CODE_ASSIST_DAILY.to_string()),
        chat_path: None,
        responses_path: None,
        org_id: None,
        project_id: None,
        extra_headers_json: None,
    };
    store.upsert_oauth_account(&row).await.map_err(|e| {
        oauth_error_message(OAuthErrorClass::Storage, format!("storage error: {e}"))
    })?;
    let access_expires = super::now_unix().saturating_add(expires_in);
    let token_info = serde_json::to_string(&json!({
        "access_token": access_token,
        "refresh_token": refresh_token,
        "access_expires_at": access_expires,
        "project_id": "",
        "has_refresh_token": has_refresh,
        "account_id": account_id,
        "provider_id": "google",
    }))
    .unwrap_or_default();
    let _ = store
        .set_app_state(&format!("google_token_{account_id}"), &token_info)
        .await;

    Ok(account_id)
}

pub(crate) async fn google_oauth_models(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(account_id): Path<String>,
) -> impl IntoResponse {
    let Some(store) = state.store.clone() else {
        return super::missing_store();
    };
    if !super::is_admin_authorized(&headers, &store).await {
        return super::unauthorized_admin();
    }
    let now = super::now_unix();
    let cache_key = super::cached_models_key("google", &account_id);
    let cached = super::load_models_cache(&store, &cache_key, now).await;
    if let Some(env) = cached.as_ref() {
        return (StatusCode::OK, Json(env.data.clone())).into_response();
    }

    let access_token = match get_google_access_token(Some(&state), &store, &account_id).await {
        Ok(t) => t,
        Err(e) => {
            return super::error_response(StatusCode::BAD_GATEWAY, ErrorCode::Internal, &e);
        }
    };
    let http = state.http_default().clone();
    let mut last_error = String::new();
    for base in google_endpoint_chain() {
        let url = format!("{}/v1internal:listModels", base.trim_end_matches('/'));
        let mut req = http.get(&url).headers(google_headers(&access_token));
        if let Some(env) = cached.as_ref() {
            if let Some(etag) = env.etag.as_ref() {
                req = req.header("if-none-match", etag);
            }
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                last_error = e.to_string();
                continue;
            }
        };
        let status = resp.status();
        if status == StatusCode::NOT_MODIFIED {
            if let Some(env) = cached.as_ref() {
                super::upsert_models_cache(&store, &cache_key, &env.data, env.etag.clone(), now)
                    .await;
                return (StatusCode::OK, Json(env.data.clone())).into_response();
            }
        }
        let etag = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let body: Value = resp.json().await.unwrap_or_default();
        if status.is_success() {
            let out = if body.get("data").is_some() {
                body
            } else {
                let data = body
                    .get("models")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|m| m.get("name").and_then(Value::as_str).map(|s| s.to_string()))
                    .map(|id| json!({"id": id}))
                    .collect::<Vec<_>>();
                json!({"object":"list","data":data})
            };
            super::upsert_models_cache(&store, &cache_key, &out, etag, now).await;
            return (StatusCode::OK, Json(out)).into_response();
        }
        last_error = body
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("models fetch failed")
            .to_string();
    }
    super::error_response(StatusCode::BAD_GATEWAY, ErrorCode::Internal, &last_error)
}

pub(crate) async fn google_oauth_models_verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(account_id): Path<String>,
    Query(query): Query<GoogleModelVerifyQuery>,
) -> impl IntoResponse {
    let Some(store) = state.store.clone() else {
        return super::missing_store();
    };
    if !super::is_admin_authorized(&headers, &store).await {
        return super::unauthorized_admin();
    }

    let refresh = parse_boolish(query.refresh.as_deref());
    let requested_models = parse_csv_models(query.models.as_deref());
    let cache_key = format!("google_models_verify_{account_id}");
    let now = super::now_unix();

    // Fast path: cached result for default model set.
    if !refresh && requested_models.is_none() {
        if let Ok(raw) = store.get_app_state(&cache_key).await {
            if let Ok(cached) = serde_json::from_str::<GoogleModelVerifyCache>(&raw) {
                if now.saturating_sub(cached.verified_at_unix) <= cached.ttl_seconds
                    && cached_verify_usable(&cached)
                {
                    return (
                        StatusCode::OK,
                        Json(json!({
                            "account_id": account_id,
                            "cached": true,
                            "verified_at_unix": cached.verified_at_unix,
                            "ttl_seconds": cached.ttl_seconds,
                            "data": cached.data,
                        })),
                    )
                        .into_response();
                }
            }
        }
    }

    let access_token = match get_google_access_token(Some(&state), &store, &account_id).await {
        Ok(t) => t,
        Err(e) => {
            return super::error_response(StatusCode::BAD_GATEWAY, ErrorCode::Internal, &e);
        }
    };
    let project_ctx = resolve_google_project_context(&store, &access_token, &account_id).await;
    let project_id = project_ctx.effective_project_id;

    let mut candidates: Vec<String> = requested_models.unwrap_or_default();
    if candidates.is_empty() {
        // Prefer live cached list first.
        let key = super::cached_models_key("google", &account_id);
        if let Ok(raw) = store.get_app_state(&key).await {
            if let Some(env) = super::decode_models_cache(&raw) {
                for id in super::parse_model_ids_from_cache_data(&env.data)
                    .into_iter()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                {
                    candidates.push(id);
                }
            }
        }
        if candidates.is_empty() {
            candidates.extend(GOOGLE_KNOWN_MODELS.iter().map(|s| s.to_string()));
        }
        candidates.extend(GOOGLE_VERIFY_EXTRA_MODELS.iter().map(|s| s.to_string()));
    }

    let mut seen = HashSet::new();
    candidates.retain(|m| seen.insert(m.to_ascii_lowercase()));

    let endpoints = google_endpoint_chain();
    let http = state.http_default().clone();
    let mut out: Vec<(usize, GoogleModelVerifyResult)> = Vec::new();
    let mut jobs = tokio::task::JoinSet::new();

    for (idx, model) in candidates.into_iter().enumerate() {
        let http_c = http.clone();
        let token_c = access_token.clone();
        let endpoints_c = endpoints.clone();
        let project_id_c = project_id.clone();
        jobs.spawn(async move {
            let result =
                probe_google_model(http_c, token_c, endpoints_c, project_id_c, model).await;
            (idx, result)
        });
        while jobs.len() >= GOOGLE_VERIFY_MAX_CONCURRENCY {
            if let Some(Ok(v)) = jobs.join_next().await {
                out.push(v);
            }
        }
    }
    while let Some(joined) = jobs.join_next().await {
        if let Ok(v) = joined {
            out.push(v);
        }
    }
    out.sort_by_key(|(idx, _)| *idx);
    let data: Vec<GoogleModelVerifyResult> = out.into_iter().map(|(_, r)| r).collect();

    if query.models.is_none() {
        let cache = GoogleModelVerifyCache {
            verified_at_unix: now,
            ttl_seconds: GOOGLE_VERIFY_CACHE_TTL_SECS,
            data: data.clone(),
        };
        if let Ok(encoded) = serde_json::to_string(&cache) {
            let _ = store.set_app_state(&cache_key, &encoded).await;
        }
    }

    (
        StatusCode::OK,
        Json(json!({
            "account_id": account_id,
            "cached": false,
            "verified_at_unix": now,
            "ttl_seconds": GOOGLE_VERIFY_CACHE_TTL_SECS,
            "data": data,
        })),
    )
        .into_response()
}

pub(crate) async fn google_oauth_usage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(account_id): Path<String>,
) -> impl IntoResponse {
    let Some(store) = state.store.clone() else {
        return super::missing_store();
    };
    if !super::is_admin_authorized(&headers, &store).await {
        return super::unauthorized_admin();
    }
    let access_token = match get_google_access_token(Some(&state), &store, &account_id).await {
        Ok(t) => t,
        Err(e) => {
            return super::error_response(StatusCode::BAD_GATEWAY, ErrorCode::Internal, &e);
        }
    };
    let mut last_error = String::new();
    let project_ctx = resolve_google_project_context(&store, &access_token, &account_id).await;
    let project_id = project_ctx.effective_project_id.clone();
    for base in google_endpoint_chain() {
        let url = format!("{}/v1internal:loadCodeAssist", base.trim_end_matches('/'));
        let mut variants: Vec<Value> = vec![
            json!({
                "metadata": {
                    "ideType": "IDE_UNSPECIFIED",
                    "platform": "PLATFORM_UNSPECIFIED",
                    "pluginType": "GEMINI"
                }
            }),
            json!({
                "metadata": {
                    "ideType": "IDE_UNSPECIFIED",
                    "platform": "PLATFORM_UNSPECIFIED",
                    "pluginType": "ANTIGRAVITY"
                }
            }),
        ];
        if !project_id.is_empty() {
            variants.push(json!({
                "cloudaicompanionProject": project_id,
                "metadata": {
                    "ideType": "IDE_UNSPECIFIED",
                    "platform": "PLATFORM_UNSPECIFIED",
                    "pluginType": "GEMINI",
                    "duetProject": project_id
                }
            }));
            variants.push(json!({
                "cloudaicompanionProject": project_id,
                "metadata": {
                    "ideType": "IDE_UNSPECIFIED",
                    "platform": "PLATFORM_UNSPECIFIED",
                    "pluginType": "ANTIGRAVITY",
                    "duetProject": project_id
                }
            }));
        }
        for req_body in variants {
            let resp = match state
                .http_default()
                .clone()
                .post(&url)
                .headers(google_headers(&access_token))
                .json(&req_body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    last_error = e.to_string();
                    continue;
                }
            };
            let status = resp.status();
            let body: Value = resp.json().await.unwrap_or_default();
            if status.is_success() {
                let tier_id = body
                    .get("currentTier")
                    .and_then(|v| v.get("id"))
                    .and_then(Value::as_str)
                    .map(|s| s.to_string())
                    .or_else(|| {
                        body.get("allowedTiers")
                            .and_then(Value::as_array)
                            .and_then(|arr| {
                                arr.iter()
                                    .find(|t| {
                                        t.get("isDefault").and_then(Value::as_bool) == Some(true)
                                    })
                                    .and_then(|t| t.get("id").and_then(Value::as_str))
                                    .map(|s| s.to_string())
                            })
                    })
                    .or(project_ctx.tier_id.clone());
                let paid_tier_id = body
                    .get("paidTier")
                    .and_then(|v| v.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or(project_ctx.paid_tier_id.clone());
                let inferred_plan = infer_google_plan(tier_id.as_deref(), paid_tier_id.as_deref())
                    .or(project_ctx.plan_type.clone());
                let out = json!({
                    "provider":"google",
                    "connected": true,
                    "token_present": !access_token.is_empty(),
                    "tier_id": tier_id,
                    "paid_tier_id": paid_tier_id,
                    "plan_type": inferred_plan,
                    "project_id": body.get("cloudaicompanionProject")
                        .and_then(Value::as_str)
                        .or_else(|| body.get("cloudaicompanionProject").and_then(|v| v.get("id")).and_then(Value::as_str))
                        .unwrap_or(project_ctx.effective_project_id.as_str()),
                    "allowed_tiers": body.get("allowedTiers").cloned().unwrap_or(Value::Array(vec![])),
                });
                return (StatusCode::OK, Json(out)).into_response();
            }
            last_error = body
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .or_else(|| body.get("message").and_then(Value::as_str))
                .unwrap_or("usage probe failed")
                .to_string();
        }
    }
    (
        StatusCode::OK,
        Json(json!({
            "provider":"google",
            "connected": true,
            "token_present": !access_token.is_empty(),
            "usage_status": "unavailable",
            "error": last_error
        })),
    )
        .into_response()
}

pub(crate) async fn oauth_connect_browser_start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<OAuthBrowserStartRequest>,
) -> impl IntoResponse {
    let Some(store) = state.store.clone() else {
        return super::missing_store();
    };
    if !super::is_admin_authorized(&headers, &store).await {
        return super::unauthorized_admin();
    }
    if !is_local_oauth_connect_attempt(&headers) {
        return super::error_response(
            StatusCode::FORBIDDEN,
            ErrorCode::AuthError,
            "oauth connect is only allowed from localhost console",
        );
    }

    let provider = req
        .provider
        .as_deref()
        .unwrap_or("openai")
        .trim()
        .to_ascii_lowercase();
    if provider == ANTHROPIC_PROVIDER_ID {
        let callback_return_to = browser_callback_return_to(&headers, &state.local_base_url);
        let callback_path = req.callback_path.as_deref().unwrap_or("/callback");
        let server = match super::start_local_oauth_callback_server(
            callback_path,
            callback_return_to.as_deref(),
        )
        .await
        {
            Ok(s) => s,
            Err(err) => return super::map_auth_error(err),
        };
        let redirect_uri = format!("http://localhost:{}{}", server.addr.port(), callback_path);
        let (code_verifier, code_challenge) = pkce_pair();
        let oauth_state = uuid::Uuid::new_v4().to_string();
        let flow_id = uuid::Uuid::new_v4().to_string();
        let expires_at_unix = super::now_unix().saturating_add(600);
        let mut auth_url = match url::Url::parse(ANTHROPIC_AUTH_URL) {
            Ok(u) => u,
            Err(e) => {
                return super::error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorCode::Internal,
                    &format!("invalid auth url: {e}"),
                )
            }
        };
        auth_url
            .query_pairs_mut()
            .append_pair("client_id", ANTHROPIC_CLIENT_ID)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("scope", ANTHROPIC_SCOPE)
            .append_pair("state", &oauth_state)
            .append_pair("code_challenge", &code_challenge)
            .append_pair("code_challenge_method", "S256");
        state.anthropic_flows.insert(AnthropicPendingFlow {
            flow_id: flow_id.clone(),
            code_verifier,
            oauth_state: oauth_state.clone(),
            expires_at_unix,
            display_name_hint: req.display_name.clone().filter(|s| !s.is_empty()),
            redirect_uri: Some(redirect_uri.clone()),
        });
        state.browser_flows.insert(BrowserFlowInner {
            flow_id: flow_id.clone(),
            provider_id: ANTHROPIC_PROVIDER_ID.to_string(),
            expires_at_unix,
            status: BrowserFlowStatus {
                flow_id: flow_id.clone(),
                state: BrowserFlowState::Pending,
                expires_at_unix,
                authorization_url: Some(auth_url.to_string()),
                account_id: None,
                error: None,
            },
            oauth_state,
            redirect_uri: redirect_uri.clone(),
            callback_payload: None,
            callback_server: Some(server),
            return_to: callback_return_to,
            display_name_hint: req.display_name.filter(|s| !s.is_empty()),
        });
        return (
            StatusCode::OK,
            Json(json!({
                "flow_id": flow_id,
                "authorization_url": auth_url.to_string(),
                "expires_at_unix": expires_at_unix,
                "redirect_uri": redirect_uri,
            })),
        )
            .into_response();
    }

    if provider == GOOGLE_PROVIDER_ID {
        let callback_return_to = browser_callback_return_to(&headers, &state.local_base_url);
        let callback_path = req.callback_path.as_deref().unwrap_or("/auth/callback");
        let configured_redirect_uri = std::env::var("LLMHOTSPOT_GOOGLE_REDIRECT_URI")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let (redirect_uri, callback_server) = if let Some(uri) = configured_redirect_uri {
            (uri, None)
        } else {
            let server = match super::start_local_oauth_callback_server(
                callback_path,
                callback_return_to.as_deref(),
            )
            .await
            {
                Ok(s) => s,
                Err(err) => return super::map_auth_error(err),
            };
            let uri = format!("http://localhost:{}{}", server.addr.port(), callback_path);
            (uri, Some(server))
        };
        let Some(client_id) = google_oauth_client_id() else {
            return super::error_response(
                StatusCode::BAD_REQUEST,
                ErrorCode::AuthError,
                "missing LLMHOTSPOT_GOOGLE_OAUTH_CLIENT_ID",
            );
        };
        let (code_verifier, code_challenge) = pkce_pair();
        let oauth_state = uuid::Uuid::new_v4().to_string();
        let flow_id = uuid::Uuid::new_v4().to_string();
        let expires_at_unix = super::now_unix().saturating_add(600);
        let scope = google_oauth_scope();
        let mut auth_url = match url::Url::parse(GOOGLE_AUTH_URL) {
            Ok(u) => u,
            Err(e) => {
                return super::error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorCode::Internal,
                    &format!("invalid auth url: {e}"),
                )
            }
        };
        auth_url
            .query_pairs_mut()
            .append_pair("client_id", &client_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("scope", &scope)
            .append_pair("access_type", "offline")
            .append_pair("prompt", "consent")
            .append_pair("state", &oauth_state)
            .append_pair("code_challenge", &code_challenge)
            .append_pair("code_challenge_method", "S256");

        state.google_flows.insert(GooglePendingFlow {
            flow_id: flow_id.clone(),
            code_verifier,
            oauth_state: oauth_state.clone(),
            redirect_uri: redirect_uri.clone(),
            expires_at_unix,
            display_name_hint: req.display_name.clone().filter(|s| !s.is_empty()),
        });
        state.browser_flows.insert(BrowserFlowInner {
            flow_id: flow_id.clone(),
            provider_id: GOOGLE_PROVIDER_ID.to_string(),
            expires_at_unix,
            status: BrowserFlowStatus {
                flow_id: flow_id.clone(),
                state: BrowserFlowState::Pending,
                expires_at_unix,
                authorization_url: Some(auth_url.to_string()),
                account_id: None,
                error: None,
            },
            oauth_state,
            redirect_uri: redirect_uri.clone(),
            callback_payload: None,
            callback_server,
            return_to: callback_return_to.clone(),
            display_name_hint: req.display_name.clone().filter(|s| !s.is_empty()),
        });
        return (
            StatusCode::OK,
            Json(json!({
                "flow_id": flow_id,
                "authorization_url": auth_url.to_string(),
                "expires_at_unix": expires_at_unix,
                "redirect_uri": redirect_uri
            })),
        )
            .into_response();
    }

    let callback_return_to = browser_callback_return_to(&headers, &state.local_base_url);
    let callback_path = req.callback_path.as_deref().unwrap_or("/auth/callback");
    let configured_redirect_uri = std::env::var("LLMHOTSPOT_OAUTH_REDIRECT_URI")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    const FLOW_TTL: u64 = 300; // 5 minutes

    if let Some((flow_id, authorization_url, expires_at_unix, redirect_uri)) =
        state.browser_flows.extend_pending(
            FLOW_TTL,
            req.display_name.clone().filter(|s| !s.is_empty()),
            callback_return_to.clone(),
        )
    {
        return (
            StatusCode::OK,
            Json(json!({
                "flow_id": flow_id,
                "authorization_url": authorization_url,
                "expires_at_unix": expires_at_unix,
                "redirect_uri": redirect_uri
            })),
        )
            .into_response();
    }

    let (redirect_uri, callback_server) = if let Some(uri) = configured_redirect_uri {
        (uri, None)
    } else {
        let server = match super::start_local_oauth_callback_server(
            callback_path,
            callback_return_to.as_deref(),
        )
        .await
        {
            Ok(s) => s,
            Err(err) => return super::map_auth_error(err),
        };
        let uri = format!("http://localhost:{}{}", server.addr.port(), callback_path);
        (uri, Some(server))
    };

    let start = match state.oauth_client.start_flow(Some(&redirect_uri)) {
        Ok(s) => s,
        Err(err) => return super::map_auth_error(err),
    };
    let flow_id = uuid::Uuid::new_v4().to_string();
    let expires_at_unix = super::now_unix().saturating_add(FLOW_TTL);
    let browser_flows = state.browser_flows.clone();

    browser_flows.insert(BrowserFlowInner {
        flow_id: flow_id.clone(),
        provider_id: OPENAI_PROVIDER_ID.to_string(),
        expires_at_unix,
        status: BrowserFlowStatus {
            flow_id: flow_id.clone(),
            state: BrowserFlowState::Pending,
            expires_at_unix,
            authorization_url: Some(start.authorization_url.clone()),
            account_id: None,
            error: None,
        },
        oauth_state: start.state.clone(),
        redirect_uri: start.redirect_uri.clone(),
        callback_payload: None,
        callback_server,
        return_to: callback_return_to.clone(),
        display_name_hint: req.display_name.filter(|s| !s.is_empty()),
    });

    (
        StatusCode::OK,
        Json(json!({
            "flow_id": flow_id,
            "authorization_url": start.authorization_url,
            "expires_at_unix": expires_at_unix,
            "redirect_uri": start.redirect_uri
        })),
    )
        .into_response()
}

pub(crate) async fn oauth_connect_browser_callback(
    State(state): State<AppState>,
    Query(query): Query<OAuthBrowserCallbackQuery>,
) -> impl IntoResponse {
    let result = if let Some(error) = query.error.clone() {
        Err(format!("oauth callback error: {error}"))
    } else if query.code.trim().is_empty() || query.state.trim().is_empty() {
        Err("oauth callback missing required fields".to_string())
    } else {
        Ok(OAuthCallbackPayload {
            state: query.state.clone(),
            code: query.code.clone(),
        })
    };
    let matched = state
        .browser_flows
        .set_callback_for_state(&query.state, result.clone());
    let result = if matched.is_none() {
        Err(oauth_error_message(
            OAuthErrorClass::StateMismatch,
            "oauth callback state not found",
        ))
    } else {
        result
    };
    let return_to = local_console_return_to(
        sanitize_callback_return_to(query.return_to.as_deref())
            .or_else(|| state.browser_flows.return_to_for_state(&query.state)),
        &state.local_base_url,
    );
    let (title, subtitle) = match result {
        Ok(_) => ("You are being redirected, please wait.", ""),
        Err(_) => ("OAuth failed", "Returning to console..."),
    };
    Html(render_browser_callback_redirect_page(
        title, subtitle, &return_to,
    ))
}

pub(crate) async fn oauth_connect_browser_status(
    State(state): State<AppState>,
    Path(flow_id): Path<String>,
) -> impl IntoResponse {
    state.browser_flows.refresh(&flow_id, super::now_unix());
    if let Some((payload, redirect_uri, display_name_hint, provider_id, provider_flow_id)) =
        state.browser_flows.take_callback(&flow_id)
    {
        let Some(store) = state.store.clone() else {
            return super::missing_store();
        };
        if provider_id.eq_ignore_ascii_case(ANTHROPIC_PROVIDER_ID) {
            let complete = anthropic_oauth_complete_inner(
                &state,
                &store,
                AnthropicCompleteRequest {
                    flow_id: provider_flow_id.clone(),
                    code: payload.code,
                    state: Some(payload.state),
                },
            )
            .await;
            state.browser_flows.set_result(&flow_id, complete);
            let status = state
                .browser_flows
                .get(&flow_id)
                .unwrap_or(BrowserFlowStatus {
                    flow_id: flow_id.clone(),
                    state: BrowserFlowState::Failed,
                    expires_at_unix: super::now_unix(),
                    authorization_url: None,
                    account_id: None,
                    error: Some("oauth exchange failed".to_string()),
                });
            return (StatusCode::OK, Json(json!(status))).into_response();
        }
        if provider_id.eq_ignore_ascii_case(GOOGLE_PROVIDER_ID) {
            let complete = google_oauth_complete_inner(
                &state,
                &store,
                GoogleCompleteRequest {
                    flow_id: provider_flow_id.clone(),
                    code: payload.code,
                    state: Some(payload.state),
                },
            )
            .await;
            state.browser_flows.set_result(&flow_id, complete);
            let status = state
                .browser_flows
                .get(&flow_id)
                .unwrap_or(BrowserFlowStatus {
                    flow_id: flow_id.clone(),
                    state: BrowserFlowState::Failed,
                    expires_at_unix: super::now_unix(),
                    authorization_url: None,
                    account_id: None,
                    error: Some("oauth exchange failed".to_string()),
                });
            return (StatusCode::OK, Json(json!(status))).into_response();
        }
        let token = state
            .oauth_client
            .exchange_code(&payload.state, &payload.code, Some(&redirect_uri))
            .await;
        let token = match token {
            Ok(v) => v,
            Err(err) => {
                state
                    .browser_flows
                    .set_result(&flow_id, Err(err.to_string()));
                let status = state
                    .browser_flows
                    .get(&flow_id)
                    .unwrap_or(BrowserFlowStatus {
                        flow_id: flow_id.clone(),
                        state: BrowserFlowState::Failed,
                        expires_at_unix: super::now_unix(),
                        authorization_url: None,
                        account_id: None,
                        error: Some("oauth exchange failed".to_string()),
                    });
                return (StatusCode::OK, Json(json!(status))).into_response();
            }
        };

        let account_id = token
            .account_id
            .clone()
            .unwrap_or_else(|| format!("acct_{}", super::now_unix()));
        let email = token
            .email
            .clone()
            .unwrap_or_else(|| format!("{account_id}@unknown.local"));
        let display_name = display_name_hint.unwrap_or_else(|| email.clone());
        let priority = store
            .list_oauth_accounts()
            .await
            .map(|v| v.len() as i64)
            .unwrap_or(0);
        let row = OAuthAccountRecord {
            account_id: account_id.clone(),
            provider_id: "openai".to_string(),
            display_name,
            email,
            priority,
            enabled: true,
            last_used_unix: None,
            last_error: None,
            upstream_base_url: None,
            chat_path: None,
            responses_path: None,
            org_id: None,
            project_id: None,
            extra_headers_json: None,
        };
        if let Err(err) = store.upsert_oauth_account(&row).await {
            state
                .browser_flows
                .set_result(&flow_id, Err(format!("storage error: {err}")));
        } else {
            let mut rec =
                StoredTokenRecord::new(&state.workspace_id, &account_id, &token.refresh_token);
            rec.account_id = token.account_id.clone();
            if let Err(err) = store.save_token(&rec).await {
                state
                    .browser_flows
                    .set_result(&flow_id, Err(format!("token store error: {err}")));
            } else {
                state.browser_flows.set_result(&flow_id, Ok(account_id));
            }
        }
    }

    let Some(status) = state.browser_flows.get(&flow_id) else {
        return super::error_response(
            StatusCode::NOT_FOUND,
            ErrorCode::Internal,
            "oauth browser flow not found",
        );
    };
    (StatusCode::OK, Json(json!(status))).into_response()
}

pub(crate) async fn oauth_account_wham_usage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(account_id): Path<String>,
) -> impl IntoResponse {
    let Some(store) = state.store.clone() else {
        return super::missing_store();
    };
    if !super::is_admin_authorized(&headers, &store).await {
        return super::unauthorized_admin();
    }
    let manager = llm_auth::OAuthAccountManager::new(store);
    match manager
        .fetch_wham_usage_for_account(&state.workspace_id, &account_id, &state.oauth_client)
        .await
    {
        Ok(usage) => (StatusCode::OK, Json(usage)).into_response(),
        Err(e) => super::error_response(
            StatusCode::BAD_GATEWAY,
            ErrorCode::Internal,
            &format!("usage fetch failed: {e}"),
        ),
    }
}

// ─── Browser OAuth URL helpers ──────────────────────────────────────────────

fn browser_callback_return_to(headers: &HeaderMap, local_base_url: &str) -> Option<String> {
    if let Some(v) = headers.get(header::REFERER).and_then(|h| h.to_str().ok()) {
        if let Ok(url) = Url::parse(v) {
            let path = url.path().to_ascii_lowercase();
            if url.host_str().map(is_local_host).unwrap_or(false)
                && (path == "/console" || path.ends_with("/console"))
            {
                return Some(v.to_string());
            }
        }
    }
    if let Some(v) = headers.get(header::ORIGIN).and_then(|h| h.to_str().ok()) {
        if let Ok(base) = Url::parse(v) {
            if matches!(base.scheme(), "http" | "https") && base.host_str().is_some() {
                if let Ok(console) = base.join("/console") {
                    return Some(console.to_string());
                }
            }
        }
    }
    Some(format!("{}/console", local_base_url.trim_end_matches('/')))
}

fn is_local_oauth_connect_attempt(headers: &HeaderMap) -> bool {
    // Requests arriving through the remote relay are never local console attempts.
    if headers.contains_key("x-llmhotspot-proxy-mode") {
        return false;
    }

    // Explicit browser origin/referrer on a non-local host is remote.
    for key in [header::ORIGIN, header::REFERER] {
        if let Some(raw) = headers.get(key).and_then(|h| h.to_str().ok()) {
            if let Ok(url) = Url::parse(raw) {
                if let Some(host) = url.host_str() {
                    if !is_local_host(host) {
                        return false;
                    }
                }
            }
        }
    }

    // Fall back to Host when Origin/Referer are absent.
    if let Some(raw_host) = headers.get(header::HOST).and_then(|h| h.to_str().ok()) {
        let host = parse_host_header(raw_host);
        if !host.is_empty() && !is_local_host(&host) {
            return false;
        }
    }

    true
}

fn is_local_host(host: &str) -> bool {
    let host = host.trim().trim_matches(['[', ']']).to_ascii_lowercase();
    if host == "localhost" || host == "127.0.0.1" || host == "::1" {
        return true;
    }
    // Allow LAN/private hosts for local console use (e.g. 192.168.x.x).
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return match ip {
            std::net::IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
            std::net::IpAddr::V6(v6) => {
                v6.is_loopback() || v6.is_unique_local() || (v6.segments()[0] & 0xffc0) == 0xfe80
                // fe80::/10 link-local
            }
        };
    }
    false
}

fn parse_host_header(value: &str) -> String {
    let raw = value.trim();
    if raw.starts_with('[') {
        return raw
            .split(']')
            .next()
            .unwrap_or_default()
            .trim_start_matches('[')
            .to_string();
    }
    raw.split(':').next().unwrap_or_default().to_string()
}

fn sanitize_callback_return_to(value: Option<&str>) -> Option<String> {
    let raw = value?.trim();
    if raw.is_empty() {
        return None;
    }
    let parsed = Url::parse(raw).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return None;
    }
    Some(parsed.to_string())
}

fn local_console_return_to(candidate: Option<String>, local_base_url: &str) -> String {
    let fallback = format!("{}/console", local_base_url.trim_end_matches('/'));
    let Some(raw) = candidate else {
        return fallback;
    };
    let Ok(parsed) = Url::parse(&raw) else {
        return fallback;
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return fallback;
    }
    let Some(host) = parsed.host_str() else {
        return fallback;
    };
    if !is_local_host(host) {
        return fallback;
    }
    raw
}

pub(crate) fn render_browser_callback_redirect_page(
    title: &str,
    subtitle: &str,
    return_to: &str,
) -> String {
    let title = title
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let subtitle = subtitle
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let return_to = return_to
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let subtitle_html = if subtitle.trim().is_empty() {
        String::new()
    } else {
        format!("<p>{subtitle}</p>")
    };
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <meta name="color-scheme" content="light dark" />
  <link rel="icon" id="favicon" type="image/png" href="/console-logo-dark.png" />
  <link rel="icon" type="image/png" href="/console-logo-dark.png" media="(prefers-color-scheme: light)" />
  <link rel="icon" type="image/png" href="/console-logo-light.png" media="(prefers-color-scheme: dark)" />
  <title>LLM Hotspot OAuth</title>
  <style>
    :root{{--bg:#f3f4f6;--card:#fff;--ink:#111827;--muted:#4b5563;--line:#e5e7eb}}
    @media (prefers-color-scheme: dark){{:root{{--bg:#0b0f14;--card:#0f172a;--ink:#e5e7eb;--muted:#9ca3af;--line:#243041}}}}
    html{{color-scheme:light dark}}
    body{{font-family:ui-sans-serif,system-ui;margin:0;display:grid;place-items:center;min-height:100vh;background:var(--bg);color:var(--ink)}}
    .card{{background:var(--card);border:1px solid var(--line);border-radius:14px;padding:28px;max-width:680px;width:min(92vw,680px);text-align:center}}
    .logo{{width:86px;height:86px;object-fit:contain}}
    h1{{margin:14px 0 6px 0;font-size:32px;font-weight:500}}
    p{{margin:0;color:var(--muted);font-size:18px}}
  </style>
</head>
<body>
  <div class="card">
    <img id="brandLogo" class="logo" src="/console-logo-dark.png" alt="LLM Hotspot logo" />
    <h1>{title}</h1>
    {subtitle_html}
  </div>
  <script>
    (function(){{
      const THEME_KEY = 'llmhotspot_theme';
      function stored(){{ try {{ return (localStorage.getItem(THEME_KEY) || 'auto').toString(); }} catch(_){{ return 'auto'; }} }}
      const mq = window.matchMedia ? window.matchMedia('(prefers-color-scheme: dark)') : null;
      function sysDark(){{ return !!(mq && mq.matches); }}
      function eff(mode){{ return mode === 'dark' ? 'dark' : (mode === 'light' ? 'light' : (sysDark() ? 'dark' : 'light')); }}
      function apply(mode){{
        const e = eff(mode);
        const root = document.documentElement;
        if(mode === 'auto'){{ root.removeAttribute('data-theme'); }} else {{ root.setAttribute('data-theme', mode); }}
        const url = (e === 'dark') ? '/console-logo-light.png' : '/console-logo-dark.png';
        const logo = document.getElementById('brandLogo');
        const fav = document.getElementById('favicon');
        if(logo) logo.src = url;
        if(fav) fav.href = url;
      }}
      const mode = stored();
      apply(mode);
      if(mq){{ mq.addEventListener('change', function(){{ if(stored() === 'auto'){{ apply('auto'); }} }}); }}
    }})();
    setTimeout(function(){{window.location.replace("{return_to}")}},700);
  </script>
</body>
</html>"#
    )
}
