use oberon4_core::AppConfig;

pub fn tray_status(config: &AppConfig) -> String {
    format!("tray ready for {}", config.endpoint.base_url)
}

#[cfg(test)]
mod tests {
    use super::tray_status;
    use oberon4_core::AppConfig;

    #[test]
    fn tray_status_uses_shared_config() {
        let status = tray_status(&AppConfig::default());
        assert!(status.contains("https://api.openai.com"));
    }
}
