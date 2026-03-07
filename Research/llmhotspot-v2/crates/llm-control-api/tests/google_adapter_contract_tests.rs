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

fn msg(v: &Value) -> String {
    v.pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

#[tokio::test]
async fn google_generate_content_accepts_wrapped_request_payload() {
    let _guard = test_env_lock::lock_env();
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:11434".to_string(),
    ));
    let auth = create_api_key(app.clone()).await;

    let payload = include_str!("fixtures/google_generate_wrapped_request.json");
    let req = Request::builder()
        .uri("/v1/google/models/gpt-oss-120b:generateContent")
        .method("POST")
        .header("content-type", "application/json")
        .header("authorization", &auth)
        .body(Body::from(payload.to_string()))
        .expect("request");
    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 400);
    let body = resp_json(resp).await;
    assert!(
        !msg(&body).contains("invalid generateContent payload"),
        "wrapped payload should parse: {body}"
    );
}

#[tokio::test]
async fn google_generate_content_accepts_both_system_instruction_spellings() {
    let _guard = test_env_lock::lock_env();
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:11434".to_string(),
    ));
    let auth = create_api_key(app.clone()).await;

    for payload in [
        json!({
            "systemInstruction":{"parts":[{"text":"You are helpful"}]},
            "contents":[{"role":"user","parts":[{"text":"hello"}]}]
        }),
        json!({
            "system_instruction":{"parts":[{"text":"You are helpful"}]},
            "contents":[{"role":"user","parts":[{"text":"hello"}]}]
        }),
    ] {
        let req = Request::builder()
            .uri("/v1/google/models/gpt-oss-120b:generateContent")
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", &auth)
            .body(Body::from(payload.to_string()))
            .expect("request");
        let resp =
            tokio::time::timeout(std::time::Duration::from_secs(20), app.clone().oneshot(req))
                .await
                .expect("timed out waiting for response")
                .expect("response");
        assert_eq!(resp.status(), 400);
        let body = resp_json(resp).await;
        assert!(
            !msg(&body).contains("invalid generateContent payload"),
            "payload should parse for both spellings: {body}"
        );
    }
}

#[tokio::test]
async fn google_stream_route_uses_same_payload_contract() {
    let _guard = test_env_lock::lock_env();
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:11434".to_string(),
    ));
    let auth = create_api_key(app.clone()).await;

    let payload = include_str!("fixtures/google_generate_wrapped_request.json");
    let req = Request::builder()
        .uri("/v1/google/models/gpt-oss-120b:streamGenerateContent")
        .method("POST")
        .header("content-type", "application/json")
        .header("authorization", &auth)
        .body(Body::from(payload.to_string()))
        .expect("request");
    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 400);
    let body = resp_json(resp).await;
    assert!(
        !msg(&body).contains("invalid generateContent payload"),
        "stream route should parse same payload contract: {body}"
    );
}
