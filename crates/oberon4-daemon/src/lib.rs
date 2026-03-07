use oberon4_core::{
    AppConfig, ChatProvider, DaemonCommand, DaemonEvent, DaemonStatus, InMemoryConfigStore,
    InMemorySessionStore, OAuthProvider, OberonError, ServiceEnvelope, SessionStore,
};

pub fn daemon_status(config: &AppConfig) -> ServiceEnvelope<String> {
    ServiceEnvelope::success(format!(
        "daemon configured for {} using model {}",
        config.endpoint.base_url, config.endpoint.model
    ))
}

pub fn status_snapshot(config: &AppConfig) -> DaemonStatus {
    DaemonStatus {
        running: true,
        endpoint_base_url: config.endpoint.base_url.clone(),
        model: config.endpoint.model.clone(),
        active_provider: oberon4_core::ChatProviderKind::OpenAiCompatible,
        active_account: None,
    }
}

pub struct DaemonRuntime<C, S> {
    pub config_store: C,
    pub session_store: S,
}

impl Default for DaemonRuntime<InMemoryConfigStore, InMemorySessionStore> {
    fn default() -> Self {
        Self {
            config_store: InMemoryConfigStore::default(),
            session_store: InMemorySessionStore::default(),
        }
    }
}

impl<C, S> DaemonRuntime<C, S>
where
    C: oberon4_core::ConfigStore,
    S: SessionStore,
{
    pub fn handle_command<P: ChatProvider, O: OAuthProvider>(
        &mut self,
        command: DaemonCommand,
        chat_provider: &P,
        oauth_provider: &O,
    ) -> Result<DaemonEvent, OberonError> {
        match command {
            DaemonCommand::GetStatus => {
                let config = self.config_store.load()?;
                Ok(DaemonEvent::Status {
                    status: status_snapshot(&config),
                })
            }
            DaemonCommand::SendChat { messages, .. } => {
                let config = self.config_store.load()?;
                let reply = chat_provider.send_message(
                    &config.endpoint.base_url,
                    &config.endpoint.model,
                    &messages,
                )?;
                Ok(DaemonEvent::ChatComplete { message: reply })
            }
            DaemonCommand::SaveConfig { config } => {
                self.config_store.save(&config)?;
                Ok(DaemonEvent::ConfigLoaded { config })
            }
            DaemonCommand::LoadConfig => {
                let config = self.config_store.load()?;
                Ok(DaemonEvent::ConfigLoaded { config })
            }
            DaemonCommand::SaveSession { session } => {
                self.session_store.save_session(session.clone())?;
                Ok(DaemonEvent::SessionLoaded { session })
            }
            DaemonCommand::LoadSession { session_id } => {
                let session = self.session_store.load_session(&session_id)?;
                Ok(DaemonEvent::SessionLoaded { session })
            }
            DaemonCommand::ListSessions => {
                let sessions = self.session_store.list_sessions()?;
                Ok(DaemonEvent::SessionList { sessions })
            }
            DaemonCommand::ConnectAccount { provider } => {
                if provider != oauth_provider.provider_kind() {
                    return Err(OberonError::Validation(
                        "oauth provider mismatch for runtime".to_string(),
                    ));
                }
                let url = oauth_provider.start_connect_flow()?;
                Ok(DaemonEvent::OAuthStarted { provider, url })
            }
            DaemonCommand::ListAccounts => Ok(DaemonEvent::AccountsListed { accounts: vec![] }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DaemonRuntime, daemon_status};
    use oberon4_core::{
        AppConfig, ChatMessage, ChatProviderKind, ChatRole, DaemonCommand, DaemonEvent,
        FakeChatProvider, FakeOAuthProvider, OAuthProviderKind, SavedSession, SessionId,
        SessionSummary,
    };

    #[test]
    fn daemon_status_uses_shared_config() {
        let status = daemon_status(&AppConfig::default());
        assert!(status.ok);
        assert!(status.data.unwrap().contains("https://api.openai.com"));
    }

    #[test]
    fn ipc_get_status_returns_status_event() {
        let mut runtime = DaemonRuntime::default();
        let event = runtime
            .handle_command(
                DaemonCommand::GetStatus,
                &FakeChatProvider::default(),
                &FakeOAuthProvider::new(OAuthProviderKind::OpenAi),
            )
            .unwrap();

        match event {
            DaemonEvent::Status { status } => {
                assert!(status.running);
                assert_eq!(status.active_provider, ChatProviderKind::OpenAiCompatible);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn ipc_send_chat_returns_fake_reply() {
        let mut runtime = DaemonRuntime::default();
        let event = runtime
            .handle_command(
                DaemonCommand::SendChat {
                    session_id: None,
                    messages: vec![ChatMessage {
                        role: ChatRole::User,
                        content: "hello".to_string(),
                    }],
                },
                &FakeChatProvider::default(),
                &FakeOAuthProvider::new(OAuthProviderKind::OpenAi),
            )
            .unwrap();

        match event {
            DaemonEvent::ChatComplete { message } => {
                assert_eq!(message.role, ChatRole::Assistant);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn ipc_session_round_trips() {
        let mut runtime = DaemonRuntime::default();
        let saved = SavedSession {
            summary: SessionSummary {
                id: SessionId::new("session-1"),
                title: "Demo".to_string(),
                provider: ChatProviderKind::OpenAiCompatible,
                model: "gpt-4.1-mini".to_string(),
            },
            messages: vec![],
        };

        runtime
            .handle_command(
                DaemonCommand::SaveSession {
                    session: saved.clone(),
                },
                &FakeChatProvider::default(),
                &FakeOAuthProvider::new(OAuthProviderKind::OpenAi),
            )
            .unwrap();

        let event = runtime
            .handle_command(
                DaemonCommand::LoadSession {
                    session_id: "session-1".to_string(),
                },
                &FakeChatProvider::default(),
                &FakeOAuthProvider::new(OAuthProviderKind::OpenAi),
            )
            .unwrap();

        match event {
            DaemonEvent::SessionLoaded { session } => {
                assert_eq!(session.summary.id.as_str(), "session-1");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn ipc_connect_account_enforces_provider_match() {
        let mut runtime = DaemonRuntime::default();
        let err = runtime
            .handle_command(
                DaemonCommand::ConnectAccount {
                    provider: OAuthProviderKind::Anthropic,
                },
                &FakeChatProvider::default(),
                &FakeOAuthProvider::new(OAuthProviderKind::OpenAi),
            )
            .expect_err("mismatched provider should fail");

        assert_eq!(
            err.to_string(),
            "validation failed: oauth provider mismatch for runtime"
        );
    }
}
