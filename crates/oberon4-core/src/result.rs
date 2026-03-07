use serde::{Deserialize, Serialize};

use crate::{OberonError, RequestId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceEnvelope<T> {
    pub ok: bool,
    pub request_id: RequestId,
    pub data: Option<T>,
    pub error: Option<String>,
}

impl<T> ServiceEnvelope<T> {
    pub fn success(data: T) -> Self {
        Self {
            ok: true,
            request_id: RequestId::new(),
            data: Some(data),
            error: None,
        }
    }

    pub fn failure(error: &OberonError) -> Self {
        Self {
            ok: false,
            request_id: RequestId::new(),
            data: None,
            error: Some(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ServiceEnvelope;
    use crate::OberonError;

    #[test]
    fn success_envelope_carries_data() {
        let envelope = ServiceEnvelope::success("ok".to_string());
        assert!(envelope.ok);
        assert_eq!(envelope.data.as_deref(), Some("ok"));
        assert!(envelope.error.is_none());
    }

    #[test]
    fn failure_envelope_carries_sanitized_error() {
        let envelope =
            ServiceEnvelope::<String>::failure(&OberonError::Validation("bad input".into()));
        assert!(!envelope.ok);
        assert!(envelope.data.is_none());
        assert_eq!(
            envelope.error.as_deref(),
            Some("validation failed: bad input")
        );
    }
}
