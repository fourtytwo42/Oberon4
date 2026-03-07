use crate::{
    AppConfig, ChatMessage, DaemonStatus, OAuthProviderKind, OberonError, SavedSession,
    StoredAccount,
};

pub trait ChatProvider {
    fn send_message(
        &self,
        endpoint: &str,
        model: &str,
        messages: &[ChatMessage],
    ) -> Result<ChatMessage, OberonError>;
}

pub trait OAuthProvider {
    fn provider_kind(&self) -> OAuthProviderKind;
    fn start_connect_flow(&self) -> Result<String, OberonError>;
    fn exchange_callback(&self, code: &str, state: &str) -> Result<StoredAccount, OberonError>;
}

pub trait ConfigStore {
    fn load(&self) -> Result<AppConfig, OberonError>;
    fn save(&mut self, config: &AppConfig) -> Result<(), OberonError>;
}

pub trait SessionStore {
    fn save_session(&mut self, session: SavedSession) -> Result<(), OberonError>;
    fn load_session(&self, session_id: &str) -> Result<SavedSession, OberonError>;
    fn list_sessions(&self) -> Result<Vec<SavedSession>, OberonError>;
}

pub trait StatusProvider {
    fn status(&self) -> Result<DaemonStatus, OberonError>;
}
