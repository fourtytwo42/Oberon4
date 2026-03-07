use std::{
    fs,
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use oberon4_auth::{AccountRegistry, TokenBundle};
use oberon4_core::{
    AppConfig, ChatMessage, ChatProvider, ChatProviderKind, ConfigStore, DaemonCommand,
    DaemonEvent, DaemonStatus, FileConfigStore, FileSecretStore, FileSessionStore,
    InMemoryConfigStore, InMemorySessionStore, OAuthProvider, OAuthProviderKind, OberonError,
    RequestId, SecretStore, ServiceEnvelope, SessionStore,
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

fn status_snapshot(
    config: &AppConfig,
    lifecycle: LifecycleState,
    active_account: Option<String>,
) -> DaemonStatus {
    DaemonStatus {
        running: lifecycle == LifecycleState::Running,
        endpoint_base_url: config.endpoint.base_url.clone(),
        model: config.endpoint.model.clone(),
        active_provider: ChatProviderKind::OpenAiCompatible,
        active_account,
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
    C: ConfigStore,
    S: SessionStore,
{
    pub fn start(&mut self) -> Result<DaemonStatus, OberonError> {
        self.lifecycle = LifecycleState::Running;
        Ok(status_snapshot(
            &self.config_store.load()?,
            self.lifecycle,
            None,
        ))
    }

    pub fn stop(&mut self) {
        self.lifecycle = LifecycleState::Stopped;
    }

    pub fn lifecycle_state(&self) -> LifecycleState {
        self.lifecycle
    }

    pub fn status(&self) -> Result<DaemonStatus, OberonError> {
        Ok(status_snapshot(
            &self.config_store.load()?,
            self.lifecycle,
            None,
        ))
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
}

#[derive(Debug)]
struct CommandOutcome {
    event: DaemonEvent,
    should_shutdown: bool,
}

pub struct DaemonApp<C, S, P, OO, AO, SS> {
    runtime: DaemonRuntime<C, S>,
    chat_provider: P,
    openai_oauth: OO,
    anthropic_oauth: AO,
    accounts: AccountRegistry,
    secrets: SS,
}

impl Default
    for DaemonApp<
        FileConfigStore,
        FileSessionStore,
        oberon4_provider_openai::OpenAiCompatibleClient<oberon4_provider_openai::ReqwestTransport>,
        oberon4_auth::OpenAiOAuthAdapter,
        oberon4_auth::AnthropicOAuthAdapter,
        FileSecretStore,
    >
{
    fn default() -> Self {
        let config = AppConfig::default();
        Self::new(
            DaemonRuntime {
                config_store: FileConfigStore::default(),
                session_store: FileSessionStore::default(),
                lifecycle: LifecycleState::Stopped,
            },
            oberon4_provider_openai::OpenAiCompatibleClient::new(
                config.endpoint.base_url.clone(),
                oberon4_provider_openai::RetryPolicy::from_config(&config),
                oberon4_provider_openai::ReqwestTransport,
            ),
            oberon4_auth::OpenAiOAuthAdapter::default(),
            oberon4_auth::AnthropicOAuthAdapter::default(),
            AccountRegistry::default(),
            FileSecretStore::default(),
        )
    }
}

impl<C, S, P, OO, AO, SS> DaemonApp<C, S, P, OO, AO, SS>
where
    C: ConfigStore,
    S: SessionStore,
    P: ChatProvider,
    OO: OAuthProvider,
    AO: OAuthProvider,
    SS: SecretStore,
{
    pub fn new(
        runtime: DaemonRuntime<C, S>,
        chat_provider: P,
        openai_oauth: OO,
        anthropic_oauth: AO,
        accounts: AccountRegistry,
        secrets: SS,
    ) -> Self {
        Self {
            runtime,
            chat_provider,
            openai_oauth,
            anthropic_oauth,
            accounts,
            secrets,
        }
    }

    pub fn start(&mut self) -> Result<DaemonStatus, OberonError> {
        self.runtime.start()?;
        self.status()
    }

    pub fn status(&self) -> Result<DaemonStatus, OberonError> {
        let config = self.runtime.load_config()?;
        Ok(status_snapshot(
            &config,
            self.runtime.lifecycle_state(),
            self.active_account(),
        ))
    }

    pub(crate) fn handle_command(
        &mut self,
        command: DaemonCommand,
    ) -> Result<CommandOutcome, OberonError> {
        match command {
            DaemonCommand::GetStatus => Ok(CommandOutcome {
                event: DaemonEvent::Status {
                    status: self.status()?,
                },
                should_shutdown: false,
            }),
            DaemonCommand::Shutdown => {
                self.runtime.stop();
                Ok(CommandOutcome {
                    event: DaemonEvent::Ack {
                        message: "daemon stopped".to_string(),
                    },
                    should_shutdown: true,
                })
            }
            DaemonCommand::SendChat { messages, .. } => {
                if self.runtime.lifecycle_state() != LifecycleState::Running {
                    return Err(OberonError::Validation(
                        "daemon must be running before chat requests".to_string(),
                    ));
                }
                let config = self.runtime.load_config()?;
                let reply = self.chat_provider.send_message(
                    &config.endpoint.base_url,
                    &config.endpoint.model,
                    &messages,
                )?;
                Ok(CommandOutcome {
                    event: DaemonEvent::ChatComplete { message: reply },
                    should_shutdown: false,
                })
            }
            DaemonCommand::EndpointHealth => {
                if self.runtime.lifecycle_state() != LifecycleState::Running {
                    return Err(OberonError::Validation(
                        "daemon must be running before endpoint health checks".to_string(),
                    ));
                }
                let config = self.runtime.load_config()?;
                let probe = [ChatMessage {
                    role: oberon4_core::ChatRole::User,
                    content: "health".to_string(),
                }];
                let _ = self.chat_provider.send_message(
                    &config.endpoint.base_url,
                    &config.endpoint.model,
                    &probe,
                )?;
                Ok(CommandOutcome {
                    event: DaemonEvent::Ack {
                        message: "endpoint test passed".to_string(),
                    },
                    should_shutdown: false,
                })
            }
            DaemonCommand::SaveConfig { config } => {
                self.runtime.save_config(&config)?;
                Ok(CommandOutcome {
                    event: DaemonEvent::ConfigLoaded { config },
                    should_shutdown: false,
                })
            }
            DaemonCommand::LoadConfig => Ok(CommandOutcome {
                event: DaemonEvent::ConfigLoaded {
                    config: self.runtime.load_config()?,
                },
                should_shutdown: false,
            }),
            DaemonCommand::SaveSession { session } => {
                self.runtime.session_store.save_session(session.clone())?;
                Ok(CommandOutcome {
                    event: DaemonEvent::SessionLoaded { session },
                    should_shutdown: false,
                })
            }
            DaemonCommand::LoadSession { session_id } => {
                let session = self.runtime.session_store.load_session(&session_id)?;
                Ok(CommandOutcome {
                    event: DaemonEvent::SessionLoaded { session },
                    should_shutdown: false,
                })
            }
            DaemonCommand::ListSessions => Ok(CommandOutcome {
                event: DaemonEvent::SessionList {
                    sessions: self.runtime.session_store.list_sessions()?,
                },
                should_shutdown: false,
            }),
            DaemonCommand::ConnectAccount { provider } => {
                let url = self.oauth_provider(provider)?.start_connect_flow()?;
                let bundle = TokenBundle {
                    access_token: format!("access-{provider:?}").to_ascii_lowercase(),
                    refresh_token: Some(format!("refresh-{provider:?}").to_ascii_lowercase()),
                    expires_at_unix: 60,
                    account_id: format!("{}-default", provider_name(provider)),
                    label: format!("{} Default", provider_label(provider)),
                };
                let _ = self
                    .accounts
                    .connect(provider, &bundle, &mut self.secrets)?;
                Ok(CommandOutcome {
                    event: DaemonEvent::OAuthStarted { provider, url },
                    should_shutdown: false,
                })
            }
            DaemonCommand::ListAccounts => Ok(CommandOutcome {
                event: DaemonEvent::AccountsListed {
                    accounts: self.accounts.list(),
                },
                should_shutdown: false,
            }),
            DaemonCommand::ActivateAccount {
                provider,
                account_id,
            } => {
                self.accounts.activate(provider, &account_id)?;
                Ok(CommandOutcome {
                    event: DaemonEvent::Ack {
                        message: format!("activated {} {}", provider_name(provider), account_id),
                    },
                    should_shutdown: false,
                })
            }
            DaemonCommand::DisableAccount {
                provider,
                account_id,
            } => {
                self.accounts.disable(provider, &account_id)?;
                Ok(CommandOutcome {
                    event: DaemonEvent::Ack {
                        message: format!("disabled {} {}", provider_name(provider), account_id),
                    },
                    should_shutdown: false,
                })
            }
            DaemonCommand::DisconnectAccount {
                provider,
                account_id,
            } => {
                self.accounts.disconnect(provider, &account_id)?;
                Ok(CommandOutcome {
                    event: DaemonEvent::Ack {
                        message: format!("disconnected {} {}", provider_name(provider), account_id),
                    },
                    should_shutdown: false,
                })
            }
        }
    }

    fn oauth_provider(
        &self,
        provider: OAuthProviderKind,
    ) -> Result<&dyn OAuthProvider, OberonError> {
        match provider {
            OAuthProviderKind::OpenAi => Ok(&self.openai_oauth),
            OAuthProviderKind::Anthropic => Ok(&self.anthropic_oauth),
        }
    }

    fn active_account(&self) -> Option<String> {
        [OAuthProviderKind::OpenAi, OAuthProviderKind::Anthropic]
            .into_iter()
            .find_map(|provider| {
                self.accounts
                    .active_account_id(provider)
                    .map(str::to_string)
            })
    }
}

#[derive(Debug, Clone)]
pub struct DaemonIpcClient {
    socket_path: PathBuf,
}

impl DaemonIpcClient {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub fn send(&self, command: &DaemonCommand) -> Result<DaemonEvent, OberonError> {
        let mut stream = UnixStream::connect(&self.socket_path).map_err(|err| {
            OberonError::Storage(format!(
                "daemon socket connect failed ({}): {err}",
                self.socket_path.display()
            ))
        })?;
        let body = serde_json::to_vec(command)
            .map_err(|err| OberonError::Storage(format!("ipc request encode failed: {err}")))?;
        stream
            .write_all(&body)
            .map_err(|err| OberonError::Storage(format!("ipc request write failed: {err}")))?;
        let _ = stream.shutdown(std::net::Shutdown::Write);

        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .map_err(|err| OberonError::Storage(format!("ipc response read failed: {err}")))?;

        serde_json::from_slice(&response)
            .map_err(|err| OberonError::Validation(format!("invalid ipc response payload: {err}")))
    }
}

pub fn default_socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os("OBERON4_SOCKET_PATH") {
        return PathBuf::from(path);
    }
    std::env::temp_dir().join("oberon4-daemon.sock")
}

pub fn serve_unix_socket<C, S, P, OO, AO, SS>(
    socket_path: impl AsRef<Path>,
    mut app: DaemonApp<C, S, P, OO, AO, SS>,
) -> Result<(), OberonError>
where
    C: ConfigStore,
    S: SessionStore,
    P: ChatProvider,
    OO: OAuthProvider,
    AO: OAuthProvider,
    SS: SecretStore,
{
    let socket_path = socket_path.as_ref();
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            OberonError::Storage(format!("socket directory create failed: {err}"))
        })?;
    }
    if socket_path.exists() {
        fs::remove_file(socket_path)
            .map_err(|err| OberonError::Storage(format!("stale socket cleanup failed: {err}")))?;
    }

    let _ = app.start()?;
    let listener = UnixListener::bind(socket_path)
        .map_err(|err| OberonError::Storage(format!("socket bind failed: {err}")))?;

    let mut stop = false;
    while !stop {
        let (mut stream, _) = listener
            .accept()
            .map_err(|err| OberonError::Storage(format!("socket accept failed: {err}")))?;
        stop = handle_stream(&mut stream, &mut app)?;
    }

    let _ = fs::remove_file(socket_path);
    Ok(())
}

fn handle_stream<C, S, P, OO, AO, SS>(
    stream: &mut UnixStream,
    app: &mut DaemonApp<C, S, P, OO, AO, SS>,
) -> Result<bool, OberonError>
where
    C: ConfigStore,
    S: SessionStore,
    P: ChatProvider,
    OO: OAuthProvider,
    AO: OAuthProvider,
    SS: SecretStore,
{
    let mut input = Vec::new();
    stream
        .read_to_end(&mut input)
        .map_err(|err| OberonError::Storage(format!("ipc request read failed: {err}")))?;

    let response = match serde_json::from_slice::<DaemonCommand>(&input) {
        Ok(command) => match app.handle_command(command) {
            Ok(outcome) => (outcome.event, outcome.should_shutdown),
            Err(err) => (
                DaemonEvent::Error {
                    message: err.to_string(),
                },
                false,
            ),
        },
        Err(err) => (
            DaemonEvent::Error {
                message: format!("validation failed: invalid IPC command payload: {err}"),
            },
            false,
        ),
    };

    let encoded = serde_json::to_vec(&response.0)
        .map_err(|err| OberonError::Storage(format!("ipc response encode failed: {err}")))?;
    stream
        .write_all(&encoded)
        .map_err(|err| OberonError::Storage(format!("ipc response write failed: {err}")))?;
    Ok(response.1)
}

fn provider_name(provider: OAuthProviderKind) -> &'static str {
    match provider {
        OAuthProviderKind::OpenAi => "openai",
        OAuthProviderKind::Anthropic => "anthropic",
    }
}

fn provider_label(provider: OAuthProviderKind) -> &'static str {
    match provider {
        OAuthProviderKind::OpenAi => "OpenAI",
        OAuthProviderKind::Anthropic => "Anthropic",
    }
}

#[cfg(test)]
mod tests {
    use std::{thread, time::Duration};

    use super::{
        DaemonApp, DaemonIpcClient, DaemonRuntime, LifecycleState, LogEntry, LogLevel,
        daemon_status, default_socket_path, log_entry, serve_unix_socket,
    };
    use oberon4_auth::AccountRegistry;
    use oberon4_core::{
        AppConfig, ChatMessage, ChatProviderKind, ChatRole, DaemonCommand, DaemonEvent,
        FakeChatProvider, FakeOAuthProvider, InMemorySecretStore, OAuthProviderKind, SavedSession,
        SessionId, SessionSummary,
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
    fn app_handles_status_chat_and_accounts() {
        let mut app = DaemonApp::new(
            DaemonRuntime::default(),
            FakeChatProvider::default(),
            FakeOAuthProvider::new(OAuthProviderKind::OpenAi),
            FakeOAuthProvider::new(OAuthProviderKind::Anthropic),
            AccountRegistry::default(),
            InMemorySecretStore::default(),
        );
        let _ = app.start().unwrap();

        let status = app.handle_command(DaemonCommand::GetStatus).unwrap().event;
        match status {
            DaemonEvent::Status { status } => {
                assert!(status.running);
                assert_eq!(status.active_provider, ChatProviderKind::OpenAiCompatible);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let chat = app
            .handle_command(DaemonCommand::SendChat {
                session_id: None,
                messages: vec![ChatMessage {
                    role: ChatRole::User,
                    content: "hello".to_string(),
                }],
            })
            .unwrap()
            .event;
        match chat {
            DaemonEvent::ChatComplete { message } => {
                assert_eq!(message.role, ChatRole::Assistant);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let connect = app
            .handle_command(DaemonCommand::ConnectAccount {
                provider: OAuthProviderKind::OpenAi,
            })
            .unwrap()
            .event;
        match connect {
            DaemonEvent::OAuthStarted { provider, url } => {
                assert_eq!(provider, OAuthProviderKind::OpenAi);
                assert!(url.contains("openai"));
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let accounts = app
            .handle_command(DaemonCommand::ListAccounts)
            .unwrap()
            .event;
        match accounts {
            DaemonEvent::AccountsListed { accounts } => {
                assert_eq!(accounts.len(), 1);
                assert_eq!(accounts[0].id, "openai-default");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn ipc_send_chat_requires_running_daemon() {
        let mut app = DaemonApp::new(
            DaemonRuntime::default(),
            FakeChatProvider::default(),
            FakeOAuthProvider::new(OAuthProviderKind::OpenAi),
            FakeOAuthProvider::new(OAuthProviderKind::Anthropic),
            AccountRegistry::default(),
            InMemorySecretStore::default(),
        );
        let err = app
            .handle_command(DaemonCommand::SendChat {
                session_id: None,
                messages: vec![ChatMessage {
                    role: ChatRole::User,
                    content: "hello".to_string(),
                }],
            })
            .expect_err("chat should fail when daemon is stopped");

        assert_eq!(
            err.to_string(),
            "validation failed: daemon must be running before chat requests"
        );
    }

    #[test]
    fn socket_server_round_trips_status_and_shutdown() {
        let socket_path = default_socket_path()
            .with_file_name(format!("oberon4-daemon-test-{}.sock", std::process::id()));
        let thread_socket = socket_path.clone();
        let handle = thread::spawn(move || {
            let app = DaemonApp::new(
                DaemonRuntime::default(),
                FakeChatProvider::default(),
                FakeOAuthProvider::new(OAuthProviderKind::OpenAi),
                FakeOAuthProvider::new(OAuthProviderKind::Anthropic),
                AccountRegistry::default(),
                InMemorySecretStore::default(),
            );
            serve_unix_socket(&thread_socket, app)
        });

        for _ in 0..20 {
            if socket_path.exists() {
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }

        let client = DaemonIpcClient::new(&socket_path);
        let status = client.send(&DaemonCommand::GetStatus).unwrap();
        match status {
            DaemonEvent::Status { status } => assert!(status.running),
            other => panic!("unexpected event: {other:?}"),
        }

        let stop = client.send(&DaemonCommand::Shutdown).unwrap();
        match stop {
            DaemonEvent::Ack { message } => assert_eq!(message, "daemon stopped"),
            other => panic!("unexpected event: {other:?}"),
        }

        handle.join().unwrap().unwrap();
        assert!(!socket_path.exists());
    }

    #[test]
    fn session_commands_round_trip() {
        let mut app = DaemonApp::new(
            DaemonRuntime::default(),
            FakeChatProvider::default(),
            FakeOAuthProvider::new(OAuthProviderKind::OpenAi),
            FakeOAuthProvider::new(OAuthProviderKind::Anthropic),
            AccountRegistry::default(),
            InMemorySecretStore::default(),
        );
        let _ = app.start().unwrap();
        let session = SavedSession {
            summary: SessionSummary {
                id: SessionId::new("session-1"),
                title: "Demo".to_string(),
                provider: ChatProviderKind::OpenAiCompatible,
                model: "gpt-4.1-mini".to_string(),
            },
            messages: vec![ChatMessage {
                role: ChatRole::Assistant,
                content: "hi".to_string(),
            }],
        };

        let saved = app
            .handle_command(DaemonCommand::SaveSession {
                session: session.clone(),
            })
            .unwrap()
            .event;
        assert!(matches!(saved, DaemonEvent::SessionLoaded { .. }));

        let loaded = app
            .handle_command(DaemonCommand::LoadSession {
                session_id: "session-1".to_string(),
            })
            .unwrap()
            .event;
        match loaded {
            DaemonEvent::SessionLoaded { session: loaded } => assert_eq!(loaded, session),
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
