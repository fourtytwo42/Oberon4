use axum::{
    body::{self, Body},
    http::Request,
};
use llm_control_api::{app_router, AppState};
use llm_storage::{OAuthAccountRecord, SqliteStore};
use serde_json::{json, Value};
use tower::util::ServiceExt;

async fn response_json(resp: axum::response::Response) -> Value {
    let bytes = body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("bytes");
    serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}))
}

async fn seed_account(
    store: &SqliteStore,
    account_id: &str,
    provider_id: &str,
    upstream_base_url: Option<&str>,
) {
    let row = OAuthAccountRecord {
        account_id: account_id.to_string(),
        provider_id: provider_id.to_string(),
        display_name: format!("{provider_id}-acct"),
        email: format!("{account_id}@example.local"),
        priority: 0,
        enabled: true,
        last_used_unix: None,
        last_error: None,
        upstream_base_url: upstream_base_url.map(str::to_string),
        chat_path: None,
        responses_path: None,
        org_id: None,
        project_id: None,
        extra_headers_json: None,
    };
    store
        .upsert_oauth_account(&row)
        .await
        .expect("upsert account");
}

#[tokio::test]
async fn models_uses_fresh_google_cache_envelope() {
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    seed_account(
        &store,
        "google_cache_1",
        "google",
        Some("https://cloudcode-pa.googleapis.com"),
    )
    .await;

    let cache = json!({
        "version": 1,
        "fetched_at_unix": 9_999_999_999u64,
        "ttl_seconds": 300,
        "etag": "etag-1",
        "data": {
            "object": "list",
            "data": [{"id":"gemini-cache-only-model"}]
        }
    });
    store
        .set_app_state("google_models_google_cache_1", &cache.to_string())
        .await
        .expect("set cache");

    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:51824".to_string(),
    ));
    let req = Request::builder()
        .uri("/v1/models")
        .body(Body::empty())
        .expect("request");
    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 200);
    let bytes = body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let payload: Value = serde_json::from_slice(&bytes).expect("json");
    let ids: Vec<String> = payload
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    assert!(ids.iter().any(|id| id == "gemini-cache-only-model"));
}

#[tokio::test]
async fn models_ignores_stale_or_version_mismatch_cache() {
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    seed_account(
        &store,
        "google_cache_2",
        "google",
        Some("https://cloudcode-pa.googleapis.com"),
    )
    .await;

    let stale = json!({
        "version": 999,
        "fetched_at_unix": 1,
        "ttl_seconds": 1,
        "etag": "etag-bad",
        "data": {
            "object": "list",
            "data": [{"id":"gemini-should-not-appear"}]
        }
    });
    store
        .set_app_state("google_models_google_cache_2", &stale.to_string())
        .await
        .expect("set cache");

    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:51824".to_string(),
    ));
    let req = Request::builder()
        .uri("/v1/models")
        .body(Body::empty())
        .expect("request");
    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 200);
    let body = response_json(resp).await;
    let ids: Vec<String> = body
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();

    assert!(!ids.iter().any(|id| id == "gemini-should-not-appear"));
    assert!(ids.iter().any(|id| id == "gemini-2.5-flash"));
}
