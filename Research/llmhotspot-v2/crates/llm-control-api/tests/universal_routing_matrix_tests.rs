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

fn err_msg(v: &Value) -> String {
    v.pointer("/error/message")
        .and_then(Value::as_str)
        .or_else(|| v.get("message").and_then(Value::as_str))
        .unwrap_or("")
        .to_string()
}

#[tokio::test]
async fn routing_matrix_chat_endpoint_resolves_provider_from_model_prefix() {
    let _guard = test_env_lock::lock_env();
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:11434".to_string(),
    ));
    let auth = create_api_key(app.clone()).await;

    let cases = [
        ("gpt-oss-120b", "requires an enabled OpenAI OAuth account"),
        (
            "claude-opus-4-6",
            "requires an enabled Anthropic OAuth account",
        ),
        (
            "gemini-3.1-pro-preview",
            "requires an enabled Google OAuth account",
        ),
    ];

    for (model, expected) in cases {
        let req = Request::builder()
            .uri("/v1/chat/completions")
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", &auth)
            .body(Body::from(
                json!({
                    "model": model,
                    "messages":[{"role":"user","content":"hello"}],
                    "stream": false
                })
                .to_string(),
            ))
            .expect("request");
        let resp =
            tokio::time::timeout(std::time::Duration::from_secs(20), app.clone().oneshot(req))
                .await
                .expect("timed out waiting for response")
                .expect("response");
        assert_eq!(resp.status(), 400);
        let body = resp_json(resp).await;
        assert!(
            err_msg(&body).contains(expected),
            "model={model}, body={body}"
        );
    }
}

#[tokio::test]
async fn routing_matrix_messages_endpoint_accepts_cross_family_models() {
    let _guard = test_env_lock::lock_env();
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:11434".to_string(),
    ));
    let auth = create_api_key(app.clone()).await;

    let cases = [
        ("gpt-oss-120b", "unsupported model 'gpt-oss-120b'"),
        ("gemini-3-flash", "unsupported model 'gemini-3-flash'"),
    ];

    for (model, expected) in cases {
        let req = Request::builder()
            .uri("/v1/messages")
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", &auth)
            .body(Body::from(
                json!({
                    "model": model,
                    "max_tokens": 64,
                    "messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}],
                    "stream": false
                })
                .to_string(),
            ))
            .expect("request");
        let resp =
            tokio::time::timeout(std::time::Duration::from_secs(20), app.clone().oneshot(req))
                .await
                .expect("timed out waiting for response")
                .expect("response");
        assert_eq!(resp.status(), 400);
        let body = resp_json(resp).await;
        assert!(
            err_msg(&body).contains(expected),
            "model={model}, body={body}"
        );
    }
}

#[tokio::test]
async fn routing_matrix_google_endpoint_accepts_non_gemini_prefixes_structurally() {
    let _guard = test_env_lock::lock_env();
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:11434".to_string(),
    ));
    let auth = create_api_key(app.clone()).await;

    for model in ["gpt-oss-120b", "claude-opus-4-6", "gemini-3-flash"] {
        let req = Request::builder()
            .uri(format!("/v1/google/models/{model}:generateContent"))
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", &auth)
            .body(Body::from(
                json!({
                    "contents":[{"role":"user","parts":[{"text":"hello"}]}]
                })
                .to_string(),
            ))
            .expect("request");
        let resp =
            tokio::time::timeout(std::time::Duration::from_secs(20), app.clone().oneshot(req))
                .await
                .expect("timed out waiting for response")
                .expect("response");
        assert_eq!(resp.status(), 400, "model={model}");
    }
}
