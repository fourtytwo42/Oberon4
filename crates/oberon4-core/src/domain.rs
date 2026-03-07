use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatProviderKind {
    OpenAiCompatible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OAuthProviderKind {
    OpenAi,
    Anthropic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountState {
    Active,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionId(String);

impl SessionId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: SessionId,
    pub title: String,
    pub provider: ChatProviderKind,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSession {
    pub summary: SessionSummary,
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredAccount {
    pub id: String,
    pub provider: OAuthProviderKind,
    pub label: String,
    pub state: AccountState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub running: bool,
    pub endpoint_base_url: String,
    pub model: String,
    pub active_provider: ChatProviderKind,
    pub active_account: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{
        AccountState, ChatMessage, ChatProviderKind, ChatRole, DaemonStatus, OAuthProviderKind,
        SavedSession, SessionId, SessionSummary, StoredAccount,
    };

    #[test]
    fn session_id_exposes_string_view() {
        let id = SessionId::new("session-1");
        assert_eq!(id.as_str(), "session-1");
    }

    #[test]
    fn saved_session_holds_messages_and_metadata() {
        let session = SavedSession {
            summary: SessionSummary {
                id: SessionId::new("session-42"),
                title: "Demo".to_string(),
                provider: ChatProviderKind::OpenAiCompatible,
                model: "gpt-4.1-mini".to_string(),
            },
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: "hello".to_string(),
            }],
        };

        assert_eq!(session.summary.id.as_str(), "session-42");
        assert_eq!(session.messages.len(), 1);
    }

    #[test]
    fn daemon_status_tracks_active_account() {
        let status = DaemonStatus {
            running: true,
            endpoint_base_url: "https://api.openai.com".to_string(),
            model: "gpt-4.1-mini".to_string(),
            active_provider: ChatProviderKind::OpenAiCompatible,
            active_account: Some("acct-1".to_string()),
        };

        assert_eq!(status.active_account.as_deref(), Some("acct-1"));
    }

    #[test]
    fn stored_account_keeps_provider_and_state() {
        let account = StoredAccount {
            id: "acct-1".to_string(),
            provider: OAuthProviderKind::Anthropic,
            label: "primary".to_string(),
            state: AccountState::Disabled,
        };

        assert_eq!(account.provider, OAuthProviderKind::Anthropic);
        assert_eq!(account.state, AccountState::Disabled);
    }
}
