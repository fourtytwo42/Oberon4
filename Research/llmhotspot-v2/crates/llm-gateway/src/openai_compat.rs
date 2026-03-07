use crate::GatewayError;
use llm_foundation::contracts::{ChatRequest, ChatResponse};

pub trait GatewayService: Send + Sync {
    fn models(&self) -> Vec<String>;
    fn chat(&self, req: ChatRequest) -> Result<ChatResponse, GatewayError>;
    fn responses(&self, req: ChatRequest) -> Result<ChatResponse, GatewayError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayStatus {
    Connected,
    Degraded,
    Offline,
}

#[derive(Debug, Clone)]
pub struct DefaultGatewayService {
    pub status: GatewayStatus,
}

impl Default for DefaultGatewayService {
    fn default() -> Self {
        Self {
            status: GatewayStatus::Connected,
        }
    }
}

impl GatewayService for DefaultGatewayService {
    fn models(&self) -> Vec<String> {
        vec![
            "gpt-5.3-codex".to_string(),
            "gpt-5.3-codex-spark".to_string(),
            "gpt-5.2-codex".to_string(),
            "gpt-5.2".to_string(),
            "gpt-5.1-codex-max".to_string(),
            "gpt-5.1-codex".to_string(),
            "gpt-5.1-codex-mini".to_string(),
        ]
    }

    fn chat(&self, req: ChatRequest) -> Result<ChatResponse, GatewayError> {
        if self.status == GatewayStatus::Offline {
            return Err(GatewayError::TenantOffline);
        }
        if req.input.contains("rate_limit") {
            return Err(GatewayError::RateLimit);
        }
        if req.input.contains("timeout") {
            return Err(GatewayError::Timeout);
        }

        Ok(ChatResponse {
            id: "chat_1".to_string(),
            output_text: format!("echo:{}", req.input),
        })
    }

    fn responses(&self, req: ChatRequest) -> Result<ChatResponse, GatewayError> {
        self.chat(req)
    }
}
