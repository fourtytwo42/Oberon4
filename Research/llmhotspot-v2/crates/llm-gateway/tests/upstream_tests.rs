use axum::{routing::post, Json, Router};
use llm_gateway::{
    forward_openai_json, resolve_codex_oauth_config, resolve_request_config, GatewayError,
    UpstreamRoutingProfile,
};
use serde_json::{json, Value};
use std::net::SocketAddr;
use tokio::net::TcpListener;

async fn handler(Json(v): Json<Value>) -> (axum::http::StatusCode, Json<Value>) {
    if v.get("input").and_then(Value::as_str) == Some("rate") {
        return (
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error":"rate"})),
        );
    }
    (
        axum::http::StatusCode::OK,
        Json(json!({"id":"r1","output_text":"ok"})),
    )
}

async fn spawn_mock() -> SocketAddr {
    let app = Router::new().route("/v1/chat/completions", post(handler));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    addr
}

#[tokio::test]
async fn forwards_success() {
    let addr = spawn_mock().await;
    let cfg = resolve_request_config(
        &format!("http://{addr}"),
        "/v1/chat/completions",
        false,
        None,
    );
    let out = forward_openai_json(
        &cfg,
        "token",
        &json!({"model":"gpt-5.2","input":"hello","stream":false}),
        None,
    )
    .await
    .expect("ok");
    assert_eq!(
        out.body.get("output_text").and_then(Value::as_str),
        Some("ok")
    );
}

#[tokio::test]
async fn maps_rate_limit() {
    let addr = spawn_mock().await;
    let cfg = resolve_request_config(
        &format!("http://{addr}"),
        "/v1/chat/completions",
        false,
        None,
    );
    let err = forward_openai_json(
        &cfg,
        "token",
        &json!({"model":"gpt-5.2","input":"rate","stream":false}),
        None,
    )
    .await
    .expect_err("rate error");
    assert_eq!(err.kind, GatewayError::RateLimit);
}

#[test]
fn routing_profile_overrides_defaults() {
    let mut profile = UpstreamRoutingProfile {
        base_url: Some("https://proxy.example.com".to_string()),
        chat_path: Some("/chat".to_string()),
        responses_path: Some("/responses".to_string()),
        org_id: Some("org_1".to_string()),
        project_id: Some("proj_1".to_string()),
        ..Default::default()
    };
    profile
        .extra_headers
        .insert("x-extra".to_string(), "ok".to_string());

    let cfg = resolve_request_config(
        "https://api.openai.com",
        "/v1/chat/completions",
        false,
        Some(&profile),
    );
    assert_eq!(cfg.base_url, "https://proxy.example.com");
    assert_eq!(cfg.path, "/chat");
    assert_eq!(cfg.org_id.as_deref(), Some("org_1"));
    assert_eq!(cfg.project_id.as_deref(), Some("proj_1"));
    assert_eq!(
        cfg.extra_headers.get("x-extra").map(String::as_str),
        Some("ok")
    );
}

#[test]
fn codex_oauth_defaults_are_locked() {
    std::env::remove_var("LLMHOTSPOT_CODEX_BASE_URL");
    std::env::remove_var("LLMHOTSPOT_CODEX_RESPONSES_PATH");
    let cfg = resolve_codex_oauth_config();
    assert_eq!(cfg.base_url, "https://chatgpt.com/backend-api");
    assert_eq!(cfg.path, "/codex/responses");
}
