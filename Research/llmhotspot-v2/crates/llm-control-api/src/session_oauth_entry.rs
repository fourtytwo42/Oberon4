use super::*;
pub(crate) async fn console_session_exchange(
    State(state): State<AppState>,
    Json(req): Json<ConsoleSessionExchangeRequest>,
) -> impl IntoResponse {
    let Some(store) = state.store else {
        return missing_store();
    };
    let expected = store
        .get_app_state("console_admin_bootstrap_key")
        .await
        .unwrap_or_default();
    let previous = store
        .get_app_state("console_admin_previous_bootstrap_key")
        .await
        .unwrap_or_default();
    let matches_current = !expected.is_empty() && expected == req.admin_key;
    let matches_previous = !previous.is_empty() && previous == req.admin_key;
    let matches_remote_console_key = store
        .get_remote_assignment()
        .await
        .ok()
        .and_then(|remote| remote.remote_console_url)
        .and_then(|url| {
            let parsed = Url::parse(&url).ok()?;
            parsed
                .query_pairs()
                .find(|(k, _)| k == "admin_key")
                .map(|(_, v)| v.to_string())
        })
        .map(|remote_key| remote_key == req.admin_key)
        .unwrap_or(false);
    if !matches_current && !matches_previous && !matches_remote_console_key {
        return unauthorized_admin();
    }
    let token = format!("cs_{}", uuid::Uuid::new_v4().simple());
    let now = now_unix();
    let expires = now.saturating_add(12 * 60 * 60);
    let _ = store
        .set_app_state("console_admin_session_token_hash", &hash_key(&token))
        .await;
    let _ = store
        .set_app_state("console_admin_session_expires_unix", &expires.to_string())
        .await;
    let mut resp = Json(json!({"ok": true, "expires_unix": expires})).into_response();
    let cookie = format!(
        "llmhotspot_console_session={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
        12 * 60 * 60
    );
    if let Ok(v) = cookie.parse() {
        resp.headers_mut().insert(header::SET_COOKIE, v);
    }
    resp
}

#[derive(Debug, Deserialize)]
pub(crate) struct OAuthStartRequest {
    redirect_uri: Option<String>,
}

pub(crate) async fn oauth_start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<OAuthStartRequest>,
) -> impl IntoResponse {
    let Some(store) = state.store else {
        return missing_store();
    };
    if !is_admin_authorized(&headers, &store).await {
        return unauthorized_admin();
    }

    match state.oauth_client.start_flow(req.redirect_uri.as_deref()) {
        Ok(start) => (
            StatusCode::OK,
            Json(json!({
                "authorization_url": start.authorization_url,
                "state": start.state,
                "redirect_uri": start.redirect_uri
            })),
        )
            .into_response(),
        Err(err) => map_auth_error(err),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct OAuthCallbackCompleteRequest {
    state: String,
    code: String,
    redirect_uri: Option<String>,
}

pub(crate) async fn oauth_callback_complete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<OAuthCallbackCompleteRequest>,
) -> impl IntoResponse {
    let Some(store) = state.store else {
        return missing_store();
    };
    if !is_admin_authorized(&headers, &store).await {
        return unauthorized_admin();
    }

    let token = match state
        .oauth_client
        .exchange_code(&req.state, &req.code, req.redirect_uri.as_deref())
        .await
    {
        Ok(v) => v,
        Err(err) => return map_auth_error(err),
    };

    let account_id = token
        .account_id
        .clone()
        .unwrap_or_else(|| format!("acct_{}", now_unix()));
    let email = token
        .email
        .clone()
        .unwrap_or_else(|| format!("{account_id}@unknown.local"));
    let display_name = email.clone();
    let priority = store
        .list_oauth_accounts()
        .await
        .map(|v| v.len() as i64)
        .unwrap_or(0);

    let row = OAuthAccountRecord {
        account_id: account_id.clone(),
        provider_id: "openai".to_string(),
        display_name,
        email,
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
    if let Err(e) = store.upsert_oauth_account(&row).await {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            &format!("storage error: {e}"),
        );
    }

    let mut rec = StoredTokenRecord::new(&state.workspace_id, &account_id, &token.refresh_token);
    rec.account_id = token.account_id.clone();
    if let Err(e) = store.save_token(&rec).await {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            &format!("token store error: {e}"),
        );
    }

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "account_id": account_id,
            "email": token.email,
            "expires_at_unix": token.expires_at_unix
        })),
    )
        .into_response()
}
