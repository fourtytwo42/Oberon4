use super::*;
use std::time::{SystemTime, UNIX_EPOCH};
pub(crate) fn normalized_base(base: &str) -> String {
    base.trim_end_matches('/').to_string()
}

pub(crate) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn chatcmpl_id() -> String {
    format!("chatcmpl-{}", uuid::Uuid::new_v4().simple())
}

pub(crate) fn sse_chunk(
    id: &str,
    created: u64,
    model: &str,
    delta: Value,
    finish_reason: Value,
) -> Value {
    json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "system_fingerprint": Value::Null,
        "choices": [{"index": 0, "delta": delta, "finish_reason": finish_reason, "logprobs": Value::Null}]
    })
}
