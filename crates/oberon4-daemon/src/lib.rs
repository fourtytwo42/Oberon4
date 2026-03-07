use serde::{Deserialize, Serialize};

use oberon4_core::{
    AppConfig, ChatProvider, ChatProviderKind, DaemonCommand, DaemonEvent, DaemonStatus,
    InMemoryConfigStore, InMemorySessionStore, OAuthProvider, OberonError, RequestId,
    ServiceEnvelope, SessionStore,
};

pub fn daemon_status(config: &AppConfig) -> ServiceEnvelope<String> {
    ServiceEnvelope::success(format!(
        "daemon configured for {} using model {}",
        config.endpoint.base_url, config.endpoint.model
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleState {
    Stopped,
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthReport {
    pub running: bool,
    pub ready: bool,
    pub endpoint_base_url: String,
    pub model: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub level: LogLevel,
    pub request_id: RequestId,
    pub message: String,
}

pub fn log_entry(level: LogLevel, message: impl Into<String>) -> Result<String, OberonError> {
    serde_json::to_string(&LogEntry {
        level,
        request_id: RequestId::new(),
        message: message.into(),
    })
    .map_err(|err| OberonError::Storage(err.to_string()))
}

pub fn status_snapshot(config: &AppConfig, lifecycle: LifecycleState) -> DaemonStatus {
    DaemonStatus {
        running: lifecycle == LifecycleState::Running,
        endpoint_base_url: config.endpoint.base_url.clone(),
        model: config.endpoint.model.clone(),
        active_provider: ChatProviderKind::OpenAiCompatible,
        active_account: None,
    }
}

pub fn health_report(config: &AppConfig, lifecycle: LifecycleState) -> HealthReport {
    HealthReport {
        running: lifecycle == LifecycleState::Running,
        ready: lifecycle == LifecycleState::Running,
        endpoint_base_url: config.endpoint.base_url.clone(),
        model: config.endpoint.model.clone(),
    }
}

pub struct DaemonRuntime<C, S> {
    pub config_store: C,
    pub session_store: S,
    lifecycle: LifecycleState,
}

impl Default for DaemonRuntime<InMemoryConfigStore, InMemorySessionStore> {
    fn default() -> Self {
        Self {
            config_store: InMemoryConfigStore::default(),
            session_store: InMemorySessionStore::default(),
            lifecycle: LifecycleState::Stopped,
        }
    }
}

impl<C, S> DaemonRuntime<C, S>
where
    C: oberon4_core::ConfigStore,
    S: SessionStore,
{
    pub fn start(&mut self) -> Result<DaemonStatus, OberonError> {
        self.lifecycle = LifecycleState::Running;
        Ok(status_snapshot(&self.config_store.load()?, self.lifecycle))
    }

    pub fn stop(&mut self) {
        self.lifecycle = LifecycleState::Stopped;
    }

    pub fn lifecycle_state(&self) -> LifecycleState {
        self.lifecycle
    }

    pub fn status(&self) -> Result<DaemonStatus, OberonError> {
        Ok(status_snapshot(&self.config_store.load()?, self.lifecycle))
    }

    pub fn health(&self) -> Result<HealthReport, OberonError> {
        Ok(health_report(&self.config_store.load()?, self.lifecycle))
    }

    pub fn load_config(&self) -> Result<AppConfig, OberonError> {
        self.config_store.load()
    }

    pub fn save_config(&mut self, config: &AppConfig) -> Result<(), OberonError> {
        self.config_store.save(config)
    }

    pub fn handle_command<P: ChatProvider, O: OAuthProvider>(
        &mut self,
        command: DaemonCommand,
        chat_provider: &P,
        oauth_provider: &O,
    ) -> Result<DaemonEvent, OberonError> {
        match command {
            DaemonCommand::GetStatus => Ok(DaemonEvent::Status {
                status: self.status()?,
            }),
            DaemonCommand::SendChat { messages, .. } => {
                if self.lifecycle != LifecycleState::Running {
                    return Err(OberonError::Validation(
                        "daemon must be running before chat requests".to_string(),
                    ));
                }
                let config = self.config_store.load()?;
                let reply = chat_provider.send_message(
                    &config.endpoint.base_url,
                    &config.endpoint.model,
                    &messages,
                )?;
                Ok(DaemonEvent::ChatComplete { message: reply })
            }
            DaemonCommand::SaveConfig { config } => {
                self.save_config(&config)?;
                Ok(DaemonEvent::ConfigLoaded { config })
            }
            DaemonCommand::LoadConfig => Ok(DaemonEvent::ConfigLoaded {
                config: self.load_config()?,
            }),
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

pub struct LocalIpcServer<'a, C, S, P, O> {
    runtime: &'a mut DaemonRuntime<C, S>,
    chat_provider: &'a P,
    oauth_provider: &'a O,
}

impl<'a, C, S, P, O> LocalIpcServer<'a, C, S, P, O>
where
    C: oberon4_core::ConfigStore,
    S: SessionStore,
    P: ChatProvider,
    O: OAuthProvider,
{
    pub fn new(
        runtime: &'a mut DaemonRuntime<C, S>,
        chat_provider: &'a P,
        oauth_provider: &'a O,
    ) -> Self {
        Self {
            runtime,
            chat_provider,
            oauth_provider,
        }
    }

    pub fn handle_json(&mut self, input: &str) -> String {
        let response = match serde_json::from_str::<DaemonCommand>(input) {
            Ok(command) => {
                self.runtime
                    .handle_command(command, self.chat_provider, self.oauth_provider)
            }
            Err(err) => Err(OberonError::Validation(format!(
                "invalid IPC command payload: {err}"
            ))),
        };

        match response {
            Ok(event) => serde_json::to_string(&event)
                .unwrap_or_else(|err| format!(r#"{{"type":"error","message":"{err}"}}"#)),
            Err(err) => serde_json::to_string(&DaemonEvent::Error {
                message: err.to_string(),
            })
            .unwrap_or_else(|fallback_err| {
                format!(r#"{{"type":"error","message":"{fallback_err}"}}"#)
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DaemonRuntime, LifecycleState, LocalIpcServer, LogEntry, LogLevel, daemon_status, log_entry,
    };
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
    fn lifecycle_start_and_stop_update_status() {
        let mut runtime = DaemonRuntime::default();
        assert_eq!(runtime.lifecycle_state(), LifecycleState::Stopped);

        let started = runtime.start().unwrap();
        assert!(started.running);
        assert_eq!(runtime.lifecycle_state(), LifecycleState::Running);

        runtime.stop();
        assert!(!runtime.status().unwrap().running);
        assert_eq!(runtime.lifecycle_state(), LifecycleState::Stopped);
    }

    #[test]
    fn config_save_and_load_round_trip_through_runtime() {
        let mut runtime = DaemonRuntime::default();
        let mut config = runtime.load_config().unwrap();
        config.endpoint.base_url = "https://example.test".to_string();
        config.endpoint.model = "demo-model".to_string();
        runtime.save_config(&config).unwrap();

        let loaded = runtime.load_config().unwrap();
        assert_eq!(loaded.endpoint.base_url, "https://example.test");
        assert_eq!(loaded.endpoint.model, "demo-model");
    }

    #[test]
    fn health_report_reflects_lifecycle() {
        let mut runtime = DaemonRuntime::default();
        assert!(!runtime.health().unwrap().ready);
        runtime.start().unwrap();
        let health = runtime.health().unwrap();
        assert!(health.running);
        assert!(health.ready);
    }

    #[test]
    fn structured_logs_include_request_id() {
        let entry = log_entry(LogLevel::Info, "daemon started").unwrap();
        let decoded: LogEntry = serde_json::from_str(&entry).unwrap();
        assert_eq!(decoded.level, LogLevel::Info);
        assert_eq!(decoded.message, "daemon started");
        assert!(!decoded.request_id.as_str().is_empty());
    }

    #[test]
    fn ipc_get_status_returns_status_event() {
        let mut runtime = DaemonRuntime::default();
        runtime.start().unwrap();
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
    fn ipc_send_chat_requires_running_daemon() {
        let mut runtime = DaemonRuntime::default();
        let err = runtime
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
            .expect_err("chat should fail when daemon is stopped");

        assert_eq!(
            err.to_string(),
            "validation failed: daemon must be running before chat requests"
        );
    }

    #[test]
    fn ipc_send_chat_returns_fake_reply() {
        let mut runtime = DaemonRuntime::default();
        runtime.start().unwrap();
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

    #[test]
    fn local_ipc_server_handles_valid_json_requests() {
        let mut runtime = DaemonRuntime::default();
        runtime.start().unwrap();
        let command = serde_json::to_string(&DaemonCommand::GetStatus).unwrap();
        let chat_provider = FakeChatProvider::default();
        let oauth_provider = FakeOAuthProvider::new(OAuthProviderKind::OpenAi);
        let mut server = LocalIpcServer::new(&mut runtime, &chat_provider, &oauth_provider);

        let response = server.handle_json(&command);
        let event: DaemonEvent = serde_json::from_str(&response).unwrap();
        assert!(matches!(event, DaemonEvent::Status { .. }));
    }

    #[test]
    fn local_ipc_server_returns_error_event_for_malformed_requests() {
        let mut runtime = DaemonRuntime::default();
        let chat_provider = FakeChatProvider::default();
        let oauth_provider = FakeOAuthProvider::new(OAuthProviderKind::OpenAi);
        let mut server = LocalIpcServer::new(&mut runtime, &chat_provider, &oauth_provider);

        let response = server.handle_json("{bad json");
        let event: DaemonEvent = serde_json::from_str(&response).unwrap();

        match event {
            DaemonEvent::Error { message } => {
                assert!(message.contains("invalid IPC command payload"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
