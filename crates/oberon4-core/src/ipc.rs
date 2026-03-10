use serde::{Deserialize, Serialize};

use crate::{
    AppConfig, ChatMessage, ChatToolCallDelta, DaemonStatus, OAuthCallbackPayload,
    OAuthProviderKind, SavedSession, StoredAccount, ToolError, ToolResult,
    plugin_runtime::InstalledPluginStatus,
};

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonCommand {
    GetStatus,
    Shutdown,
    SendChat {
        session_id: Option<String>,
        messages: Vec<ChatMessage>,
    },
    EndpointHealth,
    SaveConfig {
        config: AppConfig,
    },
    LoadConfig,
    SaveSession {
        session: SavedSession,
    },
    LoadSession {
        session_id: String,
    },
    ListSessions,
    DeleteSession {
        session_id: String,
    },
    ConnectAccount {
        provider: OAuthProviderKind,
    },
    CompleteOAuthConnect {
        flow_id: String,
        callback: OAuthCallbackPayload,
    },
    ListAccounts,
    ActivateAccount {
        provider: OAuthProviderKind,
        account_id: String,
    },
    DisableAccount {
        provider: OAuthProviderKind,
        account_id: String,
    },
    DisconnectAccount {
        provider: OAuthProviderKind,
        account_id: String,
    },
    ListPlugins,
    LoadPluginArchive {
        archive_path: String,
    },
    UnloadPlugin {
        id: String,
    },
    SetPluginEnabled {
        id: String,
        enabled: bool,
    },
    ResolveApproval {
        approval_id: String,
        decision: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonEvent {
    Status {
        status: DaemonStatus,
    },
    ChatChunk {
        content: String,
    },
    ToolCallDelta {
        delta: ChatToolCallDelta,
    },
    ToolResult {
        result: ToolResult,
    },
    ToolError {
        error: ToolError,
    },
    ApprovalRequired {
        approval_id: String,
        payload_json: String,
    },
    ChatPaused {
        approval_id: String,
    },
    ChatComplete {
        message: ChatMessage,
    },
    ConfigLoaded {
        config: AppConfig,
    },
    SessionLoaded {
        session: SavedSession,
    },
    SessionList {
        sessions: Vec<SavedSession>,
    },
    AccountsListed {
        accounts: Vec<StoredAccount>,
    },
    PluginsListed {
        plugins: Vec<InstalledPluginStatus>,
    },
    PluginUpdated {
        plugin: InstalledPluginStatus,
        message: String,
    },
    OAuthStarted {
        provider: OAuthProviderKind,
        flow_id: String,
        url: String,
        redirect_uri: String,
    },
    Ack {
        message: String,
    },
    Error {
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use crate::{
        AppConfig, ChatMessage, ChatProviderKind, ChatRole, ChatToolCallDelta, DaemonCommand,
        DaemonEvent, DaemonStatus, InstalledPluginKind, InstalledPluginStatus,
        OAuthCallbackPayload, OAuthProviderKind, SavedSession, SessionId, SessionSummary,
    };

    #[test]
    fn command_schema_round_trips() {
        let command = DaemonCommand::SendChat {
            session_id: Some("session-1".to_string()),
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: "hello".to_string(),
                tool_call_id: None,
                tool_calls: Vec::new(),
            }],
        };

        let encoded = serde_json::to_string(&command).expect("command should serialize");
        let decoded: DaemonCommand =
            serde_json::from_str(&encoded).expect("command should deserialize");

        assert_eq!(decoded, command);
    }

    #[test]
    fn event_schema_round_trips() {
        let event = DaemonEvent::Status {
            status: DaemonStatus {
                running: true,
                endpoint_base_url: "https://api.openai.com".to_string(),
                model: "gpt-4.1-mini".to_string(),
                active_provider: ChatProviderKind::OpenAiCompatible,
                active_account: Some("acct-1".to_string()),
            },
        };

        let encoded = serde_json::to_string(&event).expect("event should serialize");
        let decoded: DaemonEvent =
            serde_json::from_str(&encoded).expect("event should deserialize");

        assert_eq!(decoded, event);
    }

    #[test]
    fn load_config_command_has_deterministic_json_tag() {
        let encoded = serde_json::to_string(&DaemonCommand::LoadConfig).unwrap();
        assert!(encoded.contains("\"type\":\"load_config\""));
    }

    #[test]
    fn session_loaded_event_carries_saved_session() {
        let event = DaemonEvent::SessionLoaded {
            session: SavedSession {
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
            },
        };

        let encoded = serde_json::to_string(&event).unwrap();
        let decoded: DaemonEvent = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, event);
    }

    #[test]
    fn oauth_started_event_carries_provider() {
        let event = DaemonEvent::OAuthStarted {
            provider: OAuthProviderKind::OpenAi,
            flow_id: "flow-1".to_string(),
            url: "http://localhost/callback".to_string(),
            redirect_uri: "http://localhost:1455/auth/callback".to_string(),
        };

        let encoded = serde_json::to_string(&event).unwrap();
        assert!(encoded.contains("\"provider\":\"open_ai\""));
    }

    #[test]
    fn complete_oauth_connect_command_round_trips() {
        let command = DaemonCommand::CompleteOAuthConnect {
            flow_id: "flow-1".to_string(),
            callback: OAuthCallbackPayload {
                state: "state-1".to_string(),
                code: "code-1".to_string(),
                return_to: None,
            },
        };

        let encoded = serde_json::to_string(&command).unwrap();
        let decoded: DaemonCommand = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, command);
    }

    #[test]
    fn save_config_command_embeds_shared_config() {
        let command = DaemonCommand::SaveConfig {
            config: AppConfig::default(),
        };

        let encoded = serde_json::to_string(&command).unwrap();
        let decoded: DaemonCommand = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, command);
    }

    #[test]
    fn delete_session_command_round_trips() {
        let command = DaemonCommand::DeleteSession {
            session_id: "session-1".to_string(),
        };

        let encoded = serde_json::to_string(&command).unwrap();
        let decoded: DaemonCommand = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, command);
    }

    #[test]
    fn tool_call_delta_event_round_trips() {
        let event = DaemonEvent::ToolCallDelta {
            delta: ChatToolCallDelta {
                index: 0,
                id_fragment: Some("call_".to_string()),
                name_fragment: Some("lookup".to_string()),
                arguments_fragment: Some("{\"q\":\"hi\"}".to_string()),
            },
        };

        let encoded = serde_json::to_string(&event).unwrap();
        let decoded: DaemonEvent = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, event);
    }

    #[test]
    fn plugin_admin_commands_and_events_round_trip() {
        let command = DaemonCommand::SetPluginEnabled {
            id: "versioned-workspace".to_string(),
            enabled: false,
        };
        let encoded = serde_json::to_string(&command).unwrap();
        let decoded: DaemonCommand = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, command);

        let event = DaemonEvent::PluginUpdated {
            plugin: InstalledPluginStatus {
                id: "versioned-workspace".to_string(),
                version: "0.1.0".to_string(),
                kind: InstalledPluginKind::FirstPartyPackage,
                host_trusted: true,
                installed: true,
                enabled: false,
                archive_path: Some("/tmp/versioned-workspace.obpkg.zip".to_string()),
                archive_checksum: Some("sha256:abc123".to_string()),
                config_path: Some("/tmp/versioned-workspace.toml".to_string()),
                load_status: Some("ready".to_string()),
                last_error: None,
                reason_codes: vec!["blocked_by_disable".to_string()],
            },
            message: "disabled plugin versioned-workspace".to_string(),
        };
        let encoded = serde_json::to_string(&event).unwrap();
        let decoded: DaemonEvent = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, event);
    }
}
