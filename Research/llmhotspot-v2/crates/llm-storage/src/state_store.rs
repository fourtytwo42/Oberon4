use crate::StorageError;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

pub trait StateStore: Send + Sync {
    fn put(&self, key: &str, value: &str, ttl_secs: Option<u64>) -> Result<(), StorageError>;
    fn get(&self, key: &str) -> Result<String, StorageError>;
    fn delete(&self, key: &str) -> Result<(), StorageError>;
}

#[derive(Debug, Clone, Default)]
pub struct InMemoryStateStore {
    inner: Arc<Mutex<HashMap<String, StateEntry>>>,
}

#[derive(Debug, Clone)]
struct StateEntry {
    value: String,
    expires_unix: Option<u64>,
}

impl StateStore for InMemoryStateStore {
    fn put(&self, key: &str, value: &str, ttl_secs: Option<u64>) -> Result<(), StorageError> {
        if key.trim().is_empty() {
            return Err(StorageError::InvalidKey);
        }
        let mut guard = self.inner.lock().expect("state store mutex poisoned");
        prune(&mut guard);
        guard.insert(
            key.to_string(),
            StateEntry {
                value: value.to_string(),
                expires_unix: ttl_secs.map(|ttl| now_unix().saturating_add(ttl)),
            },
        );
        Ok(())
    }

    fn get(&self, key: &str) -> Result<String, StorageError> {
        let mut guard = self.inner.lock().expect("state store mutex poisoned");
        prune(&mut guard);
        guard
            .get(key)
            .map(|v| v.value.clone())
            .ok_or(StorageError::NotFound)
    }

    fn delete(&self, key: &str) -> Result<(), StorageError> {
        let mut guard = self.inner.lock().expect("state store mutex poisoned");
        guard.remove(key);
        Ok(())
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn prune(map: &mut HashMap<String, StateEntry>) {
    let now = now_unix();
    map.retain(|_, v| v.expires_unix.map(|x| x > now).unwrap_or(true));
}
