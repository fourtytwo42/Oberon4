use std::{
    collections::BTreeMap,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use oberon4_auth::{AccountRegistry, AnthropicOAuthAdapter, OpenAiOAuthAdapter};
use oberon4_core::{
    AppConfig, ChatMessage, ChatProvider, ChatProviderKind, ChatRole, ChatToolCallDelta,
    DaemonCommand, DaemonEvent, FileSecretStore, InMemoryConfigStore, InMemorySecretStore,
    InMemorySessionStore, OAuthProvider, OAuthProviderKind, OberonError, SecretStore, TokenBundle,
    ToolError, ToolResult, default_data_dir,
};
use oberon4_daemon::{DaemonIpcClient, DaemonRuntime, LogLevel, default_socket_path, log_entry};
use oberon4_provider_openai::{
    FakeTransport, OpenAiCompatibleClient, OpenAiTransport, RetryPolicy,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliCommand {
    DaemonStart,
    DaemonStop,
    DaemonStatus,
    Tui {
        fullscreen: bool,
    },
    ConfigShow,
    ConfigSet {
        key: String,
        value: String,
    },
    EndpointTest,
    Chat {
        message: String,
    },
    LogsTail,
    Doctor {
        json: bool,
    },
    PluginList,
    PluginLoad {
        archive_path: String,
    },
    PluginUnload {
        id: String,
    },
    PluginEnable {
        id: String,
    },
    PluginDisable {
        id: String,
    },
    AccountConnect {
        provider: OAuthProviderKind,
    },
    AccountList,
    AccountActivate {
        provider: OAuthProviderKind,
        account: String,
    },
    AccountDisable {
        provider: OAuthProviderKind,
        account: String,
    },
    AccountDisconnect {
        provider: OAuthProviderKind,
        account: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub stdout_emitted: bool,
    pub stderr_emitted: bool,
}

impl CliOutput {
    fn success(stdout: impl Into<String>) -> Self {
        Self {
            stdout: stdout.into(),
            stderr: String::new(),
            exit_code: 0,
            stdout_emitted: false,
            stderr_emitted: false,
        }
    }

    fn error(stderr: impl Into<String>, exit_code: i32) -> Self {
        Self {
            stdout: String::new(),
            stderr: stderr.into(),
            exit_code,
            stdout_emitted: false,
            stderr_emitted: false,
        }
    }
}

pub struct CliApp {
    runtime: DaemonRuntime<InMemoryConfigStore, InMemorySessionStore>,
    provider_client: OpenAiCompatibleClient<FakeTransport>,
    openai_oauth: OpenAiOAuthAdapter,
    anthropic_oauth: AnthropicOAuthAdapter,
    accounts: AccountRegistry,
    secrets: InMemorySecretStore,
}

#[derive(Debug, Clone)]
struct DoctorSnapshot {
    running: bool,
    ready: bool,
    endpoint_base_url: String,
    model: String,
    endpoint_check: String,
    error: Option<String>,
    exit_code: i32,
    config_path: String,
    data_dir: String,
    socket_path: String,
    api_key_env: Option<String>,
    config_exists: bool,
    daemon_reachable: bool,
    version_check: String,
    models_check: String,
    auth_check: String,
    diagnostics_check: String,
    available_models: Vec<String>,
    endpoint_version: Option<String>,
    auth_summary: Option<String>,
    diagnostics_summary: Option<String>,
    last_error: Option<String>,
}

#[derive(Debug, Clone)]
struct DoctorControlPlane {
    version_check: String,
    models_check: String,
    auth_check: String,
    diagnostics_check: String,
    available_models: Vec<String>,
    endpoint_version: Option<String>,
    auth_summary: Option<String>,
    diagnostics_summary: Option<String>,
    last_error: Option<String>,
}

impl DoctorControlPlane {
    fn skipped() -> Self {
        Self {
            version_check: "skip".to_string(),
            models_check: "skip".to_string(),
            auth_check: "skip".to_string(),
            diagnostics_check: "skip".to_string(),
            available_models: Vec::new(),
            endpoint_version: None,
            auth_summary: None,
            diagnostics_summary: None,
            last_error: None,
        }
    }

    fn all_pass(&self) -> bool {
        self.version_check == "pass"
            && self.models_check == "pass"
            && self.auth_check == "pass"
            && self.diagnostics_check == "pass"
    }
}

impl Default for CliApp {
    fn default() -> Self {
        Self {
            runtime: DaemonRuntime::default(),
            provider_client: default_provider_client(),
            openai_oauth: OpenAiOAuthAdapter::default(),
            anthropic_oauth: AnthropicOAuthAdapter::default(),
            accounts: AccountRegistry::default(),
            secrets: InMemorySecretStore::default(),
        }
    }
}

impl CliApp {
    pub fn run(&mut self, command: CliCommand) -> CliOutput {
        match self.execute(command) {
            Ok(output) => output,
            Err(err) => CliOutput::error(err.to_string(), 3),
        }
    }

    fn execute(&mut self, command: CliCommand) -> Result<CliOutput, OberonError> {
        match command {
            CliCommand::DaemonStart => {
                let status = self.runtime.start()?;
                Ok(CliOutput::success(format!(
                    "running={} endpoint={} model={}",
                    status.running, status.endpoint_base_url, status.model
                )))
            }
            CliCommand::DaemonStop => {
                self.runtime.stop();
                Ok(CliOutput::success("running=false"))
            }
            CliCommand::DaemonStatus => {
                let status = self.runtime.status()?;
                Ok(CliOutput::success(format!(
                    "running={} endpoint={} model={}",
                    status.running, status.endpoint_base_url, status.model
                )))
            }
            CliCommand::Tui { fullscreen } => launch_tui(fullscreen),
            CliCommand::ConfigShow => {
                let config = self.runtime.load_config()?;
                Ok(CliOutput::success(redacted_config_json(&config)?))
            }
            CliCommand::ConfigSet { key, value } => {
                let mut config = self.runtime.load_config()?;
                apply_config_value(&mut config, &key, &value)?;
                self.provider_client.apply_runtime_config(&config)?;
                self.runtime.save_config(&config)?;
                Ok(CliOutput::success(format!("set {key}={value}")))
            }
            CliCommand::EndpointTest => {
                let config = self.runtime.load_config()?;
                materialize_api_key_env_from_store(&config, &self.secrets)?;
                self.provider_client
                    .endpoint_health(Some(&config.endpoint.base_url), &config.endpoint.model)?;
                Ok(CliOutput::success("endpoint test passed"))
            }
            CliCommand::Chat { message } => {
                ensure_daemon_running()?;
                stream_chat_to_console(message)
            }
            CliCommand::LogsTail => Ok(CliOutput::success(log_entry(
                LogLevel::Info,
                "cli requested log tail",
            )?)),
            CliCommand::Doctor { json } => {
                let health = self.runtime.health()?;
                let data_dir = default_data_dir();
                let config_path = data_dir.join("config.toml");
                let config = self.runtime.load_config()?;
                let secret_materialized =
                    materialize_api_key_env_from_store(&config, &self.secrets)?;
                let snapshot = DoctorSnapshot {
                    running: health.running,
                    ready: health.ready,
                    endpoint_base_url: health.endpoint_base_url.clone(),
                    model: health.model.clone(),
                    endpoint_check: if health.ready {
                        "pass".to_string()
                    } else {
                        "fail".to_string()
                    },
                    error: if health.ready {
                        None
                    } else {
                        Some("daemon is not running".to_string())
                    },
                    exit_code: if health.ready { 0 } else { 3 },
                    config_path: config_path.display().to_string(),
                    data_dir: data_dir.display().to_string(),
                    socket_path: default_socket_path().display().to_string(),
                    api_key_env: config.endpoint.api_key_env.clone(),
                    config_exists: config_path.exists(),
                    daemon_reachable: health.running,
                    version_check: "skip".to_string(),
                    models_check: "skip".to_string(),
                    auth_check: "skip".to_string(),
                    diagnostics_check: "skip".to_string(),
                    available_models: Vec::new(),
                    endpoint_version: None,
                    auth_summary: None,
                    diagnostics_summary: None,
                    last_error: None,
                };
                if !health.ready {
                    return if json {
                        doctor_json_output(&snapshot)
                    } else {
                        Ok(CliOutput::error("daemon is not running", 3))
                    };
                }
                let endpoint_result = self
                    .provider_client
                    .endpoint_health(Some(&health.endpoint_base_url), &health.model);
                let control_plane = probe_control_plane(
                    &self.provider_client,
                    Some(&health.endpoint_base_url),
                    &health.model,
                );
                match endpoint_result {
                    Ok(()) if json => {
                        let healthy = control_plane.all_pass();
                        let last_error = if healthy {
                            None
                        } else {
                            prefer_secret_resolution_error(
                                control_plane.last_error.clone(),
                                &config,
                                secret_materialized,
                            )
                        };
                        doctor_json_output(&DoctorSnapshot {
                            error: last_error.clone(),
                            exit_code: if healthy { 0 } else { 3 },
                            version_check: control_plane.version_check,
                            models_check: control_plane.models_check,
                            auth_check: control_plane.auth_check,
                            diagnostics_check: control_plane.diagnostics_check,
                            available_models: control_plane.available_models,
                            endpoint_version: control_plane.endpoint_version,
                            auth_summary: control_plane.auth_summary,
                            diagnostics_summary: control_plane.diagnostics_summary,
                            last_error,
                            ..snapshot
                        })
                    }
                    Ok(()) => Ok(CliOutput::success("doctor passed")),
                    Err(err) if json => {
                        let mut control_plane = control_plane;
                        if control_plane.last_error.is_none() {
                            control_plane.last_error = Some(err.to_string());
                        }
                        let last_error = prefer_secret_resolution_error(
                            control_plane.last_error.clone(),
                            &config,
                            secret_materialized,
                        );
                        doctor_json_output(&DoctorSnapshot {
                            endpoint_check: "fail".to_string(),
                            error: Some(err.to_string()),
                            exit_code: 3,
                            version_check: control_plane.version_check,
                            models_check: control_plane.models_check,
                            auth_check: control_plane.auth_check,
                            diagnostics_check: control_plane.diagnostics_check,
                            available_models: control_plane.available_models,
                            endpoint_version: control_plane.endpoint_version,
                            auth_summary: control_plane.auth_summary,
                            diagnostics_summary: control_plane.diagnostics_summary,
                            last_error,
                            ..snapshot
                        })
                    }
                    Err(err) => Err(err),
                }
            }
            CliCommand::PluginList => {
                ensure_daemon_running()?;
                match daemon_client()?.send(&oberon4_core::DaemonCommand::ListPlugins)? {
                    oberon4_core::DaemonEvent::PluginsListed { plugins } => {
                        let plugins = serde_json::to_string_pretty(&plugins)
                            .map_err(|err| OberonError::Storage(err.to_string()))?;
                        Ok(CliOutput::success(plugins))
                    }
                    event => daemon_protocol_error(event),
                }
            }
            CliCommand::PluginLoad { archive_path } => {
                ensure_daemon_running()?;
                match daemon_client()?
                    .send(&oberon4_core::DaemonCommand::LoadPluginArchive { archive_path })?
                {
                    oberon4_core::DaemonEvent::PluginUpdated { message, plugin } => {
                        Ok(CliOutput::success(format!(
                            "{}\n{}",
                            message,
                            serde_json::to_string_pretty(&plugin)
                                .map_err(|err| OberonError::Storage(err.to_string()))?
                        )))
                    }
                    event => daemon_protocol_error(event),
                }
            }
            CliCommand::PluginUnload { id } => {
                ensure_daemon_running()?;
                match daemon_client()?.send(&oberon4_core::DaemonCommand::UnloadPlugin { id })? {
                    oberon4_core::DaemonEvent::PluginUpdated { message, plugin } => {
                        Ok(CliOutput::success(format!(
                            "{}\n{}",
                            message,
                            serde_json::to_string_pretty(&plugin)
                                .map_err(|err| OberonError::Storage(err.to_string()))?
                        )))
                    }
                    event => daemon_protocol_error(event),
                }
            }
            CliCommand::PluginEnable { id } => {
                ensure_daemon_running()?;
                match daemon_client()?
                    .send(&oberon4_core::DaemonCommand::SetPluginEnabled { id, enabled: true })?
                {
                    oberon4_core::DaemonEvent::PluginUpdated { message, plugin } => {
                        Ok(CliOutput::success(format!(
                            "{}\n{}",
                            message,
                            serde_json::to_string_pretty(&plugin)
                                .map_err(|err| OberonError::Storage(err.to_string()))?
                        )))
                    }
                    event => daemon_protocol_error(event),
                }
            }
            CliCommand::PluginDisable { id } => {
                ensure_daemon_running()?;
                match daemon_client()?
                    .send(&oberon4_core::DaemonCommand::SetPluginEnabled { id, enabled: false })?
                {
                    oberon4_core::DaemonEvent::PluginUpdated { message, plugin } => {
                        Ok(CliOutput::success(format!(
                            "{}\n{}",
                            message,
                            serde_json::to_string_pretty(&plugin)
                                .map_err(|err| OberonError::Storage(err.to_string()))?
                        )))
                    }
                    event => daemon_protocol_error(event),
                }
            }
            CliCommand::AccountConnect { provider } => {
                let (start, bundle) = match provider {
                    OAuthProviderKind::OpenAi => {
                        let (start, _) = self.openai_oauth.start_connect_flow()?;
                        let bundle = TokenBundle {
                            access_token: "access-openai-local-cli".to_string(),
                            refresh_token: Some("refresh-openai-local-cli".to_string()),
                            expires_at_unix: 600,
                            account_id: "acct-local-cli".to_string(),
                            label: "OpenAI Local CLI".to_string(),
                        };
                        (start, bundle)
                    }
                    OAuthProviderKind::Anthropic => {
                        let (start, _) = self.anthropic_oauth.start_connect_flow()?;
                        let bundle = TokenBundle {
                            access_token: "access-anthropic-local-cli".to_string(),
                            refresh_token: Some("refresh-anthropic-local-cli".to_string()),
                            expires_at_unix: 600,
                            account_id: "acct-local-cli".to_string(),
                            label: "Anthropic Local CLI".to_string(),
                        };
                        (start, bundle)
                    }
                };
                let _ = self
                    .accounts
                    .connect(provider, &bundle, &mut self.secrets)?;
                Ok(CliOutput::success(start.authorization_url))
            }
            CliCommand::AccountList => {
                let accounts = serde_json::to_string_pretty(&self.accounts.list())
                    .map_err(|err| OberonError::Storage(err.to_string()))?;
                Ok(CliOutput::success(accounts))
            }
            CliCommand::AccountActivate { provider, account } => {
                self.accounts.activate(provider, &account)?;
                Ok(CliOutput::success(format!(
                    "activated {} {}",
                    provider_name(provider),
                    account
                )))
            }
            CliCommand::AccountDisable { provider, account } => {
                self.accounts.disable(provider, &account)?;
                Ok(CliOutput::success(format!(
                    "disabled {} {}",
                    provider_name(provider),
                    account
                )))
            }
            CliCommand::AccountDisconnect { provider, account } => {
                self.accounts.disconnect(provider, &account)?;
                Ok(CliOutput::success(format!(
                    "disconnected {} {}",
                    provider_name(provider),
                    account
                )))
            }
        }
    }
}

pub fn run_cli(args: &[String]) -> CliOutput {
    match parse_command(args) {
        Ok(command) => {
            if std::env::var_os("OBERON4_FORCE_LOCAL").is_some() {
                let mut app = CliApp::default();
                return app.run(command);
            }
            run_cli_via_daemon(command)
        }
        Err(err) => CliOutput::error(err.to_string(), 2),
    }
}

fn run_cli_via_daemon(command: CliCommand) -> CliOutput {
    match execute_via_daemon(command) {
        Ok(output) => output,
        Err(err) => CliOutput::error(err.to_string(), 3),
    }
}

fn execute_via_daemon(command: CliCommand) -> Result<CliOutput, OberonError> {
    match command {
        CliCommand::DaemonStart => {
            restart_daemon()?;
            format_status_output(daemon_client()?.send(&oberon4_core::DaemonCommand::GetStatus)?)
        }
        CliCommand::DaemonStop => {
            let client = daemon_client()?;
            match client.send(&oberon4_core::DaemonCommand::Shutdown) {
                Ok(oberon4_core::DaemonEvent::Ack { .. }) => {
                    Ok(CliOutput::success("running=false"))
                }
                Ok(event) => daemon_protocol_error(event),
                Err(_) => Ok(CliOutput::success("running=false")),
            }
        }
        CliCommand::DaemonStatus => {
            let client = daemon_client()?;
            match client.send(&oberon4_core::DaemonCommand::GetStatus) {
                Ok(event) => format_status_output(event),
                Err(_) => {
                    let default = AppConfig::default();
                    Ok(CliOutput::success(format!(
                        "running=false endpoint={} model={}",
                        default.endpoint.base_url, default.endpoint.model
                    )))
                }
            }
        }
        CliCommand::Tui { fullscreen } => launch_tui(fullscreen),
        CliCommand::ConfigShow => {
            ensure_daemon_running()?;
            match daemon_client()?.send(&oberon4_core::DaemonCommand::LoadConfig)? {
                oberon4_core::DaemonEvent::ConfigLoaded { config } => {
                    Ok(CliOutput::success(redacted_config_json(&config)?))
                }
                event => daemon_protocol_error(event),
            }
        }
        CliCommand::ConfigSet { key, value } => {
            ensure_daemon_running()?;
            let client = daemon_client()?;
            let mut config = match client.send(&oberon4_core::DaemonCommand::LoadConfig)? {
                oberon4_core::DaemonEvent::ConfigLoaded { config } => config,
                event => return daemon_protocol_error(event),
            };
            apply_config_value(&mut config, &key, &value)?;
            match client.send(&oberon4_core::DaemonCommand::SaveConfig { config })? {
                oberon4_core::DaemonEvent::ConfigLoaded { .. } => {
                    Ok(CliOutput::success(format!("set {key}={value}")))
                }
                event => daemon_protocol_error(event),
            }
        }
        CliCommand::EndpointTest => {
            ensure_daemon_running()?;
            match daemon_client()?.send(&oberon4_core::DaemonCommand::EndpointHealth)? {
                oberon4_core::DaemonEvent::Ack { message } => Ok(CliOutput::success(message)),
                event => daemon_protocol_error(event),
            }
        }
        CliCommand::Chat { message } => {
            ensure_daemon_running()?;
            stream_chat_to_console(message)
        }
        CliCommand::LogsTail => Ok(CliOutput::success(log_entry(
            LogLevel::Info,
            "cli requested log tail",
        )?)),
        CliCommand::Doctor { json } => {
            let default = AppConfig::default();
            let data_dir = default_data_dir();
            let config_path = data_dir.join("config.toml");
            let socket_path = default_socket_path();
            let default_snapshot = DoctorSnapshot {
                running: false,
                ready: false,
                endpoint_base_url: default.endpoint.base_url.clone(),
                model: default.endpoint.model.clone(),
                endpoint_check: "fail".to_string(),
                error: None,
                exit_code: 3,
                config_path: config_path.display().to_string(),
                data_dir: data_dir.display().to_string(),
                socket_path: socket_path.display().to_string(),
                api_key_env: default.endpoint.api_key_env.clone(),
                config_exists: config_path.exists(),
                daemon_reachable: false,
                version_check: "skip".to_string(),
                models_check: "skip".to_string(),
                auth_check: "skip".to_string(),
                diagnostics_check: "skip".to_string(),
                available_models: Vec::new(),
                endpoint_version: None,
                auth_summary: None,
                diagnostics_summary: None,
                last_error: None,
            };
            if let Err(err) = ensure_daemon_running() {
                return if json {
                    doctor_json_output(&DoctorSnapshot {
                        error: Some(err.to_string()),
                        last_error: Some(err.to_string()),
                        ..default_snapshot
                    })
                } else {
                    Err(err)
                };
            }

            let client = daemon_client()?;
            let loaded_config = match client.send(&oberon4_core::DaemonCommand::LoadConfig) {
                Ok(oberon4_core::DaemonEvent::ConfigLoaded { config }) => Some(config),
                Ok(_) => None,
                Err(_) => None,
            };
            let status = match client.send(&oberon4_core::DaemonCommand::GetStatus) {
                Ok(oberon4_core::DaemonEvent::Status { status }) => status,
                Ok(event) => {
                    return if json {
                        doctor_json_output(&DoctorSnapshot {
                            error: Some(
                                OberonError::Storage(format!(
                                    "unexpected daemon response: {event:?}"
                                ))
                                .to_string(),
                            ),
                            ..default_snapshot
                        })
                    } else {
                        daemon_protocol_error(event)
                    };
                }
                Err(err) => {
                    return if json {
                        doctor_json_output(&DoctorSnapshot {
                            error: Some(err.to_string()),
                            ..default_snapshot
                        })
                    } else {
                        Err(err)
                    };
                }
            };
            let config = loaded_config.unwrap_or_else(|| default.clone());
            let file_secrets = FileSecretStore::default();
            let secret_materialized = materialize_api_key_env_from_store(&config, &file_secrets)?;
            let base_snapshot = DoctorSnapshot {
                running: status.running,
                ready: status.running,
                endpoint_base_url: status.endpoint_base_url.clone(),
                model: status.model.clone(),
                endpoint_check: if status.running {
                    "pass".to_string()
                } else {
                    "fail".to_string()
                },
                error: if status.running {
                    None
                } else {
                    Some("daemon is not running".to_string())
                },
                exit_code: if status.running { 0 } else { 3 },
                config_path: config_path.display().to_string(),
                data_dir: data_dir.display().to_string(),
                socket_path: socket_path.display().to_string(),
                api_key_env: config.endpoint.api_key_env.clone(),
                config_exists: config_path.exists(),
                daemon_reachable: true,
                version_check: "skip".to_string(),
                models_check: "skip".to_string(),
                auth_check: "skip".to_string(),
                diagnostics_check: "skip".to_string(),
                available_models: Vec::new(),
                endpoint_version: None,
                auth_summary: None,
                diagnostics_summary: None,
                last_error: None,
            };

            if !status.running {
                return if json {
                    doctor_json_output(&base_snapshot)
                } else {
                    Ok(CliOutput::error("daemon is not running", 3))
                };
            }

            let endpoint_result = client.send(&oberon4_core::DaemonCommand::EndpointHealth);
            let control_plane = probe_control_plane_with_transport(
                &OpenAiCompatibleClient::new(
                    config.endpoint.base_url.clone(),
                    config.endpoint.api_key_env.clone(),
                    RetryPolicy::from_config(&config),
                    oberon4_provider_openai::ReqwestTransport,
                ),
                Some(&config.endpoint.base_url),
                &config.endpoint.model,
            );
            match endpoint_result {
                Ok(oberon4_core::DaemonEvent::Ack { message: _ }) if json => {
                    let healthy = control_plane.all_pass();
                    let last_error = if healthy {
                        None
                    } else {
                        prefer_secret_resolution_error(
                            control_plane.last_error.clone(),
                            &config,
                            secret_materialized,
                        )
                    };
                    doctor_json_output(&DoctorSnapshot {
                        error: last_error.clone(),
                        exit_code: if healthy { 0 } else { 3 },
                        version_check: control_plane.version_check,
                        models_check: control_plane.models_check,
                        auth_check: control_plane.auth_check,
                        diagnostics_check: control_plane.diagnostics_check,
                        available_models: control_plane.available_models,
                        endpoint_version: control_plane.endpoint_version,
                        auth_summary: control_plane.auth_summary,
                        diagnostics_summary: control_plane.diagnostics_summary,
                        last_error,
                        ..base_snapshot
                    })
                }
                Ok(oberon4_core::DaemonEvent::Ack { message }) => {
                    Ok(CliOutput::success(if message == "endpoint test passed" {
                        "doctor passed".to_string()
                    } else {
                        message
                    }))
                }
                Ok(event) => {
                    let err =
                        OberonError::Storage(format!("unexpected daemon response: {event:?}"));
                    if json {
                        let mut control_plane = control_plane;
                        if control_plane.last_error.is_none() {
                            control_plane.last_error = Some(err.to_string());
                        }
                        doctor_json_output(&DoctorSnapshot {
                            endpoint_check: "fail".to_string(),
                            error: Some(err.to_string()),
                            exit_code: 3,
                            version_check: control_plane.version_check,
                            models_check: control_plane.models_check,
                            auth_check: control_plane.auth_check,
                            diagnostics_check: control_plane.diagnostics_check,
                            available_models: control_plane.available_models,
                            endpoint_version: control_plane.endpoint_version,
                            auth_summary: control_plane.auth_summary,
                            diagnostics_summary: control_plane.diagnostics_summary,
                            last_error: control_plane.last_error,
                            ..base_snapshot
                        })
                    } else {
                        daemon_protocol_error(event)
                    }
                }
                Err(err) => {
                    if json {
                        let mut control_plane = control_plane;
                        if control_plane.last_error.is_none() {
                            control_plane.last_error = Some(err.to_string());
                        }
                        let last_error = prefer_secret_resolution_error(
                            control_plane.last_error.clone(),
                            &config,
                            secret_materialized,
                        );
                        doctor_json_output(&DoctorSnapshot {
                            endpoint_check: "fail".to_string(),
                            error: Some(err.to_string()),
                            exit_code: 3,
                            version_check: control_plane.version_check,
                            models_check: control_plane.models_check,
                            auth_check: control_plane.auth_check,
                            diagnostics_check: control_plane.diagnostics_check,
                            available_models: control_plane.available_models,
                            endpoint_version: control_plane.endpoint_version,
                            auth_summary: control_plane.auth_summary,
                            diagnostics_summary: control_plane.diagnostics_summary,
                            last_error,
                            ..base_snapshot
                        })
                    } else {
                        Err(err)
                    }
                }
            }
        }
        CliCommand::PluginList => {
            ensure_daemon_running()?;
            match daemon_client()?.send(&oberon4_core::DaemonCommand::ListPlugins)? {
                oberon4_core::DaemonEvent::PluginsListed { plugins } => {
                    let plugins = serde_json::to_string_pretty(&plugins)
                        .map_err(|err| OberonError::Storage(err.to_string()))?;
                    Ok(CliOutput::success(plugins))
                }
                event => daemon_protocol_error(event),
            }
        }
        CliCommand::PluginLoad { archive_path } => {
            ensure_daemon_running()?;
            match daemon_client()?
                .send(&oberon4_core::DaemonCommand::LoadPluginArchive { archive_path })?
            {
                oberon4_core::DaemonEvent::PluginUpdated { message, plugin } => {
                    Ok(CliOutput::success(format!(
                        "{}\n{}",
                        message,
                        serde_json::to_string_pretty(&plugin)
                            .map_err(|err| OberonError::Storage(err.to_string()))?
                    )))
                }
                event => daemon_protocol_error(event),
            }
        }
        CliCommand::PluginUnload { id } => {
            ensure_daemon_running()?;
            match daemon_client()?.send(&oberon4_core::DaemonCommand::UnloadPlugin { id })? {
                oberon4_core::DaemonEvent::PluginUpdated { message, plugin } => {
                    Ok(CliOutput::success(format!(
                        "{}\n{}",
                        message,
                        serde_json::to_string_pretty(&plugin)
                            .map_err(|err| OberonError::Storage(err.to_string()))?
                    )))
                }
                event => daemon_protocol_error(event),
            }
        }
        CliCommand::PluginEnable { id } => {
            ensure_daemon_running()?;
            match daemon_client()?
                .send(&oberon4_core::DaemonCommand::SetPluginEnabled { id, enabled: true })?
            {
                oberon4_core::DaemonEvent::PluginUpdated { message, plugin } => {
                    Ok(CliOutput::success(format!(
                        "{}\n{}",
                        message,
                        serde_json::to_string_pretty(&plugin)
                            .map_err(|err| OberonError::Storage(err.to_string()))?
                    )))
                }
                event => daemon_protocol_error(event),
            }
        }
        CliCommand::PluginDisable { id } => {
            ensure_daemon_running()?;
            match daemon_client()?
                .send(&oberon4_core::DaemonCommand::SetPluginEnabled { id, enabled: false })?
            {
                oberon4_core::DaemonEvent::PluginUpdated { message, plugin } => {
                    Ok(CliOutput::success(format!(
                        "{}\n{}",
                        message,
                        serde_json::to_string_pretty(&plugin)
                            .map_err(|err| OberonError::Storage(err.to_string()))?
                    )))
                }
                event => daemon_protocol_error(event),
            }
        }
        CliCommand::AccountConnect { provider } => {
            ensure_daemon_running()?;
            match daemon_client()?
                .send(&oberon4_core::DaemonCommand::ConnectAccount { provider })?
            {
                oberon4_core::DaemonEvent::OAuthStarted { url, .. } => Ok(CliOutput::success(url)),
                event => daemon_protocol_error(event),
            }
        }
        CliCommand::AccountList => {
            ensure_daemon_running()?;
            match daemon_client()?.send(&oberon4_core::DaemonCommand::ListAccounts)? {
                oberon4_core::DaemonEvent::AccountsListed { accounts } => {
                    let accounts = serde_json::to_string_pretty(&accounts)
                        .map_err(|err| OberonError::Storage(err.to_string()))?;
                    Ok(CliOutput::success(accounts))
                }
                event => daemon_protocol_error(event),
            }
        }
        CliCommand::AccountActivate { provider, account } => {
            ensure_daemon_running()?;
            match daemon_client()?.send(&oberon4_core::DaemonCommand::ActivateAccount {
                provider,
                account_id: account,
            })? {
                oberon4_core::DaemonEvent::Ack { message } => Ok(CliOutput::success(message)),
                event => daemon_protocol_error(event),
            }
        }
        CliCommand::AccountDisable { provider, account } => {
            ensure_daemon_running()?;
            match daemon_client()?.send(&oberon4_core::DaemonCommand::DisableAccount {
                provider,
                account_id: account,
            })? {
                oberon4_core::DaemonEvent::Ack { message } => Ok(CliOutput::success(message)),
                event => daemon_protocol_error(event),
            }
        }
        CliCommand::AccountDisconnect { provider, account } => {
            ensure_daemon_running()?;
            match daemon_client()?.send(&oberon4_core::DaemonCommand::DisconnectAccount {
                provider,
                account_id: account,
            })? {
                oberon4_core::DaemonEvent::Ack { message } => Ok(CliOutput::success(message)),
                event => daemon_protocol_error(event),
            }
        }
    }
}

fn materialize_api_key_env_from_store<S>(
    config: &AppConfig,
    secrets: &S,
) -> Result<bool, OberonError>
where
    S: SecretStore,
{
    let Some(env_name) = config
        .endpoint
        .api_key_env
        .as_deref()
        .filter(|name| !name.trim().is_empty())
    else {
        return Ok(false);
    };

    if std::env::var(env_name).is_ok() {
        return Ok(false);
    }

    let key = format!("env.{env_name}");
    match secrets.get_secret(&key) {
        Ok(value) => {
            unsafe {
                std::env::set_var(env_name, value);
            }
            Ok(true)
        }
        Err(OberonError::NotFound(_)) => Ok(false),
        Err(err) => Err(err),
    }
}

fn prefer_secret_resolution_error(
    current: Option<String>,
    config: &AppConfig,
    secret_materialized: bool,
) -> Option<String> {
    let Some(env_name) = config.endpoint.api_key_env.as_deref() else {
        return current;
    };
    if secret_materialized || std::env::var(env_name).is_ok() {
        return current;
    }
    match current {
        Some(message) if message.contains("api key environment variable is not set") => {
            Some(format!(
                "api key environment variable is not set and no local secret found: {env_name}"
            ))
        }
        other => other,
    }
}

fn daemon_client() -> Result<DaemonIpcClient, OberonError> {
    Ok(DaemonIpcClient::new(oberon4_daemon::default_socket_path()))
}

const DAEMON_PROBE_DELAY: Duration = Duration::from_millis(50);
const DAEMON_SHUTDOWN_PROBE_ATTEMPTS: usize = 100;
const DAEMON_READY_PROBE_ATTEMPTS: usize = 240;

fn daemon_is_reachable(client: &DaemonIpcClient) -> bool {
    client.send(&oberon4_core::DaemonCommand::GetStatus).is_ok()
}

fn wait_for_daemon_reachability(
    client: &DaemonIpcClient,
    reachable: bool,
    attempts: usize,
) -> bool {
    for _ in 0..attempts {
        if daemon_is_reachable(client) == reachable {
            return true;
        }
        thread::sleep(DAEMON_PROBE_DELAY);
    }
    daemon_is_reachable(client) == reachable
}

fn ensure_daemon_running() -> Result<(), OberonError> {
    let client = daemon_client()?;
    if daemon_is_reachable(&client) {
        return Ok(());
    }

    spawn_daemon_process(client.socket_path().to_path_buf())?;

    if wait_for_daemon_reachability(&client, true, DAEMON_READY_PROBE_ATTEMPTS) {
        return Ok(());
    }

    Err(OberonError::Storage(
        "daemon failed to become ready after spawn".to_string(),
    ))
}

fn restart_daemon() -> Result<(), OberonError> {
    let client = daemon_client()?;
    if daemon_is_reachable(&client) {
        let _ = client.send(&oberon4_core::DaemonCommand::Shutdown);
        if !wait_for_daemon_reachability(&client, false, DAEMON_SHUTDOWN_PROBE_ATTEMPTS) {
            return Err(OberonError::Storage(
                "daemon failed to stop before restart".to_string(),
            ));
        }
    }

    spawn_daemon_process(client.socket_path().to_path_buf())?;

    if wait_for_daemon_reachability(&client, true, DAEMON_READY_PROBE_ATTEMPTS) {
        return Ok(());
    }

    Err(OberonError::Storage(
        "daemon failed to become ready after restart".to_string(),
    ))
}

fn spawn_daemon_process(socket_path: PathBuf) -> Result<(), OberonError> {
    spawn_binary(
        binary_launch_target("oberon4-daemon", "OBERON4_DAEMON_BIN")?,
        socket_path,
    )?;
    Ok(())
}

fn launch_tui(fullscreen: bool) -> Result<CliOutput, OberonError> {
    let mut command = launch_command(binary_launch_target("oberon4-tui", "OBERON4_TUI_BIN")?);
    if fullscreen {
        command.arg("--fullscreen");
    } else {
        command.arg("--no-alt-screen");
    }
    let status = command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|err| OberonError::Storage(format!("failed to launch tui: {err}")))?;

    if status.success() {
        Ok(CliOutput::success(String::new()))
    } else {
        Err(OberonError::Storage(format!(
            "tui exited with status {}",
            status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_string())
        )))
    }
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

fn binary_launch_target(
    binary_name: &str,
    override_env: &str,
) -> Result<BinaryLaunchTarget, OberonError> {
    if let Some(path) = std::env::var_os(override_env) {
        return Ok(BinaryLaunchTarget::Direct(PathBuf::from(path)));
    }

    let current = std::env::current_exe()
        .map_err(|err| OberonError::Storage(format!("current executable lookup failed: {err}")))?;

    resolve_binary_launch_target(&current, binary_name).ok_or_else(|| {
        OberonError::Storage(format!(
            "{binary_name} binary not found and no cargo workspace fallback was available"
        ))
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

fn launch_command(target: BinaryLaunchTarget) -> Command {
    match target {
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
    }
}

fn spawn_binary(target: BinaryLaunchTarget, socket_path: PathBuf) -> Result<(), OberonError> {
    let mut command = launch_command(target);
    command
        .env("OBERON4_SOCKET_PATH", socket_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| OberonError::Storage(format!("failed to spawn daemon: {err}")))?;
    Ok(())
}

fn format_status_output(event: oberon4_core::DaemonEvent) -> Result<CliOutput, OberonError> {
    match event {
        oberon4_core::DaemonEvent::Status { status } => Ok(CliOutput::success(format!(
            "running={} endpoint={} model={}",
            status.running, status.endpoint_base_url, status.model
        ))),
        other => daemon_protocol_error(other),
    }
}

fn daemon_protocol_error(event: oberon4_core::DaemonEvent) -> Result<CliOutput, OberonError> {
    match event {
        oberon4_core::DaemonEvent::Error { message } => Err(OberonError::Validation(message)),
        other => Err(OberonError::Storage(format!(
            "unexpected daemon response: {other:?}"
        ))),
    }
}

fn stream_chat_to_console(message: String) -> Result<CliOutput, OberonError> {
    let mut stream_state = CliStreamState::default();
    let client = daemon_client()?;

    stream_daemon_chat(
        &client,
        &DaemonCommand::SendChat {
            session_id: None,
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: message,
                tool_call_id: None,
                tool_calls: Vec::new(),
            }],
        },
        &mut stream_state,
    )?;

    let final_message = stream_state.final_message.ok_or_else(|| {
        OberonError::Storage("daemon stream ended without ChatComplete".to_string())
    })?;

    if !stream_state.stdout_emitted && !final_message.content.is_empty() {
        print!("{}", final_message.content);
        io::stdout()
            .flush()
            .map_err(|err| OberonError::Storage(format!("stdout flush failed: {err}")))?;
        stream_state.stdout_buffer = final_message.content.clone();
        stream_state.stdout_emitted = true;
    }
    if stream_state.stdout_emitted {
        println!();
    }
    for tool_call in &final_message.tool_calls {
        let summary = format!("tool: {} {}", tool_call.name, tool_call.arguments);
        if !stream_state.stderr_buffer.contains(&summary) {
            eprintln!("{summary}");
            stream_state.stderr_buffer.push_str(&summary);
            stream_state.stderr_buffer.push('\n');
            stream_state.stderr_emitted = true;
        }
    }

    Ok(CliOutput {
        stdout: if stream_state.stdout_buffer.is_empty() {
            final_message.content
        } else {
            stream_state.stdout_buffer
        },
        stderr: stream_state.stderr_buffer,
        exit_code: 0,
        stdout_emitted: stream_state.stdout_emitted,
        stderr_emitted: stream_state.stderr_emitted,
    })
}

#[derive(Default)]
struct CliStreamState {
    stdout_buffer: String,
    stderr_buffer: String,
    stdout_emitted: bool,
    stderr_emitted: bool,
    final_message: Option<ChatMessage>,
    tool_state: BTreeMap<usize, (String, String)>,
}

fn stream_daemon_chat(
    client: &DaemonIpcClient,
    command: &DaemonCommand,
    state: &mut CliStreamState,
) -> Result<(), OberonError> {
    client.send_stream(command, &mut |event| match event {
        DaemonEvent::ChatChunk { content } => {
            print!("{content}");
            io::stdout()
                .flush()
                .map_err(|err| OberonError::Storage(format!("stdout flush failed: {err}")))?;
            state.stdout_buffer.push_str(&content);
            state.stdout_emitted = true;
            Ok(())
        }
        DaemonEvent::ToolCallDelta { delta } => {
            let line = render_tool_delta(&mut state.tool_state, delta);
            eprintln!("{line}");
            state.stderr_buffer.push_str(&line);
            state.stderr_buffer.push('\n');
            state.stderr_emitted = true;
            Ok(())
        }
        DaemonEvent::ToolResult { result } => {
            let line = render_tool_result(&result);
            eprintln!("{line}");
            state.stderr_buffer.push_str(&line);
            state.stderr_buffer.push('\n');
            state.stderr_emitted = true;
            Ok(())
        }
        DaemonEvent::ToolError { error } => {
            let line = render_tool_error(&error);
            eprintln!("{line}");
            state.stderr_buffer.push_str(&line);
            state.stderr_buffer.push('\n');
            state.stderr_emitted = true;
            Ok(())
        }
        DaemonEvent::ApprovalRequired {
            approval_id,
            payload_json,
        } => {
            let prompt = format!(
                "approval required ({approval_id}): {}\napprove? [y/N]: ",
                summarize_approval_payload(&payload_json)
            );
            eprint!("{prompt}");
            io::stderr()
                .flush()
                .map_err(|err| OberonError::Storage(format!("stderr flush failed: {err}")))?;
            state.stderr_buffer.push_str(&prompt);
            let decision = read_cli_approval_decision()?;
            state.stderr_buffer.push_str(&decision);
            state.stderr_buffer.push('\n');
            state.stderr_emitted = true;
            stream_daemon_chat(
                client,
                &DaemonCommand::ResolveApproval {
                    approval_id,
                    decision,
                },
                state,
            )
        }
        DaemonEvent::ChatPaused { .. } => Ok(()),
        DaemonEvent::ChatComplete { message } => {
            state.final_message = Some(message);
            Ok(())
        }
        DaemonEvent::Error { message } => Err(OberonError::Validation(message)),
        other => Err(OberonError::Storage(format!(
            "unexpected daemon response: {other:?}"
        ))),
    })
}

fn render_tool_result(result: &ToolResult) -> String {
    format!(
        "tool result: {}.{} [{:?}] {}",
        result.tool, result.command, result.status, result.summary
    )
}

fn render_tool_error(error: &ToolError) -> String {
    format!(
        "tool error: {} [{}] {}",
        error.command, error.code, error.message
    )
}

fn summarize_approval_payload(payload_json: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(payload_json) else {
        return payload_json.to_string();
    };
    let action = value
        .get("actionSummary")
        .and_then(|v| v.as_str())
        .unwrap_or("pending tool action");
    let tool = value.get("tool").and_then(|v| v.as_str()).unwrap_or("tool");
    let command = value
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("command");
    format!("{tool}.{command}: {action}")
}

fn read_cli_approval_decision() -> Result<String, OberonError> {
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|err| OberonError::Storage(format!("stdin read failed: {err}")))?;
    let trimmed = input.trim().to_ascii_lowercase();
    Ok(match trimmed.as_str() {
        "y" | "yes" | "approve" => "approve".to_string(),
        _ => "deny".to_string(),
    })
}

fn render_tool_delta(
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
    format!(
        "tool[{idx}] {name} {args}",
        idx = delta.index,
        args = entry.1
    )
}

fn doctor_json_output(snapshot: &DoctorSnapshot) -> Result<CliOutput, OberonError> {
    let stdout = serde_json::to_string_pretty(&serde_json::json!({
        "running": snapshot.running,
        "ready": snapshot.ready,
        "endpoint_base_url": snapshot.endpoint_base_url,
        "model": snapshot.model,
        "endpoint_check": snapshot.endpoint_check,
        "error": snapshot.error,
        "config_path": snapshot.config_path,
        "data_dir": snapshot.data_dir,
        "socket_path": snapshot.socket_path,
        "api_key_env": snapshot.api_key_env,
        "config_exists": snapshot.config_exists,
        "daemon_reachable": snapshot.daemon_reachable,
        "version_check": snapshot.version_check,
        "models_check": snapshot.models_check,
        "auth_check": snapshot.auth_check,
        "diagnostics_check": snapshot.diagnostics_check,
        "available_models": snapshot.available_models,
        "endpoint_version": snapshot.endpoint_version,
        "auth_summary": snapshot.auth_summary,
        "diagnostics_summary": snapshot.diagnostics_summary,
        "last_error": snapshot.last_error,
    }))
    .map_err(|err| OberonError::Storage(err.to_string()))?;

    Ok(CliOutput {
        stdout,
        stderr: String::new(),
        exit_code: snapshot.exit_code,
        stdout_emitted: false,
        stderr_emitted: false,
    })
}

fn probe_control_plane<T>(
    client: &OpenAiCompatibleClient<T>,
    endpoint_override: Option<&str>,
    required_model: &str,
) -> DoctorControlPlane
where
    T: OpenAiTransport,
{
    probe_control_plane_with_transport(client, endpoint_override, required_model)
}

fn probe_control_plane_with_transport<T>(
    client: &OpenAiCompatibleClient<T>,
    endpoint_override: Option<&str>,
    required_model: &str,
) -> DoctorControlPlane
where
    T: OpenAiTransport,
{
    let mut result = DoctorControlPlane::skipped();

    match client.control_plane_text(endpoint_override, "/v1/version") {
        Ok(body) => match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(value) => {
                result.version_check = if extract_endpoint_version(&value).is_some() {
                    "pass".to_string()
                } else {
                    result.last_error = Some("version response missing version field".to_string());
                    "fail".to_string()
                };
                result.endpoint_version = extract_endpoint_version(&value);
            }
            Err(err) => {
                result.version_check = "fail".to_string();
                result.last_error = Some(format!("invalid version response: {err}"));
            }
        },
        Err(err) => {
            result.version_check = "fail".to_string();
            result.last_error = Some(err.to_string());
        }
    }

    match client.control_plane_text(endpoint_override, "/v1/models") {
        Ok(body) => match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(value) => {
                result.available_models = extract_model_ids(&value);
                if result
                    .available_models
                    .iter()
                    .any(|model| model == required_model)
                {
                    result.models_check = "pass".to_string();
                } else {
                    result.models_check = "fail".to_string();
                    result.last_error = Some(format!(
                        "models response missing required model: {required_model}"
                    ));
                }
            }
            Err(err) => {
                result.models_check = "fail".to_string();
                result.last_error = Some(format!("invalid models response: {err}"));
            }
        },
        Err(err) => {
            result.models_check = "fail".to_string();
            result.last_error = Some(err.to_string());
        }
    }

    match client.control_plane_text(endpoint_override, "/v1/auth/status") {
        Ok(body) => match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(value) => {
                result.auth_check = "pass".to_string();
                result.auth_summary = summarize_json_object(&value);
            }
            Err(err) => {
                result.auth_check = "fail".to_string();
                result.last_error = Some(format!("invalid auth status response: {err}"));
            }
        },
        Err(err) => {
            result.auth_check = "fail".to_string();
            result.last_error = Some(err.to_string());
        }
    }

    match client.control_plane_text(endpoint_override, "/v1/diagnostics") {
        Ok(body) => match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(value) => {
                result.diagnostics_check = "pass".to_string();
                result.diagnostics_summary = summarize_json_object(&value);
            }
            Err(err) => {
                result.diagnostics_check = "fail".to_string();
                result.last_error = Some(format!("invalid diagnostics response: {err}"));
            }
        },
        Err(err) => {
            result.diagnostics_check = "fail".to_string();
            result.last_error = Some(err.to_string());
        }
    }

    result
}

fn extract_endpoint_version(value: &serde_json::Value) -> Option<String> {
    value
        .get("version")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

fn extract_model_ids(value: &serde_json::Value) -> Vec<String> {
    let items = value
        .get("data")
        .and_then(serde_json::Value::as_array)
        .or_else(|| value.as_array());

    items
        .into_iter()
        .flatten()
        .filter_map(|item| {
            item.as_str()
                .map(ToOwned::to_owned)
                .or_else(|| {
                    item.get("id")
                        .and_then(serde_json::Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .or_else(|| {
                    item.get("model")
                        .and_then(serde_json::Value::as_str)
                        .map(ToOwned::to_owned)
                })
        })
        .collect()
}

fn summarize_json_object(value: &serde_json::Value) -> Option<String> {
    let object = value.as_object()?;
    let mut fragments = Vec::new();
    for (key, value) in object.iter().take(4) {
        let rendered = if let Some(string) = value.as_str() {
            string.to_string()
        } else if let Some(boolean) = value.as_bool() {
            boolean.to_string()
        } else if let Some(number) = value.as_i64() {
            number.to_string()
        } else if let Some(number) = value.as_u64() {
            number.to_string()
        } else if let Some(number) = value.as_f64() {
            number.to_string()
        } else if let Some(array) = value.as_array() {
            format!("array[{}]", array.len())
        } else if value.is_object() {
            "object".to_string()
        } else if value.is_null() {
            "null".to_string()
        } else {
            continue;
        };
        fragments.push(format!("{key}={rendered}"));
    }

    if fragments.is_empty() {
        Some(format!("keys={}", object.len()))
    } else {
        Some(fragments.join(", "))
    }
}

pub fn parse_command(args: &[String]) -> Result<CliCommand, OberonError> {
    let mut parts = args.iter().map(String::as_str);
    let Some(command) = parts.next() else {
        return Err(OberonError::Validation("missing command".to_string()));
    };

    match command {
        "daemon" => match parts.next() {
            Some("start") => Ok(CliCommand::DaemonStart),
            Some("stop") => Ok(CliCommand::DaemonStop),
            Some("status") => Ok(CliCommand::DaemonStatus),
            _ => Err(OberonError::Validation(
                "expected daemon start|stop|status".to_string(),
            )),
        },
        "tui" => match parts.next() {
            None => Ok(CliCommand::Tui { fullscreen: true }),
            Some("--no-alt-screen") if parts.next().is_none() => {
                Ok(CliCommand::Tui { fullscreen: false })
            }
            Some("--fullscreen") if parts.next().is_none() => {
                Ok(CliCommand::Tui { fullscreen: true })
            }
            _ => Err(OberonError::Validation(
                "expected tui [--fullscreen|--no-alt-screen]".to_string(),
            )),
        },
        "config" => match parts.next() {
            Some("show") => Ok(CliCommand::ConfigShow),
            Some("set") => {
                let key = parts
                    .next()
                    .ok_or_else(|| OberonError::Validation("missing config key".to_string()))?;
                let value = parts
                    .next()
                    .ok_or_else(|| OberonError::Validation("missing config value".to_string()))?;
                Ok(CliCommand::ConfigSet {
                    key: key.to_string(),
                    value: value.to_string(),
                })
            }
            _ => Err(OberonError::Validation(
                "expected config show|set".to_string(),
            )),
        },
        "endpoint" => match parts.next() {
            Some("test") => Ok(CliCommand::EndpointTest),
            _ => Err(OberonError::Validation(
                "expected endpoint test".to_string(),
            )),
        },
        "chat" => {
            let flag = parts
                .next()
                .ok_or_else(|| OberonError::Validation("missing chat flag".to_string()))?;
            if flag != "--message" {
                return Err(OberonError::Validation(
                    "expected chat --message <text>".to_string(),
                ));
            }
            let message = parts
                .next()
                .ok_or_else(|| OberonError::Validation("missing chat message".to_string()))?;
            Ok(CliCommand::Chat {
                message: message.to_string(),
            })
        }
        "logs" => match parts.next() {
            Some("tail") => Ok(CliCommand::LogsTail),
            _ => Err(OberonError::Validation("expected logs tail".to_string())),
        },
        "doctor" => match parts.next() {
            None => Ok(CliCommand::Doctor { json: false }),
            Some("--json") if parts.next().is_none() => Ok(CliCommand::Doctor { json: true }),
            _ => Err(OberonError::Validation(
                "expected doctor [--json]".to_string(),
            )),
        },
        "plugin" => match parts.next() {
            Some("list") => Ok(CliCommand::PluginList),
            Some("load") => {
                let archive_path = parts.next().ok_or_else(|| {
                    OberonError::Validation("missing plugin archive path".to_string())
                })?;
                Ok(CliCommand::PluginLoad {
                    archive_path: archive_path.to_string(),
                })
            }
            Some("unload") => {
                let id = parts
                    .next()
                    .ok_or_else(|| OberonError::Validation("missing plugin id".to_string()))?;
                Ok(CliCommand::PluginUnload { id: id.to_string() })
            }
            Some("enable") => {
                let id = parts
                    .next()
                    .ok_or_else(|| OberonError::Validation("missing plugin id".to_string()))?;
                Ok(CliCommand::PluginEnable { id: id.to_string() })
            }
            Some("disable") => {
                let id = parts
                    .next()
                    .ok_or_else(|| OberonError::Validation("missing plugin id".to_string()))?;
                Ok(CliCommand::PluginDisable { id: id.to_string() })
            }
            _ => Err(OberonError::Validation(
                "expected plugin list|load|unload|enable|disable".to_string(),
            )),
        },
        "account" => match parts.next() {
            Some("connect") => Ok(CliCommand::AccountConnect {
                provider: parse_provider_flag(parts.collect())?,
            }),
            Some("list") => Ok(CliCommand::AccountList),
            Some("activate") => {
                let (provider, account) = parse_provider_and_account(parts.collect())?;
                Ok(CliCommand::AccountActivate { provider, account })
            }
            Some("disable") => {
                let (provider, account) = parse_provider_and_account(parts.collect())?;
                Ok(CliCommand::AccountDisable { provider, account })
            }
            Some("disconnect") => {
                let (provider, account) = parse_provider_and_account(parts.collect())?;
                Ok(CliCommand::AccountDisconnect { provider, account })
            }
            _ => Err(OberonError::Validation(
                "expected account connect|list|activate|disable|disconnect".to_string(),
            )),
        },
        _ => Err(OberonError::Validation("unsupported command".to_string())),
    }
}

fn parse_provider_flag(args: Vec<&str>) -> Result<OAuthProviderKind, OberonError> {
    let mut provider = None;
    let mut iter = args.into_iter();
    while let Some(part) = iter.next() {
        if part == "--provider" {
            provider = Some(parse_provider(iter.next().ok_or_else(|| {
                OberonError::Validation("missing provider value".to_string())
            })?)?);
        }
    }
    provider.ok_or_else(|| OberonError::Validation("missing --provider flag".to_string()))
}

fn parse_provider_and_account(args: Vec<&str>) -> Result<(OAuthProviderKind, String), OberonError> {
    let mut provider = None;
    let mut account = None;
    let mut iter = args.into_iter();
    while let Some(part) = iter.next() {
        match part {
            "--provider" => {
                provider = Some(parse_provider(iter.next().ok_or_else(|| {
                    OberonError::Validation("missing provider value".to_string())
                })?)?)
            }
            "--account" => {
                account = Some(
                    iter.next()
                        .ok_or_else(|| {
                            OberonError::Validation("missing account value".to_string())
                        })?
                        .to_string(),
                )
            }
            _ => {}
        }
    }
    Ok((
        provider.ok_or_else(|| OberonError::Validation("missing --provider flag".to_string()))?,
        account.ok_or_else(|| OberonError::Validation("missing --account flag".to_string()))?,
    ))
}

fn parse_provider(value: &str) -> Result<OAuthProviderKind, OberonError> {
    match value {
        "openai" => Ok(OAuthProviderKind::OpenAi),
        "anthropic" => Ok(OAuthProviderKind::Anthropic),
        _ => Err(OberonError::Validation("unsupported provider".to_string())),
    }
}

fn provider_name(provider: OAuthProviderKind) -> &'static str {
    match provider {
        OAuthProviderKind::OpenAi => "openai",
        OAuthProviderKind::Anthropic => "anthropic",
    }
}

fn apply_config_value(config: &mut AppConfig, key: &str, value: &str) -> Result<(), OberonError> {
    match key {
        "endpoint.base_url" => config.endpoint.base_url = value.to_string(),
        "endpoint.model" => config.endpoint.model = value.to_string(),
        "endpoint.provider" => {
            config.endpoint.provider = match value {
                "open_ai_compatible" | "openai" | "openai_compatible" => {
                    ChatProviderKind::OpenAiCompatible
                }
                "anthropic" => ChatProviderKind::Anthropic,
                _ => {
                    return Err(OberonError::Validation(
                        "endpoint.provider must be open_ai_compatible or anthropic".to_string(),
                    ));
                }
            }
        }
        "endpoint.api_key_env" => config.endpoint.api_key_env = Some(value.to_string()),
        "runtime.request_timeout_ms" => {
            config.runtime.request_timeout_ms = value.parse().map_err(|_| {
                OberonError::Validation("runtime.request_timeout_ms must be u64".to_string())
            })?
        }
        "runtime.stream_idle_timeout_ms" => {
            config.runtime.stream_idle_timeout_ms = value.parse().map_err(|_| {
                OberonError::Validation("runtime.stream_idle_timeout_ms must be u64".to_string())
            })?
        }
        "runtime.max_retries" => {
            config.runtime.max_retries = value.parse().map_err(|_| {
                OberonError::Validation("runtime.max_retries must be u8".to_string())
            })?
        }
        "runtime.log_level" => config.runtime.log_level = value.to_string(),
        "runtime.stream_responses" => {
            config.runtime.stream_responses = value.parse().map_err(|_| {
                OberonError::Validation("runtime.stream_responses must be bool".to_string())
            })?
        }
        _ => {
            return Err(OberonError::Validation(
                "unsupported config key".to_string(),
            ));
        }
    }
    Ok(())
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

fn default_provider_client() -> OpenAiCompatibleClient<FakeTransport> {
    let mut client = OpenAiCompatibleClient::new(
        "https://api.openai.com",
        None,
        RetryPolicy {
            timeout_ms: 30_000,
            max_retries: 0,
        },
        FakeTransport::new(vec![
            Ok(ok_response("pong")),
            Ok(ok_response("assistant reply")),
            Ok(ok_response("healthy")),
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
    use std::{
        io::{Read, Write},
        net::TcpListener,
        path::PathBuf,
        sync::{Mutex, OnceLock},
        thread,
        time::Duration,
    };

    use oberon4_auth::{AccountRegistry, InMemoryAccountMetadataStore};
    use oberon4_core::{
        AppConfig, DaemonCommand, DaemonEvent, FakeChatProvider, FakeOAuthProvider,
        InMemorySecretStore, OAuthProviderKind, SecretStore,
    };
    use oberon4_daemon::{DaemonApp, DaemonIpcClient, DaemonRuntime, serve_unix_socket};
    use oberon4_provider_openai::{FakeTransport, OpenAiCompatibleClient, RetryPolicy};

    use super::{
        BinaryLaunchTarget, CliApp, CliCommand, default_data_dir, default_socket_path,
        parse_command, resolve_binary_launch_target, run_cli,
    };

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        env_lock().lock().unwrap_or_else(|err| err.into_inner())
    }

    fn local_doctor_provider() -> OpenAiCompatibleClient<FakeTransport> {
        OpenAiCompatibleClient::new(
            "https://api.openai.com",
            None,
            RetryPolicy {
                timeout_ms: 30_000,
                max_retries: 0,
            },
            FakeTransport::new(vec![
                Ok(r#"{"version":"test-1.0.0"}"#.to_string()),
                Ok(r#"{"version":"test-1.0.0"}"#.to_string()),
                Ok(r#"{"data":[{"id":"gpt-4.1-mini"},{"id":"o4-mini"}]}"#.to_string()),
                Ok(r#"{"entitled":true,"mode":"oauth"}"#.to_string()),
                Ok(r#"{"requests":7,"failovers":1}"#.to_string()),
            ]),
        )
    }

    fn secret_backed_local_doctor_provider() -> OpenAiCompatibleClient<FakeTransport> {
        OpenAiCompatibleClient::new(
            "https://api.openai.com",
            Some("OPENAI_API_KEY".to_string()),
            RetryPolicy {
                timeout_ms: 30_000,
                max_retries: 0,
            },
            FakeTransport::new(vec![
                Ok(r#"{"version":"test-1.0.0"}"#.to_string()),
                Ok(r#"{"version":"test-1.0.0"}"#.to_string()),
                Ok(r#"{"data":[{"id":"gpt-4.1-mini"},{"id":"o4-mini"}]}"#.to_string()),
                Ok(r#"{"entitled":true,"mode":"oauth"}"#.to_string()),
                Ok(r#"{"requests":7,"failovers":1}"#.to_string()),
            ]),
        )
    }

    fn spawn_control_plane_server(model: &str) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind control-plane server");
        let addr = listener.local_addr().expect("local addr");
        let model = model.to_string();
        let handle = thread::spawn(move || {
            for _ in 0..4 {
                let (mut stream, _) = listener.accept().expect("accept");
                let mut buffer = [0_u8; 4096];
                let bytes = stream.read(&mut buffer).expect("read");
                let request = String::from_utf8_lossy(&buffer[..bytes]);
                let (status_line, body) = if request.starts_with("GET /v1/version HTTP/1.1") {
                    (
                        "HTTP/1.1 200 OK",
                        r#"{"version":"test","git_hash":"mock"}"#.to_string(),
                    )
                } else if request.starts_with("GET /v1/models HTTP/1.1") {
                    (
                        "HTTP/1.1 200 OK",
                        format!(r#"{{"data":[{{"id":"{model}"}},{{"id":"fallback-model"}}]}}"#),
                    )
                } else if request.starts_with("GET /v1/auth/status HTTP/1.1") {
                    (
                        "HTTP/1.1 200 OK",
                        r#"{"entitled":true,"mode":"oauth"}"#.to_string(),
                    )
                } else if request.starts_with("GET /v1/diagnostics HTTP/1.1") {
                    (
                        "HTTP/1.1 200 OK",
                        r#"{"requests":11,"failovers":0}"#.to_string(),
                    )
                } else {
                    (
                        "HTTP/1.1 404 Not Found",
                        r#"{"error":"not-found"}"#.to_string(),
                    )
                };
                let response = format!(
                    "{status_line}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
            }
        });
        (format!("http://{}", addr), handle)
    }

    #[test]
    fn parses_daemon_commands() {
        let command = parse_command(&["daemon".into(), "start".into()]).unwrap();
        assert_eq!(command, CliCommand::DaemonStart);
    }

    #[test]
    fn parses_chat_message_command() {
        let command = parse_command(&["chat".into(), "--message".into(), "hello".into()]).unwrap();
        assert_eq!(
            command,
            CliCommand::Chat {
                message: "hello".to_string()
            }
        );
    }

    #[test]
    fn parses_doctor_json_command() {
        let command = parse_command(&["doctor".into(), "--json".into()]).unwrap();
        assert_eq!(command, CliCommand::Doctor { json: true });
    }

    #[test]
    fn parses_tui_no_alt_screen_command() {
        let command = parse_command(&["tui".into(), "--no-alt-screen".into()]).unwrap();
        assert_eq!(command, CliCommand::Tui { fullscreen: false });
    }

    #[test]
    fn parses_tui_default_to_fullscreen() {
        let command = parse_command(&["tui".into()]).unwrap();
        assert_eq!(command, CliCommand::Tui { fullscreen: true });
    }

    #[test]
    fn parses_tui_fullscreen_command() {
        let command = parse_command(&["tui".into(), "--fullscreen".into()]).unwrap();
        assert_eq!(command, CliCommand::Tui { fullscreen: true });
    }

    #[test]
    fn parses_plugin_management_commands() {
        let list = parse_command(&["plugin".into(), "list".into()]).unwrap();
        assert_eq!(list, CliCommand::PluginList);

        let load = parse_command(&[
            "plugin".into(),
            "load".into(),
            "/tmp/example.obplg.zip".into(),
        ])
        .unwrap();
        assert_eq!(
            load,
            CliCommand::PluginLoad {
                archive_path: "/tmp/example.obplg.zip".to_string(),
            }
        );

        let unload = parse_command(&["plugin".into(), "unload".into(), "demo".into()]).unwrap();
        assert_eq!(
            unload,
            CliCommand::PluginUnload {
                id: "demo".to_string(),
            }
        );
    }

    #[test]
    fn rejects_missing_provider_flag() {
        let err = parse_command(&["account".into(), "connect".into()]).unwrap_err();
        assert_eq!(
            err.to_string(),
            "validation failed: missing --provider flag"
        );
    }

    #[test]
    fn daemon_start_and_status_are_deterministic() {
        let mut app = CliApp::default();
        let start = app.run(CliCommand::DaemonStart);
        let status = app.run(CliCommand::DaemonStatus);

        assert_eq!(start.exit_code, 0);
        assert!(start.stdout.contains("running=true"));
        assert!(status.stdout.contains("running=true"));
    }

    #[test]
    fn config_set_updates_config_show() {
        let mut app = CliApp::default();
        let set = app.run(CliCommand::ConfigSet {
            key: "endpoint.model".to_string(),
            value: "o4-mini".to_string(),
        });
        let shown = app.run(CliCommand::ConfigShow);

        assert_eq!(set.exit_code, 0);
        assert!(shown.stdout.contains("\"model\": \"o4-mini\""));
        assert!(shown.stdout.contains("<redacted>"));
    }

    #[test]
    fn endpoint_test_and_doctor_use_provider_client() {
        let mut app = CliApp::default();
        let endpoint = app.run(CliCommand::EndpointTest);
        let doctor_before_start = app.run(CliCommand::Doctor { json: false });
        let _ = app.run(CliCommand::DaemonStart);
        let doctor_after_start = app.run(CliCommand::Doctor { json: false });

        assert_eq!(endpoint.exit_code, 0);
        assert_eq!(endpoint.stdout, "endpoint test passed");
        assert_eq!(doctor_before_start.exit_code, 3);
        assert_eq!(doctor_after_start.exit_code, 0);
        assert_eq!(doctor_after_start.stdout, "doctor passed");
    }

    #[test]
    fn doctor_json_reports_unhealthy_local_state() {
        let _guard = env_guard();
        unsafe {
            std::env::remove_var("OBERON4_SOCKET_PATH");
            std::env::remove_var("OBERON4_DAEMON_BIN");
        }
        let mut app = CliApp::default();
        let output = app.run(CliCommand::Doctor { json: true });
        let value: serde_json::Value = serde_json::from_str(&output.stdout).unwrap();
        let expected_data_dir = default_data_dir();
        let expected_config_path = expected_data_dir.join("config.toml");
        let expected_socket_path = default_socket_path();

        assert_eq!(output.exit_code, 3);
        assert_eq!(value["running"], false);
        assert_eq!(value["ready"], false);
        assert_eq!(value["endpoint_check"], "fail");
        assert_eq!(value["error"], "daemon is not running");
        assert_eq!(value["data_dir"], expected_data_dir.display().to_string());
        assert_eq!(
            value["config_path"],
            expected_config_path.display().to_string()
        );
        assert_eq!(
            value["socket_path"],
            expected_socket_path.display().to_string()
        );
        assert_eq!(value["api_key_env"], "OPENAI_API_KEY");
        assert_eq!(value["config_exists"], expected_config_path.exists());
        assert_eq!(value["daemon_reachable"], false);
        assert_eq!(value["version_check"], "skip");
        assert_eq!(value["models_check"], "skip");
        assert_eq!(value["auth_check"], "skip");
        assert_eq!(value["diagnostics_check"], "skip");
        assert_eq!(value["available_models"], serde_json::json!([]));
        assert!(value["endpoint_version"].is_null());
        assert!(value["auth_summary"].is_null());
        assert!(value["diagnostics_summary"].is_null());
        assert!(value["last_error"].is_null());
        assert!(
            !output.stdout.contains("test-key"),
            "doctor JSON must not leak secret values"
        );
    }

    #[test]
    fn doctor_json_reports_healthy_local_state() {
        let _guard = env_guard();
        unsafe {
            std::env::remove_var("OBERON4_SOCKET_PATH");
            std::env::remove_var("OBERON4_DAEMON_BIN");
        }
        let mut app = CliApp {
            provider_client: local_doctor_provider(),
            ..CliApp::default()
        };
        let _ = app.run(CliCommand::DaemonStart);
        let output = app.run(CliCommand::Doctor { json: true });
        let value: serde_json::Value = serde_json::from_str(&output.stdout).unwrap();

        assert_eq!(output.exit_code, 0);
        assert_eq!(value["running"], true);
        assert_eq!(value["ready"], true);
        assert_eq!(value["endpoint_check"], "pass");
        assert!(value["error"].is_null());
        assert_eq!(value["api_key_env"], "OPENAI_API_KEY");
        assert_eq!(value["daemon_reachable"], true);
        assert_eq!(value["version_check"], "pass");
        assert_eq!(value["models_check"], "pass");
        assert_eq!(value["auth_check"], "pass");
        assert_eq!(value["diagnostics_check"], "pass");
        assert_eq!(
            value["available_models"],
            serde_json::json!(["gpt-4.1-mini", "o4-mini"])
        );
        assert_eq!(value["endpoint_version"], "test-1.0.0");
        assert_eq!(value["last_error"], serde_json::Value::Null);
        assert!(
            value["auth_summary"]
                .as_str()
                .unwrap()
                .contains("entitled=true")
        );
        assert!(
            value["diagnostics_summary"]
                .as_str()
                .unwrap()
                .contains("requests=7")
        );
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
        }
    }

    #[test]
    fn doctor_json_materializes_local_secret_when_env_missing() {
        let _guard = env_guard();
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
            std::env::remove_var("OBERON4_SOCKET_PATH");
            std::env::remove_var("OBERON4_DAEMON_BIN");
        }
        let mut app = CliApp {
            provider_client: secret_backed_local_doctor_provider(),
            ..CliApp::default()
        };
        app.secrets
            .set_secret("env.OPENAI_API_KEY", "from-local-secret")
            .unwrap();

        let _ = app.run(CliCommand::DaemonStart);
        let output = app.run(CliCommand::Doctor { json: true });
        let value: serde_json::Value = serde_json::from_str(&output.stdout).unwrap();

        assert_eq!(output.exit_code, 0);
        assert_eq!(value["endpoint_check"], "pass");
        assert_eq!(value["version_check"], "pass");
        assert_eq!(
            std::env::var("OPENAI_API_KEY").ok().as_deref(),
            Some("from-local-secret")
        );
        assert!(
            !output.stdout.contains("from-local-secret"),
            "doctor JSON must not leak recovered secrets"
        );
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
        }
    }

    #[test]
    fn doctor_json_reports_missing_local_secret_clearly() {
        let _guard = env_guard();
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
            std::env::remove_var("OBERON4_SOCKET_PATH");
            std::env::remove_var("OBERON4_DAEMON_BIN");
        }
        let mut app = CliApp {
            provider_client: secret_backed_local_doctor_provider(),
            ..CliApp::default()
        };

        let _ = app.run(CliCommand::DaemonStart);
        let output = app.run(CliCommand::Doctor { json: true });
        let value: serde_json::Value = serde_json::from_str(&output.stdout).unwrap();

        assert_eq!(output.exit_code, 3);
        assert_eq!(value["endpoint_check"], "fail");
        assert!(
            value["last_error"]
                .as_str()
                .unwrap()
                .contains("no local secret found: OPENAI_API_KEY")
        );
    }

    #[test]
    fn chat_command_returns_assistant_reply() {
        let _guard = env_guard();
        let socket_path =
            std::env::temp_dir().join(format!("oberon4-cli-chat-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket_path);

        unsafe {
            std::env::remove_var("OBERON4_FORCE_LOCAL");
            std::env::set_var("OBERON4_SOCKET_PATH", &socket_path);
        }

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

        let mut app = CliApp::default();
        let reply = app.run(CliCommand::Chat {
            message: "hello".to_string(),
        });

        assert_eq!(reply.exit_code, 0);
        assert!(!reply.stdout.trim().is_empty());
        assert!(reply.stderr.is_empty());

        let stop = DaemonIpcClient::new(&socket_path)
            .send(&DaemonCommand::Shutdown)
            .unwrap();
        match stop {
            DaemonEvent::Ack { message } => assert_eq!(message, "daemon stopped"),
            other => panic!("unexpected stop event: {other:?}"),
        }
        handle.join().unwrap().unwrap();

        unsafe {
            std::env::remove_var("OBERON4_SOCKET_PATH");
        }
    }

    #[test]
    fn account_commands_round_trip_registry() {
        let mut app = CliApp::default();
        let connect = app.run(CliCommand::AccountConnect {
            provider: oberon4_core::OAuthProviderKind::OpenAi,
        });
        let listed = app.run(CliCommand::AccountList);
        let disable = app.run(CliCommand::AccountDisable {
            provider: oberon4_core::OAuthProviderKind::OpenAi,
            account: "acct-local-cli".to_string(),
        });
        let activate = app.run(CliCommand::AccountActivate {
            provider: oberon4_core::OAuthProviderKind::OpenAi,
            account: "acct-local-cli".to_string(),
        });
        let disconnect = app.run(CliCommand::AccountDisconnect {
            provider: oberon4_core::OAuthProviderKind::OpenAi,
            account: "acct-local-cli".to_string(),
        });

        assert_eq!(connect.exit_code, 0);
        assert!(connect.stdout.contains("auth.openai.com"));
        assert!(listed.stdout.contains("acct-local-cli"));
        assert_eq!(disable.exit_code, 0);
        assert_eq!(activate.exit_code, 0);
        assert_eq!(disconnect.exit_code, 0);
    }

    #[test]
    fn logs_tail_returns_json_log_line() {
        let mut app = CliApp::default();
        let output = app.run(CliCommand::LogsTail);
        assert_eq!(output.exit_code, 0);
        assert!(output.stdout.contains("\"request_id\""));
    }

    #[test]
    fn tui_command_launches_override_binary() {
        let _guard = env_guard();
        unsafe {
            std::env::set_var("OBERON4_TUI_BIN", "/bin/true");
        }

        let mut app = CliApp::default();
        let output = app.run(CliCommand::Tui { fullscreen: true });

        assert_eq!(output.exit_code, 0);

        unsafe {
            std::env::remove_var("OBERON4_TUI_BIN");
        }
    }

    #[test]
    fn tui_command_reports_override_binary_failure() {
        let _guard = env_guard();
        unsafe {
            std::env::set_var("OBERON4_TUI_BIN", "/bin/false");
        }

        let mut app = CliApp::default();
        let output = app.run(CliCommand::Tui { fullscreen: true });

        assert_eq!(output.exit_code, 3);
        assert!(output.stderr.contains("tui exited with status"));

        unsafe {
            std::env::remove_var("OBERON4_TUI_BIN");
        }
    }

    #[test]
    fn resolves_workspace_cargo_fallback_when_peer_binary_is_missing() {
        let root =
            std::env::temp_dir().join(format!("oberon4-cli-launcher-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
        let current = root.join("target/debug/oberon4-cli");
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

    #[test]
    fn resolves_direct_peer_binary_when_present() {
        let current = PathBuf::from("/bin/true");
        let target =
            resolve_binary_launch_target(&current, "sh").expect("direct binary should resolve");

        match target {
            BinaryLaunchTarget::Direct(path) => assert_eq!(path, PathBuf::from("/bin/sh")),
            other => panic!("expected direct binary, got {other:?}"),
        }
    }

    #[test]
    fn doctor_json_reports_healthy_daemon_backed_state() {
        let _guard = env_guard();
        let model = "gpt-4.1-mini";
        let (base_url, server_handle) = spawn_control_plane_server(model);
        let socket_path = std::env::temp_dir().join(format!(
            "oberon4-cli-doctor-json-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&socket_path);

        unsafe {
            std::env::remove_var("OBERON4_FORCE_LOCAL");
            std::env::set_var("OBERON4_SOCKET_PATH", &socket_path);
        }

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

        let mut config = AppConfig::default();
        config.endpoint.base_url = base_url.clone();
        config.endpoint.model = model.to_string();
        config.endpoint.api_key_env = None;
        let save = DaemonIpcClient::new(&socket_path)
            .send(&DaemonCommand::SaveConfig { config })
            .unwrap();
        match save {
            DaemonEvent::ConfigLoaded { .. } => {}
            other => panic!("unexpected save event: {other:?}"),
        }

        let output = run_cli(&["doctor".into(), "--json".into()]);
        let value: serde_json::Value = serde_json::from_str(&output.stdout).unwrap();
        assert_eq!(output.exit_code, 0);
        assert_eq!(value["running"], true);
        assert_eq!(value["ready"], true);
        assert_eq!(value["endpoint_check"], "pass");
        assert!(value["error"].is_null());
        assert_eq!(value["api_key_env"], serde_json::Value::Null);
        let expected_data_dir = std::env::var_os("OBERON4_DATA_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(default_data_dir);
        let expected_config_path = expected_data_dir.join("config.toml");
        assert_eq!(value["config_exists"], expected_config_path.exists());
        assert_eq!(value["daemon_reachable"], true);
        assert_eq!(value["version_check"], "pass");
        assert_eq!(value["models_check"], "pass");
        assert_eq!(value["auth_check"], "pass");
        assert_eq!(value["diagnostics_check"], "pass");
        assert_eq!(value["endpoint_version"], "test");
        assert_eq!(
            value["available_models"],
            serde_json::json!(["gpt-4.1-mini", "fallback-model"])
        );
        assert_eq!(
            value["socket_path"],
            socket_path.display().to_string(),
            "doctor JSON should report the effective daemon socket path"
        );

        let stop = DaemonIpcClient::new(&socket_path)
            .send(&DaemonCommand::Shutdown)
            .unwrap();
        match stop {
            DaemonEvent::Ack { message } => assert_eq!(message, "daemon stopped"),
            other => panic!("unexpected stop event: {other:?}"),
        }
        handle.join().unwrap().unwrap();
        server_handle.join().unwrap();

        unsafe {
            std::env::remove_var("OBERON4_SOCKET_PATH");
        }
    }

    #[test]
    fn doctor_json_reports_daemon_start_failure() {
        let _guard = env_guard();
        let socket_path = std::env::temp_dir().join(format!(
            "oberon4-cli-doctor-json-fail-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&socket_path);

        unsafe {
            std::env::remove_var("OBERON4_FORCE_LOCAL");
            std::env::set_var("OBERON4_SOCKET_PATH", &socket_path);
            std::env::set_var("OBERON4_DAEMON_BIN", "/bin/false");
        }

        let output = run_cli(&["doctor".into(), "--json".into()]);
        let value: serde_json::Value = serde_json::from_str(&output.stdout).unwrap();
        assert_eq!(output.exit_code, 3);
        assert_eq!(value["running"], false);
        assert_eq!(value["ready"], false);
        assert_eq!(value["endpoint_check"], "fail");
        assert_eq!(value["daemon_reachable"], false);
        assert_eq!(value["version_check"], "skip");
        assert_eq!(value["models_check"], "skip");
        assert_eq!(value["auth_check"], "skip");
        assert_eq!(value["diagnostics_check"], "skip");
        assert!(
            value["error"]
                .as_str()
                .unwrap()
                .contains("daemon failed to become ready after spawn")
        );
        assert!(
            value["last_error"]
                .as_str()
                .unwrap()
                .contains("daemon failed to become ready after spawn")
        );

        unsafe {
            std::env::remove_var("OBERON4_SOCKET_PATH");
            std::env::remove_var("OBERON4_DAEMON_BIN");
        }
    }
}
