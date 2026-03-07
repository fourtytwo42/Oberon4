use axum::{extract::Form, routing::post, Router};
use llm_auth::{AuthError, OpenAiChatGptOAuthClient, OpenAiOAuthConfig};
use std::net::SocketAddr;
use tokio::net::TcpListener;

#[derive(Debug, serde::Deserialize)]
struct TokenForm {
    grant_type: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

async fn token_handler(Form(form): Form<TokenForm>) -> axum::Json<serde_json::Value> {
    let body = if form.grant_type == "authorization_code" {
        serde_json::json!({
            "access_token": "x.eyJlbWFpbCI6InVzZXJAZXhhbXBsZS5jb20iLCJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjdF8xMjMifX0.y",
            "refresh_token": "refresh_1",
            "expires_in": 3600
        })
    } else if form.refresh_token.as_deref() == Some("refresh_no_rotate") {
        serde_json::json!({
            "access_token": "x.eyJlbWFpbCI6InVzZXIyQGV4YW1wbGUuY29tIiwiaHR0cHM6Ly9hcGkub3BlbmFpLmNvbS9hdXRoIjp7ImNoYXRncHRfYWNjb3VudF9pZCI6ImFjY3RfNDU2In19.y",
            "expires_in": 3600
        })
    } else {
        serde_json::json!({
            "access_token": "x.eyJlbWFpbCI6InVzZXIyQGV4YW1wbGUuY29tIiwiaHR0cHM6Ly9hcGkub3BlbmFpLmNvbS9hdXRoIjp7ImNoYXRncHRfYWNjb3VudF9pZCI6ImFjY3RfNDU2In19.y",
            "refresh_token": "refresh_2",
            "expires_in": 3600
        })
    };
    axum::Json(body)
}

async fn spawn_mock_server() -> SocketAddr {
    let app = Router::new().route("/oauth/token", post(token_handler));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    addr
}

#[tokio::test]
async fn exchange_and_replay_protection() {
    let addr = spawn_mock_server().await;
    let config = OpenAiOAuthConfig {
        token_url: format!("http://{addr}/oauth/token"),
        authorize_url: "https://example.com/oauth".to_string(),
        ..Default::default()
    };

    let client = OpenAiChatGptOAuthClient::new(config);

    let flow = client.start_flow(None).expect("start");
    let token = client
        .exchange_code(&flow.state, "code_1", None)
        .await
        .expect("exchange");
    assert_eq!(token.refresh_token, "refresh_1");
    assert_eq!(token.account_id.as_deref(), Some("acct_123"));

    let err = client
        .exchange_code(&flow.state, "code_1", None)
        .await
        .expect_err("replay should fail");
    assert!(matches!(err, AuthError::InvalidState));
}

#[tokio::test]
async fn refresh_works() {
    let addr = spawn_mock_server().await;
    let config = OpenAiOAuthConfig {
        token_url: format!("http://{addr}/oauth/token"),
        authorize_url: "https://example.com/oauth".to_string(),
        ..Default::default()
    };

    let client = OpenAiChatGptOAuthClient::new(config);
    let token = client.refresh("refresh_1").await.expect("refresh");
    assert_eq!(token.refresh_token, "refresh_2");
    assert_eq!(token.account_id.as_deref(), Some("acct_456"));
}

#[tokio::test]
async fn refresh_uses_prior_refresh_token_when_omitted() {
    let addr = spawn_mock_server().await;
    let config = OpenAiOAuthConfig {
        token_url: format!("http://{addr}/oauth/token"),
        authorize_url: "https://example.com/oauth".to_string(),
        ..Default::default()
    };

    let client = OpenAiChatGptOAuthClient::new(config);
    let token = client.refresh("refresh_no_rotate").await.expect("refresh");
    assert_eq!(token.refresh_token, "refresh_no_rotate");
    assert_eq!(token.account_id.as_deref(), Some("acct_456"));
}
