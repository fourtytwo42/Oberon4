use crate::GatewayError;
use llm_auth::OAuthAccountManager;
use llm_foundation::contracts::{ChatRequest, ChatResponse};

pub trait AccountGatewayClient: Send + Sync {
    fn chat_for_account(
        &self,
        account_id: &str,
        req: &ChatRequest,
    ) -> Result<ChatResponse, GatewayError>;
}

pub async fn chat_with_failover<C: AccountGatewayClient>(
    manager: &OAuthAccountManager,
    client: &C,
    req: &ChatRequest,
) -> Result<ChatResponse, GatewayError> {
    let chain = manager
        .select_failover_chain()
        .await
        .map_err(|_| GatewayError::Auth)?;

    if chain.is_empty() {
        return Err(GatewayError::TenantOffline);
    }

    let mut last_error = GatewayError::TenantOffline;
    for account in chain {
        match client.chat_for_account(&account.account_id, req) {
            Ok(resp) => {
                let _ = manager.mark_success(&account.account_id).await;
                return Ok(resp);
            }
            Err(err) => {
                let _ = manager
                    .mark_failure(&account.account_id, &format!("{err}"))
                    .await;
                last_error = err;
            }
        }
    }

    Err(last_error)
}
