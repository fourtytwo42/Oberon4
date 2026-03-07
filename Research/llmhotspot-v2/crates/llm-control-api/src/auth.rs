use axum::http::{header, HeaderMap};
use llm_storage::ApiKeyRecord;
use std::sync::OnceLock;
use tokio::sync::Mutex as AsyncMutex;

use super::{AppState, SqliteStore};

pub(crate) struct BearerAuthDecision {
    pub(crate) authorized: bool,
    pub(crate) reason: String,
    pub(crate) key_id: Option<String>,
}

pub(crate) async fn evaluate_bearer_auth(
    headers: &HeaderMap,
    state: &AppState,
) -> BearerAuthDecision {
    let Some(store) = &state.store else {
        return BearerAuthDecision {
            authorized: true,
            reason: "authorized_no_store".to_string(),
            key_id: None,
        };
    };

    let key_count = match store.count_api_keys().await {
        Ok(v) => v,
        Err(_) => {
            return BearerAuthDecision {
                authorized: false,
                reason: "storage_api_key_count_error".to_string(),
                key_id: None,
            }
        }
    };
    if key_count == 0 {
        if no_key_auth_bypass_enabled() {
            return BearerAuthDecision {
                authorized: true,
                reason: "authorized_no_api_keys".to_string(),
                key_id: None,
            };
        }
        if ensure_default_endpoint_api_key(store, &state.workspace_id)
            .await
            .is_err()
        {
            return BearerAuthDecision {
                authorized: false,
                reason: "storage_bootstrap_key_error".to_string(),
                key_id: None,
            };
        }
    }

    let Some(token) = bearer_token(headers) else {
        return BearerAuthDecision {
            authorized: false,
            reason: "missing_bearer_header".to_string(),
            key_id: None,
        };
    };

    let now = super::now_unix();
    let token_hash = super::hash_key(token);
    let key = match store.get_api_key_by_hash(&token_hash).await {
        Ok(Some(k)) => k,
        Ok(None) => {
            return BearerAuthDecision {
                authorized: false,
                reason: "token_hash_not_found".to_string(),
                key_id: None,
            };
        }
        Err(_) => {
            return BearerAuthDecision {
                authorized: false,
                reason: "storage_api_key_lookup_error".to_string(),
                key_id: None,
            };
        }
    };
    if key.revoked_unix.is_some() {
        return BearerAuthDecision {
            authorized: false,
            reason: "token_revoked".to_string(),
            key_id: None,
        };
    }
    if key.expires_unix.map(|v| v <= now).unwrap_or(false) {
        return BearerAuthDecision {
            authorized: false,
            reason: "token_expired".to_string(),
            key_id: None,
        };
    }
    let _ = store.touch_api_key_last_used(&key.key_id, now).await;
    BearerAuthDecision {
        authorized: true,
        reason: "authorized".to_string(),
        key_id: Some(key.key_id),
    }
}

pub(crate) async fn record_bearer_auth_rejection(state: &AppState, reason: &str) {
    let Some(store) = &state.store else {
        return;
    };
    let _ = store
        .set_app_state("diag_last_bearer_auth_rejection", reason)
        .await;
    let _ = store
        .set_app_state(
            "diag_last_bearer_auth_rejection_unix",
            &super::now_unix().to_string(),
        )
        .await;
}

pub(crate) async fn is_admin_authorized(headers: &HeaderMap, store: &SqliteStore) -> bool {
    if is_console_session_authorized(headers, store).await {
        return true;
    }

    // The console bootstrap key is always valid as a Bearer, regardless of API key list.
    let bootstrap = store
        .get_app_state("console_admin_bootstrap_key")
        .await
        .unwrap_or_default();
    if !bootstrap.is_empty()
        && bearer_token(headers)
            .map(|t| t == bootstrap)
            .unwrap_or(false)
    {
        return true;
    }

    let Some(token) = bearer_token(headers) else {
        return false;
    };
    let token_hash = super::hash_key(token);
    let now = super::now_unix();
    match store.get_api_key_by_hash(&token_hash).await {
        Ok(Some(key)) => {
            return key.revoked_unix.is_none() && key.expires_unix.map(|v| v > now).unwrap_or(true);
        }
        Ok(None) => {}
        Err(_) => return false,
    }

    let key_count = match store.count_api_keys().await {
        Ok(v) => v,
        Err(_) => return false,
    };
    if key_count == 0 {
        return std::env::var("LLMHOTSPOT_BOOTSTRAP_TOKEN")
            .ok()
            .map(|bt| token == bt)
            .unwrap_or(false);
    }
    false
}

pub(crate) async fn is_console_session_authorized(
    headers: &HeaderMap,
    store: &SqliteStore,
) -> bool {
    let Some(raw) = cookie_value(headers, "llmhotspot_console_session") else {
        return false;
    };
    let expected_hash = store
        .get_app_state("console_admin_session_token_hash")
        .await
        .unwrap_or_default();
    if expected_hash.is_empty() {
        return false;
    }
    let expires = store
        .get_app_state("console_admin_session_expires_unix")
        .await
        .unwrap_or_default()
        .parse::<u64>()
        .unwrap_or(0);
    if expires <= super::now_unix() {
        return false;
    }
    super::hash_key(&raw) == expected_hash
}

pub(crate) fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    if let Some(raw) = headers.get("authorization").and_then(|h| h.to_str().ok()) {
        if let Some(token) = raw.strip_prefix("Bearer ").map(str::trim) {
            if !token.is_empty() {
                return Some(token);
            }
        }
    }
    headers
        .get("x-api-key")
        .and_then(|h| h.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            headers
                .get("api-key")
                .and_then(|h| h.to_str().ok())
                .map(str::trim)
                .filter(|s| !s.is_empty())
        })
}

pub(crate) fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let piece = part.trim();
        let Some((k, v)) = piece.split_once('=') else {
            continue;
        };
        if k.trim() == name {
            return Some(v.trim().to_string());
        }
    }
    None
}

fn no_key_auth_bypass_enabled() -> bool {
    std::env::var("LLMHOTSPOT_ALLOW_UNAUTH_NO_KEYS")
        .ok()
        .map(|v| {
            let n = v.trim().to_ascii_lowercase();
            n == "1" || n == "true" || n == "yes" || n == "on"
        })
        .unwrap_or(false)
}

fn bootstrap_key_lock() -> &'static AsyncMutex<()> {
    static LOCK: OnceLock<AsyncMutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| AsyncMutex::new(()))
}

async fn ensure_default_endpoint_api_key(
    store: &SqliteStore,
    workspace_id: &str,
) -> Result<(), String> {
    if store.count_api_keys().await.map_err(|e| e.to_string())? > 0 {
        return Ok(());
    }

    let _guard = bootstrap_key_lock().lock().await;
    if store.count_api_keys().await.map_err(|e| e.to_string())? > 0 {
        return Ok(());
    }

    let raw_key = format!("hs_{}", uuid::Uuid::new_v4().simple());
    let key = ApiKeyRecord {
        key_id: uuid::Uuid::new_v4().to_string(),
        label: "default-lan-key".to_string(),
        key_hash: super::hash_key(&raw_key),
        tool_choice_policy: "auto".to_string(),
        created_unix: super::now_unix(),
        expires_unix: None,
        revoked_unix: None,
        last_used_unix: None,
    };
    store
        .create_api_key(&key)
        .await
        .map_err(|e| e.to_string())?;
    if let Err(e) =
        super::set_api_key_raw_with_fallback(store, workspace_id, &key.key_id, &raw_key).await
    {
        let _ = store.delete_api_key(&key.key_id).await;
        return Err(format!("raw_key_persist_failed:{e}"));
    }
    if let Err(e) = store
        .set_app_state("default_endpoint_api_key_id", &key.key_id)
        .await
    {
        let _ = store.delete_api_key(&key.key_id).await;
        return Err(e.to_string());
    }
    Ok(())
}

#[cfg(test)]
mod auth_tests {
    use super::evaluate_bearer_auth;
    use crate::AppState;
    use axum::http::HeaderMap;
    use llm_storage::SqliteStore;

    #[tokio::test]
    async fn missing_token_bootstraps_default_key_and_denies_request() {
        std::env::remove_var("LLMHOTSPOT_ALLOW_UNAUTH_NO_KEYS");
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        let state = AppState::with_store(store.clone(), "http://127.0.0.1:51824");
        let decision = evaluate_bearer_auth(&HeaderMap::new(), &state).await;
        assert!(!decision.authorized);
        assert_eq!(decision.reason, "missing_bearer_header");
        let keys = store.list_api_keys().await.expect("list keys");
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].label, "default-lan-key");
    }

    #[tokio::test]
    async fn default_bootstrap_key_authorizes() {
        std::env::remove_var("LLMHOTSPOT_ALLOW_UNAUTH_NO_KEYS");
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        let state = AppState::with_store(store.clone(), "http://127.0.0.1:51824");
        let _ = evaluate_bearer_auth(&HeaderMap::new(), &state).await;

        let keys = store.list_api_keys().await.expect("list keys");
        let raw = crate::get_api_key_raw_with_legacy_migration(&store, "default", &keys[0].key_id)
            .await
            .expect("get key")
            .expect("raw key");
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            format!("Bearer {raw}").parse().expect("valid header value"),
        );
        let decision = evaluate_bearer_auth(&headers, &state).await;
        assert!(decision.authorized);
        assert_eq!(decision.reason, "authorized");
        assert_eq!(decision.key_id.as_deref(), Some(keys[0].key_id.as_str()));
    }
}
