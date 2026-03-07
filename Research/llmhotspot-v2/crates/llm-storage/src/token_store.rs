use crate::StorageError;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredTokenRecord {
    pub schema_version: String,
    pub workspace_id: String,
    pub connection_id: String,
    pub refresh_token: String,
    pub account_id: Option<String>,
    pub stored_at_unix: u64,
}

impl StoredTokenRecord {
    pub fn new(workspace_id: &str, connection_id: &str, refresh_token: &str) -> Self {
        Self {
            schema_version: "llm_storage_token_v1".to_string(),
            workspace_id: workspace_id.to_string(),
            connection_id: connection_id.to_string(),
            refresh_token: refresh_token.to_string(),
            account_id: None,
            stored_at_unix: now_unix(),
        }
    }
}

pub trait TokenStore: Send + Sync {
    fn save(
        &self,
        workspace_id: &str,
        connection_id: &str,
        record: &StoredTokenRecord,
    ) -> Result<(), StorageError>;
    fn load(
        &self,
        workspace_id: &str,
        connection_id: &str,
    ) -> Result<Option<StoredTokenRecord>, StorageError>;
    fn delete(&self, workspace_id: &str, connection_id: &str) -> Result<(), StorageError>;
}

#[derive(Debug, Clone)]
pub struct KeyringTokenStore {
    service_name: String,
}

impl Default for KeyringTokenStore {
    fn default() -> Self {
        Self {
            service_name: "llmhotspot-v2-token-store".to_string(),
        }
    }
}

impl KeyringTokenStore {
    pub fn new(service_name: &str) -> Self {
        Self {
            service_name: service_name.to_string(),
        }
    }

    fn username(&self, workspace_id: &str, connection_id: &str) -> String {
        format!("{workspace_id}:{connection_id}")
    }
}

impl TokenStore for KeyringTokenStore {
    fn save(
        &self,
        workspace_id: &str,
        connection_id: &str,
        record: &StoredTokenRecord,
    ) -> Result<(), StorageError> {
        let entry = keyring::Entry::new(
            &self.service_name,
            &self.username(workspace_id, connection_id),
        )
        .map_err(|e| StorageError::Keyring(e.to_string()))?;
        let payload = serde_json::to_string(record)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        entry
            .set_password(&payload)
            .map_err(|e| StorageError::Keyring(e.to_string()))
    }

    fn load(
        &self,
        workspace_id: &str,
        connection_id: &str,
    ) -> Result<Option<StoredTokenRecord>, StorageError> {
        let entry = keyring::Entry::new(
            &self.service_name,
            &self.username(workspace_id, connection_id),
        )
        .map_err(|e| StorageError::Keyring(e.to_string()))?;

        match entry.get_password() {
            Ok(raw) => {
                let parsed = serde_json::from_str(&raw)
                    .map_err(|e| StorageError::Serialization(e.to_string()))?;
                Ok(Some(parsed))
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("No entry") || msg.contains("NoPassword") {
                    return Ok(None);
                }
                Err(StorageError::Keyring(msg))
            }
        }
    }

    fn delete(&self, workspace_id: &str, connection_id: &str) -> Result<(), StorageError> {
        let entry = keyring::Entry::new(
            &self.service_name,
            &self.username(workspace_id, connection_id),
        )
        .map_err(|e| StorageError::Keyring(e.to_string()))?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("No entry") || msg.contains("NoPassword") {
                    return Ok(());
                }
                Err(StorageError::Keyring(msg))
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct InMemoryTokenStore {
    inner: Arc<Mutex<HashMap<String, StoredTokenRecord>>>,
}

impl InMemoryTokenStore {
    fn key(workspace_id: &str, connection_id: &str) -> String {
        format!("{workspace_id}:{connection_id}")
    }
}

impl TokenStore for InMemoryTokenStore {
    fn save(
        &self,
        workspace_id: &str,
        connection_id: &str,
        record: &StoredTokenRecord,
    ) -> Result<(), StorageError> {
        let mut guard = self.inner.lock().expect("token store mutex poisoned");
        guard.insert(Self::key(workspace_id, connection_id), record.clone());
        Ok(())
    }

    fn load(
        &self,
        workspace_id: &str,
        connection_id: &str,
    ) -> Result<Option<StoredTokenRecord>, StorageError> {
        let guard = self.inner.lock().expect("token store mutex poisoned");
        Ok(guard.get(&Self::key(workspace_id, connection_id)).cloned())
    }

    fn delete(&self, workspace_id: &str, connection_id: &str) -> Result<(), StorageError> {
        let mut guard = self.inner.lock().expect("token store mutex poisoned");
        guard.remove(&Self::key(workspace_id, connection_id));
        Ok(())
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
