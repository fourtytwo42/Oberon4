use std::collections::BTreeMap;

use crate::{
    AppConfig, ChatMessage, ChatProvider, ChatRole, ChatStreamEvent, ConfigStore,
    OAuthCallbackPayload, OAuthProvider, OAuthProviderKind, OAuthStart, OberonError,
    PendingOAuthFlow, ProviderCapabilityMatrix, ProviderFinishState, ProviderToolDefinition,
    SavedSession, SessionStore, TokenBundle, ToolAwareChatProvider, ToolAwareProviderResponse,
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
            tool_call_id: None,
            tool_calls: Vec::new(),
        })
    }
}

impl ToolAwareChatProvider for FakeChatProvider {
    fn stream_tool_aware_message(
        &self,
        _endpoint: &str,
        _model: &str,
        _messages: &[ChatMessage],
        _tools: &[ProviderToolDefinition],
        on_event: &mut dyn FnMut(ChatStreamEvent) -> Result<(), OberonError>,
    ) -> Result<ToolAwareProviderResponse, OberonError> {
        on_event(ChatStreamEvent::TextDelta {
            content: self.reply.clone(),
        })?;
        Ok(ToolAwareProviderResponse {
            message: ChatMessage {
                role: ChatRole::Assistant,
                content: self.reply.clone(),
                tool_call_id: None,
                tool_calls: Vec::new(),
            },
            finish_state: ProviderFinishState::ResponseComplete,
        })
    }

    fn provider_capabilities(&self) -> ProviderCapabilityMatrix {
        ProviderCapabilityMatrix {
            tool_definitions: true,
            non_streaming_tool_calls: true,
            streaming_tool_calls: true,
            partial_argument_deltas: true,
            mixed_text_and_tool_responses: true,
            parallel_tool_calls: false,
            schema_cleaning_required: false,
            delegated_pending_support: false,
        }
    }

    fn endpoint_health(&self, _endpoint: &str, _model: &str) -> Result<(), OberonError> {
        Ok(())
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

    fn start_connect_flow(&self) -> Result<(OAuthStart, PendingOAuthFlow), OberonError> {
        let provider_name = match self.kind {
            OAuthProviderKind::OpenAi => "openai",
            OAuthProviderKind::Anthropic => "anthropic",
            OAuthProviderKind::Github => "github",
        };
        let flow_id = format!("{provider_name}-flow");
        let state = format!("{provider_name}-state");
        let redirect_uri = match self.kind {
            OAuthProviderKind::OpenAi => "http://localhost:1455/auth/callback".to_string(),
            OAuthProviderKind::Anthropic => "http://localhost:1455/callback".to_string(),
            OAuthProviderKind::Github => "http://localhost:1455/auth/callback".to_string(),
        };
        Ok((
            OAuthStart {
                flow_id: flow_id.clone(),
                provider: self.kind,
                authorization_url: format!(
                    "https://example.test/oauth/{provider_name}?state={state}"
                ),
                redirect_uri: redirect_uri.clone(),
                expires_at_unix: 600,
            },
            PendingOAuthFlow {
                flow_id,
                provider: self.kind,
                oauth_state: state,
                code_verifier: format!("{provider_name}-verifier"),
                redirect_uri,
                expires_at_unix: 600,
            },
        ))
    }

    fn exchange_callback(
        &self,
        flow: &PendingOAuthFlow,
        callback: OAuthCallbackPayload,
    ) -> Result<TokenBundle, OberonError> {
        if callback.code.trim().is_empty() {
            return Err(OberonError::Validation(
                "oauth code cannot be empty".to_string(),
            ));
        }

        if callback.state != flow.oauth_state {
            return Err(OberonError::Validation(
                "oauth callback state mismatch".to_string(),
            ));
        }

        Ok(TokenBundle {
            access_token: format!("access-{}", callback.code),
            refresh_token: Some(format!("refresh-{}", callback.code)),
            expires_at_unix: 600,
            account_id: format!("acct-{}", callback.code),
            label: "fake-account".to_string(),
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

    fn delete_session(&mut self, session_id: &str) -> Result<(), OberonError> {
        self.sessions
            .remove(session_id)
            .map(|_| ())
            .ok_or_else(|| OberonError::NotFound(format!("session:{session_id}")))
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
        let (start, flow) = provider.start_connect_flow().unwrap();
        let bundle = provider
            .exchange_callback(
                &flow,
                crate::OAuthCallbackPayload {
                    code: "demo".to_string(),
                    state: flow.oauth_state.clone(),
                    return_to: None,
                },
            )
            .unwrap();

        assert!(start.authorization_url.contains("anthropic"));
        assert_eq!(bundle.account_id, "acct-demo");
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

    #[test]
    fn session_store_deletes_saved_sessions() {
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

        store.delete_session("session-1").unwrap();
        assert!(matches!(
            store.load_session("session-1"),
            Err(crate::OberonError::NotFound(_))
        ));
    }
}
