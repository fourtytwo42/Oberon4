use super::*;
use std::sync::OnceLock;

#[derive(Clone)]
pub(crate) struct HttpClients {
    default_json: reqwest::Client,
    long_stream: reqwest::Client,
    oauth_short: reqwest::Client,
}

impl Default for HttpClients {
    fn default() -> Self {
        fn build(timeout: std::time::Duration) -> reqwest::Client {
            reqwest::Client::builder()
                .timeout(timeout)
                .build()
                .unwrap_or_else(|_| reqwest::Client::new())
        }

        Self {
            default_json: reqwest::Client::new(),
            long_stream: build(std::time::Duration::from_secs(300)),
            oauth_short: build(std::time::Duration::from_secs(15)),
        }
    }
}

impl HttpClients {
    pub(crate) fn default_json(&self) -> &reqwest::Client {
        &self.default_json
    }

    pub(crate) fn long_stream(&self) -> &reqwest::Client {
        &self.long_stream
    }

    pub(crate) fn oauth_short(&self) -> &reqwest::Client {
        &self.oauth_short
    }
}

fn fallback_clients() -> &'static HttpClients {
    static FALLBACK: OnceLock<HttpClients> = OnceLock::new();
    FALLBACK.get_or_init(HttpClients::default)
}

pub(crate) fn http_default_for(state: Option<&AppState>) -> reqwest::Client {
    state
        .map(|s| s.http_default().clone())
        .unwrap_or_else(|| fallback_clients().default_json().clone())
}
