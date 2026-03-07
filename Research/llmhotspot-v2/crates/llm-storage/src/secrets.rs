use crate::StorageError;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub trait SecretStore: Send + Sync {
    fn put_secret(&self, key: &str, value: &str) -> Result<(), StorageError>;
    fn get_secret(&self, key: &str) -> Result<String, StorageError>;
    fn delete_secret(&self, key: &str) -> Result<(), StorageError>;
}

#[derive(Debug, Clone, Default)]
pub struct InMemorySecretStore {
    inner: Arc<Mutex<HashMap<String, String>>>,
}

impl SecretStore for InMemorySecretStore {
    fn put_secret(&self, key: &str, value: &str) -> Result<(), StorageError> {
        if key.trim().is_empty() {
            return Err(StorageError::InvalidKey);
        }
        let mut guard = self.inner.lock().expect("secret store mutex poisoned");
        guard.insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn get_secret(&self, key: &str) -> Result<String, StorageError> {
        let guard = self.inner.lock().expect("secret store mutex poisoned");
        guard.get(key).cloned().ok_or(StorageError::NotFound)
    }

    fn delete_secret(&self, key: &str) -> Result<(), StorageError> {
        let mut guard = self.inner.lock().expect("secret store mutex poisoned");
        guard.remove(key);
        Ok(())
    }
}
