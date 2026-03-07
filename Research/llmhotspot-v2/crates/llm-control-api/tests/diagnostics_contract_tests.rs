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
    assert_eq!(resp.status(), 200);
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

async fn resp_json(resp: axum::response::Response) -> Value {
    let bytes = body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("bytes");
    serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}))
}

#[tokio::test]
async fn diagnostics_recent_request_contains_requested_resolved_and_routed_fields() {
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
            json!({
                "model":"gemini-3-flash-20260228",
                "messages":[{"role":"user","content":"hello"}],
                "stream":false
            })
            .to_string(),
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
    assert_eq!(diag_resp.status(), 200);
    let body = resp_json(diag_resp).await;
    let recent = body
        .get("recent_requests")
        .and_then(Value::as_array)
        .expect("recent_requests array");
    let last = recent.last().expect("at least one request");

    assert_eq!(
        last.get("requested_model").and_then(Value::as_str),
        Some("gemini-3-flash-20260228")
    );
    assert_eq!(
        last.get("resolved_model").and_then(Value::as_str),
        Some("gemini-3-flash")
    );
    assert_eq!(
        last.get("routed_provider").and_then(Value::as_str),
        Some("google")
    );
}

#[tokio::test]
async fn diagnostics_includes_doctor_last_run_field() {
    let _guard = test_env_lock::lock_env();
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:11434".to_string(),
    ));

    let req = Request::builder()
        .uri("/v1/diagnostics")
        .method("GET")
        .body(Body::empty())
        .expect("request");
    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 200);
    let body = resp_json(resp).await;
    assert!(
        body.get("doctor_last_run").is_some(),
        "doctor_last_run should be present in diagnostics payload"
    );
    let pending = body
        .get("runtime_ipc")
        .and_then(|v| v.get("pending_shell_deltas"))
        .expect("runtime_ipc.pending_shell_deltas");
    assert_eq!(pending.get("hits").and_then(Value::as_u64), Some(0));
    assert_eq!(pending.get("fallbacks").and_then(Value::as_u64), Some(0));
    assert_eq!(pending.get("fails").and_then(Value::as_u64), Some(0));
}
