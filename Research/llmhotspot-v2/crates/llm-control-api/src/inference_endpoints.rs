use super::*;
pub(crate) async fn chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<Value>,
) -> impl IntoResponse {
    if trace_io_enabled() {
        trace_io(
            "chat_completions_in",
            request_trace_metadata(&headers, &req),
        );
    }
    let _ = increment_counter(&state, "diag_requests_total").await;
    push_recent_request_bounded(&state, &req, "/v1/chat/completions").await;
    let bearer_auth = evaluate_bearer_auth(&headers, &state).await;
    if !bearer_auth.authorized {
        record_bearer_auth_rejection(&state, &bearer_auth.reason).await;
        if trace_io_enabled() {
            trace_io(
                "bearer_auth_rejected",
                json!({
                    "path": "/v1/chat/completions",
                    "reason": bearer_auth.reason,
                }),
            );
        }
        let _ = push_recent_error(&state, "auth: missing or invalid bearer token").await;
        return openai_error_response(
            StatusCode::UNAUTHORIZED,
            "missing or invalid bearer token",
            "authentication_error",
            Some("authorization"),
        );
    }

    let mut req = req;
    normalize_requested_model_alias_inplace(&state, &mut req).await;

    // Cursor Agent (and some other agents) POST a Responses-style payload to `/v1/chat/completions`.
    // We support both formats by selecting a mode based on the presence of `messages` vs `input`.
    let has_messages = req.get("messages").is_some();
    let has_input = req.get("input").is_some();
    if has_messages && has_input {
        let _ = push_recent_error(&state, "request: ambiguous payload (messages + input)").await;
        return openai_error_response(
            StatusCode::BAD_REQUEST,
            "invalid request: provide either `messages` (Chat Completions) or `input` (Responses), not both",
            "invalid_request_error",
            None,
        );
    }

    // Strict Chat Completions path.
    if has_messages {
        let msg_repair = normalize_openai_messages_inplace(&mut req);
        if msg_repair.repaired && trace_io_enabled() {
            trace_io(
                "messages_normalized",
                json!({
                    "dropped_count": msg_repair.dropped_count,
                    "pruned_count": msg_repair.pruned_count,
                }),
            );
        }
        let tool_repair = normalize_openai_tools_inplace(&mut req);
        if tool_repair.unrecoverable {
            let msg = format!(
                "retryable request normalization error: {}; retry with canonical OpenAI tools[].type=\"function\" schema",
                tool_repair
                    .reason
                    .as_deref()
                    .unwrap_or("unrecognized tools payload mismatch")
            );
            let _ = push_recent_error(&state, &format!("request: {msg}")).await;
            return openai_error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                &msg,
                "server_error",
                Some("tools"),
            );
        }
        normalize_openai_tool_choice_inplace(&mut req);
        if let Err(resp) = validate_chat_completions_strict(&req) {
            let _ = push_recent_error(&state, "request: invalid chat payload").await;
            return resp;
        }
        if let Err(resp) = validate_requested_model(&state, &req).await {
            let _ = push_recent_error(&state, "request: unsupported model").await;
            return resp;
        }
        let requested_model = model_for_diag_usage(&req);
        let _ = record_model_usage(&state, requested_model.clone()).await;
        let client_stream = req.get("stream").and_then(Value::as_bool).unwrap_or(false);

        return match route_chat_with_failover(&state, req, "/v1/chat/completions").await {
            Ok((RoutedResponse::Json(body), account_id)) => {
                if trace_io_enabled() {
                    trace_io(
                        "chat_completions_out",
                        json!({
                            "status": 200,
                            "type": "json",
                            "summary": response_trace_summary(&body),
                        }),
                    );
                }
                let inp = body
                    .get("usage")
                    .and_then(|u| u.get("prompt_tokens"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let out = body
                    .get("usage")
                    .and_then(|u| u.get("completion_tokens"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                if let Some(kid) = &bearer_auth.key_id {
                    let _ =
                        record_key_model_usage(&state, kid, &requested_model, inp, out, 0).await;
                }
                let _ = add_model_token_usage(&state, &requested_model, inp, out, 0).await;
                if let Some(aid) = &account_id {
                    let _ = record_oauth_account_usage(&state, aid, &requested_model, inp, out, 0)
                        .await;
                }
                if client_stream {
                    chat_completion_json_to_chunk_sse(&body)
                } else {
                    (StatusCode::OK, Json(body)).into_response()
                }
            }
            Ok((RoutedResponse::Sse(resp), account_id)) => {
                if trace_io_enabled() {
                    trace_io(
                        "chat_completions_out",
                        json!({
                            "status": resp.status().as_u16(),
                            "type": "sse",
                            "mode": "responses_to_chat_chunks",
                        }),
                    );
                }
                chat_completion_sse_from_responses(
                    resp,
                    true, // bridge custom tool events (ApplyPatch etc.) for all streaming clients
                    state.clone(),
                    bearer_auth.key_id.clone(),
                    account_id,
                    requested_model,
                )
            }
            Err(err) => {
                if trace_io_enabled() {
                    trace_io(
                        "chat_completions_out",
                        json!({
                            "status": "error",
                            "error": err.to_string(),
                        }),
                    );
                }
                let _ = increment_counter(&state, "diag_failovers_total").await;
                let _ = push_recent_error(&state, &format!("gateway: {err}")).await;
                map_gateway_error_openai(err)
            }
        };
    }

    // Responses shim: accept `input` payloads, route as Responses, but return Chat Completions output.
    if has_input {
        if let Err(resp) = validate_responses_payload_minimal(&req) {
            let _ = push_recent_error(&state, "request: invalid responses payload").await;
            return resp;
        }
        if let Err(resp) = validate_requested_model(&state, &req).await {
            let _ = push_recent_error(&state, "request: unsupported model").await;
            return resp;
        }
        let requested_model = model_for_diag_usage(&req);
        let _ = record_model_usage(&state, requested_model.clone()).await;

        let client_stream = req.get("stream").and_then(Value::as_bool).unwrap_or(false);
        let mut upstream_req = req.clone();
        upstream_req["stream"] = Value::Bool(true);

        return match route_chat_with_failover(&state, upstream_req, "/v1/responses").await {
            Ok((RoutedResponse::Sse(resp), account_id)) => {
                if client_stream {
                    chat_completion_sse_from_responses(
                        resp,
                        true,
                        state.clone(),
                        bearer_auth.key_id.clone(),
                        account_id,
                        requested_model.clone(),
                    )
                } else {
                    // Collect the Responses SSE and synthesize a Chat Completions JSON response.
                    match collect_codex_sse(resp).await {
                        Ok(projected) => {
                            let finish_reason = if projected.tool_calls.is_empty() {
                                "stop"
                            } else {
                                "tool_calls"
                            };
                            let mut message = json!({
                                "role": "assistant",
                                "content": if projected.text.is_empty() {
                                    Value::Null
                                } else {
                                    Value::String(projected.text)
                                }
                            });
                            if !projected.tool_calls.is_empty() {
                                message["tool_calls"] = Value::Array(projected.tool_calls);
                            }
                            let mdl = req
                                .get("model")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string();
                            if let Some(kid) = &bearer_auth.key_id {
                                let _ = record_key_model_usage(
                                    &state,
                                    kid,
                                    &mdl,
                                    projected.usage_prompt_tokens,
                                    projected.usage_completion_tokens,
                                    projected.usage_cached_tokens,
                                )
                                .await;
                            }
                            let _ = add_model_token_usage(
                                &state,
                                &mdl,
                                projected.usage_prompt_tokens,
                                projected.usage_completion_tokens,
                                projected.usage_cached_tokens,
                            )
                            .await;
                            if let Some(aid) = &account_id {
                                let _ = record_oauth_account_usage(
                                    &state,
                                    aid,
                                    &mdl,
                                    projected.usage_prompt_tokens,
                                    projected.usage_completion_tokens,
                                    projected.usage_cached_tokens,
                                )
                                .await;
                            }
                            (
                                StatusCode::OK,
                                Json(json!({
                                    "id": chatcmpl_id(),
                                    "object": "chat.completion",
                                    "created": now_unix() as i64,
                                    "model": req.get("model").and_then(Value::as_str).unwrap_or("gpt-5.2"),
                                    "system_fingerprint": Value::Null,
                                    "choices": [{
                                        "index": 0,
                                        "finish_reason": finish_reason,
                                        "message": message,
                                        "logprobs": Value::Null
                                    }],
                                    "usage": {
                                        "prompt_tokens": projected.usage_prompt_tokens,
                                        "completion_tokens": projected.usage_completion_tokens,
                                        "total_tokens": projected.usage_total_tokens
                                    }
                                })),
                            )
                            .into_response()
                        }
                        Err(_) => map_gateway_error_openai(GatewayError::BadGateway),
                    }
                }
            }
            Ok((RoutedResponse::Json(body), _)) => (StatusCode::OK, Json(body)).into_response(),
            Err(err) => {
                let _ = increment_counter(&state, "diag_failovers_total").await;
                let _ = push_recent_error(&state, &format!("gateway: {err}")).await;
                map_gateway_error_openai(err)
            }
        };
    }

    let _ = push_recent_error(&state, "request: missing messages/input").await;
    openai_error_response(
        StatusCode::BAD_REQUEST,
        "invalid request: missing `messages` (Chat Completions) or `input` (Responses)",
        "invalid_request_error",
        None,
    )
}

pub(crate) async fn anthropic_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<Value>,
) -> impl IntoResponse {
    if trace_io_enabled() {
        trace_io(
            "anthropic_messages_in",
            request_trace_metadata(&headers, &req),
        );
    }
    let _ = increment_counter(&state, "diag_requests_total").await;
    push_recent_request_bounded(&state, &req, "/v1/messages").await;
    let bearer_auth = evaluate_bearer_auth(&headers, &state).await;
    if !bearer_auth.authorized {
        record_bearer_auth_rejection(&state, &bearer_auth.reason).await;
        return anthropic_error_response(
            StatusCode::UNAUTHORIZED,
            "missing or invalid API key",
            "authentication_error",
        );
    }

    let stream_requested = req.get("stream").and_then(Value::as_bool).unwrap_or(false);

    let requested_model = req
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(resolve_anthropic_model)
        .unwrap_or_else(|| "claude-sonnet-4-6".to_string());

    if stream_requested && is_claude_model(&requested_model) {
        let Some(store) = &state.store else {
            return anthropic_error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "storage is not configured",
                "api_error",
            );
        };
        let model = requested_model.clone();
        if req.get("messages").and_then(Value::as_array).is_none() {
            return anthropic_error_response(
                StatusCode::BAD_REQUEST,
                "missing required field: messages",
                "invalid_request_error",
            );
        }
        let mut upstream_payload = req.clone();
        upstream_payload["model"] = Value::String(model);
        upstream_payload["stream"] = Value::Bool(true);
        if let Some(messages) = upstream_payload.get("messages").and_then(Value::as_array) {
            let has_tool_result_in_context = anthropic_messages_have_tool_result(messages);
            let pure_tool_result_followup = anthropic_last_message_is_pure_tool_result(messages);
            let disable_tools = match upstream_payload.get("tool_choice") {
                Some(Value::String(s)) => s == "none",
                Some(Value::Object(obj)) => obj.get("type").and_then(Value::as_str) == Some("none"),
                _ => false,
            };
            if disable_tools || pure_tool_result_followup {
                if let Some(obj) = upstream_payload.as_object_mut() {
                    obj.remove("tools");
                    obj.remove("tool_choice");
                }
            } else if has_tool_result_in_context {
                let force_any = match upstream_payload.get("tool_choice") {
                    Some(Value::String(s)) => s == "any",
                    Some(Value::Object(obj)) => {
                        obj.get("type").and_then(Value::as_str) == Some("any")
                    }
                    _ => false,
                };
                if force_any {
                    upstream_payload["tool_choice"] = json!({"type":"auto"});
                }
            }
        }

        let accounts = match store.list_oauth_accounts().await {
            Ok(v) => v,
            Err(_) => {
                return anthropic_error_response(
                    StatusCode::BAD_GATEWAY,
                    "upstream failure",
                    "api_error",
                )
            }
        };
        let anthropic_accounts: Vec<_> = accounts
            .into_iter()
            .filter(|a| a.enabled && is_anthropic_account(a))
            .collect();
        if anthropic_accounts.is_empty() {
            return anthropic_error_response(
                StatusCode::BAD_GATEWAY,
                "no enabled Anthropic account",
                "api_error",
            );
        }

        let http = state.http_long().clone();

        for account in &anthropic_accounts {
            let token =
                match get_anthropic_access_token(Some(&state), store, &account.account_id).await {
                    Ok(t) => t,
                    Err(_) => continue,
                };

            let mut up = http
                .post(format!("{}/v1/messages", ANTHROPIC_UPSTREAM))
                .header("Authorization", format!("Bearer {token}"))
                .header("Content-Type", "application/json")
                .header("anthropic-version", "2023-06-01");
            if is_anthropic_oauth_token(&token) {
                up = up.header("anthropic-beta", ANTHROPIC_OAUTH_BETA_HEADER);
            }
            let resp = match up.json(&upstream_payload).send().await {
                Ok(r) => r,
                Err(_) => continue,
            };
            let status = resp.status().as_u16();
            if status == 401 || status == 403 || status == 429 {
                continue;
            }
            if (200..300).contains(&status) {
                let _ = record_model_usage(&state, model_for_diag_usage(&upstream_payload)).await;
                return anthropic_sse_passthrough_response(resp);
            }
        }

        return anthropic_error_response(StatusCode::BAD_GATEWAY, "upstream failure", "api_error");
    }

    let req_for_convert = if stream_requested && !is_claude_model(&requested_model) {
        let mut adjusted = req.clone();
        adjusted["stream"] = Value::Bool(false);
        adjusted
    } else {
        req.clone()
    };
    let converted = match anthropic_messages_to_openai_chat(&req_for_convert) {
        Ok(v) => v,
        Err(msg) => {
            let _ = push_recent_error(
                &state,
                &format!("request: invalid anthropic payload: {msg}"),
            )
            .await;
            return anthropic_error_response(
                StatusCode::BAD_REQUEST,
                &msg,
                "invalid_request_error",
            );
        }
    };

    if validate_chat_completions_strict(&converted).is_err() {
        let _ = push_recent_error(&state, "request: invalid anthropic messages payload").await;
        return anthropic_error_response(
            StatusCode::BAD_REQUEST,
            "invalid messages payload",
            "invalid_request_error",
        );
    }
    if validate_requested_model(&state, &converted).await.is_err() {
        let _ = push_recent_error(&state, "request: unsupported model").await;
        let requested = converted
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        return anthropic_error_response(
            StatusCode::BAD_REQUEST,
            &format!("unsupported model '{requested}'"),
            "invalid_request_error",
        );
    }

    let converted_model = converted
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("claude")
        .to_string();
    let _ = record_model_usage(&state, converted_model.clone()).await;

    match route_chat_with_failover(&state, converted, "/v1/chat/completions").await {
        Ok((RoutedResponse::Json(body), account_id)) => {
            let inp = body
                .get("usage")
                .and_then(|u| u.get("prompt_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let out = body
                .get("usage")
                .and_then(|u| u.get("completion_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            if let Some(kid) = &bearer_auth.key_id {
                let _ = record_key_model_usage(&state, kid, &converted_model, inp, out, 0).await;
            }
            let _ = add_model_token_usage(&state, &converted_model, inp, out, 0).await;
            if let Some(aid) = &account_id {
                let _ =
                    record_oauth_account_usage(&state, aid, &converted_model, inp, out, 0).await;
            }

            let out_body = openai_chat_to_anthropic_response(&body);
            (StatusCode::OK, Json(out_body)).into_response()
        }
        Ok((RoutedResponse::Sse(_), _)) => anthropic_error_response(
            StatusCode::BAD_GATEWAY,
            "unexpected streaming upstream response",
            "api_error",
        ),
        Err(err) => {
            let _ = increment_counter(&state, "diag_failovers_total").await;
            let _ = push_recent_error(&state, &format!("gateway: {err}")).await;
            anthropic_error_response(StatusCode::BAD_GATEWAY, "upstream failure", "api_error")
        }
    }
}

pub(crate) async fn responses(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<Value>,
) -> impl IntoResponse {
    if trace_io_enabled() {
        trace_io("responses_in", request_trace_metadata(&headers, &req));
    }
    let _ = increment_counter(&state, "diag_requests_total").await;
    push_recent_request_bounded(&state, &req, "/v1/responses").await;
    let bearer_auth = evaluate_bearer_auth(&headers, &state).await;
    if !bearer_auth.authorized {
        record_bearer_auth_rejection(&state, &bearer_auth.reason).await;
        if trace_io_enabled() {
            trace_io(
                "bearer_auth_rejected",
                json!({
                    "path": "/v1/responses",
                    "reason": bearer_auth.reason,
                }),
            );
        }
        let _ = push_recent_error(&state, "auth: missing or invalid bearer token").await;
        return openai_error_response(
            StatusCode::UNAUTHORIZED,
            "missing or invalid bearer token",
            "authentication_error",
            Some("authorization"),
        );
    }
    let mut req = req;
    normalize_requested_model_alias_inplace(&state, &mut req).await;
    if let Err(resp) = validate_requested_model(&state, &req).await {
        let _ = push_recent_error(&state, "request: unsupported model").await;
        return resp;
    }
    let requested_model = model_for_diag_usage(&req);
    let _ = record_model_usage(&state, requested_model.clone()).await;

    match route_chat_with_failover(&state, req, "/v1/responses").await {
        Ok((RoutedResponse::Json(body), account_id)) => {
            let inp = body
                .get("usage")
                .and_then(|u| u.get("prompt_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let out = body
                .get("usage")
                .and_then(|u| u.get("completion_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            if let Some(kid) = &bearer_auth.key_id {
                let _ = record_key_model_usage(&state, kid, &requested_model, inp, out, 0).await;
            }
            let _ = add_model_token_usage(&state, &requested_model, inp, out, 0).await;
            if let Some(aid) = &account_id {
                let _ =
                    record_oauth_account_usage(&state, aid, &requested_model, inp, out, 0).await;
            }
            (StatusCode::OK, Json(body)).into_response()
        }
        Ok((RoutedResponse::Sse(resp), _)) => responses_sse_normalized(resp),
        Err(err) => {
            let _ = increment_counter(&state, "diag_failovers_total").await;
            let _ = push_recent_error(&state, &format!("gateway: {err}")).await;
            map_gateway_error_openai(err)
        }
    }
}

#[allow(clippy::result_large_err)] // Keep direct Response errors to preserve existing handler call sites.
pub(crate) fn validate_responses_payload_minimal(
    req: &Value,
) -> Result<(), axum::response::Response> {
    // Minimal guardrails: require `input` and reject ambiguous payloads.
    if req.get("input").is_none() {
        return Err(openai_error_response(
            StatusCode::BAD_REQUEST,
            "missing required field: input",
            "invalid_request_error",
            Some("input"),
        ));
    }
    if req.get("messages").is_some() {
        return Err(openai_error_response(
            StatusCode::BAD_REQUEST,
            "invalid request: provide either `messages` or `input`, not both",
            "invalid_request_error",
            None,
        ));
    }
    if let Some(arr) = req.get("input").and_then(Value::as_array) {
        if arr.is_empty() {
            return Err(openai_error_response(
                StatusCode::BAD_REQUEST,
                "invalid input: must contain at least 1 item",
                "invalid_request_error",
                Some("input"),
            ));
        }
    }
    Ok(())
}

#[allow(clippy::result_large_err)] // Keep direct Response errors to preserve existing handler call sites.
pub(crate) fn validate_chat_completions_strict(
    req: &Value,
) -> Result<(), axum::response::Response> {
    // Strict OpenAI Chat Completions: require `messages[]` and reject Responses-format `input`.
    if req.get("input").is_some() {
        return Err(openai_error_response(
            StatusCode::BAD_REQUEST,
            "`/v1/chat/completions` requires `messages[]`; `input` is not supported. Configure your client to use Chat Completions format.",
            "invalid_request_error",
            Some("input"),
        ));
    }

    let Some(model) = req.get("model").and_then(Value::as_str) else {
        return Err(openai_error_response(
            StatusCode::BAD_REQUEST,
            "missing required field: model",
            "invalid_request_error",
            Some("model"),
        ));
    };
    if model.trim().is_empty() {
        return Err(openai_error_response(
            StatusCode::BAD_REQUEST,
            "invalid model: empty string",
            "invalid_request_error",
            Some("model"),
        ));
    }

    let Some(messages) = req.get("messages").and_then(Value::as_array) else {
        return Err(openai_error_response(
            StatusCode::BAD_REQUEST,
            "missing required field: messages (array)",
            "invalid_request_error",
            Some("messages"),
        ));
    };
    if messages.is_empty() {
        return Err(openai_error_response(
            StatusCode::BAD_REQUEST,
            "invalid messages: must contain at least 1 message",
            "invalid_request_error",
            Some("messages"),
        ));
    }
    let has_material = messages.iter().any(|m| {
        if m.get("tool_calls")
            .and_then(Value::as_array)
            .is_some_and(|tc| !tc.is_empty())
        {
            return true;
        }
        !chat_content_to_text(m.get("content").unwrap_or(&Value::Null)).is_empty()
    });
    if !has_material {
        return Err(openai_error_response(
            StatusCode::BAD_REQUEST,
            "invalid messages: no usable content or tool_calls provided",
            "invalid_request_error",
            Some("messages"),
        ));
    }

    if let Some(tools) = req.get("tools").and_then(Value::as_array) {
        for (i, tool) in tools.iter().enumerate() {
            if let Some(funcs) = tool.get("functionDeclarations").and_then(Value::as_array) {
                for (j, f) in funcs.iter().enumerate() {
                    let name_ok = f
                        .get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.trim().is_empty());
                    if !name_ok {
                        return Err(openai_error_response(
                            StatusCode::BAD_REQUEST,
                            &format!(
                                "invalid tools[{i}].functionDeclarations[{j}]: name must be a non-empty string"
                            ),
                            "invalid_request_error",
                            Some("tools"),
                        ));
                    }
                    if let Some(schema) = f.get("parameters").or_else(|| f.get("input_schema")) {
                        if !schema.is_object() && !schema.is_null() {
                            return Err(openai_error_response(
                                StatusCode::BAD_REQUEST,
                                &format!(
                                    "invalid tools[{i}].functionDeclarations[{j}]: parameters must be an object"
                                ),
                                "invalid_request_error",
                                Some("tools"),
                            ));
                        }
                    }
                }
                continue;
            }
            let t = tool.get("type").and_then(Value::as_str).unwrap_or_default();
            if t != "function" {
                let name_ok = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.trim().is_empty());
                if name_ok {
                    if let Some(schema) =
                        tool.get("input_schema").or_else(|| tool.get("parameters"))
                    {
                        if !schema.is_object() && !schema.is_null() {
                            return Err(openai_error_response(
                                StatusCode::BAD_REQUEST,
                                &format!("invalid tools[{i}]: tool schema must be an object"),
                                "invalid_request_error",
                                Some("tools"),
                            ));
                        }
                    }
                    continue;
                }
                return Err(openai_error_response(
                    StatusCode::BAD_REQUEST,
                    &format!(
                        "invalid tools[{i}]: expected OpenAI function tool, Anthropic tool, or Google functionDeclarations"
                    ),
                    "invalid_request_error",
                    Some("tools"),
                ));
            }
            let Some(function) = tool.get("function").and_then(Value::as_object) else {
                let name_ok = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.trim().is_empty());
                if name_ok {
                    if let Some(params) =
                        tool.get("parameters").or_else(|| tool.get("input_schema"))
                    {
                        if !params.is_object() && !params.is_null() {
                            return Err(openai_error_response(
                                StatusCode::BAD_REQUEST,
                                &format!(
                                    "invalid tools[{i}]: function.parameters must be an object"
                                ),
                                "invalid_request_error",
                                Some("tools"),
                            ));
                        }
                    }
                    continue;
                }
                return Err(openai_error_response(
                    StatusCode::BAD_REQUEST,
                    &format!("invalid tools[{i}]: missing function object"),
                    "invalid_request_error",
                    Some("tools"),
                ));
            };
            let name_ok = function
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty());
            if !name_ok {
                return Err(openai_error_response(
                    StatusCode::BAD_REQUEST,
                    &format!("invalid tools[{i}]: function.name must be a non-empty string"),
                    "invalid_request_error",
                    Some("tools"),
                ));
            }
            // parameters is optional; some tools take no arguments
            if let Some(params) = function.get("parameters") {
                if !params.is_object() && !params.is_null() {
                    return Err(openai_error_response(
                        StatusCode::BAD_REQUEST,
                        &format!("invalid tools[{i}]: function.parameters must be an object"),
                        "invalid_request_error",
                        Some("tools"),
                    ));
                }
            }
        }
    }

    if let Some(choice) = req.get("tool_choice") {
        // Accept null (treated as absent) or any string/object; only reject obviously wrong strings.
        if !choice.is_null() {
            if let Some(s) = choice.as_str() {
                if !matches!(s, "none" | "auto" | "required") {
                    return Err(openai_error_response(
                        StatusCode::BAD_REQUEST,
                        "invalid tool_choice: must be one of none|auto|required or a function object",
                        "invalid_request_error",
                        Some("tool_choice"),
                    ));
                }
            }
            // Object-form tool_choice (e.g. {type:"function",...} or {type:"auto"}) â€” pass through.
        }
    }

    Ok(())
}
