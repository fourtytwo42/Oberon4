pub mod failover;
pub mod openai_compat;
pub mod relay;
pub mod upstream;

pub use failover::{chat_with_failover, AccountGatewayClient};
pub use openai_compat::{DefaultGatewayService, GatewayService, GatewayStatus};
pub use relay::{assemble_chunks, BackoffState, RelayErrorKind, RelayFrame};
pub use upstream::{
    forward_openai_json, forward_openai_sse, resolve_codex_oauth_config, resolve_request_config,
    UpstreamJsonResponse, UpstreamRequestConfig, UpstreamRoutingProfile, UpstreamSseResponse,
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct GatewayErrorDetail {
    pub kind: GatewayError,
    pub url: Option<String>,
    pub status: Option<u16>,
    pub body_snippet: Option<String>,
    pub transport_message: Option<String>,
}

impl GatewayErrorDetail {
    pub fn new(kind: GatewayError) -> Self {
        Self {
            kind,
            url: None,
            status: None,
            body_snippet: None,
            transport_message: None,
        }
    }

    pub fn with_url(mut self, url: String) -> Self {
        self.url = Some(url);
        self
    }

    pub fn with_status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_body_snippet(mut self, body: String) -> Self {
        self.body_snippet = Some(body);
        self
    }

    pub fn with_transport_message(mut self, msg: String) -> Self {
        self.transport_message = Some(msg);
        self
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, thiserror::Error, PartialEq, Eq)]
pub enum GatewayError {
    #[error("auth error")]
    Auth,
    #[error("tenant offline")]
    TenantOffline,
    #[error("rate limit")]
    RateLimit,
    #[error("upstream timeout")]
    Timeout,
    #[error("bad gateway")]
    BadGateway,
}

impl GatewayError {
    pub fn http_status(&self) -> u16 {
        match self {
            GatewayError::Auth => 401,
            GatewayError::TenantOffline => 503,
            GatewayError::RateLimit => 429,
            GatewayError::Timeout | GatewayError::BadGateway => 502,
        }
    }
}
