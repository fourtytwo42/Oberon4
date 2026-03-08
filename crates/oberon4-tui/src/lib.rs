use std::{
    collections::BTreeMap,
    io::{self, BufRead, BufReader, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crossterm::{
    cursor::{MoveTo, MoveToColumn},
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        MouseEventKind,
    },
    execute,
    terminal::{
        Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};
use oberon4_auth::AccountRegistry;
use oberon4_core::{
    AppConfig, ChatMessage, ChatProvider, ChatRole, ChatToolCallDelta, InMemoryConfigStore,
    InMemorySessionStore, OAuthProviderKind, OberonError, SavedSession, SessionId, SessionStore,
    SessionSummary, StoredAccount,
};
use oberon4_daemon::{DaemonIpcClient, DaemonRuntime, LifecycleState};
use oberon4_provider_openai::{FakeTransport, OpenAiCompatibleClient, RetryPolicy};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Clear as WidgetClear, List, ListItem, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Wrap,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiCommand {
    Help,
    Quit,
    Stop,
    New,
    Status,
    Sessions,
    Settings,
    Accounts,
    Diagnostics,
    More,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthConnectStart {
    pub provider: OAuthProviderKind,
    pub flow_id: String,
    pub url: String,
    pub redirect_uri: String,
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
enum FocusTarget {
    Composer,
    Menu,
    List,
    Form,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Workspace {
    Chat,
    Sessions,
    Settings,
    Status,
    Help,
    Accounts,
    Diagnostics,
    Reserved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AccountActionMode {
    Manage,
    Activate,
    Disable,
    Disconnect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormAction {
    Model,
    Endpoint,
    SystemPrompt,
    RequestTimeout,
    MaxRetries,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionSidebarItem {
    CurrentChat,
    Saved(usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct SessionPickerState {
    query: String,
    filtered_items: Vec<SessionSidebarItem>,
    selected: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScreenRoute {
    ChatRoot,
    SessionsRoot,
    SettingsRoot,
    SettingsProvider,
    SettingsOpenAiOAuth,
    SettingsAnthropicOAuth,
    SettingsModelForm,
    SettingsEndpointForm,
    SettingsSystemPromptForm,
    SettingsRuntime,
    SettingsRuntimeTimeoutForm,
    SettingsRuntimeRetriesForm,
    StatusRoot,
    HelpRoot,
    AccountsRoot,
    AccountList {
        provider: OAuthProviderKind,
        action: AccountActionMode,
    },
    AccountDetail {
        provider: OAuthProviderKind,
    },
    DiagnosticsRoot,
    ReservedRoot,
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
    input_history: Vec<String>,
    input_history_index: Option<usize>,
    input_history_scratch: Option<String>,
    status: String,
    activity: ActivityState,
    should_quit: bool,
    transcript: Vec<UiLine>,
    workspace: Workspace,
    screen_stack: Vec<ScreenRoute>,
    focus: FocusTarget,
    sessions: Vec<SessionSummary>,
    session_index: usize,
    session_picker: SessionPickerState,
    accounts: Vec<StoredAccount>,
    account_index: usize,
    menu_index: usize,
    current_session_id: Option<String>,
    current_session_title: Option<String>,
    baseline_signature: Option<String>,
    status_snapshot: StatusSnapshot,
    config_snapshot: AppConfig,
    form_action: Option<FormAction>,
    diagnostics_endpoint_health: String,
    autocomplete: Option<AutocompleteState>,
    render_tick: u64,
    transcript_scroll: u16,
    transcript_follow: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TuiLaunchOptions {
    pub fullscreen: bool,
}

impl Default for TuiLaunchOptions {
    fn default() -> Self {
        Self { fullscreen: true }
    }
}

const FOOTER_HEIGHT: u16 = 5;
const CODE_BLOCK_BG: Color = Color::Rgb(28, 32, 40);
const CODE_BLOCK_FG: Color = Color::Rgb(185, 220, 255);
const COMMAND_CATALOG: [(&str, bool); 18] = [
    ("/help", false),
    ("/quit", false),
    ("/stop", false),
    ("/new", false),
    ("/status", false),
    ("/sessions", false),
    ("/settings", false),
    ("/accounts", false),
    ("/diagnostics", false),
    ("/more", false),
    ("/config", false),
    ("/model", true),
    ("/endpoint", true),
    ("/system", true),
    ("/clear", false),
    ("/save", true),
    ("/load", true),
    ("/fork", true),
];
const EXTENDED_HELP: &str = "/help /quit /stop /new /status /sessions /settings /accounts /diagnostics /more /config /model <name> /endpoint <url> /system <text> /clear /save <name> /load <name> /fork <name>";
const SESSION_SIDEBAR_WIDTH: u16 = 18;
const SESSION_VISIBLE_WIDTH: usize = 12;

struct PendingSubmission {
    receiver: Receiver<AsyncUiEvent>,
    started_at: Instant,
    timeout: Duration,
    assistant_line_index: usize,
    tool_line_indices: BTreeMap<usize, usize>,
    tool_state: BTreeMap<usize, (String, String)>,
}

struct PendingOAuthConnect {
    receiver: Receiver<OAuthUiEvent>,
    cancel_flag: Arc<AtomicBool>,
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

#[derive(Debug)]
enum OAuthUiEvent {
    Complete(String),
    Failed(OberonError),
}

enum MenuAction {
    None,
    StartOAuth(PendingOAuthConnect),
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
            input_history: Vec::new(),
            input_history_index: None,
            input_history_scratch: None,
            status: "ready".to_string(),
            activity: ActivityState::Idle,
            should_quit: false,
            transcript: Vec::new(),
            workspace: Workspace::Chat,
            screen_stack: vec![ScreenRoute::ChatRoot],
            focus: FocusTarget::Composer,
            sessions: Vec::new(),
            session_index: 0,
            session_picker: SessionPickerState::default(),
            accounts: Vec::new(),
            account_index: 0,
            menu_index: 0,
            current_session_id: None,
            current_session_title: None,
            baseline_signature: None,
            status_snapshot: StatusSnapshot::default(),
            config_snapshot: AppConfig::default(),
            form_action: None,
            diagnostics_endpoint_health: "unknown".to_string(),
            autocomplete: None,
            render_tick: 0,
            transcript_scroll: 0,
            transcript_follow: true,
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
            TuiCommand::Stop => Ok(TuiResponse {
                output: "no request in flight".to_string(),
                should_quit: false,
            }),
            TuiCommand::New => {
                self.transcript.clear();
                Ok(TuiResponse {
                    output: "new session opened".to_string(),
                    should_quit: false,
                })
            }
            TuiCommand::Sessions => Ok(TuiResponse {
                output: "sessions focused".to_string(),
                should_quit: false,
            }),
            TuiCommand::Settings => Ok(TuiResponse {
                output: "settings workspace opened".to_string(),
                should_quit: false,
            }),
            TuiCommand::Accounts => Ok(TuiResponse {
                output: "accounts workspace opened".to_string(),
                should_quit: false,
            }),
            TuiCommand::Diagnostics => Ok(TuiResponse {
                output: "diagnostics workspace opened".to_string(),
                should_quit: false,
            }),
            TuiCommand::More => Ok(TuiResponse {
                output: "more workspace opened".to_string(),
                should_quit: false,
            }),
            TuiCommand::Status => {
                let status = self.runtime.status()?;
                Ok(TuiResponse {
                    output: format!(
                        "running={} request_endpoint={} model={} oberon4_account={}",
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
            TuiCommand::Stop => Ok(TuiResponse {
                output: "no request in flight".to_string(),
                should_quit: false,
            }),
            TuiCommand::New => {
                self.transcript.clear();
                Ok(TuiResponse {
                    output: "new session opened".to_string(),
                    should_quit: false,
                })
            }
            TuiCommand::Sessions => Ok(TuiResponse {
                output: "sessions focused".to_string(),
                should_quit: false,
            }),
            TuiCommand::Settings => Ok(TuiResponse {
                output: "settings workspace opened".to_string(),
                should_quit: false,
            }),
            TuiCommand::Accounts => Ok(TuiResponse {
                output: "accounts workspace opened".to_string(),
                should_quit: false,
            }),
            TuiCommand::Diagnostics => Ok(TuiResponse {
                output: "diagnostics workspace opened".to_string(),
                should_quit: false,
            }),
            TuiCommand::More => Ok(TuiResponse {
                output: "more workspace opened".to_string(),
                should_quit: false,
            }),
            TuiCommand::Status => {
                match self.client.send(&oberon4_core::DaemonCommand::GetStatus)? {
                    oberon4_core::DaemonEvent::Status { status } => Ok(TuiResponse {
                        output: format!(
                            "running={} request_endpoint={} model={} oberon4_account={}",
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

    pub fn delete_session(&self, session_id: &str) -> Result<String, OberonError> {
        match self.client.send(&oberon4_core::DaemonCommand::DeleteSession {
            session_id: session_id.to_string(),
        })? {
            oberon4_core::DaemonEvent::Ack { message } => Ok(message),
            event => Err(protocol_error_inner(event)),
        }
    }

    pub fn preview_session(&self, session_id: &str) -> Result<SavedSession, OberonError> {
        match self
            .client
            .send(&oberon4_core::DaemonCommand::LoadSession {
                session_id: session_id.to_string(),
            })? {
            oberon4_core::DaemonEvent::SessionLoaded { session } => Ok(session),
            event => Err(protocol_error_inner(event)),
        }
    }

    pub fn list_accounts(&self) -> Result<Vec<StoredAccount>, OberonError> {
        match self
            .client
            .send(&oberon4_core::DaemonCommand::ListAccounts)?
        {
            oberon4_core::DaemonEvent::AccountsListed { mut accounts } => {
                accounts.sort_by(|left, right| {
                    left.provider
                        .cmp(&right.provider)
                        .then_with(|| left.label.cmp(&right.label))
                        .then_with(|| left.id.cmp(&right.id))
                });
                Ok(accounts)
            }
            event => Err(protocol_error_inner(event)),
        }
    }

    pub fn connect_account(
        &self,
        provider: OAuthProviderKind,
    ) -> Result<OAuthConnectStart, OberonError> {
        match self
            .client
            .send(&oberon4_core::DaemonCommand::ConnectAccount { provider })?
        {
            oberon4_core::DaemonEvent::OAuthStarted {
                provider,
                flow_id,
                url,
                redirect_uri,
            } => Ok(OAuthConnectStart {
                provider,
                flow_id,
                url,
                redirect_uri,
            }),
            event => Err(protocol_error_inner(event)),
        }
    }

    pub fn complete_oauth_connect(
        &self,
        flow_id: &str,
        callback: oberon4_core::OAuthCallbackPayload,
    ) -> Result<String, OberonError> {
        match self
            .client
            .send(&oberon4_core::DaemonCommand::CompleteOAuthConnect {
                flow_id: flow_id.to_string(),
                callback,
            })? {
            oberon4_core::DaemonEvent::Ack { message } => Ok(message),
            event => Err(protocol_error_inner(event)),
        }
    }

    pub fn activate_account(
        &self,
        provider: OAuthProviderKind,
        account_id: &str,
    ) -> Result<String, OberonError> {
        match self
            .client
            .send(&oberon4_core::DaemonCommand::ActivateAccount {
                provider,
                account_id: account_id.to_string(),
            })? {
            oberon4_core::DaemonEvent::Ack { message } => Ok(message),
            event => Err(protocol_error_inner(event)),
        }
    }

    pub fn disable_account(
        &self,
        provider: OAuthProviderKind,
        account_id: &str,
    ) -> Result<String, OberonError> {
        match self
            .client
            .send(&oberon4_core::DaemonCommand::DisableAccount {
                provider,
                account_id: account_id.to_string(),
            })? {
            oberon4_core::DaemonEvent::Ack { message } => Ok(message),
            event => Err(protocol_error_inner(event)),
        }
    }

    pub fn disconnect_account(
        &self,
        provider: OAuthProviderKind,
        account_id: &str,
    ) -> Result<String, OberonError> {
        match self
            .client
            .send(&oberon4_core::DaemonCommand::DisconnectAccount {
                provider,
                account_id: account_id.to_string(),
            })? {
            oberon4_core::DaemonEvent::Ack { message } => Ok(message),
            event => Err(protocol_error_inner(event)),
        }
    }

    pub fn endpoint_health(&self) -> Result<String, OberonError> {
        match self
            .client
            .send(&oberon4_core::DaemonCommand::EndpointHealth)?
        {
            oberon4_core::DaemonEvent::Ack { message } => Ok(message),
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
    fn ui_lines_from_messages(messages: &[ChatMessage]) -> Vec<UiLine> {
        messages
            .iter()
            .map(|message| UiLine {
                role: role_label(message.role).to_string(),
                content: message.content.clone(),
            })
            .collect()
    }

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
        self.clear_history_navigation_if_active();
        self.input.push(ch);
        self.focus = FocusTarget::Composer;
        self.refresh_autocomplete();
    }

    pub fn backspace(&mut self) {
        self.clear_history_navigation_if_active();
        let _ = self.input.pop();
        self.refresh_autocomplete();
    }

    pub fn insert_newline(&mut self) {
        self.clear_history_navigation_if_active();
        self.input.push('\n');
        self.refresh_autocomplete();
    }

    fn push_input_history_entry(&mut self, value: &str) {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return;
        }
        self.input_history.push(trimmed.to_string());
        self.reset_input_history_navigation();
    }

    fn reset_input_history_navigation(&mut self) {
        self.input_history_index = None;
        self.input_history_scratch = None;
    }

    fn clear_history_navigation_if_active(&mut self) {
        if self.input_history_index.is_some() {
            self.input_history_index = None;
            self.input_history_scratch = None;
        }
    }

    fn recall_input_history_older(&mut self) -> bool {
        if self.input_history.is_empty() {
            return false;
        }
        let next_index = match self.input_history_index {
            Some(index) => index.saturating_sub(1),
            None => {
                self.input_history_scratch = Some(self.input.clone());
                self.input_history.len().saturating_sub(1)
            }
        };
        self.input_history_index = Some(next_index);
        self.input = self.input_history[next_index].clone();
        self.refresh_autocomplete();
        true
    }

    fn recall_input_history_newer(&mut self) -> bool {
        let Some(index) = self.input_history_index else {
            return false;
        };
        if index + 1 < self.input_history.len() {
            let next_index = index + 1;
            self.input_history_index = Some(next_index);
            self.input = self.input_history[next_index].clone();
        } else {
            self.input_history_index = None;
            self.input = self.input_history_scratch.take().unwrap_or_default();
        }
        self.refresh_autocomplete();
        true
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

    fn current_route(&self) -> ScreenRoute {
        self.screen_stack
            .last()
            .copied()
            .unwrap_or(ScreenRoute::ChatRoot)
    }

    fn workspace_title(&self) -> &'static str {
        match self.workspace {
            Workspace::Chat => "Chat",
            Workspace::Sessions => "Sessions",
            Workspace::Settings => "Settings",
            Workspace::Status => "Status",
            Workspace::Help => "Help",
            Workspace::Accounts => "Accounts",
            Workspace::Diagnostics => "Diagnostics",
            Workspace::Reserved => "More",
        }
    }

    fn root_route(workspace: Workspace) -> ScreenRoute {
        match workspace {
            Workspace::Chat => ScreenRoute::ChatRoot,
            Workspace::Sessions => ScreenRoute::SessionsRoot,
            Workspace::Settings => ScreenRoute::SettingsRoot,
            Workspace::Status => ScreenRoute::StatusRoot,
            Workspace::Help => ScreenRoute::HelpRoot,
            Workspace::Accounts => ScreenRoute::AccountsRoot,
            Workspace::Diagnostics => ScreenRoute::DiagnosticsRoot,
            Workspace::Reserved => ScreenRoute::ReservedRoot,
        }
    }

    fn set_workspace(&mut self, workspace: Workspace) {
        self.workspace = workspace;
        self.screen_stack = vec![Self::root_route(workspace)];
        self.menu_index = 0;
        self.form_action = None;
        self.focus = match workspace {
            Workspace::Chat => FocusTarget::Composer,
            Workspace::Sessions => FocusTarget::List,
            Workspace::Status
            | Workspace::Help
            | Workspace::Diagnostics
            | Workspace::Reserved
            | Workspace::Settings
            | Workspace::Accounts => FocusTarget::Menu,
        };
    }

    fn push_route(&mut self, route: ScreenRoute, focus: FocusTarget) {
        self.screen_stack.push(route);
        self.focus = focus;
        self.menu_index = 0;
        self.form_action = match route {
            ScreenRoute::SettingsModelForm => Some(FormAction::Model),
            ScreenRoute::SettingsEndpointForm => Some(FormAction::Endpoint),
            ScreenRoute::SettingsSystemPromptForm => Some(FormAction::SystemPrompt),
            ScreenRoute::SettingsRuntimeTimeoutForm => Some(FormAction::RequestTimeout),
            ScreenRoute::SettingsRuntimeRetriesForm => Some(FormAction::MaxRetries),
            _ => None,
        };
    }

    fn route_for_command(command: &TuiCommand) -> Option<Workspace> {
        match command {
            TuiCommand::Help => Some(Workspace::Help),
            TuiCommand::Status => Some(Workspace::Status),
            TuiCommand::Sessions => Some(Workspace::Sessions),
            TuiCommand::Settings | TuiCommand::Config => Some(Workspace::Settings),
            TuiCommand::Accounts => Some(Workspace::Accounts),
            TuiCommand::Diagnostics => Some(Workspace::Diagnostics),
            TuiCommand::More => Some(Workspace::Reserved),
            _ => None,
        }
    }

    fn overlay_open(&self) -> bool {
        !matches!(self.workspace, Workspace::Chat)
    }

    fn close_overlay(&mut self) {
        self.workspace = Workspace::Chat;
        self.screen_stack = vec![ScreenRoute::ChatRoot];
        self.menu_index = 0;
        self.form_action = None;
        self.focus = FocusTarget::Composer;
    }

    fn handle_escape(&mut self) {
        if self.autocomplete.is_some() {
            self.autocomplete = None;
            return;
        }
        if matches!(self.focus, FocusTarget::List) && matches!(self.workspace, Workspace::Chat) {
            self.focus = FocusTarget::Composer;
            self.status = "returned to composer".to_string();
            return;
        }
        if matches!(self.focus, FocusTarget::Form) && !self.input.is_empty() {
            self.input.clear();
            self.status = "form cleared".to_string();
            return;
        }
        if self.screen_stack.len() > 1 {
            self.screen_stack.pop();
            self.form_action = None;
            self.focus = match self.current_route() {
                ScreenRoute::ChatRoot => FocusTarget::Composer,
                ScreenRoute::SessionsRoot | ScreenRoute::AccountList { .. } => FocusTarget::List,
                _ => FocusTarget::Menu,
            };
            return;
        }
        if !matches!(self.workspace, Workspace::Chat) {
            self.close_overlay();
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

    fn should_accept_autocomplete_on_enter(&self) -> bool {
        let trimmed = self.input.trim();
        let Some(state) = self.autocomplete.as_ref() else {
            return false;
        };
        let Some(selected) = state.matches.get(state.selected) else {
            return false;
        };
        trimmed != selected || command_requires_argument(selected)
    }

    fn move_selection(current: &mut usize, len: usize, delta: isize) {
        if len == 0 {
            return;
        }
        let next = (*current as isize + delta).clamp(0, len as isize - 1);
        *current = next as usize;
    }

    pub fn move_session_selection(&mut self, delta: isize) {
        let len = self.session_sidebar_items().len();
        Self::move_selection(&mut self.session_index, len, delta);
    }

    pub fn move_session_picker_selection(&mut self, delta: isize) {
        let len = self.session_picker.filtered_items.len();
        Self::move_selection(&mut self.session_picker.selected, len, delta);
    }

    pub fn move_account_selection(&mut self, delta: isize) {
        let len = self
            .accounts_for_current_route()
            .map(|accounts| accounts.len())
            .unwrap_or(0);
        Self::move_selection(&mut self.account_index, len, delta);
    }

    pub fn move_menu_selection(&mut self, delta: isize) {
        let len = self.menu_items().len();
        Self::move_selection(&mut self.menu_index, len, delta);
    }

    fn set_activity(&mut self, activity: ActivityState, status: impl Into<String>) {
        self.activity = activity;
        self.status = status.into();
    }

    fn scroll_transcript_by(&mut self, delta: i32) {
        self.transcript_follow = false;
        let next = (self.transcript_scroll as i32 + delta).max(0) as u16;
        self.transcript_scroll = next;
    }

    fn scroll_transcript_to_top(&mut self) {
        self.transcript_follow = false;
        self.transcript_scroll = 0;
    }

    fn scroll_transcript_to_bottom(&mut self) {
        self.transcript_follow = true;
    }

    pub fn refresh_runtime_data(&mut self, app: &DaemonBackedTuiApp) {
        if let Ok(snapshot) = app.load_status_snapshot() {
            self.status_snapshot = snapshot;
        }
        if let Ok(sessions) = app.list_sessions() {
            self.sessions = sessions;
            if self.session_index >= self.sessions.len() {
                self.session_index = self.sessions.len().saturating_sub(1);
            }
        }
        if let Ok(accounts) = app.list_accounts() {
            self.accounts = accounts;
        }
        if let Ok(config) = app.load_config() {
            self.config_snapshot = config;
        }
        if let Ok(health) = app.endpoint_health() {
            self.diagnostics_endpoint_health = health;
        }
        if matches!(self.workspace, Workspace::Sessions) {
            self.refresh_session_picker_matches();
            if self.session_picker.query.is_empty() {
                self.select_current_session_in_picker();
            }
        }
    }

    pub fn mark_loaded_session(
        &mut self,
        session_id: Option<String>,
        session_title: Option<String>,
    ) {
        self.current_session_id = session_id;
        self.current_session_title = session_title;
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

    fn menu_items(&self) -> Vec<&'static str> {
        match self.current_route() {
            ScreenRoute::SettingsRoot => vec![
                "Provider",
                "OpenAI OAuth",
                "Anthropic OAuth",
                "Model",
                "Endpoint",
                "System Prompt",
                "Runtime",
                "Back to Chat",
            ],
            ScreenRoute::SettingsProvider => vec!["OpenAI-compatible", "Back"],
            ScreenRoute::SettingsOpenAiOAuth | ScreenRoute::SettingsAnthropicOAuth => vec![
                "Connect",
                "List Accounts",
                "Activate Account",
                "Disable Account",
                "Disconnect Account",
                "Back",
            ],
            ScreenRoute::SettingsRuntime => {
                vec!["Request Timeout", "Max Retries", "Toggle Streaming", "Back"]
            }
            ScreenRoute::AccountsRoot => vec!["OpenAI Accounts", "Anthropic Accounts", "Back"],
            ScreenRoute::AccountDetail { .. } => {
                vec!["Activate", "Disable", "Disconnect", "Back"]
            }
            _ => Vec::new(),
        }
    }

    fn accounts_for_current_route(&self) -> Option<Vec<&StoredAccount>> {
        let provider = match self.current_route() {
            ScreenRoute::AccountList { provider, .. } | ScreenRoute::AccountDetail { provider } => {
                provider
            }
            _ => return None,
        };
        Some(
            self.accounts
                .iter()
                .filter(|account| account.provider == provider)
                .collect(),
        )
    }

    fn current_account(&self) -> Option<&StoredAccount> {
        let accounts = self.accounts_for_current_route()?;
        accounts.get(self.account_index).copied()
    }

    fn open_route_for_command(&mut self, command: &TuiCommand) {
        match command {
            TuiCommand::Sessions => {
                self.workspace = Workspace::Chat;
                self.screen_stack = vec![ScreenRoute::ChatRoot];
                self.focus = FocusTarget::List;
            }
            _ => {
                if let Some(workspace) = Self::route_for_command(command) {
                    self.set_workspace(workspace);
                }
            }
        }
    }

    fn open_form(&mut self, action: FormAction, value: String, route: ScreenRoute) {
        self.input = value;
        self.autocomplete = None;
        self.push_route(route, FocusTarget::Form);
        self.form_action = Some(action);
    }

    fn session_title_for_header(&self) -> &str {
        self.current_session_title
            .as_deref()
            .or(self.current_session_id.as_deref())
            .unwrap_or("unsaved")
    }

    fn current_chat_sidebar_title(&self) -> String {
        self.current_session_title
            .clone()
            .or_else(|| self.current_session_id.clone())
            .unwrap_or_else(|| "current chat".to_string())
    }

    fn session_sidebar_items(&self) -> Vec<SessionSidebarItem> {
        let mut items = vec![SessionSidebarItem::CurrentChat];
        for (index, session) in self.sessions.iter().enumerate() {
            if self.current_session_id.as_deref() == Some(session.id.as_str()) {
                continue;
            }
            items.push(SessionSidebarItem::Saved(index));
        }
        items
    }

    fn session_picker_search_text(&self, item: SessionSidebarItem) -> String {
        match item {
            SessionSidebarItem::CurrentChat => format!(
                "{} {} current",
                self.current_chat_sidebar_title(),
                self.current_session_id.as_deref().unwrap_or_default()
            ),
            SessionSidebarItem::Saved(index) => self
                .sessions
                .get(index)
                .map(|session| {
                    format!(
                        "{} {} {}",
                        session.title,
                        session.id.as_str(),
                        session.model
                    )
                })
                .unwrap_or_default(),
        }
    }

    #[allow(dead_code)]
    fn refresh_session_picker_matches(&mut self) {
        let mut scored = self
            .session_sidebar_items()
            .into_iter()
            .filter_map(|item| {
                session_match_score(
                    self.session_picker.query.trim(),
                    &self.session_picker_search_text(item),
                )
                .map(|score| (score, item))
            })
            .collect::<Vec<_>>();
        scored.sort_by(|(left_score, left_item), (right_score, right_item)| {
            left_score
                .cmp(right_score)
                .then_with(|| self.session_picker_label(*left_item).cmp(&self.session_picker_label(*right_item)))
        });
        self.session_picker.filtered_items = scored.into_iter().map(|(_, item)| item).collect();
        if self.session_picker.filtered_items.is_empty() {
            self.session_picker.selected = 0;
        } else if self.session_picker.selected >= self.session_picker.filtered_items.len() {
            self.session_picker.selected = self.session_picker.filtered_items.len().saturating_sub(1);
        }
    }

    #[allow(dead_code)]
    fn select_current_session_in_picker(&mut self) {
        self.session_picker.selected = self
            .session_picker
            .filtered_items
            .iter()
            .position(|item| matches!(item, SessionSidebarItem::CurrentChat))
            .unwrap_or(0);
    }

    #[allow(dead_code)]
    fn clear_session_picker_query(&mut self) {
        self.session_picker.query.clear();
        self.refresh_session_picker_matches();
        self.select_current_session_in_picker();
    }

    #[allow(dead_code)]
    fn session_picker_label(&self, item: SessionSidebarItem) -> String {
        match item {
            SessionSidebarItem::CurrentChat => format!("{} (current)", self.current_chat_sidebar_title()),
            SessionSidebarItem::Saved(index) => self
                .sessions
                .get(index)
                .map(|session| session.title.clone())
                .unwrap_or_else(|| "missing".to_string()),
        }
    }

    #[allow(dead_code)]
    fn selected_session_picker_item(&self) -> Option<SessionSidebarItem> {
        self.session_picker
            .filtered_items
            .get(self.session_picker.selected)
            .copied()
    }

    #[allow(dead_code)]
    fn update_session_picker_query(
        &mut self,
        query: String,
        app: &DaemonBackedTuiApp,
    ) -> Result<(), OberonError> {
        self.session_picker.query = query;
        self.refresh_session_picker_matches();
        if self.session_picker.filtered_items.is_empty() {
            self.restore_active_chat_preview(app);
            self.set_activity(ActivityState::Idle, "no sessions match");
        } else {
            self.preview_selected_session(app)?;
        }
        Ok(())
    }

    fn sync_loaded_session_from_id(&mut self, session_id: &str) {
        if let Some(session) = self
            .sessions
            .iter()
            .find(|session| session.id.as_str() == session_id)
        {
            self.mark_loaded_session(
                Some(session.id.as_str().to_string()),
                Some(session.title.clone()),
            );
        } else {
            self.mark_loaded_session(Some(session_id.to_string()), Some(session_id.to_string()));
        }
    }

    fn session_title_from_exchange(prompt: &str, assistant: &str) -> String {
        let source = if assistant.trim().is_empty() {
            prompt
        } else {
            assistant
        };
        let normalized = source
            .lines()
            .next()
            .unwrap_or(source)
            .replace("```", " ")
            .split_whitespace()
            .take(6)
            .collect::<Vec<_>>()
            .join(" ");
        let trimmed = normalized.trim();
        if trimmed.is_empty() {
            "New chat".to_string()
        } else {
            trimmed.to_string()
        }
    }

    fn session_storage_id(title: &str) -> String {
        let slug = title
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() {
                    ch.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect::<String>()
            .split('-')
            .filter(|segment| !segment.is_empty())
            .take(4)
            .collect::<Vec<_>>()
            .join("-");
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        if slug.is_empty() {
            format!("chat-{nonce}")
        } else {
            format!("{slug}-{nonce}")
        }
    }

    fn restore_active_chat_preview(&mut self, app: &DaemonBackedTuiApp) {
        self.transcript = Self::ui_lines_from_messages(app.transcript());
        self.scroll_transcript_to_bottom();
    }

    fn preview_selected_session(
        &mut self,
        app: &DaemonBackedTuiApp,
    ) -> Result<(), OberonError> {
        let selected_item = if matches!(self.workspace, Workspace::Sessions) {
            self.selected_session_picker_item()
        } else {
            self.session_sidebar_items().get(self.session_index).copied()
        };
        let Some(item) = selected_item else {
            return Ok(());
        };

        match item {
            SessionSidebarItem::CurrentChat => {
                self.restore_active_chat_preview(app);
                self.set_activity(ActivityState::Idle, "previewing current chat");
            }
            SessionSidebarItem::Saved(index) => {
                let Some(session) = self.sessions.get(index) else {
                    return Ok(());
                };
                let session_id = session.id.as_str().to_string();
                let session_title = session.title.clone();
                let preview = app.preview_session(&session_id)?;
                self.transcript = Self::ui_lines_from_messages(&preview.messages);
                self.scroll_transcript_to_bottom();
                self.set_activity(ActivityState::Idle, format!("previewing {}", session_title));
            }
        }

        Ok(())
    }

    fn autosave_current_session(
        &mut self,
        app: &mut DaemonBackedTuiApp,
        prompt: &str,
        assistant: &str,
    ) -> Result<(), OberonError> {
        let (session_id, session_title) = match (
            self.current_session_id.clone(),
            self.current_session_title.clone(),
        ) {
            (Some(id), Some(title)) => (id, title),
            (Some(id), None) => (id.clone(), id),
            (None, _) => {
                let title = Self::session_title_from_exchange(prompt, assistant);
                let id = Self::session_storage_id(&title);
                (id, title)
            }
        };

        let session = SavedSession {
            summary: SessionSummary {
                id: SessionId::new(session_id.clone()),
                title: session_title.clone(),
                provider: oberon4_core::ChatProviderKind::OpenAiCompatible,
                model: app.load_config()?.endpoint.model,
            },
            messages: app.transcript.clone(),
        };

        match app
            .client
            .send(&oberon4_core::DaemonCommand::SaveSession { session })?
        {
            oberon4_core::DaemonEvent::SessionLoaded { .. } => {
                self.mark_loaded_session(Some(session_id), Some(session_title));
                Ok(())
            }
            event => Err(protocol_error_inner(event)),
        }
    }

    fn apply_form(&mut self, app: &mut DaemonBackedTuiApp) -> Result<bool, OberonError> {
        let value = self.input.trim().to_string();
        let Some(action) = self.form_action else {
            return Ok(false);
        };
        match action {
            FormAction::Model if !value.is_empty() => {
                app.handle_input(&format!("/model {value}"))?;
                self.config_snapshot.endpoint.model = value.clone();
                self.set_activity(ActivityState::Idle, format!("model set to {value}"));
            }
            FormAction::Endpoint if !value.is_empty() => {
                app.handle_input(&format!("/endpoint {value}"))?;
                self.config_snapshot.endpoint.base_url = value.clone();
                self.set_activity(ActivityState::Idle, format!("endpoint set to {value}"));
            }
            FormAction::SystemPrompt => {
                app.handle_input(&format!("/system {value}"))?;
                self.set_activity(ActivityState::Idle, "system prompt updated");
            }
            FormAction::RequestTimeout if !value.is_empty() => {
                let parsed = value.parse::<u64>().map_err(|err| {
                    OberonError::Validation(format!("invalid request timeout: {err}"))
                })?;
                let mut config = app.load_config()?;
                config.runtime.request_timeout_ms = parsed;
                app.save_config(config.clone())?;
                self.config_snapshot = config;
                self.set_activity(
                    ActivityState::Idle,
                    format!("request timeout set to {parsed}"),
                );
            }
            FormAction::MaxRetries if !value.is_empty() => {
                let parsed = value.parse::<u8>().map_err(|err| {
                    OberonError::Validation(format!("invalid max retries: {err}"))
                })?;
                let mut config = app.load_config()?;
                config.runtime.max_retries = parsed;
                app.save_config(config.clone())?;
                self.config_snapshot = config;
                self.set_activity(ActivityState::Idle, format!("max retries set to {parsed}"));
            }
            _ => {
                self.set_activity(ActivityState::Idle, "form value was empty");
            }
        }
        self.input.clear();
        self.screen_stack.pop();
        self.form_action = None;
        self.focus = FocusTarget::Menu;
        self.refresh_runtime_data(app);
        Ok(true)
    }

    fn activate_current_menu_item(
        &mut self,
        app: &mut DaemonBackedTuiApp,
    ) -> Result<MenuAction, OberonError> {
        match self.current_route() {
            ScreenRoute::SettingsRoot => match self.menu_index {
                0 => self.push_route(ScreenRoute::SettingsProvider, FocusTarget::Menu),
                1 => self.push_route(ScreenRoute::SettingsOpenAiOAuth, FocusTarget::Menu),
                2 => self.push_route(ScreenRoute::SettingsAnthropicOAuth, FocusTarget::Menu),
                3 => self.open_form(
                    FormAction::Model,
                    self.config_snapshot.endpoint.model.clone(),
                    ScreenRoute::SettingsModelForm,
                ),
                4 => self.open_form(
                    FormAction::Endpoint,
                    self.config_snapshot.endpoint.base_url.clone(),
                    ScreenRoute::SettingsEndpointForm,
                ),
                5 => self.open_form(
                    FormAction::SystemPrompt,
                    String::new(),
                    ScreenRoute::SettingsSystemPromptForm,
                ),
                6 => self.push_route(ScreenRoute::SettingsRuntime, FocusTarget::Menu),
                7 => self.set_workspace(Workspace::Chat),
                _ => {}
            },
            ScreenRoute::SettingsProvider => match self.menu_index {
                0 => self.set_activity(ActivityState::Idle, "OpenAI-compatible provider is active"),
                _ => {
                    self.screen_stack.pop();
                }
            },
            ScreenRoute::SettingsOpenAiOAuth | ScreenRoute::SettingsAnthropicOAuth => {
                let provider = if matches!(self.current_route(), ScreenRoute::SettingsOpenAiOAuth) {
                    OAuthProviderKind::OpenAi
                } else {
                    OAuthProviderKind::Anthropic
                };
                match self.menu_index {
                    0 => {
                        self.set_activity(ActivityState::Loading, "starting oauth flow");
                        return Ok(MenuAction::StartOAuth(start_async_oauth_connect(
                            app.client.socket_path().to_path_buf(),
                            provider,
                        )?));
                    }
                    1 => self.push_route(
                        ScreenRoute::AccountList {
                            provider,
                            action: AccountActionMode::Manage,
                        },
                        FocusTarget::List,
                    ),
                    2 => self.push_route(
                        ScreenRoute::AccountList {
                            provider,
                            action: AccountActionMode::Activate,
                        },
                        FocusTarget::List,
                    ),
                    3 => self.push_route(
                        ScreenRoute::AccountList {
                            provider,
                            action: AccountActionMode::Disable,
                        },
                        FocusTarget::List,
                    ),
                    4 => self.push_route(
                        ScreenRoute::AccountList {
                            provider,
                            action: AccountActionMode::Disconnect,
                        },
                        FocusTarget::List,
                    ),
                    _ => {
                        self.screen_stack.pop();
                    }
                }
            }
            ScreenRoute::SettingsRuntime => match self.menu_index {
                0 => self.open_form(
                    FormAction::RequestTimeout,
                    self.config_snapshot.runtime.request_timeout_ms.to_string(),
                    ScreenRoute::SettingsRuntimeTimeoutForm,
                ),
                1 => self.open_form(
                    FormAction::MaxRetries,
                    self.config_snapshot.runtime.max_retries.to_string(),
                    ScreenRoute::SettingsRuntimeRetriesForm,
                ),
                2 => {
                    let mut config = app.load_config()?;
                    config.runtime.stream_responses = !config.runtime.stream_responses;
                    app.save_config(config.clone())?;
                    self.config_snapshot = config.clone();
                    self.set_activity(
                        ActivityState::Idle,
                        format!(
                            "streaming {}",
                            if config.runtime.stream_responses {
                                "enabled"
                            } else {
                                "disabled"
                            }
                        ),
                    );
                }
                _ => {
                    self.screen_stack.pop();
                }
            },
            ScreenRoute::AccountsRoot => match self.menu_index {
                0 => self.push_route(
                    ScreenRoute::AccountList {
                        provider: OAuthProviderKind::OpenAi,
                        action: AccountActionMode::Manage,
                    },
                    FocusTarget::List,
                ),
                1 => self.push_route(
                    ScreenRoute::AccountList {
                        provider: OAuthProviderKind::Anthropic,
                        action: AccountActionMode::Manage,
                    },
                    FocusTarget::List,
                ),
                _ => self.set_workspace(Workspace::Chat),
            },
            ScreenRoute::AccountDetail { provider } => {
                if let Some(account_id) = self.current_account().map(|account| account.id.clone()) {
                    match self.menu_index {
                        0 => self.set_activity(
                            ActivityState::Idle,
                            app.activate_account(provider, &account_id)?,
                        ),
                        1 => self.set_activity(
                            ActivityState::Idle,
                            app.disable_account(provider, &account_id)?,
                        ),
                        2 => self.set_activity(
                            ActivityState::Idle,
                            app.disconnect_account(provider, &account_id)?,
                        ),
                        _ => {
                            self.screen_stack.pop();
                            return Ok(MenuAction::None);
                        }
                    }
                    self.refresh_runtime_data(app);
                } else {
                    self.set_activity(ActivityState::Error, "no account selected");
                }
            }
            _ => return Ok(MenuAction::None),
        }
        Ok(MenuAction::None)
    }

    fn activate_current_list_item(
        &mut self,
        app: &mut DaemonBackedTuiApp,
    ) -> Result<bool, OberonError> {
        match self.current_route() {
            ScreenRoute::SessionsRoot if matches!(self.workspace, Workspace::Sessions) => {
                if let Some(item) = self.selected_session_picker_item() {
                    match item {
                        SessionSidebarItem::CurrentChat => {
                            self.restore_active_chat_preview(app);
                            self.close_overlay();
                            self.scroll_transcript_to_bottom();
                            self.set_activity(ActivityState::Idle, "current chat selected");
                            return Ok(true);
                        }
                        SessionSidebarItem::Saved(index) => {
                            if let Some(session) = self.sessions.get(index) {
                                let name = session.id.as_str().to_string();
                                let response = app.handle_command(TuiCommand::Load(name.clone()))?;
                                self.set_activity(ActivityState::Idle, response.output);
                                self.sync_loaded_session_from_id(&name);
                                self.transcript = Self::ui_lines_from_messages(app.transcript());
                                self.refresh_runtime_data(app);
                                self.close_overlay();
                                self.scroll_transcript_to_bottom();
                                return Ok(true);
                            }
                        }
                    }
                }
            }
            ScreenRoute::ChatRoot | ScreenRoute::SessionsRoot
                if matches!(self.workspace, Workspace::Chat) =>
            {
                if let Some(item) = self
                    .session_sidebar_items()
                    .get(self.session_index)
                    .copied()
                {
                    match item {
                        SessionSidebarItem::CurrentChat => {
                            self.restore_active_chat_preview(app);
                            self.set_workspace(Workspace::Chat);
                            self.focus = FocusTarget::Composer;
                            self.scroll_transcript_to_bottom();
                            self.set_activity(ActivityState::Idle, "current chat selected");
                            return Ok(true);
                        }
                        SessionSidebarItem::Saved(index) => {
                            if let Some(session) = self.sessions.get(index) {
                                let name = session.id.as_str().to_string();
                                let response =
                                    app.handle_command(TuiCommand::Load(name.clone()))?;
                                self.set_activity(ActivityState::Idle, response.output);
                                self.sync_loaded_session_from_id(&name);
                                self.transcript = Self::ui_lines_from_messages(app.transcript());
                                self.refresh_runtime_data(app);
                                self.workspace = Workspace::Chat;
                                self.screen_stack = vec![ScreenRoute::ChatRoot];
                                self.focus = FocusTarget::Composer;
                                self.scroll_transcript_to_bottom();
                                return Ok(true);
                            }
                        }
                    }
                }
            }
            ScreenRoute::AccountList { provider, action } => {
                if let Some(account) = self.current_account().cloned() {
                    match action {
                        AccountActionMode::Manage => {
                            self.push_route(
                                ScreenRoute::AccountDetail { provider },
                                FocusTarget::Menu,
                            );
                            return Ok(true);
                        }
                        AccountActionMode::Activate => {
                            self.set_activity(
                                ActivityState::Idle,
                                app.activate_account(provider, &account.id)?,
                            );
                        }
                        AccountActionMode::Disable => {
                            self.set_activity(
                                ActivityState::Idle,
                                app.disable_account(provider, &account.id)?,
                            );
                        }
                        AccountActionMode::Disconnect => {
                            self.set_activity(
                                ActivityState::Idle,
                                app.disconnect_account(provider, &account.id)?,
                            );
                        }
                    }
                    self.refresh_runtime_data(app);
                    return Ok(true);
                }
            }
            _ => {}
        }
        Ok(false)
    }

    fn delete_selected_session(
        &mut self,
        app: &mut DaemonBackedTuiApp,
    ) -> Result<bool, OberonError> {
        if !matches!(self.workspace, Workspace::Chat) || !matches!(self.focus, FocusTarget::List) {
            return Ok(false);
        }

        let Some(item) = self.session_sidebar_items().get(self.session_index).copied() else {
            return Ok(false);
        };

        let SessionSidebarItem::Saved(index) = item else {
            self.set_activity(ActivityState::Idle, "only saved sessions can be deleted");
            return Ok(true);
        };

        let Some(session) = self.sessions.get(index) else {
            return Ok(false);
        };
        let session_id = session.id.as_str().to_string();
        let session_title = session.title.clone();
        let message = match app.delete_session(&session_id) {
            Ok(message) => message,
            Err(err) if delete_session_requires_daemon_refresh(&err) => {
                restart_daemon_process(app.client.socket_path().to_path_buf())?;
                app.delete_session(&session_id)?
            }
            Err(err) => return Err(err),
        };
        self.refresh_runtime_data(app);
        let item_count = self.session_sidebar_items().len();
        if item_count == 0 {
            self.session_index = 0;
        } else if self.session_index >= item_count {
            self.session_index = item_count.saturating_sub(1);
        }
        self.preview_selected_session(app)?;
        self.set_activity(ActivityState::Idle, format!("{message} ({session_title})"));
        Ok(true)
    }

    pub fn submit(&mut self, app: &mut DaemonBackedTuiApp) -> Result<(), OberonError> {
        let input = std::mem::take(&mut self.input);
        self.autocomplete = None;
        let trimmed = input.trim();
        if trimmed.is_empty() {
            self.set_activity(ActivityState::Idle, "Input was empty");
            return Ok(());
        }

        if !trimmed.starts_with('/') && !has_trailing_user_line(&self.transcript, trimmed) {
            self.transcript.push(UiLine {
                role: "user".to_string(),
                content: trimmed.to_string(),
            });
            self.scroll_transcript_to_bottom();
        }

        let command = if trimmed.starts_with('/') {
            match parse_tui_command(trimmed) {
                Ok(command) => Some(command),
                Err(err) => {
                    self.set_activity(ActivityState::Error, err.to_string());
                    return Ok(());
                }
            }
        } else {
            None
        };

        if let Some(command) = command.as_ref() {
            self.push_input_history_entry(trimmed);
            self.open_route_for_command(command);
            if matches!(
                command,
                TuiCommand::Help
                    | TuiCommand::Status
                    | TuiCommand::Sessions
                    | TuiCommand::Settings
                    | TuiCommand::Accounts
                    | TuiCommand::Diagnostics
                    | TuiCommand::More
                    | TuiCommand::Config
            ) {
                self.refresh_runtime_data(app);
                self.set_activity(
                    ActivityState::Idle,
                    match command {
                        TuiCommand::Help => "help overlay opened",
                        TuiCommand::Status => "status overlay opened",
                        TuiCommand::Sessions => "sessions focused",
                        TuiCommand::Settings | TuiCommand::Config => "settings overlay opened",
                        TuiCommand::Accounts => "accounts overlay opened",
                        TuiCommand::Diagnostics => "diagnostics overlay opened",
                        TuiCommand::More => "more overlay opened",
                        _ => "overlay opened",
                    },
                );
                return Ok(());
            }
        }

        self.push_input_history_entry(trimmed);
        match command {
            Some(TuiCommand::New) => self.activity = ActivityState::Idle,
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
                self.scroll_transcript_to_bottom();
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
                Some(TuiCommand::New) => {
                    self.current_session_id = None;
                    self.current_session_title = None;
                    self.baseline_signature = None;
                    self.scroll_transcript_to_bottom();
                }
                Some(TuiCommand::Save(name)) => {
                    self.mark_loaded_session(Some(name.clone()), Some(name))
                }
                Some(TuiCommand::Load(name)) => self.sync_loaded_session_from_id(&name),
                Some(TuiCommand::Clear) => {}
                _ => {}
            }
            self.refresh_runtime_data(app);
            return Ok(());
        }

        if let Some(last) = app.transcript().last() {
            self.transcript.push(UiLine {
                role: role_label(last.role).to_string(),
                content: last.content.clone(),
            });
            self.scroll_transcript_to_bottom();
        }

        self.refresh_runtime_data(app);

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SessionMatchScore {
    Substring(usize),
    WordBoundary(usize),
    Fuzzy(usize),
}

fn session_match_score(query: &str, haystack: &str) -> Option<SessionMatchScore> {
    if query.is_empty() {
        return Some(SessionMatchScore::Substring(0));
    }
    let query = query.to_ascii_lowercase();
    let haystack = haystack.to_ascii_lowercase();

    if let Some(index) = haystack.find(&query) {
        return Some(SessionMatchScore::Substring(index));
    }

    if let Some(index) = find_word_boundary_match(&haystack, &query) {
        return Some(SessionMatchScore::WordBoundary(index));
    }

    fuzzy_match_positions(&haystack, &query).map(SessionMatchScore::Fuzzy)
}

fn find_word_boundary_match(haystack: &str, query: &str) -> Option<usize> {
    haystack.match_indices(query).map(|(idx, _)| idx).find(|&idx| {
        idx == 0
            || !haystack[..idx]
                .chars()
                .last()
                .map(|ch| ch.is_ascii_alphanumeric())
                .unwrap_or(false)
    })
}

fn fuzzy_match_positions(haystack: &str, query: &str) -> Option<usize> {
    let mut score = 0usize;
    let mut query_chars = query.chars();
    let mut wanted = query_chars.next()?;
    let mut last_match = 0usize;
    let mut matched_any = false;
    for (idx, ch) in haystack.chars().enumerate() {
        if ch == wanted {
            if matched_any {
                score += idx.saturating_sub(last_match);
            } else {
                score += idx;
                matched_any = true;
            }
            last_match = idx;
            if let Some(next) = query_chars.next() {
                wanted = next;
            } else {
                return Some(score);
            }
        }
    }
    None
}

pub fn run_terminal_ui() -> Result<(), OberonError> {
    run_terminal_ui_with_options(TuiLaunchOptions::default())
}

pub fn run_terminal_ui_with_options(options: TuiLaunchOptions) -> Result<(), OberonError> {
    let mut app = DaemonBackedTuiApp::default();
    let mut shell = TuiShellState::default();
    shell.refresh_runtime_data(&app);
    if options.fullscreen {
        let terminal = init_fullscreen_terminal()?;
        let mut guard = FullscreenTerminalGuard::new(terminal);
        run_terminal_loop(guard.terminal_mut(), &mut app, &mut shell)
    } else {
        let stdout = init_inline_terminal()?;
        let mut guard = InlineTerminalGuard::new(stdout);
        run_inline_terminal_loop(guard.stdout_mut(), &mut app, &mut shell)
    }
}

struct FullscreenTerminalGuard {
    terminal: Option<Terminal<CrosstermBackend<io::Stdout>>>,
}

impl FullscreenTerminalGuard {
    fn new(terminal: Terminal<CrosstermBackend<io::Stdout>>) -> Self {
        Self {
            terminal: Some(terminal),
        }
    }

    fn terminal_mut(&mut self) -> &mut Terminal<CrosstermBackend<io::Stdout>> {
        self.terminal.as_mut().expect("terminal guard must hold terminal")
    }
}

impl Drop for FullscreenTerminalGuard {
    fn drop(&mut self) {
        if let Some(terminal) = self.terminal.as_mut() {
            let _ = restore_fullscreen_terminal(terminal);
        }
    }
}

struct InlineTerminalGuard {
    stdout: Option<io::Stdout>,
}

impl InlineTerminalGuard {
    fn new(stdout: io::Stdout) -> Self {
        Self { stdout: Some(stdout) }
    }

    fn stdout_mut(&mut self) -> &mut io::Stdout {
        self.stdout.as_mut().expect("inline terminal guard must hold stdout")
    }
}

impl Drop for InlineTerminalGuard {
    fn drop(&mut self) {
        if let Some(stdout) = self.stdout.as_mut() {
            let _ = restore_inline_terminal(stdout);
        }
    }
}

fn init_inline_terminal() -> Result<io::Stdout, OberonError> {
    sanitize_terminal_state()?;
    enable_raw_mode()
        .map_err(|err| OberonError::Storage(format!("enable raw mode failed: {err}")))?;
    Ok(io::stdout())
}

fn write_inline_line(stdout: &mut io::Stdout, line: &str) -> Result<(), OberonError> {
    stdout
        .write_all(line.as_bytes())
        .map_err(|err| OberonError::Storage(format!("inline write failed: {err}")))?;
    stdout
        .write_all(b"\r\n")
        .map_err(|err| OberonError::Storage(format!("inline write failed: {err}")))?;
    Ok(())
}

fn restore_inline_terminal(stdout: &mut io::Stdout) -> Result<(), OberonError> {
    execute!(stdout, MoveToColumn(0), Clear(ClearType::CurrentLine))
        .map_err(|err| OberonError::Storage(format!("inline terminal cleanup failed: {err}")))?;
    disable_raw_mode()
        .map_err(|err| OberonError::Storage(format!("disable raw mode failed: {err}")))?;
    Ok(())
}

fn run_inline_terminal_loop(
    stdout: &mut io::Stdout,
    app: &mut DaemonBackedTuiApp,
    shell: &mut TuiShellState,
) -> Result<(), OberonError> {
    let mut pending_oauth: Option<PendingOAuthConnect> = None;
    sync_shell_transcript_from_app(shell, app);
    print_inline_home_screen(stdout, shell)?;
    redraw_inline_prompt(stdout, shell)?;

    loop {
        if shell.should_quit() {
            writeln!(stdout).map_err(|err| {
                OberonError::Storage(format!("inline terminal write failed: {err}"))
            })?;
            stdout
                .flush()
                .map_err(|err| OberonError::Storage(format!("stdout flush failed: {err}")))?;
            return Ok(());
        }

        if let Some(pending) = pending_oauth.as_ref() {
            match pending.receiver.try_recv() {
                Ok(OAuthUiEvent::Complete(message)) => {
                    pending_oauth = None;
                    shell.refresh_runtime_data(app);
                    shell.set_activity(ActivityState::Idle, message);
                }
                Ok(OAuthUiEvent::Failed(err)) => {
                    pending_oauth = None;
                    shell.set_activity(ActivityState::Error, err.to_string());
                    shell.refresh_runtime_data(app);
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    pending_oauth = None;
                    shell.set_activity(ActivityState::Error, "oauth flow worker disconnected");
                }
            }
        }

        let event = event::read()
            .map_err(|err| OberonError::Storage(format!("tui event read failed: {err}")))?;
        let Event::Key(key) = event else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        let mut rerender_workspace = false;
        let previous_workspace = shell.workspace;
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                shell.should_quit = true;
                shell.set_activity(ActivityState::Idle, "quitting");
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                shell.set_workspace(Workspace::Sessions);
                shell.refresh_runtime_data(app);
                shell.set_activity(ActivityState::Idle, "sessions opened");
                rerender_workspace = true;
            }
            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if matches!(shell.focus, FocusTarget::Composer | FocusTarget::Form) {
                    shell.insert_newline();
                }
            }
            KeyCode::Char(ch) => {
                if matches!(shell.workspace, Workspace::Sessions)
                    && matches!(shell.current_route(), ScreenRoute::SessionsRoot)
                {
                    let mut query = shell.session_picker.query.clone();
                    query.push(ch);
                    shell.update_session_picker_query(query, app)?;
                    rerender_workspace = true;
                } else if matches!(shell.focus, FocusTarget::Composer | FocusTarget::Form) {
                    shell.push_input_char(ch);
                }
            }
            KeyCode::Backspace => {
                if matches!(shell.workspace, Workspace::Sessions)
                    && matches!(shell.current_route(), ScreenRoute::SessionsRoot)
                {
                    let mut query = shell.session_picker.query.clone();
                    query.pop();
                    shell.update_session_picker_query(query, app)?;
                    rerender_workspace = true;
                } else if matches!(shell.focus, FocusTarget::Composer | FocusTarget::Form) {
                    shell.backspace();
                }
            }
            KeyCode::Tab if matches!(shell.focus, FocusTarget::Composer) => {
                shell.cycle_autocomplete();
            }
            KeyCode::Up => match shell.focus {
                FocusTarget::Menu => {
                    shell.move_menu_selection(-1);
                    rerender_workspace = true;
                }
                FocusTarget::List => {
                    match shell.current_route() {
                        ScreenRoute::SessionsRoot if matches!(shell.workspace, Workspace::Sessions) => {
                            shell.move_session_picker_selection(-1);
                            shell.preview_selected_session(app)?;
                        }
                        ScreenRoute::SessionsRoot => {
                            shell.move_session_selection(-1);
                            shell.preview_selected_session(app)?;
                        }
                        ScreenRoute::AccountList { .. } => shell.move_account_selection(-1),
                        _ => {}
                    }
                    rerender_workspace = true;
                }
                FocusTarget::Composer if matches!(shell.workspace, Workspace::Chat) => {
                    rerender_workspace = shell.recall_input_history_older();
                }
                _ => {}
            },
            KeyCode::Down => match shell.focus {
                FocusTarget::Menu => {
                    shell.move_menu_selection(1);
                    rerender_workspace = true;
                }
                FocusTarget::List => {
                    match shell.current_route() {
                        ScreenRoute::SessionsRoot if matches!(shell.workspace, Workspace::Sessions) => {
                            shell.move_session_picker_selection(1);
                            shell.preview_selected_session(app)?;
                        }
                        ScreenRoute::SessionsRoot => {
                            shell.move_session_selection(1);
                            shell.preview_selected_session(app)?;
                        }
                        ScreenRoute::AccountList { .. } => shell.move_account_selection(1),
                        _ => {}
                    }
                    rerender_workspace = true;
                }
                FocusTarget::Composer if matches!(shell.workspace, Workspace::Chat) => {
                    rerender_workspace = shell.recall_input_history_newer();
                }
                _ => {}
            },
            KeyCode::Enter => {
                if matches!(shell.focus, FocusTarget::Menu) {
                    if let MenuAction::StartOAuth(pending) = shell.activate_current_menu_item(app)? {
                        pending_oauth = Some(pending);
                    }
                    rerender_workspace = !matches!(shell.workspace, Workspace::Chat);
                } else if matches!(shell.focus, FocusTarget::List) {
                    shell.activate_current_list_item(app)?;
                    rerender_workspace = !matches!(shell.workspace, Workspace::Chat);
                } else if matches!(shell.focus, FocusTarget::Form) {
                    shell.apply_form(app)?;
                    print_inline_status(stdout, shell)?;
                    rerender_workspace = !matches!(shell.workspace, Workspace::Chat);
                } else if shell.input.trim().starts_with('/')
                    && shell.input.trim() == shell.input.trim_end()
                    && !shell.input.contains(' ')
                    && shell.should_accept_autocomplete_on_enter()
                    && shell.accept_autocomplete()
                {
                    redraw_inline_prompt(stdout, shell)?;
                    continue;
                } else {
                    submit_inline_input(stdout, app, shell)?;
                    rerender_workspace = !matches!(shell.workspace, Workspace::Chat);
                }
            }
            KeyCode::Esc => {
                if let Some(pending) = pending_oauth.take() {
                    pending.cancel_flag.store(true, Ordering::Relaxed);
                    shell.set_activity(ActivityState::Idle, "oauth flow canceled");
                }
                if matches!(shell.workspace, Workspace::Sessions)
                    || (matches!(shell.focus, FocusTarget::List)
                        && matches!(shell.workspace, Workspace::Chat))
                {
                    shell.restore_active_chat_preview(app);
                }
                let previous_workspace = shell.workspace;
                let previous_route = shell.current_route();
                let previous_should_quit = shell.should_quit;
                shell.handle_escape();
                if shell.should_quit && !previous_should_quit {
                    continue;
                }
                rerender_workspace = !matches!(shell.workspace, Workspace::Chat)
                    || !matches!(previous_workspace, Workspace::Chat)
                    || shell.current_route() != previous_route;
            }
            _ => {}
        }

        if matches!(shell.workspace, Workspace::Chat)
            && !matches!(previous_workspace, Workspace::Chat)
        {
            print_inline_transcript(stdout, shell)?;
        } else if rerender_workspace {
            print_inline_workspace(stdout, shell)?;
        }
        redraw_inline_prompt(stdout, shell)?;
    }
}

fn run_terminal_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut DaemonBackedTuiApp,
    shell: &mut TuiShellState,
) -> Result<(), OberonError> {
    let mut pending_submission: Option<PendingSubmission> = None;
    let mut pending_oauth: Option<PendingOAuthConnect> = None;

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

        if let Some(pending) = pending_oauth.as_ref() {
            match pending.receiver.try_recv() {
                Ok(OAuthUiEvent::Complete(message)) => {
                    pending_oauth = None;
                    shell.refresh_runtime_data(app);
                    shell.set_activity(ActivityState::Idle, message);
                }
                Ok(OAuthUiEvent::Failed(err)) => {
                    pending_oauth = None;
                    shell.set_activity(ActivityState::Error, err.to_string());
                    shell.refresh_runtime_data(app);
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    pending_oauth = None;
                    shell.set_activity(ActivityState::Error, "oauth flow worker disconnected");
                }
            }
        }

        terminal
            .draw(|frame| render_terminal(frame, shell))
            .map_err(|err| OberonError::Storage(format!("tui draw failed: {err}")))?;
        shell.render_tick = shell.render_tick.wrapping_add(1);

        if shell.should_quit() {
            return Ok(());
        }

        if !event::poll(Duration::from_millis(100))
            .map_err(|err| OberonError::Storage(format!("tui event poll failed: {err}")))?
        {
            continue;
        }

        match event::read()
            .map_err(|err| OberonError::Storage(format!("tui event read failed: {err}")))?
        {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                match key.code {
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        shell.should_quit = true;
                        shell.set_activity(ActivityState::Idle, "quitting");
                    }
                    KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        shell.workspace = Workspace::Sessions;
                        shell.screen_stack = vec![ScreenRoute::SessionsRoot];
                        shell.focus = FocusTarget::List;
                        shell.session_picker = SessionPickerState::default();
                        shell.refresh_runtime_data(app);
                        shell.select_current_session_in_picker();
                        shell.set_activity(ActivityState::Idle, "sessions focused");
                    }
                    KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if matches!(shell.focus, FocusTarget::Composer | FocusTarget::Form) {
                            shell.insert_newline();
                        }
                    }
                    KeyCode::Char(ch) => {
                        if matches!(shell.workspace, Workspace::Sessions)
                            && matches!(shell.current_route(), ScreenRoute::SessionsRoot)
                        {
                            let mut query = shell.session_picker.query.clone();
                            query.push(ch);
                            shell.update_session_picker_query(query, app)?;
                        } else if matches!(shell.focus, FocusTarget::Composer | FocusTarget::Form) {
                            shell.push_input_char(ch)
                        }
                    }
                    KeyCode::Backspace => {
                        if matches!(shell.workspace, Workspace::Sessions)
                            && matches!(shell.current_route(), ScreenRoute::SessionsRoot)
                        {
                            let mut query = shell.session_picker.query.clone();
                            query.pop();
                            shell.update_session_picker_query(query, app)?;
                        } else if matches!(shell.focus, FocusTarget::Composer | FocusTarget::Form) {
                            shell.backspace();
                        }
                    }
                    KeyCode::Tab if matches!(shell.focus, FocusTarget::Composer) => {
                        shell.cycle_autocomplete()
                    }
                    KeyCode::Up => match shell.focus {
                        FocusTarget::Menu => shell.move_menu_selection(-1),
                        FocusTarget::List => match shell.current_route() {
                            ScreenRoute::AccountList { .. } => shell.move_account_selection(-1),
                            ScreenRoute::SessionsRoot if matches!(shell.workspace, Workspace::Sessions) => {
                                shell.move_session_picker_selection(-1);
                                shell.preview_selected_session(app)?;
                            }
                            _ if matches!(shell.workspace, Workspace::Chat) => {
                                shell.move_session_selection(-1);
                                shell.preview_selected_session(app)?;
                            }
                            _ => {}
                        },
                        FocusTarget::Composer if matches!(shell.workspace, Workspace::Chat) => {
                            shell.recall_input_history_older();
                        }
                        _ => {}
                    },
                    KeyCode::Down => match shell.focus {
                        FocusTarget::Menu => shell.move_menu_selection(1),
                        FocusTarget::List => match shell.current_route() {
                            ScreenRoute::AccountList { .. } => shell.move_account_selection(1),
                            ScreenRoute::SessionsRoot if matches!(shell.workspace, Workspace::Sessions) => {
                                shell.move_session_picker_selection(1);
                                shell.preview_selected_session(app)?;
                            }
                            _ if matches!(shell.workspace, Workspace::Chat) => {
                                shell.move_session_selection(1);
                                shell.preview_selected_session(app)?;
                            }
                            _ => {}
                        },
                        FocusTarget::Composer if matches!(shell.workspace, Workspace::Chat) => {
                            shell.recall_input_history_newer();
                        }
                        _ => {}
                    },
                    KeyCode::PageUp
                        if matches!(shell.workspace, Workspace::Chat | Workspace::Sessions) =>
                    {
                        shell.scroll_transcript_by(-10);
                    }
                    KeyCode::PageDown
                        if matches!(shell.workspace, Workspace::Chat | Workspace::Sessions) =>
                    {
                        shell.scroll_transcript_by(10);
                    }
                    KeyCode::Home
                        if matches!(shell.workspace, Workspace::Chat | Workspace::Sessions) =>
                    {
                        shell.scroll_transcript_to_top();
                    }
                    KeyCode::End
                        if matches!(shell.workspace, Workspace::Chat | Workspace::Sessions) =>
                    {
                        shell.scroll_transcript_to_bottom();
                    }
                    KeyCode::Enter => {
                        if matches!(shell.focus, FocusTarget::Menu) {
                            if let MenuAction::StartOAuth(pending) =
                                shell.activate_current_menu_item(app)?
                            {
                                pending_oauth = Some(pending);
                            }
                        } else if matches!(shell.focus, FocusTarget::List) {
                            shell.activate_current_list_item(app)?;
                        } else if matches!(shell.focus, FocusTarget::Form) {
                            shell.apply_form(app)?;
                        } else if shell.input.trim().starts_with('/')
                            && shell.input.trim() == shell.input.trim_end()
                            && !shell.input.contains(' ')
                            && shell.should_accept_autocomplete_on_enter()
                            && shell.accept_autocomplete()
                        {
                            shell.set_activity(ActivityState::Idle, "command completed");
                        } else if pending_submission.is_some() {
                            if matches!(parse_tui_command(shell.input.trim()), Ok(TuiCommand::Stop))
                            {
                                shell.input.clear();
                                cancel_pending_submission(&mut pending_submission, shell);
                            } else if !handle_pending_local_command(shell)? {
                                shell.set_activity(
                                    ActivityState::Running,
                                    "request in flight; use /stop to cancel",
                                );
                            }
                        } else if let Some(pending) = start_async_submit(app, shell)? {
                            pending_submission = Some(pending);
                        }
                    }
                    KeyCode::Delete => {
                        let _ = shell.delete_selected_session(app)?;
                    }
                    KeyCode::Esc => {
                        if let Some(pending) = pending_oauth.take() {
                            pending.cancel_flag.store(true, Ordering::Relaxed);
                            shell.set_activity(ActivityState::Idle, "oauth flow canceled");
                        }
                        if matches!(shell.focus, FocusTarget::List)
                            && matches!(shell.workspace, Workspace::Chat)
                        {
                            shell.restore_active_chat_preview(app);
                        }
                        shell.handle_escape()
                    }
                    _ => {}
                }
            }
            Event::Mouse(mouse)
                if matches!(shell.workspace, Workspace::Chat | Workspace::Sessions) =>
            {
                match mouse.kind {
                    MouseEventKind::ScrollUp => shell.scroll_transcript_by(-3),
                    MouseEventKind::ScrollDown => shell.scroll_transcript_by(3),
                    _ => {}
                }
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

    let command = match parse_tui_command(&trimmed) {
        Ok(command) => command,
        Err(err) => {
            shell.set_activity(ActivityState::Error, err.to_string());
            return Ok(true);
        }
    };
    let response = match command {
        TuiCommand::Help => TuiResponse {
            output: EXTENDED_HELP.to_string(),
            should_quit: false,
        },
        TuiCommand::Status => TuiResponse {
            output: "request in flight".to_string(),
            should_quit: false,
        },
        TuiCommand::Stop => TuiResponse {
            output: "response stopped".to_string(),
            should_quit: false,
        },
        TuiCommand::Sessions => TuiResponse {
            output: "sessions focused".to_string(),
            should_quit: false,
        },
        TuiCommand::Settings | TuiCommand::Config => TuiResponse {
            output: "settings workspace opened".to_string(),
            should_quit: false,
        },
        TuiCommand::Accounts => TuiResponse {
            output: "accounts workspace opened".to_string(),
            should_quit: false,
        },
        TuiCommand::Diagnostics => TuiResponse {
            output: "diagnostics workspace opened".to_string(),
            should_quit: false,
        },
        TuiCommand::More => TuiResponse {
            output: "more workspace opened".to_string(),
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
    shell.scroll_transcript_to_bottom();
    if !response.output.is_empty() {
        shell.transcript.push(UiLine {
            role: "status".to_string(),
            content: response.output,
        });
        shell.scroll_transcript_to_bottom();
    }
    shell.open_route_for_command(&command);
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
        role: "assistant".to_string(),
        content: "...".to_string(),
    });
    shell.scroll_transcript_to_bottom();
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

fn cancel_pending_submission(
    pending_submission: &mut Option<PendingSubmission>,
    shell: &mut TuiShellState,
) -> bool {
    let Some(pending) = pending_submission.take() else {
        return false;
    };

    if let Some(line) = shell.transcript.get(pending.assistant_line_index) {
        if (line.content == "..." || line.content.is_empty())
            && pending.assistant_line_index < shell.transcript.len()
        {
            shell.transcript.remove(pending.assistant_line_index);
        }
    }
    shell.scroll_transcript_to_bottom();
    shell.set_activity(ActivityState::Idle, "response stopped");
    true
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
                    shell.scroll_transcript_to_bottom();
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
                    shell.scroll_transcript_to_bottom();
                }
            }
        }
        AsyncUiEvent::Complete { prompt, message } => {
            let pending = pending_submission.take();
            if let Some(pending) = pending {
                finalize_streamed_assistant(shell, &pending, &message);
            }
            app.apply_successful_prompt(&prompt, message.clone());
            let _ = shell.autosave_current_session(app, &prompt, &message.content);
            shell.set_activity(ActivityState::Idle, "response complete");
            shell.refresh_runtime_data(app);
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
            shell.scroll_transcript_to_bottom();
            shell.set_activity(ActivityState::Error, err.to_string());
            shell.refresh_runtime_data(app);
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
        let final_content = message.content.trim();
        if final_content.is_empty() && line.content.trim().is_empty() {
            line.content.clear();
        } else if final_content.is_empty() {
            line.content = line.content.trim().to_string();
        } else {
            line.content = final_content.to_string();
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
        Constraint::Min(8),
        Constraint::Length(FOOTER_HEIGHT),
    ])
    .areas(area);

    render_header(frame, header, shell);
    render_body(frame, body, shell);
    render_footer(frame, footer, shell);

    if matches!(shell.focus, FocusTarget::Composer | FocusTarget::Form) {
        let cursor_lines = composer_lines(shell.input());
        let last_line = cursor_lines.last().cloned().unwrap_or_default();
        let cursor_y = footer
            .y
            .saturating_add(1 + cursor_lines.len().saturating_sub(1) as u16);
        let cursor_x = footer.x.saturating_add(1 + last_line.len() as u16);
        frame.set_cursor(cursor_x, cursor_y);
    }
}

fn render_header(frame: &mut Frame, area: Rect, shell: &TuiShellState) {
    let session = shell.session_title_for_header();
    let dirty = if shell.is_dirty() { "*" } else { "" };
    let mode = if shell.overlay_open() {
        format!("overlay={}", shell.workspace_title())
    } else if matches!(shell.focus, FocusTarget::List) {
        "focus=sessions".to_string()
    } else {
        "focus=chat".to_string()
    };
    let header_lines = Text::from(vec![Line::styled(
        format!(
            " Oberon4 | request={} | daemon={} | {} | model={} | oberon4-account={} | session={}{} | {} ",
            shell.status_snapshot.endpoint,
            if shell.status_snapshot.running {
                "running"
            } else {
                "stopped"
            },
            shell.activity_label(),
            shell.status_snapshot.model,
            shell.status_snapshot.active_account,
            session,
            dirty,
            mode,
        ),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )]);
    let header_widget =
        Paragraph::new(header_lines).block(Block::default().borders(Borders::ALL).title("Chat"));
    frame.render_widget(header_widget, area);
}

fn render_body(frame: &mut Frame, area: Rect, shell: &TuiShellState) {
    render_chat_workspace(frame, area, shell);
    if shell.overlay_open() {
        render_overlay(frame, area, shell);
    }
}

fn render_chat_workspace(frame: &mut Frame, area: Rect, shell: &TuiShellState) {
    let [sidebar, main] = Layout::horizontal([
        Constraint::Length(SESSION_SIDEBAR_WIDTH),
        Constraint::Min(20),
    ])
    .areas(area);
    render_sessions_sidebar(frame, sidebar, shell);
    render_transcript(frame, main, shell);
}

fn render_transcript(frame: &mut Frame, area: Rect, shell: &TuiShellState) {
    let block = Block::default().borders(Borders::ALL).title("Transcript");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let viewport = build_transcript_viewport(inner, shell);
    let (text_area, scrollbar_area) = if viewport.scrollbar_visible && inner.width > 1 {
        let [text, bar] =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
        (text, Some(bar))
    } else {
        (inner, None)
    };

    frame.render_widget(Paragraph::new(Text::from(viewport.visible_rows)), text_area);

    if let Some(scrollbar_area) = scrollbar_area {
        let mut state = ScrollbarState::new(viewport.scrollable_rows)
            .position(viewport.scroll)
            .viewport_content_length(viewport.visible_height);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .thumb_style(Style::default().fg(Color::Cyan))
                .track_style(Style::default().fg(Color::DarkGray))
                .begin_symbol(None)
                .end_symbol(None),
            scrollbar_area,
            &mut state,
        );
    }
}

#[derive(Debug)]
struct TranscriptViewport {
    visible_rows: Vec<Line<'static>>,
    scrollable_rows: usize,
    scroll: usize,
    visible_height: usize,
    scrollbar_visible: bool,
}

fn build_transcript_viewport(area: Rect, shell: &TuiShellState) -> TranscriptViewport {
    let visible_height = area.height as usize;
    if visible_height == 0 {
        return TranscriptViewport {
            visible_rows: Vec::new(),
            scrollable_rows: 0,
            scroll: 0,
            visible_height: 0,
            scrollbar_visible: false,
        };
    }

    let max_width = area.width.max(1) as usize;
    let rows_without_scrollbar = build_transcript_rows(shell, max_width);
    let mut scrollbar_visible = rows_without_scrollbar.len() > visible_height;
    let mut rows = if scrollbar_visible && max_width > 1 {
        build_transcript_rows(shell, max_width.saturating_sub(1).max(1))
    } else {
        rows_without_scrollbar
    };
    scrollbar_visible = rows.len() > visible_height;

    let total_rows = rows.len();
    let max_scroll = total_rows.saturating_sub(visible_height);
    let scrollable_rows = if total_rows == 0 { 0 } else { max_scroll + 1 };
    let scroll = if shell.transcript_follow {
        max_scroll
    } else {
        usize::from(shell.transcript_scroll).min(max_scroll)
    };

    let visible_rows = if total_rows == 0 {
        Vec::new()
    } else {
        rows.drain(scroll..(scroll + visible_height).min(total_rows))
            .collect()
    };

    TranscriptViewport {
        visible_rows,
        scrollable_rows,
        scroll,
        visible_height,
        scrollbar_visible,
    }
}

fn build_transcript_rows(shell: &TuiShellState, width: usize) -> Vec<Line<'static>> {
    let logical_lines = if shell.transcript().is_empty() {
        vec![Line::raw(
            "No messages yet. Type text to chat or /help for commands.".to_string(),
        )]
    } else {
        shell
            .transcript()
            .iter()
            .flat_map(transcript_message_lines)
            .collect()
    };

    wrap_styled_lines(logical_lines, width.max(1))
}

fn transcript_message_lines(line: &UiLine) -> Vec<Line<'static>> {
    let role_style = role_style(&line.role);
    let prefix = format!("[{}] ", line.role);
    let continuation = " ".repeat(prefix.chars().count());
    let content_lines = if line.role == "assistant" {
        format_assistant_markdown(&line.content)
    } else {
        plain_content_lines(&line.content)
    };

    if content_lines.is_empty() {
        return vec![Line::from(vec![Span::styled(prefix, role_style)])];
    }

    content_lines
        .into_iter()
        .enumerate()
        .map(|(index, content)| {
            let mut spans = Vec::with_capacity(content.spans.len() + 1);
            if index == 0 {
                spans.push(Span::styled(prefix.clone(), role_style));
            } else {
                spans.push(Span::styled(
                    continuation.clone(),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            spans.extend(content.spans);
            Line::from(spans)
        })
        .collect()
}

fn plain_content_lines(content: &str) -> Vec<Line<'static>> {
    let lines: Vec<Line<'static>> = content
        .split('\n')
        .map(|segment| Line::from(vec![Span::raw(segment.to_string())]))
        .collect();
    if lines.is_empty() {
        vec![Line::default()]
    } else {
        lines
    }
}

fn format_assistant_markdown(content: &str) -> Vec<Line<'static>> {
    let mut rendered = Vec::new();
    let mut in_code_block = false;
    for raw_line in content.split('\n') {
        let trimmed = raw_line.trim_start();
        if trimmed.starts_with("```") {
            if in_code_block {
                in_code_block = false;
            } else {
                in_code_block = true;
                let code_language = trimmed
                    .trim_start_matches("```")
                    .split_whitespace()
                    .next()
                    .filter(|lang| !lang.is_empty())
                    .map(str::to_string);
                if let Some(lang) = code_language {
                    rendered.push(Line::from(vec![Span::styled(
                        format!("```{lang}"),
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::ITALIC),
                    )]));
                }
            }
            continue;
        }

        if in_code_block {
            rendered.push(Line::from(vec![Span::styled(
                raw_line.to_string(),
                Style::default().fg(CODE_BLOCK_FG).bg(CODE_BLOCK_BG),
            )]));
            continue;
        }

        if raw_line.trim().is_empty() {
            rendered.push(Line::default());
            continue;
        }

        if let Some(rest) = parse_heading(trimmed) {
            rendered.push(Line::from(parse_inline_markdown(
                rest,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("> ") {
            let mut spans = vec![Span::styled(
                "▎ ".to_string(),
                Style::default().fg(Color::LightBlue),
            )];
            spans.extend(parse_inline_markdown(
                rest,
                Style::default().fg(Color::LightBlue),
            ));
            rendered.push(Line::from(spans));
            continue;
        }

        if let Some(rest) = parse_list_item(trimmed) {
            let mut spans = vec![Span::styled(
                "• ".to_string(),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            )];
            spans.extend(parse_inline_markdown(rest, Style::default()));
            rendered.push(Line::from(spans));
            continue;
        }

        rendered.push(Line::from(parse_inline_markdown(
            raw_line,
            Style::default(),
        )));
    }

    if rendered.is_empty() {
        plain_content_lines(content)
    } else {
        rendered
    }
}

fn parse_heading(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    trimmed[hashes..].strip_prefix(' ').map(str::trim_end)
}

fn parse_list_item(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    for prefix in ["- ", "* ", "+ ", "• "] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return Some(rest);
        }
    }
    let digit_count = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    if digit_count > 0 {
        let remainder = &trimmed[digit_count..];
        if let Some(rest) = remainder.strip_prefix(". ") {
            return Some(rest);
        }
    }
    None
}

fn parse_inline_markdown(line: &str, base_style: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut in_code = false;
    let mut in_bold = false;
    let mut in_italic = false;
    let mut buffer = String::new();
    let chars: Vec<char> = line.chars().collect();
    let mut idx = 0usize;
    while idx < chars.len() {
        let ch = chars[idx];
        let next = chars.get(idx + 1).copied();
        if ch == '`' {
            if !buffer.is_empty() {
                spans.push(Span::styled(
                    std::mem::take(&mut buffer),
                    inline_markdown_style(base_style, in_code, in_bold, in_italic),
                ));
            }
            in_code = !in_code;
            idx += 1;
            continue;
        }

        if !in_code {
            if (ch == '*' || ch == '_') && next == Some(ch) {
                if !buffer.is_empty() {
                    spans.push(Span::styled(
                        std::mem::take(&mut buffer),
                        inline_markdown_style(base_style, in_code, in_bold, in_italic),
                    ));
                }
                in_bold = !in_bold;
                idx += 2;
                continue;
            }

            if ch == '*' || ch == '_' {
                if !buffer.is_empty() {
                    spans.push(Span::styled(
                        std::mem::take(&mut buffer),
                        inline_markdown_style(base_style, in_code, in_bold, in_italic),
                    ));
                }
                in_italic = !in_italic;
                idx += 1;
                continue;
            }
        }

        buffer.push(ch);
        idx += 1;
    }
    if !buffer.is_empty() {
        spans.push(Span::styled(
            buffer,
            inline_markdown_style(base_style, in_code, in_bold, in_italic),
        ));
    }
    if spans.is_empty() {
        vec![Span::styled(String::new(), base_style)]
    } else {
        spans
    }
}

fn inline_markdown_style(
    base_style: Style,
    in_code: bool,
    in_bold: bool,
    in_italic: bool,
) -> Style {
    if in_code {
        return base_style.fg(Color::Yellow).bg(Color::DarkGray);
    }

    let mut style = base_style;
    if in_bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    if in_italic {
        style = style.add_modifier(Modifier::ITALIC);
    }
    style
}

fn wrap_styled_lines(lines: Vec<Line<'static>>, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut wrapped = Vec::new();
    for line in lines {
        if line.spans.is_empty() {
            wrapped.push(Line::default());
            continue;
        }
        let mut current = Vec::new();
        let mut current_width = 0usize;
        for span in line.spans {
            let style = span.style;
            let mut segment = String::new();
            for ch in span.content.chars() {
                segment.push(ch);
                current_width += 1;
                if current_width == width {
                    current.push(Span::styled(std::mem::take(&mut segment), style));
                    wrapped.push(Line::from(std::mem::take(&mut current)));
                    current_width = 0;
                }
            }
            if !segment.is_empty() {
                current.push(Span::styled(segment, style));
            }
        }
        if current.is_empty() {
            wrapped.push(Line::default());
        } else {
            wrapped.push(Line::from(current));
        }
    }
    wrapped
}

fn role_style(role: &str) -> Style {
    match role {
        "assistant" => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        "user" => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
        "system" => Style::default().fg(Color::Yellow),
        "error" => Style::default().fg(Color::Red),
        "status" => Style::default().fg(Color::Magenta),
        _ => Style::default().fg(Color::White),
    }
}

fn render_sessions_sidebar(frame: &mut Frame, area: Rect, shell: &TuiShellState) {
    let items: Vec<ListItem<'static>> = shell
        .session_sidebar_items()
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            let selected = index == shell.session_index;
            let title = match item {
                SessionSidebarItem::CurrentChat => shell.current_chat_sidebar_title(),
                SessionSidebarItem::Saved(session_index) => shell
                    .sessions
                    .get(session_index)
                    .map(|session| session.title.clone())
                    .unwrap_or_else(|| "missing".to_string()),
            };
            let visible = compact_session_title(&title, selected, shell.render_tick);
            let style = if selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(Line::styled(visible, style))
        })
        .collect();
    frame.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title(
            if matches!(shell.focus, FocusTarget::List) {
                "Sessions *"
            } else {
                "Sessions"
            },
        )),
        area,
    );
}

fn overlay_area(area: Rect) -> Rect {
    let width = area
        .width
        .saturating_mul(3)
        .saturating_div(4)
        .max(40)
        .min(area.width);
    let height = area
        .height
        .saturating_mul(3)
        .saturating_div(4)
        .max(10)
        .min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

fn render_overlay(frame: &mut Frame, area: Rect, shell: &TuiShellState) {
    let overlay = overlay_area(area);
    frame.render_widget(WidgetClear, overlay);
    match shell.workspace {
        Workspace::Sessions => render_sessions_overlay(frame, overlay, shell),
        Workspace::Settings => render_settings_workspace(frame, overlay, shell),
        Workspace::Status => render_status_workspace(frame, overlay, shell),
        Workspace::Help => render_help_workspace(frame, overlay, shell),
        Workspace::Accounts => render_accounts_workspace(frame, overlay, shell),
        Workspace::Diagnostics => render_diagnostics_workspace(frame, overlay, shell),
        Workspace::Reserved => render_reserved_workspace(frame, overlay, shell),
        Workspace::Chat => {}
    }
}

fn render_sessions_overlay(frame: &mut Frame, area: Rect, shell: &TuiShellState) {
    let [search_area, list_area] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(5)]).areas(area);
    frame.render_widget(
        Paragraph::new(shell.session_picker.query.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Find sessions"),
        ),
        search_area,
    );

    let items = if shell.session_picker.filtered_items.is_empty() {
        vec![ListItem::new("No sessions match")]
    } else {
        shell.session_picker
            .filtered_items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let prefix = if index == shell.session_picker.selected {
                    ">"
                } else {
                    " "
                };
                ListItem::new(format!("{prefix} {}", shell.session_picker_label(*item)))
            })
            .collect::<Vec<_>>()
    };
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Sessions"),
        ),
        list_area,
    );
}

fn render_settings_workspace(frame: &mut Frame, area: Rect, shell: &TuiShellState) {
    let route = shell.current_route();
    match route {
        ScreenRoute::SettingsRoot
        | ScreenRoute::SettingsProvider
        | ScreenRoute::SettingsOpenAiOAuth
        | ScreenRoute::SettingsAnthropicOAuth
        | ScreenRoute::SettingsRuntime => {
            let title = match route {
                ScreenRoute::SettingsRoot => "Settings",
                ScreenRoute::SettingsProvider => "Provider",
                ScreenRoute::SettingsOpenAiOAuth => "OpenAI OAuth",
                ScreenRoute::SettingsAnthropicOAuth => "Anthropic OAuth",
                ScreenRoute::SettingsRuntime => "Runtime",
                _ => unreachable!(),
            };
            let items = shell
                .menu_items()
                .into_iter()
                .enumerate()
                .map(|(index, item)| {
                    let prefix = if index == shell.menu_index { ">" } else { " " };
                    ListItem::new(format!("{prefix} {item}"))
                })
                .collect::<Vec<_>>();
            frame.render_widget(
                List::new(items).block(Block::default().borders(Borders::ALL).title(title)),
                area,
            );
        }
        ScreenRoute::SettingsModelForm
        | ScreenRoute::SettingsEndpointForm
        | ScreenRoute::SettingsSystemPromptForm
        | ScreenRoute::SettingsRuntimeTimeoutForm
        | ScreenRoute::SettingsRuntimeRetriesForm => {
            let lines = match route {
                ScreenRoute::SettingsModelForm => vec![
                    Line::raw("Set the active model."),
                    Line::raw(format!("current: {}", shell.config_snapshot.endpoint.model)),
                ],
                ScreenRoute::SettingsEndpointForm => vec![
                    Line::raw("Set the endpoint base URL."),
                    Line::raw(format!(
                        "current: {}",
                        shell.config_snapshot.endpoint.base_url
                    )),
                ],
                ScreenRoute::SettingsSystemPromptForm => {
                    vec![Line::raw("Set the current session system prompt.")]
                }
                ScreenRoute::SettingsRuntimeTimeoutForm => vec![
                    Line::raw("Set request timeout (ms)."),
                    Line::raw(format!(
                        "current: {}",
                        shell.config_snapshot.runtime.request_timeout_ms
                    )),
                ],
                ScreenRoute::SettingsRuntimeRetriesForm => vec![
                    Line::raw("Set max retries."),
                    Line::raw(format!(
                        "current: {}",
                        shell.config_snapshot.runtime.max_retries
                    )),
                ],
                _ => unreachable!(),
            };
            frame.render_widget(
                Paragraph::new(Text::from(lines))
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title("Settings Form"),
                    )
                    .wrap(Wrap { trim: false }),
                area,
            );
        }
        ScreenRoute::AccountList { .. } | ScreenRoute::AccountDetail { .. } => {
            render_accounts_workspace(frame, area, shell)
        }
        _ => render_status_workspace(frame, area, shell),
    }
}

fn render_status_workspace(frame: &mut Frame, area: Rect, shell: &TuiShellState) {
    let lines = Text::from(vec![
        Line::raw(format!("activity: {}", shell.activity_label())),
        Line::raw(format!("status: {}", shell.status())),
        Line::raw(format!(
            "request endpoint: {}",
            shell.status_snapshot.endpoint
        )),
        Line::raw(format!("model: {}", shell.status_snapshot.model)),
        Line::raw(format!(
            "oberon4 account: {}",
            shell.status_snapshot.active_account
        )),
        Line::raw(format!("session: {}", shell.session_title_for_header())),
        Line::raw(format!("dirty: {}", shell.is_dirty())),
        Line::raw(format!(
            "request timeout: {}",
            shell.config_snapshot.runtime.request_timeout_ms
        )),
        Line::raw(format!(
            "streaming: {}",
            shell.config_snapshot.runtime.stream_responses
        )),
    ]);
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("Status"))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_help_workspace(frame: &mut Frame, area: Rect, shell: &TuiShellState) {
    let [top, bottom] = Layout::vertical([Constraint::Length(7), Constraint::Min(5)]).areas(area);
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::raw("/new         start fresh chat"),
            Line::raw("/stop        stop current reply"),
            Line::raw("/sessions    browse chats"),
            Line::raw("/settings    open settings"),
            Line::raw("/status      show status"),
            Line::raw("/accounts    manage accounts"),
            Line::raw("/diagnostics show diagnostics"),
            Line::raw("/more        extra tools"),
        ]))
        .block(Block::default().borders(Borders::ALL).title("Navigation"))
        .wrap(Wrap { trim: false }),
        top,
    );
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::raw(EXTENDED_HELP),
            Line::raw("Up/Down recall prompt history in composer; navigate menus and lists elsewhere."),
            Line::raw("Enter opens or activates."),
            Line::raw("Esc always backs out one level before clearing/quitting."),
            Line::raw(format!("current status: {}", shell.status())),
        ]))
        .block(Block::default().borders(Borders::ALL).title("Help"))
        .wrap(Wrap { trim: false }),
        bottom,
    );
}

fn render_accounts_workspace(frame: &mut Frame, area: Rect, shell: &TuiShellState) {
    match shell.current_route() {
        ScreenRoute::AccountsRoot => {
            let items = shell
                .menu_items()
                .into_iter()
                .enumerate()
                .map(|(index, item)| {
                    let prefix = if index == shell.menu_index { ">" } else { " " };
                    ListItem::new(format!("{prefix} {item}"))
                })
                .collect::<Vec<_>>();
            frame.render_widget(
                List::new(items).block(Block::default().borders(Borders::ALL).title("Accounts")),
                area,
            );
        }
        ScreenRoute::AccountList { provider, action } => {
            let title = match action {
                AccountActionMode::Manage => "Accounts",
                AccountActionMode::Activate => "Activate Account",
                AccountActionMode::Disable => "Disable Account",
                AccountActionMode::Disconnect => "Disconnect Account",
            };
            let items = shell
                .accounts_for_current_route()
                .unwrap_or_default()
                .into_iter()
                .enumerate()
                .map(|(index, account)| {
                    let prefix = if index == shell.account_index {
                        ">"
                    } else {
                        " "
                    };
                    ListItem::new(format!(
                        "{prefix} {} [{}]",
                        account.label,
                        match account.state {
                            oberon4_core::AccountState::Active => "active",
                            oberon4_core::AccountState::Disabled => "disabled",
                        }
                    ))
                })
                .collect::<Vec<_>>();
            frame.render_widget(
                List::new(if items.is_empty() {
                    vec![ListItem::new("No accounts")]
                } else {
                    items
                })
                .block(Block::default().borders(Borders::ALL).title(format!(
                    "{} ({})",
                    title,
                    provider_name_for_oauth(provider)
                ))),
                area,
            );
        }
        ScreenRoute::AccountDetail { provider } => {
            let account = shell.current_account();
            let mut lines = vec![Line::raw(format!(
                "provider: {}",
                provider_name_for_oauth(provider)
            ))];
            if let Some(account) = account {
                lines.push(Line::raw(format!("label: {}", account.label)));
                lines.push(Line::raw(format!("id: {}", account.id)));
                lines.push(Line::raw(format!(
                    "state: {}",
                    match account.state {
                        oberon4_core::AccountState::Active => "active",
                        oberon4_core::AccountState::Disabled => "disabled",
                    }
                )));
            }
            let menu = shell
                .menu_items()
                .into_iter()
                .enumerate()
                .map(|(index, item)| {
                    let prefix = if index == shell.menu_index { ">" } else { " " };
                    Line::raw(format!("{prefix} {item}"))
                })
                .collect::<Vec<_>>();
            lines.extend(menu);
            frame.render_widget(
                Paragraph::new(Text::from(lines))
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title("Account Detail"),
                    )
                    .wrap(Wrap { trim: false }),
                area,
            );
        }
        _ => render_status_workspace(frame, area, shell),
    }
}

fn render_diagnostics_workspace(frame: &mut Frame, area: Rect, shell: &TuiShellState) {
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::raw(format!(
                "endpoint health: {}",
                shell.diagnostics_endpoint_health
            )),
            Line::raw(format!("activity: {}", shell.activity_label())),
            Line::raw(format!("status: {}", shell.status())),
            Line::raw(format!("endpoint: {}", shell.status_snapshot.endpoint)),
            Line::raw(format!("model: {}", shell.status_snapshot.model)),
        ]))
        .block(Block::default().borders(Borders::ALL).title("Diagnostics"))
        .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_reserved_workspace(frame: &mut Frame, area: Rect, _shell: &TuiShellState) {
    frame.render_widget(
        Paragraph::new("Reserved workspace. Future tools and views will live here.")
            .block(Block::default().borders(Borders::ALL).title("More"))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_footer(frame: &mut Frame, area: Rect, shell: &TuiShellState) {
    let [input_area, hint_area] =
        Layout::vertical([Constraint::Length(4), Constraint::Length(1)]).areas(area);
    let input_title = match shell.focus {
        FocusTarget::Composer => "Composer",
        FocusTarget::Form => "Editor",
        _ => "Workspace",
    };
    frame.render_widget(WidgetClear, input_area);
    if matches!(shell.focus, FocusTarget::Composer | FocusTarget::Form) {
        let input = Paragraph::new(shell.input())
            .block(Block::default().borders(Borders::ALL).title(input_title))
            .wrap(Wrap { trim: false });
        frame.render_widget(input, input_area);
    } else {
        frame.render_widget(
            Paragraph::new(format!("{} workspace", shell.workspace_title()))
                .block(Block::default().borders(Borders::ALL).title(input_title)),
            input_area,
        );
    }

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
        let hint = if matches!(shell.focus, FocusTarget::Composer | FocusTarget::Form) {
            "/new /stop /help /sessions /settings /status /accounts /diagnostics /more /quit | Up/Down history"
        } else if matches!(shell.workspace, Workspace::Sessions) {
            "Type to filter | Up/Down preview | Enter open | Esc clear/back"
        } else {
            "Up/Down navigate | Enter select | mouse wheel scroll | Home/End jump | Esc back"
        };
        frame.render_widget(Paragraph::new(hint), hint_area);
    }
}

fn print_inline_banner(stdout: &mut io::Stdout) -> Result<(), OberonError> {
    write_inline_line(stdout, "Oberon4 TUI")?;
    write_inline_line(
        stdout,
        "Inline mode. Use /help for commands. Use /new or /sessions for chats.",
    )?;
    write_inline_line(stdout, "")?;
    stdout
        .flush()
        .map_err(|err| OberonError::Storage(format!("stdout flush failed: {err}")))?;
    Ok(())
}

fn print_inline_transcript(
    stdout: &mut io::Stdout,
    shell: &TuiShellState,
) -> Result<(), OberonError> {
    write_inline_line(stdout, "Transcript")?;
    if shell.transcript.is_empty() {
        write_inline_line(stdout, "  (empty)")?;
    } else {
        for line in &shell.transcript {
            write_inline_line(stdout, &format!("[{}] {}", line.role, line.content))?;
        }
    }
    write_inline_line(stdout, "")?;
    stdout
        .flush()
        .map_err(|err| OberonError::Storage(format!("stdout flush failed: {err}")))?;
    Ok(())
}

fn print_inline_home_screen(
    stdout: &mut io::Stdout,
    shell: &TuiShellState,
) -> Result<(), OberonError> {
    execute!(stdout, MoveTo(0, 0), Clear(ClearType::All))
        .map_err(|err| OberonError::Storage(format!("inline screen clear failed: {err}")))?;
    print_inline_banner(stdout)?;
    print_inline_status(stdout, shell)?;
    print_inline_transcript(stdout, shell)?;
    Ok(())
}

fn print_inline_status(stdout: &mut io::Stdout, shell: &TuiShellState) -> Result<(), OberonError> {
    if shell.status().trim().is_empty() {
        return Ok(());
    }
    write_inline_line(stdout, &format!("[status] {}", shell.status()))?;
    stdout
        .flush()
        .map_err(|err| OberonError::Storage(format!("stdout flush failed: {err}")))?;
    Ok(())
}

fn redraw_inline_prompt(stdout: &mut io::Stdout, shell: &TuiShellState) -> Result<(), OberonError> {
    execute!(stdout, MoveToColumn(0), Clear(ClearType::CurrentLine))
        .map_err(|err| OberonError::Storage(format!("inline prompt clear failed: {err}")))?;
    let label = match shell.focus {
        FocusTarget::Form => "edit",
        _ => "you",
    };
    write!(stdout, "{label}> {}", shell.input())
        .map_err(|err| OberonError::Storage(format!("inline prompt write failed: {err}")))?;
    stdout
        .flush()
        .map_err(|err| OberonError::Storage(format!("stdout flush failed: {err}")))?;
    Ok(())
}

fn print_inline_workspace(
    stdout: &mut io::Stdout,
    shell: &TuiShellState,
) -> Result<(), OberonError> {
    write_inline_line(stdout, "")?;
    match shell.current_route() {
        ScreenRoute::SessionsRoot => {
            write_inline_line(stdout, "Sessions")?;
            if !shell.session_picker.query.is_empty() {
                write_inline_line(stdout, &format!("search: {}", shell.session_picker.query))?;
            }
            let items = if matches!(shell.workspace, Workspace::Sessions) {
                shell.session_picker.filtered_items.clone()
            } else {
                shell.session_sidebar_items()
            };
            if items.is_empty() {
                write_inline_line(stdout, "  No sessions match")?;
            }
            for (index, item) in items.iter().enumerate() {
                let selected_index = if matches!(shell.workspace, Workspace::Sessions) {
                    shell.session_picker.selected
                } else {
                    shell.session_index
                };
                let prefix = if index == selected_index { ">" } else { " " };
                let title = match item {
                    SessionSidebarItem::CurrentChat => shell.current_chat_sidebar_title(),
                    SessionSidebarItem::Saved(saved_index) => shell
                        .sessions
                        .get(*saved_index)
                        .map(|session| session.title.clone())
                        .unwrap_or_else(|| "unknown".to_string()),
                };
                write_inline_line(stdout, &format!("{prefix} {title}"))?;
            }
        }
        ScreenRoute::SettingsRoot
        | ScreenRoute::SettingsProvider
        | ScreenRoute::SettingsOpenAiOAuth
        | ScreenRoute::SettingsAnthropicOAuth
        | ScreenRoute::SettingsRuntime
        | ScreenRoute::AccountsRoot
        | ScreenRoute::AccountDetail { .. } => {
            write_inline_line(stdout, shell.workspace_title())?;
            for (index, item) in shell.menu_items().iter().enumerate() {
                let prefix = if index == shell.menu_index { ">" } else { " " };
                write_inline_line(stdout, &format!("{prefix} {item}"))?;
            }
        }
        ScreenRoute::AccountList { provider, .. } => {
            write_inline_line(
                stdout,
                &format!("Accounts ({})", provider_name_for_oauth(provider)),
            )?;
            let accounts = shell.accounts_for_current_route().unwrap_or_default();
            if accounts.is_empty() {
                write_inline_line(stdout, "  No accounts")?;
            } else {
                for (index, account) in accounts.iter().enumerate() {
                    let prefix = if index == shell.account_index {
                        ">"
                    } else {
                        " "
                    };
                    write_inline_line(
                        stdout,
                        &format!("{prefix} {} [{}]", account.label, account.id),
                    )?;
                }
            }
        }
        ScreenRoute::StatusRoot => {
            write_inline_line(stdout, "Status")?;
            write_inline_line(
                stdout,
                &format!(
                    "  daemon: {}",
                    if shell.status_snapshot.running {
                        "running"
                    } else {
                        "stopped"
                    }
                ),
            )?;
            write_inline_line(
                stdout,
                &format!("  request endpoint: {}", shell.status_snapshot.endpoint),
            )?;
            write_inline_line(stdout, &format!("  model: {}", shell.status_snapshot.model))?;
            write_inline_line(
                stdout,
                &format!(
                    "  oberon4 account: {}",
                    shell.status_snapshot.active_account
                ),
            )?;
            write_inline_line(
                stdout,
                &format!("  session: {}", shell.session_title_for_header()),
            )?;
        }
        ScreenRoute::HelpRoot => {
            write_inline_line(stdout, "Help")?;
            write_inline_line(stdout, EXTENDED_HELP)?;
        }
        ScreenRoute::DiagnosticsRoot => {
            write_inline_line(stdout, "Diagnostics")?;
            write_inline_line(
                stdout,
                &format!("  endpoint health: {}", shell.diagnostics_endpoint_health),
            )?;
            write_inline_line(stdout, &format!("  activity: {}", shell.activity_label()))?;
            write_inline_line(stdout, &format!("  status: {}", shell.status()))?;
        }
        ScreenRoute::ReservedRoot => {
            write_inline_line(stdout, "More")?;
            write_inline_line(stdout, "  More tools coming soon.")?;
        }
        ScreenRoute::SettingsModelForm
        | ScreenRoute::SettingsEndpointForm
        | ScreenRoute::SettingsSystemPromptForm
        | ScreenRoute::SettingsRuntimeTimeoutForm
        | ScreenRoute::SettingsRuntimeRetriesForm
        | ScreenRoute::ChatRoot => {}
    }
    write_inline_line(stdout, "")?;
    stdout
        .flush()
        .map_err(|err| OberonError::Storage(format!("stdout flush failed: {err}")))?;
    Ok(())
}

fn sync_shell_transcript_from_app(shell: &mut TuiShellState, app: &DaemonBackedTuiApp) {
    shell.transcript = app
        .transcript()
        .iter()
        .map(|message| UiLine {
            role: role_label(message.role).to_string(),
            content: message.content.clone(),
        })
        .collect();
}

fn submit_inline_input(
    stdout: &mut io::Stdout,
    app: &mut DaemonBackedTuiApp,
    shell: &mut TuiShellState,
) -> Result<(), OberonError> {
    let trimmed = std::mem::take(&mut shell.input).trim().to_string();
    shell.autocomplete = None;
    execute!(stdout, MoveToColumn(0), Clear(ClearType::CurrentLine))
        .map_err(|err| OberonError::Storage(format!("inline prompt clear failed: {err}")))?;
    if trimmed.is_empty() {
        shell.set_activity(ActivityState::Idle, "input was empty");
        return Ok(());
    }

    if trimmed.starts_with('/') {
        let command = parse_tui_command(&trimmed)?;
        match command {
            TuiCommand::Help
            | TuiCommand::Status
            | TuiCommand::Sessions
            | TuiCommand::Settings
            | TuiCommand::Accounts
            | TuiCommand::Diagnostics
            | TuiCommand::More
            | TuiCommand::Config => {
                shell.open_route_for_command(&command);
                shell.refresh_runtime_data(app);
                shell.set_activity(
                    ActivityState::Idle,
                    match command {
                        TuiCommand::Help => "help opened",
                        TuiCommand::Status => "status opened",
                        TuiCommand::Sessions => "sessions focused",
                        TuiCommand::Settings | TuiCommand::Config => "settings opened",
                        TuiCommand::Accounts => "accounts opened",
                        TuiCommand::Diagnostics => "diagnostics opened",
                        TuiCommand::More => "more opened",
                        _ => "workspace opened",
                    },
                );
                print_inline_workspace(stdout, shell)?;
            }
            TuiCommand::Quit => {
                shell.should_quit = true;
                shell.set_activity(ActivityState::Idle, "quitting");
            }
            _ => {
                let response = app.handle_command(command.clone())?;
                shell.set_activity(ActivityState::Idle, response.output.clone());
                shell.should_quit = response.should_quit;
                match command {
                    TuiCommand::New => {
                        shell.current_session_id = None;
                        shell.current_session_title = None;
                        shell.baseline_signature = None;
                    }
                    TuiCommand::Load(name) => shell.sync_loaded_session_from_id(&name),
                    TuiCommand::Save(name) => {
                        shell.mark_loaded_session(Some(name.clone()), Some(name))
                    }
                    TuiCommand::Clear => {
                        shell.current_session_id = None;
                        shell.current_session_title = None;
                        shell.baseline_signature = None;
                    }
                    _ => {}
                }
                sync_shell_transcript_from_app(shell, app);
                shell.refresh_runtime_data(app);
                print_inline_status(stdout, shell)?;
            }
        }
        return Ok(());
    }

    write_inline_line(stdout, &format!("[user] {trimmed}"))?;
    stdout
        .flush()
        .map_err(|err| OberonError::Storage(format!("stdout flush failed: {err}")))?;

    let mut assistant_started = false;
    let mut tool_state: BTreeMap<usize, (String, String)> = BTreeMap::new();
    let prompt = trimmed.clone();
    match app.stream_prompt_events(&prompt, &mut |event| match event {
        oberon4_core::DaemonEvent::ChatChunk { content } => {
            if !assistant_started {
                write!(stdout, "[assistant] ").map_err(|err| {
                    OberonError::Storage(format!("inline chat write failed: {err}"))
                })?;
                assistant_started = true;
            }
            write!(stdout, "{content}")
                .map_err(|err| OberonError::Storage(format!("inline chat write failed: {err}")))?;
            stdout
                .flush()
                .map_err(|err| OberonError::Storage(format!("stdout flush failed: {err}")))?;
            Ok(())
        }
        oberon4_core::DaemonEvent::ToolCallDelta { delta } => {
            let _ = merge_tool_ui_delta(&mut tool_state, delta);
            Ok(())
        }
        other => Err(OberonError::Storage(format!(
            "unexpected daemon stream event: {other:?}"
        ))),
    }) {
        Ok(message) => {
            if assistant_started {
                write_inline_line(stdout, "")?;
            } else {
                write_inline_line(stdout, &format!("[assistant] {}", message.content))?;
            }
            app.apply_successful_prompt(&prompt, message.clone());
            let _ = shell.autosave_current_session(app, &prompt, &message.content);
            sync_shell_transcript_from_app(shell, app);
            shell.refresh_runtime_data(app);
            shell.set_activity(ActivityState::Idle, "response complete");
        }
        Err(err) => {
            if assistant_started {
                write_inline_line(stdout, "")?;
            }
            write_inline_line(stdout, &format!("[error] {err}"))?;
            shell.set_activity(ActivityState::Error, err.to_string());
        }
    }
    stdout
        .flush()
        .map_err(|err| OberonError::Storage(format!("stdout flush failed: {err}")))?;
    Ok(())
}

fn init_fullscreen_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>, OberonError> {
    sanitize_terminal_state()?;
    enable_raw_mode()
        .map_err(|err| OberonError::Storage(format!("raw mode enable failed: {err}")))?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        .map_err(|err| OberonError::Storage(format!("enter alt screen failed: {err}")))?;
    Terminal::new(CrosstermBackend::new(stdout))
        .map_err(|err| OberonError::Storage(format!("terminal init failed: {err}")))
}

fn restore_fullscreen_terminal(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<(), OberonError> {
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )
    .map_err(|err| OberonError::Storage(format!("disable mouse capture failed: {err}")))?;
    disable_raw_mode()
        .map_err(|err| OberonError::Storage(format!("raw mode disable failed: {err}")))?;
    Ok(())
}

fn sanitize_terminal_state() -> Result<(), OberonError> {
    let mut stdout = io::stdout();
    execute!(
        stdout,
        DisableMouseCapture,
        LeaveAlternateScreen,
        MoveToColumn(0),
        Clear(ClearType::CurrentLine)
    )
    .map_err(|err| OberonError::Storage(format!("terminal sanitize failed: {err}")))?;
    let _ = disable_raw_mode();
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
    let command = canonical_tui_command(command);

    match command {
        "/help" => Ok(TuiCommand::Help),
        "/quit" => Ok(TuiCommand::Quit),
        "/stop" => Ok(TuiCommand::Stop),
        "/new" => Ok(TuiCommand::New),
        "/status" => Ok(TuiCommand::Status),
        "/sessions" => Ok(TuiCommand::Sessions),
        "/settings" => Ok(TuiCommand::Settings),
        "/accounts" => Ok(TuiCommand::Accounts),
        "/diagnostics" => Ok(TuiCommand::Diagnostics),
        "/more" => Ok(TuiCommand::More),
        "/config" => Ok(TuiCommand::Config),
        "/model" => require_argument(rest, "/model").map(TuiCommand::Model),
        "/endpoint" => require_argument(rest, "/endpoint").map(TuiCommand::Endpoint),
        "/system" => require_argument(rest, "/system").map(TuiCommand::System),
        "/clear" => Ok(TuiCommand::Clear),
        "/save" => require_argument(rest, "/save").map(TuiCommand::Save),
        "/load" => require_argument(rest, "/load").map(TuiCommand::Load),
        "/fork" => require_argument(rest, "/fork").map(TuiCommand::Fork),
        _ => Err(OberonError::Validation(unsupported_command_message(command))),
    }
}

fn canonical_tui_command(command: &str) -> &str {
    match command.to_ascii_lowercase().as_str() {
        "/help" | "/halp" | "/commands" => "/help",
        "/quit" | "/exit" | "/close" => "/quit",
        "/stop" | "/cancel" | "/abort" => "/stop",
        "/new" | "/fresh" | "/start" => "/new",
        "/status" | "/stat" | "/stats" => "/status",
        "/sessions" | "/session" | "/sessoins" | "/sessons" => "/sessions",
        "/settings" | "/setting" | "/setings" | "/settigns" | "/setteings" => "/settings",
        "/accounts" | "/account" | "/acounts" | "/accouts" => "/accounts",
        "/diagnostics" | "/diagnostic" | "/diag" | "/diags" | "/diagnositcs" => {
            "/diagnostics"
        }
        "/more" | "/menu" => "/more",
        "/config" | "/cfg" | "/configuration" => "/config",
        "/model" | "/models" => "/model",
        "/endpoint" | "/endpiont" | "/url" => "/endpoint",
        "/system" | "/prompt" | "/sytem" => "/system",
        "/clear" | "/cls" => "/clear",
        "/save" | "/store" => "/save",
        "/load" | "/open" => "/load",
        "/fork" | "/copy" => "/fork",
        _ => command,
    }
}

fn unsupported_command_message(command: &str) -> String {
    if let Some(suggestion) = suggest_tui_command(command) {
        format!("unsupported TUI command. Did you mean {suggestion}? Use /help")
    } else {
        "unsupported TUI command. Use /help".to_string()
    }
}

fn suggest_tui_command(command: &str) -> Option<&'static str> {
    let normalized = command.trim().to_ascii_lowercase();
    if !normalized.starts_with('/') {
        return None;
    }

    let mut best: Option<(&'static str, usize)> = None;
    for (candidate, _) in COMMAND_CATALOG {
        let distance = edit_distance(&normalized, candidate);
        let threshold = normalized
            .len()
            .max(candidate.len())
            .clamp(2, 4);
        let shared_prefix = normalized
            .chars()
            .zip(candidate.chars())
            .take_while(|(left, right)| left == right)
            .count();
        if distance <= threshold && shared_prefix >= 2 {
            match best {
                Some((_, best_distance)) if distance >= best_distance => {}
                _ => best = Some((candidate, distance)),
            }
        }
    }

    best.map(|(candidate, _)| candidate)
}

fn edit_distance(left: &str, right: &str) -> usize {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0; right.len() + 1];

    for (i, left_char) in left.iter().enumerate() {
        current[0] = i + 1;
        for (j, right_char) in right.iter().enumerate() {
            let substitution_cost = if left_char == right_char { 0 } else { 1 };
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + substitution_cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous[right.len()]
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

fn provider_name_for_oauth(provider: OAuthProviderKind) -> &'static str {
    match provider {
        OAuthProviderKind::OpenAi => "OpenAI",
        OAuthProviderKind::Anthropic => "Anthropic",
    }
}

fn compact_session_title(title: &str, selected: bool, tick: u64) -> String {
    let normalized = title.trim();
    if normalized.is_empty() {
        return "untitled".to_string();
    }
    let chars = normalized.chars().collect::<Vec<_>>();
    if chars.len() <= SESSION_VISIBLE_WIDTH {
        return normalized.to_string();
    }
    if !selected {
        return chars[..SESSION_VISIBLE_WIDTH].iter().collect();
    }
    let mut marquee = chars.clone();
    marquee.extend("   ".chars());
    marquee.extend(chars.iter().copied());
    let offset = ((tick / 4) as usize) % (chars.len() + 3);
    marquee
        .into_iter()
        .skip(offset)
        .take(SESSION_VISIBLE_WIDTH)
        .collect()
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

fn open_browser(url: &str) -> Result<(), OberonError> {
    let commands: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("open", &[])]
    } else if cfg!(target_os = "windows") {
        &[("cmd", &["/C", "start"])]
    } else {
        &[("xdg-open", &[]), ("gio", &["open"])]
    };

    for (program, args) in commands {
        let mut command = Command::new(program);
        command.args(*args).arg(url);
        if command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
    }

    Err(OberonError::Storage(format!(
        "failed to open browser automatically; open this URL manually: {url}"
    )))
}

fn parse_redirect_uri(redirect_uri: &str) -> Result<(String, u16, String), OberonError> {
    let without_scheme = redirect_uri
        .strip_prefix("http://")
        .or_else(|| redirect_uri.strip_prefix("https://"))
        .ok_or_else(|| OberonError::Validation("oauth redirect_uri must be http(s)".to_string()))?;
    let mut parts = without_scheme.splitn(2, '/');
    let host_port = parts.next().unwrap_or_default();
    let path = format!("/{}", parts.next().unwrap_or_default());
    let mut host_port_parts = host_port.splitn(2, ':');
    let host = host_port_parts.next().unwrap_or_default().to_string();
    let port = host_port_parts
        .next()
        .unwrap_or("80")
        .parse::<u16>()
        .map_err(|err| OberonError::Validation(format!("invalid oauth callback port: {err}")))?;
    Ok((host, port, path))
}

fn decode_query_component(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = &value[index + 1..index + 3];
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    decoded.push(byte);
                    index += 3;
                } else {
                    decoded.push(bytes[index]);
                    index += 1;
                }
            }
            other => {
                decoded.push(other);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).to_string()
}

fn parse_callback_request(
    request_line: &str,
    expected_path: &str,
) -> Result<oberon4_core::OAuthCallbackPayload, OberonError> {
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    if method != "GET" {
        return Err(OberonError::Validation(
            "oauth callback must use GET".to_string(),
        ));
    }
    let mut target_parts = target.splitn(2, '?');
    let path = target_parts.next().unwrap_or_default();
    if path != expected_path {
        return Err(OberonError::Validation(format!(
            "oauth callback path mismatch: expected {expected_path}, got {path}"
        )));
    }

    let mut state = None;
    let mut code = None;
    let mut return_to = None;
    if let Some(query) = target_parts.next() {
        for pair in query.split('&') {
            let mut kv = pair.splitn(2, '=');
            let key = kv.next().unwrap_or_default();
            let value = decode_query_component(kv.next().unwrap_or_default());
            match key {
                "state" => state = Some(value),
                "code" => code = Some(value),
                "return_to" => return_to = Some(value),
                _ => {}
            }
        }
    }

    Ok(oberon4_core::OAuthCallbackPayload {
        state: state.unwrap_or_default(),
        code: code.unwrap_or_default(),
        return_to,
    })
}

fn start_async_oauth_connect(
    socket_path: PathBuf,
    provider: OAuthProviderKind,
) -> Result<PendingOAuthConnect, OberonError> {
    let (sender, receiver) = mpsc::channel();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let cancel_for_worker = Arc::clone(&cancel_flag);

    thread::spawn(move || {
        let client = DaemonIpcClient::new(&socket_path);
        let result = (|| -> Result<String, OberonError> {
            let start = match client.send(&oberon4_core::DaemonCommand::ConnectAccount { provider }) {
                Ok(oberon4_core::DaemonEvent::OAuthStarted {
                    provider: _,
                    flow_id,
                    url,
                    redirect_uri,
                }) => OAuthConnectStart {
                    provider,
                    flow_id,
                    url,
                    redirect_uri,
                },
                Ok(event) => return Err(protocol_error_inner(event)),
                Err(err) if oauth_connect_requires_daemon_refresh(&err) => {
                    restart_daemon_process(socket_path.clone())?;
                    match client.send(&oberon4_core::DaemonCommand::ConnectAccount { provider })? {
                        oberon4_core::DaemonEvent::OAuthStarted {
                            provider: _,
                            flow_id,
                            url,
                            redirect_uri,
                        } => OAuthConnectStart {
                            provider,
                            flow_id,
                            url,
                            redirect_uri,
                        },
                        event => return Err(protocol_error_inner(event)),
                    }
                }
                Err(err) => return Err(err),
            };

            open_browser(&start.url)?;
            let callback = wait_for_oauth_callback(
                &start.redirect_uri,
                Duration::from_secs(300),
                &cancel_for_worker,
            )?;
            match client.send(&oberon4_core::DaemonCommand::CompleteOAuthConnect {
                flow_id: start.flow_id,
                callback,
            })? {
                oberon4_core::DaemonEvent::Ack { message } => Ok(message),
                event => Err(protocol_error_inner(event)),
            }
        })();

        match result {
            Ok(message) => {
                let _ = sender.send(OAuthUiEvent::Complete(message));
            }
            Err(_err) if cancel_for_worker.load(Ordering::Relaxed) => {}
            Err(err) if err.to_string().contains("oauth callback canceled") => {}
            Err(err) => {
                let _ = sender.send(OAuthUiEvent::Failed(err));
            }
        }
    });

    Ok(PendingOAuthConnect {
        receiver,
        cancel_flag,
    })
}

fn wait_for_oauth_callback(
    redirect_uri: &str,
    timeout: Duration,
    cancel_flag: &AtomicBool,
) -> Result<oberon4_core::OAuthCallbackPayload, OberonError> {
    let (host, port, expected_path) = parse_redirect_uri(redirect_uri)?;
    let bind_host = if host == "localhost" {
        "127.0.0.1".to_string()
    } else {
        host
    };
    let listener = TcpListener::bind((bind_host.as_str(), port))
        .map_err(|err| OberonError::Storage(format!("oauth callback listener failed: {err}")))?;
    listener
        .set_nonblocking(true)
        .map_err(|err| OberonError::Storage(format!("oauth callback nonblocking setup failed: {err}")))?;
    let started_at = Instant::now();
    let (mut stream, _) = loop {
        if cancel_flag.load(Ordering::Relaxed) {
            return Err(OberonError::Validation("oauth callback canceled".to_string()));
        }
        if started_at.elapsed() >= timeout {
            return Err(OberonError::Storage(
                "oauth callback timed out waiting for browser redirect".to_string(),
            ));
        }
        match listener.accept() {
            Ok(connection) => break connection,
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(err) => {
                return Err(OberonError::Storage(format!(
                    "oauth callback accept failed: {err}"
                )));
            }
        }
    };
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|err| OberonError::Storage(format!("oauth callback clone failed: {err}")))?,
    );
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .map_err(|err| OberonError::Storage(format!("oauth callback read failed: {err}")))?;
    let callback = parse_callback_request(request_line.trim_end(), &expected_path)?;
    let body = "<html><body><h1>Oberon4 OAuth complete</h1><p>You can return to the terminal.</p></body></html>";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|err| OberonError::Storage(format!("oauth callback response failed: {err}")))?;
    Ok(callback)
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

fn restart_daemon_process(socket_path: PathBuf) -> Result<(), OberonError> {
    let client = DaemonIpcClient::new(&socket_path);
    let _ = client.send(&oberon4_core::DaemonCommand::Shutdown);
    thread::sleep(Duration::from_millis(100));
    spawn_daemon_process(socket_path.clone())?;

    for _ in 0..40 {
        if client.send(&oberon4_core::DaemonCommand::GetStatus).is_ok() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(50));
    }

    Err(OberonError::Storage(
        "daemon failed to become ready after restart".to_string(),
    ))
}

fn delete_session_requires_daemon_refresh(err: &OberonError) -> bool {
    let message = err.to_string();
    message.contains("unknown variant `delete_session`")
        || message.contains("unknown variant 'delete_session'")
}

fn oauth_connect_requires_daemon_refresh(err: &OberonError) -> bool {
    let message = err.to_string();
    message.contains("missing field `flow_id`")
        || message.contains("missing field 'flow_id'")
        || message.contains("unknown variant `complete_oauth_connect`")
        || message.contains("unknown variant 'complete_oauth_connect'")
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
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc;
    use std::{
        collections::BTreeMap,
        thread,
        time::{Duration, Instant},
    };

    use super::{
        AsyncUiEvent, BinaryLaunchTarget, CODE_BLOCK_BG, DaemonBackedTuiApp, FocusTarget,
        PendingSubmission, ScreenRoute, SessionMatchScore, TuiApp, TuiCommand, TuiShellState,
        UiLine, Workspace,
        build_transcript_rows, build_transcript_viewport, finalize_streamed_assistant,
        cancel_pending_submission,
        format_assistant_markdown, session_match_score,
        handle_async_ui_event, handle_pending_local_command,
        handle_pending_timeout, parse_callback_request, parse_tui_command,
        resolve_binary_launch_target, start_async_submit, status_line,
        wait_for_oauth_callback,
    };
    use oberon4_auth::{AccountRegistry, InMemoryAccountMetadataStore};
    use oberon4_core::{
        AppConfig, ChatMessage, ChatRole, FakeChatProvider, FakeOAuthProvider, InMemorySecretStore,
        OAuthProviderKind, SessionId, SessionSummary,
    };
    use oberon4_daemon::{DaemonApp, DaemonIpcClient, DaemonRuntime, serve_unix_socket};
    use ratatui::{layout::Rect, style::{Color, Modifier}};

    #[test]
    fn status_line_includes_endpoint_and_model() {
        let line = status_line(&AppConfig::default());
        assert!(line.contains("https://api.openai.com"));
        assert!(line.contains("gpt-4.1-mini"));
    }

    #[test]
    fn parses_supported_commands() {
        assert_eq!(parse_tui_command("/help").unwrap(), TuiCommand::Help);
        assert_eq!(parse_tui_command("/halp").unwrap(), TuiCommand::Help);
        assert_eq!(parse_tui_command("/new").unwrap(), TuiCommand::New);
        assert_eq!(parse_tui_command("/fresh").unwrap(), TuiCommand::New);
        assert_eq!(parse_tui_command("/stop").unwrap(), TuiCommand::Stop);
        assert_eq!(parse_tui_command("/cancel").unwrap(), TuiCommand::Stop);
        assert_eq!(
            parse_tui_command("/sessions").unwrap(),
            TuiCommand::Sessions
        );
        assert_eq!(
            parse_tui_command("/sessoins").unwrap(),
            TuiCommand::Sessions
        );
        assert_eq!(
            parse_tui_command("/settings").unwrap(),
            TuiCommand::Settings
        );
        assert_eq!(
            parse_tui_command("/setteings").unwrap(),
            TuiCommand::Settings
        );
        assert_eq!(parse_tui_command("/exit").unwrap(), TuiCommand::Quit);
        assert_eq!(
            parse_tui_command("/diagnositcs").unwrap(),
            TuiCommand::Diagnostics
        );
        assert_eq!(
            parse_tui_command("/account").unwrap(),
            TuiCommand::Accounts
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
    fn fuzzy_unknown_commands_offer_suggestions() {
        let err = parse_tui_command("/settngs").unwrap_err();
        assert_eq!(
            err.to_string(),
            "validation failed: unsupported TUI command. Did you mean /settings? Use /help"
        );

        let err = parse_tui_command("/sesions").unwrap_err();
        assert_eq!(
            err.to_string(),
            "validation failed: unsupported TUI command. Did you mean /sessions? Use /help"
        );
    }

    #[test]
    fn config_and_status_commands_return_redacted_state() {
        let mut app = TuiApp::default();
        let status = app.handle_command(TuiCommand::Status).unwrap();
        let config = app.handle_command(TuiCommand::Config).unwrap();

        assert!(status.output.contains("running=false"));
        assert!(status.output.contains("request_endpoint="));
        assert!(status.output.contains("oberon4_account="));
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
    fn exact_quit_command_does_not_autocomplete_on_enter() {
        let mut shell = TuiShellState::default();
        for ch in "/quit".chars() {
            shell.push_input_char(ch);
        }

        assert!(shell.autocomplete.is_some());
        assert!(!shell.should_accept_autocomplete_on_enter());
    }

    #[test]
    fn argument_commands_still_expand_on_enter_autocomplete() {
        let mut shell = TuiShellState::default();
        for ch in "/model".chars() {
            shell.push_input_char(ch);
        }

        assert!(shell.autocomplete.is_some());
        assert!(shell.should_accept_autocomplete_on_enter());
        assert!(shell.accept_autocomplete());
        assert_eq!(shell.input(), "/model ");
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
    fn composer_history_recalls_latest_and_restores_scratch_draft() {
        let mut shell = TuiShellState::default();
        shell.push_input_history_entry("first");
        shell.push_input_history_entry("second");
        shell.input = "draft".to_string();

        assert!(shell.recall_input_history_older());
        assert_eq!(shell.input(), "second");

        assert!(shell.recall_input_history_older());
        assert_eq!(shell.input(), "first");

        assert!(shell.recall_input_history_newer());
        assert_eq!(shell.input(), "second");

        assert!(shell.recall_input_history_newer());
        assert_eq!(shell.input(), "draft");
    }

    #[test]
    fn typing_after_history_recall_exits_history_navigation() {
        let mut shell = TuiShellState::default();
        shell.push_input_history_entry("first");
        shell.input = "draft".to_string();

        assert!(shell.recall_input_history_older());
        assert_eq!(shell.input(), "first");

        shell.push_input_char('!');
        assert_eq!(shell.input(), "first!");
        assert!(shell.input_history_index.is_none());
    }

    #[test]
    fn shell_workspace_escape_returns_to_chat_composer() {
        let mut shell = TuiShellState::default();
        shell.set_workspace(Workspace::Settings);
        shell.push_route(ScreenRoute::SettingsRuntime, FocusTarget::Menu);
        assert!(matches!(shell.workspace, Workspace::Settings));
        assert!(matches!(
            shell.current_route(),
            ScreenRoute::SettingsRuntime
        ));

        shell.handle_escape();
        assert!(matches!(shell.current_route(), ScreenRoute::SettingsRoot));
        assert!(matches!(shell.focus, FocusTarget::Menu));

        shell.handle_escape();
        assert!(matches!(shell.focus, FocusTarget::Composer));
        assert!(matches!(shell.workspace, Workspace::Chat));
    }

    #[test]
    fn shell_session_navigation_uses_list_focus() {
        let mut shell = TuiShellState::default();
        shell.set_workspace(Workspace::Sessions);
        shell.sessions = vec![
            SessionSummary {
                id: SessionId::new("first"),
                title: "first".to_string(),
                provider: oberon4_core::ChatProviderKind::OpenAiCompatible,
                model: "gpt-5.1-codex-mini".to_string(),
            },
            SessionSummary {
                id: SessionId::new("second"),
                title: "second".to_string(),
                provider: oberon4_core::ChatProviderKind::OpenAiCompatible,
                model: "gpt-5.1-codex-mini".to_string(),
            },
        ];

        shell.move_session_selection(1);

        assert!(matches!(shell.focus, FocusTarget::List));
        assert_eq!(shell.session_index, 1);
    }

    #[test]
    fn shell_workspace_tracks_dirty_state() {
        let mut shell = TuiShellState::default();
        shell.set_workspace(Workspace::Sessions);
        assert!(matches!(shell.focus, FocusTarget::List));

        shell.transcript.push(UiLine {
            role: "user".to_string(),
            content: "hello".to_string(),
        });
        assert!(shell.is_dirty());
        shell.mark_loaded_session(Some("demo".to_string()), Some("demo".to_string()));
        assert!(!shell.is_dirty());
        shell.transcript.push(UiLine {
            role: "assistant".to_string(),
            content: "reply".to_string(),
        });
        assert!(shell.is_dirty());
    }

    #[test]
    fn transcript_scroll_controls_toggle_follow_mode() {
        let mut shell = TuiShellState::default();
        assert!(shell.transcript_follow);

        shell.scroll_transcript_by(10);
        assert!(!shell.transcript_follow);
        assert_eq!(shell.transcript_scroll, 10);

        shell.scroll_transcript_to_top();
        assert!(!shell.transcript_follow);
        assert_eq!(shell.transcript_scroll, 0);

        shell.scroll_transcript_to_bottom();
        assert!(shell.transcript_follow);
    }

    #[test]
    fn transcript_viewport_shows_scrollbar_for_overflow() {
        let mut shell = TuiShellState::default();
        for idx in 0..20 {
            shell.transcript.push(UiLine {
                role: "assistant".to_string(),
                content: format!("line {idx}"),
            });
        }

        let viewport = build_transcript_viewport(
            Rect {
                x: 0,
                y: 0,
                width: 40,
                height: 6,
            },
            &shell,
        );

        assert!(viewport.scrollbar_visible);
        assert!(viewport.scrollable_rows > 1);
    }

    #[test]
    fn transcript_viewport_follow_mode_tracks_bottom_position() {
        let mut shell = TuiShellState::default();
        for idx in 0..20 {
            shell.transcript.push(UiLine {
                role: "assistant".to_string(),
                content: format!("line {idx}"),
            });
        }
        shell.transcript_follow = true;

        let viewport = build_transcript_viewport(
            Rect {
                x: 0,
                y: 0,
                width: 40,
                height: 6,
            },
            &shell,
        );

        assert_eq!(viewport.scroll, viewport.scrollable_rows.saturating_sub(1));
    }

    #[test]
    fn assistant_markdown_formats_code_blocks_and_inline_code() {
        let lines = format_assistant_markdown(
            "## Example\nUse `sqrt` here.\n```python\n    print('hi')\n```",
        );

        assert!(lines[0]
            .spans
            .iter()
            .any(|span| span.style.add_modifier.contains(Modifier::BOLD)));
        assert!(lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .any(|span| span.content.contains("sqrt") && span.style.bg == Some(Color::DarkGray)));
        assert!(lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .any(|span| span.content.contains("print('hi')") && span.style.bg == Some(CODE_BLOCK_BG)));
    }

    #[test]
    fn assistant_markdown_strips_bold_markers_and_styles_text() {
        let lines = format_assistant_markdown("- **Word Association** test");
        let flattened: String = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect();

        assert!(!flattened.contains("**"));
        assert!(lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .any(|span| span.content.contains("Word Association")
                && span.style.add_modifier.contains(Modifier::BOLD)));
    }

    #[test]
    fn assistant_markdown_strips_bold_markers_for_bulleted_game_list_shape() {
        let lines = format_assistant_markdown(
            "• **Word Association** — I say a word, you respond with the first word that comes to mind.",
        );
        let flattened: String = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect();

        assert!(!flattened.contains("**"));
        assert!(flattened.contains("• "));
        assert!(lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .any(|span| span.content.contains("Word Association")
                && span.style.add_modifier.contains(Modifier::BOLD)));
    }

    #[test]
    fn transcript_rows_render_assistant_markdown_with_role_prefix() {
        let mut shell = TuiShellState::default();
        shell.transcript.push(UiLine {
            role: "assistant".to_string(),
            content: "- one\n- two".to_string(),
        });

        let rows = build_transcript_rows(&shell, 40);

        assert!(rows[0]
            .spans
            .iter()
            .any(|span| span.content.contains("[assistant]")));
        assert!(rows
            .iter()
            .flat_map(|line| line.spans.iter())
            .any(|span| span.content.contains("• ")));
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

        assert_eq!(shell.status(), "help overlay opened");
        assert!(matches!(shell.workspace, Workspace::Help));

        let client = DaemonIpcClient::new(&socket_path);
        let _ = client.send(&oberon4_core::DaemonCommand::Shutdown).unwrap();
        handle.join().unwrap().unwrap();
    }

    #[test]
    fn submit_adds_prompts_and_commands_to_input_history() {
        let socket_path = std::env::temp_dir().join(format!(
            "oberon4-tui-shell-history-test-{}.sock",
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
            input: "hello there".to_string(),
            ..TuiShellState::default()
        };
        shell.submit(&mut app).unwrap();
        shell.input = "/help".to_string();
        shell.submit(&mut app).unwrap();
        shell.input = "   ".to_string();
        shell.submit(&mut app).unwrap();

        assert_eq!(
            shell.input_history,
            vec!["hello there".to_string(), "/help".to_string()]
        );

        let client = DaemonIpcClient::new(&socket_path);
        let _ = client.send(&oberon4_core::DaemonCommand::Shutdown).unwrap();
        handle.join().unwrap().unwrap();
    }

    #[test]
    fn shell_submit_invalid_command_stays_in_tui() {
        let socket_path = std::env::temp_dir().join(format!(
            "oberon4-tui-shell-invalid-command-test-{}.sock",
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
            input: "/bogus".to_string(),
            ..TuiShellState::default()
        };

        shell.submit(&mut app).unwrap();

        assert_eq!(
            shell.status(),
            "validation failed: unsupported TUI command. Use /help"
        );
        assert!(!shell.should_quit());
        assert!(shell.transcript().is_empty());

        let client = DaemonIpcClient::new(&socket_path);
        let _ = client.send(&oberon4_core::DaemonCommand::Shutdown).unwrap();
        handle.join().unwrap().unwrap();
    }

    #[test]
    fn oauth_callback_parser_extracts_code_and_state() {
        let callback = parse_callback_request(
            "GET /auth/callback?code=demo-code&state=demo-state HTTP/1.1",
            "/auth/callback",
        )
        .unwrap();

        assert_eq!(callback.code, "demo-code");
        assert_eq!(callback.state, "demo-state");
        assert_eq!(callback.return_to, None);
    }

    #[test]
    fn oauth_callback_wait_can_be_canceled() {
        let cancel = AtomicBool::new(true);
        let err = wait_for_oauth_callback(
            "http://localhost:0/auth/callback",
            Duration::from_millis(100),
            &cancel,
        )
        .unwrap_err();

        assert!(err.to_string().contains("oauth callback canceled"));
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

        assert_eq!(shell.status(), "sessions focused");
        assert!(matches!(shell.workspace, Workspace::Chat));
        assert!(matches!(shell.focus, FocusTarget::List));
        assert_eq!(shell.session_sidebar_items().len(), 1);

        let client = DaemonIpcClient::new(&socket_path);
        let _ = client.send(&oberon4_core::DaemonCommand::Shutdown).unwrap();
        handle.join().unwrap().unwrap();
    }

    #[test]
    fn session_rail_delete_removes_saved_session() {
        let socket_path = std::env::temp_dir().join(format!(
            "oberon4-tui-delete-session-test-{}.sock",
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
        app.handle_command(TuiCommand::Save("demo".to_string())).unwrap();

        let mut shell = TuiShellState::default();
        shell.refresh_runtime_data(&app);
        shell.focus = FocusTarget::List;
        shell.session_index = 1;

        let deleted = shell.delete_selected_session(&mut app).unwrap();
        assert!(deleted);
        assert_eq!(shell.sessions.len(), 0);
        assert!(shell.status().contains("deleted session demo"));

        let client = DaemonIpcClient::new(&socket_path);
        let _ = client.send(&oberon4_core::DaemonCommand::Shutdown).unwrap();
        handle.join().unwrap().unwrap();
    }

    #[test]
    fn session_rail_selection_previews_and_escape_restores_active_chat() {
        let socket_path = std::env::temp_dir().join(format!(
            "oberon4-tui-session-preview-test-{}.sock",
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
        app.handle_input("saved prompt").unwrap();
        app.handle_command(TuiCommand::Save("saved-one".to_string()))
            .unwrap();
        app.handle_command(TuiCommand::Clear).unwrap();
        app.handle_input("active prompt").unwrap();

        let mut shell = TuiShellState::default();
        shell.refresh_runtime_data(&app);
        shell.restore_active_chat_preview(&app);
        shell.focus = FocusTarget::List;
        shell.session_index = 0;

        assert!(
            shell
                .transcript()
                .iter()
                .any(|line| line.content.contains("active prompt"))
        );

        shell.move_session_selection(1);
        shell.preview_selected_session(&app).unwrap();
        assert!(
            shell
                .transcript()
                .iter()
                .any(|line| line.content.contains("saved prompt"))
        );

        shell.restore_active_chat_preview(&app);
        shell.handle_escape();
        assert!(matches!(shell.focus, FocusTarget::Composer));
        assert!(
            shell
                .transcript()
                .iter()
                .any(|line| line.content.contains("active prompt"))
        );

        let client = DaemonIpcClient::new(&socket_path);
        let _ = client.send(&oberon4_core::DaemonCommand::Shutdown).unwrap();
        handle.join().unwrap().unwrap();
    }

    #[test]
    fn session_picker_matcher_prioritizes_substring_prefix_and_fuzzy() {
        assert_eq!(
            session_match_score("help", "Need help soon"),
            Some(SessionMatchScore::Substring(5))
        );
        assert_eq!(
            session_match_score("word", "alpha wordplay"),
            Some(SessionMatchScore::Substring(6))
        );
        assert_eq!(
            session_match_score("bed", "big empty den"),
            Some(SessionMatchScore::Fuzzy(10))
        );
        assert_eq!(session_match_score("zzz", "alpha beta"), None);
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
        assert_eq!(shell.transcript()[1].role, "assistant");
        assert_eq!(shell.transcript()[1].content, "...");

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
        assert_eq!(shell.status(), "response complete");
        assert_eq!(app.transcript().len(), 2);
        assert_eq!(shell.transcript().len(), 2);
        assert_eq!(shell.transcript()[0].role, "user");
        assert_eq!(shell.transcript()[1].role, "assistant");
        assert_eq!(shell.transcript()[1].content, "daemon reply");

        let client = DaemonIpcClient::new(&socket_path);
        let _ = client.send(&oberon4_core::DaemonCommand::Shutdown).unwrap();
        handle.join().unwrap().unwrap();
    }

    #[test]
    fn finalize_streamed_assistant_prefers_non_empty_final_message() {
        let mut shell = TuiShellState::default();
        shell.transcript.push(UiLine {
            role: "assistant".to_string(),
            content: "Hello! Hello!".to_string(),
        });

        let pending = PendingSubmission {
            receiver: mpsc::channel().1,
            started_at: Instant::now(),
            timeout: Duration::from_secs(1),
            assistant_line_index: 0,
            tool_line_indices: BTreeMap::new(),
            tool_state: BTreeMap::new(),
        };
        let message = ChatMessage {
            role: ChatRole::Assistant,
            content: "Hello!".to_string(),
            tool_calls: Vec::new(),
        };

        finalize_streamed_assistant(&mut shell, &pending, &message);

        assert_eq!(shell.transcript[0].content, "Hello!");
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

        shell.input = "/stop".to_string();
        assert!(handle_pending_local_command(&mut shell).unwrap());
        assert_eq!(shell.status(), "response stopped");
        assert!(!shell.should_quit());

        shell.input = "/quit".to_string();
        assert!(handle_pending_local_command(&mut shell).unwrap());
        assert!(shell.should_quit());
    }

    #[test]
    fn cancel_pending_submission_stops_stream_and_keeps_partial_reply() {
        let mut shell = TuiShellState::default();
        shell.transcript.push(UiLine {
            role: "user".to_string(),
            content: "hello".to_string(),
        });
        shell.transcript.push(UiLine {
            role: "assistant".to_string(),
            content: "partial".to_string(),
        });
        let mut pending = Some(PendingSubmission {
            receiver: mpsc::channel().1,
            started_at: Instant::now(),
            timeout: Duration::from_secs(1),
            assistant_line_index: 1,
            tool_line_indices: BTreeMap::new(),
            tool_state: BTreeMap::new(),
        });

        assert!(cancel_pending_submission(&mut pending, &mut shell));
        assert!(pending.is_none());
        assert_eq!(shell.status(), "response stopped");
        assert_eq!(shell.transcript().len(), 2);
        assert_eq!(shell.transcript()[1].content, "partial");
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
