use std::{
    io,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use oberon4_auth::AccountRegistry;
use oberon4_core::{
    AppConfig, ChatMessage, ChatProvider, ChatRole, InMemoryConfigStore, InMemorySessionStore,
    OAuthProviderKind, OberonError, SavedSession, SessionId, SessionStore, SessionSummary,
};
use oberon4_daemon::{DaemonIpcClient, DaemonRuntime, LifecycleState};
use oberon4_provider_openai::{FakeTransport, OpenAiCompatibleClient, RetryPolicy};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    style::{Modifier, Style},
    text::{Line, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiLine {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiShellState {
    input: String,
    status: String,
    should_quit: bool,
    transcript: Vec<UiLine>,
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

impl Default for TuiShellState {
    fn default() -> Self {
        Self {
            input: String::new(),
            status: "Connected to daemon".to_string(),
            should_quit: false,
            transcript: Vec::new(),
        }
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

impl TuiShellState {
    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn transcript(&self) -> &[UiLine] {
        &self.transcript
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn push_input_char(&mut self, ch: char) {
        self.input.push(ch);
    }

    pub fn backspace(&mut self) {
        let _ = self.input.pop();
    }

    pub fn submit(&mut self, app: &mut DaemonBackedTuiApp) -> Result<(), OberonError> {
        let input = std::mem::take(&mut self.input);
        let trimmed = input.trim();
        if trimmed.is_empty() {
            self.status = "Input was empty".to_string();
            return Ok(());
        }

        if !trimmed.starts_with('/') {
            self.transcript.push(UiLine {
                role: "user".to_string(),
                content: trimmed.to_string(),
            });
        }

        let response = app.handle_input(trimmed)?;
        self.status = response.output.clone();
        self.should_quit = response.should_quit;

        if trimmed.starts_with('/') {
            self.transcript.push(UiLine {
                role: "system".to_string(),
                content: format!("command: {trimmed}"),
            });
            if !response.output.is_empty() {
                self.transcript.push(UiLine {
                    role: "status".to_string(),
                    content: response.output,
                });
            }
            return Ok(());
        }

        if let Some(last) = app.transcript().last() {
            self.transcript.push(UiLine {
                role: role_label(last.role).to_string(),
                content: last.content.clone(),
            });
        }

        Ok(())
    }
}

pub fn run_terminal_ui() -> Result<(), OberonError> {
    let mut app = DaemonBackedTuiApp::default();
    let mut shell = TuiShellState::default();
    let mut terminal = init_terminal()?;

    let result = run_terminal_loop(&mut terminal, &mut app, &mut shell);
    restore_terminal(&mut terminal)?;
    result
}

fn run_terminal_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut DaemonBackedTuiApp,
    shell: &mut TuiShellState,
) -> Result<(), OberonError> {
    loop {
        terminal
            .draw(|frame| render_terminal(frame, shell))
            .map_err(|err| OberonError::Storage(format!("tui draw failed: {err}")))?;

        if shell.should_quit() {
            return Ok(());
        }

        if !event::poll(Duration::from_millis(100))
            .map_err(|err| OberonError::Storage(format!("tui event poll failed: {err}")))?
        {
            continue;
        }

        let Event::Key(key) = event::read()
            .map_err(|err| OberonError::Storage(format!("tui event read failed: {err}")))?
        else {
            continue;
        };

        if key.kind != KeyEventKind::Press {
            continue;
        }

        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                shell.should_quit = true;
                shell.status = "quitting".to_string();
            }
            KeyCode::Char(ch) => shell.push_input_char(ch),
            KeyCode::Backspace => shell.backspace(),
            KeyCode::Enter => shell.submit(app)?,
            KeyCode::Esc => {
                shell.should_quit = true;
                shell.status = "quitting".to_string();
            }
            _ => {}
        }
    }
}

fn render_terminal(frame: &mut Frame, shell: &TuiShellState) {
    let area = frame.size();
    let [header, body, footer] = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(3),
        ratatui::layout::Constraint::Min(5),
        ratatui::layout::Constraint::Length(3),
    ])
    .areas(area);

    let header_text = Paragraph::new(Text::from(vec![
        Line::styled("Oberon4 TUI", Style::default().add_modifier(Modifier::BOLD)),
        Line::raw(shell.status()),
    ]))
    .block(Block::default().borders(Borders::ALL).title("Status"));

    let transcript_lines: Vec<Line<'static>> = if shell.transcript().is_empty() {
        vec![Line::raw(
            "No messages yet. Type text to chat or /help for commands.",
        )]
    } else {
        shell
            .transcript()
            .iter()
            .map(|line| Line::raw(format!("[{}] {}", line.role, line.content)))
            .collect()
    };

    let transcript = Paragraph::new(Text::from(transcript_lines))
        .block(Block::default().borders(Borders::ALL).title("Transcript"))
        .wrap(Wrap { trim: false });

    let input = Paragraph::new(shell.input())
        .block(Block::default().borders(Borders::ALL).title("Input"))
        .wrap(Wrap { trim: false });

    frame.render_widget(header_text, header);
    frame.render_widget(transcript, body);
    frame.render_widget(input, footer);
    frame.set_cursor(
        footer.x.saturating_add(shell.input().len() as u16 + 1),
        footer.y.saturating_add(1),
    );
}

fn init_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>, OberonError> {
    enable_raw_mode()
        .map_err(|err| OberonError::Storage(format!("raw mode enable failed: {err}")))?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)
        .map_err(|err| OberonError::Storage(format!("enter alt screen failed: {err}")))?;
    Terminal::new(CrosstermBackend::new(stdout))
        .map_err(|err| OberonError::Storage(format!("terminal init failed: {err}")))
}

fn restore_terminal(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<(), OberonError> {
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .map_err(|err| OberonError::Storage(format!("leave alt screen failed: {err}")))?;
    disable_raw_mode()
        .map_err(|err| OberonError::Storage(format!("raw mode disable failed: {err}")))?;
    Ok(())
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

fn role_label(role: ChatRole) -> &'static str {
    match role {
        ChatRole::System => "system",
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
    }
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

    use super::{
        DaemonBackedTuiApp, TuiApp, TuiCommand, TuiShellState, parse_tui_command, status_line,
    };
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
        let socket_path =
            std::env::temp_dir().join(format!("oberon4-tui-test-{}.sock", std::process::id()));
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

    #[test]
    fn shell_state_edits_input_and_tracks_quit() {
        let mut shell = TuiShellState::default();
        shell.push_input_char('h');
        shell.push_input_char('i');
        shell.backspace();
        assert_eq!(shell.input(), "h");
        assert!(!shell.should_quit());
    }

    #[test]
    fn shell_submit_records_command_output() {
        let socket_path = std::env::temp_dir().join(format!(
            "oberon4-tui-shell-test-{}.sock",
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
        let mut shell = TuiShellState::default();
        for ch in "/help".chars() {
            shell.push_input_char(ch);
        }
        shell.submit(&mut app).unwrap();

        assert!(shell.status().contains("/help"));
        assert_eq!(shell.transcript()[0].role, "system");

        let client = DaemonIpcClient::new(&socket_path);
        let _ = client.send(&oberon4_core::DaemonCommand::Shutdown).unwrap();
        handle.join().unwrap().unwrap();
    }
}
