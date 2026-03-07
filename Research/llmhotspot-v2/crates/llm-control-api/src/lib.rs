mod message_conversion;
use message_conversion::*;

mod oauth_handlers;
use oauth_handlers::*;

mod auth;
use auth::*;

mod model_cache;
use model_cache::*;

mod secret_store;
pub(crate) use secret_store::*;

mod key_management;
use key_management::*;

mod google_routing;
use google_routing::*;

mod model_resolution;
use model_resolution::*;

mod account_management;
use account_management::*;

mod diagnostics;
use diagnostics::*;

mod routing;
use routing::*;

mod sse_bridge;
use sse_bridge::*;

mod pro_entitlement;
use pro_entitlement::*;
mod chat_sse_conversion;
use chat_sse_conversion::*;

mod http_middleware;
pub(crate) use http_middleware::*;
mod console_assets;
pub(crate) use console_assets::*;
mod models_endpoint;
pub(crate) use models_endpoint::*;
mod session_oauth_entry;
pub(crate) use session_oauth_entry::*;
mod runtime_endpoints;
pub(crate) use runtime_endpoints::*;
mod inference_endpoints;
pub(crate) use inference_endpoints::*;
mod http_errors;
pub(crate) use http_errors::*;
mod account_helpers;
pub(crate) use account_helpers::*;
mod trace_helpers;
pub(crate) use trace_helpers::*;
mod runtime_primitives;
pub(crate) use runtime_primitives::*;
mod http_clients;
pub(crate) use http_clients::*;
mod stream_transport;
pub(crate) use stream_transport::*;

use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::IntoResponse,
    routing::{get, patch, post},
    Json, Router,
};
use llm_auth::{start_oauth_callback_server, AuthError, OpenAiChatGptOAuthClient};
use llm_foundation::contracts::{ErrorCode, ErrorEnvelope};
use llm_gateway::{DefaultGatewayService, GatewayError, GatewayService};
use llm_storage::{OAuthAccountRecord, SqliteStore, StoredTokenRecord};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{atomic::AtomicBool, Arc},
};
use tokio::sync::Mutex as AsyncMutex;
use url::Url;

#[derive(Clone, Default)]
pub struct AppState {
    pub gateway: DefaultGatewayService,
    pub store: Option<SqliteStore>,
    pub local_base_url: String,
    pub oauth_client: OpenAiChatGptOAuthClient,
    pub workspace_id: String,
    /// Notified whenever the tunnel mode is changed via the API so the tunnel
    /// manager can react immediately instead of waiting for its poll interval.
    pub tunnel_mode_notify: std::sync::Arc<tokio::sync::Notify>,
    browser_flows: BrowserFlowStore,
    anthropic_flows: AnthropicFlowStore,
    google_flows: GoogleFlowStore,
    google_tool_signatures: GoogleThoughtSignatureStore,
    oauth_refresh_locks: Arc<AsyncMutex<HashMap<String, Arc<AsyncMutex<()>>>>>,
    wire_capture: Arc<AsyncMutex<VecDeque<Value>>>,
    pub(crate) wire_capture_override_loaded: Arc<AtomicBool>,
    pub(crate) wire_capture_override_set: Arc<AtomicBool>,
    pub(crate) wire_capture_override_enabled: Arc<AtomicBool>,
    diag_usage_buffer: Arc<AsyncMutex<DiagUsageBuffer>>,
    diag_usage_flush_started: Arc<AtomicBool>,
    http_clients: HttpClients,
}

impl AppState {
    pub fn with_store(store: SqliteStore, local_base_url: impl Into<String>) -> Self {
        Self {
            gateway: DefaultGatewayService::default(),
            store: Some(store),
            local_base_url: local_base_url.into(),
            oauth_client: OpenAiChatGptOAuthClient::default(),
            workspace_id: "default".to_string(),
            tunnel_mode_notify: std::sync::Arc::new(tokio::sync::Notify::new()),
            browser_flows: BrowserFlowStore::default(),
            anthropic_flows: AnthropicFlowStore::default(),
            google_flows: GoogleFlowStore::default(),
            google_tool_signatures: GoogleThoughtSignatureStore::default(),
            oauth_refresh_locks: Arc::new(AsyncMutex::new(HashMap::new())),
            wire_capture: Arc::new(AsyncMutex::new(VecDeque::new())),
            wire_capture_override_loaded: Arc::new(AtomicBool::new(false)),
            wire_capture_override_set: Arc::new(AtomicBool::new(false)),
            wire_capture_override_enabled: Arc::new(AtomicBool::new(false)),
            diag_usage_buffer: Arc::new(AsyncMutex::new(DiagUsageBuffer::default())),
            diag_usage_flush_started: Arc::new(AtomicBool::new(false)),
            http_clients: HttpClients::default(),
        }
    }

    pub(crate) fn http_default(&self) -> &reqwest::Client {
        self.http_clients.default_json()
    }

    pub(crate) fn http_long(&self) -> &reqwest::Client {
        self.http_clients.long_stream()
    }

    pub(crate) fn http_oauth_short(&self) -> &reqwest::Client {
        self.http_clients.oauth_short()
    }
}

pub fn app_router(state: AppState) -> Router {
    ensure_diag_usage_flusher(&state);
    Router::new()
        .route("/", get(console_ui))
        .route("/console", get(console_ui))
        .route("/remote", get(console_ui))
        .route("/console-logo.png", get(console_logo))
        .route("/console-logo-dark.png", get(console_logo_dark))
        .route("/console-logo-light.png", get(console_logo_light))
        .route("/console-google-logo.webp", get(console_google_logo))
        .route("/console-logo-hover-dark.png", get(console_logo_hover_dark))
        .route(
            "/console-logo-hover-light.png",
            get(console_logo_hover_light),
        )
        .route("/v1/models", get(models))
        .route("/v1/version", get(version_info))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/messages", post(anthropic_messages))
        .route("/v1/google/models/{*tail}", post(google_model_dispatch))
        .route("/v1/responses", post(responses))
        .route("/v1/auth/status", get(auth_status))
        .route("/v1/pro/start", post(pro_start))
        .route("/v1/pro/callback/complete", post(pro_callback_complete))
        .route("/v1/pro/activate-key", post(pro_activate_key))
        .route("/v1/endpoints", get(get_endpoints))
        .route(
            "/v1/tunnel-mode",
            get(get_tunnel_mode).post(set_tunnel_mode),
        )
        .route("/v1/tray/status", get(tray_status))
        .route("/v1/oauth/accounts", get(list_oauth_accounts))
        .route(
            "/v1/api-accounts",
            get(list_api_accounts).post(create_api_account),
        )
        .route(
            "/v1/api-accounts/{account_id}",
            patch(update_api_account).post(update_api_account),
        )
        .route(
            "/v1/api-accounts/{account_id}/delete",
            post(delete_api_account),
        )
        .route(
            "/v1/api-accounts/{account_id}/models/refresh",
            post(refresh_api_account_models),
        )
        .route(
            "/v1/oauth/accounts/{account_id}/routing",
            get(get_oauth_routing).post(set_oauth_routing),
        )
        .route("/v1/keys", get(list_keys))
        .route("/v1/diagnostics", get(diagnostics))
        .route("/v1/diagnostics/clear", post(clear_diagnostics))
        .route("/v1/diagnostics/export", get(export_diagnostics))
        .route("/v1/diagnostics/wire", get(diagnostics_wire))
        .route("/v1/diagnostics/wire/toggle", post(toggle_diagnostics_wire))
        .route("/v1/diagnostics/wire/clear", post(clear_diagnostics_wire))
        .route("/v1/factory-reset", post(factory_reset))
        .route(
            "/v1/console/session/exchange",
            post(console_session_exchange),
        )
        .route("/v1/oauth/start", post(oauth_start))
        .route("/v1/oauth/callback/complete", post(oauth_callback_complete))
        .route(
            "/v1/oauth/connect/browser/start",
            post(oauth_connect_browser_start),
        )
        .route(
            "/v1/oauth/connect/browser/{flow_id}",
            get(oauth_connect_browser_status),
        )
        .route(
            "/v1/oauth/connect/browser/callback",
            get(oauth_connect_browser_callback),
        )
        .route("/v1/oauth/accounts/connect", post(connect_oauth_account))
        .route(
            "/v1/oauth/accounts/{account_id}/activate",
            post(activate_oauth_account),
        )
        .route(
            "/v1/oauth/accounts/{account_id}/enable",
            post(enable_oauth_account),
        )
        .route(
            "/v1/oauth/accounts/{account_id}/disable",
            post(disable_oauth_account),
        )
        .route(
            "/v1/oauth/accounts/{account_id}/disconnect",
            post(disconnect_oauth_account),
        )
        .route(
            "/v1/oauth/accounts/{account_id}/priority",
            post(set_oauth_priority),
        )
        .route(
            "/v1/oauth/accounts/{account_id}/wham-usage",
            get(oauth_account_wham_usage),
        )
        .route("/v1/keys/create", post(create_key))
        .route("/v1/keys/{key_id}/revoke", post(revoke_key))
        .route("/v1/keys/{key_id}/enable", post(enable_key))
        .route("/v1/keys/{key_id}/disable", post(disable_key))
        .route("/v1/keys/{key_id}/delete", post(delete_key))
        .route("/v1/keys/{key_id}/copy", get(copy_key))
        .route("/v1/usage/reset-key/{key_id}", post(reset_key_usage))
        .route(
            "/v1/usage/reset-oauth/{account_id}",
            post(reset_oauth_usage),
        )
        .route(
            "/v1/usage/reset-model/{model_name}",
            post(reset_model_usage),
        )
        .route("/v1/oauth/anthropic/start", post(anthropic_oauth_start))
        .route(
            "/v1/oauth/anthropic/complete",
            post(anthropic_oauth_complete),
        )
        .route(
            "/v1/oauth/anthropic/usage/{account_id}",
            get(anthropic_oauth_usage),
        )
        .route(
            "/v1/oauth/anthropic/models/{account_id}",
            get(anthropic_oauth_models),
        )
        .route("/v1/oauth/google/start", post(google_oauth_start))
        .route("/v1/oauth/google/complete", post(google_oauth_complete))
        .route(
            "/v1/oauth/google/models/{account_id}",
            get(google_oauth_models),
        )
        .route(
            "/v1/oauth/google/models/verify/{account_id}",
            get(google_oauth_models_verify),
        )
        .route(
            "/v1/oauth/google/usage/{account_id}",
            get(google_oauth_usage),
        )
        .layer(middleware::from_fn(request_id_middleware))
        .layer(middleware::from_fn(path_normalization_middleware))
        .with_state(state)
}

#[cfg(test)]
include!("codex_contract_tests.rs");
