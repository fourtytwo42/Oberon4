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
async fn anthropic_complete_rejects_missing_state_with_classified_400() {
    let _guard = test_env_lock::lock_env();
    std::env::set_var("LLMHOTSPOT_BOOTSTRAP_TOKEN", "bootstrap-required");
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:51824".to_string(),
    ));

    let start_req = Request::builder()
        .uri("/v1/oauth/anthropic/start")
        .method("POST")
        .header("authorization", "Bearer bootstrap-required")
        .header("content-type", "application/json")
        .body(Body::from(r#"{}"#))
        .expect("start request");
    let start_resp = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        app.clone().oneshot(start_req),
    )
    .await
    .expect("timed out waiting for response")
    .expect("start response");
    assert_eq!(start_resp.status(), 200);
    let start_body = response_json(start_resp).await;
    let flow_id = start_body
        .get("flow_id")
        .and_then(Value::as_str)
        .expect("flow_id");

    let complete_req = Request::builder()
        .uri("/v1/oauth/anthropic/complete")
        .method("POST")
        .header("authorization", "Bearer bootstrap-required")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "flow_id": flow_id,
                "code": "dummy-code"
            })
            .to_string(),
        ))
        .expect("complete request");
    let complete_resp = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        app.clone().oneshot(complete_req),
    )
    .await
    .expect("timed out waiting for response")
    .expect("complete response");

    assert_eq!(complete_resp.status(), 400);
    let body = response_json(complete_resp).await;
    let message = body
        .get("error")
        .and_then(|v| v.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("");
    assert!(message.contains("oauth_error_class=state_mismatch"));
}

#[tokio::test]
async fn google_complete_rejects_wrong_state_with_classified_400() {
    let _guard = test_env_lock::lock_env();
    std::env::set_var("LLMHOTSPOT_BOOTSTRAP_TOKEN", "bootstrap-required");
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:51824".to_string(),
    ));

    let start_req = Request::builder()
        .uri("/v1/oauth/google/start")
        .method("POST")
        .header("authorization", "Bearer bootstrap-required")
        .header("content-type", "application/json")
        .body(Body::from(r#"{}"#))
        .expect("start request");
    let start_resp = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        app.clone().oneshot(start_req),
    )
    .await
    .expect("timed out waiting for response")
    .expect("start response");
    assert_eq!(start_resp.status(), 200);
    let start_body = response_json(start_resp).await;
    let flow_id = start_body
        .get("flow_id")
        .and_then(Value::as_str)
        .expect("flow_id");

    let complete_req = Request::builder()
        .uri("/v1/oauth/google/complete")
        .method("POST")
        .header("authorization", "Bearer bootstrap-required")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "flow_id": flow_id,
                "code": "dummy-code",
                "state": "wrong-state"
            })
            .to_string(),
        ))
        .expect("complete request");
    let complete_resp = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        app.oneshot(complete_req),
    )
    .await
    .expect("timed out waiting for response")
    .expect("complete response");

    assert_eq!(complete_resp.status(), 400);
    let body = response_json(complete_resp).await;
    let message = body
        .get("error")
        .and_then(|v| v.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("");
    assert!(message.contains("oauth_error_class=state_mismatch"));
}
