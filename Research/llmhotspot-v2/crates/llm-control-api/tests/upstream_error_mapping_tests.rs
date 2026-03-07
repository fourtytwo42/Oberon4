#[path = "test_env_lock.rs"]
mod test_env_lock;

use axum::{body::Body, http::Request};
use llm_control_api::{app_router, AppState};
use llm_storage::SqliteStore;
use serde_json::json;
use tower::util::ServiceExt;

async fn response_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("bytes");
    serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}))
}

async fn response_text(resp: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("bytes");
    String::from_utf8_lossy(&bytes).to_string()
}

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
    let body = response_json(resp).await;
    let key = body
        .get("raw_key")
        .and_then(serde_json::Value::as_str)
        .expect("raw_key")
        .to_string();
    format!("Bearer {key}")
}

#[tokio::test]
async fn prefixed_model_errors_are_not_misreported_as_unsupported_model_list() {
    let app = app_router(AppState::default());
    let req = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("content-type", "application/json")
        .header("authorization", "Bearer key")
        .body(Body::from(
            json!({
                "model":"claude-4.6-opus-high-thinking",
                "messages":[{"role":"user","content":"hello"}],
                "stream":false
            })
            .to_string(),
        ))
        .expect("request");
    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn unsupported_prefix_returns_prefix_validation_error_not_auth() {
    let _guard = test_env_lock::lock_env();
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:51824".to_string(),
    ));
    let auth = create_api_key(app.clone()).await;

    let req = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("content-type", "application/json")
        .header("authorization", auth)
        .body(Body::from(
            json!({
                "model":"llama-3.3",
                "messages":[{"role":"user","content":"hello"}],
                "stream":false
            })
            .to_string(),
        ))
        .expect("request");
    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 400);
    let body = response_text(resp).await;
    assert!(!body.to_ascii_lowercase().contains("auth error"));
    assert!(body.contains("model must start with gpt-, claude-, or gemini-"));
}
