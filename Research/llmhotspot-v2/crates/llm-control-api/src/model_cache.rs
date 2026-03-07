use crate::model_resolution::parse_headers_json;
use axum::http::{header, HeaderMap, HeaderName};
use llm_storage::{OAuthAccountRecord, SqliteStore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::Duration;

pub(crate) const CUSTOM_API_ACCOUNTS_KEY: &str = "custom_api_accounts_json";
const MODEL_CATALOG_CACHE_TTL_SECS: u64 = 300;
const MODEL_CATALOG_CACHE_VERSION: u64 = 1;

pub(crate) fn hidden_model_from_advertising(model_id: &str) -> bool {
    matches!(
        model_id.to_ascii_lowercase().as_str(),
        "gemini-claude-sonnet-4-5-thinking" | "gemini-claude-opus-4-5-thinking"
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ModelsCacheEnvelope {
    pub(crate) version: u64,
    pub(crate) fetched_at_unix: u64,
    pub(crate) ttl_seconds: u64,
    pub(crate) etag: Option<String>,
    pub(crate) data: Value,
}

pub(crate) fn cached_models_key(provider: &str, account_id: &str) -> String {
    format!("{provider}_models_{account_id}")
}

fn models_cache_is_fresh(env: &ModelsCacheEnvelope, now: u64) -> bool {
    if env.version != MODEL_CATALOG_CACHE_VERSION {
        return false;
    }
    now.saturating_sub(env.fetched_at_unix) <= env.ttl_seconds
}

pub(crate) fn parse_model_ids_from_cache_data(data: &Value) -> Vec<String> {
    data.get("data")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("id").and_then(Value::as_str))
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub(crate) fn decode_models_cache(raw: &str) -> Option<ModelsCacheEnvelope> {
    if let Ok(env) = serde_json::from_str::<ModelsCacheEnvelope>(raw) {
        return Some(env);
    }
    let data = serde_json::from_str::<Value>(raw).ok()?;
    data.get("data").and_then(Value::as_array)?;
    Some(ModelsCacheEnvelope {
        version: MODEL_CATALOG_CACHE_VERSION,
        fetched_at_unix: super::now_unix(),
        ttl_seconds: MODEL_CATALOG_CACHE_TTL_SECS,
        etag: None,
        data,
    })
}

pub(crate) async fn load_models_cache(
    store: &SqliteStore,
    key: &str,
    now: u64,
) -> Option<ModelsCacheEnvelope> {
    let raw = store.get_app_state(key).await.ok()?;
    let env = decode_models_cache(&raw)?;
    if models_cache_is_fresh(&env, now) {
        Some(env)
    } else {
        None
    }
}

pub(crate) async fn upsert_models_cache(
    store: &SqliteStore,
    key: &str,
    data: &Value,
    etag: Option<String>,
    now: u64,
) {
    let env = ModelsCacheEnvelope {
        version: MODEL_CATALOG_CACHE_VERSION,
        fetched_at_unix: now,
        ttl_seconds: MODEL_CATALOG_CACHE_TTL_SECS,
        etag,
        data: data.clone(),
    };
    if let Ok(encoded) = serde_json::to_string(&env) {
        let _ = store.set_app_state(key, &encoded).await;
    }
}

pub(crate) fn model_ids_to_cache_payload(ids: &[String]) -> Value {
    json!({
        "object": "list",
        "data": ids.iter().map(|id| json!({"id": id, "object": "model"})).collect::<Vec<_>>()
    })
}

pub(crate) fn parse_openai_models_response(data: &Value) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for id in data
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| row.get("id").and_then(Value::as_str))
    {
        let norm = id.trim();
        if norm.is_empty() {
            continue;
        }
        let key = norm.to_ascii_lowercase();
        if seen.insert(key) {
            out.push(norm.to_string());
        }
    }
    out
}

pub(crate) async fn refresh_custom_api_models_for_account(
    store: &SqliteStore,
    account: &OAuthAccountRecord,
    force: bool,
) -> Result<Vec<String>, String> {
    let now = super::now_unix();
    let cache_key = cached_models_key(super::OPENAI_API_PROVIDER_ID, &account.account_id);
    if !force {
        if let Some(env) = load_models_cache(store, &cache_key, now).await {
            let ids = parse_model_ids_from_cache_data(&env.data);
            if !ids.is_empty() {
                return Ok(ids);
            }
        }
    }

    let base = account
        .upstream_base_url
        .as_deref()
        .and_then(super::canonical_base_url)
        .ok_or_else(|| "custom API account missing valid base_url".to_string())?;
    let api_key = super::get_custom_api_key(store, &account.account_id).await?;
    if api_key.trim().is_empty() {
        return Err("custom API account key is missing".to_string());
    }
    let mut headers = HeaderMap::new();
    if let Ok(auth) = format!("Bearer {}", api_key.trim()).parse() {
        headers.insert(header::AUTHORIZATION, auth);
    }
    if let Some(org) = account
        .org_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Ok(v) = org.parse() {
            headers.insert(HeaderName::from_static("openai-organization"), v);
        }
    }
    if let Some(project) = account
        .project_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Ok(v) = project.parse() {
            headers.insert(HeaderName::from_static("openai-project"), v);
        }
    }
    for (k, v) in parse_headers_json(account.extra_headers_json.as_deref()) {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(k.as_bytes()),
            v.parse::<header::HeaderValue>(),
        ) {
            headers.insert(name, value);
        }
    }

    let url = format!("{}/models", base.trim_end_matches('/'));
    let http = super::http_default_for(None);
    let resp = http
        .get(&url)
        .headers(headers)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| format!("custom models request failed: {e}"))?;
    let status = resp.status();
    let etag = resp
        .headers()
        .get(header::ETAG)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("custom models parse failed: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "custom models request failed: HTTP {} {}",
            status.as_u16(),
            super::redact_sensitive_text(&body.to_string())
        ));
    }
    let ids = parse_openai_models_response(&body);
    if ids.is_empty() {
        return Err("custom models response had no model ids".to_string());
    }
    let payload = model_ids_to_cache_payload(&ids);
    upsert_models_cache(store, &cache_key, &payload, etag, now).await;
    Ok(ids)
}
