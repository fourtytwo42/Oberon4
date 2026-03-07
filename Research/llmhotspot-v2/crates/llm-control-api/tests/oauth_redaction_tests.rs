#[path = "test_env_lock.rs"]
mod test_env_lock;

use axum::{
    body::{self, Body},
    http::Request,
};
use llm_control_api::{app_router, AppState};
use llm_storage::SqliteStore;
use serde_json::Value;
use tower::util::ServiceExt;

async fn create_api_key(app: axum::Router) -> String {
    std::env::set_var("LLMHOTSPOT_BOOTSTRAP_TOKEN", "bootstrap-required");
    let req = Request::builder()
        .uri("/v1/keys/create")
        .method("POST")
        .header("content-type", "application/json")
        .header("authorization", "Bearer bootstrap-required")
        .body(Body::from(r#"{"label":"tests","expires_unix":null}"#))
        .expect("request");
    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.clone().oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    let bytes = body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let j: Value = serde_json::from_slice(&bytes).expect("json");
    let key = j
        .get("raw_key")
        .and_then(Value::as_str)
        .expect("raw_key")
        .to_string();
    format!("Bearer {key}")
}

#[tokio::test]
async fn diagnostics_recent_errors_do_not_leak_bearer_token_literal() {
    let _guard = test_env_lock::lock_env();
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:11434".to_string(),
    ));
    let auth = create_api_key(app.clone()).await;

    let req = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("content-type", "application/json")
        .header("authorization", &auth)
        .body(Body::from(
            r#"{"messages":[{"role":"user","content":"hello"}],"stream":false}"#,
        ))
        .expect("request");
    let _ = tokio::time::timeout(std::time::Duration::from_secs(20), app.clone().oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");

    let diag_req = Request::builder()
        .uri("/v1/diagnostics")
        .method("GET")
        .body(Body::empty())
        .expect("diag request");
    let diag_resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(diag_req))
        .await
        .expect("timed out waiting for response")
        .expect("diag response");
    let bytes = body::to_bytes(diag_resp.into_body(), usize::MAX)
        .await
        .expect("diag bytes");
    let raw = String::from_utf8_lossy(&bytes);

    assert!(!raw.contains("Bearer "));
    assert!(!raw.contains("access_token"));
    assert!(!raw.contains("refresh_token"));
}
