use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    AuthError,
    TenantOffline,
    RateLimitExceeded,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ErrorEnvelope {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ChatRequest {
    pub model: String,
    pub input: String,
    #[serde(default)]
    pub stream: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ChatResponse {
    pub id: String,
    pub output_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct OpenAiRequestEnvelope {
    #[serde(flatten)]
    pub raw: Map<String, Value>,
}

impl OpenAiRequestEnvelope {
    pub fn model(&self) -> Option<&str> {
        self.raw.get("model").and_then(Value::as_str)
    }

    pub fn stream(&self) -> bool {
        self.raw
            .get("stream")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    pub fn messages(&self) -> Option<&Vec<Value>> {
        self.raw.get("messages").and_then(Value::as_array)
    }

    pub fn input(&self) -> Option<&Value> {
        self.raw.get("input")
    }

    pub fn tools(&self) -> Option<&Vec<Value>> {
        self.raw.get("tools").and_then(Value::as_array)
    }

    pub fn tool_choice(&self) -> Option<&Value> {
        self.raw.get("tool_choice")
    }

    pub fn response_format(&self) -> Option<&Value> {
        self.raw.get("response_format")
    }

    pub fn max_output_tokens(&self) -> Option<u64> {
        self.raw.get("max_output_tokens").and_then(Value::as_u64)
    }

    pub fn temperature(&self) -> Option<f64> {
        self.raw.get("temperature").and_then(Value::as_f64)
    }

    pub fn metadata(&self) -> Option<&Map<String, Value>> {
        self.raw.get("metadata").and_then(Value::as_object)
    }
}
