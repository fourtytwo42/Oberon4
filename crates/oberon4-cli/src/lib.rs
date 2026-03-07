use std::{
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use oberon4_auth::{AccountRegistry, AnthropicOAuthAdapter, OpenAiOAuthAdapter, TokenBundle};
use oberon4_core::{
    AppConfig, ChatMessage, ChatProvider, ChatRole, InMemoryConfigStore, InMemorySecretStore,
    InMemorySessionStore, OAuthProvider, OAuthProviderKind, OberonError,
};
use oberon4_daemon::{DaemonIpcClient, DaemonRuntime, LogLevel, log_entry};
use oberon4_provider_openai::{FakeTransport, OpenAiCompatibleClient, RetryPolicy};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliCommand {
    DaemonStart,
    DaemonStop,
    DaemonStatus,
    Tui,
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
    Doctor,
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
}

impl CliOutput {
    fn success(stdout: impl Into<String>) -> Self {
        Self {
            stdout: stdout.into(),
            stderr: String::new(),
            exit_code: 0,
        }
    }

    fn error(stderr: impl Into<String>, exit_code: i32) -> Self {
        Self {
            stdout: String::new(),
            stderr: stderr.into(),
            exit_code,
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
            CliCommand::Tui => Ok(CliOutput::success(
                "tui launch delegated to oberon4-tui (not yet wired in phase 5)",
            )),
            CliCommand::ConfigShow => {
                let config = self.runtime.load_config()?;
                Ok(CliOutput::success(redacted_config_json(&config)?))
            }
            CliCommand::ConfigSet { key, value } => {
                let mut config = self.runtime.load_config()?;
                apply_config_value(&mut config, &key, &value)?;
                self.runtime.save_config(&config)?;
                Ok(CliOutput::success(format!("set {key}={value}")))
            }
            CliCommand::EndpointTest => {
                let config = self.runtime.load_config()?;
                self.provider_client
                    .endpoint_health(Some(&config.endpoint.base_url), &config.endpoint.model)?;
                Ok(CliOutput::success("endpoint test passed"))
            }
            CliCommand::Chat { message } => {
                if self.runtime.lifecycle_state() == oberon4_daemon::LifecycleState::Stopped {
                    let _ = self.runtime.start()?;
                }
                let reply = self.provider_client.send_message(
                    &self.runtime.load_config()?.endpoint.base_url,
                    &self.runtime.load_config()?.endpoint.model,
                    &[ChatMessage {
                        role: ChatRole::User,
                        content: message,
                    }],
                )?;
                Ok(CliOutput::success(reply.content))
            }
            CliCommand::LogsTail => Ok(CliOutput::success(log_entry(
                LogLevel::Info,
                "cli requested log tail",
            )?)),
            CliCommand::Doctor => {
                let health = self.runtime.health()?;
                if !health.ready {
                    return Ok(CliOutput::error("daemon is not running", 3));
                }
                self.provider_client
                    .endpoint_health(Some(&health.endpoint_base_url), &health.model)?;
                Ok(CliOutput::success("doctor passed"))
            }
            CliCommand::AccountConnect { provider } => {
                let authorization_url = match provider {
                    OAuthProviderKind::OpenAi => self.openai_oauth.start_connect_flow()?,
                    OAuthProviderKind::Anthropic => self.anthropic_oauth.start_connect_flow()?,
                };
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
                Ok(CliOutput::success(authorization_url))
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
            ensure_daemon_running()?;
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
        CliCommand::Tui => Ok(CliOutput::success(
            "tui launch delegated to oberon4-tui (real daemon boundary only wired for CLI in this step)",
        )),
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
            match daemon_client()?.send(&oberon4_core::DaemonCommand::SendChat {
                session_id: None,
                messages: vec![ChatMessage {
                    role: ChatRole::User,
                    content: message,
                }],
            })? {
                oberon4_core::DaemonEvent::ChatComplete { message } => {
                    Ok(CliOutput::success(message.content))
                }
                event => daemon_protocol_error(event),
            }
        }
        CliCommand::LogsTail => Ok(CliOutput::success(log_entry(
            LogLevel::Info,
            "cli requested log tail",
        )?)),
        CliCommand::Doctor => {
            ensure_daemon_running()?;
            let client = daemon_client()?;
            match client.send(&oberon4_core::DaemonCommand::GetStatus)? {
                oberon4_core::DaemonEvent::Status { status } if status.running => {}
                oberon4_core::DaemonEvent::Status { .. } => {
                    return Ok(CliOutput::error("daemon is not running", 3));
                }
                event => return daemon_protocol_error(event),
            }
            match client.send(&oberon4_core::DaemonCommand::EndpointHealth)? {
                oberon4_core::DaemonEvent::Ack { message } => Ok(CliOutput::success(
                    if message == "endpoint test passed" {
                        "doctor passed".to_string()
                    } else {
                        message
                    },
                )),
                event => daemon_protocol_error(event),
            }
        }
        CliCommand::AccountConnect { provider } => {
            ensure_daemon_running()?;
            match daemon_client()?.send(&oberon4_core::DaemonCommand::ConnectAccount { provider })?
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

fn daemon_client() -> Result<DaemonIpcClient, OberonError> {
    Ok(DaemonIpcClient::new(oberon4_daemon::default_socket_path()))
}

fn ensure_daemon_running() -> Result<(), OberonError> {
    let client = daemon_client()?;
    if client.send(&oberon4_core::DaemonCommand::GetStatus).is_ok() {
        return Ok(());
    }

    spawn_daemon_process(client.socket_path().to_path_buf())?;

    for _ in 0..40 {
        if client.send(&oberon4_core::DaemonCommand::GetStatus).is_ok() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(50));
    }

    Err(OberonError::Storage(
        "daemon failed to become ready after spawn".to_string(),
    ))
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
        "tui" => Ok(CliCommand::Tui),
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
        "doctor" => Ok(CliCommand::Doctor),
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

fn provider_label(provider: OAuthProviderKind) -> &'static str {
    match provider {
        OAuthProviderKind::OpenAi => "OpenAI",
        OAuthProviderKind::Anthropic => "Anthropic",
    }
}

fn apply_config_value(config: &mut AppConfig, key: &str, value: &str) -> Result<(), OberonError> {
    match key {
        "endpoint.base_url" => config.endpoint.base_url = value.to_string(),
        "endpoint.model" => config.endpoint.model = value.to_string(),
        "endpoint.api_key_env" => config.endpoint.api_key_env = Some(value.to_string()),
        "runtime.request_timeout_ms" => {
            config.runtime.request_timeout_ms = value.parse().map_err(|_| {
                OberonError::Validation("runtime.request_timeout_ms must be u64".to_string())
            })?
        }
        "runtime.max_retries" => {
            config.runtime.max_retries = value.parse().map_err(|_| {
                OberonError::Validation("runtime.max_retries must be u8".to_string())
            })?
        }
        "runtime.log_level" => config.runtime.log_level = value.to_string(),
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
    OpenAiCompatibleClient::new(
        "https://api.openai.com",
        RetryPolicy {
            timeout_ms: 30_000,
            max_retries: 0,
        },
        FakeTransport::new(vec![
            Ok(ok_response("pong")),
            Ok(ok_response("assistant reply")),
            Ok(ok_response("healthy")),
        ]),
    )
}

fn ok_response(content: &str) -> String {
    format!(r#"{{"choices":[{{"message":{{"role":"assistant","content":"{content}"}}}}]}}"#)
}

#[cfg(test)]
mod tests {
    use super::{CliApp, CliCommand, parse_command};

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
        let doctor_before_start = app.run(CliCommand::Doctor);
        let _ = app.run(CliCommand::DaemonStart);
        let doctor_after_start = app.run(CliCommand::Doctor);

        assert_eq!(endpoint.exit_code, 0);
        assert_eq!(endpoint.stdout, "endpoint test passed");
        assert_eq!(doctor_before_start.exit_code, 3);
        assert_eq!(doctor_after_start.exit_code, 0);
    }

    #[test]
    fn chat_command_returns_assistant_reply() {
        let mut app = CliApp::default();
        let reply = app.run(CliCommand::Chat {
            message: "hello".to_string(),
        });

        assert_eq!(reply.exit_code, 0);
        assert_eq!(reply.stdout, "pong");
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
            account: "openai-default".to_string(),
        });
        let activate = app.run(CliCommand::AccountActivate {
            provider: oberon4_core::OAuthProviderKind::OpenAi,
            account: "openai-default".to_string(),
        });
        let disconnect = app.run(CliCommand::AccountDisconnect {
            provider: oberon4_core::OAuthProviderKind::OpenAi,
            account: "openai-default".to_string(),
        });

        assert_eq!(connect.exit_code, 0);
        assert!(connect.stdout.contains("auth.openai.com"));
        assert!(listed.stdout.contains("openai-default"));
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
}
