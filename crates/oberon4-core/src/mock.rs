use std::collections::BTreeMap;

use crate::{
    AppConfig, ChatMessage, ChatProvider, ChatRole, ChatStreamEvent, ConfigStore, OAuthProvider,
    OAuthProviderKind, OberonError, SavedSession, SessionStore, StoredAccount,
};

#[derive(Debug, Clone)]
pub struct FakeChatProvider {
    pub reply: String,
}

impl Default for FakeChatProvider {
    fn default() -> Self {
        Self {
            reply: "fake assistant reply".to_string(),
        }
    }
}

impl ChatProvider for FakeChatProvider {
    fn stream_message(
        &self,
        _endpoint: &str,
        _model: &str,
        _messages: &[ChatMessage],
        on_event: &mut dyn FnMut(ChatStreamEvent) -> Result<(), OberonError>,
    ) -> Result<ChatMessage, OberonError> {
        on_event(ChatStreamEvent::TextDelta {
            content: self.reply.clone(),
        })?;
        Ok(ChatMessage {
            role: ChatRole::Assistant,
            content: self.reply.clone(),
            tool_calls: Vec::new(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct FakeOAuthProvider {
    kind: OAuthProviderKind,
}

impl FakeOAuthProvider {
    pub fn new(kind: OAuthProviderKind) -> Self {
        Self { kind }
    }
}

impl OAuthProvider for FakeOAuthProvider {
    fn provider_kind(&self) -> OAuthProviderKind {
        self.kind
    }

    fn start_connect_flow(&self) -> Result<String, OberonError> {
        Ok(format!(
            "https://example.test/oauth/{}",
            match self.kind {
                OAuthProviderKind::OpenAi => "openai",
                OAuthProviderKind::Anthropic => "anthropic",
            }
        ))
    }

    fn exchange_callback(&self, code: &str, _state: &str) -> Result<StoredAccount, OberonError> {
        if code.trim().is_empty() {
            return Err(OberonError::Validation(
                "oauth code cannot be empty".to_string(),
            ));
        }

        Ok(StoredAccount {
            id: format!("acct-{}", code),
            provider: self.kind,
            label: "fake-account".to_string(),
            state: crate::AccountState::Active,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct InMemoryConfigStore {
    config: AppConfig,
}

impl ConfigStore for InMemoryConfigStore {
    fn load(&self) -> Result<AppConfig, OberonError> {
        Ok(self.config.clone())
    }

    fn save(&mut self, config: &AppConfig) -> Result<(), OberonError> {
        self.config = config.clone();
        Ok(())
    }
}

#[derive(Debug, Default, Clone)]
pub struct InMemorySessionStore {
    sessions: BTreeMap<String, SavedSession>,
}

impl SessionStore for InMemorySessionStore {
    fn save_session(&mut self, session: SavedSession) -> Result<(), OberonError> {
        self.sessions
            .insert(session.summary.id.as_str().to_string(), session);
        Ok(())
    }

    fn load_session(&self, session_id: &str) -> Result<SavedSession, OberonError> {
        self.sessions
            .get(session_id)
            .cloned()
            .ok_or_else(|| OberonError::NotFound(format!("session:{session_id}")))
    }

    fn list_sessions(&self) -> Result<Vec<SavedSession>, OberonError> {
        Ok(self.sessions.values().cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        ChatProvider, ChatProviderKind, ChatRole, ConfigStore, FakeChatProvider, FakeOAuthProvider,
        InMemoryConfigStore, InMemorySessionStore, OAuthProvider, OAuthProviderKind, SavedSession,
        SessionId, SessionStore, SessionSummary,
    };

    #[test]
    fn fake_chat_provider_returns_assistant_message() {
        let provider = FakeChatProvider::default();
        let reply = provider.send_message("", "", &[]).unwrap();
        assert_eq!(reply.role, ChatRole::Assistant);
        assert!(reply.tool_calls.is_empty());
    }

    #[test]
    fn fake_oauth_provider_starts_flow_and_exchanges_account() {
        let provider = FakeOAuthProvider::new(OAuthProviderKind::Anthropic);
        let url = provider.start_connect_flow().unwrap();
        let account = provider.exchange_callback("demo", "state").unwrap();

        assert!(url.contains("anthropic"));
        assert_eq!(account.provider, OAuthProviderKind::Anthropic);
    }

    #[test]
    fn config_store_round_trips_shared_config() {
        let mut store = InMemoryConfigStore::default();
        let mut config = store.load().unwrap();
        config.endpoint.model = "o4-mini".to_string();
        store.save(&config).unwrap();

        assert_eq!(store.load().unwrap().endpoint.model, "o4-mini");
    }

    #[test]
    fn session_store_lists_saved_sessions() {
        let mut store = InMemorySessionStore::default();
        store
            .save_session(SavedSession {
                summary: SessionSummary {
                    id: SessionId::new("session-1"),
                    title: "Demo".to_string(),
                    provider: ChatProviderKind::OpenAiCompatible,
                    model: "gpt-4.1-mini".to_string(),
                },
                messages: vec![],
            })
            .unwrap();

        assert_eq!(store.list_sessions().unwrap().len(), 1);
        assert_eq!(
            store.load_session("session-1").unwrap().summary.id.as_str(),
            "session-1"
        );
    }
}
