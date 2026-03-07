use super::*;
fn default_custom_api_accounts_envelope() -> CustomApiAccountsEnvelope {
    CustomApiAccountsEnvelope {
        version: 1,
        accounts: Vec::new(),
    }
}

pub(crate) async fn list_custom_api_accounts(
    store: &SqliteStore,
) -> Result<Vec<OAuthAccountRecord>, String> {
    let raw = store
        .get_app_state(CUSTOM_API_ACCOUNTS_KEY)
        .await
        .map_err(|e| e.to_string())?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let env = serde_json::from_str::<CustomApiAccountsEnvelope>(&raw)
        .unwrap_or_else(|_| default_custom_api_accounts_envelope());
    Ok(env.accounts)
}

pub(crate) async fn save_custom_api_accounts(
    store: &SqliteStore,
    accounts: &[OAuthAccountRecord],
) -> Result<(), String> {
    let mut out = accounts.to_vec();
    out.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then_with(|| a.account_id.cmp(&b.account_id))
    });
    let env = CustomApiAccountsEnvelope {
        version: 1,
        accounts: out,
    };
    let encoded = serde_json::to_string(&env).map_err(|e| e.to_string())?;
    store
        .set_app_state(CUSTOM_API_ACCOUNTS_KEY, &encoded)
        .await
        .map_err(|e| e.to_string())
}

pub(crate) fn custom_api_key_store_key(account_id: &str) -> String {
    format!("custom_api_key:{account_id}")
}

pub(crate) async fn get_custom_api_key(
    store: &SqliteStore,
    account_id: &str,
) -> Result<String, String> {
    store
        .get_app_state(&custom_api_key_store_key(account_id))
        .await
        .map_err(|e| e.to_string())
}

pub(crate) async fn set_custom_api_key(
    store: &SqliteStore,
    account_id: &str,
    api_key: &str,
) -> Result<(), String> {
    store
        .set_app_state(&custom_api_key_store_key(account_id), api_key)
        .await
        .map_err(|e| e.to_string())
}

pub(crate) async fn delete_custom_api_key(
    store: &SqliteStore,
    account_id: &str,
) -> Result<(), String> {
    store
        .delete_app_state(&custom_api_key_store_key(account_id))
        .await
        .map_err(|e| e.to_string())
}

pub(crate) async fn list_all_accounts(
    store: &SqliteStore,
) -> Result<Vec<OAuthAccountRecord>, String> {
    let mut out = store
        .list_oauth_accounts()
        .await
        .map_err(|e| e.to_string())?;
    out.extend(list_custom_api_accounts(store).await?);
    out.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then_with(|| a.account_id.cmp(&b.account_id))
    });
    Ok(out)
}

pub(crate) fn canonical_base_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let parsed = Url::parse(trimmed).ok()?;
    match parsed.scheme() {
        "http" | "https" => Some(trimmed.to_string()),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct CustomApiAccountsEnvelope {
    version: u64,
    accounts: Vec<OAuthAccountRecord>,
}
