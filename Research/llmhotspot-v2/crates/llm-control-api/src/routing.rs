use axum::http::StatusCode;
use llm_auth::OAuthAccountManager;
use llm_foundation::contracts::OpenAiRequestEnvelope;
use llm_gateway::{
    forward_openai_json, forward_openai_sse, resolve_codex_oauth_config, resolve_request_config,
    GatewayError, GatewayErrorDetail, GatewayService, UpstreamRoutingProfile,
};
use llm_storage::{OAuthAccountRecord, RemoteAssignment, SqliteStore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

use crate::diagnostics::DIAG_LAST_FAILOVER_CHAIN_KEY;
use crate::google_routing::route_google_request;
use crate::model_resolution::{
    normalize_model_name_for_routing, parse_headers_json, resolve_provider_for_model,
    route_anthropic_request,
};

use super::{
    build_codex_payload, build_codex_request_headers, cached_models_key, chatcmpl_id,
    collect_codex_sse, compat_trace_enabled, diag_truncate, get_custom_api_key, increment_counter,
    is_anthropic_account, is_google_account, is_openai_account, list_all_accounts,
    list_custom_api_accounts, load_models_cache, now_unix, oauth_refresh_lock,
    oauth_upstream_locked, openai_error_response, parse_model_ids_from_cache_data,
    push_wire_capture_event, record_last_upstream_error, redact_sensitive_text,
    refresh_custom_api_models_for_account, request_format_kind,
    responses_json_from_chat_projection, tool_profile, tool_trace_name, trace_io, trace_io_enabled,
    wire_capture_max_chars, AppState, RoutedResponse, ANTHROPIC_PROVIDER_ID, GOOGLE_PROVIDER_ID,
    OPENAI_API_PROVIDER_ID, OPENAI_PROVIDER_ID,
};

pub(crate) async fn provider_enabled_flags(state: &AppState) -> (bool, bool, bool) {
    let Some(store) = &state.store else {
        // Keep no-store/dev mode permissive.
        return (true, true, true);
    };
    let Ok(accounts) = list_all_accounts(store).await else {
        return (false, false, false);
    };
    let has_openai = accounts.iter().any(|a| a.enabled && is_openai_account(a));
    let has_anthropic = accounts
        .iter()
        .any(|a| a.enabled && is_anthropic_account(a));
    let has_google = accounts.iter().any(|a| a.enabled && is_google_account(a));
    (has_openai, has_anthropic, has_google)
}

pub(crate) async fn custom_accounts_for_model(
    store: &SqliteStore,
    model: &str,
    allow_refresh: bool,
) -> Vec<OAuthAccountRecord> {
    let normalized = model.to_ascii_lowercase();
    let mut out = Vec::new();
    let accounts = list_custom_api_accounts(store).await.unwrap_or_default();
    for acct in accounts {
        if !acct.enabled {
            continue;
        }
        let key = cached_models_key(OPENAI_API_PROVIDER_ID, &acct.account_id);
        let mut ids = if let Some(env) = load_models_cache(store, &key, now_unix()).await {
            parse_model_ids_from_cache_data(&env.data)
        } else {
            Vec::new()
        };
        if ids.is_empty() && allow_refresh {
            ids = refresh_custom_api_models_for_account(store, &acct, false)
                .await
                .unwrap_or_default();
        }
        if ids.iter().any(|id| id.eq_ignore_ascii_case(&normalized)) {
            out.push(acct);
        }
    }
    out
}

pub(crate) async fn normalize_requested_model_alias_inplace(state: &AppState, req: &mut Value) {
    let _ = state;
    let Some(model) = req.get("model").and_then(Value::as_str) else {
        return;
    };
    let normalized = normalize_model_name_for_routing(model);
    if normalized != model {
        req["model"] = Value::String(normalized);
    }
}

pub(crate) async fn validate_requested_model(
    state: &AppState,
    req: &Value,
) -> Result<(), axum::response::Response> {
    let parsed = serde_json::from_value::<OpenAiRequestEnvelope>(req.clone()).unwrap_or_default();
    let requested_model = parsed.model().unwrap_or("gpt-5.2");
    let normalized_model = normalize_model_name_for_routing(requested_model);
    let (openai_enabled, anthropic_enabled, google_enabled) = provider_enabled_flags(state).await;
    let Some(provider) = resolve_provider_for_model(&normalized_model) else {
        return Err(openai_error_response(
            StatusCode::BAD_REQUEST,
            &format!(
                "unsupported model '{requested_model}': model must start with gpt-, claude-, or gemini-"
            ),
            "invalid_request_error",
            Some("model"),
        ));
    };
    let custom_model_matches = if let Some(store) = &state.store {
        custom_accounts_for_model(store, &normalized_model, false).await
    } else {
        Vec::new()
    };
    if provider == OPENAI_PROVIDER_ID && !openai_enabled && custom_model_matches.is_empty() {
        return Err(openai_error_response(
            StatusCode::BAD_REQUEST,
            &format!("model '{requested_model}' requires an enabled OpenAI OAuth account"),
            "invalid_request_error",
            Some("model"),
        ));
    }
    if provider == ANTHROPIC_PROVIDER_ID && !anthropic_enabled && custom_model_matches.is_empty() {
        return Err(openai_error_response(
            StatusCode::BAD_REQUEST,
            &format!("model '{requested_model}' requires an enabled Anthropic OAuth account"),
            "invalid_request_error",
            Some("model"),
        ));
    }
    if provider == GOOGLE_PROVIDER_ID && !google_enabled && custom_model_matches.is_empty() {
        return Err(openai_error_response(
            StatusCode::BAD_REQUEST,
            &format!("model '{requested_model}' requires an enabled Google OAuth account"),
            "invalid_request_error",
            Some("model"),
        ));
    }
    Ok(())
}

pub(crate) async fn read_remote_assignment(state: &AppState) -> Option<RemoteAssignment> {
    let store = state.store.as_ref()?;
    let out = store.get_remote_assignment().await.ok()?;
    Some(out)
}

pub(crate) const OAUTH_FAILOVER_COOLDOWNS_KEY: &str = "oauth_failover_cooldowns_json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OAuthFailoverCooldown {
    until_unix: u64,
    reason: String,
    status: Option<u16>,
    updated_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FailoverAttemptTrace {
    attempt_index: u64,
    account_id: String,
    outcome: String,
    error_kind: Option<String>,
    status: Option<u16>,
    cooldown_until_unix: Option<u64>,
}

fn parse_reset_unix_from_body_snippet(raw: &str) -> Option<u64> {
    if let Ok(v) = serde_json::from_str::<Value>(raw) {
        if let Some(ts) = v.pointer("/error/resets_at").and_then(Value::as_u64) {
            return Some(ts);
        }
    }
    let marker = "\"resets_at\"";
    let idx = raw.find(marker)?;
    let tail = &raw[idx + marker.len()..];
    let colon = tail.find(':')?;
    let mut digits = String::new();
    for ch in tail[colon + 1..].chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
        } else if !digits.is_empty() {
            break;
        }
    }
    digits.parse::<u64>().ok()
}

fn is_codex_orphan_tool_output_error(detail: &GatewayErrorDetail) -> bool {
    if detail.status != Some(400) {
        return false;
    }
    let body = detail
        .body_snippet
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    let transport = detail
        .transport_message
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    body.contains("no tool call found for function call output")
        || transport.contains("no tool call found for function call output")
}

fn prune_orphan_function_call_outputs_inplace(payload: &mut Value) -> usize {
    let Some(input) = payload.get_mut("input").and_then(Value::as_array_mut) else {
        return 0;
    };
    let known_call_ids: HashSet<String> = input
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("function_call"))
        .filter_map(|item| item.get("call_id").and_then(Value::as_str))
        .map(str::to_string)
        .collect();

    let before = input.len();
    input.retain(|item| {
        if item.get("type").and_then(Value::as_str) != Some("function_call_output") {
            return true;
        }
        item.get("call_id")
            .and_then(Value::as_str)
            .is_some_and(|call_id| known_call_ids.contains(call_id))
    });
    let dropped = before.saturating_sub(input.len());
    if input.is_empty() {
        input.push(json!({"role":"user","content":[{"type":"input_text","text":"Continue."}]}));
    }
    dropped
}

fn classify_failover_cooldown(
    detail: &GatewayErrorDetail,
    now: u64,
) -> Option<OAuthFailoverCooldown> {
    let status = detail.status;
    if detail.kind == GatewayError::RateLimit || status == Some(429) {
        let fallback = std::env::var("LLMHOTSPOT_FAILOVER_RATE_LIMIT_COOLDOWN_SECS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(900);
        let until = detail
            .body_snippet
            .as_deref()
            .and_then(parse_reset_unix_from_body_snippet)
            .filter(|v| *v > now)
            .unwrap_or(now.saturating_add(fallback));
        return Some(OAuthFailoverCooldown {
            until_unix: until,
            reason: "rate_limit".to_string(),
            status,
            updated_unix: now,
        });
    }
    let blocked_text = detail
        .transport_message
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    let body_text = detail
        .body_snippet
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    if status == Some(403)
        || blocked_text.contains("blocked")
        || body_text.contains("request was blocked")
    {
        let cooldown = std::env::var("LLMHOTSPOT_FAILOVER_BLOCKED_COOLDOWN_SECS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(1800);
        return Some(OAuthFailoverCooldown {
            until_unix: now.saturating_add(cooldown),
            reason: "blocked".to_string(),
            status,
            updated_unix: now,
        });
    }
    None
}

async fn load_active_oauth_failover_cooldowns(
    store: &SqliteStore,
    now: u64,
) -> HashMap<String, OAuthFailoverCooldown> {
    let raw = store
        .get_app_state(OAUTH_FAILOVER_COOLDOWNS_KEY)
        .await
        .unwrap_or_default();
    let mut map: HashMap<String, OAuthFailoverCooldown> =
        serde_json::from_str(&raw).unwrap_or_default();
    map.retain(|_, c| c.until_unix > now);
    map
}

async fn save_oauth_failover_cooldowns(
    store: &SqliteStore,
    map: &HashMap<String, OAuthFailoverCooldown>,
) -> Result<(), String> {
    let encoded = serde_json::to_string(map).map_err(|e| e.to_string())?;
    store
        .set_app_state(OAUTH_FAILOVER_COOLDOWNS_KEY, &encoded)
        .await
        .map_err(|e| e.to_string())
}

async fn apply_failover_cooldown(
    state: &AppState,
    account_id: &str,
    detail: &GatewayErrorDetail,
) -> Option<OAuthFailoverCooldown> {
    let Some(store) = &state.store else {
        return None;
    };
    let now = now_unix();
    let next = classify_failover_cooldown(detail, now)?;
    let mut map = load_active_oauth_failover_cooldowns(store, now).await;
    match map.get(account_id) {
        Some(prev) if prev.until_unix >= next.until_unix => {}
        _ => {
            map.insert(account_id.to_string(), next.clone());
            let _ = save_oauth_failover_cooldowns(store, &map).await;
        }
    }
    Some(next)
}

async fn record_last_failover_chain(
    state: &AppState,
    attempts: &[FailoverAttemptTrace],
) -> Result<(), String> {
    let Some(store) = &state.store else {
        return Ok(());
    };
    let encoded = serde_json::to_string(attempts).map_err(|e| e.to_string())?;
    store
        .set_app_state(DIAG_LAST_FAILOVER_CHAIN_KEY, &encoded)
        .await
        .map_err(|e| e.to_string())
}

pub(crate) async fn route_chat_with_failover(
    state: &AppState,
    req: Value,
    default_path: &str,
) -> Result<(RoutedResponse, Option<String>), GatewayError> {
    let parsed = serde_json::from_value::<OpenAiRequestEnvelope>(req.clone()).unwrap_or_default();
    let stream = parsed.stream();

    let Some(store) = &state.store else {
        let out = state.gateway.chat(llm_foundation::contracts::ChatRequest {
            model: parsed.model().unwrap_or("gpt-5.2").to_string(),
            input: req.to_string(),
            stream,
        })?;
        return Ok((
            RoutedResponse::Json(json!({
                "id": out.id,
                "output_text": out.output_text
            })),
            None,
        ));
    };

    let requested_model = parsed.model().unwrap_or("gpt-5.2");
    let normalized_model = normalize_model_name_for_routing(requested_model);
    let provider = resolve_provider_for_model(&normalized_model).unwrap_or(OPENAI_PROVIDER_ID);
    let custom_model_accounts = custom_accounts_for_model(store, &normalized_model, true).await;
    let prefer_native_provider = custom_model_accounts.is_empty();
    if provider == ANTHROPIC_PROVIDER_ID && prefer_native_provider {
        return route_anthropic_request(state, store, &req).await;
    }
    if provider == GOOGLE_PROVIDER_ID && prefer_native_provider {
        return match route_google_request(state, store, &req).await {
            Ok(v) => Ok(v),
            Err(detail) => Err(detail.kind),
        };
    }

    let manager = OAuthAccountManager::new(store.clone());
    let sessions = {
        let openai_lock = oauth_refresh_lock(Some(state), "openai:workspace").await;
        let _openai_guard = if let Some(lock) = openai_lock.as_ref() {
            Some(lock.lock().await)
        } else {
            None
        };
        manager
            .resolve_access_sessions(&state.workspace_id, &state.oauth_client)
            .await
            .map_err(|_| GatewayError::Auth)?
    };
    let upstream_locked = oauth_upstream_locked();
    let base_url = std::env::var("LLMHOTSPOT_OPENAI_BASE_URL")
        .unwrap_or_else(|_| "https://api.openai.com".to_string());
    let is_responses = default_path == "/v1/responses";
    let mut candidates: Vec<OpenAiRouteCandidate> = Vec::new();
    for session in sessions {
        let account = store
            .get_oauth_account(&session.account_id)
            .await
            .ok()
            .flatten();
        let profile = if upstream_locked {
            None
        } else {
            account.map(|acc| UpstreamRoutingProfile {
                base_url: acc.upstream_base_url,
                chat_path: acc.chat_path,
                responses_path: acc.responses_path,
                org_id: acc.org_id,
                project_id: acc.project_id,
                extra_headers: parse_headers_json(acc.extra_headers_json.as_deref()),
            })
        };
        candidates.push(OpenAiRouteCandidate {
            account_id: session.account_id,
            account_type: "oauth",
            access_token: session.access_token,
            profile,
            codex_mode: upstream_locked,
        });
    }
    for acct in custom_model_accounts {
        let Ok(api_key) = get_custom_api_key(store, &acct.account_id).await else {
            continue;
        };
        if api_key.trim().is_empty() {
            continue;
        }
        candidates.push(OpenAiRouteCandidate {
            account_id: acct.account_id.clone(),
            account_type: "api",
            access_token: api_key.trim().to_string(),
            profile: Some(UpstreamRoutingProfile {
                base_url: acct.upstream_base_url.clone(),
                chat_path: acct.chat_path.clone(),
                responses_path: acct.responses_path.clone(),
                org_id: acct.org_id.clone(),
                project_id: acct.project_id.clone(),
                extra_headers: parse_headers_json(acct.extra_headers_json.as_deref()),
            }),
            codex_mode: false,
        });
    }
    if candidates.is_empty() {
        if provider == ANTHROPIC_PROVIDER_ID {
            return route_anthropic_request(state, store, &req).await;
        }
        if provider == GOOGLE_PROVIDER_ID {
            return match route_google_request(state, store, &req).await {
                Ok(v) => Ok(v),
                Err(detail) => Err(detail.kind),
            };
        }
        return Err(GatewayError::Auth);
    }
    let mut failover_trace: Vec<FailoverAttemptTrace> = Vec::new();
    let now = now_unix();
    let active_cooldowns = load_active_oauth_failover_cooldowns(store, now).await;
    let mut filtered_candidates = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if let Some(cooldown) = active_cooldowns.get(&candidate.account_id) {
            failover_trace.push(FailoverAttemptTrace {
                attempt_index: failover_trace.len() as u64,
                account_id: candidate.account_id.clone(),
                outcome: format!("skipped_cooldown:{}", cooldown.reason),
                error_kind: None,
                status: cooldown.status,
                cooldown_until_unix: Some(cooldown.until_unix),
            });
        } else {
            filtered_candidates.push(candidate);
        }
    }
    if filtered_candidates.is_empty() {
        let _ = record_last_failover_chain(state, &failover_trace).await;
        if provider == ANTHROPIC_PROVIDER_ID {
            return route_anthropic_request(state, store, &req).await;
        }
        if provider == GOOGLE_PROVIDER_ID {
            return match route_google_request(state, store, &req).await {
                Ok(v) => Ok(v),
                Err(detail) => Err(detail.kind),
            };
        }
        return Err(GatewayError::RateLimit);
    }

    let mut failover_hops = 0_u64;
    let mut last_error = GatewayError::TenantOffline;
    for (idx, candidate) in filtered_candidates.iter().enumerate() {
        let (cfg, payload, request_headers, codex_mode) = if candidate.codex_mode {
            let hdrs = build_codex_request_headers(
                &candidate.access_token,
                &state.workspace_id,
                &state.oauth_client.config.jwt_claim_path,
            );
            (
                resolve_codex_oauth_config(),
                build_codex_payload(&req, &state.workspace_id),
                hdrs,
                true,
            )
        } else {
            (
                resolve_request_config(
                    &base_url,
                    default_path,
                    is_responses,
                    candidate.profile.as_ref(),
                ),
                req.clone(),
                HashMap::new(),
                false,
            )
        };
        let inbound_tools = req
            .get("tools")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let forwarded_tools = payload
            .get("tools")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let inbound_tool_names: Vec<String> = inbound_tools
            .iter()
            .filter_map(tool_trace_name)
            .take(8)
            .collect();
        let forwarded_tool_names: Vec<String> = forwarded_tools
            .iter()
            .filter_map(tool_trace_name)
            .take(8)
            .collect();
        if compat_trace_enabled() {
            trace_io(
                "compat_request_profile",
                json!({
                    "attempt_index": idx,
                    "default_path": default_path,
                    "stream": stream,
                    "codex_mode": codex_mode,
                    "request_format": request_format_kind(&req),
                    "inbound_tool_profile": tool_profile(&inbound_tools),
                    "forwarded_tool_profile": tool_profile(&forwarded_tools),
                    "inbound_tool_choice": req.get("tool_choice").cloned().unwrap_or(Value::Null),
                    "forwarded_tool_choice": payload.get("tool_choice").cloned().unwrap_or(Value::Null),
                    "upstream_path": cfg.path,
                }),
            );
        }
        if trace_io_enabled() {
            trace_io(
                "routing_attempt",
                json!({
                    "attempt_index": idx,
                    "account_id": candidate.account_id,
                    "account_type": candidate.account_type,
                    "default_path": default_path,
                    "stream": stream,
                    "codex_mode": codex_mode,
                    "upstream_base_url": cfg.base_url,
                    "upstream_path": cfg.path,
                    "has_messages": req.get("messages").is_some(),
                    "has_input": req.get("input").is_some(),
                    "inbound_tool_count": inbound_tools.len(),
                    "forwarded_tool_count": forwarded_tools.len(),
                    "inbound_tool_names": inbound_tool_names,
                    "forwarded_tool_names": forwarded_tool_names,
                    "request_headers": request_headers,
                    "payload": payload,
                }),
            );
        }
        push_wire_capture_event(
            state,
            json!({
                "at_unix": now_unix(),
                "phase": "request",
                "attempt_index": idx,
                "account_id": candidate.account_id,
                "account_type": candidate.account_type,
                "default_path": default_path,
                "stream": stream,
                "codex_mode": codex_mode,
                "upstream_base_url": cfg.base_url,
                "upstream_path": cfg.path,
                "payload": diag_truncate(
                    redact_sensitive_text(&payload.to_string()),
                    wire_capture_max_chars()
                )
            }),
        )
        .await;
        let mut attempt_payload = payload.clone();
        let mut retried_orphan_tool_output = false;
        let call: Result<RoutedResponse, GatewayErrorDetail> = loop {
            let out: Result<RoutedResponse, GatewayErrorDetail> = if codex_mode && !stream {
                // Codex always streams; collect the SSE and synthesize a chat completion JSON.
                match forward_openai_sse(
                    &cfg,
                    &candidate.access_token,
                    &attempt_payload,
                    Some(&request_headers),
                )
                .await
                {
                    Ok(v) => {
                        let model_name = parsed.model().unwrap_or("gpt-5.2").to_string();
                        match collect_codex_sse(v.response).await {
                            Ok(projected) => {
                                if is_responses {
                                    Ok(RoutedResponse::Json(responses_json_from_chat_projection(
                                        projected,
                                        &model_name,
                                    )))
                                } else {
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
                                    Ok(RoutedResponse::Json(json!({
                                        "id": chatcmpl_id(),
                                        "object": "chat.completion",
                                        "created": now_unix() as i64,
                                        "model": model_name,
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
                                    })))
                                }
                            }
                            Err(e) => Err(GatewayErrorDetail::new(GatewayError::BadGateway)
                                .with_transport_message(e)),
                        }
                    }
                    Err(e) => Err(e),
                }
            } else if stream {
                forward_openai_sse(
                    &cfg,
                    &candidate.access_token,
                    &attempt_payload,
                    Some(&request_headers),
                )
                .await
                .map(|v| RoutedResponse::Sse(v.response))
            } else {
                forward_openai_json(
                    &cfg,
                    &candidate.access_token,
                    &attempt_payload,
                    Some(&request_headers),
                )
                .await
                .map(|v| RoutedResponse::Json(v.body))
            };
            let Err(detail) = &out else {
                break out;
            };
            if codex_mode
                && !retried_orphan_tool_output
                && is_codex_orphan_tool_output_error(detail)
            {
                let dropped = prune_orphan_function_call_outputs_inplace(&mut attempt_payload);
                if dropped > 0 {
                    retried_orphan_tool_output = true;
                    if trace_io_enabled() {
                        trace_io(
                            "routing_retry_orphan_tool_output",
                            json!({
                                "attempt_index": idx,
                                "account_id": candidate.account_id,
                                "dropped_function_call_outputs": dropped,
                            }),
                        );
                    }
                    push_wire_capture_event(
                        state,
                        json!({
                            "at_unix": now_unix(),
                            "phase": "request_retry",
                            "attempt_index": idx,
                            "account_id": candidate.account_id,
                            "account_type": candidate.account_type,
                            "reason": "codex_orphan_function_call_output",
                            "dropped_function_call_outputs": dropped,
                            "payload": diag_truncate(
                                redact_sensitive_text(&attempt_payload.to_string()),
                                wire_capture_max_chars()
                            ),
                        }),
                    )
                    .await;
                    continue;
                }
            }
            break out;
        };
        match call {
            Ok(resp) => {
                failover_trace.push(FailoverAttemptTrace {
                    attempt_index: failover_trace.len() as u64,
                    account_id: candidate.account_id.clone(),
                    outcome: "success".to_string(),
                    error_kind: None,
                    status: None,
                    cooldown_until_unix: None,
                });
                if trace_io_enabled() {
                    match &resp {
                        RoutedResponse::Json(body) => trace_io(
                            "routing_attempt_success",
                            json!({
                                "attempt_index": idx,
                                "account_id": candidate.account_id,
                                "account_type": candidate.account_type,
                                "response_type": "json",
                                "response_body": body,
                            }),
                        ),
                        RoutedResponse::Sse(upstream) => trace_io(
                            "routing_attempt_success",
                            json!({
                                "attempt_index": idx,
                                "account_id": candidate.account_id,
                                "account_type": candidate.account_type,
                                "response_type": "sse",
                                "upstream_status": upstream.status().as_u16(),
                            }),
                        ),
                    }
                }
                match &resp {
                    RoutedResponse::Json(body) => {
                        push_wire_capture_event(
                            state,
                            json!({
                                "at_unix": now_unix(),
                                "phase": "response",
                                "attempt_index": idx,
                                "account_id": candidate.account_id,
                                "account_type": candidate.account_type,
                                "response_type": "json",
                                "body": diag_truncate(
                                    redact_sensitive_text(&body.to_string()),
                                    wire_capture_max_chars()
                                )
                            }),
                        )
                        .await;
                    }
                    RoutedResponse::Sse(upstream) => {
                        push_wire_capture_event(
                            state,
                            json!({
                                "at_unix": now_unix(),
                                "phase": "response",
                                "attempt_index": idx,
                                "account_id": candidate.account_id,
                                "account_type": candidate.account_type,
                                "response_type": "sse",
                                "upstream_status": upstream.status().as_u16(),
                                "content_type": upstream
                                    .headers()
                                    .get("content-type")
                                    .and_then(|h| h.to_str().ok())
                                    .unwrap_or("text/event-stream"),
                            }),
                        )
                        .await;
                    }
                }
                if candidate.account_type == "oauth" {
                    let _ = manager.mark_success(&candidate.account_id).await;
                }
                let account_id_out = candidate.account_id.clone();
                if failover_hops > 0 {
                    let _ = increment_counter(state, "diag_failovers_total").await;
                }
                let _ = record_last_failover_chain(state, &failover_trace).await;
                return Ok((resp, Some(account_id_out)));
            }
            Err(detail) => {
                let cooldown = apply_failover_cooldown(state, &candidate.account_id, &detail).await;
                failover_trace.push(FailoverAttemptTrace {
                    attempt_index: failover_trace.len() as u64,
                    account_id: candidate.account_id.clone(),
                    outcome: "error".to_string(),
                    error_kind: Some(detail.kind.to_string()),
                    status: detail.status,
                    cooldown_until_unix: cooldown.as_ref().map(|c| c.until_unix),
                });
                let _ = record_last_upstream_error(state, &candidate.account_id, &detail).await;
                if trace_io_enabled() {
                    trace_io(
                        "routing_attempt_error",
                        json!({
                            "attempt_index": idx,
                            "account_id": candidate.account_id,
                            "account_type": candidate.account_type,
                            "error": detail.kind.to_string(),
                            "detail": detail,
                        }),
                    );
                }
                push_wire_capture_event(
                    state,
                    json!({
                        "at_unix": now_unix(),
                        "phase": "error",
                        "attempt_index": idx,
                        "account_id": candidate.account_id,
                        "account_type": candidate.account_type,
                        "error_kind": detail.kind.to_string(),
                        "status": detail.status,
                        "detail": diag_truncate(
                            redact_sensitive_text(&serde_json::to_string(&detail).unwrap_or_default()),
                            wire_capture_max_chars()
                        ),
                    }),
                )
                .await;
                if candidate.account_type == "oauth" {
                    let _ = manager
                        .mark_failure(&candidate.account_id, &format!("{}", detail.kind))
                        .await;
                }
                last_error = detail.kind;
                if idx + 1 < filtered_candidates.len() {
                    failover_hops = failover_hops.saturating_add(1);
                }
            }
        }
    }

    if failover_hops > 0 {
        let _ = increment_counter(state, "diag_failovers_total").await;
    }
    let _ = record_last_failover_chain(state, &failover_trace).await;
    if provider == ANTHROPIC_PROVIDER_ID {
        return route_anthropic_request(state, store, &req).await;
    }
    if provider == GOOGLE_PROVIDER_ID {
        return match route_google_request(state, store, &req).await {
            Ok(v) => Ok(v),
            Err(detail) => Err(detail.kind),
        };
    }
    Err(last_error)
}

#[derive(Debug, Clone)]
pub(crate) struct OpenAiRouteCandidate {
    account_id: String,
    account_type: &'static str,
    access_token: String,
    profile: Option<UpstreamRoutingProfile>,
    codex_mode: bool,
}

#[cfg(test)]
mod retry_guard_tests {
    use super::{is_codex_orphan_tool_output_error, prune_orphan_function_call_outputs_inplace};
    use llm_gateway::{GatewayError, GatewayErrorDetail};
    use serde_json::json;

    #[test]
    fn codex_orphan_tool_output_error_detection_is_precise() {
        let detail = GatewayErrorDetail::new(GatewayError::BadGateway)
            .with_status(400)
            .with_body_snippet(
                "{\"error\":{\"message\":\"No tool call found for function call output with call_id call_1.\"}}"
                    .to_string(),
            );
        assert!(is_codex_orphan_tool_output_error(&detail));

        let not_match = GatewayErrorDetail::new(GatewayError::BadGateway)
            .with_status(400)
            .with_body_snippet(
                "{\"error\":{\"message\":\"different validation error\"}}".to_string(),
            );
        assert!(!is_codex_orphan_tool_output_error(&not_match));
    }

    #[test]
    fn prune_orphan_function_call_outputs_keeps_only_linked_outputs() {
        let mut payload = json!({
            "input": [
                {"type":"function_call_output","call_id":"call_orphan","output":"x"},
                {"type":"function_call","call_id":"call_ok","name":"exec_command","arguments":"{}"},
                {"type":"function_call_output","call_id":"call_ok","output":"ok"},
                {"role":"user","content":[{"type":"input_text","text":"hello"}]}
            ]
        });
        let dropped = prune_orphan_function_call_outputs_inplace(&mut payload);
        assert_eq!(dropped, 1);
        let input = payload
            .get("input")
            .and_then(|v| v.as_array())
            .expect("input array");
        assert!(input.iter().any(|item| {
            item.get("type").and_then(|v| v.as_str()) == Some("function_call_output")
                && item.get("call_id").and_then(|v| v.as_str()) == Some("call_ok")
        }));
        assert!(!input.iter().any(|item| {
            item.get("type").and_then(|v| v.as_str()) == Some("function_call_output")
                && item.get("call_id").and_then(|v| v.as_str()) == Some("call_orphan")
        }));
    }
}
