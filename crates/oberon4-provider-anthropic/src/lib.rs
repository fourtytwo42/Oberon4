use std::{cell::RefCell, collections::VecDeque, time::Duration};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use oberon4_core::{
    AppConfig, ChatMessage, ChatRole, ChatStreamEvent, ChatToolCall, ChatToolCallDelta,
    OberonError, ProviderCapabilityMatrix, ProviderFinishState, ProviderToolDefinition,
    ToolAwareChatProvider, ToolAwareProviderResponse,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub timeout_ms: u64,
    pub max_retries: u8,
}

impl RetryPolicy {
    pub fn from_config(config: &AppConfig) -> Self {
        Self {
            timeout_ms: config.runtime.request_timeout_ms,
            max_retries: config.runtime.max_retries,
        }
    }
}

pub trait AnthropicTransport {
    fn post_json(
        &self,
        url: &str,
        body: &str,
        timeout_ms: u64,
        api_key: Option<&str>,
    ) -> Result<String, OberonError>;

    fn get_text(
        &self,
        url: &str,
        timeout_ms: u64,
        api_key: Option<&str>,
    ) -> Result<String, OberonError>;
}

#[derive(Debug, Clone, Default)]
pub struct ReqwestTransport;

impl AnthropicTransport for ReqwestTransport {
    fn post_json(
        &self,
        url: &str,
        body: &str,
        timeout_ms: u64,
        api_key: Option<&str>,
    ) -> Result<String, OberonError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .map_err(|err| OberonError::Storage(format!("transport client build failed: {err}")))?;
        let mut request = client
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header("anthropic-version", "2023-06-01");
        if let Some(key) = api_key {
            request = request.header("x-api-key", key);
        }
        let response = request
            .body(body.to_string())
            .send()
            .map_err(|err| OberonError::Storage(format!("transport request failed: {err}")))?;
        let status = response.status();
        let text = response.text().map_err(|err| {
            OberonError::Storage(format!("transport response read failed: {err}"))
        })?;
        if !status.is_success() {
            return Err(OberonError::Validation(format!(
                "provider returned HTTP {}",
                status.as_u16()
            )));
        }
        Ok(text)
    }

    fn get_text(
        &self,
        url: &str,
        timeout_ms: u64,
        api_key: Option<&str>,
    ) -> Result<String, OberonError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .map_err(|err| OberonError::Storage(format!("transport client build failed: {err}")))?;
        let mut request = client.get(url).header("anthropic-version", "2023-06-01");
        if let Some(key) = api_key {
            request = request.header("x-api-key", key);
        }
        let response = request
            .send()
            .map_err(|err| OberonError::Storage(format!("transport request failed: {err}")))?;
        let status = response.status();
        let text = response.text().map_err(|err| {
            OberonError::Storage(format!("transport response read failed: {err}"))
        })?;
        if !status.is_success() {
            return Err(OberonError::Validation(format!(
                "provider returned HTTP {}",
                status.as_u16()
            )));
        }
        Ok(text)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    tools: Vec<AnthropicToolDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AnthropicMessage {
    role: String,
    content: Vec<AnthropicContentBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AnthropicResponse {
    id: String,
    #[serde(default)]
    content: Vec<AnthropicContentBlock>,
    stop_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AnthropicToolDefinition {
    name: String,
    description: String,
    input_schema: Value,
}

#[derive(Debug, Clone)]
pub struct AnthropicClient<T> {
    base_url: String,
    api_key_env: Option<String>,
    retry_policy: RetryPolicy,
    transport: T,
}

impl<T> AnthropicClient<T> {
    pub fn new(
        base_url: impl Into<String>,
        api_key_env: Option<String>,
        retry_policy: RetryPolicy,
        transport: T,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            api_key_env,
            retry_policy,
            transport,
        }
    }

    pub fn set_api_key_env(&mut self, api_key_env: Option<String>) {
        self.api_key_env = api_key_env;
    }

    pub fn set_retry_policy(&mut self, retry_policy: RetryPolicy) {
        self.retry_policy = retry_policy;
    }

    fn effective_base_url<'a>(&'a self, endpoint_override: &'a str) -> &'a str {
        if endpoint_override.trim().is_empty() {
            &self.base_url
        } else {
            endpoint_override
        }
    }

    fn bearer_token(&self) -> Result<Option<String>, OberonError> {
        match self.api_key_env.as_deref() {
            Some(name) if !name.trim().is_empty() => std::env::var(name).map(Some).map_err(|_| {
                OberonError::Validation(format!("api key environment variable is not set: {name}"))
            }),
            _ => Ok(None),
        }
    }
}

impl<T> ToolAwareChatProvider for AnthropicClient<T>
where
    T: AnthropicTransport,
{
    fn stream_tool_aware_message(
        &self,
        endpoint: &str,
        model: &str,
        messages: &[ChatMessage],
        tools: &[ProviderToolDefinition],
        on_event: &mut dyn FnMut(ChatStreamEvent) -> Result<(), OberonError>,
    ) -> Result<ToolAwareProviderResponse, OberonError> {
        let request = AnthropicRequest {
            model: model.to_string(),
            max_tokens: 1024,
            messages: messages
                .iter()
                .cloned()
                .map(into_anthropic_message)
                .collect(),
            tools: tools
                .iter()
                .cloned()
                .map(|tool| AnthropicToolDefinition {
                    name: tool.name,
                    description: tool.description,
                    input_schema: tool.parameters_schema,
                })
                .collect(),
        };
        let body = serde_json::to_string(&request)
            .map_err(|err| OberonError::Storage(format!("request serialization failed: {err}")))?;
        let url = format!(
            "{}/v1/messages",
            self.effective_base_url(endpoint).trim_end_matches('/')
        );
        let response = self.transport.post_json(
            &url,
            &body,
            self.retry_policy.timeout_ms,
            self.bearer_token()?.as_deref(),
        )?;
        let decoded: AnthropicResponse = serde_json::from_str(&response).map_err(|err| {
            OberonError::Validation(format!("malformed provider response: {err}"))
        })?;
        let mut content = String::new();
        let mut tool_calls = Vec::new();
        for (index, block) in decoded.content.iter().enumerate() {
            match block {
                AnthropicContentBlock::Text { text } => {
                    if !text.is_empty() {
                        content.push_str(text);
                        on_event(ChatStreamEvent::TextDelta {
                            content: text.clone(),
                        })?;
                    }
                }
                AnthropicContentBlock::ToolUse { id, name, input } => {
                    let arguments = serde_json::to_string(input)
                        .map_err(|err| OberonError::Storage(err.to_string()))?;
                    on_event(ChatStreamEvent::ToolCallDelta(ChatToolCallDelta {
                        index,
                        id_fragment: Some(id.clone()),
                        name_fragment: Some(name.clone()),
                        arguments_fragment: Some(arguments.clone()),
                    }))?;
                    tool_calls.push(ChatToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        arguments,
                    });
                }
                AnthropicContentBlock::ToolResult { .. } => {}
            }
        }
        Ok(ToolAwareProviderResponse {
            message: ChatMessage {
                role: ChatRole::Assistant,
                content,
                tool_call_id: None,
                tool_calls,
            },
            finish_state: if decoded.stop_reason.as_deref() == Some("tool_use") {
                ProviderFinishState::ToolCallRequested
            } else {
                ProviderFinishState::ResponseComplete
            },
        })
    }

    fn provider_capabilities(&self) -> ProviderCapabilityMatrix {
        ProviderCapabilityMatrix {
            tool_definitions: true,
            non_streaming_tool_calls: true,
            streaming_tool_calls: true,
            partial_argument_deltas: false,
            mixed_text_and_tool_responses: true,
            parallel_tool_calls: true,
            schema_cleaning_required: false,
            delegated_pending_support: false,
        }
    }

    fn apply_runtime_config(&mut self, config: &AppConfig) -> Result<(), OberonError> {
        self.set_retry_policy(RetryPolicy::from_config(config));
        self.set_api_key_env(config.endpoint.api_key_env.clone());
        Ok(())
    }

    fn endpoint_health(&self, endpoint: &str, _model: &str) -> Result<(), OberonError> {
        let url = format!(
            "{}/v1/models",
            self.effective_base_url(endpoint).trim_end_matches('/')
        );
        self.transport.get_text(
            &url,
            self.retry_policy.timeout_ms,
            self.bearer_token()?.as_deref(),
        )?;
        Ok(())
    }
}

fn into_anthropic_message(message: ChatMessage) -> AnthropicMessage {
    let role = match message.role {
        ChatRole::System => "user".to_string(),
        ChatRole::User => "user".to_string(),
        ChatRole::Assistant => "assistant".to_string(),
        ChatRole::Tool => "user".to_string(),
    };
    let content = match message.role {
        ChatRole::Tool => vec![AnthropicContentBlock::ToolResult {
            tool_use_id: message
                .tool_call_id
                .unwrap_or_else(|| "tool_call".to_string()),
            content: message.content,
            is_error: None,
        }],
        ChatRole::Assistant if !message.tool_calls.is_empty() => {
            let mut blocks = Vec::new();
            if !message.content.is_empty() {
                blocks.push(AnthropicContentBlock::Text {
                    text: message.content,
                });
            }
            blocks.extend(message.tool_calls.into_iter().map(|tool_call| {
                AnthropicContentBlock::ToolUse {
                    id: tool_call.id,
                    name: tool_call.name,
                    input: serde_json::from_str(&tool_call.arguments)
                        .unwrap_or_else(|_| json!({ "raw": tool_call.arguments })),
                }
            }));
            blocks
        }
        _ => vec![AnthropicContentBlock::Text {
            text: message.content,
        }],
    };

    AnthropicMessage { role, content }
}

#[derive(Debug)]
pub struct FakeTransport {
    responses: RefCell<VecDeque<Result<String, OberonError>>>,
    requests: RefCell<Vec<FakeRequestRecord>>,
}

type FakeRequestRecord = (String, String, u64, Option<String>);

impl FakeTransport {
    pub fn new(responses: Vec<Result<String, OberonError>>) -> Self {
        Self {
            responses: RefCell::new(responses.into()),
            requests: RefCell::new(Vec::new()),
        }
    }

    pub fn requests(&self) -> Vec<FakeRequestRecord> {
        self.requests.borrow().clone()
    }
}

impl AnthropicTransport for FakeTransport {
    fn post_json(
        &self,
        url: &str,
        _body: &str,
        timeout_ms: u64,
        api_key: Option<&str>,
    ) -> Result<String, OberonError> {
        self.requests.borrow_mut().push((
            "POST".to_string(),
            url.to_string(),
            timeout_ms,
            api_key.map(str::to_string),
        ));
        self.responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| Err(OberonError::Storage("no fake response queued".to_string())))
    }

    fn get_text(
        &self,
        url: &str,
        timeout_ms: u64,
        api_key: Option<&str>,
    ) -> Result<String, OberonError> {
        self.requests.borrow_mut().push((
            "GET".to_string(),
            url.to_string(),
            timeout_ms,
            api_key.map(str::to_string),
        ));
        self.responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| Err(OberonError::Storage("no fake response queued".to_string())))
    }
}

#[cfg(test)]
mod tests {
    use super::{AnthropicClient, FakeTransport, RetryPolicy};
    use oberon4_core::{
        ChatMessage, ChatRole, ProviderFinishState, ProviderToolDefinition, ToolAwareChatProvider,
    };
    use serde_json::json;

    #[test]
    fn anthropic_tool_use_is_normalized() {
        let transport = FakeTransport::new(vec![Ok(
            r#"{"id":"msg_1","content":[{"type":"text","text":"Working"},{"type":"tool_use","id":"toolu_1","name":"tool_search","input":{"query":"git diff"}}],"stop_reason":"tool_use"}"#.to_string(),
        )]);
        let client = AnthropicClient::new(
            "https://api.anthropic.com",
            None,
            RetryPolicy {
                timeout_ms: 1000,
                max_retries: 0,
            },
            transport,
        );

        let response = client
            .send_tool_aware_message(
                "",
                "claude-3-7-sonnet",
                &[ChatMessage {
                    role: ChatRole::User,
                    content: "find git diff".to_string(),
                    tool_call_id: None,
                    tool_calls: Vec::new(),
                }],
                &[ProviderToolDefinition {
                    name: "tool_search".to_string(),
                    description: "search tools".to_string(),
                    parameters_schema: json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}),
                }],
            )
            .unwrap();

        assert_eq!(
            response.finish_state,
            ProviderFinishState::ToolCallRequested
        );
        assert_eq!(response.message.tool_calls.len(), 1);
        assert_eq!(response.message.tool_calls[0].name, "tool_search");
    }
}
