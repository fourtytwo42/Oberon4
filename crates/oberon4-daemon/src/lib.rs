use oberon4_core::{AppConfig, ServiceEnvelope};

pub fn daemon_status(config: &AppConfig) -> ServiceEnvelope<String> {
    ServiceEnvelope::success(format!(
        "daemon configured for {} using model {}",
        config.endpoint.base_url, config.endpoint.model
    ))
}

#[cfg(test)]
mod tests {
    use super::daemon_status;
    use oberon4_core::AppConfig;

    #[test]
    fn daemon_status_uses_shared_config() {
        let status = daemon_status(&AppConfig::default());
        assert!(status.ok);
        assert!(status.data.unwrap().contains("https://api.openai.com"));
    }
}
