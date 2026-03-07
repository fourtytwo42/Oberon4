use llm_gateway::{GatewayError, GatewayErrorDetail};
use llm_storage::SqliteStore;
use serde_json::Value;
use std::collections::HashMap;

use super::{
    project_chat_stream_from_responses_sse, AppState, ChatStreamProjection, RoutedResponse,
    SseFrameParser,
};

use crate::message_conversion::{anthropic_response_to_openai, build_anthropic_payload};
use crate::oauth_handlers::{
    get_anthropic_access_token, is_anthropic_oauth_token, ANTHROPIC_OAUTH_BETA_HEADER,
    ANTHROPIC_PROVIDER_ID, ANTHROPIC_UPSTREAM, GOOGLE_PROVIDER_ID, OPENAI_PROVIDER_ID,
};

pub(crate) fn parse_headers_json(raw: Option<&str>) -> HashMap<String, String> {
    let Some(raw) = raw else {
        return HashMap::new();
    };
    serde_json::from_str::<HashMap<String, String>>(raw).unwrap_or_default()
}

pub(crate) fn chat_content_to_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.to_string(),
        Value::Array(parts) => {
            let mut out = String::new();
            for part in parts {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(text);
                    }
                }
            }
            out
        }
        Value::Object(obj) => obj
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    }
}

pub(crate) fn tool_trace_name(tool: &Value) -> Option<String> {
    if let Some(name) = tool.get("name").and_then(Value::as_str) {
        return Some(name.to_string());
    }
    tool.get("function")
        .and_then(|f| f.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Returns the reasoning effort to forward to Codex.
/// Checks `reasoning_effort` (Chat Completions style) then `reasoning.effort`
/// (Responses API style). Falls back to "medium" if neither is set.
pub(crate) fn resolve_reasoning_effort(raw: &Value) -> &'static str {
    // Chat Completions style: "reasoning_effort": "high"
    if let Some(s) = raw.get("reasoning_effort").and_then(Value::as_str) {
        return normalize_effort(s);
    }
    // Responses API style: "reasoning": {"effort": "high"}
    if let Some(s) = raw
        .get("reasoning")
        .and_then(|r| r.get("effort"))
        .and_then(Value::as_str)
    {
        return normalize_effort(s);
    }
    "medium"
}

pub(crate) fn normalize_effort(s: &str) -> &'static str {
    match s.to_ascii_lowercase().as_str() {
        "low" => "low",
        "high" => "high",
        _ => "medium",
    }
}

pub(crate) fn resolve_codex_model(requested: &str) -> &str {
    let lower = requested.to_ascii_lowercase();
    let _ = lower; // used below for prefix stripping
                   // Strip any "openai-codex/" prefix (e.g. from openclaw clients).
    if let Some(stripped) = requested.strip_prefix("openai-codex/") {
        return stripped;
    }
    requested
}

pub(crate) fn is_claude_model(model: &str) -> bool {
    model.to_ascii_lowercase().starts_with("claude")
}

pub(crate) fn is_google_model(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    m.starts_with("gemini") || m.starts_with("google/")
}

pub(crate) fn is_openai_model(model: &str) -> bool {
    model.to_ascii_lowercase().starts_with("gpt")
}

pub(crate) fn strip_trailing_model_date_suffix(model: &str) -> String {
    let trimmed = model.trim();
    if let Some((head, tail)) = trimmed.rsplit_once('-') {
        let is_yyyymmdd = tail.len() == 8 && tail.chars().all(|c| c.is_ascii_digit());
        if is_yyyymmdd {
            return head.to_string();
        }
    }
    trimmed.to_string()
}

pub(crate) fn normalize_model_name_for_routing(model: &str) -> String {
    let trimmed = model.trim();
    let without_prefix = trimmed
        .strip_prefix("google/")
        .or_else(|| trimmed.strip_prefix("models/"))
        .unwrap_or(trimmed);
    strip_trailing_model_date_suffix(without_prefix)
}

pub(crate) fn resolve_google_model(requested: &str) -> String {
    let normalized = normalize_model_name_for_routing(requested)
        .trim()
        .to_ascii_lowercase();
    let compact = normalized.replace(['.', '_', ' '], "-");

    if compact.starts_with("gemini-3-flash-preview") {
        return "gemini-3-flash".to_string();
    }

    compact
}

pub(crate) fn should_inject_google_antigravity_instruction(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    m.contains("claude") || m.contains("gemini-3-pro") || m.contains("gemini-3-flash")
}

pub(crate) fn resolve_provider_for_model(model: &str) -> Option<&'static str> {
    let normalized = normalize_model_name_for_routing(model);
    if is_openai_model(&normalized) {
        return Some(OPENAI_PROVIDER_ID);
    }
    if is_claude_model(&normalized) {
        return Some(ANTHROPIC_PROVIDER_ID);
    }
    if is_google_model(&normalized) {
        return Some(GOOGLE_PROVIDER_ID);
    }
    None
}

pub(crate) fn openai_last_message_is_tool_result(messages: &[Value]) -> bool {
    let Some(last) = messages.last() else {
        return false;
    };
    last.get("role").and_then(Value::as_str) == Some("tool")
        && last.get("tool_call_id").and_then(Value::as_str).is_some()
}

pub(crate) fn openai_last_message_has_anthropic_tool_result_block(messages: &[Value]) -> bool {
    let Some(last) = messages.last() else {
        return false;
    };
    last.get("role").and_then(Value::as_str) == Some("user")
        && last
            .get("content")
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .any(|p| p.get("type").and_then(Value::as_str) == Some("tool_result"))
            })
            .unwrap_or(false)
}

/// Route a Claude model request through a connected Anthropic OAuth account.
/// Always collects the response (no real-time streaming).
pub(crate) async fn route_anthropic_request(
    state: &AppState,
    store: &SqliteStore,
    req: &Value,
) -> Result<(RoutedResponse, Option<String>), GatewayError> {
    let accounts = store
        .list_oauth_accounts()
        .await
        .map_err(|_| GatewayError::Auth)?;
    let anthropic_accounts: Vec<_> = accounts
        .into_iter()
        .filter(|a| a.enabled && super::is_anthropic_account(a))
        .collect();
    if anthropic_accounts.is_empty() {
        return Err(GatewayError::Auth);
    }

    let model = req
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("claude-sonnet-4-5")
        .to_string();
    let mut payload = build_anthropic_payload(req);
    payload["stream"] = Value::Bool(false); // collect mode for now
    let mut saw_non_auth_upstream_error = false;
    let mut saw_rate_limited = false;
    let mut saw_auth_error = false;
    let mut last_detail: Option<(String, GatewayErrorDetail)> = None;

    for account in &anthropic_accounts {
        let token = match get_anthropic_access_token(Some(state), store, &account.account_id).await
        {
            Ok(t) => t,
            Err(_) => continue,
        };

        let http = state.http_long().clone();

        let mut req = http
            .post(format!("{}/v1/messages", ANTHROPIC_UPSTREAM))
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .header("anthropic-version", "2023-06-01");
        if is_anthropic_oauth_token(&token) {
            req = req.header("anthropic-beta", ANTHROPIC_OAUTH_BETA_HEADER);
        }
        let resp = match req.json(&payload).send().await {
            Ok(r) => r,
            Err(_) => continue,
        };

        let status = resp.status().as_u16();
        let body: Value = resp.json().await.unwrap_or_default();

        if status == 401 || status == 403 {
            saw_auth_error = true;
            continue;
        }
        if status == 429 {
            saw_rate_limited = true;
            let body_snippet = {
                let mut s = body.to_string().replace(['\r', '\n'], " ");
                s = s.trim().to_string();
                if s.len() > 240 {
                    s.truncate(240);
                    s.push_str("...");
                }
                s
            };
            last_detail = Some((
                account.account_id.clone(),
                GatewayErrorDetail::new(GatewayError::RateLimit)
                    .with_url(format!("{}/v1/messages", ANTHROPIC_UPSTREAM))
                    .with_status(status)
                    .with_body_snippet(body_snippet),
            ));
            continue;
        }
        if !(200..300).contains(&status) {
            saw_non_auth_upstream_error = true;
            let body_snippet = {
                let mut s = body.to_string().replace(['\r', '\n'], " ");
                s = s.trim().to_string();
                if s.len() > 240 {
                    s.truncate(240);
                    s.push_str("...");
                }
                s
            };
            last_detail = Some((
                account.account_id.clone(),
                GatewayErrorDetail::new(GatewayError::BadGateway)
                    .with_url(format!("{}/v1/messages", ANTHROPIC_UPSTREAM))
                    .with_status(status)
                    .with_body_snippet(body_snippet),
            ));
            continue;
        }

        let converted = anthropic_response_to_openai(&body, &model);
        return Ok((
            RoutedResponse::Json(converted),
            Some(account.account_id.clone()),
        ));
    }

    if let Some((account_id, detail)) = last_detail {
        let _ = super::record_last_upstream_error(state, &account_id, &detail).await;
    }
    if saw_rate_limited {
        Err(GatewayError::RateLimit)
    } else if saw_non_auth_upstream_error {
        Err(GatewayError::BadGateway)
    } else {
        let _ = saw_auth_error;
        Err(GatewayError::Auth)
    }
}

pub(crate) async fn collect_codex_sse(
    resp: reqwest::Response,
) -> Result<ChatStreamProjection, String> {
    let mut resp = resp;
    let mut buffer = String::new();
    let mut raw = String::new();
    let mut parser = SseFrameParser::default();

    let mut push_frame_data = |frame_data: &str| {
        let data = frame_data.trim();
        if data.is_empty() || data == "[DONE]" {
            return;
        }
        raw.push_str("data: ");
        raw.push_str(data);
        raw.push_str("\n\n");
    };

    loop {
        let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| format!("stream read: {e}"))?
        else {
            break;
        };
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(newline_pos) = buffer.find('\n') {
            let line = buffer[..newline_pos].to_string();
            buffer = buffer[newline_pos + 1..].to_string();
            if let Some(frame) = parser.push_line(&line) {
                push_frame_data(&frame.data);
            }
        }
    }
    if !buffer.trim().is_empty() {
        let _ = parser.push_line(&buffer);
    }
    if let Some(frame) = parser.flush() {
        push_frame_data(&frame.data);
    }
    if let Some(message) = extract_codex_sse_failure(&raw) {
        return Err(message);
    }
    Ok(project_chat_stream_from_responses_sse(&raw))
}

pub(crate) fn extract_codex_sse_failure(raw: &str) -> Option<String> {
    for line in raw.lines() {
        let Some(data) = line.strip_prefix("data: ") else {
            continue;
        };
        if data.trim().is_empty() || data.trim() == "[DONE]" {
            continue;
        }
        let Ok(event) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };
        if event.get("type").and_then(Value::as_str) != Some("response.failed") {
            continue;
        }
        let err_obj = event.get("response").and_then(|r| r.get("error"));
        let message = err_obj
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("Codex response failed");
        let code = err_obj.and_then(|e| e.get("code")).and_then(Value::as_str);
        let full = if let Some(c) = code {
            format!("{message} (code: {c})")
        } else {
            message.to_string()
        };
        return Some(full);
    }
    None
}
