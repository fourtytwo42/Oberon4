use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use llm_gateway::GatewayErrorDetail;
use llm_storage::{RemoteAssignment, SqliteStore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use tokio::time::{sleep, Duration};

use crate::model_resolution::{
    chat_content_to_text, normalize_model_name_for_routing, resolve_provider_for_model,
};
use crate::routing::OAUTH_FAILOVER_COOLDOWNS_KEY;

use super::{
    cached_models_key, delete_custom_api_key, error_response, is_admin_authorized,
    list_custom_api_accounts, missing_store, now_unix, redact_sensitive_text,
    save_custom_api_accounts, unauthorized_admin, AppState, ErrorCode, OPENAI_API_PROVIDER_ID,
};

pub(crate) const DIAG_LAST_FAILOVER_CHAIN_KEY: &str = "diag_last_failover_chain_json";
const DIAG_WIRE_CAPTURE_OVERRIDE_KEY: &str = "diag_wire_capture_enabled_override";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ModelUsageSnapshot {
    pub(crate) total_requests: u64,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) cached_tokens: u64,
    pub(crate) last_seen_unix: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct KeyModelUsageEntry {
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) cached_tokens: u64,
    pub(crate) request_count: u64,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct DiagUsageBuffer {
    model_usage: HashMap<String, ModelUsageSnapshot>,
    key_usage: HashMap<String, HashMap<String, KeyModelUsageEntry>>,
    oauth_usage: HashMap<String, HashMap<String, KeyModelUsageEntry>>,
}

fn diag_usage_flush_ms() -> u64 {
    std::env::var("LLMHOTSPOT_DIAG_USAGE_FLUSH_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v >= 500)
        .unwrap_or(5_000)
}

pub(crate) fn ensure_diag_usage_flusher(state: &AppState) {
    if state
        .diag_usage_flush_started
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    let state_clone = state.clone();
    let shutdown_state = state.clone();
    tokio::spawn(async move {
        let flush_every = Duration::from_millis(diag_usage_flush_ms());
        loop {
            sleep(flush_every).await;
            let _ = flush_diag_usage_now(&state_clone).await;
        }
    });
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = flush_diag_usage_now(&shutdown_state).await;
    });
}

pub(crate) async fn flush_diag_usage_now(state: &AppState) -> Result<(), String> {
    let Some(store) = &state.store else {
        return Ok(());
    };
    let pending = {
        let mut guard = state.diag_usage_buffer.lock().await;
        if guard.model_usage.is_empty()
            && guard.key_usage.is_empty()
            && guard.oauth_usage.is_empty()
        {
            return Ok(());
        }
        std::mem::take(&mut *guard)
    };

    if !pending.model_usage.is_empty() {
        let raw = store
            .get_app_state("diag_model_usage_json")
            .await
            .unwrap_or_default();
        let mut usage: HashMap<String, ModelUsageSnapshot> =
            serde_json::from_str(&raw).unwrap_or_default();
        for (model, delta) in pending.model_usage {
            let entry = usage.entry(model).or_default();
            entry.total_requests = entry.total_requests.saturating_add(delta.total_requests);
            entry.input_tokens = entry.input_tokens.saturating_add(delta.input_tokens);
            entry.output_tokens = entry.output_tokens.saturating_add(delta.output_tokens);
            entry.cached_tokens = entry.cached_tokens.saturating_add(delta.cached_tokens);
            entry.last_seen_unix = entry.last_seen_unix.max(delta.last_seen_unix);
        }
        let encoded = serde_json::to_string(&usage).map_err(|e| e.to_string())?;
        store
            .set_app_state("diag_model_usage_json", &encoded)
            .await
            .map_err(|e| e.to_string())?;
    }

    if !pending.key_usage.is_empty() {
        let raw = store
            .get_app_state("diag_key_usage_json")
            .await
            .unwrap_or_default();
        let mut usage: HashMap<String, HashMap<String, KeyModelUsageEntry>> =
            serde_json::from_str(&raw).unwrap_or_default();
        merge_usage_delta(&mut usage, pending.key_usage);
        let encoded = serde_json::to_string(&usage).map_err(|e| e.to_string())?;
        store
            .set_app_state("diag_key_usage_json", &encoded)
            .await
            .map_err(|e| e.to_string())?;
    }

    if !pending.oauth_usage.is_empty() {
        let raw = store
            .get_app_state("diag_oauth_usage_json")
            .await
            .unwrap_or_default();
        let mut usage: HashMap<String, HashMap<String, KeyModelUsageEntry>> =
            serde_json::from_str(&raw).unwrap_or_default();
        merge_usage_delta(&mut usage, pending.oauth_usage);
        let encoded = serde_json::to_string(&usage).map_err(|e| e.to_string())?;
        store
            .set_app_state("diag_oauth_usage_json", &encoded)
            .await
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

fn merge_usage_delta(
    current: &mut HashMap<String, HashMap<String, KeyModelUsageEntry>>,
    delta: HashMap<String, HashMap<String, KeyModelUsageEntry>>,
) {
    for (id, models) in delta {
        let current_models = current.entry(id).or_default();
        for (model, change) in models {
            let entry = current_models.entry(model).or_default();
            entry.input_tokens = entry.input_tokens.saturating_add(change.input_tokens);
            entry.output_tokens = entry.output_tokens.saturating_add(change.output_tokens);
            entry.cached_tokens = entry.cached_tokens.saturating_add(change.cached_tokens);
            entry.request_count = entry.request_count.saturating_add(change.request_count);
        }
    }
}

pub(crate) async fn diagnostics(State(state): State<AppState>) -> impl IntoResponse {
    let _ = flush_diag_usage_now(&state).await;
    let wire_enabled = wire_capture_enabled_for_state(&state).await;
    let Some(store) = state.store else {
        return (
            StatusCode::OK,
            Json(json!({
                "requests": 0,
                "failover_events": 0,
                "recent_errors": [],
                "last_bearer_auth_rejection": null,
                "last_bearer_auth_rejection_unix": null,
                "model_usage": {},
                "oauth_usage": {},
                "wire_capture_enabled": wire_enabled,
                "runtime_ipc": {
                    "protocol_version": 1,
                    "server_requests_total": 0,
                    "server_errors_total": 0,
                    "shell_ipc_hits_total": 0,
                    "shell_fallback_total": 0,
                    "shell_fail_total": 0,
                    "pending_shell_deltas": {
                        "hits": 0,
                        "fallbacks": 0,
                        "fails": 0,
                        "updated_unix": null
                    }
                },
                "doctor_last_run": null,
                "last_failover_chain": []
            })),
        )
            .into_response();
    };

    let requests = store
        .get_app_state("diag_requests_total")
        .await
        .unwrap_or_default()
        .parse::<u64>()
        .unwrap_or(0);
    let failovers = store
        .get_app_state("diag_failovers_total")
        .await
        .unwrap_or_default()
        .parse::<u64>()
        .unwrap_or(0);
    let errors = store
        .get_app_state("diag_recent_errors_json")
        .await
        .unwrap_or_default();
    let last_bearer_auth_rejection = store
        .get_app_state("diag_last_bearer_auth_rejection")
        .await
        .unwrap_or_default();
    let last_bearer_auth_rejection_unix = store
        .get_app_state("diag_last_bearer_auth_rejection_unix")
        .await
        .unwrap_or_default()
        .parse::<u64>()
        .ok();

    let recent_errors: Vec<String> = serde_json::from_str(&errors).unwrap_or_default();
    let model_usage = store
        .get_app_state("diag_model_usage_json")
        .await
        .unwrap_or_default();
    let model_usage: Value = serde_json::from_str(&model_usage).unwrap_or_else(|_| json!({}));

    let key_usage = store
        .get_app_state("diag_key_usage_json")
        .await
        .unwrap_or_default();
    let key_usage: Value = serde_json::from_str(&key_usage).unwrap_or_else(|_| json!({}));

    let oauth_usage = store
        .get_app_state("diag_oauth_usage_json")
        .await
        .unwrap_or_default();
    let oauth_usage: Value = serde_json::from_str(&oauth_usage).unwrap_or_else(|_| json!({}));

    let key_usage_baseline = store
        .get_app_state("diag_key_usage_baseline_json")
        .await
        .unwrap_or_default();
    let key_usage_baseline: Value =
        serde_json::from_str(&key_usage_baseline).unwrap_or_else(|_| json!({}));

    let oauth_usage_baseline = store
        .get_app_state("diag_oauth_usage_baseline_json")
        .await
        .unwrap_or_default();
    let oauth_usage_baseline: Value =
        serde_json::from_str(&oauth_usage_baseline).unwrap_or_else(|_| json!({}));

    let model_usage_baseline = store
        .get_app_state("diag_model_usage_baseline_json")
        .await
        .unwrap_or_default();
    let model_usage_baseline: Value =
        serde_json::from_str(&model_usage_baseline).unwrap_or_else(|_| json!({}));

    let last_upstream_error = store
        .get_app_state("diag_last_upstream_error_json")
        .await
        .unwrap_or_default();
    let last_upstream_error: Value =
        serde_json::from_str(&last_upstream_error).unwrap_or(Value::Null);
    let last_failover_chain = store
        .get_app_state(DIAG_LAST_FAILOVER_CHAIN_KEY)
        .await
        .unwrap_or_default();
    let last_failover_chain: Value =
        serde_json::from_str(&last_failover_chain).unwrap_or_else(|_| json!([]));

    let recent_requests_raw = store
        .get_app_state("diag_recent_requests_json")
        .await
        .unwrap_or_default();
    let recent_requests: Value =
        serde_json::from_str(&recent_requests_raw).unwrap_or_else(|_| json!([]));
    let runtime_ipc_server_requests_total = store
        .get_app_state("runtime_ipc_server_requests_total")
        .await
        .unwrap_or_default()
        .parse::<u64>()
        .unwrap_or(0);
    let runtime_ipc_server_errors_total = store
        .get_app_state("runtime_ipc_server_errors_total")
        .await
        .unwrap_or_default()
        .parse::<u64>()
        .unwrap_or(0);
    let runtime_ipc_shell_hits_total = store
        .get_app_state("runtime_ipc_shell_hits_total")
        .await
        .unwrap_or_default()
        .parse::<u64>()
        .unwrap_or(0);
    let runtime_ipc_shell_fallback_total = store
        .get_app_state("runtime_ipc_shell_fallback_total")
        .await
        .unwrap_or_default()
        .parse::<u64>()
        .unwrap_or(0);
    let runtime_ipc_shell_fail_total = store
        .get_app_state("runtime_ipc_shell_fail_total")
        .await
        .unwrap_or_default()
        .parse::<u64>()
        .unwrap_or(0);
    let runtime_ipc_pending_raw = store
        .get_app_state("runtime_ipc_shell_pending_json")
        .await
        .unwrap_or_default();
    let runtime_ipc_pending: Value = if runtime_ipc_pending_raw.trim().is_empty() {
        json!({
            "hits": 0,
            "fallbacks": 0,
            "fails": 0,
            "updated_unix": null
        })
    } else {
        serde_json::from_str(&runtime_ipc_pending_raw).unwrap_or_else(|_| {
            json!({
                "hits": 0,
                "fallbacks": 0,
                "fails": 0,
                "updated_unix": null
            })
        })
    };
    let doctor_last_run_raw = store
        .get_app_state("doctor_last_run_json")
        .await
        .unwrap_or_default();
    let doctor_last_run: Value = if doctor_last_run_raw.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&doctor_last_run_raw).unwrap_or(Value::Null)
    };

    (
        StatusCode::OK,
        Json(json!({
            "requests": requests,
            "failover_events": failovers,
            "recent_errors": recent_errors,
            "last_bearer_auth_rejection": if last_bearer_auth_rejection.is_empty() { Value::Null } else { Value::String(last_bearer_auth_rejection) },
            "last_bearer_auth_rejection_unix": last_bearer_auth_rejection_unix,
            "model_usage": model_usage,
            "key_usage": key_usage,
            "oauth_usage": oauth_usage,
            "key_usage_baseline": key_usage_baseline,
            "oauth_usage_baseline": oauth_usage_baseline,
            "model_usage_baseline": model_usage_baseline,
            "wire_capture_enabled": wire_enabled,
            "runtime_ipc": {
                "protocol_version": 1,
                "server_requests_total": runtime_ipc_server_requests_total,
                "server_errors_total": runtime_ipc_server_errors_total,
                "shell_ipc_hits_total": runtime_ipc_shell_hits_total,
                "shell_fallback_total": runtime_ipc_shell_fallback_total,
                "shell_fail_total": runtime_ipc_shell_fail_total,
                "pending_shell_deltas": runtime_ipc_pending
            },
            "doctor_last_run": doctor_last_run,
            "last_upstream_error": last_upstream_error,
            "last_failover_chain": last_failover_chain,
            "recent_requests": recent_requests,
        })),
    )
        .into_response()
}

pub(crate) async fn clear_diagnostics(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let _ = flush_diag_usage_now(&state).await;
    let Some(store) = &state.store else {
        return missing_store();
    };
    if !is_admin_authorized(&headers, store).await {
        return unauthorized_admin();
    }

    let writes = [
        ("diag_requests_total", "0"),
        ("diag_failovers_total", "0"),
        ("diag_recent_errors_json", "[]"),
        ("diag_recent_requests_json", "[]"),
        ("diag_last_bearer_auth_rejection", ""),
        ("diag_last_bearer_auth_rejection_unix", ""),
        ("diag_last_upstream_error_json", ""),
        ("runtime_ipc_server_requests_total", "0"),
        ("runtime_ipc_server_errors_total", "0"),
        ("runtime_ipc_shell_hits_total", "0"),
        ("runtime_ipc_shell_fallback_total", "0"),
        ("runtime_ipc_shell_fail_total", "0"),
        ("runtime_ipc_shell_pending_json", ""),
        ("doctor_last_run_json", ""),
        ("doctor_last_error", ""),
        (DIAG_LAST_FAILOVER_CHAIN_KEY, "[]"),
        (OAUTH_FAILOVER_COOLDOWNS_KEY, "{}"),
    ];
    for (k, v) in writes {
        if let Err(e) = store.set_app_state(k, v).await {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                &format!("storage error: {e}"),
            );
        }
    }
    {
        let mut guard = state.diag_usage_buffer.lock().await;
        *guard = DiagUsageBuffer::default();
    }
    (StatusCode::OK, Json(json!({"ok": true}))).into_response()
}

pub(crate) async fn diagnostics_wire(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let Some(store) = &state.store else {
        return missing_store();
    };
    if !is_admin_authorized(&headers, store).await {
        return unauthorized_admin();
    }
    let events = {
        let guard = state.wire_capture.lock().await;
        guard.iter().cloned().collect::<Vec<Value>>()
    };
    let enabled = wire_capture_enabled_for_state(&state).await;
    (
        StatusCode::OK,
        Json(json!({
            "enabled": enabled,
            "events": events
        })),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
pub(crate) struct ToggleWireCaptureRequest {
    enabled: bool,
}

pub(crate) async fn toggle_diagnostics_wire(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ToggleWireCaptureRequest>,
) -> impl IntoResponse {
    let Some(store) = &state.store else {
        return missing_store();
    };
    if !is_admin_authorized(&headers, store).await {
        return unauthorized_admin();
    }
    let value = if req.enabled { "1" } else { "0" };
    if let Err(e) = store
        .set_app_state(DIAG_WIRE_CAPTURE_OVERRIDE_KEY, value)
        .await
    {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            &format!("storage error: {e}"),
        );
    }
    state
        .wire_capture_override_enabled
        .store(req.enabled, Ordering::SeqCst);
    state
        .wire_capture_override_set
        .store(true, Ordering::SeqCst);
    state
        .wire_capture_override_loaded
        .store(true, Ordering::SeqCst);
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "enabled": req.enabled
        })),
    )
        .into_response()
}

pub(crate) async fn clear_diagnostics_wire(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let Some(store) = &state.store else {
        return missing_store();
    };
    if !is_admin_authorized(&headers, store).await {
        return unauthorized_admin();
    }
    {
        let mut guard = state.wire_capture.lock().await;
        guard.clear();
    }
    (StatusCode::OK, Json(json!({"ok": true}))).into_response()
}

pub(crate) async fn export_diagnostics(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let _ = flush_diag_usage_now(&state).await;
    let Some(store) = &state.store else {
        return missing_store();
    };
    if !is_admin_authorized(&headers, store).await {
        return unauthorized_admin();
    }

    // Gather all diagnostic data
    let requests: u64 = store
        .get_app_state("diag_requests_total")
        .await
        .unwrap_or_default()
        .parse()
        .unwrap_or(0);
    let failovers: u64 = store
        .get_app_state("diag_failovers_total")
        .await
        .unwrap_or_default()
        .parse()
        .unwrap_or(0);
    let errors_raw = store
        .get_app_state("diag_recent_errors_json")
        .await
        .unwrap_or_default();
    let errors: Vec<String> = serde_json::from_str(&errors_raw).unwrap_or_default();
    let model_usage_raw = store
        .get_app_state("diag_model_usage_json")
        .await
        .unwrap_or_default();
    let model_usage: HashMap<String, ModelUsageSnapshot> =
        serde_json::from_str(&model_usage_raw).unwrap_or_default();
    let model_baseline_raw = store
        .get_app_state("diag_model_usage_baseline_json")
        .await
        .unwrap_or_default();
    let model_baseline: HashMap<String, ModelUsageSnapshot> =
        serde_json::from_str(&model_baseline_raw).unwrap_or_default();
    let key_usage_raw = store
        .get_app_state("diag_key_usage_json")
        .await
        .unwrap_or_default();
    let key_usage: HashMap<String, HashMap<String, KeyModelUsageEntry>> =
        serde_json::from_str(&key_usage_raw).unwrap_or_default();
    let key_baseline_raw = store
        .get_app_state("diag_key_usage_baseline_json")
        .await
        .unwrap_or_default();
    let key_baseline: HashMap<String, HashMap<String, KeyModelUsageEntry>> =
        serde_json::from_str(&key_baseline_raw).unwrap_or_default();
    let oauth_usage_raw = store
        .get_app_state("diag_oauth_usage_json")
        .await
        .unwrap_or_default();
    let oauth_usage: HashMap<String, HashMap<String, KeyModelUsageEntry>> =
        serde_json::from_str(&oauth_usage_raw).unwrap_or_default();
    let oauth_baseline_raw = store
        .get_app_state("diag_oauth_usage_baseline_json")
        .await
        .unwrap_or_default();
    let oauth_baseline: HashMap<String, HashMap<String, KeyModelUsageEntry>> =
        serde_json::from_str(&oauth_baseline_raw).unwrap_or_default();
    let upstream_err_raw = store
        .get_app_state("diag_last_upstream_error_json")
        .await
        .unwrap_or_default();
    let upstream_err: Value = serde_json::from_str(&upstream_err_raw).unwrap_or(Value::Null);
    let chain_raw = store
        .get_app_state(DIAG_LAST_FAILOVER_CHAIN_KEY)
        .await
        .unwrap_or_default();
    let chain: Vec<Value> = serde_json::from_str(&chain_raw).unwrap_or_default();
    let recent_raw = store
        .get_app_state("diag_recent_requests_json")
        .await
        .unwrap_or_default();
    let recent: Vec<Value> = serde_json::from_str(&recent_raw).unwrap_or_default();
    let last_rejection = store
        .get_app_state("diag_last_bearer_auth_rejection")
        .await
        .unwrap_or_default();
    let last_rejection_unix: u64 = store
        .get_app_state("diag_last_bearer_auth_rejection_unix")
        .await
        .unwrap_or_default()
        .parse()
        .unwrap_or(0);
    let doctor_last_run_raw = store
        .get_app_state("doctor_last_run_json")
        .await
        .unwrap_or_default();
    let doctor_last_run: Value = if doctor_last_run_raw.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&doctor_last_run_raw).unwrap_or(Value::Null)
    };

    // OAuth account labels for human-readable names
    let accounts = store.list_oauth_accounts().await.unwrap_or_default();
    let account_labels: HashMap<String, String> = accounts
        .iter()
        .map(|a| {
            (
                a.account_id.clone(),
                if !a.display_name.is_empty() {
                    a.display_name.clone()
                } else {
                    a.account_id[..8.min(a.account_id.len())].to_string()
                },
            )
        })
        .collect();

    let now = now_unix();
    let now_str = {
        let secs = now;
        let mins = secs / 60 % 60;
        let hours = secs / 3600 % 24;
        let days = secs / 86400;
        format!("unix:{} (~day {} {:02}:{:02} UTC)", secs, days, hours, mins)
    };

    fn fmt_tokens(n: u64) -> String {
        if n >= 1_000_000 {
            format!("{:.1}M", n as f64 / 1_000_000.0)
        } else if n >= 1_000 {
            format!("{:.1}K", n as f64 / 1_000.0)
        } else {
            n.to_string()
        }
    }

    let mut md = String::new();
    md.push_str("# LLM Hotspot Diagnostics Export\n\n");
    md.push_str(&format!("**Generated:** {now_str}  \n"));
    md.push_str(&format!("**Version:** v{}\n\n", env!("CARGO_PKG_VERSION")));

    md.push_str("## Summary\n\n");
    md.push_str("| Metric | Value |\n|--------|-------|\n");
    md.push_str(&format!("| Total requests | {} |\n", requests));
    md.push_str(&format!("| Failover events | {} |\n", failovers));
    md.push_str(&format!(
        "| Last auth rejection | {} |\n",
        if last_rejection.is_empty() {
            "none".to_string()
        } else {
            format!("{} (unix:{})", last_rejection, last_rejection_unix)
        }
    ));
    if let Some(run_unix) = doctor_last_run.get("run_unix").and_then(Value::as_u64) {
        let ok = doctor_last_run
            .get("ok")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        md.push_str(&format!(
            "| Last doctor run | unix:{} ({}) |\n",
            run_unix,
            if ok { "ok" } else { "failed" }
        ));
    } else {
        md.push_str("| Last doctor run | none |\n");
    }
    md.push('\n');

    md.push_str("## Recent Errors\n\n");
    if errors.is_empty() {
        md.push_str("_No recent errors_\n\n");
    } else {
        for e in &errors {
            md.push_str(&format!("- {}\n", e));
        }
        md.push('\n');
    }

    md.push_str("## Last Upstream Error\n\n");
    if upstream_err.is_null() || upstream_err == Value::Null {
        md.push_str("_None_\n\n");
    } else {
        md.push_str("```json\n");
        md.push_str(&serde_json::to_string_pretty(&upstream_err).unwrap_or_default());
        md.push_str("\n```\n\n");
    }

    md.push_str("## Last Failover Chain\n\n");
    if chain.is_empty() {
        md.push_str("_No failover events recorded_\n\n");
    } else {
        md.push_str("| # | Account | Outcome | Status | Cooldown |\n|---|---------|---------|--------|----------|\n");
        for a in &chain {
            let idx = a
                .get("attempt_index")
                .and_then(Value::as_u64)
                .map(|n| n.to_string())
                .unwrap_or_default();
            let acct = a.get("account_id").and_then(Value::as_str).unwrap_or("?");
            let label = account_labels
                .get(acct)
                .cloned()
                .unwrap_or_else(|| acct[..8.min(acct.len())].to_string());
            let outcome = a.get("outcome").and_then(Value::as_str).unwrap_or("?");
            let status = a
                .get("status")
                .and_then(Value::as_u64)
                .map(|s| s.to_string())
                .unwrap_or_else(|| "-".to_string());
            let cooldown = a
                .get("cooldown_until_unix")
                .and_then(Value::as_u64)
                .map(|t| format!("until unix:{}", t))
                .unwrap_or_else(|| "-".to_string());
            md.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                idx, label, outcome, status, cooldown
            ));
        }
        md.push('\n');
    }

    md.push_str("## Usage - Per Model (since last reset)\n\n");
    if model_usage.is_empty() {
        md.push_str("_No model usage recorded_\n\n");
    } else {
        md.push_str("| Model | Requests | Input | Output | Cached |\n|-------|----------|-------|--------|--------|\n");
        let mut models: Vec<_> = model_usage.keys().collect();
        models.sort();
        for m in models {
            let e = &model_usage[m];
            let b = model_baseline.get(m);
            let reqs = e
                .total_requests
                .saturating_sub(b.map(|x| x.total_requests).unwrap_or(0));
            let inp = e
                .input_tokens
                .saturating_sub(b.map(|x| x.input_tokens).unwrap_or(0));
            let out = e
                .output_tokens
                .saturating_sub(b.map(|x| x.output_tokens).unwrap_or(0));
            let cac = e
                .cached_tokens
                .saturating_sub(b.map(|x| x.cached_tokens).unwrap_or(0));
            md.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                m,
                reqs,
                fmt_tokens(inp),
                fmt_tokens(out),
                fmt_tokens(cac)
            ));
        }
        md.push('\n');
    }

    md.push_str("## Usage - Per API Key (since last reset)\n\n");
    if key_usage.is_empty() {
        md.push_str("_No key usage recorded_\n\n");
    } else {
        let keys_list = store.list_api_keys().await.unwrap_or_default();
        let key_labels: HashMap<String, String> = keys_list
            .iter()
            .map(|k| {
                (
                    k.key_id.clone(),
                    if !k.label.is_empty() {
                        k.label.clone()
                    } else {
                        k.key_id[..8.min(k.key_id.len())].to_string()
                    },
                )
            })
            .collect();
        let mut key_ids: Vec<_> = key_usage.keys().collect();
        key_ids.sort();
        for kid in key_ids {
            let label = key_labels
                .get(kid)
                .cloned()
                .unwrap_or_else(|| kid[..8.min(kid.len())].to_string());
            md.push_str(&format!("### Key: {} (`{}`)\n\n", label, kid));
            md.push_str("| Model | Requests | Input | Output | Cached |\n|-------|----------|-------|--------|--------|\n");
            let models = &key_usage[kid];
            let baseline_models = key_baseline.get(kid);
            let mut model_names: Vec<_> = models.keys().collect();
            model_names.sort();
            for m in model_names {
                let e = &models[m];
                let b = baseline_models.and_then(|bm| bm.get(m));
                let reqs = e
                    .request_count
                    .saturating_sub(b.map(|x| x.request_count).unwrap_or(0));
                let inp = e
                    .input_tokens
                    .saturating_sub(b.map(|x| x.input_tokens).unwrap_or(0));
                let out = e
                    .output_tokens
                    .saturating_sub(b.map(|x| x.output_tokens).unwrap_or(0));
                let cac = e
                    .cached_tokens
                    .saturating_sub(b.map(|x| x.cached_tokens).unwrap_or(0));
                md.push_str(&format!(
                    "| {} | {} | {} | {} | {} |\n",
                    m,
                    reqs,
                    fmt_tokens(inp),
                    fmt_tokens(out),
                    fmt_tokens(cac)
                ));
            }
            md.push('\n');
        }
    }

    md.push_str("## Usage - Per OAuth Account (since last reset)\n\n");
    if oauth_usage.is_empty() {
        md.push_str("_No OAuth usage recorded_\n\n");
    } else {
        let mut acct_ids: Vec<_> = oauth_usage.keys().collect();
        acct_ids.sort();
        for aid in acct_ids {
            let label = account_labels
                .get(aid)
                .cloned()
                .unwrap_or_else(|| aid[..8.min(aid.len())].to_string());
            md.push_str(&format!("### Account: {} (`{}`)\n\n", label, aid));
            md.push_str("| Model | Requests | Input | Output | Cached |\n|-------|----------|-------|--------|--------|\n");
            let models = &oauth_usage[aid];
            let baseline_models = oauth_baseline.get(aid);
            let mut model_names: Vec<_> = models.keys().collect();
            model_names.sort();
            for m in model_names {
                let e = &models[m];
                let b = baseline_models.and_then(|bm| bm.get(m));
                let reqs = e
                    .request_count
                    .saturating_sub(b.map(|x| x.request_count).unwrap_or(0));
                let inp = e
                    .input_tokens
                    .saturating_sub(b.map(|x| x.input_tokens).unwrap_or(0));
                let out = e
                    .output_tokens
                    .saturating_sub(b.map(|x| x.output_tokens).unwrap_or(0));
                let cac = e
                    .cached_tokens
                    .saturating_sub(b.map(|x| x.cached_tokens).unwrap_or(0));
                md.push_str(&format!(
                    "| {} | {} | {} | {} | {} |\n",
                    m,
                    reqs,
                    fmt_tokens(inp),
                    fmt_tokens(out),
                    fmt_tokens(cac)
                ));
            }
            md.push('\n');
        }
    }

    md.push_str("## Recent Requests (last 10, newest first)\n\n");
    if recent.is_empty() {
        md.push_str("_No requests recorded_\n\n");
    } else {
        for r in recent.iter().rev() {
            let t = r
                .get("at_unix")
                .and_then(Value::as_u64)
                .map(|u| format!("unix:{}", u))
                .unwrap_or_default();
            let model = r.get("model").and_then(Value::as_str).unwrap_or("?");
            let endpoint = r.get("endpoint").and_then(Value::as_str).unwrap_or("?");
            let msg_count = r.get("msg_count").and_then(Value::as_u64).unwrap_or(0);
            let stream = r.get("stream").and_then(Value::as_bool).unwrap_or(false);
            let tools: Vec<String> = r
                .get("tools")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let tool_choice = r.get("tool_choice").cloned().unwrap_or(Value::Null);
            let last_msg_len = r
                .get("last_user_msg_len")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let roles = r.get("message_roles").and_then(Value::as_str).unwrap_or("");
            let last_role = r.get("last_role").and_then(Value::as_str).unwrap_or("");
            let last_has_tr = r
                .get("last_has_tool_result_block")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            md.push_str(&format!(
                "**{}** `{}` -> `{}` | {} msgs | stream={}\n",
                t, endpoint, model, msg_count, stream
            ));
            if !tools.is_empty() {
                md.push_str(&format!(
                    "- Tools ({}):** {}\n",
                    tools.len(),
                    tools.join(", ")
                ));
            }
            if !tool_choice.is_null() {
                md.push_str(&format!("- tool_choice: `{}`\n", tool_choice));
            }
            if !roles.is_empty() {
                md.push_str(&format!("- Roles: `{}`\n", roles));
                if !last_role.is_empty() {
                    md.push_str(&format!(
                        "- Last role: `{}` (anthropic tool_result block on last msg: `{}`)\n",
                        last_role, last_has_tr
                    ));
                }
            }
            if last_msg_len > 0 {
                md.push_str(&format!(
                    "- Last user msg length: `{}` chars\n",
                    last_msg_len
                ));
            }
            md.push('\n');
        }
    }

    let filename = format!("llmhotspot-diag-{}.md", now);
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/markdown; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                &format!("attachment; filename=\"{filename}\""),
            ),
        ],
        md,
    )
        .into_response()
}

pub(crate) async fn factory_reset(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let Some(store) = &state.store else {
        return missing_store();
    };
    if !is_admin_authorized(&headers, store).await {
        return unauthorized_admin();
    }

    let accounts = match store.list_oauth_accounts().await {
        Ok(v) => v,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                &format!("storage error: {e}"),
            )
        }
    };
    for a in accounts {
        if let Err(e) = store.delete_oauth_account(&a.account_id).await {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                &format!("storage error: {e}"),
            );
        }
        let _ = store.delete_token(&state.workspace_id, &a.account_id).await;
    }
    if let Ok(custom_accounts) = list_custom_api_accounts(store).await {
        for a in &custom_accounts {
            let _ = delete_custom_api_key(store, &a.account_id).await;
            let _ = store
                .delete_app_state(&cached_models_key(OPENAI_API_PROVIDER_ID, &a.account_id))
                .await;
        }
    }
    let _ = save_custom_api_accounts(store, &[]).await;

    let keys = match store.list_api_keys().await {
        Ok(v) => v,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                &format!("storage error: {e}"),
            )
        }
    };
    for k in keys {
        if let Err(e) = store.delete_api_key(&k.key_id).await {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                &format!("storage error: {e}"),
            );
        }
        let _ = store
            .delete_app_state(&format!("key_raw:{}", k.key_id))
            .await;
        let _ =
            super::delete_api_key_raw_with_fallback(store, &state.workspace_id, &k.key_id).await;
    }

    let clear_app_state = [
        ("diag_requests_total", "0"),
        ("diag_failovers_total", "0"),
        ("diag_recent_errors_json", "[]"),
        ("diag_recent_requests_json", "[]"),
        ("diag_last_bearer_auth_rejection", ""),
        ("diag_last_bearer_auth_rejection_unix", ""),
        ("diag_model_usage_json", "{}"),
        ("diag_key_usage_json", "{}"),
        ("diag_oauth_usage_json", "{}"),
        ("diag_key_usage_baseline_json", "{}"),
        ("diag_oauth_usage_baseline_json", "{}"),
        ("diag_model_usage_baseline_json", "{}"),
        ("diag_last_upstream_error_json", ""),
        (DIAG_LAST_FAILOVER_CHAIN_KEY, "[]"),
        (OAUTH_FAILOVER_COOLDOWNS_KEY, "{}"),
        ("remote_base_url", ""),
        ("remote_console_url", ""),
        ("remote_assignment_lease_expires_unix", "0"),
        ("remote_assignment_last_error", ""),
        ("remote_assignment_last_refresh_unix", "0"),
        ("remote_assignment_force_refresh", "1"),
    ];
    for (k, v) in clear_app_state {
        if let Err(e) = store.set_app_state(k, v).await {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                &format!("storage error: {e}"),
            );
        }
    }
    {
        let mut guard = state.diag_usage_buffer.lock().await;
        *guard = DiagUsageBuffer::default();
    }

    let _ = store
        .set_remote_assignment(&RemoteAssignment {
            remote_base_url: None,
            remote_console_url: None,
        })
        .await;

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "message": "factory reset complete",
            "remote_refresh_triggered": true
        })),
    )
        .into_response()
}

pub(crate) fn wire_capture_enabled() -> bool {
    match std::env::var("LLMHOTSPOT_WIRE_CAPTURE") {
        Ok(v) => {
            let n = v.trim().to_ascii_lowercase();
            n == "1" || n == "true" || n == "yes" || n == "on"
        }
        Err(_) => false,
    }
}

pub(crate) async fn wire_capture_enabled_for_state(state: &AppState) -> bool {
    if !state.wire_capture_override_loaded.load(Ordering::SeqCst)
        && state
            .wire_capture_override_loaded
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    {
        if let Some(store) = &state.store {
            let raw = tokio::time::timeout(
                Duration::from_millis(150),
                store.get_app_state(DIAG_WIRE_CAPTURE_OVERRIDE_KEY),
            )
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
            let n = raw.trim().to_ascii_lowercase();
            if n == "1" || n == "true" || n == "yes" || n == "on" {
                state
                    .wire_capture_override_set
                    .store(true, Ordering::SeqCst);
                state
                    .wire_capture_override_enabled
                    .store(true, Ordering::SeqCst);
            } else if n == "0" || n == "false" || n == "no" || n == "off" {
                state
                    .wire_capture_override_set
                    .store(true, Ordering::SeqCst);
                state
                    .wire_capture_override_enabled
                    .store(false, Ordering::SeqCst);
            }
        }
    }
    if state.wire_capture_override_set.load(Ordering::SeqCst) {
        return state.wire_capture_override_enabled.load(Ordering::SeqCst);
    }
    wire_capture_enabled()
}

fn wire_capture_max_entries() -> usize {
    std::env::var("LLMHOTSPOT_WIRE_CAPTURE_MAX_ENTRIES")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v >= 20)
        .unwrap_or(300)
}

pub(crate) fn wire_capture_max_chars() -> usize {
    std::env::var("LLMHOTSPOT_WIRE_CAPTURE_MAX_CHARS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v >= 512)
        .unwrap_or(8_000)
}

pub(crate) fn diag_truncate(mut s: String, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s;
    }
    let truncated: String = s.chars().take(max_chars).collect();
    s.clear();
    format!("{truncated}...[truncated]")
}

pub(crate) async fn push_wire_capture_event(state: &AppState, event: Value) {
    if !wire_capture_enabled_for_state(state).await {
        return;
    }
    let Ok(mut guard) = state.wire_capture.try_lock() else {
        return;
    };
    guard.push_back(event);
    let keep = wire_capture_max_entries();
    while guard.len() > keep {
        let _ = guard.pop_front();
    }
}

fn diag_write_timeout_ms() -> u64 {
    std::env::var("LLMHOTSPOT_DIAG_WRITE_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v >= 10)
        .unwrap_or(150)
}

pub(crate) async fn push_recent_request_bounded(state: &AppState, req: &Value, endpoint: &str) {
    let _ = tokio::time::timeout(
        Duration::from_millis(diag_write_timeout_ms()),
        push_recent_request(state, req, endpoint),
    )
    .await;
}

pub(crate) fn model_for_diag_usage(req: &Value) -> String {
    let requested = req
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("gpt-5.2");
    requested.to_string()
}

pub(crate) async fn record_model_usage(state: &AppState, model: String) -> Result<(), String> {
    let Some(_) = &state.store else {
        return Ok(());
    };
    let mut guard = state.diag_usage_buffer.lock().await;
    let entry = guard.model_usage.entry(model).or_default();
    entry.total_requests = entry.total_requests.saturating_add(1);
    entry.last_seen_unix = now_unix();
    Ok(())
}

pub(crate) async fn add_model_token_usage(
    state: &AppState,
    model: &str,
    input: u64,
    output: u64,
    cached: u64,
) -> Result<(), String> {
    let Some(_) = &state.store else {
        return Ok(());
    };
    let mut guard = state.diag_usage_buffer.lock().await;
    let entry = guard.model_usage.entry(model.to_string()).or_default();
    entry.input_tokens = entry.input_tokens.saturating_add(input);
    entry.output_tokens = entry.output_tokens.saturating_add(output);
    entry.cached_tokens = entry.cached_tokens.saturating_add(cached);
    entry.last_seen_unix = now_unix();
    Ok(())
}

pub(crate) async fn record_key_model_usage(
    state: &AppState,
    key_id: &str,
    model: &str,
    input: u64,
    output: u64,
    cached: u64,
) -> Result<(), String> {
    let Some(_) = &state.store else {
        return Ok(());
    };
    let mut guard = state.diag_usage_buffer.lock().await;
    let entry = guard
        .key_usage
        .entry(key_id.to_string())
        .or_default()
        .entry(model.to_string())
        .or_default();
    entry.input_tokens = entry.input_tokens.saturating_add(input);
    entry.output_tokens = entry.output_tokens.saturating_add(output);
    entry.cached_tokens = entry.cached_tokens.saturating_add(cached);
    entry.request_count = entry.request_count.saturating_add(1);
    Ok(())
}

pub(crate) async fn record_last_upstream_error(
    state: &AppState,
    account_id: &str,
    detail: &GatewayErrorDetail,
) -> Result<(), String> {
    let Some(store) = &state.store else {
        return Ok(());
    };
    let now = now_unix();
    let safe_url = detail.url.as_ref().map(|v| redact_sensitive_text(v));
    let safe_body = detail
        .body_snippet
        .as_ref()
        .map(|v| redact_sensitive_text(v));
    let safe_transport = detail
        .transport_message
        .as_ref()
        .map(|v| redact_sensitive_text(v));
    let encoded = serde_json::to_string(&json!({
        "seen_unix": now,
        "account_id": account_id,
        "kind": detail.kind,
        "url": safe_url,
        "status": detail.status,
        "body_snippet": safe_body,
        "transport_message": safe_transport,
    }))
    .map_err(|e| e.to_string())?;
    store
        .set_app_state("diag_last_upstream_error_json", &encoded)
        .await
        .map_err(|e| e.to_string())
}

pub(crate) async fn record_oauth_account_usage(
    state: &AppState,
    account_id: &str,
    model: &str,
    input: u64,
    output: u64,
    cached: u64,
) -> Result<(), String> {
    let Some(_) = &state.store else {
        return Ok(());
    };
    let mut guard = state.diag_usage_buffer.lock().await;
    let entry = guard
        .oauth_usage
        .entry(account_id.to_string())
        .or_default()
        .entry(model.to_string())
        .or_default();
    entry.input_tokens = entry.input_tokens.saturating_add(input);
    entry.output_tokens = entry.output_tokens.saturating_add(output);
    entry.cached_tokens = entry.cached_tokens.saturating_add(cached);
    entry.request_count = entry.request_count.saturating_add(1);
    Ok(())
}

/// Snapshots the current lifetime usage for `entity_id` into the baseline
/// table so that reported usage appears to start from zero again.
/// Shared by reset_key_usage, reset_oauth_usage, and reset_model_usage.
async fn apply_usage_reset<V>(
    store: &SqliteStore,
    entity_id: &str,
    lifetime_key: &str,
    baseline_key: &str,
) where
    V: serde::de::DeserializeOwned + serde::Serialize + Default + Clone,
{
    let raw = store.get_app_state(lifetime_key).await.unwrap_or_default();
    let lifetime: HashMap<String, V> = serde_json::from_str(&raw).unwrap_or_default();
    let base_raw = store.get_app_state(baseline_key).await.unwrap_or_default();
    let mut baseline: HashMap<String, V> = serde_json::from_str(&base_raw).unwrap_or_default();
    baseline.insert(
        entity_id.to_string(),
        lifetime.get(entity_id).cloned().unwrap_or_default(),
    );
    let encoded = serde_json::to_string(&baseline).unwrap_or_default();
    let _ = store.set_app_state(baseline_key, &encoded).await;
}

pub(crate) async fn reset_key_usage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
) -> impl IntoResponse {
    let _ = flush_diag_usage_now(&state).await;
    let Some(store) = &state.store else {
        return missing_store();
    };
    if !is_admin_authorized(&headers, store).await {
        return unauthorized_admin();
    }
    apply_usage_reset::<HashMap<String, KeyModelUsageEntry>>(
        store,
        &key_id,
        "diag_key_usage_json",
        "diag_key_usage_baseline_json",
    )
    .await;
    (StatusCode::OK, Json(json!({"ok": true}))).into_response()
}

pub(crate) async fn reset_oauth_usage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(account_id): Path<String>,
) -> impl IntoResponse {
    let _ = flush_diag_usage_now(&state).await;
    let Some(store) = &state.store else {
        return missing_store();
    };
    if !is_admin_authorized(&headers, store).await {
        return unauthorized_admin();
    }
    apply_usage_reset::<HashMap<String, KeyModelUsageEntry>>(
        store,
        &account_id,
        "diag_oauth_usage_json",
        "diag_oauth_usage_baseline_json",
    )
    .await;
    (StatusCode::OK, Json(json!({"ok": true}))).into_response()
}

pub(crate) async fn reset_model_usage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(model_name): Path<String>,
) -> impl IntoResponse {
    let _ = flush_diag_usage_now(&state).await;
    let Some(store) = &state.store else {
        return missing_store();
    };
    if !is_admin_authorized(&headers, store).await {
        return unauthorized_admin();
    }
    apply_usage_reset::<ModelUsageSnapshot>(
        store,
        &model_name,
        "diag_model_usage_json",
        "diag_model_usage_baseline_json",
    )
    .await;
    (StatusCode::OK, Json(json!({"ok": true}))).into_response()
}

pub(crate) async fn increment_counter(state: &AppState, key: &str) -> Result<u64, String> {
    let Some(store) = &state.store else {
        return Ok(0);
    };
    let current = store
        .get_app_state(key)
        .await
        .unwrap_or_default()
        .parse::<u64>()
        .unwrap_or(0)
        .saturating_add(1);
    store
        .set_app_state(key, &current.to_string())
        .await
        .map_err(|e| e.to_string())?;
    Ok(current)
}

pub(crate) async fn push_recent_error(state: &AppState, message: &str) -> Result<(), String> {
    let Some(store) = &state.store else {
        return Ok(());
    };
    let raw = store
        .get_app_state("diag_recent_errors_json")
        .await
        .unwrap_or_default();
    let mut arr: Vec<String> = serde_json::from_str(&raw).unwrap_or_default();
    arr.push(redact_sensitive_text(message));
    if arr.len() > 50 {
        arr.drain(0..arr.len().saturating_sub(50));
    }
    let encoded = serde_json::to_string(&arr).map_err(|e| e.to_string())?;
    store
        .set_app_state("diag_recent_errors_json", &encoded)
        .await
        .map_err(|e| e.to_string())
}

async fn push_recent_request(state: &AppState, req: &Value, endpoint: &str) -> Result<(), String> {
    let Some(store) = &state.store else {
        return Ok(());
    };
    let model = req
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("?")
        .to_string();
    let resolved_model = normalize_model_name_for_routing(&model);
    let routed_provider = resolve_provider_for_model(&resolved_model)
        .unwrap_or("unknown")
        .to_string();
    let stream = req.get("stream").and_then(Value::as_bool).unwrap_or(false);
    let messages = req.get("messages").and_then(Value::as_array);
    let input = req.get("input").and_then(Value::as_array);
    let msg_count = messages
        .map(|m| m.len())
        .or_else(|| input.map(|i| i.len()))
        .unwrap_or(0);
    // Handle both Chat Completions nested format {function:{name:...}} and
    // Responses API flat format {name:...} for tool name extraction.
    let tools: Vec<String> = req
        .get("tools")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|t| {
                    t.get("function")
                        .and_then(|f| f.get("name"))
                        .or_else(|| t.get("name"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    // Avoid storing request content in diagnostics; only track metadata.
    let last_user_msg_len: usize = messages
        .or(input)
        .and_then(|msgs| {
            msgs.iter()
                .rev()
                .find(|m| m.get("role").and_then(Value::as_str) == Some("user"))
                .map(|m| chat_content_to_text(m.get("content").unwrap_or(&Value::Null)).len())
        })
        .unwrap_or(0);
    let tool_choice = req.get("tool_choice").cloned().unwrap_or(Value::Null);
    let (message_roles, last_role, last_has_tool_result_block) = messages
        .or(input)
        .map(|msgs| {
            let roles: Vec<String> = msgs
                .iter()
                .map(|m| {
                    let role = m.get("role").and_then(Value::as_str).unwrap_or("?");
                    let has_tc = m
                        .get("tool_calls")
                        .and_then(Value::as_array)
                        .is_some_and(|v| !v.is_empty());
                    if has_tc {
                        format!("{role}(tc)")
                    } else {
                        role.to_string()
                    }
                })
                .collect();
            let last = msgs
                .last()
                .and_then(|m| m.get("role"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let last_tool_result = msgs
                .last()
                .and_then(|m| m.get("content"))
                .and_then(Value::as_array)
                .map(|parts| {
                    parts
                        .iter()
                        .any(|p| p.get("type").and_then(Value::as_str) == Some("tool_result"))
                })
                .unwrap_or(false);
            (roles.join(" -> "), last, last_tool_result)
        })
        .unwrap_or_else(|| (String::new(), String::new(), false));
    let entry = json!({
        "at_unix": now_unix(),
        "endpoint": endpoint,
        "model": model,
        "requested_model": model,
        "resolved_model": resolved_model,
        "routed_provider": routed_provider,
        "stream": stream,
        "msg_count": msg_count,
        "tool_choice": tool_choice,
        "tools": tools,
        "last_user_msg_len": last_user_msg_len,
        "message_roles": message_roles,
        "last_role": last_role,
        "last_has_tool_result_block": last_has_tool_result_block,
    });
    let raw = store
        .get_app_state("diag_recent_requests_json")
        .await
        .unwrap_or_default();
    let mut arr: Vec<Value> = serde_json::from_str(&raw).unwrap_or_default();
    arr.push(entry);
    if arr.len() > 10 {
        arr.drain(0..arr.len().saturating_sub(10));
    }
    let encoded = serde_json::to_string(&arr).map_err(|e| e.to_string())?;
    store
        .set_app_state("diag_recent_requests_json", &encoded)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod diagnostics_flag_tests {
    use super::wire_capture_enabled;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn wire_capture_defaults_off_when_unset() {
        let _guard = env_lock().lock().expect("env lock");
        std::env::remove_var("LLMHOTSPOT_WIRE_CAPTURE");
        assert!(!wire_capture_enabled());
    }

    #[test]
    fn wire_capture_accepts_truthy_values() {
        let _guard = env_lock().lock().expect("env lock");
        for v in ["1", "true", "TRUE", "yes", "on"] {
            std::env::set_var("LLMHOTSPOT_WIRE_CAPTURE", v);
            assert!(wire_capture_enabled(), "expected enabled for value={v}");
        }
        std::env::remove_var("LLMHOTSPOT_WIRE_CAPTURE");
    }

    #[test]
    fn wire_capture_rejects_falsey_and_unknown_values() {
        let _guard = env_lock().lock().expect("env lock");
        for v in ["0", "false", "no", "off", "maybe"] {
            std::env::set_var("LLMHOTSPOT_WIRE_CAPTURE", v);
            assert!(!wire_capture_enabled(), "expected disabled for value={v}");
        }
        std::env::remove_var("LLMHOTSPOT_WIRE_CAPTURE");
    }
}
