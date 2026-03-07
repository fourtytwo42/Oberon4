use super::*;

pub(crate) fn trace_io_enabled() -> bool {
    parse_trace_flag("LLMHOTSPOT_TRACE_IO")
}

pub(crate) fn compat_trace_enabled() -> bool {
    parse_trace_flag("LLMHOTSPOT_TRACE_COMPAT")
}

pub(crate) fn parse_trace_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|v| {
            let n = v.trim().to_ascii_lowercase();
            n == "1" || n == "true" || n == "yes" || n == "on"
        })
        .unwrap_or(false)
}

pub(crate) fn trace_io(event: &str, payload: Value) {
    if !trace_io_enabled() {
        return;
    }
    eprintln!(
        "[llmhotspot-trace] {}",
        json!({
            "event": event,
            "at_unix": now_unix(),
            "payload": payload,
        })
    );
}

pub(crate) fn redact_header_map(headers: &HeaderMap) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for (name, value) in headers {
        let key = name.as_str().to_string();
        let raw = value.to_str().unwrap_or("<non-utf8>").to_string();
        let val = if is_sensitive_header_name(&key) {
            if raw.len() <= 16 {
                "***".to_string()
            } else {
                format!("{}***{}", &raw[..8], &raw[raw.len() - 4..])
            }
        } else {
            raw
        };
        out.insert(key, val);
    }
    out
}

pub(crate) fn is_sensitive_header_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("authorization")
        || name.eq_ignore_ascii_case("x-api-key")
        || name.eq_ignore_ascii_case("api-key")
        || name.eq_ignore_ascii_case("cookie")
        || name.eq_ignore_ascii_case("set-cookie")
}

pub(crate) fn request_format_kind(req: &Value) -> &'static str {
    match (req.get("messages").is_some(), req.get("input").is_some()) {
        (true, false) => "chat_completions",
        (false, true) => "responses",
        (true, true) => "hybrid",
        (false, false) => "unknown",
    }
}

pub(crate) fn request_trace_metadata(headers: &HeaderMap, req: &Value) -> Value {
    json!({
        "headers": redact_header_map(headers),
        "format": request_format_kind(req),
        "model": req.get("model").and_then(Value::as_str).unwrap_or(""),
        "stream": req.get("stream").and_then(Value::as_bool).unwrap_or(false),
        "message_count": req.get("messages").and_then(Value::as_array).map(|v| v.len()).unwrap_or(0),
        "input_count": req.get("input").and_then(Value::as_array).map(|v| v.len()).unwrap_or(0),
        "tool_count": req.get("tools").and_then(Value::as_array).map(|v| v.len()).unwrap_or(0),
        "has_tool_choice": req.get("tool_choice").is_some(),
    })
}

pub(crate) fn response_trace_summary(body: &Value) -> Value {
    json!({
        "id": body.get("id").and_then(Value::as_str).unwrap_or(""),
        "object": body.get("object").and_then(Value::as_str).unwrap_or(""),
        "model": body.get("model").and_then(Value::as_str).unwrap_or(""),
        "usage": body.get("usage").cloned().unwrap_or(Value::Null),
        "choices_count": body.get("choices").and_then(Value::as_array).map(|v| v.len()).unwrap_or(0),
    })
}

pub(crate) fn tool_profile(tools: &[Value]) -> Value {
    let mut wrapped_function = 0_u64;
    let mut flat_function = 0_u64;
    let mut custom = 0_u64;
    let mut unknown = 0_u64;
    for tool in tools {
        let kind = tool.get("type").and_then(Value::as_str).unwrap_or_default();
        match kind {
            "custom" => custom = custom.saturating_add(1),
            "function" => {
                if tool.get("function").is_some() {
                    wrapped_function = wrapped_function.saturating_add(1);
                } else if tool.get("name").is_some() {
                    flat_function = flat_function.saturating_add(1);
                } else {
                    unknown = unknown.saturating_add(1);
                }
            }
            _ => unknown = unknown.saturating_add(1),
        }
    }
    let names: Vec<String> = tools.iter().filter_map(tool_trace_name).take(8).collect();
    json!({
        "count": tools.len(),
        "wrapped_function": wrapped_function,
        "flat_function": flat_function,
        "custom": custom,
        "unknown": unknown,
        "names_head": names,
    })
}

#[cfg(test)]
mod request_normalization_tests {
    use super::{is_sensitive_header_name, normalize_incoming_path};

    #[test]
    fn normalizes_duplicate_and_misjoined_paths() {
        assert_eq!(
            normalize_incoming_path("/v1//chat/completions/"),
            "/v1/chat/completions"
        );
        assert_eq!(
            normalize_incoming_path("/v1/chat/completions/chat/completions"),
            "/v1/chat/completions"
        );
        assert_eq!(
            normalize_incoming_path("/v1/responses/chat/completions"),
            "/v1/chat/completions"
        );
        assert_eq!(
            normalize_incoming_path("/v1/chat/completions/responses"),
            "/v1/responses"
        );
    }

    #[test]
    fn marks_sensitive_headers_for_redaction() {
        assert!(is_sensitive_header_name("authorization"));
        assert!(is_sensitive_header_name("x-api-key"));
        assert!(is_sensitive_header_name("api-key"));
        assert!(is_sensitive_header_name("cookie"));
        assert!(is_sensitive_header_name("set-cookie"));
        assert!(!is_sensitive_header_name("x-request-id"));
    }
}
