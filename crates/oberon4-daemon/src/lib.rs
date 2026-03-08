pub mod plugin_runtime;

use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, BufReader, Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use oberon4_auth::{AccountMetadataStore, AccountRegistry, FileAccountMetadataStore};
use oberon4_core::{
    AppConfig, ChatProvider, ChatProviderKind, ChatStreamEvent, ConfigStore, DaemonCommand,
    DaemonEvent, DaemonStatus, FileConfigStore, FileSecretStore, FileSessionStore,
    InMemoryConfigStore, InMemorySessionStore, OAuthProvider, OAuthProviderKind, OberonError,
    PendingOAuthFlow, RequestId, SecretStore, ServiceEnvelope, SessionStore,
};

pub use plugin_runtime::{
    ArtifactBrokerFoundation, FilePluginRegistry, HOST_RUNTIME_API_VERSION, PluginRuntimeHost,
    RegisteredPlugin, RuntimeBrokerFoundations, StaticPolicyResolver, StorageBrokerFoundation,
    validate_manifest,
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

pub struct DaemonApp<C, S, P, OO, AO, SS, AS> {
    runtime: DaemonRuntime<C, S>,
    chat_provider: P,
    openai_oauth: OO,
    anthropic_oauth: AO,
    accounts: AccountRegistry,
    pending_oauth_flows: BTreeMap<String, PendingOAuthFlow>,
    secrets: SS,
    account_store: AS,
}

impl Default
    for DaemonApp<
        FileConfigStore,
        FileSessionStore,
        oberon4_provider_openai::OpenAiCompatibleClient<oberon4_provider_openai::ReqwestTransport>,
        oberon4_auth::OpenAiOAuthAdapter,
        oberon4_auth::AnthropicOAuthAdapter,
        FileSecretStore,
        FileAccountMetadataStore,
    >
{
    fn default() -> Self {
        let config = AppConfig::default();
        let account_store = FileAccountMetadataStore::default();
        let accounts = account_store.load_registry().unwrap_or_default();
        let mut client = oberon4_provider_openai::OpenAiCompatibleClient::new(
            config.endpoint.base_url.clone(),
            config.endpoint.api_key_env.clone(),
            oberon4_provider_openai::RetryPolicy::from_config(&config),
            oberon4_provider_openai::ReqwestTransport,
        );
        client.set_streaming_config(
            config.runtime.stream_responses,
            config.runtime.stream_idle_timeout_ms,
        );
        Self::new(
            DaemonRuntime {
                config_store: FileConfigStore::default(),
                session_store: FileSessionStore::default(),
                lifecycle: LifecycleState::Stopped,
            },
            client,
            oberon4_auth::OpenAiOAuthAdapter::default(),
            oberon4_auth::AnthropicOAuthAdapter::default(),
            accounts,
            FileSecretStore::default(),
            account_store,
        )
    }
}

impl<C, S, P, OO, AO, SS, AS> DaemonApp<C, S, P, OO, AO, SS, AS>
where
    C: ConfigStore,
    S: SessionStore,
    P: ChatProvider,
    OO: OAuthProvider,
    AO: OAuthProvider,
    SS: SecretStore,
    AS: AccountMetadataStore,
{
    pub fn new(
        runtime: DaemonRuntime<C, S>,
        chat_provider: P,
        openai_oauth: OO,
        anthropic_oauth: AO,
        accounts: AccountRegistry,
        secrets: SS,
        account_store: AS,
    ) -> Self {
        Self {
            runtime,
            chat_provider,
            openai_oauth,
            anthropic_oauth,
            accounts,
            pending_oauth_flows: BTreeMap::new(),
            secrets,
            account_store,
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

    fn sync_api_key_secret(&mut self, config: &AppConfig) -> Result<(), OberonError> {
        let Some(env_name) = config
            .endpoint
            .api_key_env
            .as_deref()
            .filter(|name| !name.trim().is_empty())
        else {
            return Ok(());
        };

        if let Ok(value) = std::env::var(env_name) {
            self.secrets
                .set_secret(&format!("env.{env_name}"), &value)?;
        }

        Ok(())
    }

    fn materialize_api_key_env(&mut self, config: &AppConfig) -> Result<(), OberonError> {
        let Some(env_name) = config
            .endpoint
            .api_key_env
            .as_deref()
            .filter(|name| !name.trim().is_empty())
        else {
            return Ok(());
        };

        if std::env::var(env_name).is_ok() {
            return Ok(());
        }

        let value = match self.secrets.get_secret(&format!("env.{env_name}")) {
            Ok(value) => value,
            Err(OberonError::NotFound(_)) => return Ok(()),
            Err(err) => return Err(err),
        };
        unsafe {
            std::env::set_var(env_name, value);
        }
        Ok(())
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
            DaemonCommand::SendChat { .. } => Err(OberonError::Validation(
                "SendChat must be handled through the streaming IPC path".to_string(),
            )),
            DaemonCommand::EndpointHealth => {
                if self.runtime.lifecycle_state() != LifecycleState::Running {
                    return Err(OberonError::Validation(
                        "daemon must be running before endpoint health checks".to_string(),
                    ));
                }
                let config = self.runtime.load_config()?;
                self.materialize_api_key_env(&config)?;
                self.chat_provider
                    .endpoint_health(&config.endpoint.base_url, &config.endpoint.model)?;
                Ok(CommandOutcome {
                    event: DaemonEvent::Ack {
                        message: "endpoint test passed".to_string(),
                    },
                    should_shutdown: false,
                })
            }
            DaemonCommand::SaveConfig { config } => {
                self.sync_api_key_secret(&config)?;
                self.chat_provider.apply_runtime_config(&config)?;
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
            DaemonCommand::DeleteSession { session_id } => {
                self.runtime.session_store.delete_session(&session_id)?;
                Ok(CommandOutcome {
                    event: DaemonEvent::Ack {
                        message: format!("deleted session {session_id}"),
                    },
                    should_shutdown: false,
                })
            }
            DaemonCommand::ConnectAccount { provider } => {
                let (start, pending) = self.oauth_provider(provider)?.start_connect_flow()?;
                self.pending_oauth_flows
                    .insert(start.flow_id.clone(), pending);
                Ok(CommandOutcome {
                    event: DaemonEvent::OAuthStarted {
                        provider,
                        flow_id: start.flow_id,
                        url: start.authorization_url,
                        redirect_uri: start.redirect_uri,
                    },
                    should_shutdown: false,
                })
            }
            DaemonCommand::CompleteOAuthConnect { flow_id, callback } => {
                let pending = self
                    .pending_oauth_flows
                    .remove(&flow_id)
                    .ok_or_else(|| OberonError::NotFound(format!("oauth_flow:{flow_id}")))?;
                let provider = pending.provider;
                let bundle = self
                    .oauth_provider(provider)?
                    .exchange_callback(&pending, callback)?;
                let account = self
                    .accounts
                    .connect(provider, &bundle, &mut self.secrets)?;
                self.persist_accounts()?;
                Ok(CommandOutcome {
                    event: DaemonEvent::Ack {
                        message: format!("connected {} {}", provider_name(provider), account.id),
                    },
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
                self.persist_accounts()?;
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
                self.persist_accounts()?;
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
                self.persist_accounts()?;
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

    fn persist_accounts(&mut self) -> Result<(), OberonError> {
        self.account_store.save_registry(&self.accounts)
    }

    fn handle_chat_stream(
        &mut self,
        messages: Vec<oberon4_core::ChatMessage>,
        on_event: &mut dyn FnMut(DaemonEvent) -> Result<(), OberonError>,
    ) -> Result<(), OberonError> {
        if self.runtime.lifecycle_state() != LifecycleState::Running {
            return Err(OberonError::Validation(
                "daemon must be running before chat requests".to_string(),
            ));
        }

        let config = self.runtime.load_config()?;
        self.materialize_api_key_env(&config)?;
        let reply = self.chat_provider.stream_message(
            &config.endpoint.base_url,
            &config.endpoint.model,
            &messages,
            &mut |event| match event {
                ChatStreamEvent::TextDelta { content } => {
                    on_event(DaemonEvent::ChatChunk { content })
                }
                ChatStreamEvent::ToolCallDelta(delta) => {
                    on_event(DaemonEvent::ToolCallDelta { delta })
                }
            },
        )?;
        on_event(DaemonEvent::ChatComplete { message: reply })
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
        let mut events = Vec::new();
        self.send_stream(command, &mut |event| {
            events.push(event);
            Ok(())
        })?;
        match events.as_slice() {
            [event] => Ok(event.clone()),
            [] => Err(OberonError::Storage(
                "daemon stream closed without any response events".to_string(),
            )),
            _ => Err(OberonError::Storage(
                "daemon returned multiple events for unary request".to_string(),
            )),
        }
    }

    pub fn send_stream(
        &self,
        command: &DaemonCommand,
        on_event: &mut dyn FnMut(DaemonEvent) -> Result<(), OberonError>,
    ) -> Result<(), OberonError> {
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

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        loop {
            line.clear();
            let bytes = reader
                .read_line(&mut line)
                .map_err(|err| OberonError::Storage(format!("ipc response read failed: {err}")))?;
            if bytes == 0 {
                return Ok(());
            }
            if line.trim().is_empty() {
                continue;
            }
            let event: DaemonEvent = serde_json::from_str(line.trim_end()).map_err(|err| {
                OberonError::Validation(format!("invalid ipc response payload: {err}"))
            })?;
            on_event(event)?;
        }
    }
}

pub fn default_socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os("OBERON4_SOCKET_PATH") {
        return PathBuf::from(path);
    }
    std::env::temp_dir().join("oberon4-daemon.sock")
}

pub fn serve_unix_socket<C, S, P, OO, AO, SS, AS>(
    socket_path: impl AsRef<Path>,
    mut app: DaemonApp<C, S, P, OO, AO, SS, AS>,
) -> Result<(), OberonError>
where
    C: ConfigStore,
    S: SessionStore,
    P: ChatProvider,
    OO: OAuthProvider,
    AO: OAuthProvider,
    SS: SecretStore,
    AS: AccountMetadataStore,
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

fn handle_stream<C, S, P, OO, AO, SS, AS>(
    stream: &mut UnixStream,
    app: &mut DaemonApp<C, S, P, OO, AO, SS, AS>,
) -> Result<bool, OberonError>
where
    C: ConfigStore,
    S: SessionStore,
    P: ChatProvider,
    OO: OAuthProvider,
    AO: OAuthProvider,
    SS: SecretStore,
    AS: AccountMetadataStore,
{
    let mut input = Vec::new();
    stream
        .read_to_end(&mut input)
        .map_err(|err| OberonError::Storage(format!("ipc request read failed: {err}")))?;

    match serde_json::from_slice::<DaemonCommand>(&input) {
        Ok(DaemonCommand::SendChat { messages, .. }) => {
            let result = app.handle_chat_stream(messages, &mut |event| write_event(stream, &event));
            if let Err(err) = result {
                write_event(
                    stream,
                    &DaemonEvent::Error {
                        message: err.to_string(),
                    },
                )?;
            }
            Ok(false)
        }
        Ok(command) => {
            let response = match app.handle_command(command) {
                Ok(outcome) => (outcome.event, outcome.should_shutdown),
                Err(err) => (
                    DaemonEvent::Error {
                        message: err.to_string(),
                    },
                    false,
                ),
            };
            write_event(stream, &response.0)?;
            Ok(response.1)
        }
        Err(err) => {
            write_event(
                stream,
                &DaemonEvent::Error {
                    message: format!("validation failed: invalid IPC command payload: {err}"),
                },
            )?;
            Ok(false)
        }
    }
}

fn write_event(stream: &mut UnixStream, event: &DaemonEvent) -> Result<(), OberonError> {
    let encoded = serde_json::to_vec(event)
        .map_err(|err| OberonError::Storage(format!("ipc response encode failed: {err}")))?;
    stream
        .write_all(&encoded)
        .and_then(|_| stream.write_all(b"\n"))
        .map_err(|err| OberonError::Storage(format!("ipc response write failed: {err}")))
}

fn provider_name(provider: OAuthProviderKind) -> &'static str {
    match provider {
        OAuthProviderKind::OpenAi => "openai",
        OAuthProviderKind::Anthropic => "anthropic",
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs, thread,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use super::{
        DaemonApp, DaemonIpcClient, DaemonRuntime, LifecycleState, LogEntry, LogLevel,
        daemon_status, default_socket_path, log_entry, serve_unix_socket,
    };
    use oberon4_auth::{
        AccountMetadataStore, AccountRegistry, FileAccountMetadataStore,
        InMemoryAccountMetadataStore,
    };
    use oberon4_core::{
        AppConfig, ChatMessage, ChatProviderKind, ChatRole, DaemonCommand, DaemonEvent,
        FakeChatProvider, FakeOAuthProvider, FileConfigStore, FileSecretStore, FileSessionStore,
        InMemorySecretStore, OAuthCallbackPayload, OAuthProviderKind, OberonError, SavedSession,
        SecretStore, SessionId, SessionSummary,
    };
    use oberon4_provider_openai::{FakeTransport, OpenAiCompatibleClient, RetryPolicy};

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("oberon4-daemon-{name}-{nonce}"))
    }

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
            InMemoryAccountMetadataStore::default(),
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

        let mut events = Vec::new();
        app.handle_chat_stream(
            vec![ChatMessage {
                role: ChatRole::User,
                content: "hello".to_string(),
                tool_calls: Vec::new(),
            }],
            &mut |event| {
                events.push(event);
                Ok(())
            },
        )
        .unwrap();
        match events.last().unwrap() {
            DaemonEvent::ChatComplete { message } => assert_eq!(message.role, ChatRole::Assistant),
            other => panic!("unexpected event: {other:?}"),
        }

        let connect = app
            .handle_command(DaemonCommand::ConnectAccount {
                provider: OAuthProviderKind::OpenAi,
            })
            .unwrap()
            .event;
        let (flow_id, state) = match connect {
            DaemonEvent::OAuthStarted {
                provider,
                flow_id,
                url,
                redirect_uri,
            } => {
                assert_eq!(provider, OAuthProviderKind::OpenAi);
                assert!(url.contains("openai"));
                assert!(redirect_uri.contains("localhost:1455"));
                let state = url
                    .split("state=")
                    .nth(1)
                    .and_then(|fragment| fragment.split('&').next())
                    .expect("state should be present")
                    .to_string();
                (flow_id, state)
            }
            other => panic!("unexpected event: {other:?}"),
        };

        let completed = app
            .handle_command(DaemonCommand::CompleteOAuthConnect {
                flow_id,
                callback: OAuthCallbackPayload {
                    state,
                    code: "demo".to_string(),
                    return_to: None,
                },
            })
            .unwrap()
            .event;
        assert!(matches!(completed, DaemonEvent::Ack { .. }));

        let accounts = app
            .handle_command(DaemonCommand::ListAccounts)
            .unwrap()
            .event;
        match accounts {
            DaemonEvent::AccountsListed { accounts } => {
                assert_eq!(accounts.len(), 1);
                assert_eq!(accounts[0].id, "acct-demo");
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
            InMemoryAccountMetadataStore::default(),
        );
        let err = app
            .handle_chat_stream(
                vec![ChatMessage {
                    role: ChatRole::User,
                    content: "hello".to_string(),
                    tool_calls: Vec::new(),
                }],
                &mut |_| Ok(()),
            )
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
                InMemoryAccountMetadataStore::default(),
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
            InMemoryAccountMetadataStore::default(),
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
                tool_calls: Vec::new(),
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

        let deleted = app
            .handle_command(DaemonCommand::DeleteSession {
                session_id: "session-1".to_string(),
            })
            .unwrap()
            .event;
        match deleted {
            DaemonEvent::Ack { message } => assert_eq!(message, "deleted session session-1"),
            other => panic!("unexpected event: {other:?}"),
        }

        let missing = app.handle_command(DaemonCommand::LoadSession {
            session_id: "session-1".to_string(),
        });
        assert!(matches!(missing, Err(OberonError::NotFound(_))));
    }

    #[test]
    fn account_metadata_persists_across_daemon_restart() {
        let dir = temp_dir("accounts");
        let config_path = dir.join("config.toml");
        let session_dir = dir.join("sessions");
        let secret_path = dir.join("secrets.json");
        let account_path = dir.join("accounts.json");

        let mut app = DaemonApp::new(
            DaemonRuntime {
                config_store: FileConfigStore::new(&config_path),
                session_store: FileSessionStore::new(&session_dir),
                lifecycle: LifecycleState::Stopped,
            },
            FakeChatProvider::default(),
            FakeOAuthProvider::new(OAuthProviderKind::OpenAi),
            FakeOAuthProvider::new(OAuthProviderKind::Anthropic),
            AccountRegistry::default(),
            FileSecretStore::new(&secret_path),
            FileAccountMetadataStore::new(&account_path),
        );
        let _ = app.start().unwrap();
        let connect = app
            .handle_command(DaemonCommand::ConnectAccount {
                provider: OAuthProviderKind::OpenAi,
            })
            .unwrap()
            .event;
        let (flow_id, state) = match connect {
            DaemonEvent::OAuthStarted { flow_id, url, .. } => {
                let state = url
                    .split("state=")
                    .nth(1)
                    .and_then(|fragment| fragment.split('&').next())
                    .expect("state should be present")
                    .to_string();
                (flow_id, state)
            }
            other => panic!("unexpected event: {other:?}"),
        };
        let _ = app
            .handle_command(DaemonCommand::CompleteOAuthConnect {
                flow_id,
                callback: OAuthCallbackPayload {
                    state,
                    code: "demo".to_string(),
                    return_to: None,
                },
            })
            .unwrap();

        let store = FileAccountMetadataStore::new(&account_path);
        let registry = store.load_registry().unwrap();
        assert_eq!(registry.list().len(), 1);
        assert_eq!(
            registry.active_account_id(OAuthProviderKind::OpenAi),
            Some("acct-demo")
        );

        let reloaded = DaemonApp::new(
            DaemonRuntime {
                config_store: FileConfigStore::new(&config_path),
                session_store: FileSessionStore::new(&session_dir),
                lifecycle: LifecycleState::Stopped,
            },
            FakeChatProvider::default(),
            FakeOAuthProvider::new(OAuthProviderKind::OpenAi),
            FakeOAuthProvider::new(OAuthProviderKind::Anthropic),
            store.load_registry().unwrap(),
            FileSecretStore::new(&secret_path),
            store,
        );
        let status = reloaded.status().unwrap();
        assert_eq!(status.active_account.as_deref(), Some("acct-demo"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn chat_materializes_api_key_from_secret_store_when_env_missing() {
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
        }

        let mut secrets = InMemorySecretStore::default();
        secrets
            .set_secret("env.OPENAI_API_KEY", "hs_test_key")
            .unwrap();

        let transport = FakeTransport::new(vec![Ok(concat!(
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
            "data: [DONE]\n\n"
        )
        .to_string())]);
        let provider = OpenAiCompatibleClient::new(
            "http://gateway.example.test",
            Some("OPENAI_API_KEY".to_string()),
            RetryPolicy {
                timeout_ms: 5_000,
                max_retries: 0,
            },
            transport,
        );

        let mut app = DaemonApp::new(
            DaemonRuntime::default(),
            provider,
            FakeOAuthProvider::new(OAuthProviderKind::OpenAi),
            FakeOAuthProvider::new(OAuthProviderKind::Anthropic),
            AccountRegistry::default(),
            secrets,
            InMemoryAccountMetadataStore::default(),
        );
        let _ = app.start().unwrap();

        let _ = app
            .handle_command(DaemonCommand::SaveConfig {
                config: AppConfig {
                    endpoint: oberon4_core::EndpointConfig {
                        base_url: "http://gateway.example.test".to_string(),
                        model: "gpt-5.1-codex-mini".to_string(),
                        api_key_env: Some("OPENAI_API_KEY".to_string()),
                    },
                    runtime: oberon4_core::RuntimeConfig {
                        request_timeout_ms: 5_000,
                        stream_idle_timeout_ms: 5_000,
                        max_retries: 0,
                        log_level: "info".to_string(),
                        stream_responses: true,
                    },
                },
            })
            .unwrap();

        let mut events = Vec::new();
        app.handle_chat_stream(
            vec![ChatMessage {
                role: ChatRole::User,
                content: "hello".to_string(),
                tool_calls: Vec::new(),
            }],
            &mut |event| {
                events.push(event);
                Ok(())
            },
        )
        .unwrap();

        match events.last().unwrap() {
            DaemonEvent::ChatComplete { message } => assert_eq!(message.content, "ok"),
            other => panic!("unexpected event: {other:?}"),
        }

        assert_eq!(
            std::env::var("OPENAI_API_KEY").ok().as_deref(),
            Some("hs_test_key")
        );
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
        }
    }
}
