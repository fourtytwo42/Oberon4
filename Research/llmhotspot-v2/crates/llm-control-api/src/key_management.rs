use crate::pro_entitlement::current_entitlement_tier;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use llm_foundation::contracts::ErrorCode;
use llm_storage::ApiKeyRecord;
use serde::Deserialize;
use serde_json::json;

use super::AppState;

#[derive(Debug, Deserialize)]
pub(crate) struct CreateKeyRequest {
    label: String,
    expires_unix: Option<u64>,
}

pub(crate) async fn create_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateKeyRequest>,
) -> impl IntoResponse {
    let Some(store) = &state.store else {
        return super::missing_store();
    };

    if !super::is_admin_authorized(&headers, store).await {
        return super::unauthorized_admin();
    }
    let tier = current_entitlement_tier(store).await;
    if tier != "pro" {
        if let Ok(keys) = store.list_api_keys().await {
            if !keys.is_empty() {
                return super::error_response(
                    StatusCode::PAYMENT_REQUIRED,
                    ErrorCode::AuthError,
                    "upgrade_required",
                );
            }
        }
    }

    let raw_key = format!("hs_{}", uuid::Uuid::new_v4().simple());
    let key_hash = super::hash_key(&raw_key);
    let now = super::now_unix();
    let key = ApiKeyRecord {
        key_id: uuid::Uuid::new_v4().to_string(),
        label: req.label,
        key_hash,
        tool_choice_policy: "auto".to_string(),
        created_unix: now,
        expires_unix: req.expires_unix,
        revoked_unix: None,
        last_used_unix: None,
    };

    match store.create_api_key(&key).await {
        Ok(()) => {
            if let Err(e) = super::set_api_key_raw_with_fallback(
                store,
                &state.workspace_id,
                &key.key_id,
                &raw_key,
            )
            .await
            {
                // Do not leave a key row behind if its raw secret cannot be recovered.
                let _ = store.delete_api_key(&key.key_id).await;
                return super::error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorCode::Internal,
                    &format!("storage error: failed to persist raw key: {e}"),
                );
            }
            (
                StatusCode::OK,
                Json(json!({"key_id": key.key_id, "raw_key": raw_key})),
            )
                .into_response()
        }
        Err(e) => super::error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            &format!("storage error: {e}"),
        ),
    }
}

pub(crate) async fn list_keys(State(state): State<AppState>) -> impl IntoResponse {
    let Some(store) = state.store else {
        return (StatusCode::OK, Json(json!({"keys":[]}))).into_response();
    };

    match store.list_api_keys().await {
        Ok(keys) => (StatusCode::OK, Json(json!({"keys": keys}))).into_response(),
        Err(e) => super::error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            &format!("storage error: {e}"),
        ),
    }
}

pub(crate) async fn revoke_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
) -> impl IntoResponse {
    let Some(store) = &state.store else {
        return super::missing_store();
    };
    if !super::is_admin_authorized(&headers, store).await {
        return super::unauthorized_admin();
    }

    match store.revoke_api_key(&key_id, super::now_unix()).await {
        Ok(()) => (StatusCode::OK, Json(json!({"ok": true}))).into_response(),
        Err(e) => super::error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            &format!("storage error: {e}"),
        ),
    }
}

pub(crate) async fn enable_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
) -> impl IntoResponse {
    let Some(store) = &state.store else {
        return super::missing_store();
    };
    if !super::is_admin_authorized(&headers, store).await {
        return super::unauthorized_admin();
    }

    match store.update_api_key_revoked(&key_id, None).await {
        Ok(()) => (StatusCode::OK, Json(json!({"ok": true}))).into_response(),
        Err(e) => super::error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            &format!("storage error: {e}"),
        ),
    }
}

pub(crate) async fn disable_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
) -> impl IntoResponse {
    let Some(store) = &state.store else {
        return super::missing_store();
    };
    if !super::is_admin_authorized(&headers, store).await {
        return super::unauthorized_admin();
    }

    match store
        .update_api_key_revoked(&key_id, Some(super::now_unix()))
        .await
    {
        Ok(()) => (StatusCode::OK, Json(json!({"ok": true}))).into_response(),
        Err(e) => super::error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            &format!("storage error: {e}"),
        ),
    }
}

pub(crate) async fn delete_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
) -> impl IntoResponse {
    let Some(store) = &state.store else {
        return super::missing_store();
    };
    if !super::is_admin_authorized(&headers, store).await {
        return super::unauthorized_admin();
    }

    match store.delete_api_key(&key_id).await {
        Ok(()) => {
            let _ =
                super::delete_api_key_raw_with_fallback(store, &state.workspace_id, &key_id).await;
            (StatusCode::OK, Json(json!({"ok": true}))).into_response()
        }
        Err(e) => super::error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            &format!("storage error: {e}"),
        ),
    }
}

pub(crate) async fn copy_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
) -> impl IntoResponse {
    let Some(store) = &state.store else {
        return super::missing_store();
    };
    if !super::is_admin_authorized(&headers, store).await {
        return super::unauthorized_admin();
    }

    let raw =
        match super::get_api_key_raw_with_legacy_migration(store, &state.workspace_id, &key_id)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                return super::error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorCode::Internal,
                    &format!("storage error: {e}"),
                );
            }
        };
    let Some(raw) = raw else {
        return super::error_response(
            StatusCode::NOT_FOUND,
            ErrorCode::Internal,
            "raw key not available",
        );
    };

    (
        StatusCode::OK,
        Json(json!({"key_id": key_id, "raw_key": raw})),
    )
        .into_response()
}
