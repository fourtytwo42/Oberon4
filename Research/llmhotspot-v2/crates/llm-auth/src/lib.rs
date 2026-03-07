mod account_manager;
mod callback_loopback;
mod claims;
mod oauth_flow;

pub use account_manager::{AccountAccessSession, ManagedAccount, OAuthAccountManager};
pub use callback_loopback::{
    start_oauth_callback_server, OAuthCallbackPayload, OAuthCallbackServer,
};
pub use claims::{decode_jwt_payload, openai_chatgpt_account_id, openai_email_from_access_token};
pub use oauth_flow::{
    pkce_challenge_s256, random_urlsafe_token, InMemoryOAuthStateStore, OAuthStart,
    OAuthStateRecord, OpenAiChatGptOAuthClient, OpenAiOAuthConfig, TokenBundle, WhamUsage,
    WhamUsageWindow,
};

#[derive(Debug, thiserror::Error, Clone)]
pub enum AuthError {
    #[error("invalid oauth state")]
    InvalidState,
    #[error("oauth code missing")]
    CodeMissing,
    #[error("oauth redirect mismatch")]
    RedirectMismatch,
    #[error("refresh token missing")]
    RefreshMissing,
    #[error("invalid config: {0}")]
    InvalidConfig(String),
    #[error("http error: {0}")]
    Http(String),
    #[error("token exchange failed: status={status} body={body}")]
    TokenExchangeFailed { status: u16, body: String },
    #[error("refresh failed: status={status} body={body}")]
    RefreshFailed { status: u16, body: String },
    #[error("token payload invalid: {0}")]
    TokenPayloadInvalid(String),
    #[error("oauth callback bind failed: {0}")]
    CallbackBindFailed(String),
    #[error("oauth callback missing required fields")]
    CallbackMissingFields,
    #[error("oauth callback error: {0}")]
    CallbackError(String),
    #[error("storage error: {0}")]
    Storage(String),
}

impl From<reqwest::Error> for AuthError {
    fn from(value: reqwest::Error) -> Self {
        Self::Http(value.to_string())
    }
}
