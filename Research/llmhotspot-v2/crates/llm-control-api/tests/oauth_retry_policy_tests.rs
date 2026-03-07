#[path = "test_env_lock.rs"]
mod test_env_lock;

use axum::{
    body::{self, Body},
    http::Request,
};
use llm_control_api::{app_router, AppState};
use llm_storage::SqliteStore;
use serde_json::{json, Value};
use tower::util::ServiceExt;

async fn response_json(resp: axum::response::Response) -> Value {
    let bytes = body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("bytes");
    serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}))
}

#[tokio::test]
async fn google_complete_unknown_flow_has_classified_error() {
    let _guard = test_env_lock::lock_env();
    std::env::set_var("LLMHOTSPOT_BOOTSTRAP_TOKEN", "bootstrap-required");
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:51824".to_string(),
    ));

    let req = Request::builder()
        .uri("/v1/oauth/google/complete")
        .method("POST")
        .header("authorization", "Bearer bootstrap-required")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"flow_id":"missing","code":"x","state":"y"}).to_string(),
        ))
        .expect("request");

    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 502);
    let body = response_json(resp).await;
    let message = body
        .get("error")
        .and_then(|v| v.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("");
    assert!(message.contains("oauth_error_class="));
}
