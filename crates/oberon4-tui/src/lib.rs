use std::{
    io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
    time::{Duration, Instant},
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

struct PendingSubmission {
    receiver: Receiver<Result<(DaemonBackedTuiApp, TuiShellState), (TuiShellState, OberonError)>>,
    started_at: Instant,
    timeout: Duration,
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
                self.provider_client.apply_runtime_config(&config)?;
                self.runtime.save_config(&config)?;
                Ok(TuiResponse {
                    output: format!("model set to {model}"),
                    should_quit: false,
                })
            }
            TuiCommand::Endpoint(url) => {
                let mut config = self.runtime.load_config()?;
                config.endpoint.base_url = url.clone();
                self.provider_client.apply_runtime_config(&config)?;
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

        if !trimmed.starts_with('/') && !has_trailing_user_line(&self.transcript, trimmed) {
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
    let mut pending_submission: Option<PendingSubmission> = None;

    loop {
        handle_pending_timeout(&mut pending_submission, shell);

        if let Some(pending) = pending_submission.as_ref() {
            match pending.receiver.try_recv() {
                Ok(result) => {
                    pending_submission = None;
                    match result {
                        Ok((updated_app, updated_shell)) => {
                            let preserved_input = std::mem::take(&mut shell.input);
                            *app = updated_app;
                            *shell = updated_shell;
                            shell.input = preserved_input;
                        }
                        Err((failed_shell, err)) => {
                            let preserved_input = std::mem::take(&mut shell.input);
                            *shell = failed_shell;
                            shell.input = preserved_input;
                            shell.status = err.to_string();
                            shell.transcript.push(UiLine {
                                role: "error".to_string(),
                                content: err.to_string(),
                            });
                        }
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    pending_submission = None;
                    shell.status = "request worker disconnected".to_string();
                }
            }
        }

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
            KeyCode::Enter => {
                if pending_submission.is_some() {
                    if !handle_pending_local_command(shell)? {
                        shell.status =
                            "request in flight; wait, or press Esc/Ctrl+C to quit".to_string();
                    }
                } else if let Some(pending) = start_async_submit(app, shell)? {
                    pending_submission = Some(pending);
                }
            }
            KeyCode::Esc => {
                shell.should_quit = true;
                shell.status = "quitting".to_string();
            }
            _ => {}
        }
    }
}

fn handle_pending_local_command(shell: &mut TuiShellState) -> Result<bool, OberonError> {
    let trimmed = shell.input.trim().to_string();
    if !trimmed.starts_with('/') {
        return Ok(false);
    }

    let command = parse_tui_command(&trimmed)?;
    let response = match command {
        TuiCommand::Help => TuiResponse {
            output: "/help /quit /status /config /model <name> /endpoint <url> /system <text> /clear /save <name> /load <name>".to_string(),
            should_quit: false,
        },
        TuiCommand::Status => TuiResponse {
            output: "request in flight".to_string(),
            should_quit: false,
        },
        TuiCommand::Quit => TuiResponse {
            output: "quitting".to_string(),
            should_quit: true,
        },
        _ => return Ok(false),
    };

    shell.input.clear();
    shell.status = response.output.clone();
    shell.should_quit = response.should_quit;
    shell.transcript.push(UiLine {
        role: "system".to_string(),
        content: format!("command: {trimmed}"),
    });
    if !response.output.is_empty() {
        shell.transcript.push(UiLine {
            role: "status".to_string(),
            content: response.output,
        });
    }
    Ok(true)
}

fn start_async_submit(
    app: &DaemonBackedTuiApp,
    shell: &mut TuiShellState,
) -> Result<Option<PendingSubmission>, OberonError> {
    let trimmed = shell.input.trim().to_string();
    if trimmed.is_empty() {
        shell.input.clear();
        shell.status = "Input was empty".to_string();
        return Ok(None);
    }

    let mut app_clone = app.clone();
    let mut shell_clone = shell.clone();
    shell_clone.input = std::mem::take(&mut shell.input);
    let timeout = app
        .load_config()
        .map(|config| Duration::from_millis(config.runtime.request_timeout_ms))
        .unwrap_or_else(|_| Duration::from_secs(30));
    shell.status = if trimmed.starts_with('/') {
        format!("running command: {trimmed}")
    } else {
        shell.transcript.push(UiLine {
            role: "user".to_string(),
            content: trimmed.clone(),
        });
        shell.transcript.push(UiLine {
            role: "status".to_string(),
            content: "waiting on provider response...".to_string(),
        });
        "waiting on provider response...".to_string()
    };

    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = match shell_clone.submit(&mut app_clone) {
            Ok(()) => Ok((app_clone, shell_clone)),
            Err(err) => Err((shell_clone, err)),
        };
        let _ = sender.send(result);
    });

    Ok(Some(PendingSubmission {
        receiver,
        started_at: Instant::now(),
        timeout,
    }))
}

fn has_trailing_user_line(transcript: &[UiLine], content: &str) -> bool {
    transcript
        .last()
        .map(|line| line.role == "user" && line.content == content)
        .unwrap_or(false)
}

fn handle_pending_timeout(
    pending_submission: &mut Option<PendingSubmission>,
    shell: &mut TuiShellState,
) {
    let should_timeout = pending_submission
        .as_ref()
        .map(|pending| pending.started_at.elapsed() > pending.timeout.saturating_add(Duration::from_millis(500)))
        .unwrap_or(false);

    if !should_timeout {
        return;
    }

    *pending_submission = None;
    shell.status = "request timed out".to_string();
    shell.transcript.push(UiLine {
        role: "error".to_string(),
        content: "request timed out".to_string(),
    });
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
    let mut command = daemon_launch_command()?;
    command
        .env("OBERON4_SOCKET_PATH", socket_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| OberonError::Storage(format!("failed to spawn daemon: {err}")))?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BinaryLaunchTarget {
    Direct(PathBuf),
    CargoRun {
        cargo_bin: PathBuf,
        manifest_path: PathBuf,
        package: String,
    },
}

fn daemon_launch_command() -> Result<Command, OberonError> {
    Ok(match resolve_daemon_launch_target()? {
        BinaryLaunchTarget::Direct(path) => Command::new(path),
        BinaryLaunchTarget::CargoRun {
            cargo_bin,
            manifest_path,
            package,
        } => {
            let mut command = Command::new(cargo_bin);
            command
                .arg("run")
                .arg("-p")
                .arg(package)
                .arg("--manifest-path")
                .arg(manifest_path)
                .arg("--quiet");
            command
        }
    })
}

fn resolve_daemon_launch_target() -> Result<BinaryLaunchTarget, OberonError> {
    if let Some(path) = std::env::var_os("OBERON4_DAEMON_BIN") {
        return Ok(BinaryLaunchTarget::Direct(PathBuf::from(path)));
    }

    let current = std::env::current_exe()
        .map_err(|err| OberonError::Storage(format!("current executable lookup failed: {err}")))?;

    resolve_binary_launch_target(&current, "oberon4-daemon").ok_or_else(|| {
        OberonError::Storage(
            "oberon4-daemon binary not found and no cargo workspace fallback was available"
                .to_string(),
        )
    })
}

fn resolve_binary_launch_target(current_exe: &Path, binary_name: &str) -> Option<BinaryLaunchTarget> {
    for candidate in binary_path_candidates(current_exe, binary_name) {
        if candidate.exists() {
            return Some(BinaryLaunchTarget::Direct(candidate));
        }
    }

    workspace_manifest_path(current_exe).map(|manifest_path| BinaryLaunchTarget::CargoRun {
        cargo_bin: std::env::var_os("CARGO")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("cargo")),
        manifest_path,
        package: binary_name.to_string(),
    })
}

fn binary_path_candidates(current_exe: &Path, binary_name: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(parent) = current_exe.parent() {
        candidates.push(parent.join(binary_name));
        if let Some(parent_parent) = parent.parent() {
            candidates.push(parent_parent.join(binary_name));
        }
    }
    candidates
}

fn workspace_manifest_path(current_exe: &Path) -> Option<PathBuf> {
    let mut cursor = current_exe.parent();
    while let Some(dir) = cursor {
        let manifest = dir.join("Cargo.toml");
        if manifest.exists() {
            return Some(manifest);
        }
        cursor = dir.parent();
    }
    None
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
        None,
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
    use std::{thread, time::{Duration, Instant}};
    use std::sync::mpsc;

    use super::{
        BinaryLaunchTarget, DaemonBackedTuiApp, PendingSubmission, TuiApp, TuiCommand,
        TuiShellState, handle_pending_local_command, handle_pending_timeout, parse_tui_command,
        resolve_binary_launch_target, start_async_submit, status_line,
    };
    use oberon4_auth::{AccountRegistry, InMemoryAccountMetadataStore};
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

    #[test]
    fn async_submit_completes_and_updates_shell() {
        let socket_path = std::env::temp_dir().join(format!(
            "oberon4-tui-async-test-{}.sock",
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

        let mut app = DaemonBackedTuiApp::new(DaemonIpcClient::new(&socket_path));
        let mut shell = TuiShellState::default();
        for ch in "hello".chars() {
            shell.push_input_char(ch);
        }

        let pending = start_async_submit(&app, &mut shell)
            .unwrap()
            .expect("submission should start");
        assert_eq!(shell.input(), "");
        assert_eq!(shell.status(), "waiting on provider response...");
        assert_eq!(shell.transcript()[0].role, "user");
        assert_eq!(shell.transcript()[0].content, "hello");
        assert_eq!(shell.transcript()[1].role, "status");
        assert_eq!(
            shell.transcript()[1].content,
            "waiting on provider response..."
        );

        shell.push_input_char('x');

        let (updated_app, updated_shell) = pending
            .receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("worker should finish")
            .expect("submission should succeed");

        let preserved_input = std::mem::take(&mut shell.input);
        app = updated_app;
        shell = updated_shell;
        shell.input = preserved_input;

        assert_eq!(shell.input(), "x");
        assert_eq!(shell.status(), "daemon reply");
        assert_eq!(app.transcript().len(), 2);
        assert_eq!(shell.transcript()[0].role, "user");
        assert_eq!(shell.transcript()[1].role, "assistant");

        let client = DaemonIpcClient::new(&socket_path);
        let _ = client.send(&oberon4_core::DaemonCommand::Shutdown).unwrap();
        handle.join().unwrap().unwrap();
    }

    #[test]
    fn pending_request_allows_help_status_and_quit_commands() {
        let mut shell = TuiShellState {
            input: "/help".to_string(),
            ..TuiShellState::default()
        };
        assert!(handle_pending_local_command(&mut shell).unwrap());
        assert!(shell.status().contains("/help"));
        assert!(!shell.should_quit());

        shell.input = "/status".to_string();
        assert!(handle_pending_local_command(&mut shell).unwrap());
        assert_eq!(shell.status(), "request in flight");
        assert!(!shell.should_quit());

        shell.input = "/quit".to_string();
        assert!(handle_pending_local_command(&mut shell).unwrap());
        assert!(shell.should_quit());
    }

    #[test]
    fn async_submit_error_preserves_user_message() {
        unsafe {
            std::env::set_var("OBERON4_DAEMON_BIN", "/definitely/missing/oberon4-daemon");
        }
        let socket_path = std::env::temp_dir().join(format!(
            "oberon4-tui-async-error-{}.sock",
            std::process::id()
        ));
        let app = DaemonBackedTuiApp::new(DaemonIpcClient::new(&socket_path));
        let mut shell = TuiShellState::default();
        for ch in "hello".chars() {
            shell.push_input_char(ch);
        }

        let pending = start_async_submit(&app, &mut shell)
            .unwrap()
            .expect("submission should start");
        let result = pending
            .receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("worker should finish");

        match result {
            Ok((_app, shell)) => {
                assert_eq!(shell.transcript()[0].role, "user");
                assert_eq!(shell.transcript()[0].content, "hello");
            }
            Err((shell, err)) => {
                assert_eq!(shell.transcript()[0].role, "user");
                assert_eq!(shell.transcript()[0].content, "hello");
                assert!(!err.to_string().is_empty());
            }
        }

        unsafe {
            std::env::remove_var("OBERON4_DAEMON_BIN");
        }
    }

    #[test]
    fn pending_submission_times_out_and_recovers_ui() {
        let (_sender, receiver) = mpsc::channel();
        let mut pending = Some(PendingSubmission {
            receiver,
            started_at: Instant::now() - Duration::from_secs(2),
            timeout: Duration::from_millis(10),
        });
        let mut shell = TuiShellState::default();

        handle_pending_timeout(&mut pending, &mut shell);

        assert!(pending.is_none());
        assert_eq!(shell.status(), "request timed out");
        assert_eq!(shell.transcript()[0].role, "error");
    }

    #[test]
    fn resolves_workspace_cargo_fallback_when_daemon_binary_is_missing() {
        let root = std::env::temp_dir().join(format!(
            "oberon4-tui-launcher-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
        let current = root.join("target/debug/oberon4-tui");
        let target = resolve_binary_launch_target(&current, "oberon4-daemon")
            .expect("workspace fallback should resolve");

        match target {
            BinaryLaunchTarget::CargoRun {
                manifest_path,
                package,
                ..
            } => {
                assert_eq!(manifest_path, root.join("Cargo.toml"));
                assert_eq!(package, "oberon4-daemon");
            }
            other => panic!("expected cargo fallback, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&root);
    }
}
