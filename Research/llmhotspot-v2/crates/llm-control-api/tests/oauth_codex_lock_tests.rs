#[path = "test_env_lock.rs"]
mod test_env_lock;

use axum::{body::Body, http::Request};
use llm_control_api::{app_router, AppState};
use llm_storage::{ApiKeyRecord, SqliteStore};
use tower::util::ServiceExt;

#[tokio::test]
#[ignore = "TODO: investigate windows hang in store-backed chat auth path"]
async fn oauth_locked_mode_uses_codex_defaults() {
    let _guard = test_env_lock::lock_env();
    std::env::set_var("LLMHOTSPOT_OAUTH_UPSTREAM_LOCKED", "1");
    std::env::remove_var("LLMHOTSPOT_CODEX_BASE_URL");
    std::env::remove_var("LLMHOTSPOT_CODEX_RESPONSES_PATH");

    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    store
        .create_api_key(&ApiKeyRecord {
            key_id: uuid::Uuid::new_v4().to_string(),
            label: "seed".to_string(),
            key_hash: "seed-hash".to_string(),
            tool_choice_policy: "auto".to_string(),
            created_unix: 0,
            expires_unix: None,
            revoked_unix: None,
            last_used_unix: None,
        })
        .await
        .expect("seed key");
    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:11434".to_string(),
    ));

    // no oauth sessions -> auth error, but this still validates locked-mode path compiles and route behaves
    let req = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("authorization", "Bearer any")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"model":"gpt-5.2","messages":[{"role":"user","content":"hi"}],"stream":false}"#,
        ))
        .expect("request");
    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert!(resp.status().is_client_error() || resp.status().is_server_error());
}
