use crate::{
    AppConfig, ChatMessage, ChatToolCallDelta, DaemonStatus, OAuthProviderKind, OberonError,
    SavedSession,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatStreamEvent {
    TextDelta { content: String },
    ToolCallDelta(ChatToolCallDelta),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFinishState {
    AssistantTextComplete,
    ToolCallRequested,
    ToolCallPending,
    ToolResultRequired,
    ResponseComplete,
    ResponseIncomplete,
    ProviderError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilityMatrix {
    pub tool_definitions: bool,
    pub non_streaming_tool_calls: bool,
    pub streaming_tool_calls: bool,
    pub partial_argument_deltas: bool,
    pub mixed_text_and_tool_responses: bool,
    pub parallel_tool_calls: bool,
    pub schema_cleaning_required: bool,
    pub delegated_pending_support: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters_schema: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolAwareProviderResponse {
    pub message: ChatMessage,
    pub finish_state: ProviderFinishState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingOAuthFlow {
    pub flow_id: String,
    pub provider: OAuthProviderKind,
    pub oauth_state: String,
    pub code_verifier: String,
    pub redirect_uri: String,
    pub expires_at_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthStart {
    pub flow_id: String,
    pub provider: OAuthProviderKind,
    pub authorization_url: String,
    pub redirect_uri: String,
    pub expires_at_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthCallbackPayload {
    pub state: String,
    pub code: String,
    pub return_to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenBundle {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at_unix: u64,
    pub account_id: String,
    pub label: String,
}

pub trait ChatProvider {
    fn stream_message(
        &self,
        endpoint: &str,
        model: &str,
        messages: &[ChatMessage],
        on_event: &mut dyn FnMut(ChatStreamEvent) -> Result<(), OberonError>,
    ) -> Result<ChatMessage, OberonError>;

    fn send_message(
        &self,
        endpoint: &str,
        model: &str,
        messages: &[ChatMessage],
    ) -> Result<ChatMessage, OberonError> {
        self.stream_message(endpoint, model, messages, &mut |_| Ok(()))
    }

    fn endpoint_health(&self, endpoint: &str, model: &str) -> Result<(), OberonError> {
        let probe = [ChatMessage {
            role: crate::ChatRole::User,
            content: "health".to_string(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        }];
        self.send_message(endpoint, model, &probe).map(|_| ())
    }

    fn apply_runtime_config(&mut self, _config: &AppConfig) -> Result<(), OberonError> {
        Ok(())
    }
}

pub trait ToolAwareChatProvider {
    fn stream_tool_aware_message(
        &self,
        endpoint: &str,
        model: &str,
        messages: &[ChatMessage],
        tools: &[ProviderToolDefinition],
        on_event: &mut dyn FnMut(ChatStreamEvent) -> Result<(), OberonError>,
    ) -> Result<ToolAwareProviderResponse, OberonError>;

    fn send_tool_aware_message(
        &self,
        endpoint: &str,
        model: &str,
        messages: &[ChatMessage],
        tools: &[ProviderToolDefinition],
    ) -> Result<ToolAwareProviderResponse, OberonError> {
        self.stream_tool_aware_message(endpoint, model, messages, tools, &mut |_| Ok(()))
    }

    fn provider_capabilities(&self) -> ProviderCapabilityMatrix;

    fn apply_runtime_config(&mut self, _config: &AppConfig) -> Result<(), OberonError> {
        Ok(())
    }

    fn endpoint_health(&self, endpoint: &str, model: &str) -> Result<(), OberonError>;
}

pub trait OAuthProvider {
    fn provider_kind(&self) -> OAuthProviderKind;
    fn start_connect_flow(&self) -> Result<(OAuthStart, PendingOAuthFlow), OberonError>;
    fn exchange_callback(
        &self,
        flow: &PendingOAuthFlow,
        callback: OAuthCallbackPayload,
    ) -> Result<TokenBundle, OberonError>;
}

pub trait ConfigStore {
    fn load(&self) -> Result<AppConfig, OberonError>;
    fn save(&mut self, config: &AppConfig) -> Result<(), OberonError>;
}

pub trait SessionStore {
    fn save_session(&mut self, session: SavedSession) -> Result<(), OberonError>;
    fn load_session(&self, session_id: &str) -> Result<SavedSession, OberonError>;
    fn list_sessions(&self) -> Result<Vec<SavedSession>, OberonError>;
    fn delete_session(&mut self, session_id: &str) -> Result<(), OberonError>;
}

pub trait StatusProvider {
    fn status(&self) -> Result<DaemonStatus, OberonError>;
}
