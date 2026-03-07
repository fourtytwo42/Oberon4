#[path = "test_env_lock.rs"]
mod test_env_lock;

use axum::{body::Body, http::Request};
use llm_control_api::{app_router, AppState};
use llm_storage::{OAuthAccountRecord, SqliteStore};
use serde_json::json;
use tower::util::ServiceExt;

#[tokio::test]
async fn concurrent_google_usage_requests_fail_consistently_without_panics() {
    let _guard = test_env_lock::lock_env();
    std::env::set_var("LLMHOTSPOT_BOOTSTRAP_TOKEN", "bootstrap-required");
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");

    let account_id = "google_test_account";
    let row = OAuthAccountRecord {
        account_id: account_id.to_string(),
        provider_id: "google".to_string(),
        display_name: "test".to_string(),
        email: "test@example.com".to_string(),
        priority: 0,
        enabled: true,
        last_used_unix: None,
        last_error: None,
        upstream_base_url: Some("https://cloudcode-pa.googleapis.com".to_string()),
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

    let token_info = json!({
        "access_token": "",
        "refresh_token": "",
        "access_expires_at": 0,
        "project_id": ""
    })
    .to_string();
    store
        .set_app_state(&format!("google_token_{account_id}"), &token_info)
        .await
        .expect("set token state");

    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:51824".to_string(),
    ));

    let mut tasks = Vec::new();
    for _ in 0..4 {
        let app_clone = app.clone();
        let uri = format!("/v1/oauth/google/usage/{account_id}");
        tasks.push(tokio::spawn(async move {
            let req = Request::builder()
                .uri(uri)
                .method("GET")
                .header("authorization", "Bearer bootstrap-required")
                .body(Body::empty())
                .expect("request");
            let resp =
                tokio::time::timeout(std::time::Duration::from_secs(20), app_clone.oneshot(req))
                    .await
                    .expect("timed out waiting for response")
                    .expect("response");
            resp.status().as_u16()
        }));
    }

    for task in tasks {
        let status = task.await.expect("task join");
        assert_eq!(status, 502);
    }
}
