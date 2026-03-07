use llm_storage::SqliteStore;
use std::time::Duration;

const API_KEY_SECRET_SERVICE: &str = "llmhotspot";
const SECRET_STORE_FALLBACK_ENV: &str = "LLMHOTSPOT_SECRET_STORE_APP_STATE_FALLBACK";
const KEYRING_IO_TIMEOUT: Duration = Duration::from_secs(2);

fn secret_account(workspace_id: &str, key_id: &str) -> String {
    format!("api_key_raw:{workspace_id}:{key_id}")
}

fn is_missing_secret_error(msg: &str) -> bool {
    msg.contains("No entry")
        || msg.contains("NoPassword")
        || msg.contains("Item not found")
        || msg.contains("The system cannot find the file specified")
}

fn truthy_env(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn use_app_state_fallback() -> bool {
    if cfg!(test) {
        return true;
    }
    std::env::var(SECRET_STORE_FALLBACK_ENV)
        .ok()
        .map(|v| truthy_env(&v))
        .unwrap_or(false)
}

pub(crate) fn set_api_key_raw(workspace_id: &str, key_id: &str, raw: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(
        API_KEY_SECRET_SERVICE,
        &secret_account(workspace_id, key_id),
    )
    .map_err(|e| e.to_string())?;
    entry.set_password(raw).map_err(|e| e.to_string())
}

pub(crate) async fn set_api_key_raw_with_fallback(
    store: &SqliteStore,
    workspace_id: &str,
    key_id: &str,
    raw: &str,
) -> Result<(), String> {
    // Always persist an app_state backup so key copy remains available even if
    // keyring backends are inconsistent across environments.
    store
        .set_app_state(&format!("key_raw:{key_id}"), raw)
        .await
        .map_err(|e| e.to_string())?;

    if use_app_state_fallback() {
        return Ok(());
    }
    let workspace_id = workspace_id.to_string();
    let key_id_owned = key_id.to_string();
    let raw_owned = raw.to_string();
    let set_res = tokio::time::timeout(
        KEYRING_IO_TIMEOUT,
        tokio::task::spawn_blocking(move || {
            set_api_key_raw(&workspace_id, &key_id_owned, &raw_owned)
        }),
    )
    .await;
    let _ = matches!(set_res, Ok(Ok(Ok(()))));
    Ok(())
}

pub(crate) fn get_api_key_raw(workspace_id: &str, key_id: &str) -> Result<Option<String>, String> {
    let entry = keyring::Entry::new(
        API_KEY_SECRET_SERVICE,
        &secret_account(workspace_id, key_id),
    )
    .map_err(|e| e.to_string())?;
    match entry.get_password() {
        Ok(raw) => Ok(Some(raw)),
        Err(e) => {
            let msg = e.to_string();
            if is_missing_secret_error(&msg) {
                Ok(None)
            } else {
                Err(msg)
            }
        }
    }
}

pub(crate) fn delete_api_key_raw(workspace_id: &str, key_id: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(
        API_KEY_SECRET_SERVICE,
        &secret_account(workspace_id, key_id),
    )
    .map_err(|e| e.to_string())?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(e) => {
            let msg = e.to_string();
            if is_missing_secret_error(&msg) {
                Ok(())
            } else {
                Err(msg)
            }
        }
    }
}

pub(crate) async fn delete_api_key_raw_with_fallback(
    store: &SqliteStore,
    workspace_id: &str,
    key_id: &str,
) -> Result<(), String> {
    if use_app_state_fallback() {
        return store
            .delete_app_state(&format!("key_raw:{key_id}"))
            .await
            .map_err(|e| e.to_string());
    }
    let workspace_id = workspace_id.to_string();
    let key_id_owned = key_id.to_string();
    let _ = tokio::time::timeout(
        KEYRING_IO_TIMEOUT,
        tokio::task::spawn_blocking(move || delete_api_key_raw(&workspace_id, &key_id_owned)),
    )
    .await;
    store
        .delete_app_state(&format!("key_raw:{key_id}"))
        .await
        .map_err(|e| e.to_string())
}

pub(crate) async fn get_api_key_raw_with_legacy_migration(
    store: &SqliteStore,
    workspace_id: &str,
    key_id: &str,
) -> Result<Option<String>, String> {
    if use_app_state_fallback() {
        let legacy = store
            .get_app_state(&format!("key_raw:{key_id}"))
            .await
            .map_err(|e| e.to_string())?;
        return if legacy.trim().is_empty() {
            Ok(None)
        } else {
            Ok(Some(legacy))
        };
    }

    let workspace_id_owned = workspace_id.to_string();
    let key_id_owned = key_id.to_string();
    match tokio::time::timeout(
        KEYRING_IO_TIMEOUT,
        tokio::task::spawn_blocking(move || get_api_key_raw(&workspace_id_owned, &key_id_owned)),
    )
    .await
    {
        Ok(Ok(Ok(Some(raw)))) => return Ok(Some(raw)),
        Ok(Ok(Ok(None))) => {}
        Ok(Ok(Err(_))) => {}
        Ok(Err(_)) => {}
        Err(_) => {}
    }

    let legacy_key = format!("key_raw:{key_id}");
    let legacy = store
        .get_app_state(&legacy_key)
        .await
        .map_err(|e| e.to_string())?;
    if legacy.trim().is_empty() {
        return Ok(None);
    }

    // Keep the app_state backup as the source of truth for copy reliability.
    // Keyring backends can be unavailable/intermittent, so reads must not
    // delete recoverable key material.
    let _ = set_api_key_raw(workspace_id, key_id, &legacy);
    Ok(Some(legacy))
}
