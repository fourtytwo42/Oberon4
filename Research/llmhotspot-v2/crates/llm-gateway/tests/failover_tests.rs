use llm_auth::OAuthAccountManager;
use llm_foundation::contracts::{ChatRequest, ChatResponse};
use llm_gateway::{chat_with_failover, AccountGatewayClient, GatewayError};
use llm_storage::SqliteStore;

struct MockClient;

impl AccountGatewayClient for MockClient {
    fn chat_for_account(
        &self,
        account_id: &str,
        _req: &ChatRequest,
    ) -> Result<ChatResponse, GatewayError> {
        if account_id == "acct_1" {
            return Err(GatewayError::RateLimit);
        }
        Ok(ChatResponse {
            id: "ok".to_string(),
            output_text: format!("from:{account_id}"),
        })
    }
}

#[tokio::test]
async fn failover_uses_next_account() {
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let mgr = OAuthAccountManager::new(store);
    mgr.connect_account("acct_1", "A1", "a1@example.com")
        .await
        .expect("a1");
    mgr.connect_account("acct_2", "A2", "a2@example.com")
        .await
        .expect("a2");

    let response = chat_with_failover(
        &mgr,
        &MockClient,
        &ChatRequest {
            model: "gpt-5.2".to_string(),
            input: "hello".to_string(),
            stream: false,
        },
    )
    .await
    .expect("success with failover");

    assert_eq!(response.output_text, "from:acct_2");
}
