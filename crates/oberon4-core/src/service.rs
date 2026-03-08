use crate::{
    AppConfig, ChatMessage, ChatToolCallDelta, DaemonStatus, OAuthProviderKind, OberonError,
    SavedSession, StoredAccount,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatStreamEvent {
    TextDelta { content: String },
    ToolCallDelta(ChatToolCallDelta),
}

pub trait ChatProvider {
    fn stream_message(
        &self,
        endpoint: &str,
        model: &str,
        messages: &[ChatMessage],
        on_event: &mut dyn FnMut(ChatStreamEvent) -> Result<(), OberonError>,
    ) -> Result<ChatMessage, OberonError>;

    fn send_message(
        &self,
        endpoint: &str,
        model: &str,
        messages: &[ChatMessage],
    ) -> Result<ChatMessage, OberonError> {
        self.stream_message(endpoint, model, messages, &mut |_| Ok(()))
    }

    fn endpoint_health(&self, endpoint: &str, model: &str) -> Result<(), OberonError> {
        let probe = [ChatMessage {
            role: crate::ChatRole::User,
            content: "health".to_string(),
            tool_calls: Vec::new(),
        }];
        self.send_message(endpoint, model, &probe).map(|_| ())
    }

    fn apply_runtime_config(&mut self, _config: &AppConfig) -> Result<(), OberonError> {
        Ok(())
    }
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
    fn delete_session(&mut self, session_id: &str) -> Result<(), OberonError>;
}

pub trait StatusProvider {
    fn status(&self) -> Result<DaemonStatus, OberonError>;
}
