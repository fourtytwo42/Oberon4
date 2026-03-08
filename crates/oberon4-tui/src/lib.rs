use std::{
    collections::BTreeMap,
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
    AppConfig, ChatMessage, ChatProvider, ChatRole, ChatToolCallDelta, InMemoryConfigStore,
    InMemorySessionStore, OAuthProviderKind, OberonError, SavedSession, SessionId, SessionStore,
    SessionSummary,
};
use oberon4_daemon::{DaemonIpcClient, DaemonRuntime, LifecycleState};
use oberon4_provider_openai::{FakeTransport, OpenAiCompatibleClient, RetryPolicy};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs, Wrap},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiCommand {
    Help,
    Quit,
    Status,
    Sessions,
    Config,
    Model(String),
    Endpoint(String),
    System(String),
    Clear,
    Save(String),
    Load(String),
    Fork(String),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivityState {
    Idle,
    Running,
    WaitingOnProvider,
    Streaming,
    Saving,
    Loading,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SidePanelTab {
    Status,
    Sessions,
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusTarget {
    Composer,
    Sessions,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct StatusSnapshot {
    running: bool,
    endpoint: String,
    model: String,
    active_account: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AutocompleteState {
    matches: Vec<String>,
    selected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiShellState {
    input: String,
    status: String,
    activity: ActivityState,
    should_quit: bool,
    transcript: Vec<UiLine>,
    side_panel_tab: SidePanelTab,
    side_panel_visible: bool,
    overlay_visible: bool,
    focus: FocusTarget,
    sessions: Vec<SessionSummary>,
    session_index: usize,
    current_session: Option<String>,
    baseline_signature: Option<String>,
    status_snapshot: StatusSnapshot,
    autocomplete: Option<AutocompleteState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TuiLaunchOptions {
    pub no_alt_screen: bool,
}

const PANEL_SPLIT_WIDTH: u16 = 120;
const FOOTER_HEIGHT: u16 = 5;
const COMMAND_CATALOG: [(&str, bool); 12] = [
    ("/help", false),
    ("/quit", false),
    ("/status", false),
    ("/sessions", false),
    ("/config", false),
    ("/model", true),
    ("/endpoint", true),
    ("/system", true),
    ("/clear", false),
    ("/save", true),
    ("/load", true),
    ("/fork", true),
];
const EXTENDED_HELP: &str = "/help /quit /status /sessions /config /model <name> /endpoint <url> /system <text> /clear /save <name> /load <name> /fork <name>";

struct PendingSubmission {
    receiver: Receiver<AsyncUiEvent>,
    started_at: Instant,
    timeout: Duration,
    assistant_line_index: usize,
    tool_line_indices: BTreeMap<usize, usize>,
    tool_state: BTreeMap<usize, (String, String)>,
}

#[derive(Debug)]
enum AsyncUiEvent {
    TextDelta(String),
    ToolCallDelta(ChatToolCallDelta),
    Complete {
        prompt: String,
        message: ChatMessage,
    },
    Failed(OberonError),
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
            status: "ready".to_string(),
            activity: ActivityState::Idle,
            should_quit: false,
            transcript: Vec::new(),
            side_panel_tab: SidePanelTab::Status,
            side_panel_visible: true,
            overlay_visible: false,
            focus: FocusTarget::Composer,
            sessions: Vec::new(),
            session_index: 0,
            current_session: None,
            baseline_signature: None,
            status_snapshot: StatusSnapshot::default(),
            autocomplete: None,
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
                output: EXTENDED_HELP.to_string(),
                should_quit: false,
            }),
            TuiCommand::Quit => Ok(TuiResponse {
                output: "quitting".to_string(),
                should_quit: true,
            }),
            TuiCommand::Sessions => Ok(TuiResponse {
                output: "sessions panel opened".to_string(),
                should_quit: false,
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
            TuiCommand::Fork(name) => {
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
                    output: format!("forked session {name}"),
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

    pub fn list_sessions(&self) -> Result<Vec<SessionSummary>, OberonError> {
        let mut sessions = self
            .runtime
            .session_store
            .list_sessions()?
            .into_iter()
            .map(|session| session.summary)
            .collect::<Vec<_>>();
        sort_sessions(&mut sessions);
        Ok(sessions)
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
                    tool_calls: Vec::new(),
                });
            }
        }
        self.transcript.push(ChatMessage {
            role: ChatRole::User,
            content: prompt.to_string(),
            tool_calls: Vec::new(),
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
                output: EXTENDED_HELP.to_string(),
                should_quit: false,
            }),
            TuiCommand::Quit => Ok(TuiResponse {
                output: "quitting".to_string(),
                should_quit: true,
            }),
            TuiCommand::Sessions => Ok(TuiResponse {
                output: "sessions panel opened".to_string(),
                should_quit: false,
            }),
            TuiCommand::Status => {
                match self.client.send(&oberon4_core::DaemonCommand::GetStatus)? {
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
                }
            }
            TuiCommand::Config => {
                match self.client.send(&oberon4_core::DaemonCommand::LoadConfig)? {
                    oberon4_core::DaemonEvent::ConfigLoaded { config } => Ok(TuiResponse {
                        output: redacted_config_json(&config)?,
                        should_quit: false,
                    }),
                    event => protocol_error(event),
                }
            }
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
            TuiCommand::Load(name) => {
                match self
                    .client
                    .send(&oberon4_core::DaemonCommand::LoadSession {
                        session_id: name.clone(),
                    })? {
                    oberon4_core::DaemonEvent::SessionLoaded { session } => {
                        self.transcript = session.messages;
                        Ok(TuiResponse {
                            output: format!("loaded session {name}"),
                            should_quit: false,
                        })
                    }
                    event => protocol_error(event),
                }
            }
            TuiCommand::Fork(name) => {
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
                        output: format!("forked session {name}"),
                        should_quit: false,
                    }),
                    event => protocol_error(event),
                }
            }
        }
    }

    pub fn transcript(&self) -> &[ChatMessage] {
        &self.transcript
    }

    pub fn system_prompt(&self) -> Option<&str> {
        self.system_prompt.as_deref()
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionSummary>, OberonError> {
        match self
            .client
            .send(&oberon4_core::DaemonCommand::ListSessions)?
        {
            oberon4_core::DaemonEvent::SessionList { sessions } => {
                let mut summaries = sessions
                    .into_iter()
                    .map(|session| session.summary)
                    .collect::<Vec<_>>();
                sort_sessions(&mut summaries);
                Ok(summaries)
            }
            event => Err(protocol_error_inner(event)),
        }
    }

    fn load_status_snapshot(&self) -> Result<StatusSnapshot, OberonError> {
        match self.client.send(&oberon4_core::DaemonCommand::GetStatus)? {
            oberon4_core::DaemonEvent::Status { status } => Ok(StatusSnapshot {
                running: status.running,
                endpoint: status.endpoint_base_url,
                model: status.model,
                active_account: status.active_account.unwrap_or_else(|| "none".to_string()),
            }),
            event => Err(protocol_error_inner(event)),
        }
    }

    fn send_prompt(&mut self, prompt: &str) -> Result<TuiResponse, OberonError> {
        let message = self.stream_prompt_events(prompt, &mut |_| Ok(()))?;
        self.apply_successful_prompt(prompt, message.clone());
        Ok(TuiResponse {
            output: message.content,
            should_quit: false,
        })
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

    fn build_request_messages(&self, prompt: &str) -> Vec<ChatMessage> {
        let mut messages = self.transcript.clone();
        if let Some(prompt_text) = self.system_prompt.clone() {
            if messages.is_empty() {
                messages.push(ChatMessage {
                    role: ChatRole::System,
                    content: prompt_text,
                    tool_calls: Vec::new(),
                });
            }
        }
        messages.push(ChatMessage {
            role: ChatRole::User,
            content: prompt.to_string(),
            tool_calls: Vec::new(),
        });
        messages
    }

    fn apply_successful_prompt(&mut self, prompt: &str, message: ChatMessage) {
        if let Some(prompt_text) = self.system_prompt.clone() {
            if self.transcript.is_empty() {
                self.transcript.push(ChatMessage {
                    role: ChatRole::System,
                    content: prompt_text,
                    tool_calls: Vec::new(),
                });
            }
        }
        self.transcript.push(ChatMessage {
            role: ChatRole::User,
            content: prompt.to_string(),
            tool_calls: Vec::new(),
        });
        self.transcript.push(message);
    }

    fn stream_prompt_events(
        &self,
        prompt: &str,
        on_event: &mut dyn FnMut(oberon4_core::DaemonEvent) -> Result<(), OberonError>,
    ) -> Result<ChatMessage, OberonError> {
        let mut final_message = None;
        self.client.send_stream(
            &oberon4_core::DaemonCommand::SendChat {
                session_id: None,
                messages: self.build_request_messages(prompt),
            },
            &mut |event| match event {
                oberon4_core::DaemonEvent::ChatComplete { message } => {
                    final_message = Some(message);
                    Ok(())
                }
                oberon4_core::DaemonEvent::Error { message } => {
                    Err(OberonError::Validation(message))
                }
                other => on_event(other),
            },
        )?;

        final_message.ok_or_else(|| {
            OberonError::Storage("daemon stream ended without ChatComplete".to_string())
        })
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
        self.focus = FocusTarget::Composer;
        self.refresh_autocomplete();
    }

    pub fn backspace(&mut self) {
        let _ = self.input.pop();
        self.refresh_autocomplete();
    }

    pub fn insert_newline(&mut self) {
        self.input.push('\n');
        self.refresh_autocomplete();
    }

    pub fn activity_label(&self) -> &'static str {
        match self.activity {
            ActivityState::Idle => "idle",
            ActivityState::Running => "running",
            ActivityState::WaitingOnProvider => "waiting on provider",
            ActivityState::Streaming => "streaming",
            ActivityState::Saving => "saving",
            ActivityState::Loading => "loading",
            ActivityState::Error => "error",
        }
    }

    fn show_tab(&mut self, tab: SidePanelTab, terminal_width: u16) {
        self.side_panel_tab = tab;
        if terminal_width >= PANEL_SPLIT_WIDTH {
            self.side_panel_visible = true;
            self.overlay_visible = false;
        } else {
            self.overlay_visible = true;
            self.side_panel_visible = false;
        }
        self.focus = if matches!(tab, SidePanelTab::Sessions) {
            FocusTarget::Sessions
        } else {
            FocusTarget::Composer
        };
    }

    pub fn handle_escape(&mut self) {
        if self.autocomplete.is_some() {
            self.autocomplete = None;
            return;
        }
        if self.overlay_visible {
            self.overlay_visible = false;
            self.focus = FocusTarget::Composer;
            return;
        }
        if !self.input.is_empty() {
            self.input.clear();
            self.refresh_autocomplete();
            self.status = "draft cleared".to_string();
            return;
        }
        self.should_quit = true;
        self.status = "quitting".to_string();
    }

    pub fn cycle_autocomplete(&mut self) {
        let Some(state) = self.autocomplete.as_mut() else {
            self.refresh_autocomplete();
            return;
        };
        if !state.matches.is_empty() {
            state.selected = (state.selected + 1) % state.matches.len();
        }
    }

    pub fn accept_autocomplete(&mut self) -> bool {
        let Some(state) = self.autocomplete.as_ref() else {
            return false;
        };
        let Some(selected) = state.matches.get(state.selected) else {
            return false;
        };
        if command_requires_argument(selected) {
            self.input = format!("{selected} ");
        } else {
            self.input = selected.clone();
        }
        self.autocomplete = None;
        true
    }

    pub fn move_session_selection(&mut self, delta: isize) {
        if self.sessions.is_empty() {
            return;
        }
        let len = self.sessions.len() as isize;
        let next = (self.session_index as isize + delta).clamp(0, len - 1);
        self.session_index = next as usize;
    }

    fn set_activity(&mut self, activity: ActivityState, status: impl Into<String>) {
        self.activity = activity;
        self.status = status.into();
    }

    pub fn refresh_sidebar(&mut self, app: &DaemonBackedTuiApp) {
        if let Ok(snapshot) = app.load_status_snapshot() {
            self.status_snapshot = snapshot;
        }
        if let Ok(sessions) = app.list_sessions() {
            self.sessions = sessions;
            if self.session_index >= self.sessions.len() {
                self.session_index = self.sessions.len().saturating_sub(1);
            }
        }
    }

    pub fn mark_loaded_session(&mut self, session: Option<String>) {
        self.current_session = session;
        self.baseline_signature = Some(transcript_signature(&self.transcript));
    }

    pub fn is_dirty(&self) -> bool {
        let current = transcript_signature(&self.transcript);
        match &self.baseline_signature {
            Some(baseline) => baseline != &current,
            None => !self.transcript.is_empty(),
        }
    }

    fn refresh_autocomplete(&mut self) {
        let trimmed = self.input.trim();
        if !trimmed.starts_with('/') || trimmed.contains(' ') || trimmed.is_empty() {
            self.autocomplete = None;
            return;
        }
        let mut matches = COMMAND_CATALOG
            .iter()
            .map(|(command, _)| (*command).to_string())
            .filter(|command| command.starts_with(trimmed))
            .collect::<Vec<_>>();
        matches.sort();
        if matches.is_empty() {
            self.autocomplete = None;
        } else {
            let selected = self
                .autocomplete
                .as_ref()
                .map(|state| state.selected.min(matches.len().saturating_sub(1)))
                .unwrap_or(0);
            self.autocomplete = Some(AutocompleteState { matches, selected });
        }
    }

    pub fn submit(&mut self, app: &mut DaemonBackedTuiApp) -> Result<(), OberonError> {
        let input = std::mem::take(&mut self.input);
        self.autocomplete = None;
        let trimmed = input.trim();
        if trimmed.is_empty() {
            self.set_activity(ActivityState::Idle, "Input was empty");
            return Ok(());
        }

        if trimmed == "/sessions" {
            self.show_tab(SidePanelTab::Sessions, PANEL_SPLIT_WIDTH.saturating_sub(1));
            self.refresh_sidebar(app);
            self.set_activity(ActivityState::Idle, "sessions panel opened");
            return Ok(());
        }

        if !trimmed.starts_with('/') && !has_trailing_user_line(&self.transcript, trimmed) {
            self.transcript.push(UiLine {
                role: "user".to_string(),
                content: trimmed.to_string(),
            });
        }

        let command = if trimmed.starts_with('/') {
            Some(parse_tui_command(trimmed)?)
        } else {
            None
        };

        if matches!(command, Some(TuiCommand::Help)) {
            self.show_tab(SidePanelTab::Help, PANEL_SPLIT_WIDTH.saturating_sub(1));
        } else if matches!(command, Some(TuiCommand::Status)) {
            self.show_tab(SidePanelTab::Status, PANEL_SPLIT_WIDTH.saturating_sub(1));
        }

        match command {
            Some(TuiCommand::Save(_)) => self.activity = ActivityState::Saving,
            Some(TuiCommand::Load(_)) => self.activity = ActivityState::Loading,
            Some(TuiCommand::Fork(_)) => self.activity = ActivityState::Saving,
            _ => self.activity = ActivityState::Running,
        }

        let response = app.handle_input(trimmed)?;
        self.set_activity(ActivityState::Idle, response.output.clone());
        self.should_quit = response.should_quit;

        if trimmed.starts_with('/') {
            if trimmed == "/clear" {
                self.transcript.clear();
            }
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
            match command {
                Some(TuiCommand::Save(name)) => self.mark_loaded_session(Some(name)),
                Some(TuiCommand::Load(name)) => self.mark_loaded_session(Some(name)),
                Some(TuiCommand::Clear) => {}
                _ => {}
            }
            self.refresh_sidebar(app);
            return Ok(());
        }

        if let Some(last) = app.transcript().last() {
            self.transcript.push(UiLine {
                role: role_label(last.role).to_string(),
                content: last.content.clone(),
            });
        }

        self.refresh_sidebar(app);

        Ok(())
    }
}

pub fn run_terminal_ui() -> Result<(), OberonError> {
    run_terminal_ui_with_options(TuiLaunchOptions::default())
}

pub fn run_terminal_ui_with_options(options: TuiLaunchOptions) -> Result<(), OberonError> {
    let mut app = DaemonBackedTuiApp::default();
    let mut shell = TuiShellState::default();
    shell.refresh_sidebar(&app);
    let mut terminal = init_terminal(options)?;

    let result = run_terminal_loop(&mut terminal, &mut app, &mut shell);
    restore_terminal(&mut terminal, options)?;
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
                Ok(event) => handle_async_ui_event(event, &mut pending_submission, app, shell),
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
                shell.set_activity(ActivityState::Idle, "quitting");
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let width = terminal
                    .size()
                    .map_err(|err| OberonError::Storage(format!("tui size lookup failed: {err}")))?
                    .width;
                shell.show_tab(SidePanelTab::Sessions, width);
                shell.refresh_sidebar(app);
                shell.set_activity(ActivityState::Idle, "sessions panel opened");
            }
            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                shell.insert_newline();
            }
            KeyCode::Char(ch) => shell.push_input_char(ch),
            KeyCode::Backspace => shell.backspace(),
            KeyCode::Tab => shell.cycle_autocomplete(),
            KeyCode::Up if matches!(shell.focus, FocusTarget::Sessions) => {
                shell.move_session_selection(-1)
            }
            KeyCode::Down if matches!(shell.focus, FocusTarget::Sessions) => {
                shell.move_session_selection(1)
            }
            KeyCode::Enter => {
                if matches!(shell.focus, FocusTarget::Sessions)
                    && !shell.sessions.is_empty()
                    && pending_submission.is_none()
                {
                    let name = shell.sessions[shell.session_index].id.as_str().to_string();
                    shell.input = format!("/load {name}");
                    shell.overlay_visible = false;
                    shell.focus = FocusTarget::Composer;
                    shell.submit(app)?;
                } else if shell.input.trim().starts_with('/')
                    && shell.input.trim() == shell.input.trim_end()
                    && !shell.input.contains(' ')
                    && shell.accept_autocomplete()
                {
                    shell.set_activity(ActivityState::Idle, "command completed");
                } else if pending_submission.is_some() {
                    if !handle_pending_local_command(shell)? {
                        shell.set_activity(
                            ActivityState::Running,
                            "request in flight; wait, or press Esc/Ctrl+C to quit",
                        );
                    }
                } else if let Some(pending) = start_async_submit(app, shell)? {
                    pending_submission = Some(pending);
                }
            }
            KeyCode::Esc => shell.handle_escape(),
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
            output: EXTENDED_HELP.to_string(),
            should_quit: false,
        },
        TuiCommand::Status => TuiResponse {
            output: "request in flight".to_string(),
            should_quit: false,
        },
        TuiCommand::Sessions => TuiResponse {
            output: "sessions panel opened".to_string(),
            should_quit: false,
        },
        TuiCommand::Quit => TuiResponse {
            output: "quitting".to_string(),
            should_quit: true,
        },
        _ => return Ok(false),
    };

    shell.input.clear();
    shell.set_activity(ActivityState::Idle, response.output.clone());
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
    app: &mut DaemonBackedTuiApp,
    shell: &mut TuiShellState,
) -> Result<Option<PendingSubmission>, OberonError> {
    let trimmed = shell.input.trim().to_string();
    if trimmed.is_empty() {
        shell.input.clear();
        shell.set_activity(ActivityState::Idle, "Input was empty");
        return Ok(None);
    }

    if trimmed.starts_with('/') {
        shell.submit(app)?;
        return Ok(None);
    }

    let app_clone = app.clone();
    let prompt = trimmed.clone();
    shell.input.clear();
    let timeout = app
        .load_config()
        .map(|config| Duration::from_millis(config.runtime.stream_idle_timeout_ms))
        .unwrap_or_else(|_| Duration::from_secs(30));
    shell.transcript.push(UiLine {
        role: "user".to_string(),
        content: trimmed,
    });
    shell.transcript.push(UiLine {
        role: "status".to_string(),
        content: "waiting on provider response...".to_string(),
    });
    shell.transcript.push(UiLine {
        role: "assistant".to_string(),
        content: "...".to_string(),
    });
    let assistant_line_index = shell.transcript.len() - 1;
    shell.set_activity(
        ActivityState::WaitingOnProvider,
        "waiting on provider response...",
    );
    shell.status = "waiting on provider response...".to_string();

    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let stream_sender = sender.clone();
        let result = app_clone.stream_prompt_events(&prompt, &mut |event| {
            let mapped = match event {
                oberon4_core::DaemonEvent::ChatChunk { content } => {
                    AsyncUiEvent::TextDelta(content)
                }
                oberon4_core::DaemonEvent::ToolCallDelta { delta } => {
                    AsyncUiEvent::ToolCallDelta(delta)
                }
                other => {
                    return Err(OberonError::Storage(format!(
                        "unexpected daemon stream event: {other:?}"
                    )));
                }
            };
            stream_sender
                .send(mapped)
                .map_err(|_| OberonError::Storage("tui stream channel closed".to_string()))
        });

        let _ = match result {
            Ok(message) => sender.send(AsyncUiEvent::Complete { prompt, message }),
            Err(err) => sender.send(AsyncUiEvent::Failed(err)),
        };
    });

    Ok(Some(PendingSubmission {
        receiver,
        started_at: Instant::now(),
        timeout,
        assistant_line_index,
        tool_line_indices: BTreeMap::new(),
        tool_state: BTreeMap::new(),
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
        .map(|pending| {
            pending.started_at.elapsed()
                > pending.timeout.saturating_add(Duration::from_millis(500))
        })
        .unwrap_or(false);

    if !should_timeout {
        return;
    }

    if let Some(pending) = pending_submission.take() {
        if let Some(line) = shell.transcript.get_mut(pending.assistant_line_index) {
            if line.content == "..." || line.content.is_empty() {
                line.role = "error".to_string();
                line.content = "request timed out".to_string();
            }
        }
    }
    shell.set_activity(ActivityState::Error, "request timed out");
}

fn handle_async_ui_event(
    event: AsyncUiEvent,
    pending_submission: &mut Option<PendingSubmission>,
    app: &mut DaemonBackedTuiApp,
    shell: &mut TuiShellState,
) {
    match event {
        AsyncUiEvent::TextDelta(content) => {
            if let Some(pending) = pending_submission.as_mut() {
                if let Some(line) = shell.transcript.get_mut(pending.assistant_line_index) {
                    if line.content == "..." {
                        line.content.clear();
                    }
                    line.content.push_str(&content);
                    shell.set_activity(ActivityState::Streaming, "streaming response...");
                }
            }
        }
        AsyncUiEvent::ToolCallDelta(delta) => {
            if let Some(pending) = pending_submission.as_mut() {
                let line_index = if let Some(index) = pending.tool_line_indices.get(&delta.index) {
                    *index
                } else {
                    shell.transcript.push(UiLine {
                        role: "tool".to_string(),
                        content: String::new(),
                    });
                    let index = shell.transcript.len() - 1;
                    pending.tool_line_indices.insert(delta.index, index);
                    index
                };
                let content = merge_tool_ui_delta(&mut pending.tool_state, delta);
                if let Some(line) = shell.transcript.get_mut(line_index) {
                    line.content = content;
                }
            }
        }
        AsyncUiEvent::Complete { prompt, message } => {
            let pending = pending_submission.take();
            if let Some(pending) = pending {
                finalize_streamed_assistant(shell, &pending, &message);
            }
            app.apply_successful_prompt(&prompt, message.clone());
            shell.set_activity(
                ActivityState::Idle,
                if message.content.is_empty() {
                    "stream complete".to_string()
                } else {
                    message.content.clone()
                },
            );
            shell.refresh_sidebar(app);
        }
        AsyncUiEvent::Failed(err) => {
            if let Some(pending) = pending_submission.take() {
                if let Some(line) = shell.transcript.get_mut(pending.assistant_line_index) {
                    line.role = "error".to_string();
                    line.content = err.to_string();
                } else {
                    shell.transcript.push(UiLine {
                        role: "error".to_string(),
                        content: err.to_string(),
                    });
                }
            }
            shell.set_activity(ActivityState::Error, err.to_string());
            shell.refresh_sidebar(app);
        }
    }
}

fn merge_tool_ui_delta(
    tool_state: &mut BTreeMap<usize, (String, String)>,
    delta: ChatToolCallDelta,
) -> String {
    let entry = tool_state.entry(delta.index).or_default();
    if let Some(name) = delta.name_fragment.as_deref() {
        entry.0.push_str(name);
    }
    if let Some(arguments) = delta.arguments_fragment.as_deref() {
        entry.1.push_str(arguments);
    }
    let name = if entry.0.is_empty() {
        format!("tool_{}", delta.index)
    } else {
        entry.0.clone()
    };
    format!("{name} {}", entry.1)
}

fn finalize_streamed_assistant(
    shell: &mut TuiShellState,
    pending: &PendingSubmission,
    message: &ChatMessage,
) {
    if let Some(line) = shell.transcript.get_mut(pending.assistant_line_index) {
        let streamed = line.content.trim();
        let final_content = message.content.trim();
        if final_content.is_empty() && streamed.is_empty() {
            line.content.clear();
        } else if final_content.is_empty() {
            line.content = streamed.to_string();
        } else if streamed.is_empty() || streamed == "..." || final_content.len() >= streamed.len()
        {
            line.content = final_content.to_string();
        } else {
            line.content = streamed.to_string();
        }
    }

    for (index, tool_call) in message.tool_calls.iter().enumerate() {
        if let Some(line_index) = pending.tool_line_indices.get(&index).copied() {
            if let Some(line) = shell.transcript.get_mut(line_index) {
                line.content = format!("{} {}", tool_call.name, tool_call.arguments);
            }
        }
    }
}

fn render_terminal(frame: &mut Frame, shell: &TuiShellState) {
    let area = frame.size();
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(5),
        Constraint::Length(FOOTER_HEIGHT),
    ])
    .areas(area);

    render_header(frame, header, shell);
    render_body(frame, body, shell);
    render_footer(frame, footer, shell);

    let cursor_lines = composer_lines(shell.input());
    let last_line = cursor_lines.last().cloned().unwrap_or_default();
    let cursor_y = footer
        .y
        .saturating_add(1 + cursor_lines.len().saturating_sub(1) as u16);
    let cursor_x = footer.x.saturating_add(1 + last_line.len() as u16);
    frame.set_cursor(cursor_x, cursor_y);
}

fn render_header(frame: &mut Frame, area: Rect, shell: &TuiShellState) {
    let session = shell.current_session.as_deref().unwrap_or("unsaved");
    let dirty = if shell.is_dirty() { "*" } else { "" };
    let header_lines = Text::from(vec![
        Line::styled(
            "Oberon4 TUI",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Line::raw(format!(
            "daemon={} activity={} endpoint={} model={} account={} session={}{}",
            if shell.status_snapshot.running {
                "running"
            } else {
                "stopped"
            },
            shell.activity_label(),
            shell.status_snapshot.endpoint,
            shell.status_snapshot.model,
            shell.status_snapshot.active_account,
            session,
            dirty,
        )),
    ]);
    let header_widget = Paragraph::new(header_lines)
        .block(Block::default().borders(Borders::ALL).title("Overview"));
    frame.render_widget(header_widget, area);
}

fn render_body(frame: &mut Frame, area: Rect, shell: &TuiShellState) {
    let transcript_area;
    let panel_area;
    if area.width >= PANEL_SPLIT_WIDTH {
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(68), Constraint::Percentage(32)])
                .areas(area);
        transcript_area = left;
        panel_area = Some(right);
    } else {
        transcript_area = area;
        panel_area = None;
    }

    render_transcript(frame, transcript_area, shell);

    if let Some(panel_area) = panel_area {
        if shell.side_panel_visible {
            render_side_panel(frame, panel_area, shell);
        }
    } else if shell.overlay_visible {
        let popup = centered_rect(area, 80, 70);
        frame.render_widget(Clear, popup);
        render_side_panel(frame, popup, shell);
    }
}

fn render_transcript(frame: &mut Frame, area: Rect, shell: &TuiShellState) {
    let transcript_lines: Vec<Line<'static>> = if shell.transcript().is_empty() {
        vec![Line::raw(
            "No messages yet. Type text to chat, /sessions to browse saved chats, or /help for commands.",
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
    frame.render_widget(transcript, area);
}

fn render_side_panel(frame: &mut Frame, area: Rect, shell: &TuiShellState) {
    let [tabs_area, content_area] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(5)]).areas(area);
    let titles = vec!["Status", "Sessions", "Help"];
    let selected = match shell.side_panel_tab {
        SidePanelTab::Status => 0,
        SidePanelTab::Sessions => 1,
        SidePanelTab::Help => 2,
    };
    let tabs = Tabs::new(titles)
        .select(selected)
        .block(Block::default().borders(Borders::ALL).title("Panel"))
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(tabs, tabs_area);

    match shell.side_panel_tab {
        SidePanelTab::Status => {
            let lines = Text::from(vec![
                Line::raw(format!("activity: {}", shell.activity_label())),
                Line::raw(format!("status: {}", shell.status())),
                Line::raw(format!("endpoint: {}", shell.status_snapshot.endpoint)),
                Line::raw(format!("model: {}", shell.status_snapshot.model)),
                Line::raw(format!(
                    "active account: {}",
                    shell.status_snapshot.active_account
                )),
                Line::raw(format!(
                    "session: {}",
                    shell.current_session.as_deref().unwrap_or("unsaved")
                )),
                Line::raw(format!("dirty: {}", shell.is_dirty())),
            ]);
            frame.render_widget(
                Paragraph::new(lines)
                    .block(Block::default().borders(Borders::ALL).title("Status"))
                    .wrap(Wrap { trim: false }),
                content_area,
            );
        }
        SidePanelTab::Sessions => {
            let items: Vec<ListItem<'static>> = if shell.sessions.is_empty() {
                vec![ListItem::new("No saved sessions")]
            } else {
                shell
                    .sessions
                    .iter()
                    .enumerate()
                    .map(|(index, session)| {
                        let prefix = if index == shell.session_index {
                            ">"
                        } else {
                            " "
                        };
                        ListItem::new(format!("{prefix} {} ({})", session.title, session.model))
                    })
                    .collect()
            };
            frame.render_widget(
                List::new(items).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Sessions (Ctrl+R, Up/Down, Enter)"),
                ),
                content_area,
            );
        }
        SidePanelTab::Help => {
            frame.render_widget(
                Paragraph::new(Text::from(vec![
                    Line::raw(EXTENDED_HELP),
                    Line::raw("Enter send | Ctrl+J newline | Tab autocomplete"),
                    Line::raw("Ctrl+R sessions | Esc close/clear/quit"),
                ]))
                .block(Block::default().borders(Borders::ALL).title("Help"))
                .wrap(Wrap { trim: false }),
                content_area,
            );
        }
    }
}

fn render_footer(frame: &mut Frame, area: Rect, shell: &TuiShellState) {
    let [input_area, hint_area] =
        Layout::vertical([Constraint::Length(4), Constraint::Length(1)]).areas(area);
    let input = Paragraph::new(shell.input())
        .block(Block::default().borders(Borders::ALL).title("Composer"))
        .wrap(Wrap { trim: false });
    frame.render_widget(Clear, input_area);
    frame.render_widget(input, input_area);

    if let Some(autocomplete) = shell.autocomplete.as_ref() {
        let suggestion = autocomplete
            .matches
            .get(autocomplete.selected)
            .cloned()
            .unwrap_or_default();
        frame.render_widget(
            Paragraph::new(format!("autocomplete: {suggestion}")),
            hint_area,
        );
    } else {
        frame.render_widget(
            Paragraph::new("Enter send | Ctrl+J newline | Tab command autocomplete | Ctrl+R sessions | Esc close/clear/quit"),
            hint_area,
        );
    }
}

fn init_terminal(
    options: TuiLaunchOptions,
) -> Result<Terminal<CrosstermBackend<io::Stdout>>, OberonError> {
    enable_raw_mode()
        .map_err(|err| OberonError::Storage(format!("raw mode enable failed: {err}")))?;
    let mut stdout = io::stdout();
    if !options.no_alt_screen {
        execute!(stdout, EnterAlternateScreen)
            .map_err(|err| OberonError::Storage(format!("enter alt screen failed: {err}")))?;
    }
    Terminal::new(CrosstermBackend::new(stdout))
        .map_err(|err| OberonError::Storage(format!("terminal init failed: {err}")))
}

fn restore_terminal(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    options: TuiLaunchOptions,
) -> Result<(), OberonError> {
    if !options.no_alt_screen {
        execute!(terminal.backend_mut(), LeaveAlternateScreen)
            .map_err(|err| OberonError::Storage(format!("leave alt screen failed: {err}")))?;
    }
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
        "/sessions" => Ok(TuiCommand::Sessions),
        "/config" => Ok(TuiCommand::Config),
        "/model" => require_argument(rest, "/model").map(TuiCommand::Model),
        "/endpoint" => require_argument(rest, "/endpoint").map(TuiCommand::Endpoint),
        "/system" => require_argument(rest, "/system").map(TuiCommand::System),
        "/clear" => Ok(TuiCommand::Clear),
        "/save" => require_argument(rest, "/save").map(TuiCommand::Save),
        "/load" => require_argument(rest, "/load").map(TuiCommand::Load),
        "/fork" => require_argument(rest, "/fork").map(TuiCommand::Fork),
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

fn sort_sessions(sessions: &mut [SessionSummary]) {
    sessions.sort_by(|left, right| {
        left.title
            .cmp(&right.title)
            .then_with(|| left.id.as_str().cmp(right.id.as_str()))
    });
}

fn command_requires_argument(command: &str) -> bool {
    COMMAND_CATALOG
        .iter()
        .find(|(candidate, _)| *candidate == command)
        .map(|(_, requires_argument)| *requires_argument)
        .unwrap_or(false)
}

fn transcript_signature(lines: &[UiLine]) -> String {
    lines
        .iter()
        .map(|line| format!("{}:{}", line.role, line.content))
        .collect::<Vec<_>>()
        .join("\n")
}

fn composer_lines(input: &str) -> Vec<&str> {
    if input.is_empty() {
        vec![""]
    } else {
        input.split('\n').collect()
    }
}

fn centered_rect(area: Rect, width_percent: u16, height_percent: u16) -> Rect {
    let [_top, middle, _bottom] = Layout::vertical([
        Constraint::Percentage((100_u16.saturating_sub(height_percent)) / 2),
        Constraint::Percentage(height_percent),
        Constraint::Percentage(100_u16.saturating_sub(height_percent).div_ceil(2)),
    ])
    .areas(area);
    let [_left, center, _right] = Layout::horizontal([
        Constraint::Percentage((100_u16.saturating_sub(width_percent)) / 2),
        Constraint::Percentage(width_percent),
        Constraint::Percentage(100_u16.saturating_sub(width_percent).div_ceil(2)),
    ])
    .areas(middle);
    center
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

fn resolve_binary_launch_target(
    current_exe: &Path,
    binary_name: &str,
) -> Option<BinaryLaunchTarget> {
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
    let mut client = OpenAiCompatibleClient::new(
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
    );
    client.set_streaming_config(false, 30_000);
    client
}

fn ok_response(content: &str) -> String {
    format!(r#"{{"choices":[{{"message":{{"role":"assistant","content":"{content}"}}}}]}}"#)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::{
        collections::BTreeMap,
        thread,
        time::{Duration, Instant},
    };

    use super::{
        AsyncUiEvent, BinaryLaunchTarget, DaemonBackedTuiApp, FocusTarget, PANEL_SPLIT_WIDTH,
        PendingSubmission, SidePanelTab, TuiApp, TuiCommand, TuiShellState, UiLine,
        handle_async_ui_event, handle_pending_local_command, handle_pending_timeout,
        parse_tui_command, resolve_binary_launch_target, start_async_submit, status_line,
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
            parse_tui_command("/sessions").unwrap(),
            TuiCommand::Sessions
        );
        assert_eq!(
            parse_tui_command("/model o4-mini").unwrap(),
            TuiCommand::Model("o4-mini".to_string())
        );
        assert_eq!(
            parse_tui_command("/save demo").unwrap(),
            TuiCommand::Save("demo".to_string())
        );
        assert_eq!(
            parse_tui_command("/fork demo-copy").unwrap(),
            TuiCommand::Fork("demo-copy".to_string())
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
    fn fork_command_saves_independent_session() {
        let mut app = TuiApp::default();
        let _ = app.handle_input("hello").unwrap();

        let fork = app
            .handle_command(TuiCommand::Fork("demo-copy".to_string()))
            .unwrap();
        let clear = app.handle_command(TuiCommand::Clear).unwrap();
        let load = app
            .handle_command(TuiCommand::Load("demo-copy".to_string()))
            .unwrap();

        assert_eq!(fork.output, "forked session demo-copy");
        assert_eq!(clear.output, "transcript cleared");
        assert_eq!(load.output, "loaded session demo-copy");
        assert_eq!(app.transcript().len(), 2);
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
    fn shell_autocomplete_cycles_and_accepts_commands() {
        let mut shell = TuiShellState::default();
        for ch in "/s".chars() {
            shell.push_input_char(ch);
        }

        let matches = shell
            .autocomplete
            .as_ref()
            .expect("autocomplete should open")
            .matches
            .clone();
        assert!(matches.contains(&"/save".to_string()));
        assert!(matches.contains(&"/sessions".to_string()));
        assert!(matches.contains(&"/status".to_string()));

        shell.cycle_autocomplete();
        assert!(shell.accept_autocomplete());
        assert!(shell.input().starts_with("/s"));
    }

    #[test]
    fn shell_multiline_and_escape_precedence_behave_as_expected() {
        let mut shell = TuiShellState::default();
        shell.push_input_char('/');
        shell.push_input_char('s');
        assert!(shell.autocomplete.is_some());
        shell.handle_escape();
        assert!(shell.autocomplete.is_none());

        shell.insert_newline();
        shell.push_input_char('x');
        assert_eq!(shell.input(), "/s\nx");
        shell.handle_escape();
        assert_eq!(shell.input(), "");
        assert!(!shell.should_quit());

        shell.handle_escape();
        assert!(shell.should_quit());
    }

    #[test]
    fn shell_session_panel_tracks_layout_and_dirty_state() {
        let mut shell = TuiShellState::default();
        shell.show_tab(SidePanelTab::Sessions, PANEL_SPLIT_WIDTH);
        assert!(shell.side_panel_visible);
        assert!(!shell.overlay_visible);
        assert!(matches!(shell.focus, FocusTarget::Sessions));

        shell.show_tab(SidePanelTab::Sessions, PANEL_SPLIT_WIDTH - 1);
        assert!(!shell.side_panel_visible);
        assert!(shell.overlay_visible);

        shell.transcript.push(UiLine {
            role: "user".to_string(),
            content: "hello".to_string(),
        });
        assert!(shell.is_dirty());
        shell.mark_loaded_session(Some("demo".to_string()));
        assert!(!shell.is_dirty());
        shell.transcript.push(UiLine {
            role: "assistant".to_string(),
            content: "reply".to_string(),
        });
        assert!(shell.is_dirty());
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
    fn shell_submit_opens_sessions_panel() {
        let socket_path = std::env::temp_dir().join(format!(
            "oberon4-tui-shell-sessions-test-{}.sock",
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
        let mut shell = TuiShellState {
            input: "/sessions".to_string(),
            ..TuiShellState::default()
        };
        shell.submit(&mut app).unwrap();

        assert_eq!(shell.status(), "sessions panel opened");
        assert!(matches!(shell.side_panel_tab, SidePanelTab::Sessions));

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

        let pending = start_async_submit(&mut app, &mut shell)
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
        assert_eq!(shell.transcript()[2].role, "assistant");
        assert_eq!(shell.transcript()[2].content, "...");

        shell.push_input_char('x');

        let mut pending = Some(pending);
        loop {
            let event = pending
                .as_ref()
                .unwrap()
                .receiver
                .recv_timeout(Duration::from_secs(2))
                .expect("worker should finish");
            handle_async_ui_event(event, &mut pending, &mut app, &mut shell);
            if pending.is_none() {
                break;
            }
        }

        assert_eq!(shell.input(), "x");
        assert_eq!(shell.status(), "daemon reply");
        assert_eq!(app.transcript().len(), 2);
        assert_eq!(shell.transcript()[0].role, "user");
        assert_eq!(shell.transcript()[2].role, "assistant");
        assert_eq!(shell.transcript()[2].content, "daemon reply");

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

        let mut app = app;
        let pending = start_async_submit(&mut app, &mut shell)
            .unwrap()
            .expect("submission should start");
        let result = pending
            .receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("worker should finish");

        match result {
            AsyncUiEvent::Failed(err) => {
                assert!(!err.to_string().is_empty());
            }
            other => panic!("expected failure event, got {other:?}"),
        }

        assert_eq!(shell.transcript()[0].role, "user");
        assert_eq!(shell.transcript()[0].content, "hello");

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
            assistant_line_index: 0,
            tool_line_indices: BTreeMap::new(),
            tool_state: BTreeMap::new(),
        });
        let mut shell = TuiShellState::default();
        shell.transcript.push(UiLine {
            role: "assistant".to_string(),
            content: "...".to_string(),
        });

        handle_pending_timeout(&mut pending, &mut shell);

        assert!(pending.is_none());
        assert_eq!(shell.status(), "request timed out");
        assert_eq!(shell.transcript()[0].role, "error");
    }

    #[test]
    fn resolves_workspace_cargo_fallback_when_daemon_binary_is_missing() {
        let root =
            std::env::temp_dir().join(format!("oberon4-tui-launcher-{}", std::process::id()));
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
