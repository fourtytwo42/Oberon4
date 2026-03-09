use std::{
    env, fs,
    path::{Path, PathBuf},
};

use crate::{AppConfig, ConfigStore, OberonError, SavedSession, SessionStore};

#[derive(Debug, Clone)]
pub struct FileConfigStore {
    path: PathBuf,
}

impl Default for FileConfigStore {
    fn default() -> Self {
        Self::new(default_data_dir().join("config.toml"))
    }
}

impl FileConfigStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ConfigStore for FileConfigStore {
    fn load(&self) -> Result<AppConfig, OberonError> {
        if !self.path.exists() {
            return Ok(AppConfig::default());
        }
        let raw = fs::read_to_string(&self.path)
            .map_err(|err| OberonError::Storage(format!("config read failed: {err}")))?;
        AppConfig::from_toml(&raw)
    }

    fn save(&mut self, config: &AppConfig) -> Result<(), OberonError> {
        ensure_parent_dir(&self.path)?;
        let encoded = config.to_toml()?;
        fs::write(&self.path, encoded)
            .map_err(|err| OberonError::Storage(format!("config write failed: {err}")))
    }
}

#[derive(Debug, Clone)]
pub struct FileSessionStore {
    dir: PathBuf,
}

impl Default for FileSessionStore {
    fn default() -> Self {
        Self::new(default_data_dir().join("sessions"))
    }
}

impl FileSessionStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn session_path(&self, session_id: &str) -> PathBuf {
        self.dir.join(format!("{session_id}.json"))
    }
}

impl SessionStore for FileSessionStore {
    fn save_session(&mut self, session: SavedSession) -> Result<(), OberonError> {
        fs::create_dir_all(&self.dir)
            .map_err(|err| OberonError::Storage(format!("session dir create failed: {err}")))?;
        let path = self.session_path(session.summary.id.as_str());
        let encoded = serde_json::to_vec_pretty(&session)
            .map_err(|err| OberonError::Storage(format!("session encode failed: {err}")))?;
        fs::write(path, encoded)
            .map_err(|err| OberonError::Storage(format!("session write failed: {err}")))
    }

    fn load_session(&self, session_id: &str) -> Result<SavedSession, OberonError> {
        let path = self.session_path(session_id);
        let raw =
            fs::read(&path).map_err(|_| OberonError::NotFound(format!("session:{session_id}")))?;
        serde_json::from_slice(&raw)
            .map_err(|err| OberonError::Validation(format!("session decode failed: {err}")))
    }

    fn list_sessions(&self) -> Result<Vec<SavedSession>, OberonError> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }

        let mut sessions = Vec::new();
        for entry in fs::read_dir(&self.dir)
            .map_err(|err| OberonError::Storage(format!("session dir read failed: {err}")))?
        {
            let entry = entry
                .map_err(|err| OberonError::Storage(format!("session entry read failed: {err}")))?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let raw = fs::read(&path)
                .map_err(|err| OberonError::Storage(format!("session file read failed: {err}")))?;
            let session: SavedSession = serde_json::from_slice(&raw)
                .map_err(|err| OberonError::Validation(format!("session decode failed: {err}")))?;
            sessions.push(session);
        }

        sessions.sort_by(|left, right| left.summary.id.as_str().cmp(right.summary.id.as_str()));
        Ok(sessions)
    }

    fn delete_session(&mut self, session_id: &str) -> Result<(), OberonError> {
        let path = self.session_path(session_id);
        if !path.exists() {
            return Err(OberonError::NotFound(format!("session:{session_id}")));
        }
        fs::remove_file(&path)
            .map_err(|err| OberonError::Storage(format!("session delete failed: {err}")))
    }
}

pub fn default_data_dir() -> PathBuf {
    if let Some(path) = env::var_os("OBERON4_DATA_DIR") {
        return PathBuf::from(path);
    }
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".oberon4")
}

fn ensure_parent_dir(path: &Path) -> Result<(), OberonError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    fs::create_dir_all(parent)
        .map_err(|err| OberonError::Storage(format!("parent dir create failed: {err}")))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::{
        ChatMessage, ChatProviderKind, ChatRole, ConfigStore, SavedSession, SessionId,
        SessionStore, SessionSummary,
    };

    use super::{FileConfigStore, FileSessionStore};

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("oberon4-core-{name}-{nonce}"))
    }

    #[test]
    fn file_config_store_round_trips_config() {
        let dir = temp_dir("config");
        let path = dir.join("config.toml");
        let mut store = FileConfigStore::new(&path);
        let mut config = crate::AppConfig::default();
        config.endpoint.model = "o4-mini".to_string();
        store.save(&config).unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.endpoint.model, "o4-mini");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn file_session_store_round_trips_sessions() {
        let dir = temp_dir("sessions");
        let mut store = FileSessionStore::new(&dir);
        let session = SavedSession {
            summary: SessionSummary {
                id: SessionId::new("demo"),
                title: "Demo".to_string(),
                provider: ChatProviderKind::OpenAiCompatible,
                model: "gpt-4.1-mini".to_string(),
            },
            messages: vec![ChatMessage {
                role: ChatRole::Assistant,
                content: "hello".to_string(),
                tool_call_id: None,
                tool_calls: Vec::new(),
            }],
        };
        store.save_session(session.clone()).unwrap();

        assert_eq!(store.load_session("demo").unwrap(), session);
        assert_eq!(store.list_sessions().unwrap().len(), 1);
        store.delete_session("demo").unwrap();
        assert!(matches!(
            store.load_session("demo"),
            Err(crate::OberonError::NotFound(_))
        ));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn file_config_store_loads_legacy_config_shapes() {
        let dir = temp_dir("config-legacy");
        let path = dir.join("config.toml");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            &path,
            include_str!("../tests/fixtures/config-endpoint-only.toml"),
        )
        .unwrap();

        let store = FileConfigStore::new(&path);
        let loaded = store.load().unwrap();

        assert_eq!(loaded.endpoint.base_url, "http://gateway.example.test");
        assert_eq!(loaded.endpoint.model, "gpt-5.1-codex-mini");
        assert_eq!(loaded.runtime.request_timeout_ms, 30_000);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn file_session_store_loads_legacy_messages_without_tool_calls() {
        let dir = temp_dir("sessions-legacy");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("legacy-session.json"),
            include_str!("../tests/fixtures/session-legacy-missing-tool-calls.json"),
        )
        .unwrap();

        let store = FileSessionStore::new(&dir);
        let session = store.load_session("legacy-session").unwrap();

        assert_eq!(session.summary.id.as_str(), "legacy-session");
        assert_eq!(session.messages.len(), 1);
        assert!(session.messages[0].tool_calls.is_empty());
        let _ = fs::remove_dir_all(dir);
    }
}
