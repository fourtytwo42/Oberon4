use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeInfo {
    pub service_name: String,
    pub version: String,
    pub started_unix: u64,
}

impl RuntimeInfo {
    pub fn new(service_name: &str, version: &str) -> Self {
        Self {
            service_name: service_name.to_string(),
            version: version.to_string(),
            started_unix: now_unix(),
        }
    }
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
