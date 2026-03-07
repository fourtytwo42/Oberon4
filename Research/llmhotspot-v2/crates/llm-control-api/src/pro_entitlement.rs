use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use llm_storage::SqliteStore;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::time::Duration;
use url::Url;

use super::{
    error_response, is_admin_authorized, missing_store, now_unix, unauthorized_admin, AppState,
    ErrorCode,
};

pub(crate) async fn auth_status(State(state): State<AppState>) -> impl IntoResponse {
    if let Some(store) = &state.store {
        // Keep console loads responsive even when broker/network is degraded.
        let refreshed = tokio::time::timeout(
            Duration::from_millis(900),
            refresh_app_entitlement_from_broker(store),
        )
        .await
        .unwrap_or(false);
        if refreshed {
            state.tunnel_mode_notify.notify_one();
        }
    }
    let connected = if let Some(store) = &state.store {
        store
            .list_oauth_accounts()
            .await
            .map(|list| !list.is_empty())
            .unwrap_or(false)
    } else {
        false
    };
    let (
        tier,
        provisioning_mode,
        session_expires_unix,
        broker_last_ok_unix,
        broker_offline_since_unix,
        broker_offline_seconds,
        broker_enforcement_deadline_unix,
        broker_warning,
    ) = if let Some(store) = &state.store {
        let tier = store
            .get_app_state("app_entitlement_tier")
            .await
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let tier = if tier == "pro"
            || std::env::var("LLMHOTSPOT_LICENSE_KEY")
                .ok()
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false)
        {
            "pro".to_string()
        } else {
            "free".to_string()
        };
        let provisioning_mode = store
            .get_app_state("app_provisioning_mode")
            .await
            .unwrap_or_else(|_| {
                if tier == "pro" {
                    "managed".to_string()
                } else {
                    "quick".to_string()
                }
            });
        let session_expires_unix = store
            .get_app_state("app_auth_session_expires_unix")
            .await
            .ok()
            .and_then(|v| v.parse::<u64>().ok());
        let broker_last_ok_unix = store
            .get_app_state(APP_BROKER_LAST_OK_UNIX_KEY)
            .await
            .ok()
            .map(|v| parse_state_u64(&v))
            .filter(|v| *v > 0);
        let broker_offline_since_unix = store
            .get_app_state(APP_BROKER_OFFLINE_SINCE_UNIX_KEY)
            .await
            .ok()
            .map(|v| parse_state_u64(&v))
            .filter(|v| *v > 0);
        let broker_offline_seconds = broker_offline_since_unix
            .map(|v| now_unix().saturating_sub(v))
            .unwrap_or(0);
        let broker_enforcement_deadline_unix =
            broker_offline_since_unix.map(|v| v.saturating_add(BROKER_OFFLINE_ENFORCEMENT_SECS));
        let broker_warning = store
            .get_app_state(APP_BROKER_WARNING_KEY)
            .await
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        (
            tier,
            provisioning_mode,
            session_expires_unix,
            broker_last_ok_unix,
            broker_offline_since_unix,
            broker_offline_seconds,
            broker_enforcement_deadline_unix,
            broker_warning,
        )
    } else {
        (
            "free".to_string(),
            "quick".to_string(),
            None,
            None,
            None,
            0,
            None,
            None,
        )
    };
    let has_license_key = std::env::var("LLMHOTSPOT_LICENSE_KEY")
        .ok()
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    let now = now_unix();
    let has_live_session = session_expires_unix
        .map(|exp| exp > now && exp > 0)
        .unwrap_or(false);
    let broker_rejected = broker_warning
        .as_deref()
        .map(|msg| {
            let m = msg.to_ascii_lowercase();
            m.contains("session rejected")
                || m.contains("license_key_required")
                || m.contains("http 401")
                || m.contains("unauthorized")
        })
        .unwrap_or(false);
    let effective_pro = tier == "pro" && (has_license_key || has_live_session) && !broker_rejected;
    let effective_tier = if effective_pro { "pro" } else { "free" };
    let effective_provisioning_mode = if effective_pro {
        provisioning_mode.clone()
    } else {
        "quick".to_string()
    };
    let is_pro = effective_pro;
    let upgrade_url = std::env::var("LLMHOTSPOT_WEBSITE_BASE_URL")
        .ok()
        .or_else(|| std::env::var("APP_BASE_URL").ok())
        .unwrap_or_else(|| "https://llmhotspot.com".to_string());
    Json(json!({
        "connected": connected,
        "tier": effective_tier,
        "pro_active": is_pro,
        "provisioning_mode": effective_provisioning_mode,
        "session_expires_unix": session_expires_unix,
        "broker_reachable": broker_offline_since_unix.is_none(),
        "broker_last_ok_unix": broker_last_ok_unix,
        "broker_offline_since_unix": broker_offline_since_unix,
        "broker_offline_seconds": broker_offline_seconds,
        "broker_enforcement_deadline_unix": broker_enforcement_deadline_unix,
        "broker_warning": broker_warning,
        "capabilities": {
            "managed_tunnel": is_pro,
            "max_enabled_oauth_accounts": if is_pro { -1 } else { 1 },
            "max_api_keys": if is_pro { -1 } else { 1 }
        },
        "upgrade_url": upgrade_url
    }))
}

fn pro_bootstrap_base_url() -> String {
    std::env::var("LLMHOTSPOT_WEBSITE_BASE_URL")
        .ok()
        .or_else(|| std::env::var("APP_BASE_URL").ok())
        .unwrap_or_else(|| "https://llmhotspot.com".to_string())
}

const BROKER_OFFLINE_ENFORCEMENT_SECS: u64 = 24 * 60 * 60;
const APP_BROKER_LAST_OK_UNIX_KEY: &str = "app_broker_last_ok_unix";
const APP_BROKER_OFFLINE_SINCE_UNIX_KEY: &str = "app_broker_offline_since_unix";
const APP_BROKER_LAST_ERROR_KEY: &str = "app_broker_last_error";
const APP_BROKER_WARNING_KEY: &str = "app_broker_warning";
const APP_MANAGED_TUNNEL_ID_KEY: &str = "app_managed_tunnel_id";
const APP_MANAGED_TUNNEL_HOSTNAME_KEY: &str = "app_managed_tunnel_hostname";
const APP_MANAGED_TUNNEL_TOKEN_KEY: &str = "app_managed_tunnel_token";
const APP_MANAGED_TUNNEL_CACHED_UNIX_KEY: &str = "app_managed_tunnel_cached_unix";

fn parse_state_u64(raw: &str) -> u64 {
    raw.trim().parse::<u64>().unwrap_or(0)
}

async fn broker_connectivity_healthy(store: &SqliteStore) {
    let _ = store
        .set_app_state(APP_BROKER_LAST_OK_UNIX_KEY, &now_unix().to_string())
        .await;
    let _ = store
        .set_app_state(APP_BROKER_OFFLINE_SINCE_UNIX_KEY, "0")
        .await;
    let _ = store.set_app_state(APP_BROKER_LAST_ERROR_KEY, "").await;
    let _ = store.set_app_state(APP_BROKER_WARNING_KEY, "").await;
}

async fn broker_offline_since_unix(store: &SqliteStore) -> u64 {
    parse_state_u64(
        &store
            .get_app_state(APP_BROKER_OFFLINE_SINCE_UNIX_KEY)
            .await
            .unwrap_or_default(),
    )
}

async fn mark_broker_unavailable(store: &SqliteStore, err: &str) -> u64 {
    let now = now_unix();
    let mut offline_since = broker_offline_since_unix(store).await;
    if offline_since == 0 {
        offline_since = now;
        let _ = store
            .set_app_state(
                APP_BROKER_OFFLINE_SINCE_UNIX_KEY,
                &offline_since.to_string(),
            )
            .await;
    }
    let elapsed = now.saturating_sub(offline_since);
    let remaining = BROKER_OFFLINE_ENFORCEMENT_SECS.saturating_sub(elapsed);
    let warning = format!(
        "Broker offline for {}s; auto-free in {}s if connectivity is not restored.",
        elapsed, remaining
    );
    let _ = store.set_app_state(APP_BROKER_LAST_ERROR_KEY, err).await;
    let _ = store.set_app_state(APP_BROKER_WARNING_KEY, &warning).await;
    elapsed
}

pub(crate) async fn current_entitlement_tier(store: &SqliteStore) -> String {
    let stored = store
        .get_app_state("app_entitlement_tier")
        .await
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if stored == "pro"
        || std::env::var("LLMHOTSPOT_LICENSE_KEY")
            .ok()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
    {
        "pro".to_string()
    } else {
        "free".to_string()
    }
}

fn pro_broker_base_url() -> Option<String> {
    std::env::var("LLMHOTSPOT_TUNNEL_BROKER_URL")
        .ok()
        .or_else(|| std::env::var("LLMHOTSPOT_WEBSITE_BASE_URL").ok())
        .or_else(|| std::env::var("APP_BASE_URL").ok())
}

async fn set_free_tier_entitlement(store: &SqliteStore) {
    let _ = store.set_app_state(APP_MANAGED_TUNNEL_ID_KEY, "").await;
    let _ = store
        .set_app_state(APP_MANAGED_TUNNEL_HOSTNAME_KEY, "")
        .await;
    let _ = store.set_app_state(APP_MANAGED_TUNNEL_TOKEN_KEY, "").await;
    let _ = store
        .set_app_state(APP_MANAGED_TUNNEL_CACHED_UNIX_KEY, "0")
        .await;
    let _ = store.set_app_state("app_entitlement_tier", "free").await;
    let _ = store.set_app_state("app_auth_session_token", "").await;
    let _ = store
        .set_app_state("app_auth_session_expires_unix", "0")
        .await;
    let _ = store.set_app_state("app_entitlement_license_id", "").await;
    let _ = store.set_app_state("app_entitlement_seat_index", "0").await;
    let _ = store.set_app_state("app_provisioning_mode", "quick").await;
}

async fn refresh_app_entitlement_from_broker(store: &SqliteStore) -> bool {
    let token = store
        .get_app_state("app_auth_session_token")
        .await
        .unwrap_or_default();
    if token.trim().is_empty() {
        return false;
    }
    let expires = store
        .get_app_state("app_auth_session_expires_unix")
        .await
        .unwrap_or_default()
        .parse::<u64>()
        .unwrap_or(0);
    if expires <= now_unix() {
        let _ = store
            .set_app_state(
                APP_BROKER_WARNING_KEY,
                "Pro session expired. Sign in again to restore Pro features.",
            )
            .await;
        set_free_tier_entitlement(store).await;
        return true;
    }
    let Some(base) = pro_broker_base_url() else {
        let elapsed = mark_broker_unavailable(store, "broker base url not configured").await;
        if elapsed >= BROKER_OFFLINE_ENFORCEMENT_SECS {
            let _ = store
                .set_app_state(
                    APP_BROKER_WARNING_KEY,
                    "Pro expired due 24h broker connectivity loss.",
                )
                .await;
            set_free_tier_entitlement(store).await;
            return true;
        }
        return false;
    };
    let device_id = store
        .get_app_state("device_id")
        .await
        .unwrap_or_default()
        .trim()
        .to_string();
    if device_id.is_empty() {
        return false;
    }
    let url = format!("{}/api/app/session/status", base.trim_end_matches('/'));
    let resp = super::http_default_for(None)
        .post(url)
        .header("content-type", "application/json")
        .json(&json!({
            "session_token": token,
            "device_id": device_id
        }))
        .send()
        .await;
    let Ok(resp) = resp else {
        let elapsed =
            mark_broker_unavailable(store, "broker session validation transport error").await;
        if elapsed >= BROKER_OFFLINE_ENFORCEMENT_SECS {
            let _ = store
                .set_app_state(
                    APP_BROKER_WARNING_KEY,
                    "Pro expired due 24h broker connectivity loss.",
                )
                .await;
            set_free_tier_entitlement(store).await;
            return true;
        }
        return false;
    };
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        if status == 401 || status == 403 {
            let _ = store
                .set_app_state(
                    APP_BROKER_WARNING_KEY,
                    &format!("Broker session rejected (HTTP {}).", status),
                )
                .await;
            set_free_tier_entitlement(store).await;
            return true;
        }
        let elapsed = mark_broker_unavailable(
            store,
            &format!("broker session status failed (HTTP {})", status),
        )
        .await;
        if elapsed >= BROKER_OFFLINE_ENFORCEMENT_SECS {
            let _ = store
                .set_app_state(
                    APP_BROKER_WARNING_KEY,
                    "Pro expired due 24h broker connectivity loss.",
                )
                .await;
            set_free_tier_entitlement(store).await;
            return true;
        }
        return false;
    }
    let body: Value = resp.json().await.unwrap_or_else(|_| json!({}));
    let tier = body
        .get("tier")
        .and_then(Value::as_str)
        .unwrap_or("free")
        .to_ascii_lowercase();
    if tier != "pro" {
        let _ = store
            .set_app_state(
                APP_BROKER_WARNING_KEY,
                "Pro access is no longer active on broker. Switched to Free mode.",
            )
            .await;
        set_free_tier_entitlement(store).await;
        return true;
    }
    broker_connectivity_healthy(store).await;
    let _ = store.set_app_state("app_entitlement_tier", "pro").await;
    if let Some(v) = body.get("licenseId").and_then(Value::as_str) {
        let _ = store.set_app_state("app_entitlement_license_id", v).await;
    }
    if let Some(v) = body.get("seatIndex").and_then(Value::as_i64) {
        let _ = store
            .set_app_state("app_entitlement_seat_index", &v.to_string())
            .await;
    }
    false
}

async fn resolve_or_create_device_id(store: &SqliteStore) -> String {
    if let Ok(v) = store.get_app_state("device_id").await {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    let id = format!("dev_{}", uuid::Uuid::new_v4().simple());
    let _ = store.set_app_state("device_id", &id).await;
    id
}

fn resolve_device_name() -> Option<String> {
    // Best-effort host name for seat management visibility.
    for key in ["COMPUTERNAME", "HOSTNAME"] {
        if let Ok(v) = std::env::var(key) {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn resolve_os_name() -> Option<String> {
    let raw = std::env::consts::OS.trim();
    if raw.is_empty() {
        None
    } else {
        Some(raw.to_string())
    }
}

fn pkce_s256_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

#[derive(Debug, Deserialize)]
pub(crate) struct ProStartRequest {
    #[serde(default)]
    redirect_uri: Option<String>,
}

pub(crate) async fn pro_start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ProStartRequest>,
) -> impl IntoResponse {
    let Some(store) = state.store else {
        return missing_store();
    };
    if !is_admin_authorized(&headers, &store).await {
        return unauthorized_admin();
    }
    let device_id = resolve_or_create_device_id(&store).await;
    let device_name = resolve_device_name();
    let os_name = resolve_os_name();
    let state_token = format!("st_{}", uuid::Uuid::new_v4().simple());
    let code_verifier = format!(
        "cv_{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let challenge = pkce_s256_challenge(&code_verifier);
    let created = now_unix();
    let flow_payload = json!({
        "code_verifier": code_verifier,
        "device_id": device_id,
        "device_name": device_name,
        "os": os_name,
        "created_unix": created
    });
    let _ = store
        .set_app_state(
            &format!("app_pro_flow:{}", state_token),
            &flow_payload.to_string(),
        )
        .await;
    let redirect_uri = req
        .redirect_uri
        .unwrap_or_else(|| "llmhotspot://auth/callback".to_string());
    let base = pro_bootstrap_base_url();
    let mut authorize_url = Url::parse(&format!("{}/app/authorize", base.trim_end_matches('/')))
        .unwrap_or_else(|_| Url::parse("https://llmhotspot.com/app/authorize").expect("static"));
    authorize_url
        .query_pairs_mut()
        .append_pair("state", &state_token)
        .append_pair("code_challenge", &challenge)
        .append_pair(
            "device_id",
            flow_payload["device_id"].as_str().unwrap_or(""),
        )
        .append_pair("redirect_uri", &redirect_uri);
    (
        StatusCode::OK,
        Json(json!({"ok":true,"state":state_token,"authorize_url":authorize_url.to_string()})),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
pub(crate) struct ProCallbackCompleteRequest {
    code: String,
    state: String,
}

pub(crate) async fn pro_callback_complete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ProCallbackCompleteRequest>,
) -> impl IntoResponse {
    let Some(store) = state.store.clone() else {
        return missing_store();
    };
    if !is_admin_authorized(&headers, &store).await {
        return unauthorized_admin();
    }
    let code = req.code.trim();
    let state_token = req.state.trim();
    if code.is_empty() || state_token.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            ErrorCode::Internal,
            "missing code/state",
        );
    }
    let raw_flow = match store
        .get_app_state(&format!("app_pro_flow:{}", state_token))
        .await
    {
        Ok(v) => v,
        Err(_) => {
            return error_response(
                StatusCode::UNAUTHORIZED,
                ErrorCode::AuthError,
                "invalid_or_expired_state",
            )
        }
    };
    let flow: Value = match serde_json::from_str(&raw_flow) {
        Ok(v) => v,
        Err(_) => {
            return error_response(
                StatusCode::UNAUTHORIZED,
                ErrorCode::AuthError,
                "invalid_or_expired_state",
            )
        }
    };
    let code_verifier = flow
        .get("code_verifier")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let device_id = flow
        .get("device_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let device_name = flow
        .get("device_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .or_else(resolve_device_name);
    let os_name = flow
        .get("os")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .or_else(resolve_os_name);
    if code_verifier.is_empty() || device_id.is_empty() {
        return error_response(
            StatusCode::UNAUTHORIZED,
            ErrorCode::AuthError,
            "invalid_or_expired_state",
        );
    }
    let Some(base) = pro_broker_base_url() else {
        return error_response(
            StatusCode::BAD_REQUEST,
            ErrorCode::Internal,
            "broker base url not configured",
        );
    };
    let url = format!("{}/api/app/oauth/token", base.trim_end_matches('/'),);
    let mut builder = super::http_default_for(None)
        .post(url)
        .header("content-type", "application/json")
        .json(&json!({
            "code": code,
            "code_verifier": code_verifier,
            "device_id": device_id,
            "device_name": device_name,
            "os": os_name
        }));
    if let Ok(secret) = std::env::var("LLMHOTSPOT_BROKER_SHARED_SECRET") {
        if !secret.trim().is_empty() {
            builder = builder.header("x-broker-secret", secret);
        }
    }
    let resp = match builder.send().await {
        Ok(v) => v,
        Err(err) => {
            return error_response(
                StatusCode::BAD_GATEWAY,
                ErrorCode::Internal,
                &format!("pro token exchange failed: {err}"),
            )
        }
    };
    let status = resp.status();
    let payload: Value = resp.json().await.unwrap_or_else(|_| json!({}));
    if !status.is_success() {
        let msg = payload
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("pro_token_exchange_failed");
        // Before giving up, check if there's already a valid Pro session in the local store.
        // This handles the case where the user is re-running the OAuth flow after a previous
        // successful callback (broker rejects duplicate token exchanges but the session is fine).
        let existing_token = store
            .get_app_state("app_auth_session_token")
            .await
            .unwrap_or_default();
        if !existing_token.trim().is_empty() {
            let existing_expires = store
                .get_app_state("app_auth_session_expires_unix")
                .await
                .unwrap_or_default()
                .parse::<u64>()
                .unwrap_or(0);
            if existing_expires > now_unix() {
                // Valid session already exists - confirm Pro without a new token exchange.
                let _ = store.set_app_state("app_entitlement_tier", "pro").await;
                state.tunnel_mode_notify.notify_one();
                return (
                    StatusCode::OK,
                    Json(json!({"ok":true,"tier":"pro","reused_session":true})),
                )
                    .into_response();
            }
        }
        return error_response(
            StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
            ErrorCode::AuthError,
            msg,
        );
    }
    let session_token = payload
        .get("sessionToken")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let expires_unix = payload
        .get("expiresUnix")
        .and_then(Value::as_u64)
        .unwrap_or(now_unix() + 8 * 60 * 60);
    let license_id = payload
        .get("licenseId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let seat_index = payload
        .get("seatIndex")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let _ = store
        .set_app_state("app_auth_session_token", &session_token)
        .await;
    let _ = store
        .set_app_state("app_auth_session_expires_unix", &expires_unix.to_string())
        .await;
    let _ = store.set_app_state("app_entitlement_tier", "pro").await;
    let _ = store
        .set_app_state("app_entitlement_license_id", &license_id)
        .await;
    let _ = store
        .set_app_state("app_entitlement_seat_index", &seat_index.to_string())
        .await;
    let _ = store
        .set_app_state(
            "app_entitlement_last_verified_unix",
            &now_unix().to_string(),
        )
        .await;
    let _ = store
        .set_app_state(&format!("app_pro_flow:{}", state_token), "")
        .await;
    state.tunnel_mode_notify.notify_one();
    (
        StatusCode::OK,
        Json(json!({"ok":true,"tier":"pro","licenseId":license_id,"seatIndex":seat_index})),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
pub(crate) struct ProActivateKeyRequest {
    license_key: String,
}

pub(crate) async fn pro_activate_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ProActivateKeyRequest>,
) -> impl IntoResponse {
    let Some(store) = state.store.clone() else {
        return missing_store();
    };
    if !is_admin_authorized(&headers, &store).await {
        return unauthorized_admin();
    }
    let license_key = req.license_key.trim();
    if license_key.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            ErrorCode::Internal,
            "license_key_required",
        );
    }
    let Some(base) = pro_broker_base_url() else {
        return error_response(
            StatusCode::BAD_REQUEST,
            ErrorCode::Internal,
            "broker base url not configured",
        );
    };
    let device_id = resolve_or_create_device_id(&store).await;
    let device_name = resolve_device_name();
    let os_name = resolve_os_name();
    let url = format!("{}/api/licenses/activate", base.trim_end_matches('/'));
    let mut builder = super::http_default_for(None)
        .post(url)
        .header("content-type", "application/json")
        .json(&json!({
            "license_key": license_key,
            "device_id": device_id,
            "device_name": device_name,
            "os": os_name
        }));
    if let Ok(secret) = std::env::var("LLMHOTSPOT_BROKER_SHARED_SECRET") {
        if !secret.trim().is_empty() {
            builder = builder.header("x-broker-secret", secret);
        }
    }
    let resp = match builder.send().await {
        Ok(v) => v,
        Err(err) => {
            return error_response(
                StatusCode::BAD_GATEWAY,
                ErrorCode::Internal,
                &format!("activation request failed: {err}"),
            )
        }
    };
    let status = resp.status();
    let payload: Value = resp.json().await.unwrap_or_else(|_| json!({}));
    if !status.is_success() {
        let msg = payload
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("activation_failed");
        return error_response(
            StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
            ErrorCode::AuthError,
            msg,
        );
    }
    let _ = store.set_app_state("app_entitlement_tier", "pro").await;
    let _ = store
        .set_app_state(
            "app_entitlement_license_id",
            payload
                .get("licenseId")
                .and_then(Value::as_str)
                .unwrap_or(""),
        )
        .await;
    let _ = store
        .set_app_state(
            "app_entitlement_seat_index",
            &payload
                .get("seatIndex")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                .to_string(),
        )
        .await;
    let _ = store
        .set_app_state(
            "app_entitlement_last_verified_unix",
            &now_unix().to_string(),
        )
        .await;
    let _ = store.set_app_state("app_auth_session_token", "").await;
    state.tunnel_mode_notify.notify_one();
    (StatusCode::OK, Json(json!({"ok":true,"tier":"pro"}))).into_response()
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConsoleSessionExchangeRequest {
    pub(crate) admin_key: String,
}

pub(crate) async fn get_tunnel_mode(State(state): State<AppState>) -> impl IntoResponse {
    let (tier, provisioning_mode) = if let Some(store) = &state.store {
        let tier = current_entitlement_tier(store).await;
        let provisioning_mode = store
            .get_app_state("app_provisioning_mode")
            .await
            .unwrap_or_else(|_| {
                if tier == "pro" {
                    "managed".to_string()
                } else {
                    "quick".to_string()
                }
            });
        (tier, provisioning_mode)
    } else {
        ("free".to_string(), "quick".to_string())
    };
    let mode = if tier == "pro" {
        "broker"
    } else {
        "trycloudflare"
    };
    // Also surface whether a broker URL is configured so the UI can decide whether
    // to show the toggle at all.
    let broker_available = [
        "LLMHOTSPOT_TUNNEL_BROKER_URL",
        "LLMHOTSPOT_WEBSITE_BASE_URL",
        "APP_BASE_URL",
    ]
    .iter()
    .any(|key| {
        std::env::var(key)
            .ok()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
    });
    Json(serde_json::json!({
        "mode": mode,
        "broker_available": broker_available,
        "read_only": true,
        "tier": tier,
        "provisioning_mode": provisioning_mode
    }))
}

pub(crate) async fn set_tunnel_mode(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let _ = body;
    let _ = state;
    (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({
        "ok": false,
        "error": "mode_control_disabled",
        "message": "Tunnel mode is entitlement-managed. Use Go Pro to enable managed endpoints."
        })),
    )
}

#[cfg(test)]
mod entitlement_connectivity_tests {
    use super::*;
    use axum::{body::to_bytes, extract::State, response::IntoResponse};
    use std::sync::OnceLock;
    use tokio::sync::Mutex;

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[tokio::test]
    async fn auth_status_includes_broker_connectivity_fields() {
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        store
            .set_app_state(APP_BROKER_OFFLINE_SINCE_UNIX_KEY, "100")
            .await
            .expect("set offline");
        store
            .set_app_state(APP_BROKER_WARNING_KEY, "broker unavailable")
            .await
            .expect("set warning");

        let state = AppState::with_store(store, "http://127.0.0.1:11434");
        let resp = auth_status(State(state)).await.into_response();
        let body = to_bytes(resp.into_body(), usize::MAX).await.expect("bytes");
        let payload: Value = serde_json::from_slice(&body).expect("json");
        assert!(payload.get("broker_reachable").is_some());
        assert!(payload.get("broker_last_ok_unix").is_some());
        assert!(payload.get("broker_offline_since_unix").is_some());
        assert!(payload.get("broker_offline_seconds").is_some());
        assert!(payload.get("broker_enforcement_deadline_unix").is_some());
        assert_eq!(
            payload
                .get("broker_warning")
                .and_then(Value::as_str)
                .unwrap_or(""),
            "broker unavailable"
        );
    }

    #[tokio::test]
    async fn refresh_enforces_24h_offline_downgrade() {
        let _guard = env_lock().lock().await;
        let prev_broker = std::env::var("LLMHOTSPOT_TUNNEL_BROKER_URL").ok();
        let prev_site = std::env::var("LLMHOTSPOT_WEBSITE_BASE_URL").ok();
        let prev_app = std::env::var("APP_BASE_URL").ok();
        std::env::remove_var("LLMHOTSPOT_TUNNEL_BROKER_URL");
        std::env::remove_var("LLMHOTSPOT_WEBSITE_BASE_URL");
        std::env::remove_var("APP_BASE_URL");

        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        store
            .set_app_state("app_entitlement_tier", "pro")
            .await
            .expect("set tier");
        store
            .set_app_state("app_auth_session_token", "tok_1")
            .await
            .expect("set token");
        store
            .set_app_state(
                "app_auth_session_expires_unix",
                &(now_unix() + 10_000).to_string(),
            )
            .await
            .expect("set exp");
        store
            .set_app_state(
                APP_BROKER_OFFLINE_SINCE_UNIX_KEY,
                &(now_unix().saturating_sub(BROKER_OFFLINE_ENFORCEMENT_SECS + 10)).to_string(),
            )
            .await
            .expect("set offline");

        let changed = refresh_app_entitlement_from_broker(&store).await;
        assert!(changed);
        let tier = store
            .get_app_state("app_entitlement_tier")
            .await
            .expect("tier");
        assert_eq!(tier, "free");

        match prev_broker {
            Some(v) => std::env::set_var("LLMHOTSPOT_TUNNEL_BROKER_URL", v),
            None => std::env::remove_var("LLMHOTSPOT_TUNNEL_BROKER_URL"),
        }
        match prev_site {
            Some(v) => std::env::set_var("LLMHOTSPOT_WEBSITE_BASE_URL", v),
            None => std::env::remove_var("LLMHOTSPOT_WEBSITE_BASE_URL"),
        }
        match prev_app {
            Some(v) => std::env::set_var("APP_BASE_URL", v),
            None => std::env::remove_var("APP_BASE_URL"),
        }
    }
}
