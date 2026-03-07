use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use llm_storage::OAuthAccountRecord;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

use crate::model_resolution::parse_headers_json;
use crate::pro_entitlement::current_entitlement_tier;

use super::{
    cached_models_key, canonical_base_url, delete_custom_api_key, error_response,
    is_admin_authorized, is_custom_openai_api_account, list_all_accounts, list_custom_api_accounts,
    missing_store, push_recent_error, refresh_custom_api_models_for_account,
    save_custom_api_accounts, set_custom_api_key, unauthorized_admin, AppState, ErrorCode,
    OPENAI_API_PROVIDER_ID,
};

#[derive(Debug, Deserialize)]
pub(crate) struct CreateApiAccountRequest {
    label: Option<String>,
    base_url: String,
    api_key: String,
    organization: Option<String>,
    project: Option<String>,
    #[serde(default)]
    extra_headers: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateApiAccountRequest {
    label: Option<String>,
    base_url: Option<String>,
    api_key: Option<String>,
    organization: Option<String>,
    project: Option<String>,
    enabled: Option<bool>,
    #[serde(default)]
    extra_headers: Option<HashMap<String, String>>,
}

pub(crate) async fn list_api_accounts(State(state): State<AppState>) -> impl IntoResponse {
    let Some(store) = state.store else {
        return (StatusCode::OK, Json(json!({"accounts":[]}))).into_response();
    };
    let accounts = match list_custom_api_accounts(&store).await {
        Ok(v) => v,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                &format!("storage error: {e}"),
            )
        }
    };
    let rows: Vec<Value> = accounts
        .into_iter()
        .map(|account| {
            json!({
                "account": account,
                "has_api_key": true,
            })
        })
        .collect();
    (StatusCode::OK, Json(json!({"accounts": rows}))).into_response()
}

pub(crate) async fn create_api_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateApiAccountRequest>,
) -> impl IntoResponse {
    let Some(store) = &state.store else {
        return missing_store();
    };
    if !is_admin_authorized(&headers, store).await {
        return unauthorized_admin();
    }
    let Some(base_url) = canonical_base_url(&req.base_url) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            ErrorCode::Internal,
            "base_url must be a valid http(s) URL",
        );
    };
    if req.api_key.trim().is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            ErrorCode::Internal,
            "api_key is required",
        );
    }

    let mut accounts = match list_custom_api_accounts(store).await {
        Ok(v) => v,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                &format!("storage error: {e}"),
            )
        }
    };
    let all_accounts = list_all_accounts(store).await.unwrap_or_default();
    let priority = all_accounts.len() as i64;
    let account_id = format!("api_{}", uuid::Uuid::new_v4().simple());
    let display_name = req
        .label
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Custom OpenAI API")
        .to_string();
    let tier = current_entitlement_tier(store).await;
    if tier != "pro" {
        for a in &mut accounts {
            a.enabled = false;
        }
        if let Ok(existing) = store.list_oauth_accounts().await {
            for a in existing {
                if a.enabled {
                    let _ = store.update_oauth_enabled(&a.account_id, false).await;
                }
            }
        }
    }

    let row = OAuthAccountRecord {
        account_id: account_id.clone(),
        provider_id: OPENAI_API_PROVIDER_ID.to_string(),
        display_name,
        email: base_url.clone(),
        priority,
        enabled: true,
        last_used_unix: None,
        last_error: None,
        upstream_base_url: Some(base_url),
        chat_path: None,
        responses_path: None,
        org_id: req.organization.and_then(|s| {
            let t = s.trim().to_string();
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        }),
        project_id: req.project.and_then(|s| {
            let t = s.trim().to_string();
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        }),
        extra_headers_json: serde_json::to_string(&req.extra_headers).ok(),
    };
    accounts.push(row.clone());
    if let Err(e) = save_custom_api_accounts(store, &accounts).await {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            &format!("storage error: {e}"),
        );
    }
    if let Err(e) = set_custom_api_key(store, &account_id, req.api_key.trim()).await {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            &format!("storage error: {e}"),
        );
    }
    if let Err(err) = refresh_custom_api_models_for_account(store, &row, true).await {
        let _ = push_recent_error(&state, &format!("custom models refresh failed: {err}")).await;
    }
    (
        StatusCode::OK,
        Json(json!({"ok": true, "account": row, "has_api_key": true})),
    )
        .into_response()
}

pub(crate) async fn update_api_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(account_id): Path<String>,
    Json(req): Json<UpdateApiAccountRequest>,
) -> impl IntoResponse {
    let Some(store) = &state.store else {
        return missing_store();
    };
    if !is_admin_authorized(&headers, store).await {
        return unauthorized_admin();
    }
    let mut accounts = match list_custom_api_accounts(store).await {
        Ok(v) => v,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                &format!("storage error: {e}"),
            )
        }
    };
    let Some(idx) = accounts.iter().position(|a| a.account_id == account_id) else {
        return error_response(
            StatusCode::NOT_FOUND,
            ErrorCode::Internal,
            "account not found",
        );
    };

    if let Some(label) = req.label {
        let t = label.trim();
        if !t.is_empty() {
            accounts[idx].display_name = t.to_string();
        }
    }
    if let Some(base) = req.base_url.as_deref() {
        let Some(normalized) = canonical_base_url(base) else {
            return error_response(
                StatusCode::BAD_REQUEST,
                ErrorCode::Internal,
                "base_url must be a valid http(s) URL",
            );
        };
        accounts[idx].upstream_base_url = Some(normalized.clone());
        accounts[idx].email = normalized;
    }
    if let Some(org) = req.organization {
        let t = org.trim().to_string();
        accounts[idx].org_id = if t.is_empty() { None } else { Some(t) };
    }
    if let Some(project) = req.project {
        let t = project.trim().to_string();
        accounts[idx].project_id = if t.is_empty() { None } else { Some(t) };
    }
    if let Some(headers_map) = req.extra_headers {
        accounts[idx].extra_headers_json = serde_json::to_string(&headers_map).ok();
    }
    if let Some(enabled) = req.enabled {
        if enabled {
            let tier = current_entitlement_tier(store).await;
            if tier != "pro" {
                if let Ok(existing) = list_all_accounts(store).await {
                    for a in existing {
                        if a.account_id == account_id || !a.enabled {
                            continue;
                        }
                        if is_custom_openai_api_account(&a) {
                            if let Some(slot) =
                                accounts.iter_mut().find(|x| x.account_id == a.account_id)
                            {
                                slot.enabled = false;
                            }
                        } else {
                            let _ = store.update_oauth_enabled(&a.account_id, false).await;
                        }
                    }
                }
            }
        }
        accounts[idx].enabled = enabled;
    }
    if let Some(api_key) = req.api_key.as_deref() {
        if !api_key.trim().is_empty() {
            if let Err(e) = set_custom_api_key(store, &account_id, api_key.trim()).await {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorCode::Internal,
                    &format!("storage error: {e}"),
                );
            }
        }
    }
    let account = accounts[idx].clone();
    if let Err(e) = save_custom_api_accounts(store, &accounts).await {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            &format!("storage error: {e}"),
        );
    }
    if req.base_url.is_some() || req.api_key.is_some() {
        if let Err(err) = refresh_custom_api_models_for_account(store, &account, true).await {
            let _ =
                push_recent_error(&state, &format!("custom models refresh failed: {err}")).await;
        }
    }
    (
        StatusCode::OK,
        Json(json!({"ok": true, "account": account, "has_api_key": true})),
    )
        .into_response()
}

pub(crate) async fn delete_api_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(account_id): Path<String>,
) -> impl IntoResponse {
    let Some(store) = state.store else {
        return missing_store();
    };
    if !is_admin_authorized(&headers, &store).await {
        return unauthorized_admin();
    }
    let mut accounts = match list_custom_api_accounts(&store).await {
        Ok(v) => v,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                &format!("storage error: {e}"),
            )
        }
    };
    let before = accounts.len();
    accounts.retain(|a| a.account_id != account_id);
    if accounts.len() == before {
        return error_response(
            StatusCode::NOT_FOUND,
            ErrorCode::Internal,
            "account not found",
        );
    }
    if let Err(e) = save_custom_api_accounts(&store, &accounts).await {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            &format!("storage error: {e}"),
        );
    }
    let _ = delete_custom_api_key(&store, &account_id).await;
    let _ = store
        .delete_app_state(&cached_models_key(OPENAI_API_PROVIDER_ID, &account_id))
        .await;
    (StatusCode::OK, Json(json!({"ok": true}))).into_response()
}

pub(crate) async fn refresh_api_account_models(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(account_id): Path<String>,
) -> impl IntoResponse {
    let Some(store) = state.store else {
        return missing_store();
    };
    if !is_admin_authorized(&headers, &store).await {
        return unauthorized_admin();
    }
    let accounts = match list_custom_api_accounts(&store).await {
        Ok(v) => v,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                &format!("storage error: {e}"),
            )
        }
    };
    let Some(account) = accounts.into_iter().find(|a| a.account_id == account_id) else {
        return error_response(
            StatusCode::NOT_FOUND,
            ErrorCode::Internal,
            "account not found",
        );
    };
    match refresh_custom_api_models_for_account(&store, &account, true).await {
        Ok(models) => (StatusCode::OK, Json(json!({"ok": true, "models": models}))).into_response(),
        Err(e) => error_response(
            StatusCode::BAD_GATEWAY,
            ErrorCode::Internal,
            &format!("model refresh failed: {e}"),
        ),
    }
}

pub(crate) async fn list_oauth_accounts(State(state): State<AppState>) -> impl IntoResponse {
    let Some(store) = state.store else {
        return (StatusCode::OK, Json(json!({"accounts":[]}))).into_response();
    };

    let accounts = match list_all_accounts(&store).await {
        Ok(v) => v,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                &format!("storage error: {e}"),
            )
        }
    };

    let mut rows = Vec::with_capacity(accounts.len());
    for account in accounts {
        let usage = store
            .list_oauth_usage(&account.account_id)
            .await
            .unwrap_or_default();
        rows.push(json!({"account": account, "usage": usage}));
    }
    (StatusCode::OK, Json(json!({"accounts": rows}))).into_response()
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub(crate) struct RoutingProfileRequest {
    upstream_base_url: Option<String>,
    chat_path: Option<String>,
    responses_path: Option<String>,
    org_id: Option<String>,
    project_id: Option<String>,
    #[serde(default)]
    extra_headers: HashMap<String, String>,
}

pub(crate) async fn get_oauth_routing(
    State(state): State<AppState>,
    Path(account_id): Path<String>,
) -> impl IntoResponse {
    let Some(store) = state.store else {
        return missing_store();
    };
    let account = match store.get_oauth_account(&account_id).await {
        Ok(Some(account)) => Some(account),
        Ok(None) => list_custom_api_accounts(&store)
            .await
            .ok()
            .and_then(|v| v.into_iter().find(|a| a.account_id == account_id)),
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                &format!("storage error: {e}"),
            )
        }
    };
    let Some(account) = account else {
        return error_response(
            StatusCode::NOT_FOUND,
            ErrorCode::Internal,
            "account not found",
        );
    };
    (
        StatusCode::OK,
        Json(json!(RoutingProfileRequest {
            upstream_base_url: account.upstream_base_url,
            chat_path: account.chat_path,
            responses_path: account.responses_path,
            org_id: account.org_id,
            project_id: account.project_id,
            extra_headers: parse_headers_json(account.extra_headers_json.as_deref()),
        })),
    )
        .into_response()
}

pub(crate) async fn set_oauth_routing(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(account_id): Path<String>,
    Json(req): Json<RoutingProfileRequest>,
) -> impl IntoResponse {
    let Some(store) = state.store else {
        return missing_store();
    };
    if !is_admin_authorized(&headers, &store).await {
        return unauthorized_admin();
    }

    let mut account = match store.get_oauth_account(&account_id).await {
        Ok(Some(v)) => v,
        Ok(None) => {
            let mut custom = match list_custom_api_accounts(&store).await {
                Ok(v) => v,
                Err(e) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        ErrorCode::Internal,
                        &format!("storage error: {e}"),
                    )
                }
            };
            let Some(idx) = custom.iter().position(|a| a.account_id == account_id) else {
                return error_response(
                    StatusCode::NOT_FOUND,
                    ErrorCode::Internal,
                    "account not found",
                );
            };
            custom[idx].upstream_base_url = req.upstream_base_url;
            custom[idx].chat_path = req.chat_path;
            custom[idx].responses_path = req.responses_path;
            custom[idx].org_id = req.org_id;
            custom[idx].project_id = req.project_id;
            custom[idx].extra_headers_json = serde_json::to_string(&req.extra_headers).ok();
            return match save_custom_api_accounts(&store, &custom).await {
                Ok(()) => (StatusCode::OK, Json(json!({"ok": true}))).into_response(),
                Err(e) => error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorCode::Internal,
                    &format!("storage error: {e}"),
                ),
            };
        }
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                &format!("storage error: {e}"),
            )
        }
    };

    account.upstream_base_url = req.upstream_base_url;
    account.chat_path = req.chat_path;
    account.responses_path = req.responses_path;
    account.org_id = req.org_id;
    account.project_id = req.project_id;
    account.extra_headers_json = serde_json::to_string(&req.extra_headers).ok();

    match store.upsert_oauth_account(&account).await {
        Ok(()) => (StatusCode::OK, Json(json!({"ok": true}))).into_response(),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            &format!("storage error: {e}"),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConnectAccountRequest {
    account_id: String,
    display_name: String,
    email: String,
}

pub(crate) async fn connect_oauth_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ConnectAccountRequest>,
) -> impl IntoResponse {
    let Some(store) = state.store else {
        return missing_store();
    };
    if !is_admin_authorized(&headers, &store).await {
        return unauthorized_admin();
    }
    let tier = current_entitlement_tier(&store).await;
    if tier != "pro" {
        // Free mode: only one enabled OAuth account at a time.
        if let Ok(accounts) = list_all_accounts(&store).await {
            let mut custom_accounts = list_custom_api_accounts(&store).await.unwrap_or_default();
            for a in accounts {
                if a.enabled {
                    if is_custom_openai_api_account(&a) {
                        if let Some(slot) = custom_accounts
                            .iter_mut()
                            .find(|x| x.account_id == a.account_id)
                        {
                            slot.enabled = false;
                        }
                    } else {
                        let _ = store.update_oauth_enabled(&a.account_id, false).await;
                    }
                }
            }
            let _ = save_custom_api_accounts(&store, &custom_accounts).await;
        }
    }

    let priority = store
        .list_oauth_accounts()
        .await
        .map(|v| v.len() as i64)
        .unwrap_or(0);
    let row = OAuthAccountRecord {
        account_id: req.account_id,
        provider_id: "openai".to_string(),
        display_name: req.display_name,
        email: req.email,
        priority,
        enabled: true,
        last_used_unix: None,
        last_error: None,
        upstream_base_url: None,
        chat_path: None,
        responses_path: None,
        org_id: None,
        project_id: None,
        extra_headers_json: None,
    };

    match store.upsert_oauth_account(&row).await {
        Ok(()) => (StatusCode::OK, Json(json!({"ok": true}))).into_response(),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            &format!("storage error: {e}"),
        ),
    }
}

pub(crate) async fn activate_oauth_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(account_id): Path<String>,
) -> impl IntoResponse {
    set_priority_impl(state, headers, account_id, 0).await
}

#[derive(Debug, Deserialize)]
pub(crate) struct PriorityRequest {
    index: usize,
}

pub(crate) async fn set_oauth_priority(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(account_id): Path<String>,
    Json(req): Json<PriorityRequest>,
) -> impl IntoResponse {
    set_priority_impl(state, headers, account_id, req.index).await
}

async fn set_priority_impl(
    state: AppState,
    headers: HeaderMap,
    account_id: String,
    index: usize,
) -> axum::response::Response {
    let Some(store) = state.store else {
        return missing_store();
    };
    if !is_admin_authorized(&headers, &store).await {
        return unauthorized_admin();
    }

    let mut accounts = match list_all_accounts(&store).await {
        Ok(v) => v,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                &format!("storage error: {e}"),
            )
        }
    };

    let Some(current_idx) = accounts.iter().position(|a| a.account_id == account_id) else {
        return error_response(
            StatusCode::NOT_FOUND,
            ErrorCode::Internal,
            "account not found",
        );
    };

    let account = accounts.remove(current_idx);
    let idx = index.min(accounts.len());
    accounts.insert(idx, account);

    let mut custom_accounts = match list_custom_api_accounts(&store).await {
        Ok(v) => v,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                &format!("storage error: {e}"),
            )
        }
    };

    for (priority, item) in accounts.into_iter().enumerate() {
        if is_custom_openai_api_account(&item) {
            if let Some(slot) = custom_accounts
                .iter_mut()
                .find(|a| a.account_id == item.account_id)
            {
                slot.priority = priority as i64;
            }
            continue;
        }
        if let Err(e) = store
            .set_oauth_priority(&item.account_id, priority as i64)
            .await
        {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                &format!("storage error: {e}"),
            );
        }
    }
    if let Err(e) = save_custom_api_accounts(&store, &custom_accounts).await {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            &format!("storage error: {e}"),
        );
    }

    (StatusCode::OK, Json(json!({"ok": true}))).into_response()
}

pub(crate) async fn enable_oauth_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(account_id): Path<String>,
) -> impl IntoResponse {
    update_account_enabled(state, headers, account_id, true).await
}

pub(crate) async fn disable_oauth_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(account_id): Path<String>,
) -> impl IntoResponse {
    update_account_enabled(state, headers, account_id, false).await
}

async fn update_account_enabled(
    state: AppState,
    headers: HeaderMap,
    account_id: String,
    enabled: bool,
) -> axum::response::Response {
    let Some(store) = state.store else {
        return missing_store();
    };
    if !is_admin_authorized(&headers, &store).await {
        return unauthorized_admin();
    }
    let mut custom_accounts = match list_custom_api_accounts(&store).await {
        Ok(v) => v,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                &format!("storage error: {e}"),
            )
        }
    };
    let custom_idx = custom_accounts
        .iter()
        .position(|a| a.account_id == account_id);
    let tier = current_entitlement_tier(&store).await;
    if tier != "pro" && enabled {
        if let Ok(accounts) = list_all_accounts(&store).await {
            for a in accounts {
                if a.account_id == account_id || !a.enabled {
                    continue;
                }
                if is_custom_openai_api_account(&a) {
                    if let Some(slot) = custom_accounts
                        .iter_mut()
                        .find(|x| x.account_id == a.account_id)
                    {
                        slot.enabled = false;
                    }
                } else {
                    let _ = store.update_oauth_enabled(&a.account_id, false).await;
                }
            }
        }
    }

    if let Some(idx) = custom_idx {
        custom_accounts[idx].enabled = enabled;
        return match save_custom_api_accounts(&store, &custom_accounts).await {
            Ok(()) => (StatusCode::OK, Json(json!({"ok": true}))).into_response(),
            Err(e) => error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                &format!("storage error: {e}"),
            ),
        };
    }

    match store.update_oauth_enabled(&account_id, enabled).await {
        Ok(()) => (StatusCode::OK, Json(json!({"ok": true}))).into_response(),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            &format!("storage error: {e}"),
        ),
    }
}

pub(crate) async fn disconnect_oauth_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(account_id): Path<String>,
) -> impl IntoResponse {
    let Some(store) = state.store else {
        return missing_store();
    };
    if !is_admin_authorized(&headers, &store).await {
        return unauthorized_admin();
    }

    let mut custom_accounts = match list_custom_api_accounts(&store).await {
        Ok(v) => v,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                &format!("storage error: {e}"),
            )
        }
    };
    if custom_accounts.iter().any(|a| a.account_id == account_id) {
        custom_accounts.retain(|a| a.account_id != account_id);
        if let Err(e) = save_custom_api_accounts(&store, &custom_accounts).await {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                &format!("storage error: {e}"),
            );
        }
        let _ = delete_custom_api_key(&store, &account_id).await;
        let _ = store
            .delete_app_state(&cached_models_key(OPENAI_API_PROVIDER_ID, &account_id))
            .await;
        return (StatusCode::OK, Json(json!({"ok": true}))).into_response();
    }

    match store.delete_oauth_account(&account_id).await {
        Ok(()) => {
            let _ = store.delete_token(&state.workspace_id, &account_id).await;
            (StatusCode::OK, Json(json!({"ok": true}))).into_response()
        }
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            &format!("storage error: {e}"),
        ),
    }
}
