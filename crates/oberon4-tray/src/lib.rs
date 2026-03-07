use oberon4_core::AppConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayPlatform {
    Linux,
    MacOs,
    Windows,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    OpenUi,
    ShowStatus,
    Restart,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayStatus {
    pub supported: bool,
    pub platform: TrayPlatform,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayActionResult {
    pub action: TrayAction,
    pub performed: bool,
    pub message: String,
}

pub trait TrayAdapter {
    fn platform(&self) -> TrayPlatform;
    fn status(&self, config: &AppConfig) -> TrayStatus;
    fn perform(&mut self, action: TrayAction, config: &AppConfig) -> TrayActionResult;
}

#[derive(Debug, Default)]
pub struct UnsupportedTrayAdapter;

impl TrayAdapter for UnsupportedTrayAdapter {
    fn platform(&self) -> TrayPlatform {
        TrayPlatform::Unsupported
    }

    fn status(&self, config: &AppConfig) -> TrayStatus {
        TrayStatus {
            supported: false,
            platform: TrayPlatform::Unsupported,
            message: format!(
                "tray unavailable on this platform for {}",
                config.endpoint.base_url
            ),
        }
    }

    fn perform(&mut self, action: TrayAction, _config: &AppConfig) -> TrayActionResult {
        TrayActionResult {
            action,
            performed: false,
            message: "tray action unavailable on this platform".to_string(),
        }
    }
}

#[derive(Debug, Default)]
pub struct MockTrayAdapter {
    pub actions: Vec<TrayAction>,
}

impl TrayAdapter for MockTrayAdapter {
    fn platform(&self) -> TrayPlatform {
        TrayPlatform::Linux
    }

    fn status(&self, config: &AppConfig) -> TrayStatus {
        TrayStatus {
            supported: true,
            platform: TrayPlatform::Linux,
            message: format!("tray ready for {}", config.endpoint.base_url),
        }
    }

    fn perform(&mut self, action: TrayAction, _config: &AppConfig) -> TrayActionResult {
        self.actions.push(action);
        TrayActionResult {
            action,
            performed: true,
            message: match action {
                TrayAction::OpenUi => "opening ui".to_string(),
                TrayAction::ShowStatus => "showing status".to_string(),
                TrayAction::Restart => "restarting daemon".to_string(),
                TrayAction::Quit => "quitting tray".to_string(),
            },
        }
    }
}

pub fn tray_status(config: &AppConfig) -> String {
    format!("tray ready for {}", config.endpoint.base_url)
}

pub fn default_platform() -> TrayPlatform {
    if cfg!(target_os = "linux") {
        TrayPlatform::Linux
    } else if cfg!(target_os = "macos") {
        TrayPlatform::MacOs
    } else if cfg!(target_os = "windows") {
        TrayPlatform::Windows
    } else {
        TrayPlatform::Unsupported
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MockTrayAdapter, TrayAction, TrayAdapter, TrayPlatform, UnsupportedTrayAdapter,
        default_platform, tray_status,
    };
    use oberon4_core::AppConfig;

    #[test]
    fn tray_status_uses_shared_config() {
        let status = tray_status(&AppConfig::default());
        assert!(status.contains("https://api.openai.com"));
    }

    #[test]
    fn unsupported_adapter_reports_explicit_status() {
        let adapter = UnsupportedTrayAdapter;
        let status = adapter.status(&AppConfig::default());
        assert!(!status.supported);
        assert_eq!(status.platform, TrayPlatform::Unsupported);
        assert!(status.message.contains("tray unavailable"));
    }

    #[test]
    fn unsupported_adapter_rejects_all_actions() {
        let mut adapter = UnsupportedTrayAdapter;
        for action in [
            TrayAction::OpenUi,
            TrayAction::ShowStatus,
            TrayAction::Restart,
            TrayAction::Quit,
        ] {
            let result = adapter.perform(action, &AppConfig::default());
            assert!(!result.performed);
            assert_eq!(result.action, action);
        }
    }

    #[test]
    fn mock_adapter_supports_all_required_actions() {
        let mut adapter = MockTrayAdapter::default();
        let config = AppConfig::default();

        let open = adapter.perform(TrayAction::OpenUi, &config);
        let status = adapter.perform(TrayAction::ShowStatus, &config);
        let restart = adapter.perform(TrayAction::Restart, &config);
        let quit = adapter.perform(TrayAction::Quit, &config);

        assert!(open.performed);
        assert_eq!(status.message, "showing status");
        assert_eq!(restart.message, "restarting daemon");
        assert_eq!(quit.message, "quitting tray");
        assert_eq!(adapter.actions.len(), 4);
    }

    #[test]
    fn mock_adapter_status_reports_supported_platform() {
        let adapter = MockTrayAdapter::default();
        let status = adapter.status(&AppConfig::default());
        assert!(status.supported);
        assert_eq!(status.platform, TrayPlatform::Linux);
    }

    #[test]
    fn default_platform_resolves_to_known_variant() {
        let platform = default_platform();
        assert!(matches!(
            platform,
            TrayPlatform::Linux
                | TrayPlatform::MacOs
                | TrayPlatform::Windows
                | TrayPlatform::Unsupported
        ));
    }
}
