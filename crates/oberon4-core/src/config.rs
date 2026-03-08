use serde::{Deserialize, Serialize};

use crate::error::OberonError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AppConfig {
    pub endpoint: EndpointConfig,
    pub runtime: RuntimeConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EndpointConfig {
    pub base_url: String,
    pub model: String,
    pub api_key_env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeConfig {
    pub request_timeout_ms: u64,
    pub stream_idle_timeout_ms: u64,
    pub max_retries: u8,
    pub log_level: String,
    pub stream_responses: bool,
}

impl AppConfig {
    pub fn from_toml(input: &str) -> Result<Self, OberonError> {
        toml::from_str(input).map_err(OberonError::ConfigParse)
    }

    pub fn to_toml(&self) -> Result<String, OberonError> {
        toml::to_string_pretty(self).map_err(OberonError::ConfigSerialize)
    }
}

impl Default for EndpointConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com".to_string(),
            model: "gpt-4.1-mini".to_string(),
            api_key_env: Some("OPENAI_API_KEY".to_string()),
        }
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            request_timeout_ms: 30_000,
            stream_idle_timeout_ms: 30_000,
            max_retries: 2,
            log_level: "info".to_string(),
            stream_responses: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AppConfig;

    #[test]
    fn default_config_round_trips_through_toml() {
        let config = AppConfig::default();
        let encoded = config.to_toml().expect("config should serialize");
        let decoded = AppConfig::from_toml(&encoded).expect("config should parse");
        assert_eq!(decoded, config);
    }

    #[test]
    fn invalid_toml_returns_parse_error() {
        let err = AppConfig::from_toml("endpoint = [").expect_err("invalid TOML should fail");
        assert!(matches!(err, crate::error::OberonError::ConfigParse(_)));
    }

    #[test]
    fn missing_runtime_fields_fall_back_to_defaults() {
        let decoded = AppConfig::from_toml(
            r#"
[endpoint]
base_url = "http://gateway.example.test"
model = "gpt-5.1-codex-mini"
api_key_env = "OPENAI_API_KEY"

[runtime]
request_timeout_ms = 8000
max_retries = 0
log_level = "info"
stream_responses = true
"#,
        )
        .expect("config with legacy runtime fields should parse");

        assert_eq!(decoded.runtime.request_timeout_ms, 8000);
        assert_eq!(decoded.runtime.stream_idle_timeout_ms, 30_000);
        assert_eq!(decoded.runtime.max_retries, 0);
        assert!(decoded.runtime.stream_responses);
    }
}
