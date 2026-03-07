#[path = "test_env_lock.rs"]
mod test_env_lock;

use axum::{body::Body, http::Request};
use llm_control_api::{app_router, AppState};
use llm_storage::{OAuthAccountRecord, SqliteStore};
use serde_json::json;
use tower::util::ServiceExt;

async fn response_text(resp: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("bytes");
    String::from_utf8_lossy(&bytes).to_string()
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
async fn google_refresh_guard_rejects_account_mismatch_without_upstream_refresh() {
    let _guard = test_env_lock::lock_env();
    std::env::set_var("LLMHOTSPOT_BOOTSTRAP_TOKEN", "bootstrap-required");
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let account_id = "google_guard_target";
    seed_account(
        &store,
        account_id,
        "google",
        Some("https://cloudcode-pa.googleapis.com"),
    )
    .await;

    let token_info = json!({
        "access_token": "",
        "refresh_token": "r1",
        "access_expires_at": 0,
        "account_id": "different-account",
        "provider_id": "google"
    })
    .to_string();
    store
        .set_app_state(&format!("google_token_{account_id}"), &token_info)
        .await
        .expect("set token");

    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:51824".to_string(),
    ));
    let req = Request::builder()
        .uri(format!("/v1/oauth/google/usage/{account_id}"))
        .method("GET")
        .header("authorization", "Bearer bootstrap-required")
        .body(Body::empty())
        .expect("request");
    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 502);
    let body = response_text(resp).await;
    assert!(body.contains("oauth_error_class=storage"));
}

#[tokio::test]
async fn anthropic_refresh_guard_rejects_provider_mismatch() {
    let _guard = test_env_lock::lock_env();
    std::env::set_var("LLMHOTSPOT_BOOTSTRAP_TOKEN", "bootstrap-required");
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let account_id = "anthropic_guard_target";
    seed_account(
        &store,
        account_id,
        "anthropic",
        Some("https://api.anthropic.com"),
    )
    .await;

    let token_info = json!({
        "access_token": "",
        "refresh_token": "r1",
        "access_expires_at": 0,
        "account_id": account_id,
        "provider_id": "google"
    })
    .to_string();
    store
        .set_app_state(&format!("anthropic_token_{account_id}"), &token_info)
        .await
        .expect("set token");

    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:51824".to_string(),
    ));
    let req = Request::builder()
        .uri(format!("/v1/oauth/anthropic/models/{account_id}"))
        .method("GET")
        .header("authorization", "Bearer bootstrap-required")
        .body(Body::empty())
        .expect("request");
    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 502);
    let body = response_text(resp).await;
    assert!(body.contains("oauth_error_class=storage"));
}
