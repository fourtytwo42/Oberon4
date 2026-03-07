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
        .unwrap_or("")
        .to_string()
}

#[tokio::test]
async fn chat_accepts_openai_anthropic_and_google_tool_shapes() {
    let _guard = test_env_lock::lock_env();
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:11434".to_string(),
    ));
    let auth = create_api_key(app.clone()).await;

    let payloads = vec![
        include_str!("fixtures/chat_openai_with_google_tools.json").to_string(),
        json!({
            "model":"gpt-oss-120b",
            "messages":[{"role":"user","content":"hello"}],
            "tools":[{"name":"read_file","input_schema":{"type":"object","properties":{"path":{"type":"string"}}}}],
            "stream":false
        })
        .to_string(),
        json!({
            "model":"gpt-oss-120b",
            "messages":[{"role":"user","content":"hello"}],
            "tools":[{"type":"function","function":{"name":"read_file","parameters":{"type":"object","properties":{"path":{"type":"string"}}}}}],
            "stream":false
        })
        .to_string(),
    ];

    for payload in payloads {
        let req = Request::builder()
            .uri("/v1/chat/completions")
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", &auth)
            .body(Body::from(payload))
            .expect("request");
        let resp =
            tokio::time::timeout(std::time::Duration::from_secs(20), app.clone().oneshot(req))
                .await
                .expect("timed out waiting for response")
                .expect("response");
        assert_eq!(resp.status(), 400);
        let body = resp_json(resp).await;
        let msg = err_msg(&body);
        assert!(
            !msg.contains("invalid tools["),
            "unexpected tool-shape rejection: {body}"
        );
        assert!(
            msg.contains("requires an enabled OpenAI OAuth account"),
            "expected provider gating after tool validation: {body}"
        );
    }
}

#[tokio::test]
async fn chat_rejects_malformed_google_function_declarations_shape() {
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
                "model":"gpt-oss-120b",
                "messages":[{"role":"user","content":"hello"}],
                "tools":[{"functionDeclarations":[{"name":"","parameters":"not-an-object"}]}],
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
    let body = resp_json(resp).await;
    assert!(
        err_msg(&body).contains("functionDeclarations"),
        "body={body}"
    );
}
