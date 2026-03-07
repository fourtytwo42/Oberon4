use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

// ========== Structs ==========

#[derive(Debug, Clone, Default)]
pub(crate) struct ToolRepairResult {
    #[allow(dead_code)] // read in tests to verify normalization fired
    pub(crate) repaired: bool,
    pub(crate) unrecoverable: bool,
    pub(crate) reason: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct MessageRepairResult {
    pub(crate) repaired: bool,
    pub(crate) dropped_count: usize,
    pub(crate) pruned_count: usize,
}

// ========== Utilities ==========

pub(crate) fn hash_key(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub(crate) fn normalize_codex_call_id<'a>(
    original: &'a str,
    remap: &mut HashMap<String, String>,
) -> std::borrow::Cow<'a, str> {
    // Codex enforces `call_id` max length 64.
    if original.len() <= 64 {
        return std::borrow::Cow::Borrowed(original);
    }
    if let Some(mapped) = remap.get(original) {
        return std::borrow::Cow::Owned(mapped.clone());
    }
    // Stable, deterministic short id. Prefix keeps it obvious in logs.
    let digest = hash_key(original);
    let mapped = format!("cid_{}", &digest[..60]);
    remap.insert(original.to_string(), mapped.clone());
    std::borrow::Cow::Owned(mapped)
}

pub(crate) fn normalize_role(raw: &str) -> &str {
    match raw {
        "assistant" => "assistant",
        "system" | "developer" => "developer",
        _ => "user",
    }
}

pub(crate) fn max_inbound_messages() -> usize {
    std::env::var("LLMHOTSPOT_MAX_INBOUND_MESSAGES")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v >= 50)
        .unwrap_or(220)
}

pub(crate) fn normalize_message_role(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "system" => Some("system"),
        "developer" => Some("developer"),
        "user" => Some("user"),
        "assistant" | "model" => Some("assistant"),
        "tool" | "function" => Some("tool"),
        _ => None,
    }
}

pub(crate) fn looks_like_tool_result_item(v: &Value) -> bool {
    let t = v
        .get("type")
        .and_then(Value::as_str)
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    matches!(
        t.as_str(),
        "tool_result" | "tool_response" | "function_call_output" | "function_result"
    )
}

pub(crate) fn content_has_tool_result_block(content: &Value) -> bool {
    content
        .as_array()
        .map(|parts| parts.iter().any(looks_like_tool_result_item))
        .unwrap_or(false)
}

/// Returns true if the request already contains tool results in its conversation history,
/// meaning we're on a follow-up turn of an agentic loop.
pub(crate) fn request_has_tool_results(raw: &Value) -> bool {
    // Chat Completions: role "tool" messages
    if let Some(messages) = raw.get("messages").and_then(Value::as_array) {
        if messages
            .iter()
            .any(|m| m.get("role").and_then(Value::as_str) == Some("tool"))
        {
            return true;
        }
    }
    // Responses API: function_call_output input items
    if let Some(input) = raw.get("input").and_then(Value::as_array) {
        if input
            .iter()
            .any(|item| item.get("type").and_then(Value::as_str) == Some("function_call_output"))
        {
            return true;
        }
    }
    false
}

// ========== Anthropic Model Resolution ==========

pub(crate) fn resolve_anthropic_model(requested: &str) -> String {
    let normalized = super::strip_trailing_model_date_suffix(requested)
        .trim()
        .to_ascii_lowercase();
    let compact = normalized.replace('.', "-");

    if compact.starts_with("claude-4-6-opus") {
        return "claude-opus-4-6".to_string();
    }
    if compact.starts_with("claude-4-6-sonnet") {
        return "claude-sonnet-4-6".to_string();
    }
    if compact.starts_with("claude-4-5-opus") {
        return "claude-opus-4-5".to_string();
    }
    if compact.starts_with("claude-4-5-sonnet") {
        return "claude-sonnet-4-5".to_string();
    }
    if compact.starts_with("claude-4-5-haiku") {
        return "claude-haiku-4-5".to_string();
    }
    if compact.starts_with("claude-3-7-sonnet") {
        return "claude-3-7-sonnet".to_string();
    }
    if compact.starts_with("claude-3-5-sonnet") {
        return "claude-3-5-sonnet".to_string();
    }
    if compact.starts_with("claude-3-5-haiku") {
        return "claude-3-5-haiku".to_string();
    }
    if compact.starts_with("claude-3-opus") {
        return "claude-3-opus".to_string();
    }

    compact
}

// ========== Anthropic Conversions ==========

pub(crate) fn anthropic_messages_have_tool_result(messages: &[Value]) -> bool {
    messages.iter().any(|m| {
        m.get("content")
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .any(|p| p.get("type").and_then(Value::as_str) == Some("tool_result"))
            })
            .unwrap_or(false)
    })
}

pub(crate) fn anthropic_last_message_is_pure_tool_result(messages: &[Value]) -> bool {
    let Some(last) = messages.last() else {
        return false;
    };
    if last.get("role").and_then(Value::as_str) != Some("user") {
        return false;
    }
    let Some(parts) = last.get("content").and_then(Value::as_array) else {
        return false;
    };
    if parts.is_empty() {
        return false;
    }
    parts
        .iter()
        .all(|p| p.get("type").and_then(Value::as_str) == Some("tool_result"))
}

/// Convert OpenAI content (string or array with text/image_url parts) to Anthropic content blocks.
pub(crate) fn openai_content_to_anthropic(content: &Value) -> Vec<Value> {
    match content {
        Value::String(s) if !s.is_empty() => vec![json!({"type": "text", "text": s})],
        Value::Array(parts) => {
            let mut out = Vec::new();
            for part in parts {
                let part_type = part.get("type").and_then(Value::as_str).unwrap_or("text");
                if part_type == "image_url" {
                    let Some(img) = part.get("image_url") else {
                        continue;
                    };
                    let url = img.get("url").and_then(Value::as_str).unwrap_or("");
                    if url.is_empty() {
                        continue;
                    }
                    if let Some(data_rest) = url.strip_prefix("data:") {
                        if let Some(semi) = data_rest.find(';') {
                            let media_type = &data_rest[..semi];
                            if let Some(b64) = data_rest[semi + 1..].strip_prefix("base64,") {
                                out.push(json!({
                                    "type": "image",
                                    "source": {
                                        "type": "base64",
                                        "media_type": media_type,
                                        "data": b64
                                    }
                                }));
                                continue;
                            }
                        }
                    }
                    // URL image
                    out.push(json!({
                        "type": "image",
                        "source": {"type": "url", "url": url}
                    }));
                } else if part_type == "tool_result" {
                    let tool_use_id = part
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if !tool_use_id.is_empty() {
                        out.push(json!({
                            "type": "tool_result",
                            "tool_use_id": tool_use_id,
                            "content": part.get("content").cloned().unwrap_or(Value::String(String::new()))
                        }));
                    }
                } else if part_type == "tool_use" {
                    let id = part.get("id").and_then(Value::as_str).unwrap_or_default();
                    let name = part.get("name").and_then(Value::as_str).unwrap_or_default();
                    if !id.is_empty() && !name.is_empty() {
                        out.push(json!({
                            "type": "tool_use",
                            "id": id,
                            "name": name,
                            "input": part.get("input").cloned().unwrap_or_else(|| json!({}))
                        }));
                    }
                } else if let Some(text) = part.get("text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        out.push(json!({"type": "text", "text": text}));
                    }
                }
            }
            out
        }
        _ => vec![],
    }
}

/// Convert OpenAI Chat Completions request to Anthropic Messages API payload.
pub(crate) fn build_anthropic_payload(raw: &Value) -> Value {
    let model = raw
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("claude-sonnet-4-5")
        .to_string();
    let model = resolve_anthropic_model(&model);
    let max_tokens = raw
        .get("max_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(8192) as u32;
    let stream = raw.get("stream").and_then(Value::as_bool).unwrap_or(false);
    let temperature = raw.get("temperature").and_then(Value::as_f64);

    let mut system_parts: Vec<String> = Vec::new();
    let mut messages: Vec<Value> = Vec::new();

    if let Some(msgs) = raw.get("messages").and_then(Value::as_array) {
        for msg in msgs {
            let role = msg.get("role").and_then(Value::as_str).unwrap_or("user");
            let content = msg.get("content").unwrap_or(&Value::Null);

            if role == "system" || role == "developer" {
                let text = super::chat_content_to_text(content);
                if !text.trim().is_empty() {
                    system_parts.push(text);
                }
                continue;
            }

            // Tool result messages
            if role == "tool" {
                let text = super::chat_content_to_text(content);
                let tool_call_id = msg
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap_or("0");
                messages.push(json!({
                    "role": "user",
                    "content": [{"type": "tool_result", "tool_use_id": tool_call_id, "content": text}]
                }));
                continue;
            }

            // Assistant with tool calls
            if role == "assistant" {
                if let Some(tool_calls) = msg.get("tool_calls").and_then(Value::as_array) {
                    let mut content_parts = Vec::new();
                    let text = super::chat_content_to_text(content);
                    if !text.is_empty() {
                        content_parts.push(json!({"type": "text", "text": text}));
                    }
                    for tc in tool_calls {
                        let id = tc.get("id").and_then(Value::as_str).unwrap_or("");
                        let func = tc.get("function").unwrap_or(&Value::Null);
                        let name = func.get("name").and_then(Value::as_str).unwrap_or("");
                        let arguments = func
                            .get("arguments")
                            .and_then(Value::as_str)
                            .unwrap_or("{}");
                        let input: Value = serde_json::from_str(arguments).unwrap_or(json!({}));
                        content_parts.push(json!({
                            "type": "tool_use",
                            "id": id,
                            "name": name,
                            "input": input
                        }));
                    }
                    messages.push(json!({"role": "assistant", "content": content_parts}));
                    continue;
                }
            }

            let anth_role = if role == "assistant" {
                "assistant"
            } else {
                "user"
            };
            let parts = openai_content_to_anthropic(content);
            if !parts.is_empty() {
                let msg_content = if parts.len() == 1
                    && parts[0].get("type").and_then(Value::as_str) == Some("text")
                {
                    Value::String(
                        parts[0]
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                    )
                } else {
                    Value::Array(parts)
                };
                messages.push(json!({"role": anth_role, "content": msg_content}));
            }
        }
    }

    let mut payload = json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": messages,
        "stream": stream,
    });
    if !system_parts.is_empty() {
        payload["system"] = Value::String(system_parts.join("\n\n"));
    }
    if let Some(temp) = temperature {
        payload["temperature"] = json!(temp);
    }

    // Forward tools (convert OpenAI → Anthropic format)
    if let Some(tools) = raw.get("tools").and_then(Value::as_array) {
        let anthropic_tools: Vec<Value> = tools
            .iter()
            .filter_map(chat_tool_to_anthropic_tool)
            .collect();
        if !anthropic_tools.is_empty() {
            let pure_tool_result_followup = raw
                .get("messages")
                .and_then(Value::as_array)
                .map(|msgs| {
                    super::openai_last_message_is_tool_result(msgs)
                        || super::openai_last_message_has_anthropic_tool_result_block(msgs)
                })
                .unwrap_or(false);
            let disable_tools =
                matches!(raw.get("tool_choice").and_then(Value::as_str), Some("none"));
            if disable_tools || pure_tool_result_followup {
                return payload;
            }
            payload["tools"] = json!(anthropic_tools);
            let has_tool_result_in_context = raw
                .get("messages")
                .and_then(Value::as_array)
                .map(|msgs| {
                    msgs.iter().any(|m| {
                        m.get("role").and_then(Value::as_str) == Some("tool")
                            && m.get("tool_call_id").and_then(Value::as_str).is_some()
                    })
                })
                .unwrap_or(false);

            // Convert tool_choice from OpenAI → Anthropic format
            if let Some(tc) = raw.get("tool_choice") {
                let anthropic_tc = if let Some(s) = tc.as_str() {
                    match s {
                        "auto" => Some(json!({"type": "auto"})),
                        "required" => Some(if has_tool_result_in_context {
                            json!({"type": "auto"})
                        } else {
                            json!({"type": "any"})
                        }),
                        "none" => None, // omit — Anthropic has no "none" tool_choice
                        _ => None,
                    }
                } else if let Some(obj) = tc.as_object() {
                    let kind = obj.get("type").and_then(Value::as_str).unwrap_or("");
                    if kind == "function" || kind == "tool" {
                        obj.get("name")
                            .and_then(Value::as_str)
                            .or_else(|| {
                                obj.get("function")
                                    .and_then(|f| f.get("name"))
                                    .and_then(Value::as_str)
                            })
                            .map(|name| json!({"type": "tool", "name": name}))
                    } else if kind == "auto" || kind == "any" {
                        Some(json!({"type": kind}))
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some(tc_val) = anthropic_tc {
                    payload["tool_choice"] = tc_val;
                }
            }
        }
    }

    payload
}

pub(crate) fn chat_tool_to_anthropic_tool(tool: &Value) -> Option<Value> {
    let function = tool.get("function").and_then(Value::as_object);
    let name = function
        .and_then(|f| f.get("name"))
        .and_then(Value::as_str)
        .or_else(|| tool.get("name").and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())?;

    let mut t = json!({ "name": name });
    if let Some(desc) = function
        .and_then(|f| f.get("description"))
        .and_then(Value::as_str)
        .or_else(|| tool.get("description").and_then(Value::as_str))
    {
        t["description"] = json!(desc);
    }
    // Anthropic requires input_schema.
    let schema = function
        .and_then(|f| f.get("parameters"))
        .or_else(|| tool.get("input_schema"))
        .or_else(|| tool.get("parameters"))
        .filter(|p| p.is_object())
        .cloned()
        .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
    t["input_schema"] = schema;
    Some(t)
}

pub(crate) fn anthropic_stop_reason_to_openai(stop_reason: Option<&str>) -> &'static str {
    match stop_reason {
        Some("end_turn") | Some("stop_sequence") => "stop",
        Some("max_tokens") => "length",
        Some("tool_use") => "tool_calls",
        _ => "stop",
    }
}

/// Convert Anthropic Messages API response body to OpenAI Chat Completions format.
pub(crate) fn anthropic_response_to_openai(body: &Value, model: &str) -> Value {
    let id = format!(
        "chatcmpl-anth-{}",
        body.get("id").and_then(Value::as_str).unwrap_or("0")
    );
    let input_tokens = body
        .get("usage")
        .and_then(|u| u.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = body
        .get("usage")
        .and_then(|u| u.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let stop_reason = body.get("stop_reason").and_then(Value::as_str);
    let finish_reason = anthropic_stop_reason_to_openai(stop_reason);

    let mut text = String::new();
    let mut tool_calls = Vec::new();
    if let Some(content) = body.get("content").and_then(Value::as_array) {
        for block in content {
            let block_type = block.get("type").and_then(Value::as_str).unwrap_or("");
            if block_type == "text" {
                if let Some(t) = block.get("text").and_then(Value::as_str) {
                    text.push_str(t);
                }
            } else if block_type == "tool_use" {
                let tc_id = block.get("id").and_then(Value::as_str).unwrap_or("");
                let tc_name = block.get("name").and_then(Value::as_str).unwrap_or("");
                let tc_input = block.get("input").cloned().unwrap_or(json!({}));
                let tc_args = serde_json::to_string(&tc_input).unwrap_or_else(|_| "{}".to_string());
                tool_calls.push(json!({
                    "id": tc_id,
                    "type": "function",
                    "function": {"name": tc_name, "arguments": tc_args}
                }));
            }
        }
    }

    let mut message = json!({"role": "assistant", "content": Value::Null});
    if !tool_calls.is_empty() {
        message["tool_calls"] = Value::Array(tool_calls);
    } else if !text.is_empty() {
        message["content"] = Value::String(text);
    }

    json!({
        "id": id,
        "object": "chat.completion",
        "created": super::now_unix() as i64,
        "model": model,
        "system_fingerprint": Value::Null,
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish_reason,
            "logprobs": Value::Null
        }],
        "usage": {
            "prompt_tokens": input_tokens,
            "completion_tokens": output_tokens,
            "total_tokens": input_tokens + output_tokens
        }
    })
}

pub(crate) fn anthropic_messages_to_openai_chat(raw: &Value) -> Result<Value, String> {
    let model = raw
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "missing required field: model".to_string())?;
    let model = resolve_anthropic_model(model);
    let max_tokens = raw
        .get("max_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(1024);
    let temperature = raw.get("temperature").and_then(Value::as_f64);

    let mut messages: Vec<Value> = Vec::new();
    if let Some(system) = raw.get("system") {
        let sys = super::chat_content_to_text(system);
        if !sys.trim().is_empty() {
            messages.push(json!({"role":"system","content":sys}));
        }
    }

    let Some(inbound_messages) = raw.get("messages").and_then(Value::as_array) else {
        return Err("missing required field: messages".to_string());
    };
    if inbound_messages.is_empty() {
        return Err("messages must contain at least one item".to_string());
    }

    let has_tool_result_in_context = anthropic_messages_have_tool_result(inbound_messages);
    let pure_tool_result_followup = anthropic_last_message_is_pure_tool_result(inbound_messages);

    for msg in inbound_messages {
        let role = msg.get("role").and_then(Value::as_str).unwrap_or("user");
        let content = msg.get("content").unwrap_or(&Value::Null);

        if let Some(text) = content.as_str() {
            if !text.trim().is_empty() {
                messages.push(json!({"role": role, "content": text}));
            }
            continue;
        }

        if let Some(parts) = content.as_array() {
            let mut text_parts: Vec<String> = Vec::new();
            let mut tool_calls: Vec<Value> = Vec::new();
            let mut tool_results: Vec<Value> = Vec::new();
            for part in parts {
                let part_type = part.get("type").and_then(Value::as_str).unwrap_or("text");
                match part_type {
                    "text" => {
                        if let Some(text) = part.get("text").and_then(Value::as_str) {
                            if !text.trim().is_empty() {
                                text_parts.push(text.to_string());
                            }
                        }
                    }
                    "tool_use" => {
                        let id = part.get("id").and_then(Value::as_str).unwrap_or("call_0");
                        let name = part.get("name").and_then(Value::as_str).unwrap_or("tool");
                        let input = part.get("input").cloned().unwrap_or_else(|| json!({}));
                        let arguments =
                            serde_json::to_string(&input).unwrap_or_else(|_| "{}".to_string());
                        tool_calls.push(json!({
                            "id": id,
                            "type": "function",
                            "function": { "name": name, "arguments": arguments }
                        }));
                    }
                    "tool_result" => {
                        let tool_call_id = part
                            .get("tool_use_id")
                            .and_then(Value::as_str)
                            .unwrap_or("call_0");
                        let result = super::chat_content_to_text(
                            part.get("content").unwrap_or(&Value::Null),
                        );
                        tool_results.push(json!({
                            "role": "tool",
                            "tool_call_id": tool_call_id,
                            "content": result
                        }));
                    }
                    _ => {}
                }
            }

            let text = text_parts.join("\n");
            if role == "assistant" && !tool_calls.is_empty() {
                let content_val = if text.is_empty() {
                    Value::Null
                } else {
                    Value::String(text)
                };
                messages.push(json!({
                    "role": "assistant",
                    "content": content_val,
                    "tool_calls": tool_calls
                }));
            } else if !text.is_empty() {
                messages.push(json!({"role": role, "content": text}));
            }
            messages.extend(tool_results);
        }
    }

    if messages.is_empty() {
        return Err("messages did not contain usable text/tool content".to_string());
    }

    let mut out = json!({
        "model": model,
        "messages": messages,
        "max_tokens": max_tokens,
        "stream": raw.get("stream").and_then(Value::as_bool).unwrap_or(false),
    });
    if let Some(temp) = temperature {
        out["temperature"] = json!(temp);
    }

    let disable_tools = pure_tool_result_followup
        || match raw.get("tool_choice") {
            Some(Value::String(s)) => s == "none",
            Some(Value::Object(obj)) => obj.get("type").and_then(Value::as_str) == Some("none"),
            _ => false,
        };

    if !disable_tools {
        if let Some(tools) = raw.get("tools").and_then(Value::as_array) {
            let mapped: Vec<Value> = tools
                .iter()
                .filter_map(|tool| {
                    let name = tool.get("name").and_then(Value::as_str)?;
                    if name.trim().is_empty() {
                        return None;
                    }
                    let mut f = json!({ "name": name });
                    if let Some(desc) = tool.get("description").and_then(Value::as_str) {
                        f["description"] = json!(desc);
                    }
                    let schema = tool
                        .get("input_schema")
                        .or_else(|| tool.get("parameters"))
                        .filter(|v| v.is_object())
                        .cloned()
                        .unwrap_or_else(|| json!({"type":"object","properties":{}}));
                    f["parameters"] = schema;
                    Some(json!({ "type": "function", "function": f }))
                })
                .collect();
            if !mapped.is_empty() {
                out["tools"] = Value::Array(mapped);
            }
        }
    }

    if let Some(tc) = raw.get("tool_choice") {
        let mapped = if let Some(kind) = tc.as_str() {
            match kind {
                "auto" => Some(Value::String("auto".to_string())),
                "any" => Some(Value::String(
                    if pure_tool_result_followup {
                        "none"
                    } else if has_tool_result_in_context {
                        "auto"
                    } else {
                        "required"
                    }
                    .to_string(),
                )),
                "none" => Some(Value::String("none".to_string())),
                _ => None,
            }
        } else if let Some(obj) = tc.as_object() {
            let kind = obj.get("type").and_then(Value::as_str).unwrap_or("");
            match kind {
                "auto" => Some(Value::String("auto".to_string())),
                "any" => Some(Value::String(
                    if pure_tool_result_followup {
                        "none"
                    } else if has_tool_result_in_context {
                        "auto"
                    } else {
                        "required"
                    }
                    .to_string(),
                )),
                "none" => Some(Value::String("none".to_string())),
                "tool" => obj
                    .get("name")
                    .and_then(Value::as_str)
                    .map(|name| json!({"type":"function","function":{"name": name}})),
                _ => None,
            }
        } else {
            None
        };
        if let Some(val) = mapped {
            out["tool_choice"] = val;
        }
    }

    Ok(out)
}

pub(crate) fn openai_chat_to_anthropic_response(body: &Value) -> Value {
    let choice = body
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let message = choice.get("message").cloned().unwrap_or_else(|| json!({}));
    let mut content = Vec::new();

    if let Some(text) = message.get("content").and_then(Value::as_str) {
        if !text.trim().is_empty() {
            content.push(json!({"type":"text","text": text}));
        }
    }

    if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
        for tc in tool_calls {
            let id = tc.get("id").and_then(Value::as_str).unwrap_or("call_0");
            let name = tc
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("tool");
            let input = tc
                .get("function")
                .and_then(|f| f.get("arguments"))
                .and_then(Value::as_str)
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .filter(|v| v.is_object())
                .unwrap_or_else(|| json!({}));
            content.push(json!({
                "type":"tool_use",
                "id": id,
                "name": name,
                "input": input
            }));
        }
    }

    if content.is_empty() {
        content.push(json!({"type":"text","text":""}));
    }

    let stop_reason = match choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .unwrap_or("stop")
    {
        "tool_calls" => "tool_use",
        "length" => "max_tokens",
        _ => "end_turn",
    };

    let input_tokens = body
        .get("usage")
        .and_then(|u| u.get("prompt_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = body
        .get("usage")
        .and_then(|u| u.get("completion_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    json!({
        "id": format!("msg_{}", uuid::Uuid::new_v4().simple()),
        "type": "message",
        "role": "assistant",
        "model": body.get("model").and_then(Value::as_str).unwrap_or("claude"),
        "content": content,
        "stop_reason": stop_reason,
        "stop_sequence": Value::Null,
        "usage": {
            "input_tokens": input_tokens,
            "output_tokens": output_tokens
        }
    })
}

// ========== Google Conversions ==========

pub(crate) fn google_parts_to_text(parts: &[Value]) -> String {
    parts
        .iter()
        .filter_map(|p| p.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn google_tools_to_openai(tools: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    for tool in tools {
        if let Some(funcs) = tool.get("functionDeclarations").and_then(Value::as_array) {
            for f in funcs {
                let name = f
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("");
                if name.is_empty() {
                    continue;
                }
                out.push(json!({
                    "type": "function",
                    "function": {
                        "name": name,
                        "description": f.get("description").cloned().unwrap_or(Value::Null),
                        "parameters": f.get("parameters").or_else(|| f.get("input_schema")).cloned().unwrap_or_else(|| json!({"type":"object","properties":{}}))
                    }
                }));
            }
            continue;
        }
        let function = tool.get("function").unwrap_or(tool);
        let name = function
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("");
        if name.is_empty() {
            continue;
        }
        out.push(json!({
            "type": "function",
            "function": {
                "name": name,
                "description": function.get("description").cloned().unwrap_or(Value::Null),
                "parameters": function
                    .get("parameters")
                    .or_else(|| function.get("input_schema"))
                    .cloned()
                    .unwrap_or_else(|| json!({"type":"object","properties":{}}))
            }
        }));
    }
    out
}

pub(crate) fn google_generate_to_openai_chat(
    model: &str,
    req: &Value,
    stream: bool,
) -> Result<Value, String> {
    let req_body = req.get("request").unwrap_or(req);
    let mut messages: Vec<Value> = Vec::new();
    if let Some(system_parts) = req_body
        .get("systemInstruction")
        .or_else(|| req_body.get("system_instruction"))
        .and_then(|v| v.get("parts"))
        .and_then(Value::as_array)
    {
        let text = google_parts_to_text(system_parts);
        if !text.trim().is_empty() {
            messages.push(json!({"role":"system","content":text}));
        }
    }
    let Some(contents) = req_body.get("contents").and_then(Value::as_array) else {
        return Err("missing required field: contents".to_string());
    };
    for item in contents {
        let role = match item.get("role").and_then(Value::as_str).unwrap_or("user") {
            "model" => "assistant",
            other => other,
        };
        let parts = item
            .get("parts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let text = google_parts_to_text(&parts);
        messages.push(json!({"role": role, "content": text}));
    }
    let mut out = json!({
        "model": super::resolve_google_model(model),
        "messages": messages,
        "stream": stream
    });
    if let Some(gtools) = req_body.get("tools").and_then(Value::as_array) {
        let tools = google_tools_to_openai(gtools);
        if !tools.is_empty() {
            out["tools"] = Value::Array(tools);
        }
    }
    if let Some(mode) = req_body
        .get("toolConfig")
        .and_then(|v| v.get("functionCallingConfig"))
        .and_then(|v| v.get("mode"))
        .and_then(Value::as_str)
    {
        let mapped = match mode.to_ascii_uppercase().as_str() {
            "NONE" => Some(Value::String("none".to_string())),
            "ANY" => Some(Value::String("required".to_string())),
            "AUTO" => Some(Value::String("auto".to_string())),
            _ => None,
        };
        if let Some(tool_choice) = mapped {
            out["tool_choice"] = tool_choice;
        }
    }
    Ok(out)
}

pub(crate) fn openai_chat_to_google_response(body: &Value) -> Value {
    let text = body
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|msg| msg.get("content"))
        .map(super::chat_content_to_text)
        .unwrap_or_default();
    json!({
        "candidates": [{
            "content": {
                "role": "model",
                "parts": [{"text": text}]
            },
            "finishReason": "STOP",
            "index": 0
        }],
        "usageMetadata": {
            "promptTokenCount": body.get("usage").and_then(|u| u.get("prompt_tokens")).and_then(Value::as_u64).unwrap_or(0),
            "candidatesTokenCount": body.get("usage").and_then(|u| u.get("completion_tokens")).and_then(Value::as_u64).unwrap_or(0),
            "totalTokenCount": body.get("usage").and_then(|u| u.get("total_tokens")).and_then(Value::as_u64).unwrap_or(0)
        }
    })
}

// ========== Compact Params ==========

/// Strip verbose metadata from a JSON Schema parameters object, keeping only what
/// the model needs to call the tool correctly: property types, enums, array item
/// types, and the required list.  Descriptions on individual properties are the
/// main source of bloat (Cursor ships 18 tools with long prose per param).
pub(crate) fn compact_params(params: &Value) -> Value {
    let props = params.get("properties").and_then(Value::as_object);
    let required = params
        .get("required")
        .cloned()
        .unwrap_or(Value::Array(vec![]));

    let compact_props: serde_json::Map<String, Value> = props
        .map(|map| {
            map.iter()
                .map(|(pname, pinfo)| {
                    let mut p = serde_json::Map::new();
                    // Keep type
                    if let Some(t) = pinfo.get("type") {
                        p.insert("type".to_string(), t.clone());
                    }
                    // Keep enum values
                    if let Some(e) = pinfo.get("enum") {
                        p.insert("enum".to_string(), e.clone());
                    }
                    // Keep items.type for arrays
                    if let Some(items) = pinfo.get("items").and_then(Value::as_object) {
                        let item_type = items
                            .get("type")
                            .cloned()
                            .unwrap_or(Value::String("object".to_string()));
                        p.insert("items".to_string(), json!({"type": item_type}));
                    }
                    (pname.clone(), Value::Object(p))
                })
                .collect()
        })
        .unwrap_or_default();

    json!({
        "type": "object",
        "properties": compact_props,
        "required": required,
    })
}

// ========== Codex / Responses API Conversions ==========

pub(crate) fn map_chat_tools_to_responses(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .filter_map(|tool| {
            let kind = tool.get("type").and_then(Value::as_str).unwrap_or_default();
            if kind != "function" {
                return None;
            }
            // Chat Completions format (nested: {type:"function", function:{name, description?, parameters}})
            let function = tool.get("function")?;
            let name = function.get("name").and_then(Value::as_str)?;
            let params = function
                .get("parameters")
                .map(compact_params)
                .unwrap_or_else(|| json!({"type":"object","properties":{},"required":[]}));
            let mut out = json!({
                "type": "function",
                "name": name,
                "parameters": params,
            });
            if let Some(desc) = function.get("description").and_then(Value::as_str) {
                if !desc.trim().is_empty() {
                    out["description"] = Value::String(desc.to_string());
                }
            }
            Some(out)
        })
        .collect()
}

pub(crate) fn map_chat_tool_choice_to_responses(
    tool_choice: Option<&Value>,
    has_tools: bool,
    has_tool_results: bool,
) -> Value {
    match tool_choice {
        Some(Value::String(s)) if s == "required" => Value::String("required".to_string()),
        Some(Value::String(s)) if s == "none" => Value::String("none".to_string()),
        Some(Value::String(s)) if s == "auto" => Value::String("auto".to_string()),
        Some(Value::Object(obj)) => {
            let function_name = obj
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str);
            if let Some(name) = function_name {
                return json!({"type":"function","name":name});
            }
            Value::String("auto".to_string())
        }
        // null or absent:
        // - first turn (no tool results in context yet): "required" so the model actually
        //   calls a tool instead of outputting planning text.
        // - subsequent turns (tool results already in context): "auto" so the model can
        //   choose to stop calling tools and give a final answer.
        _ => {
            if has_tools && !has_tool_results {
                Value::String("required".to_string())
            } else {
                Value::String("auto".to_string())
            }
        }
    }
}

/// Convert OpenAI content to Codex Responses-format parts (supports vision).
pub(crate) fn chat_content_to_codex_parts(content: &Value, is_assistant: bool) -> Vec<Value> {
    let text_type = if is_assistant {
        "output_text"
    } else {
        "input_text"
    };
    match content {
        Value::String(s) if !s.is_empty() => {
            vec![json!({"type": text_type, "text": s})]
        }
        Value::Array(parts) => {
            let mut out = Vec::new();
            for part in parts {
                let part_type = part.get("type").and_then(Value::as_str).unwrap_or("text");
                if part_type == "image_url" {
                    if let Some(img) = part.get("image_url") {
                        let url = img.get("url").and_then(Value::as_str).unwrap_or("");
                        if !url.is_empty() {
                            out.push(json!({"type": "input_image", "image_url": url}));
                        }
                    }
                } else if let Some(text) = part.get("text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        out.push(json!({"type": text_type, "text": text}));
                    }
                }
            }
            out
        }
        _ => vec![],
    }
}

pub(crate) fn build_codex_payload(raw: &Value, workspace_id: &str) -> Value {
    let requested_model = raw
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("gpt-5.2");
    let model = super::resolve_codex_model(requested_model).to_string();
    let inbound_is_responses = raw.get("input").is_some() && raw.get("messages").is_none();
    let mut call_id_remap: HashMap<String, String> = HashMap::new();
    // The Codex `/codex/responses` endpoint currently requires an `instructions` string.
    // We derive it from any inbound system/developer messages; otherwise we provide a
    // stable minimal default so Chat Completions clients work out of the box.
    let mut instructions_parts: Vec<String> = Vec::new();
    let explicit_instructions = raw
        .get("instructions")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let mut input_items = Vec::<Value>::new();
    if let Some(messages) = raw.get("messages").and_then(Value::as_array) {
        for message in messages {
            let source_role = message
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("user");
            let raw_content = message.get("content").unwrap_or(&Value::Null);
            if source_role == "system" || source_role == "developer" {
                let content = super::chat_content_to_text(raw_content);
                if !content.trim().is_empty() {
                    instructions_parts.push(content);
                }
                continue;
            }
            if source_role == "assistant" {
                if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
                    for tool_call in tool_calls {
                        let id = tool_call
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let call_id = normalize_codex_call_id(id, &mut call_id_remap);
                        let function = tool_call.get("function").unwrap_or(&Value::Null);
                        let name = function
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let arguments = function
                            .get("arguments")
                            .and_then(Value::as_str)
                            .unwrap_or("{}");
                        if call_id.is_empty() || name.is_empty() {
                            continue;
                        }
                        input_items.push(json!({
                            "type": "function_call",
                            "call_id": call_id,
                            "name": name,
                            "arguments": arguments,
                        }));
                    }
                }
            } else if source_role == "tool" {
                let call_id = message
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !call_id.is_empty() {
                    let call_id = normalize_codex_call_id(call_id, &mut call_id_remap);
                    let content = super::chat_content_to_text(raw_content);
                    input_items.push(json!({
                        "type": "function_call_output",
                        "call_id": call_id,
                        "output": content,
                    }));
                    continue;
                }
            }
            let is_assistant = source_role == "assistant";
            let parts = chat_content_to_codex_parts(raw_content, is_assistant);
            if !parts.is_empty() {
                let normalized_role = normalize_role(source_role);
                input_items.push(json!({
                    "role": normalized_role,
                    "content": parts
                }));
            }
        }
    } else if let Some(input) = raw.get("input") {
        // Responses-format inbound payload (Cursor Agent and other Responses-native clients).
        if let Some(arr) = input.as_array() {
            for item in arr {
                // Preserve native Responses items (function_call, function_call_output, etc.).
                if item.get("type").is_some() {
                    let mut cloned = item.clone();
                    // Codex enforces `input[i].call_id` max length (currently 64) across *all*
                    // tool-call item types (including Cursor's `custom_tool_call`).
                    if let Some(call_id) = cloned.get("call_id").and_then(Value::as_str) {
                        let mapped =
                            normalize_codex_call_id(call_id, &mut call_id_remap).into_owned();
                        cloned["call_id"] = Value::String(mapped);
                    }
                    input_items.push(cloned);
                    continue;
                }

                let role = item.get("role").and_then(Value::as_str).unwrap_or("user");
                if role == "system" || role == "developer" {
                    let content =
                        super::chat_content_to_text(item.get("content").unwrap_or(&Value::Null));
                    if !content.trim().is_empty() {
                        instructions_parts.push(content);
                    }
                    continue;
                }

                if role == "assistant" {
                    if let Some(tool_calls) = item.get("tool_calls").and_then(Value::as_array) {
                        for tool_call in tool_calls {
                            let id = tool_call
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            let call_id = normalize_codex_call_id(id, &mut call_id_remap);
                            let function = tool_call.get("function").unwrap_or(&Value::Null);
                            let name = function
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            let arguments = function
                                .get("arguments")
                                .and_then(Value::as_str)
                                .unwrap_or("{}");
                            if call_id.is_empty() || name.is_empty() {
                                continue;
                            }
                            input_items.push(json!({
                                "type": "function_call",
                                "call_id": call_id,
                                "name": name,
                                "arguments": arguments,
                            }));
                        }
                        continue;
                    }
                } else if role == "tool" {
                    let call_id = item
                        .get("tool_call_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if !call_id.is_empty() {
                        let call_id = normalize_codex_call_id(call_id, &mut call_id_remap);
                        let content = super::chat_content_to_text(
                            item.get("content").unwrap_or(&Value::Null),
                        );
                        input_items.push(json!({
                            "type": "function_call_output",
                            "call_id": call_id,
                            "output": content,
                        }));
                        continue;
                    }
                }

                let content =
                    super::chat_content_to_text(item.get("content").unwrap_or(&Value::Null));
                if !content.is_empty() {
                    let normalized_role = normalize_role(role);
                    let content_type = if normalized_role == "assistant" {
                        "output_text"
                    } else {
                        "input_text"
                    };
                    input_items.push(json!({
                        "role": normalized_role,
                        "content": [{"type": content_type, "text": content}]
                    }));
                }
            }
        } else if let Some(text) = input.as_str() {
            if !text.trim().is_empty() {
                input_items.push(json!({
                    "role": "user",
                    "content": [{"type":"input_text","text": text}]
                }));
            }
        }
    }
    if input_items.is_empty() {
        // Should be unreachable: strict validation requires at least one message with usable content/tool calls.
        input_items
            .push(json!({"role":"user","content":[{"type":"input_text","text":"Continue."}]}));
    }

    let instructions = explicit_instructions.unwrap_or_else(|| {
        if !instructions_parts.is_empty() {
            instructions_parts.join("\n\n")
        } else {
            std::env::var("LLMHOTSPOT_CODEX_DEFAULT_INSTRUCTIONS")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "You are a helpful assistant.".to_string())
        }
    });

    let mut payload = json!({
        "model": model,
        "store": false,
        "stream": true,  // Codex endpoint always requires streaming
        "input": input_items,
        "instructions": instructions,
        "text": {"verbosity":"medium"},
        "include": ["reasoning.encrypted_content"],
        "prompt_cache_key": workspace_id,
        // Passthrough reasoning effort if client sends it; default to medium.
        // Accepts Chat Completions `reasoning_effort` or Responses `reasoning.effort`.
        "reasoning": {
            "effort": super::resolve_reasoning_effort(raw)
        },
    });

    let has_previous_response_id = raw
        .get("previous_response_id")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty());

    // Guardrail: Codex rejects function_call_output items whose call_id cannot be resolved.
    // Keep unpaired outputs only when client supplies previous_response_id (server-side context).
    if !has_previous_response_id {
        let known_call_ids: HashSet<String> = input_items
            .iter()
            .filter(|item| item.get("type").and_then(Value::as_str) == Some("function_call"))
            .filter_map(|item| item.get("call_id").and_then(Value::as_str))
            .map(str::to_string)
            .collect();
        input_items.retain(|item| {
            if item.get("type").and_then(Value::as_str) != Some("function_call_output") {
                return true;
            }
            item.get("call_id")
                .and_then(Value::as_str)
                .is_some_and(|call_id| known_call_ids.contains(call_id))
        });
        if input_items.is_empty() {
            input_items
                .push(json!({"role":"user","content":[{"type":"input_text","text":"Continue."}]}));
        }
    }
    payload["input"] = Value::Array(input_items);

    // NOTE: The Codex endpoint does NOT support `max_output_tokens`.
    // Omit token-limit fields entirely and let upstream defaults apply.
    let tools = raw
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    if inbound_is_responses {
        if let Some(previous_response_id) = raw
            .get("previous_response_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            payload["previous_response_id"] = Value::String(previous_response_id.to_string());
        }
        // Responses-native clients may send `tools` in Responses format, including `type:"custom"`.
        // Do not remap these; pass through to Codex as-is.
        if !tools.is_empty() {
            payload["tools"] = Value::Array(tools);
            // Default to "required" on the first turn; "auto" once tool results are in context.
            let tc = raw.get("tool_choice");
            let is_null_or_absent = matches!(tc, None | Some(Value::Null));
            payload["tool_choice"] = if is_null_or_absent {
                if request_has_tool_results(raw) {
                    Value::String("auto".to_string())
                } else {
                    Value::String("required".to_string())
                }
            } else {
                tc.unwrap().clone()
            };
        } else if let Some(tool_choice) = raw.get("tool_choice").filter(|v| !v.is_null()) {
            payload["tool_choice"] = tool_choice.clone();
        }
        if let Some(parallel) = raw.get("parallel_tool_calls").and_then(Value::as_bool) {
            payload["parallel_tool_calls"] = Value::Bool(parallel);
        }
    } else {
        // Chat Completions tools must be mapped into Responses tool definitions.
        let mapped_tools = map_chat_tools_to_responses(&tools);
        if !mapped_tools.is_empty() {
            payload["tools"] = Value::Array(mapped_tools);
            payload["tool_choice"] = map_chat_tool_choice_to_responses(
                raw.get("tool_choice"),
                true,
                request_has_tool_results(raw),
            );
            payload["parallel_tool_calls"] = Value::Bool(true);
        }
    }
    payload
}

pub(crate) fn build_codex_request_headers(
    access_token: &str,
    workspace_id: &str,
    jwt_claim_path: &str,
) -> HashMap<String, String> {
    let chatgpt_account_id = llm_auth::openai_chatgpt_account_id(access_token, jwt_claim_path);
    let mut hdrs = HashMap::new();
    hdrs.insert(
        "openai-beta".to_string(),
        "responses=experimental".to_string(),
    );
    hdrs.insert("originator".to_string(), "pi".to_string());
    hdrs.insert("user-agent".to_string(), "llmhotspot/0.1".to_string());
    hdrs.insert("accept".to_string(), "application/json".to_string());
    hdrs.insert("session_id".to_string(), workspace_id.to_string());
    if let Some(account_id) = chatgpt_account_id {
        hdrs.insert("chatgpt-account-id".to_string(), account_id);
    }
    hdrs
}

// ========== OpenAI Normalization ==========

pub(crate) fn normalize_openai_tool_choice_inplace(req: &mut Value) {
    fn normalize_tool_choice_value(tc: &Value) -> Option<Value> {
        match tc {
            Value::Null => None,
            Value::String(s) => {
                let kind = s.trim().to_ascii_lowercase();
                match kind.as_str() {
                    "none" | "auto" | "required" => Some(Value::String(kind)),
                    // Anthropic/legacy synonym used by some clients.
                    "any" => Some(Value::String("required".to_string())),
                    _ => {
                        // Some clients accidentally send object-form tool_choice as a JSON string.
                        serde_json::from_str::<Value>(s)
                            .ok()
                            .and_then(|parsed| normalize_tool_choice_value(&parsed))
                    }
                }
            }
            Value::Object(obj) => {
                let kind = obj
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();
                match kind.as_str() {
                    "none" | "auto" | "required" => Some(Value::String(kind)),
                    "any" => Some(Value::String("required".to_string())),
                    "function" | "tool" => obj
                        .get("name")
                        .and_then(Value::as_str)
                        .or_else(|| {
                            obj.get("function")
                                .and_then(|f| f.get("name"))
                                .and_then(Value::as_str)
                        })
                        .map(|name| json!({"type":"function","function":{"name": name}})),
                    // Handle direct function selector objects missing `type`.
                    "" => obj
                        .get("name")
                        .and_then(Value::as_str)
                        .or_else(|| {
                            obj.get("function")
                                .and_then(|f| f.get("name"))
                                .and_then(Value::as_str)
                        })
                        .map(|name| json!({"type":"function","function":{"name": name}})),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    let had_tool_choice = req.get("tool_choice").is_some();
    let normalized = req.get("tool_choice").and_then(normalize_tool_choice_value);

    if let Some(tc) = normalized {
        req["tool_choice"] = tc;
    } else if had_tool_choice {
        if let Some(obj) = req.as_object_mut() {
            // Drop invalid shapes so upstream doesn't reject the entire request.
            obj.remove("tool_choice");
        }
    }
}

pub(crate) fn normalize_tool_name_for_declaration(raw: &str) -> String {
    let mut name = raw.trim();
    if let Some(stripped) = name.rsplit('.').next() {
        name = stripped;
    }
    let mut out = String::with_capacity(name.len() + 8);
    let mut prev_is_lower_or_digit = false;
    let mut prev_is_underscore = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            let is_upper = ch.is_ascii_uppercase();
            if is_upper && prev_is_lower_or_digit && !prev_is_underscore {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
            prev_is_lower_or_digit = ch.is_ascii_lowercase() || ch.is_ascii_digit();
            prev_is_underscore = false;
        } else if !prev_is_underscore {
            out.push('_');
            prev_is_lower_or_digit = false;
            prev_is_underscore = true;
        }
    }
    let out = out.trim_matches('_').to_string();
    if out.is_empty() {
        "tool".to_string()
    } else {
        out
    }
}

pub(crate) fn looks_like_json_schema_object(obj: &serde_json::Map<String, Value>) -> bool {
    obj.contains_key("type")
        || obj.contains_key("properties")
        || obj.contains_key("required")
        || obj.contains_key("oneOf")
        || obj.contains_key("anyOf")
        || obj.contains_key("allOf")
        || obj.contains_key("$schema")
}

pub(crate) fn infer_json_schema_from_example(value: &Value, depth: usize) -> Value {
    if depth > 4 {
        return json!({"type":"string"});
    }
    match value {
        Value::Null => json!({"type":"string"}),
        Value::Bool(_) => json!({"type":"boolean"}),
        Value::Number(n) => {
            if n.is_f64() {
                json!({"type":"number"})
            } else {
                json!({"type":"integer"})
            }
        }
        Value::String(_) => json!({"type":"string"}),
        Value::Array(arr) => {
            let item_schema = arr
                .first()
                .map(|v| infer_json_schema_from_example(v, depth + 1))
                .unwrap_or_else(|| json!({"type":"string"}));
            json!({"type":"array","items": item_schema})
        }
        Value::Object(map) => {
            let mut props = serde_json::Map::new();
            let mut required: Vec<Value> = Vec::new();
            for (k, v) in map {
                props.insert(k.clone(), infer_json_schema_from_example(v, depth + 1));
                required.push(Value::String(k.clone()));
            }
            json!({
                "type":"object",
                "properties": Value::Object(props),
                "required": Value::Array(required)
            })
        }
    }
}

pub(crate) fn schema_from_tool_field(raw: Option<&Value>) -> Value {
    let Some(v) = raw else {
        return json!({"type":"object","properties":{}});
    };
    if let Value::Object(obj) = v {
        if looks_like_json_schema_object(obj) {
            return v.clone();
        }
    }
    infer_json_schema_from_example(v, 0)
}

pub(crate) fn openai_function_tool(name: &str, schema: Value, description: Option<&str>) -> Value {
    let mut function = json!({
        "name": normalize_tool_name_for_declaration(name),
        "parameters": schema,
    });
    if let Some(desc) = description.map(str::trim).filter(|s| !s.is_empty()) {
        function["description"] = Value::String(desc.to_string());
    }
    json!({
        "type":"function",
        "function": function
    })
}

pub(crate) fn salvage_tool_definition(tool: &Value, out: &mut Vec<Value>) -> bool {
    match tool {
        Value::String(s) => {
            if let Ok(parsed) = serde_json::from_str::<Value>(s) {
                return salvage_tool_definition(&parsed, out);
            }
            false
        }
        Value::Object(obj) => {
            if let Some(arr) = obj.get("tool_uses").and_then(Value::as_array) {
                let mut any = false;
                for item in arr {
                    if salvage_tool_definition(item, out) {
                        any = true;
                    }
                }
                return any;
            }

            if let Some(recipient) = obj.get("recipient_name").and_then(Value::as_str) {
                let params_schema = schema_from_tool_field(obj.get("parameters"));
                out.push(openai_function_tool(recipient, params_schema, None));
                return true;
            }

            if let Some(fns) = obj.get("functionDeclarations").and_then(Value::as_array) {
                let mut any = false;
                for f in fns {
                    let Some(name) = f.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    let schema = schema_from_tool_field(
                        f.get("parameters").or_else(|| f.get("input_schema")),
                    );
                    let desc = f.get("description").and_then(Value::as_str);
                    out.push(openai_function_tool(name, schema, desc));
                    any = true;
                }
                return any;
            }

            if let Some(function_obj) = obj.get("function").and_then(Value::as_object) {
                if let Some(name) = function_obj.get("name").and_then(Value::as_str) {
                    let schema = schema_from_tool_field(function_obj.get("parameters"));
                    let desc = function_obj.get("description").and_then(Value::as_str);
                    out.push(openai_function_tool(name, schema, desc));
                    return true;
                }
            }

            if let Some(name) = obj
                .get("name")
                .and_then(Value::as_str)
                .or_else(|| obj.get("tool_name").and_then(Value::as_str))
            {
                let schema = schema_from_tool_field(
                    obj.get("input_schema").or_else(|| obj.get("parameters")),
                );
                let desc = obj.get("description").and_then(Value::as_str);
                out.push(openai_function_tool(name, schema, desc));
                return true;
            }

            false
        }
        _ => false,
    }
}

pub(crate) fn normalize_openai_tools_inplace(req: &mut Value) -> ToolRepairResult {
    let Some(tools_value) = req.get("tools").cloned() else {
        return ToolRepairResult::default();
    };
    let tools = match tools_value.as_array() {
        Some(v) => v,
        None => {
            return ToolRepairResult {
                repaired: false,
                unrecoverable: true,
                reason: Some("tools is not an array".to_string()),
            };
        }
    };
    // Preserve native Google tool declarations so strict validation can surface
    // precise functionDeclarations errors instead of generic repair fallbacks.
    if tools
        .iter()
        .any(|t| t.get("functionDeclarations").is_some())
    {
        return ToolRepairResult::default();
    }

    let mut repaired = false;
    let mut unrecoverable = false;
    let mut normalized: Vec<Value> = Vec::new();
    for tool in tools {
        let before_len = normalized.len();
        if salvage_tool_definition(tool, &mut normalized) {
            if normalized.len() != before_len || !tool.get("type").is_some_and(|t| t == "function")
            {
                repaired = true;
            }
        } else {
            unrecoverable = true;
        }
    }

    if normalized.is_empty() && !tools.is_empty() {
        unrecoverable = true;
    }
    req["tools"] = Value::Array(normalized);
    ToolRepairResult {
        repaired,
        unrecoverable,
        reason: if unrecoverable {
            Some("one or more tools entries could not be normalized".to_string())
        } else {
            None
        },
    }
}

pub(crate) fn normalize_openai_messages_inplace(req: &mut Value) -> MessageRepairResult {
    let mut out = MessageRepairResult::default();
    let Some(messages) = req.get_mut("messages").and_then(Value::as_array_mut) else {
        return out;
    };
    let mut normalized: Vec<Value> = Vec::with_capacity(messages.len());
    for msg in messages.iter() {
        let Some(obj) = msg.as_object() else {
            out.repaired = true;
            out.dropped_count = out.dropped_count.saturating_add(1);
            continue;
        };
        let mut item = Value::Object(obj.clone());
        let mut role = item
            .get("role")
            .and_then(Value::as_str)
            .and_then(normalize_message_role);
        if role.is_none()
            && (item.get("type").is_some_and(looks_like_tool_result_item)
                || content_has_tool_result_block(item.get("content").unwrap_or(&Value::Null)))
        {
            role = Some("tool");
            out.repaired = true;
        }
        if role.is_none() {
            role = if item
                .get("tool_calls")
                .and_then(Value::as_array)
                .is_some_and(|arr| !arr.is_empty())
            {
                Some("assistant")
            } else if item.get("tool_call_id").and_then(Value::as_str).is_some() {
                Some("tool")
            } else if !super::chat_content_to_text(item.get("content").unwrap_or(&Value::Null))
                .is_empty()
            {
                Some("user")
            } else {
                None
            };
            if role.is_some() {
                out.repaired = true;
            }
        }
        if let Some(r) = role {
            item["role"] = Value::String(r.to_string());
            if r == "tool" {
                if item.get("tool_call_id").and_then(Value::as_str).is_none() {
                    if let Some(call_id) = item
                        .get("call_id")
                        .and_then(Value::as_str)
                        .or_else(|| item.get("id").and_then(Value::as_str))
                    {
                        item["tool_call_id"] = Value::String(call_id.to_string());
                        out.repaired = true;
                    }
                }
                if super::chat_content_to_text(item.get("content").unwrap_or(&Value::Null))
                    .is_empty()
                {
                    if let Some(s) = item
                        .get("output")
                        .or_else(|| item.get("result"))
                        .or_else(|| item.get("text"))
                        .and_then(Value::as_str)
                    {
                        item["content"] = Value::String(s.to_string());
                        out.repaired = true;
                    }
                }
            }
            normalized.push(item);
        } else {
            out.repaired = true;
            out.dropped_count = out.dropped_count.saturating_add(1);
        }
    }
    let max_keep = max_inbound_messages();
    if normalized.len() > max_keep {
        let mut head: Vec<Value> = normalized
            .iter()
            .take(8)
            .filter(|m| {
                m.get("role")
                    .and_then(Value::as_str)
                    .is_some_and(|r| r == "system" || r == "developer")
            })
            .cloned()
            .collect();
        let tail_keep = max_keep.saturating_sub(head.len());
        let start = normalized.len().saturating_sub(tail_keep);
        let mut tail = normalized[start..].to_vec();
        head.append(&mut tail);
        out.pruned_count = normalized.len().saturating_sub(head.len());
        out.repaired = true;
        normalized = head;
    }
    messages.clear();
    messages.extend(normalized);
    out
}
