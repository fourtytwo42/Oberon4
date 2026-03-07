use thiserror::Error;

#[derive(Debug, Error)]
pub enum OberonError {
    #[error("configuration parse error: {0}")]
    ConfigParse(#[source] toml::de::Error),
    #[error("configuration serialization error: {0}")]
    ConfigSerialize(#[source] toml::ser::Error),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("storage failure: {0}")]
    Storage(String),
}
