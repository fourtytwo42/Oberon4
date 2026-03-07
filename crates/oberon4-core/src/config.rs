use serde::{Deserialize, Serialize};

use crate::error::OberonError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    pub endpoint: EndpointConfig,
    pub runtime: RuntimeConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointConfig {
    pub base_url: String,
    pub model: String,
    pub api_key_env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub request_timeout_ms: u64,
    pub max_retries: u8,
    pub log_level: String,
}

impl AppConfig {
    pub fn from_toml(input: &str) -> Result<Self, OberonError> {
        toml::from_str(input).map_err(OberonError::ConfigParse)
    }

    pub fn to_toml(&self) -> Result<String, OberonError> {
        toml::to_string_pretty(self).map_err(OberonError::ConfigSerialize)
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            endpoint: EndpointConfig {
                base_url: "https://api.openai.com".to_string(),
                model: "gpt-4.1-mini".to_string(),
                api_key_env: Some("OPENAI_API_KEY".to_string()),
            },
            runtime: RuntimeConfig {
                request_timeout_ms: 30_000,
                max_retries: 2,
                log_level: "info".to_string(),
            },
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
}
