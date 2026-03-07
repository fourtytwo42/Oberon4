use crate::AuthError;
use crate::{OpenAiChatGptOAuthClient, WhamUsage};
use llm_storage::{OAuthAccountRecord, OAuthUsageRecord, SqliteStore, StoredTokenRecord};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedAccount {
    pub account: OAuthAccountRecord,
    pub usage: Vec<OAuthUsageRecord>,
}

#[derive(Debug, Clone)]
pub struct OAuthAccountManager {
    store: SqliteStore,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountAccessSession {
    pub account_id: String,
    pub access_token: String,
    pub expires_at_unix: u64,
}

impl OAuthAccountManager {
    pub fn new(store: SqliteStore) -> Self {
        Self { store }
    }

    pub async fn connect_account(
        &self,
        account_id: &str,
        display_name: &str,
        email: &str,
    ) -> Result<(), AuthError> {
        let accounts = self
            .store
            .list_oauth_accounts()
            .await
            .map_err(storage_err)?;
        let default_priority = accounts.len() as i64;
        let row = OAuthAccountRecord {
            account_id: account_id.to_string(),
            provider_id: "openai".to_string(),
            display_name: display_name.to_string(),
            email: email.to_string(),
            priority: default_priority,
            enabled: true,
            last_used_unix: None,
            last_error: None,
            upstream_base_url: None,
            chat_path: None,
            responses_path: None,
            org_id: None,
            project_id: None,
            extra_headers_json: None,
        };
        self.store
            .upsert_oauth_account(&row)
            .await
            .map_err(storage_err)
    }

    pub async fn list_accounts(&self) -> Result<Vec<ManagedAccount>, AuthError> {
        let accounts = self
            .store
            .list_oauth_accounts()
            .await
            .map_err(storage_err)?;
        let mut out = Vec::with_capacity(accounts.len());
        for account in accounts {
            let usage = self
                .store
                .list_oauth_usage(&account.account_id)
                .await
                .map_err(storage_err)?;
            out.push(ManagedAccount { account, usage });
        }
        Ok(out)
    }

    pub async fn enable(&self, account_id: &str) -> Result<(), AuthError> {
        self.store
            .update_oauth_enabled(account_id, true)
            .await
            .map_err(storage_err)
    }

    pub async fn disable(&self, account_id: &str) -> Result<(), AuthError> {
        self.store
            .update_oauth_enabled(account_id, false)
            .await
            .map_err(storage_err)
    }

    pub async fn disconnect(&self, account_id: &str) -> Result<(), AuthError> {
        self.store
            .delete_oauth_account(account_id)
            .await
            .map_err(storage_err)
    }

    pub async fn set_priority(&self, account_id: &str, index: usize) -> Result<(), AuthError> {
        let mut accounts = self
            .store
            .list_oauth_accounts()
            .await
            .map_err(storage_err)?;

        let Some(pos) = accounts.iter().position(|a| a.account_id == account_id) else {
            return Err(AuthError::Storage(format!(
                "unknown account_id={account_id}"
            )));
        };

        let item = accounts.remove(pos);
        let idx = index.min(accounts.len());
        accounts.insert(idx, item);

        for (priority, account) in accounts.into_iter().enumerate() {
            self.store
                .set_oauth_priority(&account.account_id, priority as i64)
                .await
                .map_err(storage_err)?;
        }

        Ok(())
    }

    pub async fn activate(&self, account_id: &str) -> Result<(), AuthError> {
        self.set_priority(account_id, 0).await
    }

    pub async fn select_failover_chain(&self) -> Result<Vec<OAuthAccountRecord>, AuthError> {
        let accounts = self
            .store
            .list_oauth_accounts()
            .await
            .map_err(storage_err)?;
        Ok(accounts.into_iter().filter(|a| a.enabled).collect())
    }

    pub async fn resolve_access_sessions(
        &self,
        workspace_id: &str,
        oauth_client: &OpenAiChatGptOAuthClient,
    ) -> Result<Vec<AccountAccessSession>, AuthError> {
        let chain = self.select_failover_chain().await?;
        let mut sessions = Vec::new();
        for account in chain {
            let record = self
                .store
                .load_token(workspace_id, &account.account_id)
                .await
                .map_err(storage_err)?;
            let Some(record) = record else {
                continue;
            };
            match oauth_client.refresh(&record.refresh_token).await {
                Ok(bundle) => {
                    // Persist the rotated refresh token so the next refresh succeeds.
                    let mut new_rec = StoredTokenRecord::new(
                        workspace_id,
                        &account.account_id,
                        &bundle.refresh_token,
                    );
                    new_rec.account_id = bundle.account_id.clone();
                    let _ = self.store.save_token(&new_rec).await;
                    sessions.push(AccountAccessSession {
                        account_id: account.account_id,
                        access_token: bundle.access_token,
                        expires_at_unix: bundle.expires_at_unix,
                    });
                }
                Err(err) => {
                    let _ = self
                        .mark_failure(&account.account_id, &format!("refresh_failed:{err}"))
                        .await;
                }
            }
        }
        Ok(sessions)
    }

    pub async fn fetch_wham_usage_for_account(
        &self,
        workspace_id: &str,
        account_id: &str,
        oauth_client: &OpenAiChatGptOAuthClient,
    ) -> Result<WhamUsage, AuthError> {
        let record = self
            .store
            .load_token(workspace_id, account_id)
            .await
            .map_err(storage_err)?;
        let Some(record) = record else {
            return Err(AuthError::Storage(format!(
                "no token stored for account {account_id}"
            )));
        };
        let token = oauth_client.refresh(&record.refresh_token).await?;
        // Persist the rotated refresh token so the next refresh succeeds.
        let mut new_rec = StoredTokenRecord::new(workspace_id, account_id, &token.refresh_token);
        new_rec.account_id = token.account_id.clone();
        let _ = self.store.save_token(&new_rec).await;
        oauth_client
            .fetch_wham_usage(&token.access_token, token.account_id.as_deref())
            .await
    }

    pub async fn mark_success(&self, account_id: &str) -> Result<(), AuthError> {
        let mut accounts = self
            .store
            .list_oauth_accounts()
            .await
            .map_err(storage_err)?;

        let now = now_unix();
        if let Some(existing) = accounts.iter_mut().find(|a| a.account_id == account_id) {
            existing.last_used_unix = Some(now);
            existing.last_error = None;
            self.store
                .upsert_oauth_account(existing)
                .await
                .map_err(storage_err)?;
        }

        self.bump_usage(account_id, "5h", 1, 1_000, now as i64 + 5 * 60 * 60)
            .await?;
        self.bump_usage(
            account_id,
            "weekly",
            1,
            10_000,
            now as i64 + 7 * 24 * 60 * 60,
        )
        .await
    }

    pub async fn mark_failure(&self, account_id: &str, error: &str) -> Result<(), AuthError> {
        let mut accounts = self
            .store
            .list_oauth_accounts()
            .await
            .map_err(storage_err)?;
        if let Some(existing) = accounts.iter_mut().find(|a| a.account_id == account_id) {
            existing.last_error = Some(error.to_string());
            self.store
                .upsert_oauth_account(existing)
                .await
                .map_err(storage_err)?;
        }
        Ok(())
    }

    async fn bump_usage(
        &self,
        account_id: &str,
        window_type: &str,
        delta: i64,
        default_limit: i64,
        reset_unix: i64,
    ) -> Result<(), AuthError> {
        let mut current = self
            .store
            .list_oauth_usage(account_id)
            .await
            .map_err(storage_err)?
            .into_iter()
            .find(|u| u.window_type == window_type)
            .unwrap_or(OAuthUsageRecord {
                account_id: account_id.to_string(),
                window_type: window_type.to_string(),
                used: 0,
                limit: default_limit,
                reset_unix,
            });
        current.used += delta;
        current.reset_unix = reset_unix;
        self.store
            .upsert_oauth_usage(&current)
            .await
            .map_err(storage_err)
    }
}

fn storage_err(err: llm_storage::StorageError) -> AuthError {
    AuthError::Storage(err.to_string())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
