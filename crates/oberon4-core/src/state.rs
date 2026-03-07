use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

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

#[derive(Debug, Clone)]
pub struct FileSecretStore {
    path: PathBuf,
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

impl Default for FileSecretStore {
    fn default() -> Self {
        Self::new(crate::persistence::default_data_dir().join("secrets.json"))
    }
}

impl FileSecretStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn load_map(&self) -> Result<BTreeMap<String, String>, OberonError> {
        if !self.path.exists() {
            return Ok(BTreeMap::new());
        }
        let raw = fs::read(&self.path)
            .map_err(|err| OberonError::Storage(format!("secret store read failed: {err}")))?;
        serde_json::from_slice(&raw)
            .map_err(|err| OberonError::Validation(format!("secret store decode failed: {err}")))
    }

    fn save_map(&self, values: &BTreeMap<String, String>) -> Result<(), OberonError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                OberonError::Storage(format!("secret store parent create failed: {err}"))
            })?;
        }
        let encoded = serde_json::to_vec_pretty(values)
            .map_err(|err| OberonError::Storage(format!("secret store encode failed: {err}")))?;
        fs::write(&self.path, encoded)
            .map_err(|err| OberonError::Storage(format!("secret store write failed: {err}")))?;
        set_private_permissions(&self.path)?;
        Ok(())
    }
}

impl SecretStore for FileSecretStore {
    fn set_secret(&mut self, key: &str, value: &str) -> Result<(), OberonError> {
        if key.trim().is_empty() {
            return Err(OberonError::Validation("secret key cannot be empty".into()));
        }
        let mut values = self.load_map()?;
        values.insert(key.to_string(), value.to_string());
        self.save_map(&values)
    }

    fn get_secret(&self, key: &str) -> Result<String, OberonError> {
        self.load_map()?
            .remove(key)
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

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> Result<(), OberonError> {
    use std::os::unix::fs::PermissionsExt;

    let permissions = fs::Permissions::from_mode(0o600);
    fs::set_permissions(path, permissions)
        .map_err(|err| OberonError::Storage(format!("secret permission update failed: {err}")))
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> Result<(), OberonError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        FileSecretStore, InMemorySecretStore, InMemoryStateStore, SecretStore, StateStore,
    };
    use crate::OberonError;

    fn temp_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("oberon4-state-{name}-{nonce}.json"))
    }

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

    #[test]
    fn file_secret_store_round_trips_values() {
        let path = temp_path("secret");
        let mut store = FileSecretStore::new(&path);
        store.set_secret("provider.openai", "token").unwrap();

        assert_eq!(store.get_secret("provider.openai").unwrap(), "token");
        let _ = fs::remove_file(path);
    }
}
