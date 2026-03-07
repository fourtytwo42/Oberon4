use axum::{
    body::{Body, Bytes},
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use llm_gateway::{GatewayError, GatewayErrorDetail};
use llm_storage::SqliteStore;
use serde_json::{json, Value};
use std::collections::HashMap;

use super::{AppState, RoutedResponse};

// Items from sibling modules (pub(crate) but not re-exported via `pub use` in lib.rs)
use crate::message_conversion::{google_generate_to_openai_chat, openai_chat_to_google_response};
use crate::model_resolution::{
    chat_content_to_text, resolve_google_model, should_inject_google_antigravity_instruction,
};
use crate::oauth_handlers::{
    get_google_access_token, google_endpoint_chain, google_headers, resolve_google_project_context,
    GOOGLE_ANTIGRAVITY_BASE_SYSTEM_INSTRUCTION,
};

pub(crate) async fn route_google_request(
    state: &AppState,
    store: &SqliteStore,
    req: &Value,
) -> Result<(RoutedResponse, Option<String>), GatewayErrorDetail> {
    let accounts = store
        .list_oauth_accounts()
        .await
        .map_err(|_| GatewayErrorDetail::new(GatewayError::Auth))?;
    let google_accounts: Vec<_> = accounts
        .into_iter()
        .filter(|a| a.enabled && super::is_google_account(a))
        .collect();
    if google_accounts.is_empty() {
        return Err(GatewayErrorDetail::new(GatewayError::Auth));
    }
    let model = req
        .get("model")
        .and_then(Value::as_str)
        .map(resolve_google_model)
        .unwrap_or_else(|| "gemini-2.5-flash".to_string());

    let mut contents: Vec<Value> = Vec::new();
    let mut function_meta_by_call_id: HashMap<String, (String, Option<String>)> = HashMap::new();
    let mut system_text = String::new();
    if let Some(messages) = req.get("messages").and_then(Value::as_array) {
        for msg in messages {
            let role = msg.get("role").and_then(Value::as_str).unwrap_or("user");
            if role == "system" || role == "developer" {
                let t = chat_content_to_text(msg.get("content").unwrap_or(&Value::Null));
                if !t.trim().is_empty() {
                    if !system_text.is_empty() {
                        system_text.push_str("\n\n");
                    }
                    system_text.push_str(&t);
                }
                continue;
            }

            if role == "tool" {
                let call_id = msg
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let (tool_name, thought_signature) = function_meta_by_call_id
                    .get(&call_id)
                    .cloned()
                    .or_else(|| {
                        state
                            .google_tool_signatures
                            .get(&call_id)
                            .map(|sig| ("tool".to_string(), Some(sig)))
                    })
                    .unwrap_or_else(|| ("tool".to_string(), None));
                let raw = chat_content_to_text(msg.get("content").unwrap_or(&Value::Null));
                let parsed: Value =
                    serde_json::from_str(&raw).unwrap_or_else(|_| json!({ "content": raw }));
                let mut part = json!({
                    "role": "user",
                    "parts": [{
                        "functionResponse": {
                            "name": tool_name,
                            "response": parsed
                        }
                    }]
                });
                if let Some(sig) = thought_signature {
                    if let Some(p0) = part
                        .get_mut("parts")
                        .and_then(Value::as_array_mut)
                        .and_then(|arr| arr.get_mut(0))
                    {
                        p0["thought_signature"] = Value::String(sig);
                    }
                }
                contents.push(part);
                continue;
            }

            if role == "assistant" {
                let mut parts: Vec<Value> = Vec::new();
                let text = chat_content_to_text(msg.get("content").unwrap_or(&Value::Null));
                if !text.trim().is_empty() {
                    parts.push(json!({"text": text}));
                }
                if let Some(tool_calls) = msg.get("tool_calls").and_then(Value::as_array) {
                    for tc in tool_calls {
                        let call_id = tc
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        let name = tc
                            .get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(Value::as_str)
                            .unwrap_or("tool")
                            .to_string();
                        let mut thought_signature = tc
                            .get("function")
                            .and_then(|f| {
                                f.get("thought_signature")
                                    .or_else(|| f.get("thoughtSignature"))
                            })
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        let args_raw = tc
                            .get("function")
                            .and_then(|f| f.get("arguments"))
                            .and_then(Value::as_str)
                            .unwrap_or("{}");
                        let args: Value =
                            serde_json::from_str(args_raw).unwrap_or_else(|_| json!({}));
                        if thought_signature.is_none() {
                            thought_signature = state
                                .google_tool_signatures
                                .get_for_call_or_fingerprint(&call_id, &name, &args);
                        }
                        if !call_id.is_empty() {
                            function_meta_by_call_id
                                .insert(call_id, (name.clone(), thought_signature.clone()));
                        }
                        let fc = json!({"name": name, "args": args});
                        let mut part = json!({"functionCall": fc});
                        if let Some(sig) = thought_signature {
                            part["thought_signature"] = Value::String(sig);
                        }
                        parts.push(part);
                    }
                }
                if !parts.is_empty() {
                    contents.push(json!({"role":"model","parts": parts}));
                }
                continue;
            }

            let text = chat_content_to_text(msg.get("content").unwrap_or(&Value::Null));
            if !text.trim().is_empty() {
                contents.push(json!({"role":"user","parts":[{"text": text}]}));
            }
        }
    } else {
        let prompt = req
            .get("input")
            .map(chat_content_to_text)
            .unwrap_or_default();
        if !prompt.trim().is_empty() {
            contents.push(json!({"role":"user","parts":[{"text": prompt}]}));
        }
    }
    if contents.is_empty() {
        contents.push(json!({"role":"user","parts":[{"text": ""}]}));
    }

    let mut google_tools: Vec<Value> = Vec::new();
    if let Some(tools) = req.get("tools").and_then(Value::as_array) {
        for tool in tools {
            let function = tool.get("function").unwrap_or(tool);
            let Some(name) = function.get("name").and_then(Value::as_str).map(str::trim) else {
                continue;
            };
            if name.is_empty() {
                continue;
            }
            google_tools.push(json!({
                "name": name,
                "description": function.get("description").cloned().unwrap_or(Value::Null),
                "parameters": function.get("parameters").cloned().unwrap_or_else(|| json!({"type":"object","properties":{}}))
            }));
        }
    }
    let mut function_calling_config = json!({"mode":"AUTO"});
    if let Some(tc) = req.get("tool_choice") {
        if let Some(s) = tc.as_str() {
            function_calling_config["mode"] = Value::String(
                match s {
                    "none" => "NONE",
                    "required" => "ANY",
                    _ => "AUTO",
                }
                .to_string(),
            );
        } else if let Some(obj) = tc.as_object() {
            let t = obj.get("type").and_then(Value::as_str).unwrap_or("");
            if t == "none" {
                function_calling_config["mode"] = Value::String("NONE".to_string());
            } else if t == "required" || t == "any" {
                function_calling_config["mode"] = Value::String("ANY".to_string());
            } else if t == "function" || t == "tool" {
                function_calling_config["mode"] = Value::String("ANY".to_string());
                if let Some(fname) = obj.get("name").and_then(Value::as_str).or_else(|| {
                    obj.get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(Value::as_str)
                }) {
                    function_calling_config["allowedFunctionNames"] =
                        Value::Array(vec![json!(fname)]);
                }
            }
        }
    }

    let http = state.http_default().clone();
    let mut last_error: Option<(GatewayErrorDetail, String)> = None;
    for account in &google_accounts {
        let token = match get_google_access_token(Some(state), store, &account.account_id).await {
            Ok(t) => t,
            Err(_) => continue,
        };
        let project_ctx = resolve_google_project_context(store, &token, &account.account_id).await;
        let project_id = project_ctx.effective_project_id;
        let mut request_body = json!({
            "sessionId": uuid::Uuid::new_v4().to_string(),
            "contents": contents,
            "generationConfig": {"thinkingConfig":{"includeThoughts":false}}
        });
        if should_inject_google_antigravity_instruction(&model) {
            if system_text.trim().is_empty() {
                system_text = GOOGLE_ANTIGRAVITY_BASE_SYSTEM_INSTRUCTION.to_string();
            } else {
                system_text = format!(
                    "{}\n\n{}",
                    GOOGLE_ANTIGRAVITY_BASE_SYSTEM_INSTRUCTION, system_text
                );
            }
        }
        if !system_text.trim().is_empty() {
            request_body["systemInstruction"] = json!({"parts":[{"text": system_text}]});
        }
        if !google_tools.is_empty() {
            request_body["tools"] = json!([{"functionDeclarations": google_tools}]);
            request_body["toolConfig"] = json!({"functionCallingConfig": function_calling_config});
        }
        let payload = json!({
            "project": project_id,
            "model": model,
            "userAgent": "antigravity",
            "requestType": "agent",
            "requestId": uuid::Uuid::new_v4().to_string(),
            "request": request_body
        });
        for base in google_endpoint_chain() {
            let url = format!(
                "{}/v1internal:generateContent",
                base.trim_end_matches(['/'])
            );
            let resp = match http
                .post(&url)
                .headers(google_headers(&token))
                .json(&payload)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    last_error = Some((
                        GatewayErrorDetail::new(GatewayError::BadGateway)
                            .with_url(url.clone())
                            .with_transport_message(e.to_string()),
                        account.account_id.clone(),
                    ));
                    continue;
                }
            };
            let status = resp.status().as_u16();
            let body: Value = resp.json().await.unwrap_or_default();
            if (200..300).contains(&status) {
                let parts: Vec<Value> = body
                    .pointer("/response/candidates/0/content/parts")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let txt = parts
                    .iter()
                    .filter_map(|p| p.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n");
                let mut tool_calls: Vec<Value> = Vec::new();
                for p in &parts {
                    let Some(fc) = p.get("functionCall").or_else(|| p.get("function_call")) else {
                        continue;
                    };
                    let name = fc.get("name").and_then(Value::as_str).unwrap_or("tool");
                    let args_obj = fc.get("args").cloned().unwrap_or_else(|| json!({}));
                    let args =
                        serde_json::to_string(&args_obj).unwrap_or_else(|_| "{}".to_string());
                    let thought_signature = fc
                        .get("thought_signature")
                        .or_else(|| fc.get("thoughtSignature"))
                        .or_else(|| p.get("thought_signature"))
                        .or_else(|| p.get("thoughtSignature"))
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    let tool_call_id = format!("call_{}", uuid::Uuid::new_v4().simple());
                    if let Some(sig) = thought_signature.as_deref() {
                        state
                            .google_tool_signatures
                            .put(&tool_call_id, name, &args_obj, sig);
                    }
                    let mut function = json!({
                        "name": name,
                        "arguments": args
                    });
                    if let Some(sig) = thought_signature {
                        function["thought_signature"] = Value::String(sig);
                    }
                    tool_calls.push(json!({
                        "id": tool_call_id,
                        "type": "function",
                        "function": function
                    }));
                }
                let usage_obj = body
                    .get("usageMetadata")
                    .or_else(|| body.get("response").and_then(|r| r.get("usageMetadata")));
                let usage_fallback = body.get("response").and_then(|r| r.get("usage"));
                let prompt_tokens = usage_obj
                    .and_then(|u| {
                        u.get("promptTokenCount")
                            .or_else(|| u.get("inputTokenCount"))
                    })
                    .and_then(Value::as_u64)
                    .or_else(|| {
                        usage_fallback
                            .and_then(|u| u.get("input_tokens"))
                            .and_then(Value::as_u64)
                    })
                    .unwrap_or(0);
                let completion_tokens = usage_obj
                    .and_then(|u| {
                        u.get("candidatesTokenCount")
                            .or_else(|| u.get("outputTokenCount"))
                    })
                    .and_then(Value::as_u64)
                    .or_else(|| {
                        usage_fallback
                            .and_then(|u| u.get("output_tokens"))
                            .and_then(Value::as_u64)
                    })
                    .unwrap_or(0);
                let total_tokens = usage_obj
                    .and_then(|u| u.get("totalTokenCount"))
                    .and_then(Value::as_u64)
                    .or_else(|| {
                        usage_fallback
                            .and_then(|u| u.get("total_tokens"))
                            .and_then(Value::as_u64)
                    })
                    .unwrap_or(prompt_tokens.saturating_add(completion_tokens));
                let out = json!({
                    "id": super::chatcmpl_id(),
                    "object":"chat.completion",
                    "created": super::now_unix() as i64,
                    "model": model,
                    "choices":[{
                        "index":0,
                        "finish_reason": if tool_calls.is_empty() { "stop" } else { "tool_calls" },
                        "message":{
                            "role":"assistant",
                            "content": if txt.is_empty() { Value::Null } else { Value::String(txt) },
                            "tool_calls": if tool_calls.is_empty() { Value::Null } else { Value::Array(tool_calls) }
                        },
                        "logprobs":Value::Null
                    }],
                    "usage":{"prompt_tokens":prompt_tokens,"completion_tokens":completion_tokens,"total_tokens":total_tokens}
                });
                return Ok((RoutedResponse::Json(out), Some(account.account_id.clone())));
            }

            let kind = if status == 429 {
                GatewayError::RateLimit
            } else if status == 401 || status == 403 {
                GatewayError::Auth
            } else if status == 503 {
                GatewayError::TenantOffline
            } else {
                GatewayError::BadGateway
            };
            let body_snippet = {
                let mut s = body.to_string().replace(['\r', '\n'], " ");
                s = s.trim().to_string();
                if s.len() > 240 {
                    s.truncate(240);
                    s.push_str("...");
                }
                s
            };
            last_error = Some((
                GatewayErrorDetail::new(kind)
                    .with_url(url.clone())
                    .with_status(status)
                    .with_body_snippet(body_snippet),
                account.account_id.clone(),
            ));
        }
    }
    if let Some((detail, account_id)) = last_error {
        let _ = store
            .set_app_state(
                "diag_last_upstream_error_json",
                &serde_json::to_string(&json!({
                    "seen_unix": super::now_unix(),
                    "account_id": account_id,
                    "kind": detail.kind,
                    "url": detail.url,
                    "status": detail.status,
                    "body_snippet": detail.body_snippet,
                    "transport_message": detail.transport_message,
                }))
                .unwrap_or_default(),
            )
            .await;
        return Err(detail);
    }
    Err(GatewayErrorDetail::new(GatewayError::BadGateway))
}

fn google_stream_frame(payload: &Value) -> Bytes {
    let mut s = String::new();
    s.push_str("data: ");
    s.push_str(&payload.to_string());
    s.push_str("\n\n");
    Bytes::from(s)
}

fn google_sse_one_shot(payload: &Value) -> axum::response::Response {
    let body = Body::from_stream(tokio_stream::once(Ok::<Bytes, std::io::Error>(
        google_stream_frame(payload),
    )));
    let mut resp = axum::response::Response::new(body);
    *resp.status_mut() = StatusCode::OK;
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        "text/event-stream".parse().expect("valid content-type"),
    );
    resp
}

pub(crate) async fn google_model_dispatch(
    State(state): State<AppState>,
    Path(tail): Path<String>,
    headers: HeaderMap,
    Json(req): Json<Value>,
) -> impl IntoResponse {
    // Compatibility for Google-style routes with colon suffix:
    // /v1/google/models/{model}:generateContent and :streamGenerateContent
    let (model, stream_mode) = if let Some(idx) = tail.rfind(":streamGenerateContent") {
        (tail[..idx].to_string(), true)
    } else if let Some(idx) = tail.rfind(":generateContent") {
        (tail[..idx].to_string(), false)
    } else if let Some(idx) = tail.rfind("/streamGenerateContent") {
        (tail[..idx].to_string(), true)
    } else if let Some(idx) = tail.rfind("/generateContent") {
        (tail[..idx].to_string(), false)
    } else {
        return super::google_error_response(
            StatusCode::BAD_REQUEST,
            "invalid google route; expected generateContent or streamGenerateContent",
            "INVALID_ARGUMENT",
        );
    };
    google_generate_content_impl(&state, &headers, &model, &req, stream_mode).await
}

pub(crate) async fn google_generate_content_impl(
    state: &AppState,
    headers: &HeaderMap,
    model: &str,
    req: &Value,
    stream_mode: bool,
) -> axum::response::Response {
    let _ = super::increment_counter(state, "diag_requests_total").await;
    let recent_path = if stream_mode {
        "/v1/google/models/{model}:streamGenerateContent"
    } else {
        "/v1/google/models/{model}:generateContent"
    };
    super::push_recent_request_bounded(state, req, recent_path).await;
    let bearer_auth = super::evaluate_bearer_auth(headers, state).await;
    if !bearer_auth.authorized {
        super::record_bearer_auth_rejection(state, &bearer_auth.reason).await;
        return super::google_error_response(
            StatusCode::UNAUTHORIZED,
            "missing or invalid API key",
            "AUTHENTICATION_ERROR",
        );
    }
    let converted = match google_generate_to_openai_chat(model, req, stream_mode) {
        Ok(v) => v,
        Err(msg) => {
            return super::google_error_response(
                StatusCode::BAD_REQUEST,
                msg.as_str(),
                "INVALID_ARGUMENT",
            )
        }
    };
    if super::validate_chat_completions_strict(&converted).is_err() {
        return super::google_error_response(
            StatusCode::BAD_REQUEST,
            "invalid generateContent payload",
            "INVALID_ARGUMENT",
        );
    }
    if super::validate_requested_model(state, &converted)
        .await
        .is_err()
    {
        return super::google_error_response(
            StatusCode::BAD_REQUEST,
            "unsupported model",
            "INVALID_ARGUMENT",
        );
    }
    let requested_model = converted
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or(model)
        .to_string();
    let _ = super::record_model_usage(state, requested_model.clone()).await;

    match super::route_chat_with_failover(state, converted, "/v1/chat/completions").await {
        Ok((RoutedResponse::Json(body), account_id)) => {
            let inp = body
                .get("usage")
                .and_then(|u| u.get("prompt_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let out_tokens = body
                .get("usage")
                .and_then(|u| u.get("completion_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            if let Some(kid) = &bearer_auth.key_id {
                let _ =
                    super::record_key_model_usage(state, kid, &requested_model, inp, out_tokens, 0)
                        .await;
            }
            let _ = super::add_model_token_usage(state, &requested_model, inp, out_tokens, 0).await;
            if let Some(aid) = &account_id {
                let _ = super::record_oauth_account_usage(
                    state,
                    aid,
                    &requested_model,
                    inp,
                    out_tokens,
                    0,
                )
                .await;
            }
            let out = openai_chat_to_google_response(&body);
            if stream_mode {
                google_sse_one_shot(&out)
            } else {
                (StatusCode::OK, Json(out)).into_response()
            }
        }
        Ok((RoutedResponse::Sse(resp), _)) => {
            if stream_mode {
                super::sse_response(resp)
            } else {
                super::google_error_response(
                    StatusCode::BAD_GATEWAY,
                    "unexpected streaming upstream response",
                    "INTERNAL",
                )
            }
        }
        Err(_) => {
            super::google_error_response(StatusCode::BAD_GATEWAY, "upstream failure", "INTERNAL")
        }
    }
}
