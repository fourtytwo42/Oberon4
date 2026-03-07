use std::collections::BTreeMap;

use crate::error::OberonError;

pub trait SecretStore {
    fn set_secret(&mut self, key: &str, value: &str) -> Result<(), OberonError>;
    fn get_secret(&self, key: &str) -> Result<String, OberonError>;
}

pub trait StateStore {
    fn put(&mut self, key: &str, value: &str) -> Result<(), OberonError>;
    fn get(&self, key: &str) -> Result<String, OberonError>;
}

#[derive(Debug, Default)]
pub struct InMemorySecretStore {
    secrets: BTreeMap<String, String>,
}

#[derive(Debug, Default)]
pub struct InMemoryStateStore {
    values: BTreeMap<String, String>,
}

impl SecretStore for InMemorySecretStore {
    fn set_secret(&mut self, key: &str, value: &str) -> Result<(), OberonError> {
        if key.trim().is_empty() {
            return Err(OberonError::Validation("secret key cannot be empty".into()));
        }
        self.secrets.insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn get_secret(&self, key: &str) -> Result<String, OberonError> {
        self.secrets
            .get(key)
            .cloned()
            .ok_or_else(|| OberonError::NotFound(format!("secret:{key}")))
    }
}

impl StateStore for InMemoryStateStore {
    fn put(&mut self, key: &str, value: &str) -> Result<(), OberonError> {
        if key.trim().is_empty() {
            return Err(OberonError::Validation("state key cannot be empty".into()));
        }
        self.values.insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn get(&self, key: &str) -> Result<String, OberonError> {
        self.values
            .get(key)
            .cloned()
            .ok_or_else(|| OberonError::NotFound(format!("state:{key}")))
    }
}

#[cfg(test)]
mod tests {
    use super::{InMemorySecretStore, InMemoryStateStore, SecretStore, StateStore};
    use crate::OberonError;

    #[test]
    fn secret_store_round_trips_values() {
        let mut store = InMemorySecretStore::default();
        store
            .set_secret("provider.openai", "token")
            .expect("secret should store");
        assert_eq!(store.get_secret("provider.openai").unwrap(), "token");
    }

    #[test]
    fn state_store_round_trips_values() {
        let mut store = InMemoryStateStore::default();
        store
            .put("active.provider", "openai")
            .expect("state should store");
        assert_eq!(store.get("active.provider").unwrap(), "openai");
    }

    #[test]
    fn empty_keys_are_rejected() {
        let mut secret_store = InMemorySecretStore::default();
        let mut state_store = InMemoryStateStore::default();

        let secret_err = secret_store
            .set_secret("", "token")
            .expect_err("empty secret key should fail");
        let state_err = state_store
            .put("", "value")
            .expect_err("empty state key should fail");

        assert!(matches!(secret_err, OberonError::Validation(_)));
        assert!(matches!(state_err, OberonError::Validation(_)));
    }
}
