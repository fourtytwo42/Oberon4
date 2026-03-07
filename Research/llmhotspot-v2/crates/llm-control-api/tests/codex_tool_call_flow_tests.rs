use axum::{
    body::{self, Body},
    extract::{Form, State},
    http::{HeaderMap, Request},
    routing::post,
    Json, Router,
};
use llm_auth::{OpenAiChatGptOAuthClient, OpenAiOAuthConfig};
use llm_control_api::{app_router, AppState};
use llm_storage::{OAuthAccountRecord, SqliteStore, StoredTokenRecord};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::net::TcpListener;
use tower::util::ServiceExt;

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<Value>>>,
    chatgpt_account_ids: Arc<Mutex<Vec<Option<String>>>>,
}

fn env_test_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn configure_test_env_defaults() {
    std::env::set_var("LLMHOTSPOT_ALLOW_UNAUTH_NO_KEYS", "1");
    std::env::set_var("LLMHOTSPOT_SECRET_STORE_APP_STATE_FALLBACK", "1");
}

fn oauth_cfg_for_mock(mock_addr: SocketAddr) -> OpenAiOAuthConfig {
    OpenAiOAuthConfig {
        token_url: format!("http://{mock_addr}/oauth/token"),
        ..Default::default()
    }
}

fn oauth_cfg_for_mock_with_jwt_claim_path(
    mock_addr: SocketAddr,
    jwt_claim_path: &str,
) -> OpenAiOAuthConfig {
    OpenAiOAuthConfig {
        token_url: format!("http://{mock_addr}/oauth/token"),
        jwt_claim_path: jwt_claim_path.to_string(),
        ..Default::default()
    }
}

async fn oauth_token_handler(
    Form(form): Form<HashMap<String, String>>,
) -> (axum::http::StatusCode, Json<Value>) {
    let refresh = form
        .get("refresh_token")
        .cloned()
        .unwrap_or_else(|| "missing".to_string());
    let access_token = match refresh.as_str() {
        "refresh_exact" => {
            "x.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjdF9leGFjdCJ9fQ.y".to_string()
        }
        "refresh_nested" => {
            "x.eyJjdXN0b20iOnsibmVzdGVkIjp7ImNoYXRncHRfYWNjb3VudF9pZCI6ImFjY3RfbmVzdGVkIn19fQ.y".to_string()
        }
        "refresh_missing" => "x.e30.y".to_string(),
        _ => format!("access_{refresh}"),
    };
    (
        axum::http::StatusCode::OK,
        Json(json!({
            "access_token": access_token,
            "refresh_token": refresh,
            "expires_in": 3600
        })),
    )
}

fn first_user_text(payload: &Value) -> Option<String> {
    payload
        .get("input")
        .and_then(Value::as_array)
        .and_then(|items| {
            items.iter().find_map(|item| {
                let role = item.get("role").and_then(Value::as_str)?;
                if role != "user" {
                    return None;
                }
                let content = item.get("content")?.as_array()?;
                for part in content {
                    if part.get("type").and_then(Value::as_str) == Some("input_text") {
                        if let Some(text) = part.get("text").and_then(Value::as_str) {
                            return Some(text.to_string());
                        }
                    }
                }
                None
            })
        })
}

fn has_function_call_output(payload: &Value) -> bool {
    payload
        .get("input")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|item| {
                item.get("type").and_then(Value::as_str) == Some("function_call_output")
            })
        })
}

async fn codex_handler(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> (
    axum::http::StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    state
        .requests
        .lock()
        .expect("mock state mutex")
        .push(payload.clone());
    let account_id = headers
        .get("chatgpt-account-id")
        .and_then(|v| v.to_str().ok())
        .map(ToString::to_string);
    state
        .chatgpt_account_ids
        .lock()
        .expect("mock state mutex")
        .push(account_id);

    let scenario = first_user_text(&payload).unwrap_or_default();
    if scenario.contains("SCENARIO_ORPHAN_OUTPUT") {
        let has_prev = payload
            .get("previous_response_id")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.trim().is_empty());
        if has_function_call_output(&payload) && !has_prev {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                r#"{"error":{"message":"No tool call found for function call output with call_id call_orphan.","type":"invalid_request_error","param":"input","code":null}}"#.to_string(),
            );
        }
    }

    if scenario.contains("SCENARIO_PREV_OUTPUT") {
        let has_prev = payload
            .get("previous_response_id")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.trim().is_empty());
        if !has_prev || !has_function_call_output(&payload) {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                r#"{"error":{"message":"missing previous_response_id or function_call_output","type":"invalid_request_error","param":"input","code":null}}"#.to_string(),
            );
        }
    }
    if scenario.contains("SCENARIO_RETRY_ORPHAN") && has_function_call_output(&payload) {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            r#"{"error":{"message":"No tool call found for function call output with call_id call_orphan.","type":"invalid_request_error","param":"input","code":null}}"#.to_string(),
        );
    }

    let sse = if scenario.contains("SCENARIO_TOOLCALL") {
        [
            "data: {\"type\":\"response.created\",\"response\":{\"model\":\"gpt-5.1\"}}",
            "",
            "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"fc_1\",\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"exec_command\",\"arguments\":\"\"}}",
            "",
            "data: {\"type\":\"response.function_call_arguments.done\",\"item_id\":\"fc_1\",\"arguments\":\"{\\\"cmd\\\":\\\"ls -la\\\"}\"}",
            "",
            "data: [DONE]",
            "",
        ]
        .join("\n")
    } else if scenario.contains("SCENARIO_LEAK_TEXT_DELTA") {
        [
            "data: {\"type\":\"response.created\",\"response\":{\"model\":\"gpt-5.1\"}}",
            "",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Clean intro.\\n\"}",
            "",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"*** Begin Patch\\n\"}",
            "",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"*** End Patch\\n\"}",
            "",
            "data: [DONE]",
            "",
        ]
        .join("\n")
    } else if scenario.contains("SCENARIO_LEAK_ITEM_TEXT") {
        [
            "data: {\"type\":\"response.created\",\"response\":{\"model\":\"gpt-5.1\"}}",
            "",
            "data: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"safe line\\n*** Begin Patch\\n*** Update File: src/main.rs\"}]}}",
            "",
            "data: [DONE]",
            "",
        ]
        .join("\n")
    } else if scenario.contains("SCENARIO_FALSE_POSITIVE_PROSE") {
        [
            "data: {\"type\":\"response.created\",\"response\":{\"model\":\"gpt-5.1\"}}",
            "",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"This prose explains patch workflows and recipient_name fields without being protocol data.\"}",
            "",
            "data: [DONE]",
            "",
        ]
        .join("\n")
    } else if scenario.contains("SCENARIO_MIXED_LEAK_AND_TOOLCALL") {
        [
            "data: {\"type\":\"response.created\",\"response\":{\"model\":\"gpt-5.1\"}}",
            "",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Before call\\n\"}",
            "",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"to=functions.exec_command\\n\"}",
            "",
            "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"fc_2\",\"type\":\"function_call\",\"call_id\":\"call_2\",\"name\":\"exec_command\",\"arguments\":\"\"}}",
            "",
            "data: {\"type\":\"response.function_call_arguments.done\",\"item_id\":\"fc_2\",\"arguments\":\"{\\\"cmd\\\":\\\"pwd\\\"}\"}",
            "",
            "data: [DONE]",
            "",
        ]
        .join("\n")
    } else if scenario.contains("SCENARIO_LEAK_JSON_SPACED") {
        [
            "data: {\"type\":\"response.created\",\"response\":{\"model\":\"gpt-5.1\"}}",
            "",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Visible line\\n\"}",
            "",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"{\\\"recipient_name\\\": \\\"functions.exec_command\\\", \\\"parameters\\\": {\\\"cmd\\\":\\\"pwd\\\"}}\\n\"}",
            "",
            "data: [DONE]",
            "",
        ]
        .join("\n")
    } else if scenario.contains("SCENARIO_CUSTOM_TOOL") {
        [
            "data: {\"type\":\"response.created\",\"response\":{\"model\":\"gpt-5.1\"}}",
            "",
            "data: {\"type\":\"response.custom_tool_call_input.delta\",\"item_id\":\"ct_1\",\"name\":\"ApplyPatch\",\"delta\":\"*** Begin Patch\\n\"}",
            "",
            "data: {\"type\":\"response.custom_tool_call_input.delta\",\"item_id\":\"ct_1\",\"name\":\"ApplyPatch\",\"delta\":\"*** End Patch\\n\"}",
            "",
            "data: [DONE]",
            "",
        ]
        .join("\n")
    } else if scenario.contains("SCENARIO_PROGRAMMING") {
        [
            "data: {\"type\":\"response.created\",\"response\":{\"model\":\"gpt-5.1\"}}",
            "",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Use a HashMap and return Result.\"}",
            "",
            "data: [DONE]",
            "",
        ]
        .join("\n")
    } else {
        [
            "data: {\"type\":\"response.created\",\"response\":{\"model\":\"gpt-5.1\"}}",
            "",
            "data: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"message-only-output\"}]}}",
            "",
            "data: [DONE]",
            "",
        ]
        .join("\n")
    };

    (
        axum::http::StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
        sse,
    )
}

async fn spawn_mock() -> (SocketAddr, MockState) {
    let state = MockState::default();
    let app = Router::new()
        .route("/oauth/token", post(oauth_token_handler))
        .route("/codex/responses", post(codex_handler))
        .with_state(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (addr, state)
}

fn test_account() -> OAuthAccountRecord {
    OAuthAccountRecord {
        account_id: "acct_1".to_string(),
        provider_id: "openai".to_string(),
        display_name: "Account 1".to_string(),
        email: "acct1@example.com".to_string(),
        priority: 0,
        enabled: true,
        last_used_unix: None,
        last_error: None,
        upstream_base_url: None,
        chat_path: None,
        responses_path: None,
        org_id: None,
        project_id: None,
        extra_headers_json: None,
    }
}

#[tokio::test]
async fn codex_tool_calls_and_tool_context_roundtrip() {
    let _guard = env_test_lock().lock().await;
    configure_test_env_defaults();
    let (mock_addr, mock_state) = spawn_mock().await;
    std::env::set_var("LLMHOTSPOT_OAUTH_UPSTREAM_LOCKED", "1");
    std::env::set_var("LLMHOTSPOT_CODEX_BASE_URL", format!("http://{mock_addr}"));
    std::env::set_var("LLMHOTSPOT_CODEX_RESPONSES_PATH", "/codex/responses");

    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    store
        .upsert_oauth_account(&test_account())
        .await
        .expect("upsert oauth account");
    let token = StoredTokenRecord::new("default", "acct_1", "refresh_1");
    store.save_token(&token).await.expect("save token");

    let mut state = AppState::with_store(store, "http://127.0.0.1:11434".to_string());
    let oauth_cfg = oauth_cfg_for_mock(mock_addr);
    state.oauth_client = OpenAiChatGptOAuthClient::new(oauth_cfg);
    let app = app_router(state);

    let tool_call_req = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("authorization", "Bearer any")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"model":"gpt-5.2","stream":false,"messages":[{"role":"user","content":"SCENARIO_TOOLCALL"}]}"#,
        ))
        .expect("request");
    let tool_call_resp =
        tokio::time::timeout(Duration::from_secs(20), app.clone().oneshot(tool_call_req))
            .await
            .expect("timed out waiting for response")
            .expect("response");
    assert_eq!(tool_call_resp.status(), 200);
    let tool_call_body = body::to_bytes(tool_call_resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let tool_call_json: Value = serde_json::from_slice(&tool_call_body).expect("json");
    assert_eq!(
        tool_call_json
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str),
        Some("tool_calls")
    );
    assert_eq!(
        tool_call_json
            .pointer("/choices/0/message/tool_calls/0/function/name")
            .and_then(Value::as_str),
        Some("exec_command")
    );

    let programming_with_tool_context_req = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("authorization", "Bearer any")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{
              "model":"gpt-5.2",
              "stream":false,
              "messages":[
                {"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"exec_command","arguments":"{\"cmd\":\"pwd\"}"}}]},
                {"role":"tool","tool_call_id":"call_1","content":"{\"stdout\":\"/repo\"}"},
                {"role":"user","content":"SCENARIO_PROGRAMMING write a Rust parser"}
              ]
            }"#,
        ))
        .expect("request");
    let programming_resp = tokio::time::timeout(
        Duration::from_secs(20),
        app.clone().oneshot(programming_with_tool_context_req),
    )
    .await
    .expect("timed out waiting for response")
    .expect("response");
    assert_eq!(programming_resp.status(), 200);
    let programming_body = body::to_bytes(programming_resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let programming_json: Value = serde_json::from_slice(&programming_body).expect("json");
    assert!(programming_json
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .contains("HashMap"));

    let message_only_req = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("authorization", "Bearer any")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"model":"gpt-5.2","stream":false,"messages":[{"role":"user","content":"SCENARIO_MESSAGE_ONLY"}]}"#,
        ))
        .expect("request");
    let message_only_resp =
        tokio::time::timeout(Duration::from_secs(20), app.oneshot(message_only_req))
            .await
            .expect("timed out waiting for response")
            .expect("response");
    assert_eq!(message_only_resp.status(), 200);
    let message_only_body = body::to_bytes(message_only_resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let message_only_json: Value = serde_json::from_slice(&message_only_body).expect("json");
    assert_eq!(
        message_only_json
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str),
        Some("message-only-output")
    );

    let seen = mock_state
        .requests
        .lock()
        .expect("mock state mutex")
        .clone();
    assert!(seen.len() >= 3);
    let programming_payload = &seen[1];
    let input = programming_payload
        .get("input")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(input.iter().any(|item| {
        item.get("type").and_then(Value::as_str) == Some("function_call")
            && item.get("name").and_then(Value::as_str) == Some("exec_command")
    }));
    assert!(input.iter().any(|item| {
        item.get("type").and_then(Value::as_str) == Some("function_call_output")
            && item.get("call_id").and_then(Value::as_str) == Some("call_1")
    }));
}

#[tokio::test]
async fn chat_completions_accepts_responses_style_input_via_shim() {
    let _guard = env_test_lock().lock().await;
    configure_test_env_defaults();
    let (mock_addr, _mock_state) = spawn_mock().await;
    std::env::set_var("LLMHOTSPOT_OAUTH_UPSTREAM_LOCKED", "1");
    std::env::set_var("LLMHOTSPOT_CODEX_BASE_URL", format!("http://{mock_addr}"));
    std::env::set_var("LLMHOTSPOT_CODEX_RESPONSES_PATH", "/codex/responses");

    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    store
        .upsert_oauth_account(&test_account())
        .await
        .expect("upsert oauth account");
    let token = StoredTokenRecord::new("default", "acct_1", "refresh_1");
    store.save_token(&token).await.expect("save token");

    let mut state = AppState::with_store(store, "http://127.0.0.1:11434".to_string());
    let oauth_cfg = oauth_cfg_for_mock(mock_addr);
    state.oauth_client = OpenAiChatGptOAuthClient::new(oauth_cfg);
    let app = app_router(state);

    // Non-streaming: send Responses-format `input[]` to `/v1/chat/completions`; shim should return Chat Completions JSON.
    let req = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("authorization", "Bearer any")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"model":"gpt-5.2","stream":false,"input":[{"role":"user","content":[{"type":"input_text","text":"SCENARIO_TOOLCALL"}]}]}"#,
        ))
        .expect("request");
    let resp = tokio::time::timeout(Duration::from_secs(20), app.clone().oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 200);
    let body = body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(
        json.pointer("/choices/0/finish_reason")
            .and_then(Value::as_str),
        Some("tool_calls")
    );

    // Streaming: custom tool deltas should be bridged to tool_calls chunks.
    let req_stream = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("authorization", "Bearer any")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"model":"gpt-5.2","stream":true,"input":[{"role":"user","content":[{"type":"input_text","text":"SCENARIO_CUSTOM_TOOL"}]}]}"#,
        ))
        .expect("request");
    let resp_stream = tokio::time::timeout(Duration::from_secs(20), app.oneshot(req_stream))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp_stream.status(), 200);
    let body = body::to_bytes(resp_stream.into_body(), usize::MAX)
        .await
        .expect("body");
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("chat.completion.chunk"));
    assert!(text.contains("tool_calls"));
    assert!(text.contains("ApplyPatch"));
    assert!(text.contains("[DONE]"));
}

#[tokio::test]
async fn chat_completions_non_stream_filters_protocol_leakage_text() {
    let _guard = env_test_lock().lock().await;
    configure_test_env_defaults();
    let (mock_addr, _mock_state) = spawn_mock().await;
    std::env::set_var("LLMHOTSPOT_OAUTH_UPSTREAM_LOCKED", "1");
    std::env::set_var("LLMHOTSPOT_CODEX_BASE_URL", format!("http://{mock_addr}"));
    std::env::set_var("LLMHOTSPOT_CODEX_RESPONSES_PATH", "/codex/responses");

    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    store
        .upsert_oauth_account(&test_account())
        .await
        .expect("upsert oauth account");
    let token = StoredTokenRecord::new("default", "acct_1", "refresh_1");
    store.save_token(&token).await.expect("save token");

    let mut state = AppState::with_store(store, "http://127.0.0.1:11434".to_string());
    let oauth_cfg = oauth_cfg_for_mock(mock_addr);
    state.oauth_client = OpenAiChatGptOAuthClient::new(oauth_cfg);
    let app = app_router(state);

    let req = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("authorization", "Bearer any")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"model":"gpt-5.2","stream":false,"messages":[{"role":"user","content":"SCENARIO_LEAK_ITEM_TEXT"}]}"#,
        ))
        .expect("request");
    let resp = tokio::time::timeout(Duration::from_secs(20), app.clone().oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 200);
    let body = body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: Value = serde_json::from_slice(&body).expect("json");
    let content = json
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(content.contains("safe line"));
    assert!(!content.contains("*** Begin Patch"));
    assert!(!content.contains("*** Update File"));
}

#[tokio::test]
async fn chat_completions_stream_filters_protocol_leakage_text() {
    let _guard = env_test_lock().lock().await;
    configure_test_env_defaults();
    let (mock_addr, _mock_state) = spawn_mock().await;
    std::env::set_var("LLMHOTSPOT_OAUTH_UPSTREAM_LOCKED", "1");
    std::env::set_var("LLMHOTSPOT_CODEX_BASE_URL", format!("http://{mock_addr}"));
    std::env::set_var("LLMHOTSPOT_CODEX_RESPONSES_PATH", "/codex/responses");

    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    store
        .upsert_oauth_account(&test_account())
        .await
        .expect("upsert oauth account");
    let token = StoredTokenRecord::new("default", "acct_1", "refresh_1");
    store.save_token(&token).await.expect("save token");

    let mut state = AppState::with_store(store, "http://127.0.0.1:11434".to_string());
    let oauth_cfg = oauth_cfg_for_mock(mock_addr);
    state.oauth_client = OpenAiChatGptOAuthClient::new(oauth_cfg);
    let app = app_router(state);

    let req = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("authorization", "Bearer any")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"model":"gpt-5.2","stream":true,"messages":[{"role":"user","content":"SCENARIO_LEAK_TEXT_DELTA"}]}"#,
        ))
        .expect("request");
    let resp = tokio::time::timeout(Duration::from_secs(20), app.clone().oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 200);
    let body = body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("Clean intro."));
    assert!(!text.contains("*** Begin Patch"));
    assert!(!text.contains("*** End Patch"));
}

#[tokio::test]
async fn chat_completions_stream_filters_spaced_json_protocol_markers() {
    let _guard = env_test_lock().lock().await;
    configure_test_env_defaults();
    let (mock_addr, _mock_state) = spawn_mock().await;
    std::env::set_var("LLMHOTSPOT_OAUTH_UPSTREAM_LOCKED", "1");
    std::env::set_var("LLMHOTSPOT_CODEX_BASE_URL", format!("http://{mock_addr}"));
    std::env::set_var("LLMHOTSPOT_CODEX_RESPONSES_PATH", "/codex/responses");

    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    store
        .upsert_oauth_account(&test_account())
        .await
        .expect("upsert oauth account");
    let token = StoredTokenRecord::new("default", "acct_1", "refresh_1");
    store.save_token(&token).await.expect("save token");

    let mut state = AppState::with_store(store, "http://127.0.0.1:11434".to_string());
    let oauth_cfg = oauth_cfg_for_mock(mock_addr);
    state.oauth_client = OpenAiChatGptOAuthClient::new(oauth_cfg);
    let app = app_router(state);

    let req = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("authorization", "Bearer any")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"model":"gpt-5.2","stream":true,"messages":[{"role":"user","content":"SCENARIO_LEAK_JSON_SPACED"}]}"#,
        ))
        .expect("request");
    let resp = tokio::time::timeout(Duration::from_secs(20), app.clone().oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 200);
    let body = body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("Visible line"));
    assert!(!text.contains("\"recipient_name\": \"functions.exec_command\""));
}

#[tokio::test]
async fn chat_completions_mixed_leak_and_tool_calls_preserves_tool_shape() {
    let _guard = env_test_lock().lock().await;
    configure_test_env_defaults();
    let (mock_addr, _mock_state) = spawn_mock().await;
    std::env::set_var("LLMHOTSPOT_OAUTH_UPSTREAM_LOCKED", "1");
    std::env::set_var("LLMHOTSPOT_CODEX_BASE_URL", format!("http://{mock_addr}"));
    std::env::set_var("LLMHOTSPOT_CODEX_RESPONSES_PATH", "/codex/responses");

    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    store
        .upsert_oauth_account(&test_account())
        .await
        .expect("upsert oauth account");
    let token = StoredTokenRecord::new("default", "acct_1", "refresh_1");
    store.save_token(&token).await.expect("save token");

    let mut state = AppState::with_store(store, "http://127.0.0.1:11434".to_string());
    let oauth_cfg = oauth_cfg_for_mock(mock_addr);
    state.oauth_client = OpenAiChatGptOAuthClient::new(oauth_cfg);
    let app = app_router(state);

    let req = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("authorization", "Bearer any")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"model":"gpt-5.2","stream":false,"messages":[{"role":"user","content":"SCENARIO_MIXED_LEAK_AND_TOOLCALL"}]}"#,
        ))
        .expect("request");
    let resp = tokio::time::timeout(Duration::from_secs(20), app.clone().oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 200);
    let body = body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: Value = serde_json::from_slice(&body).expect("json");

    assert_eq!(
        json.pointer("/choices/0/finish_reason")
            .and_then(Value::as_str),
        Some("tool_calls")
    );
    assert_eq!(
        json.pointer("/choices/0/message/tool_calls/0/id")
            .and_then(Value::as_str),
        Some("call_2")
    );
    assert_eq!(
        json.pointer("/choices/0/message/tool_calls/0/function/name")
            .and_then(Value::as_str),
        Some("exec_command")
    );
    assert_eq!(
        json.pointer("/choices/0/message/tool_calls/0/function/arguments")
            .and_then(Value::as_str),
        Some("{\"cmd\":\"pwd\"}")
    );
    let content = json
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(content.contains("Before call"));
    assert!(!content.contains("to=functions.exec_command"));
}

#[tokio::test]
async fn chat_completions_false_positive_prose_is_preserved() {
    let _guard = env_test_lock().lock().await;
    configure_test_env_defaults();
    let (mock_addr, _mock_state) = spawn_mock().await;
    std::env::set_var("LLMHOTSPOT_OAUTH_UPSTREAM_LOCKED", "1");
    std::env::set_var("LLMHOTSPOT_CODEX_BASE_URL", format!("http://{mock_addr}"));
    std::env::set_var("LLMHOTSPOT_CODEX_RESPONSES_PATH", "/codex/responses");

    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    store
        .upsert_oauth_account(&test_account())
        .await
        .expect("upsert oauth account");
    let token = StoredTokenRecord::new("default", "acct_1", "refresh_1");
    store.save_token(&token).await.expect("save token");

    let mut state = AppState::with_store(store, "http://127.0.0.1:11434".to_string());
    let oauth_cfg = oauth_cfg_for_mock(mock_addr);
    state.oauth_client = OpenAiChatGptOAuthClient::new(oauth_cfg);
    let app = app_router(state);

    let req = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("authorization", "Bearer any")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"model":"gpt-5.2","stream":false,"messages":[{"role":"user","content":"SCENARIO_FALSE_POSITIVE_PROSE"}]}"#,
        ))
        .expect("request");
    let resp = tokio::time::timeout(Duration::from_secs(20), app.clone().oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 200);
    let body = body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: Value = serde_json::from_slice(&body).expect("json");
    let content = json
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(content.contains("patch workflows"));
    assert!(content.contains("recipient_name fields"));
}

#[tokio::test]
async fn responses_non_stream_sanitizes_text_and_keeps_function_call_output() {
    let _guard = env_test_lock().lock().await;
    configure_test_env_defaults();
    let (mock_addr, _mock_state) = spawn_mock().await;
    std::env::set_var("LLMHOTSPOT_OAUTH_UPSTREAM_LOCKED", "1");
    std::env::set_var("LLMHOTSPOT_CODEX_BASE_URL", format!("http://{mock_addr}"));
    std::env::set_var("LLMHOTSPOT_CODEX_RESPONSES_PATH", "/codex/responses");

    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    store
        .upsert_oauth_account(&test_account())
        .await
        .expect("upsert oauth account");
    let token = StoredTokenRecord::new("default", "acct_1", "refresh_1");
    store.save_token(&token).await.expect("save token");

    let mut state = AppState::with_store(store, "http://127.0.0.1:11434".to_string());
    let oauth_cfg = oauth_cfg_for_mock(mock_addr);
    state.oauth_client = OpenAiChatGptOAuthClient::new(oauth_cfg);
    let app = app_router(state);

    let req = Request::builder()
        .uri("/v1/responses")
        .method("POST")
        .header("authorization", "Bearer any")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"model":"gpt-5.2","stream":false,"input":"SCENARIO_MIXED_LEAK_AND_TOOLCALL"}"#,
        ))
        .expect("request");
    let resp = tokio::time::timeout(Duration::from_secs(20), app.clone().oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 200);
    let body = body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json.get("object").and_then(Value::as_str), Some("response"));
    assert_eq!(
        json.get("status").and_then(Value::as_str),
        Some("incomplete")
    );
    assert_eq!(
        json.pointer("/output/0/type").and_then(Value::as_str),
        Some("function_call")
    );
    assert_eq!(
        json.pointer("/output/0/name").and_then(Value::as_str),
        Some("exec_command")
    );
    assert_eq!(
        json.pointer("/output/0/arguments").and_then(Value::as_str),
        Some("{\"cmd\":\"pwd\"}")
    );
    assert_eq!(
        json.pointer("/output/1/type").and_then(Value::as_str),
        Some("message")
    );
    let text = json
        .pointer("/output/1/content/0/text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(text.contains("Before call"));
    assert!(!text.contains("to=functions.exec_command"));
}

#[tokio::test]
async fn responses_endpoint_returns_responses_object_in_locked_mode() {
    let _guard = env_test_lock().lock().await;
    configure_test_env_defaults();
    let (mock_addr, _mock_state) = spawn_mock().await;
    std::env::set_var("LLMHOTSPOT_OAUTH_UPSTREAM_LOCKED", "1");
    std::env::set_var("LLMHOTSPOT_CODEX_BASE_URL", format!("http://{mock_addr}"));
    std::env::set_var("LLMHOTSPOT_CODEX_RESPONSES_PATH", "/codex/responses");

    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    store
        .upsert_oauth_account(&test_account())
        .await
        .expect("upsert oauth account");
    let token = StoredTokenRecord::new("default", "acct_1", "refresh_1");
    store.save_token(&token).await.expect("save token");

    let mut state = AppState::with_store(store, "http://127.0.0.1:11434".to_string());
    let oauth_cfg = oauth_cfg_for_mock(mock_addr);
    state.oauth_client = OpenAiChatGptOAuthClient::new(oauth_cfg);
    let app = app_router(state);

    // Non-streaming should synthesize a Responses object (Codex always streams upstream).
    let req = Request::builder()
        .uri("/v1/responses")
        .method("POST")
        .header("authorization", "Bearer any")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"model":"gpt-5.2","stream":false,"input":"SCENARIO_PROGRAMMING"}"#,
        ))
        .expect("request");
    let resp = tokio::time::timeout(Duration::from_secs(20), app.clone().oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 200);
    let body = body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json.get("object").and_then(Value::as_str), Some("response"));
    assert!(json.get("output").and_then(Value::as_array).is_some());

    // Streaming should pass through raw Responses SSE.
    let req_stream = Request::builder()
        .uri("/v1/responses")
        .method("POST")
        .header("authorization", "Bearer any")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"model":"gpt-5.2","stream":true,"input":"SCENARIO_PROGRAMMING"}"#,
        ))
        .expect("request");
    let resp_stream = tokio::time::timeout(Duration::from_secs(20), app.oneshot(req_stream))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp_stream.status(), 200);
    let body = body::to_bytes(resp_stream.into_body(), usize::MAX)
        .await
        .expect("body");
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("response.created"));
}

#[tokio::test]
async fn responses_orphan_tool_output_is_pruned_before_codex() {
    let _guard = env_test_lock().lock().await;
    configure_test_env_defaults();
    let (mock_addr, mock_state) = spawn_mock().await;
    std::env::set_var("LLMHOTSPOT_OAUTH_UPSTREAM_LOCKED", "1");
    std::env::set_var("LLMHOTSPOT_CODEX_BASE_URL", format!("http://{mock_addr}"));
    std::env::set_var("LLMHOTSPOT_CODEX_RESPONSES_PATH", "/codex/responses");

    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    store
        .upsert_oauth_account(&test_account())
        .await
        .expect("upsert oauth account");
    let token = StoredTokenRecord::new("default", "acct_1", "refresh_1");
    store.save_token(&token).await.expect("save token");

    let mut state = AppState::with_store(store, "http://127.0.0.1:11434".to_string());
    let oauth_cfg = oauth_cfg_for_mock(mock_addr);
    state.oauth_client = OpenAiChatGptOAuthClient::new(oauth_cfg);
    let app = app_router(state);

    let req = Request::builder()
        .uri("/v1/responses")
        .method("POST")
        .header("authorization", "Bearer any")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{
                "model":"gpt-5.2",
                "stream":false,
                "input":[
                    {"type":"function_call_output","call_id":"call_orphan","output":"ok"},
                    {"role":"user","content":[{"type":"input_text","text":"SCENARIO_ORPHAN_OUTPUT"}]}
                ]
            }"#,
        ))
        .expect("request");
    let resp = tokio::time::timeout(Duration::from_secs(20), app.clone().oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 200);

    let seen = mock_state
        .requests
        .lock()
        .expect("mock state mutex")
        .clone();
    let last = seen.last().expect("captured payload");
    assert!(
        !has_function_call_output(last),
        "orphan function_call_output should be pruned before forwarding"
    );
}

#[tokio::test]
async fn responses_tool_output_with_previous_response_id_is_forwarded_to_codex() {
    let _guard = env_test_lock().lock().await;
    configure_test_env_defaults();
    let (mock_addr, mock_state) = spawn_mock().await;
    std::env::set_var("LLMHOTSPOT_OAUTH_UPSTREAM_LOCKED", "1");
    std::env::set_var("LLMHOTSPOT_CODEX_BASE_URL", format!("http://{mock_addr}"));
    std::env::set_var("LLMHOTSPOT_CODEX_RESPONSES_PATH", "/codex/responses");

    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    store
        .upsert_oauth_account(&test_account())
        .await
        .expect("upsert oauth account");
    let token = StoredTokenRecord::new("default", "acct_1", "refresh_1");
    store.save_token(&token).await.expect("save token");

    let mut state = AppState::with_store(store, "http://127.0.0.1:11434".to_string());
    let oauth_cfg = oauth_cfg_for_mock(mock_addr);
    state.oauth_client = OpenAiChatGptOAuthClient::new(oauth_cfg);
    let app = app_router(state);

    let req = Request::builder()
        .uri("/v1/responses")
        .method("POST")
        .header("authorization", "Bearer any")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{
                "model":"gpt-5.2",
                "stream":false,
                "previous_response_id":"resp_prev_1",
                "input":[
                    {"type":"function_call_output","call_id":"call_orphan","output":"ok"},
                    {"role":"user","content":[{"type":"input_text","text":"SCENARIO_PREV_OUTPUT"}]}
                ]
            }"#,
        ))
        .expect("request");
    let resp = tokio::time::timeout(Duration::from_secs(20), app.clone().oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 200);

    let seen = mock_state
        .requests
        .lock()
        .expect("mock state mutex")
        .clone();
    let last = seen.last().expect("captured payload");
    assert_eq!(
        last.get("previous_response_id").and_then(Value::as_str),
        Some("resp_prev_1")
    );
    assert!(
        has_function_call_output(last),
        "function_call_output should be preserved when previous_response_id is present"
    );
}

#[tokio::test]
async fn responses_orphan_tool_output_retries_once_and_recovers() {
    let _guard = env_test_lock().lock().await;
    configure_test_env_defaults();
    let (mock_addr, mock_state) = spawn_mock().await;
    std::env::set_var("LLMHOTSPOT_OAUTH_UPSTREAM_LOCKED", "1");
    std::env::set_var("LLMHOTSPOT_CODEX_BASE_URL", format!("http://{mock_addr}"));
    std::env::set_var("LLMHOTSPOT_CODEX_RESPONSES_PATH", "/codex/responses");

    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    store
        .upsert_oauth_account(&test_account())
        .await
        .expect("upsert oauth account");
    let token = StoredTokenRecord::new("default", "acct_1", "refresh_1");
    store.save_token(&token).await.expect("save token");

    let mut state = AppState::with_store(store, "http://127.0.0.1:11434".to_string());
    let oauth_cfg = oauth_cfg_for_mock(mock_addr);
    state.oauth_client = OpenAiChatGptOAuthClient::new(oauth_cfg);
    let app = app_router(state);

    let req = Request::builder()
        .uri("/v1/responses")
        .method("POST")
        .header("authorization", "Bearer any")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{
                "model":"gpt-5.2",
                "stream":false,
                "input":[
                    {"type":"function_call_output","call_id":"call_orphan","output":"ok"},
                    {"role":"user","content":[{"type":"input_text","text":"SCENARIO_RETRY_ORPHAN"}]}
                ]
            }"#,
        ))
        .expect("request");
    let resp = tokio::time::timeout(Duration::from_secs(20), app.clone().oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 200);

    let seen = mock_state
        .requests
        .lock()
        .expect("mock state mutex")
        .clone();
    assert!(
        seen.len() == 1 || seen.len() == 2,
        "expected either pre-send prune (1 request) or retry-after-prune (2 requests)"
    );
    if seen.len() == 1 {
        assert!(
            !has_function_call_output(&seen[0]),
            "single request path should already be pruned"
        );
    } else {
        assert!(has_function_call_output(&seen[0]));
        assert!(!has_function_call_output(&seen[1]));
    }
}

#[tokio::test]
async fn codex_headers_include_chatgpt_account_id_for_claim_variants() {
    let _guard = env_test_lock().lock().await;
    configure_test_env_defaults();
    let (mock_addr, mock_state) = spawn_mock().await;
    std::env::set_var("LLMHOTSPOT_OAUTH_UPSTREAM_LOCKED", "1");
    std::env::set_var("LLMHOTSPOT_CODEX_BASE_URL", format!("http://{mock_addr}"));
    std::env::set_var("LLMHOTSPOT_CODEX_RESPONSES_PATH", "/codex/responses");

    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    store
        .upsert_oauth_account(&test_account())
        .await
        .expect("upsert oauth account");

    let mut state = AppState::with_store(store.clone(), "http://127.0.0.1:11434".to_string());
    let oauth_cfg =
        oauth_cfg_for_mock_with_jwt_claim_path(mock_addr, "https://api.openai.com/auth/");
    state.oauth_client = OpenAiChatGptOAuthClient::new(oauth_cfg);
    let app = app_router(state);

    let cases = [
        ("refresh_exact", Some("acct_exact")),
        ("refresh_nested", Some("acct_nested")),
        ("refresh_missing", None),
    ];

    for (refresh_token, expected_header) in cases {
        let token = StoredTokenRecord::new("default", "acct_1", refresh_token);
        store.save_token(&token).await.expect("save token");

        let req = Request::builder()
            .uri("/v1/chat/completions")
            .method("POST")
            .header("authorization", "Bearer any")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"model":"gpt-5.2","stream":false,"messages":[{"role":"user","content":"SCENARIO_PROGRAMMING header-check"}]}"#,
            ))
            .expect("request");
        let resp = tokio::time::timeout(Duration::from_secs(20), app.clone().oneshot(req))
            .await
            .expect("timed out waiting for response")
            .expect("response");
        assert_eq!(resp.status(), 200);
        let _ = body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body");

        let seen_headers = mock_state
            .chatgpt_account_ids
            .lock()
            .expect("mock state mutex")
            .clone();
        let last = seen_headers.last().cloned().flatten();
        assert_eq!(last.as_deref(), expected_header);
    }
}
