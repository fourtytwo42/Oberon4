use std::{
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use oberon4_auth::AccountRegistry;
use oberon4_core::{
    AppConfig, ChatMessage, ChatProvider, ChatRole, InMemoryConfigStore, InMemorySessionStore,
    OAuthProviderKind, OberonError, SavedSession, SessionId, SessionStore, SessionSummary,
};
use oberon4_daemon::{DaemonIpcClient, DaemonRuntime, LifecycleState};
use oberon4_provider_openai::{FakeTransport, OpenAiCompatibleClient, RetryPolicy};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiCommand {
    Help,
    Quit,
    Status,
    Config,
    Model(String),
    Endpoint(String),
    System(String),
    Clear,
    Save(String),
    Load(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiResponse {
    pub output: String,
    pub should_quit: bool,
}

pub struct TuiApp {
    runtime: DaemonRuntime<InMemoryConfigStore, InMemorySessionStore>,
    provider_client: OpenAiCompatibleClient<FakeTransport>,
    transcript: Vec<ChatMessage>,
    system_prompt: Option<String>,
    accounts: AccountRegistry,
}

#[derive(Debug, Clone)]
pub struct DaemonBackedTuiApp {
    client: DaemonIpcClient,
    transcript: Vec<ChatMessage>,
    system_prompt: Option<String>,
}

impl Default for TuiApp {
    fn default() -> Self {
        Self {
            runtime: DaemonRuntime::default(),
            provider_client: default_provider_client(),
            transcript: Vec::new(),
            system_prompt: None,
            accounts: AccountRegistry::default(),
        }
    }
}

impl Default for DaemonBackedTuiApp {
    fn default() -> Self {
        Self::new(DaemonIpcClient::new(oberon4_daemon::default_socket_path()))
    }
}

impl TuiApp {
    pub fn handle_input(&mut self, input: &str) -> Result<TuiResponse, OberonError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Ok(TuiResponse {
                output: String::new(),
                should_quit: false,
            });
        }

        if trimmed.starts_with('/') {
            self.handle_command(parse_tui_command(trimmed)?)
        } else {
            self.send_prompt(trimmed)
        }
    }

    pub fn handle_command(&mut self, command: TuiCommand) -> Result<TuiResponse, OberonError> {
        match command {
            TuiCommand::Help => Ok(TuiResponse {
                output: "/help /quit /status /config /model <name> /endpoint <url> /system <text> /clear /save <name> /load <name>".to_string(),
                should_quit: false,
            }),
            TuiCommand::Quit => Ok(TuiResponse {
                output: "quitting".to_string(),
                should_quit: true,
            }),
            TuiCommand::Status => {
                let status = self.runtime.status()?;
                Ok(TuiResponse {
                    output: format!(
                        "running={} endpoint={} model={} active_account={}",
                        status.running,
                        status.endpoint_base_url,
                        status.model,
                        self.accounts
                            .active_account_id(OAuthProviderKind::OpenAi)
                            .unwrap_or("none")
                    ),
                    should_quit: false,
                })
            }
            TuiCommand::Config => Ok(TuiResponse {
                output: redacted_config_json(&self.runtime.load_config()?)?,
                should_quit: false,
            }),
            TuiCommand::Model(model) => {
                let mut config = self.runtime.load_config()?;
                config.endpoint.model = model.clone();
                self.runtime.save_config(&config)?;
                Ok(TuiResponse {
                    output: format!("model set to {model}"),
                    should_quit: false,
                })
            }
            TuiCommand::Endpoint(url) => {
                let mut config = self.runtime.load_config()?;
                config.endpoint.base_url = url.clone();
                self.runtime.save_config(&config)?;
                Ok(TuiResponse {
                    output: format!("endpoint set to {url}"),
                    should_quit: false,
                })
            }
            TuiCommand::System(prompt) => {
                self.system_prompt = Some(prompt.clone());
                Ok(TuiResponse {
                    output: format!("system prompt set to {}", prompt),
                    should_quit: false,
                })
            }
            TuiCommand::Clear => {
                self.transcript.clear();
                Ok(TuiResponse {
                    output: "transcript cleared".to_string(),
                    should_quit: false,
                })
            }
            TuiCommand::Save(name) => {
                let session = SavedSession {
                    summary: SessionSummary {
                        id: SessionId::new(name.clone()),
                        title: name.clone(),
                        provider: oberon4_core::ChatProviderKind::OpenAiCompatible,
                        model: self.runtime.load_config()?.endpoint.model.clone(),
                    },
                    messages: self.transcript.clone(),
                };
                self.runtime.session_store.save_session(session)?;
                Ok(TuiResponse {
                    output: format!("saved session {name}"),
                    should_quit: false,
                })
            }
            TuiCommand::Load(name) => {
                let session = self.runtime.session_store.load_session(&name)?;
                self.transcript = session.messages;
                Ok(TuiResponse {
                    output: format!("loaded session {name}"),
                    should_quit: false,
                })
            }
        }
    }

    pub fn transcript(&self) -> &[ChatMessage] {
        &self.transcript
    }

    pub fn system_prompt(&self) -> Option<&str> {
        self.system_prompt.as_deref()
    }

    fn send_prompt(&mut self, prompt: &str) -> Result<TuiResponse, OberonError> {
        if self.runtime.lifecycle_state() == LifecycleState::Stopped {
            let _ = self.runtime.start()?;
        }
        if let Some(prompt_text) = self.system_prompt.clone() {
            if self.transcript.is_empty() {
                self.transcript.push(ChatMessage {
                    role: ChatRole::System,
                    content: prompt_text,
                });
            }
        }
        self.transcript.push(ChatMessage {
            role: ChatRole::User,
            content: prompt.to_string(),
        });
        let config = self.runtime.load_config()?;
        let reply = self.provider_client.send_message(
            &config.endpoint.base_url,
            &config.endpoint.model,
            &self.transcript,
        )?;
        self.transcript.push(reply.clone());
        Ok(TuiResponse {
            output: reply.content,
            should_quit: false,
        })
    }
}

impl DaemonBackedTuiApp {
    pub fn new(client: DaemonIpcClient) -> Self {
        Self {
            client,
            transcript: Vec::new(),
            system_prompt: None,
        }
    }

    pub fn handle_input(&mut self, input: &str) -> Result<TuiResponse, OberonError> {
        self.ensure_daemon_running()?;
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Ok(TuiResponse {
                output: String::new(),
                should_quit: false,
            });
        }

        if trimmed.starts_with('/') {
            self.handle_command(parse_tui_command(trimmed)?)
        } else {
            self.send_prompt(trimmed)
        }
    }

    pub fn handle_command(&mut self, command: TuiCommand) -> Result<TuiResponse, OberonError> {
        if !matches!(command, TuiCommand::Help | TuiCommand::Quit) {
            self.ensure_daemon_running()?;
        }
        match command {
            TuiCommand::Help => Ok(TuiResponse {
                output: "/help /quit /status /config /model <name> /endpoint <url> /system <text> /clear /save <name> /load <name>".to_string(),
                should_quit: false,
            }),
            TuiCommand::Quit => Ok(TuiResponse {
                output: "quitting".to_string(),
                should_quit: true,
            }),
            TuiCommand::Status => match self.client.send(&oberon4_core::DaemonCommand::GetStatus)? {
                oberon4_core::DaemonEvent::Status { status } => Ok(TuiResponse {
                    output: format!(
                        "running={} endpoint={} model={} active_account={}",
                        status.running,
                        status.endpoint_base_url,
                        status.model,
                        status.active_account.as_deref().unwrap_or("none")
                    ),
                    should_quit: false,
                }),
                event => protocol_error(event),
            },
            TuiCommand::Config => match self.client.send(&oberon4_core::DaemonCommand::LoadConfig)? {
                oberon4_core::DaemonEvent::ConfigLoaded { config } => Ok(TuiResponse {
                    output: redacted_config_json(&config)?,
                    should_quit: false,
                }),
                event => protocol_error(event),
            },
            TuiCommand::Model(model) => {
                let mut config = self.load_config()?;
                config.endpoint.model = model.clone();
                self.save_config(config)?;
                Ok(TuiResponse {
                    output: format!("model set to {model}"),
                    should_quit: false,
                })
            }
            TuiCommand::Endpoint(url) => {
                let mut config = self.load_config()?;
                config.endpoint.base_url = url.clone();
                self.save_config(config)?;
                Ok(TuiResponse {
                    output: format!("endpoint set to {url}"),
                    should_quit: false,
                })
            }
            TuiCommand::System(prompt) => {
                self.system_prompt = Some(prompt.clone());
                Ok(TuiResponse {
                    output: format!("system prompt set to {}", prompt),
                    should_quit: false,
                })
            }
            TuiCommand::Clear => {
                self.transcript.clear();
                Ok(TuiResponse {
                    output: "transcript cleared".to_string(),
                    should_quit: false,
                })
            }
            TuiCommand::Save(name) => {
                let session = SavedSession {
                    summary: SessionSummary {
                        id: SessionId::new(name.clone()),
                        title: name.clone(),
                        provider: oberon4_core::ChatProviderKind::OpenAiCompatible,
                        model: self.load_config()?.endpoint.model,
                    },
                    messages: self.transcript.clone(),
                };
                match self
                    .client
                    .send(&oberon4_core::DaemonCommand::SaveSession { session })?
                {
                    oberon4_core::DaemonEvent::SessionLoaded { .. } => Ok(TuiResponse {
                        output: format!("saved session {name}"),
                        should_quit: false,
                    }),
                    event => protocol_error(event),
                }
            }
            TuiCommand::Load(name) => match self.client.send(
                &oberon4_core::DaemonCommand::LoadSession {
                    session_id: name.clone(),
                },
            )? {
                oberon4_core::DaemonEvent::SessionLoaded { session } => {
                    self.transcript = session.messages;
                    Ok(TuiResponse {
                        output: format!("loaded session {name}"),
                        should_quit: false,
                    })
                }
                event => protocol_error(event),
            },
        }
    }

    pub fn transcript(&self) -> &[ChatMessage] {
        &self.transcript
    }

    pub fn system_prompt(&self) -> Option<&str> {
        self.system_prompt.as_deref()
    }

    fn send_prompt(&mut self, prompt: &str) -> Result<TuiResponse, OberonError> {
        if let Some(prompt_text) = self.system_prompt.clone() {
            if self.transcript.is_empty() {
                self.transcript.push(ChatMessage {
                    role: ChatRole::System,
                    content: prompt_text,
                });
            }
        }
        self.transcript.push(ChatMessage {
            role: ChatRole::User,
            content: prompt.to_string(),
        });

        match self.client.send(&oberon4_core::DaemonCommand::SendChat {
            session_id: None,
            messages: self.transcript.clone(),
        })? {
            oberon4_core::DaemonEvent::ChatComplete { message } => {
                self.transcript.push(message.clone());
                Ok(TuiResponse {
                    output: message.content,
                    should_quit: false,
                })
            }
            event => protocol_error(event),
        }
    }

    fn load_config(&self) -> Result<AppConfig, OberonError> {
        match self.client.send(&oberon4_core::DaemonCommand::LoadConfig)? {
            oberon4_core::DaemonEvent::ConfigLoaded { config } => Ok(config),
            event => Err(protocol_error_inner(event)),
        }
    }

    fn save_config(&self, config: AppConfig) -> Result<(), OberonError> {
        match self
            .client
            .send(&oberon4_core::DaemonCommand::SaveConfig { config })?
        {
            oberon4_core::DaemonEvent::ConfigLoaded { .. } => Ok(()),
            event => Err(protocol_error_inner(event)),
        }
    }

    fn ensure_daemon_running(&self) -> Result<(), OberonError> {
        if self
            .client
            .send(&oberon4_core::DaemonCommand::GetStatus)
            .is_ok()
        {
            return Ok(());
        }

        spawn_daemon_process(self.client.socket_path().to_path_buf())?;

        for _ in 0..40 {
            if self
                .client
                .send(&oberon4_core::DaemonCommand::GetStatus)
                .is_ok()
            {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(50));
        }

        Err(OberonError::Storage(
            "daemon failed to become ready after spawn".to_string(),
        ))
    }
}

pub fn parse_tui_command(input: &str) -> Result<TuiCommand, OberonError> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return Err(OberonError::Validation(
            "tui commands must start with '/'".to_string(),
        ));
    }

    let mut parts = trimmed.splitn(2, ' ');
    let command = parts.next().unwrap_or_default();
    let rest = parts.next().unwrap_or("").trim();

    match command {
        "/help" => Ok(TuiCommand::Help),
        "/quit" => Ok(TuiCommand::Quit),
        "/status" => Ok(TuiCommand::Status),
        "/config" => Ok(TuiCommand::Config),
        "/model" => require_argument(rest, "/model").map(TuiCommand::Model),
        "/endpoint" => require_argument(rest, "/endpoint").map(TuiCommand::Endpoint),
        "/system" => require_argument(rest, "/system").map(TuiCommand::System),
        "/clear" => Ok(TuiCommand::Clear),
        "/save" => require_argument(rest, "/save").map(TuiCommand::Save),
        "/load" => require_argument(rest, "/load").map(TuiCommand::Load),
        _ => Err(OberonError::Validation(
            "unsupported TUI command. Use /help".to_string(),
        )),
    }
}

pub fn status_line(config: &AppConfig) -> String {
    format!(
        "connected to {} ({})",
        config.endpoint.base_url, config.endpoint.model
    )
}

fn require_argument(value: &str, command: &str) -> Result<String, OberonError> {
    if value.is_empty() {
        Err(OberonError::Validation(format!(
            "{command} requires an argument. Use /help"
        )))
    } else {
        Ok(value.to_string())
    }
}

fn redacted_config_json(config: &AppConfig) -> Result<String, OberonError> {
    let mut value =
        serde_json::to_value(config).map_err(|err| OberonError::Storage(err.to_string()))?;
    if let Some(object) = value
        .get_mut("endpoint")
        .and_then(|endpoint| endpoint.as_object_mut())
    {
        if let Some(api_key_env) = object.get_mut("api_key_env") {
            *api_key_env = serde_json::Value::String("<redacted>".to_string());
        }
    }
    serde_json::to_string_pretty(&value).map_err(|err| OberonError::Storage(err.to_string()))
}

fn protocol_error(event: oberon4_core::DaemonEvent) -> Result<TuiResponse, OberonError> {
    Err(protocol_error_inner(event))
}

fn protocol_error_inner(event: oberon4_core::DaemonEvent) -> OberonError {
    match event {
        oberon4_core::DaemonEvent::Error { message } => OberonError::Validation(message),
        other => OberonError::Storage(format!("unexpected daemon response: {other:?}")),
    }
}

fn spawn_daemon_process(socket_path: PathBuf) -> Result<(), OberonError> {
    let daemon_binary = daemon_binary_path()?;
    Command::new(daemon_binary)
        .env("OBERON4_SOCKET_PATH", socket_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| OberonError::Storage(format!("failed to spawn daemon: {err}")))?;
    Ok(())
}

fn daemon_binary_path() -> Result<PathBuf, OberonError> {
    let current = std::env::current_exe()
        .map_err(|err| OberonError::Storage(format!("current executable lookup failed: {err}")))?;
    let direct = current
        .parent()
        .map(|dir| dir.join("oberon4-daemon"))
        .ok_or_else(|| {
            OberonError::Storage("current executable has no parent directory".to_string())
        })?;
    if direct.exists() {
        return Ok(direct);
    }
    let sibling = current
        .parent()
        .and_then(|dir| dir.parent())
        .map(|dir| dir.join("oberon4-daemon"))
        .ok_or_else(|| OberonError::Storage("daemon binary path resolution failed".to_string()))?;
    if sibling.exists() {
        return Ok(sibling);
    }
    Err(OberonError::Storage(
        "oberon4-daemon binary not found next to current executable".to_string(),
    ))
}

fn default_provider_client() -> OpenAiCompatibleClient<FakeTransport> {
    OpenAiCompatibleClient::new(
        "https://api.openai.com",
        RetryPolicy {
            timeout_ms: 30_000,
            max_retries: 0,
        },
        FakeTransport::new(vec![
            Ok(ok_response("assistant reply")),
            Ok(ok_response("second reply")),
            Ok(ok_response("third reply")),
        ]),
    )
}

fn ok_response(content: &str) -> String {
    format!(r#"{{"choices":[{{"message":{{"role":"assistant","content":"{content}"}}}}]}}"#)
}

#[cfg(test)]
mod tests {
    use std::{thread, time::Duration};

    use super::{DaemonBackedTuiApp, TuiApp, TuiCommand, parse_tui_command, status_line};
    use oberon4_auth::AccountRegistry;
    use oberon4_core::{
        AppConfig, ChatRole, FakeChatProvider, FakeOAuthProvider, InMemorySecretStore,
        OAuthProviderKind,
    };
    use oberon4_daemon::{DaemonApp, DaemonIpcClient, DaemonRuntime, serve_unix_socket};

    #[test]
    fn status_line_includes_endpoint_and_model() {
        let line = status_line(&AppConfig::default());
        assert!(line.contains("https://api.openai.com"));
        assert!(line.contains("gpt-4.1-mini"));
    }

    #[test]
    fn parses_supported_commands() {
        assert_eq!(parse_tui_command("/help").unwrap(), TuiCommand::Help);
        assert_eq!(
            parse_tui_command("/model o4-mini").unwrap(),
            TuiCommand::Model("o4-mini".to_string())
        );
        assert_eq!(
            parse_tui_command("/save demo").unwrap(),
            TuiCommand::Save("demo".to_string())
        );
    }

    #[test]
    fn rejects_invalid_commands_with_help_text() {
        let err = parse_tui_command("/bogus").unwrap_err();
        assert_eq!(
            err.to_string(),
            "validation failed: unsupported TUI command. Use /help"
        );
    }

    #[test]
    fn config_and_status_commands_return_redacted_state() {
        let mut app = TuiApp::default();
        let status = app.handle_command(TuiCommand::Status).unwrap();
        let config = app.handle_command(TuiCommand::Config).unwrap();

        assert!(status.output.contains("running=false"));
        assert!(config.output.contains("<redacted>"));
    }

    #[test]
    fn model_endpoint_and_system_commands_update_state() {
        let mut app = TuiApp::default();
        let _ = app
            .handle_command(TuiCommand::Model("o4-mini".to_string()))
            .unwrap();
        let _ = app
            .handle_command(TuiCommand::Endpoint(
                "https://gateway.example.test".to_string(),
            ))
            .unwrap();
        let _ = app
            .handle_command(TuiCommand::System("be concise".to_string()))
            .unwrap();
        let config = app.handle_command(TuiCommand::Config).unwrap();

        assert!(config.output.contains("\"model\": \"o4-mini\""));
        assert!(config.output.contains("gateway.example.test"));
        assert_eq!(app.system_prompt(), Some("be concise"));
    }

    #[test]
    fn clear_save_and_load_manage_transcript_state() {
        let mut app = TuiApp::default();
        let _ = app.handle_input("hello").unwrap();
        assert!(!app.transcript().is_empty());

        let save = app
            .handle_command(TuiCommand::Save("demo".to_string()))
            .unwrap();
        let clear = app.handle_command(TuiCommand::Clear).unwrap();
        let load = app
            .handle_command(TuiCommand::Load("demo".to_string()))
            .unwrap();

        assert_eq!(save.output, "saved session demo");
        assert_eq!(clear.output, "transcript cleared");
        assert_eq!(load.output, "loaded session demo");
        assert!(!app.transcript().is_empty());
    }

    #[test]
    fn plain_prompt_runs_chat_loop_and_records_transcript() {
        let mut app = TuiApp::default();
        let response = app.handle_input("hello there").unwrap();

        assert_eq!(response.output, "assistant reply");
        assert_eq!(app.transcript().len(), 2);
        assert_eq!(app.transcript()[0].role, ChatRole::User);
        assert_eq!(app.transcript()[1].role, ChatRole::Assistant);
    }

    #[test]
    fn system_prompt_is_prefixed_on_first_chat_turn() {
        let mut app = TuiApp::default();
        let _ = app
            .handle_command(TuiCommand::System("be concise".to_string()))
            .unwrap();
        let _ = app.handle_input("hello there").unwrap();

        assert_eq!(app.transcript().len(), 3);
        assert_eq!(app.transcript()[0].role, ChatRole::System);
        assert_eq!(app.transcript()[0].content, "be concise");
    }

    #[test]
    fn quit_command_sets_quit_flag() {
        let mut app = TuiApp::default();
        let response = app.handle_command(TuiCommand::Quit).unwrap();
        assert!(response.should_quit);
    }

    #[test]
    fn daemon_backed_tui_uses_socket_server_and_persists_session() {
        let socket_path = std::env::temp_dir().join(format!(
            "oberon4-tui-test-{}.sock",
            std::process::id()
        ));
        let thread_socket = socket_path.clone();

        let handle = thread::spawn(move || {
            let app = DaemonApp::new(
                DaemonRuntime::default(),
                FakeChatProvider {
                    reply: "daemon reply".to_string(),
                },
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

        let mut app = DaemonBackedTuiApp::new(DaemonIpcClient::new(&socket_path));
        let config = app.handle_command(TuiCommand::Config).unwrap();
        let chat = app.handle_input("hello over daemon").unwrap();
        let save = app
            .handle_command(TuiCommand::Save("daemon-demo".to_string()))
            .unwrap();
        let clear = app.handle_command(TuiCommand::Clear).unwrap();
        let load = app
            .handle_command(TuiCommand::Load("daemon-demo".to_string()))
            .unwrap();

        assert!(config.output.contains("<redacted>"));
        assert_eq!(chat.output, "daemon reply");
        assert_eq!(save.output, "saved session daemon-demo");
        assert_eq!(clear.output, "transcript cleared");
        assert_eq!(load.output, "loaded session daemon-demo");
        assert_eq!(app.transcript().len(), 2);

        let client = DaemonIpcClient::new(&socket_path);
        let _ = client.send(&oberon4_core::DaemonCommand::Shutdown).unwrap();
        handle.join().unwrap().unwrap();
    }
}
