use serde::{Deserialize, Serialize};

use crate::{ChatProviderKind, error::OberonError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AppConfig {
    pub endpoint: EndpointConfig,
    pub runtime: RuntimeConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EndpointConfig {
    pub provider: ChatProviderKind,
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
            provider: ChatProviderKind::OpenAiCompatible,
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
    use crate::ChatProviderKind;

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
        let decoded = AppConfig::from_toml(include_str!(
            "../tests/fixtures/config-legacy-missing-runtime.toml"
        ))
        .expect("config with legacy runtime fields should parse");

        assert_eq!(decoded.runtime.request_timeout_ms, 8000);
        assert_eq!(decoded.runtime.stream_idle_timeout_ms, 30_000);
        assert_eq!(decoded.runtime.max_retries, 0);
        assert!(decoded.runtime.stream_responses);
    }

    #[test]
    fn missing_api_key_env_falls_back_to_default() {
        let decoded = AppConfig::from_toml(include_str!(
            "../tests/fixtures/config-legacy-missing-api-key-env.toml"
        ))
        .expect("config without api_key_env should parse");

        assert_eq!(decoded.endpoint.base_url, "http://gateway.example.test");
        assert_eq!(
            decoded.endpoint.provider,
            ChatProviderKind::OpenAiCompatible
        );
        assert_eq!(decoded.endpoint.model, "gpt-5.1-codex-mini");
        assert_eq!(
            decoded.endpoint.api_key_env.as_deref(),
            Some("OPENAI_API_KEY")
        );
        assert_eq!(decoded.runtime.request_timeout_ms, 12_000);
        assert_eq!(decoded.runtime.stream_idle_timeout_ms, 9_000);
        assert!(!decoded.runtime.stream_responses);
    }

    #[test]
    fn endpoint_only_config_uses_runtime_defaults() {
        let decoded =
            AppConfig::from_toml(include_str!("../tests/fixtures/config-endpoint-only.toml"))
                .expect("endpoint-only config should parse");

        assert_eq!(decoded.endpoint.base_url, "http://gateway.example.test");
        assert_eq!(
            decoded.endpoint.provider,
            ChatProviderKind::OpenAiCompatible
        );
        assert_eq!(decoded.endpoint.model, "gpt-5.1-codex-mini");
        assert_eq!(
            decoded.endpoint.api_key_env.as_deref(),
            Some("OPENAI_API_KEY")
        );
        assert_eq!(decoded.runtime, super::RuntimeConfig::default());
    }
}
