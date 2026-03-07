use serde::{Deserialize, Serialize};

use crate::{AppConfig, ChatMessage, DaemonStatus, OAuthProviderKind, SavedSession, StoredAccount};

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
    ConnectAccount {
        provider: OAuthProviderKind,
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
    OAuthStarted {
        provider: OAuthProviderKind,
        url: String,
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
        AppConfig, ChatMessage, ChatProviderKind, ChatRole, DaemonCommand, DaemonEvent,
        DaemonStatus, OAuthProviderKind, SavedSession, SessionId, SessionSummary,
    };

    #[test]
    fn command_schema_round_trips() {
        let command = DaemonCommand::SendChat {
            session_id: Some("session-1".to_string()),
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: "hello".to_string(),
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
            url: "http://localhost/callback".to_string(),
        };

        let encoded = serde_json::to_string(&event).unwrap();
        assert!(encoded.contains("\"provider\":\"open_ai\""));
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
}
