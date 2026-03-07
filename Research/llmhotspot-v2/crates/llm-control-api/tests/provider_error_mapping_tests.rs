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
async fn provider_account_missing_errors_are_prefix_based_and_endpoint_agnostic() {
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
        (
            "/v1/chat/completions",
            json!({"model":"gpt-oss-120b","messages":[{"role":"user","content":"hello"}],"stream":false}),
            "requires an enabled OpenAI OAuth account",
        ),
        (
            "/v1/chat/completions",
            json!({"model":"claude-sonnet-4-6","messages":[{"role":"user","content":"hello"}],"stream":false}),
            "requires an enabled Anthropic OAuth account",
        ),
        (
            "/v1/chat/completions",
            json!({"model":"gemini-3-flash","messages":[{"role":"user","content":"hello"}],"stream":false}),
            "requires an enabled Google OAuth account",
        ),
        (
            "/v1/messages",
            json!({"model":"gpt-oss-120b","max_tokens":64,"messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}],"stream":false}),
            "unsupported model 'gpt-oss-120b'",
        ),
    ];

    for (uri, body_json, expected) in cases {
        let req = Request::builder()
            .uri(uri)
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", &auth)
            .body(Body::from(body_json.to_string()))
            .expect("request");
        let resp =
            tokio::time::timeout(std::time::Duration::from_secs(20), app.clone().oneshot(req))
                .await
                .expect("timed out waiting for response")
                .expect("response");
        assert_eq!(resp.status(), 400, "uri={uri}");
        let body = resp_json(resp).await;
        assert!(err_msg(&body).contains(expected), "uri={uri}, body={body}");
    }
}

#[tokio::test]
async fn non_prefixed_model_is_rejected_fast() {
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
                "model":"llama-3.3",
                "messages":[{"role":"user","content":"hello"}],
                "stream": false
            })
            .to_string(),
        ))
        .expect("request");
    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 400);
    let body = resp_json(resp).await;
    assert!(
        err_msg(&body).contains("must start with gpt-, claude-, or gemini-"),
        "body={body}"
    );
}
