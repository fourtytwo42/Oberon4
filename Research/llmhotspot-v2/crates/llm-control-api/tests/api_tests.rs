#[path = "test_env_lock.rs"]
mod test_env_lock;

use axum::{body, body::Body, http::Request, Router};
use llm_control_api::{app_router, AppState};
use llm_storage::{ApiKeyRecord, SqliteStore};
use tower::util::ServiceExt;

async fn create_api_key_bearer(app: Router) -> String {
    std::env::set_var("LLMHOTSPOT_BOOTSTRAP_TOKEN", "bootstrap-required");
    let create_key_req = Request::builder()
        .uri("/v1/keys/create")
        .method("POST")
        .header("content-type", "application/json")
        .header("authorization", "Bearer bootstrap-required")
        .body(Body::from(r#"{"label":"tests","expires_unix":null}"#))
        .expect("create request");
    let create_resp = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        app.clone().oneshot(create_key_req),
    )
    .await
    .expect("timed out waiting for response")
    .expect("create response");
    assert_eq!(create_resp.status(), 200);
    let create_body = body::to_bytes(create_resp.into_body(), usize::MAX)
        .await
        .expect("create body");
    let create_json: serde_json::Value = serde_json::from_slice(&create_body).expect("create json");
    let raw_key = create_json
        .get("raw_key")
        .and_then(|v| v.as_str())
        .expect("raw key");
    format!("Bearer {raw_key}")
}

#[tokio::test]
async fn models_route_works() {
    let _guard = test_env_lock::lock_env();
    let app = app_router(AppState::default());
    let req = Request::builder()
        .uri("/v1/models")
        .body(Body::empty())
        .expect("request");

    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
#[ignore = "TODO: investigate windows hang in store-backed chat auth path"]
async fn chat_requires_bearer() {
    let _guard = test_env_lock::lock_env();
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    store
        .create_api_key(&ApiKeyRecord {
            key_id: uuid::Uuid::new_v4().to_string(),
            label: "seed".to_string(),
            key_hash: "seed-hash".to_string(),
            tool_choice_policy: "auto".to_string(),
            created_unix: 0,
            expires_unix: None,
            revoked_unix: None,
            last_used_unix: None,
        })
        .await
        .expect("seed key");
    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:11434".to_string(),
    ));
    let req = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"model":"gpt-5.2","messages":[{"role":"user","content":"hello"}],"input":[{"role":"user","content":[{"type":"input_text","text":"hello"}]}],"stream":false}"#,
        ))
        .expect("request");

    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
#[ignore = "TODO: investigate windows hang in store-backed responses auth path"]
async fn responses_route_requires_bearer() {
    let _guard = test_env_lock::lock_env();
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    store
        .create_api_key(&ApiKeyRecord {
            key_id: uuid::Uuid::new_v4().to_string(),
            label: "seed".to_string(),
            key_hash: "seed-hash".to_string(),
            tool_choice_policy: "auto".to_string(),
            created_unix: 0,
            expires_unix: None,
            revoked_unix: None,
            last_used_unix: None,
        })
        .await
        .expect("seed key");
    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:11434".to_string(),
    ));
    let req = Request::builder()
        .uri("/v1/responses")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"model":"gpt-5.2","messages":[{"role":"user","content":"hello"}],"input":[{"role":"user","content":[{"type":"input_text","text":"hello"}]}],"stream":false}"#,
        ))
        .expect("request");

    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn anthropic_messages_requires_api_key() {
    let _guard = test_env_lock::lock_env();
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:11434".to_string(),
    ));
    let req = Request::builder()
        .uri("/v1/messages")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"model":"claude-4.6-sonnet-medium","max_tokens":64,"messages":[{"role":"user","content":"hello"}]}"#,
        ))
        .expect("request");

    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn anthropic_messages_accepts_x_api_key_auth() {
    let _guard = test_env_lock::lock_env();
    let app = app_router(AppState::default());
    let req = Request::builder()
        .uri("/v1/messages")
        .method("POST")
        .header("content-type", "application/json")
        .header("x-api-key", "key")
        .body(Body::from(
            // Intentionally invalid payload to verify auth passed and request reached validation.
            r#"{"max_tokens":64,"messages":[{"role":"user","content":"hello"}]}"#,
        ))
        .expect("request");

    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn chat_rejects_ambiguous_messages_and_input_payload() {
    let _guard = test_env_lock::lock_env();
    let app = app_router(AppState::default());
    let req = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("content-type", "application/json")
        .header("authorization", "Bearer key")
        .body(Body::from(
            r#"{"model":"gpt-5.2","messages":[{"role":"user","content":"hello"}],"input":[{"role":"user","content":[{"type":"input_text","text":"hello"}]}],"stream":false}"#,
        ))
        .expect("request");

    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn chat_allows_prefixed_model_passthrough_in_no_store_mode() {
    let _guard = test_env_lock::lock_env();
    let app = app_router(AppState::default());
    let req = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("content-type", "application/json")
        .header("authorization", "Bearer key")
        .body(Body::from(
            r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}],"stream":false}"#,
        ))
        .expect("request");

    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn chat_rejects_openai_model_when_no_openai_oauth_enabled() {
    let _guard = test_env_lock::lock_env();
    std::env::set_var("LLMHOTSPOT_BOOTSTRAP_TOKEN", "bootstrap-required");
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:11434".to_string(),
    ));

    let create_key_req = Request::builder()
        .uri("/v1/keys/create")
        .method("POST")
        .header("content-type", "application/json")
        .header("authorization", "Bearer bootstrap-required")
        .body(Body::from(r#"{"label":"cli","expires_unix":null}"#))
        .expect("create request");
    let create_resp = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        app.clone().oneshot(create_key_req),
    )
    .await
    .expect("timed out waiting for response")
    .expect("create response");
    assert_eq!(create_resp.status(), 200);
    let create_body = body::to_bytes(create_resp.into_body(), usize::MAX)
        .await
        .expect("create body");
    let create_json: serde_json::Value = serde_json::from_slice(&create_body).expect("create json");
    let raw_key = create_json
        .get("raw_key")
        .and_then(|v| v.as_str())
        .expect("raw key");
    let auth_header = format!("Bearer {raw_key}");

    let req = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("content-type", "application/json")
        .header("authorization", &auth_header)
        .body(Body::from(
            r#"{"model":"gpt-5.2","messages":[{"role":"user","content":"hello"}],"stream":false}"#,
        ))
        .expect("request");

    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn anthropic_messages_rejects_claude_model_when_no_anthropic_oauth_enabled() {
    let _guard = test_env_lock::lock_env();
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:11434".to_string(),
    ));

    let auth_header = create_api_key_bearer(app.clone()).await;
    let req = Request::builder()
        .uri("/v1/messages")
        .method("POST")
        .header("content-type", "application/json")
        .header("authorization", auth_header)
        .body(Body::from(
            r#"{"model":"claude-4.6-sonnet-medium","max_tokens":64,"messages":[{"role":"user","content":"hello"}]}"#,
        ))
        .expect("request");

    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn chat_accepts_custom_tools_shape_in_no_store_mode() {
    let _guard = test_env_lock::lock_env();
    let app = app_router(AppState::default());
    let req = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("content-type", "application/json")
        .header("authorization", "Bearer key")
        .body(Body::from(
            r#"{"model":"gpt-5.2","messages":[{"role":"user","content":"hello"}],"tools":[{"type":"custom","name":"ApplyPatch","description":"patch tool","format":{"type":"grammar","grammar":"patch"}}],"stream":false}"#,
        ))
        .expect("request");

    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn chat_accepts_flat_tools_in_no_store_mode() {
    let _guard = test_env_lock::lock_env();
    let app = app_router(AppState::default());
    let req = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("content-type", "application/json")
        .header("authorization", "Bearer key")
        .body(Body::from(
            r#"{"model":"gpt-5.2","messages":[{"role":"user","content":"hello"}],"tools":[{"type":"function","name":"exec_command","parameters":{"type":"object","properties":{}}}],"stream":false}"#,
        ))
        .expect("request");

    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn request_id_is_propagated() {
    let _guard = test_env_lock::lock_env();
    let app = app_router(AppState::default());
    let req = Request::builder()
        .uri("/v1/models")
        .header("x-request-id", "abc-123")
        .body(Body::empty())
        .expect("request");

    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    let req_id = resp
        .headers()
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    assert_eq!(req_id, "abc-123");
}

#[tokio::test]
async fn auth_status_route_exists() {
    let _guard = test_env_lock::lock_env();
    let app = app_router(AppState::default());
    let req = Request::builder()
        .uri("/v1/auth/status")
        .body(Body::empty())
        .expect("request");
    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn ui_and_management_routes_work_with_store() {
    let _guard = test_env_lock::lock_env();
    std::env::set_var("LLMHOTSPOT_BOOTSTRAP_TOKEN", "bootstrap-required");
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let app = app_router(AppState::with_store(
        store.clone(),
        "http://127.0.0.1:11434".to_string(),
    ));

    let req = Request::builder()
        .uri("/console")
        .body(Body::empty())
        .expect("request");
    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.clone().oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 200);

    let create_key_req = Request::builder()
        .uri("/v1/keys/create")
        .method("POST")
        .header("content-type", "application/json")
        .header("authorization", "Bearer bootstrap-required")
        .body(Body::from(r#"{"label":"cli","expires_unix":null}"#))
        .expect("create request");
    let create_resp = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        app.clone().oneshot(create_key_req),
    )
    .await
    .expect("timed out waiting for response")
    .expect("create response");
    assert_eq!(create_resp.status(), 200);
    let create_body = body::to_bytes(create_resp.into_body(), usize::MAX)
        .await
        .expect("create body");
    let create_json: serde_json::Value = serde_json::from_slice(&create_body).expect("create json");
    let raw_key = create_json
        .get("raw_key")
        .and_then(|v| v.as_str())
        .expect("raw key");
    let auth_header = format!("Bearer {raw_key}");

    let keys_req = Request::builder()
        .uri("/v1/keys")
        .body(Body::empty())
        .expect("keys request");
    let keys_resp = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        app.clone().oneshot(keys_req),
    )
    .await
    .expect("timed out waiting for response")
    .expect("keys response");
    assert_eq!(keys_resp.status(), 200);

    let oauth_start_req = Request::builder()
        .uri("/v1/oauth/start")
        .method("POST")
        .header("content-type", "application/json")
        .header("authorization", &auth_header)
        .body(Body::from(
            r#"{"redirect_uri":"http://localhost:1455/auth/callback"}"#,
        ))
        .expect("oauth start request");
    let oauth_start_resp = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        app.clone().oneshot(oauth_start_req),
    )
    .await
    .expect("timed out waiting for response")
    .expect("oauth start response");
    assert_eq!(oauth_start_resp.status(), 200);

    let oauth_complete_req = Request::builder()
        .uri("/v1/oauth/callback/complete")
        .method("POST")
        .header("content-type", "application/json")
        .header("authorization", &auth_header)
        .body(Body::from(r#"{"state":"missing","code":"x"}"#))
        .expect("oauth complete request");
    let oauth_complete_resp = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        app.oneshot(oauth_complete_req),
    )
    .await
    .expect("timed out waiting for response")
    .expect("oauth complete response");
    assert_eq!(oauth_complete_resp.status(), 400);
}

#[tokio::test]
async fn bootstrap_token_required_before_first_key() {
    let _guard = test_env_lock::lock_env();
    std::env::set_var("LLMHOTSPOT_BOOTSTRAP_TOKEN", "bootstrap-required");
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:11434".to_string(),
    ));

    let no_auth_req = Request::builder()
        .uri("/v1/keys/create")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"label":"a","expires_unix":null}"#))
        .expect("no auth req");
    let no_auth_resp = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        app.clone().oneshot(no_auth_req),
    )
    .await
    .expect("timed out waiting for response")
    .expect("no auth resp");
    assert_eq!(no_auth_resp.status(), 401);

    let wrong_auth_req = Request::builder()
        .uri("/v1/keys/create")
        .method("POST")
        .header("content-type", "application/json")
        .header("authorization", "Bearer wrong")
        .body(Body::from(r#"{"label":"a","expires_unix":null}"#))
        .expect("wrong auth req");
    let wrong_auth_resp = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        app.clone().oneshot(wrong_auth_req),
    )
    .await
    .expect("timed out waiting for response")
    .expect("wrong auth resp");
    assert_eq!(wrong_auth_resp.status(), 401);

    let correct_auth_req = Request::builder()
        .uri("/v1/keys/create")
        .method("POST")
        .header("content-type", "application/json")
        .header("authorization", "Bearer bootstrap-required")
        .body(Body::from(r#"{"label":"a","expires_unix":null}"#))
        .expect("ok auth req");
    let correct_auth_resp = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        app.oneshot(correct_auth_req),
    )
    .await
    .expect("timed out waiting for response")
    .expect("ok auth resp");
    assert_eq!(correct_auth_resp.status(), 200);
}

#[tokio::test]
async fn tray_status_route_exists() {
    let _guard = test_env_lock::lock_env();
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:11434".to_string(),
    ));
    let req = Request::builder()
        .uri("/v1/tray/status")
        .body(Body::empty())
        .expect("request");
    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(req))
        .await
        .expect("timed out waiting for response")
        .expect("response");
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn oauth_routing_profile_roundtrip() {
    let _guard = test_env_lock::lock_env();
    std::env::set_var("LLMHOTSPOT_BOOTSTRAP_TOKEN", "bootstrap-required");
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:11434".to_string(),
    ));

    let create_key_req = Request::builder()
        .uri("/v1/keys/create")
        .method("POST")
        .header("content-type", "application/json")
        .header("authorization", "Bearer bootstrap-required")
        .body(Body::from(r#"{"label":"cli","expires_unix":null}"#))
        .expect("create request");
    let create_resp = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        app.clone().oneshot(create_key_req),
    )
    .await
    .expect("timed out waiting for response")
    .expect("create response");
    let create_body = body::to_bytes(create_resp.into_body(), usize::MAX)
        .await
        .expect("create body");
    let create_json: serde_json::Value = serde_json::from_slice(&create_body).expect("create json");
    let raw_key = create_json
        .get("raw_key")
        .and_then(|v| v.as_str())
        .expect("raw key");
    let auth_header = format!("Bearer {raw_key}");

    let connect_req = Request::builder()
        .uri("/v1/oauth/accounts/connect")
        .method("POST")
        .header("content-type", "application/json")
        .header("authorization", &auth_header)
        .body(Body::from(
            r#"{"account_id":"acct_1","display_name":"A1","email":"a1@example.com"}"#,
        ))
        .expect("connect request");
    let connect_resp = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        app.clone().oneshot(connect_req),
    )
    .await
    .expect("timed out waiting for response")
    .expect("connect resp");
    assert_eq!(connect_resp.status(), 200);

    let set_req = Request::builder()
        .uri("/v1/oauth/accounts/acct_1/routing")
        .method("POST")
        .header("content-type", "application/json")
        .header("authorization", &auth_header)
        .body(Body::from(
            r#"{
              "upstream_base_url":"https://example-proxy.local",
              "chat_path":"/v1/chat/completions",
              "responses_path":"/v1/responses",
              "org_id":"org_1",
              "project_id":"proj_1",
              "extra_headers":{"x-foo":"bar","authorization":"ignore-me"}
            }"#,
        ))
        .expect("set req");
    let set_resp = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        app.clone().oneshot(set_req),
    )
    .await
    .expect("timed out waiting for response")
    .expect("set resp");
    assert_eq!(set_resp.status(), 200);

    let get_req = Request::builder()
        .uri("/v1/oauth/accounts/acct_1/routing")
        .body(Body::empty())
        .expect("get req");
    let get_resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(get_req))
        .await
        .expect("timed out waiting for response")
        .expect("get resp");
    assert_eq!(get_resp.status(), 200);
}

#[tokio::test]
async fn oauth_browser_start_status_endpoints_exist() {
    let _guard = test_env_lock::lock_env();
    std::env::set_var("LLMHOTSPOT_BOOTSTRAP_TOKEN", "bootstrap-required");
    std::env::set_var(
        "LLMHOTSPOT_OAUTH_REDIRECT_URI",
        "http://localhost:1455/auth/callback",
    );
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:11434".to_string(),
    ));

    let create_key_req = Request::builder()
        .uri("/v1/keys/create")
        .method("POST")
        .header("content-type", "application/json")
        .header("authorization", "Bearer bootstrap-required")
        .body(Body::from(r#"{"label":"cli","expires_unix":null}"#))
        .expect("create request");
    let create_resp = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        app.clone().oneshot(create_key_req),
    )
    .await
    .expect("timed out waiting for response")
    .expect("create response");
    let create_body = body::to_bytes(create_resp.into_body(), usize::MAX)
        .await
        .expect("create body");
    let create_json: serde_json::Value = serde_json::from_slice(&create_body).expect("create json");
    let raw_key = create_json
        .get("raw_key")
        .and_then(|v| v.as_str())
        .expect("raw key");
    let auth_header = format!("Bearer {raw_key}");

    let start_req = Request::builder()
        .uri("/v1/oauth/connect/browser/start")
        .method("POST")
        .header("content-type", "application/json")
        .header("authorization", &auth_header)
        .body(Body::from(r#"{}"#))
        .expect("start req");
    let start_resp = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        app.clone().oneshot(start_req),
    )
    .await
    .expect("timed out waiting for response")
    .expect("start resp");
    assert_eq!(start_resp.status(), 200);

    let start_body = body::to_bytes(start_resp.into_body(), usize::MAX)
        .await
        .expect("start body");
    let start_json: serde_json::Value = serde_json::from_slice(&start_body).expect("start json");
    let flow_id = start_json
        .get("flow_id")
        .and_then(|v| v.as_str())
        .expect("flow id");

    let status_req = Request::builder()
        .uri(format!("/v1/oauth/connect/browser/{flow_id}"))
        .body(Body::empty())
        .expect("status req");
    let status_resp =
        tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(status_req))
            .await
            .expect("timed out waiting for response")
            .expect("status resp");
    assert_eq!(status_resp.status(), 200);
}

#[tokio::test]
async fn console_session_exchange_allows_cookie_admin_auth() {
    let _guard = test_env_lock::lock_env();
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    store
        .set_app_state("console_admin_bootstrap_key", "adm_test_key")
        .await
        .expect("set key");
    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:11434".to_string(),
    ));

    let exchange_req = Request::builder()
        .uri("/v1/console/session/exchange")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"admin_key":"adm_test_key"}"#))
        .expect("exchange req");
    let exchange_resp = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        app.clone().oneshot(exchange_req),
    )
    .await
    .expect("timed out waiting for response")
    .expect("exchange resp");
    assert_eq!(exchange_resp.status(), 200);
    let set_cookie = exchange_resp
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .expect("set-cookie")
        .to_string();
    let cookie_pair = set_cookie
        .split(';')
        .next()
        .expect("cookie pair")
        .to_string();

    let keys_req = Request::builder()
        .uri("/v1/keys")
        .header("cookie", cookie_pair)
        .body(Body::empty())
        .expect("keys req");
    let keys_resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.oneshot(keys_req))
        .await
        .expect("timed out waiting for response")
        .expect("keys resp");
    assert_eq!(keys_resp.status(), 200);
}

#[tokio::test]
async fn console_session_exchange_accepts_previous_bootstrap_key() {
    let _guard = test_env_lock::lock_env();
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    store
        .set_app_state("console_admin_bootstrap_key", "adm_current_key")
        .await
        .expect("set current key");
    store
        .set_app_state("console_admin_previous_bootstrap_key", "adm_previous_key")
        .await
        .expect("set previous key");
    let app = app_router(AppState::with_store(
        store,
        "http://127.0.0.1:11434".to_string(),
    ));

    let exchange_req = Request::builder()
        .uri("/v1/console/session/exchange")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"admin_key":"adm_previous_key"}"#))
        .expect("exchange req");
    let exchange_resp = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        app.oneshot(exchange_req),
    )
    .await
    .expect("timed out waiting for response")
    .expect("exchange resp");
    assert_eq!(exchange_resp.status(), 200);
}
