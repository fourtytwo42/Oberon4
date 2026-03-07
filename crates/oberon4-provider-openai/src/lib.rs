use oberon4_core::AppConfig;

pub fn endpoint_summary(config: &AppConfig) -> String {
    format!("{} -> {}", config.endpoint.base_url, config.endpoint.model)
}

#[cfg(test)]
mod tests {
    use super::endpoint_summary;
    use oberon4_core::AppConfig;

    #[test]
    fn endpoint_summary_uses_shared_config() {
        let summary = endpoint_summary(&AppConfig::default());
        assert!(summary.contains("https://api.openai.com"));
    }
}
