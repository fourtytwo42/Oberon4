pub mod plugin_runtime;
pub mod plugin_runtime_broker;
pub mod plugin_runtime_data;
pub mod plugin_runtime_exec;
pub mod plugin_runtime_versioned;

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
    AppConfig, ChatMessage, ChatProviderKind, ChatRole, ChatStreamEvent, ConfigStore,
    DaemonCommand, DaemonEvent, DaemonStatus, FileConfigStore, FileSecretStore, FileSessionStore,
    HostToolRuntime, InMemoryConfigStore, InMemorySessionStore, InvokeRequest, OAuthProvider,
    OAuthProviderKind, OberonError, PendingOAuthFlow, PlanRequest, ProviderCapabilityMatrix,
    ProviderToolDefinition, RequestId, SearchRequest, SecretStore, ServiceEnvelope, SessionStore,
    ToolAwareChatProvider, ToolAwareProviderResponse, ToolDescribeRequest, ToolError, ToolResult,
};
use serde_json::json;

pub use plugin_runtime::{
    ArtifactBrokerFoundation, FilePluginRegistry, HOST_RUNTIME_API_VERSION, PluginRuntimeHost,
    RegisteredPlugin, RuntimeBrokerFoundations, StaticPolicyResolver, StorageBrokerFoundation,
    install_plugin_archive, list_installed_plugins, set_plugin_enabled, unload_plugin_archive,
    validate_manifest,
};
pub use plugin_runtime_broker::{
    EncryptedLocalSecretBackend, InMemoryServiceTransport, LinuxSecretBackend, MacOsSecretBackend,
    NativeSecretBackendUnavailable, PluginRuntimeRedactor, PluginRuntimeSecretBroker,
    PluginRuntimeServiceBroker, ReqwestServiceTransport, SecretBackend, SecretBinding,
    ServiceBinding, ServiceTransport, WindowsSecretBackend,
};
pub use plugin_runtime_data::{PluginRuntimeArtifactBroker, PluginRuntimeWorkspaceBroker};
pub use plugin_runtime_exec::ExecutionEngine;
pub use plugin_runtime_versioned::PluginRuntimeVersionedPackage;

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
        active_provider: config.endpoint.provider,
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

#[derive(Debug, Clone)]
struct PendingApprovalTurn {
    transcript: Vec<ChatMessage>,
    endpoint: String,
    model: String,
    tool_call_id: String,
    invoke_request: InvokeRequest,
}

pub struct ProviderRouter<OP, AP> {
    active_provider: ChatProviderKind,
    openai: OP,
    anthropic: AP,
}

impl<OP, AP> ProviderRouter<OP, AP> {
    pub fn new(active_provider: ChatProviderKind, openai: OP, anthropic: AP) -> Self {
        Self {
            active_provider,
            openai,
            anthropic,
        }
    }
}

impl<OP, AP> ToolAwareChatProvider for ProviderRouter<OP, AP>
where
    OP: ToolAwareChatProvider,
    AP: ToolAwareChatProvider,
{
    fn stream_tool_aware_message(
        &self,
        endpoint: &str,
        model: &str,
        messages: &[ChatMessage],
        tools: &[ProviderToolDefinition],
        on_event: &mut dyn FnMut(ChatStreamEvent) -> Result<(), OberonError>,
    ) -> Result<ToolAwareProviderResponse, OberonError> {
        match self.active_provider {
            ChatProviderKind::OpenAiCompatible => self
                .openai
                .stream_tool_aware_message(endpoint, model, messages, tools, on_event),
            ChatProviderKind::Anthropic => self
                .anthropic
                .stream_tool_aware_message(endpoint, model, messages, tools, on_event),
        }
    }

    fn provider_capabilities(&self) -> ProviderCapabilityMatrix {
        match self.active_provider {
            ChatProviderKind::OpenAiCompatible => self.openai.provider_capabilities(),
            ChatProviderKind::Anthropic => self.anthropic.provider_capabilities(),
        }
    }

    fn apply_runtime_config(&mut self, config: &AppConfig) -> Result<(), OberonError> {
        self.active_provider = config.endpoint.provider;
        self.openai.apply_runtime_config(config)?;
        self.anthropic.apply_runtime_config(config)?;
        Ok(())
    }

    fn endpoint_health(&self, endpoint: &str, model: &str) -> Result<(), OberonError> {
        match self.active_provider {
            ChatProviderKind::OpenAiCompatible => self.openai.endpoint_health(endpoint, model),
            ChatProviderKind::Anthropic => self.anthropic.endpoint_health(endpoint, model),
        }
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

pub struct DaemonApp<C, S, P, OO, AO, GO, SS, AS> {
    runtime: DaemonRuntime<C, S>,
    chat_provider: P,
    openai_oauth: OO,
    anthropic_oauth: AO,
    github_oauth: GO,
    accounts: AccountRegistry,
    pending_oauth_flows: BTreeMap<String, PendingOAuthFlow>,
    pending_approval_turns: BTreeMap<String, PendingApprovalTurn>,
    secrets: SS,
    account_store: AS,
}

impl Default
    for DaemonApp<
        FileConfigStore,
        FileSessionStore,
        ProviderRouter<
            oberon4_provider_openai::OpenAiCompatibleClient<
                oberon4_provider_openai::ReqwestTransport,
            >,
            oberon4_provider_anthropic::AnthropicClient<
                oberon4_provider_anthropic::ReqwestTransport,
            >,
        >,
        oberon4_auth::OpenAiOAuthAdapter,
        oberon4_auth::AnthropicOAuthAdapter,
        oberon4_auth::GithubOAuthAdapter,
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
        let anthropic = oberon4_provider_anthropic::AnthropicClient::new(
            config.endpoint.base_url.clone(),
            config.endpoint.api_key_env.clone(),
            oberon4_provider_anthropic::RetryPolicy::from_config(&config),
            oberon4_provider_anthropic::ReqwestTransport,
        );
        Self::new(
            DaemonRuntime {
                config_store: FileConfigStore::default(),
                session_store: FileSessionStore::default(),
                lifecycle: LifecycleState::Stopped,
            },
            ProviderRouter::new(config.endpoint.provider, client, anthropic),
            oberon4_auth::OpenAiOAuthAdapter::default(),
            oberon4_auth::AnthropicOAuthAdapter::default(),
            oberon4_auth::GithubOAuthAdapter::new(),
            accounts,
            FileSecretStore::default(),
            account_store,
        )
    }
}

impl<C, S, P, OO, AO, GO, SS, AS> DaemonApp<C, S, P, OO, AO, GO, SS, AS>
where
    C: ConfigStore,
    S: SessionStore,
    P: ToolAwareChatProvider,
    OO: OAuthProvider,
    AO: OAuthProvider,
    GO: OAuthProvider,
    SS: SecretStore + Clone + Send + Sync + 'static,
    AS: AccountMetadataStore,
{
    pub fn new(
        runtime: DaemonRuntime<C, S>,
        chat_provider: P,
        openai_oauth: OO,
        anthropic_oauth: AO,
        github_oauth: GO,
        accounts: AccountRegistry,
        secrets: SS,
        account_store: AS,
    ) -> Self {
        Self {
            runtime,
            chat_provider,
            openai_oauth,
            anthropic_oauth,
            github_oauth,
            accounts,
            pending_oauth_flows: BTreeMap::new(),
            pending_approval_turns: BTreeMap::new(),
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

    fn build_runtime_host(
        &self,
    ) -> Result<PluginRuntimeHost<FilePluginRegistry, StaticPolicyResolver>, OberonError> {
        let config = self.runtime.load_config()?;
        let plugin_archive_roots = config
            .runtime
            .plugin_archive_roots
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        let plugin_config_root = PathBuf::from(&config.runtime.plugin_config_root);
        let plugin_roots = config
            .runtime
            .plugin_roots
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        let package_roots = config
            .runtime
            .package_roots
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        for root in &plugin_archive_roots {
            PluginRuntimeVersionedPackage::ensure_default_package_archive_installed(
                root,
                &plugin_config_root,
            )?;
        }
        let registry = FilePluginRegistry::load_from_runtime_sources(
            &plugin_archive_roots,
            &plugin_config_root,
            &config
                .runtime
                .enabled_plugins
                .iter()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>(),
            &config
                .runtime
                .disabled_plugins
                .iter()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>(),
            &plugin_roots,
            &package_roots,
            &config
                .runtime
                .enabled_packages
                .iter()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>(),
            &config
                .runtime
                .disabled_packages
                .iter()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>(),
            HOST_RUNTIME_API_VERSION,
        )?;
        let policy = StaticPolicyResolver::for_registered_plugins(
            config.runtime.operating_mode.clone(),
            config.runtime.allowed_plugin_checksums.clone(),
            registry.all_registered_plugins(),
        );
        let mut secret_bindings = Vec::new();
        let mut service_bindings = Vec::new();
        if let Some(account_id) = self.accounts.active_account_id(OAuthProviderKind::Github) {
            secret_bindings.push(SecretBinding {
                secret_handle: "sec_github".to_string(),
                store_key: format!("oauth.github.{account_id}.access_token"),
                secret_class: oberon4_core::SecretClass::OauthCredential,
                allowed_operations: vec![oberon4_core::HandleOperation::Invoke],
            });
            service_bindings.push(ServiceBinding {
                service_handle: "svc_github".to_string(),
                service_name: "github.request".to_string(),
                secret_handle: "sec_github".to_string(),
                base_url: "https://api.github.com".to_string(),
                allowed_operations: vec![oberon4_core::HandleOperation::Invoke],
            });
        }
        let secret_broker = std::sync::Arc::new(PluginRuntimeSecretBroker::new(
            EncryptedLocalSecretBackend::new(self.secrets.clone()),
            secret_bindings.clone(),
        ));
        let service_secret_bindings = secret_bindings.clone();
        let service_broker = std::sync::Arc::new(PluginRuntimeServiceBroker::new(
            PluginRuntimeSecretBroker::new(
                EncryptedLocalSecretBackend::new(self.secrets.clone()),
                service_secret_bindings,
            ),
            service_bindings,
            ReqwestServiceTransport,
        ));
        PluginRuntimeHost::with_runtime_services(
            registry,
            policy,
            RuntimeBrokerFoundations::default(),
            secret_broker,
            service_broker,
            std::sync::Arc::new(PluginRuntimeRedactor),
        )
    }

    fn provider_tool_definitions() -> Vec<ProviderToolDefinition> {
        vec![
            ProviderToolDefinition {
                name: "tool_search".to_string(),
                description: "Search installed tools and commands by capability.".to_string(),
                parameters_schema: json!({
                    "type":"object",
                    "properties":{
                        "query":{"type":"string"},
                        "limit":{"type":"integer"}
                    },
                    "required":["query"]
                }),
            },
            ProviderToolDefinition {
                name: "tool_describe".to_string(),
                description: "Describe one tool command with guidance and schemas.".to_string(),
                parameters_schema: json!({
                    "type":"object",
                    "properties":{
                        "tool":{"type":"string"},
                        "command":{"type":"string"}
                    },
                    "required":["tool","command"]
                }),
            },
            ProviderToolDefinition {
                name: "tool_plan".to_string(),
                description: "Plan a tool command before invoking it.".to_string(),
                parameters_schema: json!({
                    "type":"object",
                    "properties":{
                        "tool":{"type":"string"},
                        "command":{"type":"string"},
                        "input":{"type":"object"}
                    },
                    "required":["tool","command","input"]
                }),
            },
            ProviderToolDefinition {
                name: "tool_invoke".to_string(),
                description: "Invoke a specific tool command.".to_string(),
                parameters_schema: json!({
                    "type":"object",
                    "properties":{
                        "tool":{"type":"string"},
                        "command":{"type":"string"},
                        "input":{"type":"object"}
                    },
                    "required":["tool","command","input"]
                }),
            },
            ProviderToolDefinition {
                name: "tool_policy_inspect".to_string(),
                description: "Inspect policy and approval requirements for a tool command."
                    .to_string(),
                parameters_schema: json!({
                    "type":"object",
                    "properties":{
                        "tool":{"type":"string"},
                        "command":{"type":"string"}
                    },
                    "required":["tool","command"]
                }),
            },
        ]
    }

    fn invocation_context(&self, tool: &str, command: &str) -> oberon4_core::InvocationContext {
        oberon4_core::InvocationContext {
            request_id: RequestId::new(),
            session_id: "chat".to_string(),
            plugin_id: tool.to_string(),
            plugin_version: "builtin".to_string(),
            command: command.to_string(),
            deadline: "2099-01-01T00:00:00Z".to_string(),
            runtime_api_version: HOST_RUNTIME_API_VERSION.to_string(),
            capability_grants: Vec::new(),
            workspace_revision: None,
            memory_limit: 128 * 1024 * 1024,
            time_limit_ms: 5_000,
            output_limit_bytes: 128 * 1024,
            storage_limit_bytes: 256 * 1024 * 1024,
            network_policy: oberon4_core::NetworkPolicy::BrokeredOnly,
        }
    }

    fn parse_required_string(
        input: &serde_json::Value,
        field: &str,
    ) -> Result<String, OberonError> {
        input
            .get(field)
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .ok_or_else(|| OberonError::Validation(format!("missing required field: {field}")))
    }

    fn tool_result_message(
        tool_call_id: String,
        value: &serde_json::Value,
    ) -> Result<ChatMessage, OberonError> {
        Ok(ChatMessage {
            role: ChatRole::Tool,
            content: serde_json::to_string(value)
                .map_err(|err| OberonError::Storage(format!("tool result encode failed: {err}")))?,
            tool_call_id: Some(tool_call_id),
            tool_calls: Vec::new(),
        })
    }

    fn append_tool_result(
        transcript: &mut Vec<ChatMessage>,
        tool_call_id: String,
        result: &ToolResult,
    ) -> Result<(), OberonError> {
        let value = serde_json::to_value(result)
            .map_err(|err| OberonError::Storage(format!("tool result serialize failed: {err}")))?;
        transcript.push(Self::tool_result_message(tool_call_id, &value)?);
        Ok(())
    }

    fn append_tool_error(
        transcript: &mut Vec<ChatMessage>,
        tool_call_id: String,
        error: &ToolError,
    ) -> Result<(), OberonError> {
        let value = serde_json::to_value(error)
            .map_err(|err| OberonError::Storage(format!("tool error serialize failed: {err}")))?;
        transcript.push(Self::tool_result_message(tool_call_id, &value)?);
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
            DaemonCommand::ResolveApproval { .. } => Err(OberonError::Validation(
                "ResolveApproval must be handled through the streaming IPC path".to_string(),
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
            DaemonCommand::ListPlugins => {
                let config = self.runtime.load_config()?;
                for root in config
                    .runtime
                    .plugin_archive_roots
                    .iter()
                    .map(PathBuf::from)
                    .collect::<Vec<_>>()
                {
                    PluginRuntimeVersionedPackage::ensure_default_package_archive_installed(
                        &root,
                        Path::new(&config.runtime.plugin_config_root),
                    )?;
                }
                Ok(CommandOutcome {
                    event: DaemonEvent::PluginsListed {
                        plugins: list_installed_plugins(&config)?,
                    },
                    should_shutdown: false,
                })
            }
            DaemonCommand::LoadPluginArchive { archive_path } => {
                let config = self.runtime.load_config()?;
                for root in config
                    .runtime
                    .plugin_archive_roots
                    .iter()
                    .map(PathBuf::from)
                    .collect::<Vec<_>>()
                {
                    PluginRuntimeVersionedPackage::ensure_default_package_archive_installed(
                        &root,
                        Path::new(&config.runtime.plugin_config_root),
                    )?;
                }
                let plugin = install_plugin_archive(&config, Path::new(&archive_path))?;
                Ok(CommandOutcome {
                    event: DaemonEvent::PluginUpdated {
                        message: format!("loaded plugin archive {}", plugin.id),
                        plugin,
                    },
                    should_shutdown: false,
                })
            }
            DaemonCommand::UnloadPlugin { id } => {
                let config = self.runtime.load_config()?;
                let plugin = unload_plugin_archive(&config, &id)?;
                Ok(CommandOutcome {
                    event: DaemonEvent::PluginUpdated {
                        message: format!("unloaded plugin {}", plugin.id),
                        plugin,
                    },
                    should_shutdown: false,
                })
            }
            DaemonCommand::SetPluginEnabled { id, enabled } => {
                let mut config = self.runtime.load_config()?;
                crate::plugin_runtime::normalize_plugin_enablement(&mut config, &id, enabled)?;
                self.runtime.save_config(&config)?;
                let plugin = set_plugin_enabled(&config, &id, enabled)?;
                let action = if enabled { "enabled" } else { "disabled" };
                Ok(CommandOutcome {
                    event: DaemonEvent::PluginUpdated {
                        message: format!("{action} plugin {}", plugin.id),
                        plugin,
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
            OAuthProviderKind::Github => Ok(&self.github_oauth),
        }
    }

    fn active_account(&self) -> Option<String> {
        [
            OAuthProviderKind::OpenAi,
            OAuthProviderKind::Anthropic,
            OAuthProviderKind::Github,
        ]
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

    fn handle_tool_call(
        &mut self,
        transcript: &mut Vec<ChatMessage>,
        tool_call: &oberon4_core::ChatToolCall,
        host: &impl HostToolRuntime,
        on_event: &mut dyn FnMut(DaemonEvent) -> Result<(), OberonError>,
    ) -> Result<bool, OberonError> {
        let input: serde_json::Value =
            serde_json::from_str(&tool_call.arguments).map_err(|err| {
                OberonError::Validation(format!(
                    "invalid tool arguments for {}: {err}",
                    tool_call.name
                ))
            })?;
        match tool_call.name.as_str() {
            "tool_search" => {
                let result = host.tool_search(&SearchRequest {
                    query: Self::parse_required_string(&input, "query")?,
                    limit: input
                        .get("limit")
                        .and_then(|value| value.as_u64())
                        .map(|v| v as u32),
                    session_id: Some("chat".to_string()),
                })?;
                let data = serde_json::to_value(result).map_err(|err| {
                    OberonError::Storage(format!("search serialize failed: {err}"))
                })?;
                let tool_result = ToolResult {
                    status: oberon4_core::InvokeStatus::Ok,
                    tool: "tool_search".to_string(),
                    command: "search".to_string(),
                    summary: "Tool search completed".to_string(),
                    data,
                    artifacts: Vec::new(),
                    truncated: false,
                    next_suggested_action: Some("tool_describe".to_string()),
                };
                on_event(DaemonEvent::ToolResult {
                    result: tool_result.clone(),
                })?;
                Self::append_tool_result(transcript, tool_call.id.clone(), &tool_result)?;
                Ok(true)
            }
            "tool_describe" => {
                let result = host.tool_describe(&ToolDescribeRequest {
                    tool: Self::parse_required_string(&input, "tool")?,
                    command: Self::parse_required_string(&input, "command")?,
                })?;
                let data = serde_json::to_value(result).map_err(|err| {
                    OberonError::Storage(format!("describe serialize failed: {err}"))
                })?;
                let tool_result = ToolResult {
                    status: oberon4_core::InvokeStatus::Ok,
                    tool: "tool_describe".to_string(),
                    command: "describe".to_string(),
                    summary: "Tool description loaded".to_string(),
                    data,
                    artifacts: Vec::new(),
                    truncated: false,
                    next_suggested_action: Some("tool_plan".to_string()),
                };
                on_event(DaemonEvent::ToolResult {
                    result: tool_result.clone(),
                })?;
                Self::append_tool_result(transcript, tool_call.id.clone(), &tool_result)?;
                Ok(true)
            }
            "tool_policy_inspect" => {
                let result = host.tool_policy_inspect(&oberon4_core::PolicyInspectRequest {
                    tool: Self::parse_required_string(&input, "tool")?,
                    command: Self::parse_required_string(&input, "command")?,
                })?;
                let data = serde_json::to_value(result).map_err(|err| {
                    OberonError::Storage(format!("policy inspect serialize failed: {err}"))
                })?;
                let tool_result = ToolResult {
                    status: oberon4_core::InvokeStatus::Ok,
                    tool: "tool_policy_inspect".to_string(),
                    command: "policy_inspect".to_string(),
                    summary: "Policy inspection completed".to_string(),
                    data,
                    artifacts: Vec::new(),
                    truncated: false,
                    next_suggested_action: Some("tool_plan".to_string()),
                };
                on_event(DaemonEvent::ToolResult {
                    result: tool_result.clone(),
                })?;
                Self::append_tool_result(transcript, tool_call.id.clone(), &tool_result)?;
                Ok(true)
            }
            "tool_plan" => {
                let tool = Self::parse_required_string(&input, "tool")?;
                let command = Self::parse_required_string(&input, "command")?;
                let result = host.tool_plan(&PlanRequest {
                    command: command.clone(),
                    input: input.get("input").cloned().unwrap_or_else(|| json!({})),
                    context: self.invocation_context(&tool, &command),
                })?;
                let data = serde_json::to_value(result.clone())
                    .map_err(|err| OberonError::Storage(format!("plan serialize failed: {err}")))?;
                let tool_result = ToolResult {
                    status: oberon4_core::InvokeStatus::Ok,
                    tool: "tool_plan".to_string(),
                    command: "plan".to_string(),
                    summary: "Tool plan completed".to_string(),
                    data,
                    artifacts: Vec::new(),
                    truncated: false,
                    next_suggested_action: Some("tool_invoke".to_string()),
                };
                on_event(DaemonEvent::ToolResult {
                    result: tool_result.clone(),
                })?;
                Self::append_tool_result(transcript, tool_call.id.clone(), &tool_result)?;
                Ok(true)
            }
            "tool_invoke" => {
                let tool = Self::parse_required_string(&input, "tool")?;
                let command = Self::parse_required_string(&input, "command")?;
                let invoke_request = InvokeRequest {
                    command: command.clone(),
                    input: input.get("input").cloned().unwrap_or_else(|| json!({})),
                    context: self.invocation_context(&tool, &command),
                };
                let plan = host.tool_plan(&PlanRequest {
                    command: command.clone(),
                    input: invoke_request.input.clone(),
                    context: invoke_request.context.clone(),
                })?;
                if plan.approval_required {
                    let approval_request_id = RequestId::new();
                    let approval_id = format!("approval-{}", approval_request_id.as_str());
                    let payload_json =
                        serde_json::to_string(&plan.approval_preview).map_err(|err| {
                            OberonError::Storage(format!("approval encode failed: {err}"))
                        })?;
                    self.pending_approval_turns.insert(
                        approval_id.clone(),
                        PendingApprovalTurn {
                            transcript: transcript.clone(),
                            endpoint: self.runtime.load_config()?.endpoint.base_url,
                            model: self.runtime.load_config()?.endpoint.model,
                            tool_call_id: tool_call.id.clone(),
                            invoke_request,
                        },
                    );
                    on_event(DaemonEvent::ApprovalRequired {
                        approval_id: approval_id.clone(),
                        payload_json,
                    })?;
                    on_event(DaemonEvent::ChatPaused { approval_id })?;
                    return Ok(false);
                }
                let envelope = host.tool_invoke(&invoke_request)?;
                if let Some(result) = envelope.result {
                    on_event(DaemonEvent::ToolResult {
                        result: result.clone(),
                    })?;
                    Self::append_tool_result(transcript, tool_call.id.clone(), &result)?;
                } else if let Some(error) = envelope.error {
                    on_event(DaemonEvent::ToolError {
                        error: error.clone(),
                    })?;
                    Self::append_tool_error(transcript, tool_call.id.clone(), &error)?;
                }
                Ok(true)
            }
            other => Err(OberonError::Validation(format!(
                "unknown host tool: {other}"
            ))),
        }
    }

    fn continue_provider_loop(
        &mut self,
        mut transcript: Vec<ChatMessage>,
        endpoint: String,
        model: String,
        on_event: &mut dyn FnMut(DaemonEvent) -> Result<(), OberonError>,
    ) -> Result<(), OberonError> {
        let host = self.build_runtime_host()?;
        let tools = Self::provider_tool_definitions();
        loop {
            let reply = self.chat_provider.stream_tool_aware_message(
                &endpoint,
                &model,
                &transcript,
                &tools,
                &mut |event| match event {
                    ChatStreamEvent::TextDelta { content } => {
                        on_event(DaemonEvent::ChatChunk { content })
                    }
                    ChatStreamEvent::ToolCallDelta(delta) => {
                        on_event(DaemonEvent::ToolCallDelta { delta })
                    }
                },
            )?;
            transcript.push(reply.message.clone());
            if reply.finish_state == oberon4_core::ProviderFinishState::ToolCallRequested
                && !reply.message.tool_calls.is_empty()
            {
                for tool_call in &reply.message.tool_calls {
                    if !self.handle_tool_call(&mut transcript, tool_call, &host, on_event)? {
                        return Ok(());
                    }
                }
                continue;
            }
            on_event(DaemonEvent::ChatComplete {
                message: reply.message,
            })?;
            return Ok(());
        }
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
        self.continue_provider_loop(
            messages,
            config.endpoint.base_url,
            config.endpoint.model,
            on_event,
        )
    }

    fn handle_approval_stream(
        &mut self,
        approval_id: String,
        decision: String,
        on_event: &mut dyn FnMut(DaemonEvent) -> Result<(), OberonError>,
    ) -> Result<(), OberonError> {
        let pending = self
            .pending_approval_turns
            .remove(&approval_id)
            .ok_or_else(|| OberonError::NotFound(format!("approval:{approval_id}")))?;
        let host = self.build_runtime_host()?;
        let mut transcript = pending.transcript;
        if decision.eq_ignore_ascii_case("approve") {
            let envelope = host.tool_invoke(&pending.invoke_request)?;
            if let Some(result) = envelope.result {
                on_event(DaemonEvent::ToolResult {
                    result: result.clone(),
                })?;
                Self::append_tool_result(&mut transcript, pending.tool_call_id.clone(), &result)?;
            } else if let Some(error) = envelope.error {
                on_event(DaemonEvent::ToolError {
                    error: error.clone(),
                })?;
                Self::append_tool_error(&mut transcript, pending.tool_call_id.clone(), &error)?;
            }
        } else {
            let error = ToolError {
                status: oberon4_core::ErrorStatus::Error,
                code: "APPROVAL_DENIED".to_string(),
                message: "approval denied by operator".to_string(),
                retryable: false,
                request_id: RequestId::new(),
                plugin_id: pending.invoke_request.context.plugin_id.clone(),
                plugin_version: pending.invoke_request.context.plugin_version.clone(),
                command: pending.invoke_request.command.clone(),
                next_suggested_action: Some("tool_plan".to_string()),
            };
            on_event(DaemonEvent::ToolError {
                error: error.clone(),
            })?;
            Self::append_tool_error(&mut transcript, pending.tool_call_id.clone(), &error)?;
        }
        self.continue_provider_loop(transcript, pending.endpoint, pending.model, on_event)
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

pub fn serve_unix_socket<C, S, P, OO, AO, GO, SS, AS>(
    socket_path: impl AsRef<Path>,
    mut app: DaemonApp<C, S, P, OO, AO, GO, SS, AS>,
) -> Result<(), OberonError>
where
    C: ConfigStore,
    S: SessionStore,
    P: ToolAwareChatProvider,
    OO: OAuthProvider,
    AO: OAuthProvider,
    GO: OAuthProvider,
    SS: SecretStore + Clone + Send + Sync + 'static,
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

fn handle_stream<C, S, P, OO, AO, GO, SS, AS>(
    stream: &mut UnixStream,
    app: &mut DaemonApp<C, S, P, OO, AO, GO, SS, AS>,
) -> Result<bool, OberonError>
where
    C: ConfigStore,
    S: SessionStore,
    P: ToolAwareChatProvider,
    OO: OAuthProvider,
    AO: OAuthProvider,
    GO: OAuthProvider,
    SS: SecretStore + Clone + Send + Sync + 'static,
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
        Ok(DaemonCommand::ResolveApproval {
            approval_id,
            decision,
        }) => {
            let result = app.handle_approval_stream(approval_id, decision, &mut |event| {
                write_event(stream, &event)
            });
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
        OAuthProviderKind::Github => "github",
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        collections::VecDeque,
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
        AppConfig, ChatMessage, ChatProviderKind, ChatRole, ChatStreamEvent, ChatToolCall,
        DaemonCommand, DaemonEvent, FakeChatProvider, FakeOAuthProvider, FileConfigStore,
        FileSecretStore, FileSessionStore, InMemorySecretStore, OAuthCallbackPayload,
        OAuthProviderKind, OberonError, ProviderCapabilityMatrix, ProviderFinishState,
        ProviderToolDefinition, SavedSession, SecretStore, SessionId, SessionSummary,
        ToolAwareChatProvider, ToolAwareProviderResponse,
    };
    use oberon4_provider_openai::{FakeTransport, OpenAiCompatibleClient, RetryPolicy};

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("oberon4-daemon-{name}-{nonce}"))
    }

    #[derive(Debug)]
    struct ScriptedToolProvider {
        responses: RefCell<VecDeque<ToolAwareProviderResponse>>,
    }

    impl ScriptedToolProvider {
        fn new(responses: Vec<ToolAwareProviderResponse>) -> Self {
            Self {
                responses: RefCell::new(responses.into()),
            }
        }
    }

    impl ToolAwareChatProvider for ScriptedToolProvider {
        fn stream_tool_aware_message(
            &self,
            _endpoint: &str,
            _model: &str,
            _messages: &[ChatMessage],
            _tools: &[ProviderToolDefinition],
            _on_event: &mut dyn FnMut(ChatStreamEvent) -> Result<(), OberonError>,
        ) -> Result<ToolAwareProviderResponse, OberonError> {
            self.responses
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| OberonError::Validation("scripted provider exhausted".to_string()))
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
            FakeOAuthProvider::new(OAuthProviderKind::Github),
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
                tool_call_id: None,
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
    fn host_tool_loop_does_not_reroute_tool_search_as_plugin_lookup() {
        let provider = ScriptedToolProvider::new(vec![
            ToolAwareProviderResponse {
                message: ChatMessage {
                    role: ChatRole::Assistant,
                    content: String::new(),
                    tool_call_id: None,
                    tool_calls: vec![ChatToolCall {
                        id: "call_search".to_string(),
                        name: "tool_search".to_string(),
                        arguments: r#"{"query":"tools","limit":5}"#.to_string(),
                    }],
                },
                finish_state: ProviderFinishState::ToolCallRequested,
            },
            ToolAwareProviderResponse {
                message: ChatMessage {
                    role: ChatRole::Assistant,
                    content: String::new(),
                    tool_call_id: None,
                    tool_calls: vec![ChatToolCall {
                        id: "call_describe".to_string(),
                        name: "tool_describe".to_string(),
                        arguments: r#"{"tool":"tool_search","command":"tool_search"}"#.to_string(),
                    }],
                },
                finish_state: ProviderFinishState::ToolCallRequested,
            },
            ToolAwareProviderResponse {
                message: ChatMessage {
                    role: ChatRole::Assistant,
                    content: String::new(),
                    tool_call_id: None,
                    tool_calls: vec![ChatToolCall {
                        id: "call_plan".to_string(),
                        name: "tool_plan".to_string(),
                        arguments: r#"{"tool":"github","command":"auth_status","input":{}}"#
                            .to_string(),
                    }],
                },
                finish_state: ProviderFinishState::ToolCallRequested,
            },
            ToolAwareProviderResponse {
                message: ChatMessage {
                    role: ChatRole::Assistant,
                    content: String::new(),
                    tool_call_id: None,
                    tool_calls: vec![ChatToolCall {
                        id: "call_invoke".to_string(),
                        name: "tool_invoke".to_string(),
                        arguments: r#"{"tool":"github","command":"auth_status","input":{}}"#
                            .to_string(),
                    }],
                },
                finish_state: ProviderFinishState::ToolCallRequested,
            },
            ToolAwareProviderResponse {
                message: ChatMessage {
                    role: ChatRole::Assistant,
                    content: "done".to_string(),
                    tool_call_id: None,
                    tool_calls: Vec::new(),
                },
                finish_state: ProviderFinishState::ResponseComplete,
            },
        ]);

        let mut app = DaemonApp::new(
            DaemonRuntime::default(),
            provider,
            FakeOAuthProvider::new(OAuthProviderKind::OpenAi),
            FakeOAuthProvider::new(OAuthProviderKind::Anthropic),
            FakeOAuthProvider::new(OAuthProviderKind::Github),
            AccountRegistry::default(),
            InMemorySecretStore::default(),
            InMemoryAccountMetadataStore::default(),
        );
        let _ = app.start().unwrap();

        let mut events = Vec::new();
        app.handle_chat_stream(
            vec![ChatMessage {
                role: ChatRole::User,
                content: "what tools do you have?".to_string(),
                tool_call_id: None,
                tool_calls: Vec::new(),
            }],
            &mut |event| {
                events.push(event);
                Ok(())
            },
        )
        .unwrap();

        assert!(!events.iter().any(|event| matches!(
            event,
            DaemonEvent::Error { message }
            if message.contains("plugin:tool_search")
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            DaemonEvent::ToolResult { result } if result.tool == "tool_search"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            DaemonEvent::ChatComplete { message } if message.content == "done"
        )));
    }

    #[test]
    fn ipc_send_chat_requires_running_daemon() {
        let mut app = DaemonApp::new(
            DaemonRuntime::default(),
            FakeChatProvider::default(),
            FakeOAuthProvider::new(OAuthProviderKind::OpenAi),
            FakeOAuthProvider::new(OAuthProviderKind::Anthropic),
            FakeOAuthProvider::new(OAuthProviderKind::Github),
            AccountRegistry::default(),
            InMemorySecretStore::default(),
            InMemoryAccountMetadataStore::default(),
        );
        let err = app
            .handle_chat_stream(
                vec![ChatMessage {
                    role: ChatRole::User,
                    content: "hello".to_string(),
                    tool_call_id: None,
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
                FakeOAuthProvider::new(OAuthProviderKind::Github),
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
            FakeOAuthProvider::new(OAuthProviderKind::Github),
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
                tool_call_id: None,
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
            FakeOAuthProvider::new(OAuthProviderKind::Github),
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
            FakeOAuthProvider::new(OAuthProviderKind::Github),
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
            FakeOAuthProvider::new(OAuthProviderKind::Github),
            AccountRegistry::default(),
            secrets,
            InMemoryAccountMetadataStore::default(),
        );
        let _ = app.start().unwrap();

        let _ = app
            .handle_command(DaemonCommand::SaveConfig {
                config: AppConfig {
                    endpoint: oberon4_core::EndpointConfig {
                        provider: ChatProviderKind::OpenAiCompatible,
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
                        operating_mode: oberon4_core::OperatingMode::Dev,
                        allowed_plugin_checksums: Vec::new(),
                        plugin_archive_roots: oberon4_core::RuntimeConfig::default()
                            .plugin_archive_roots,
                        plugin_config_root: oberon4_core::RuntimeConfig::default()
                            .plugin_config_root,
                        enabled_plugins: Vec::new(),
                        disabled_plugins: Vec::new(),
                        plugin_roots: oberon4_core::RuntimeConfig::default().plugin_roots,
                        package_roots: oberon4_core::RuntimeConfig::default().package_roots,
                        enabled_packages: Vec::new(),
                        disabled_packages: Vec::new(),
                    },
                },
            })
            .unwrap();

        let mut events = Vec::new();
        app.handle_chat_stream(
            vec![ChatMessage {
                role: ChatRole::User,
                content: "hello".to_string(),
                tool_call_id: None,
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
