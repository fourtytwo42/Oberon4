use super::*;
pub(crate) fn map_gateway_error_openai(err: GatewayError) -> axum::response::Response {
    match err {
        GatewayError::Auth => openai_error_response(
            StatusCode::UNAUTHORIZED,
            "auth error",
            "authentication_error",
            Some("authorization"),
        ),
        GatewayError::RateLimit => openai_error_response(
            StatusCode::TOO_MANY_REQUESTS,
            "rate limit",
            "rate_limit_error",
            None,
        ),
        GatewayError::TenantOffline => openai_error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "tenant offline",
            "server_error",
            None,
        ),
        GatewayError::Timeout | GatewayError::BadGateway => openai_error_response(
            StatusCode::BAD_GATEWAY,
            "upstream failure",
            "server_error",
            None,
        ),
    }
}

pub(crate) fn map_auth_error(err: AuthError) -> axum::response::Response {
    error_response(
        StatusCode::BAD_REQUEST,
        ErrorCode::AuthError,
        &format!("oauth error: {err}"),
    )
}

pub(crate) async fn start_local_oauth_callback_server(
    callback_path: &str,
    return_to: Option<&str>,
) -> Result<llm_auth::OAuthCallbackServer, AuthError> {
    // Try preferred port first, then nearby ports, then ephemeral as a last resort.
    let ports = [1455_u16, 1456, 1457, 1458, 1459, 0];
    let mut last_bind_error: Option<String> = None;
    for port in ports {
        match start_oauth_callback_server(None, Some(port), Some(callback_path), return_to).await {
            Ok(server) => return Ok(server),
            Err(err) => {
                let msg = err.to_string();
                // Only retry bind-type failures; propagate other OAuth errors immediately.
                if !msg.contains("oauth callback bind failed") {
                    return Err(err);
                }
                last_bind_error = Some(msg);
            }
        }
    }
    Err(AuthError::CallbackBindFailed(
        last_bind_error.unwrap_or_else(|| "unable to bind callback listener".to_string()),
    ))
}

pub(crate) fn error_response(
    status: StatusCode,
    code: ErrorCode,
    message: &str,
) -> axum::response::Response {
    (
        status,
        Json(json!({
            "error": ErrorEnvelope {
                code,
                message: message.to_string(),
            }
        })),
    )
        .into_response()
}

pub(crate) fn openai_error_response(
    status: StatusCode,
    message: &str,
    err_type: &str,
    param: Option<&str>,
) -> axum::response::Response {
    (
        status,
        Json(json!({
            "error": {
                "message": message,
                "type": err_type,
                "param": param,
                "code": Value::Null
            }
        })),
    )
        .into_response()
}

pub(crate) fn anthropic_error_response(
    status: StatusCode,
    message: &str,
    err_type: &str,
) -> axum::response::Response {
    (
        status,
        Json(json!({
            "type": "error",
            "error": {
                "type": err_type,
                "message": message,
            }
        })),
    )
        .into_response()
}

pub(crate) fn google_error_response(
    status: StatusCode,
    message: &str,
    err_status: &str,
) -> axum::response::Response {
    (
        status,
        Json(json!({
            "error": {
                "code": status.as_u16(),
                "message": message,
                "status": err_status
            }
        })),
    )
        .into_response()
}

pub(crate) fn missing_store() -> axum::response::Response {
    error_response(
        StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::Internal,
        "storage is not configured",
    )
}

pub(crate) fn unauthorized_admin() -> axum::response::Response {
    error_response(
        StatusCode::UNAUTHORIZED,
        ErrorCode::AuthError,
        "admin token required",
    )
}

pub(crate) fn is_anthropic_account(account: &OAuthAccountRecord) -> bool {
    account
        .provider_id
        .eq_ignore_ascii_case(ANTHROPIC_PROVIDER_ID)
        || account.upstream_base_url.as_deref() == Some(ANTHROPIC_UPSTREAM)
}

pub(crate) fn is_google_account(account: &OAuthAccountRecord) -> bool {
    account.provider_id.eq_ignore_ascii_case(GOOGLE_PROVIDER_ID)
}

pub(crate) fn is_custom_openai_api_account(account: &OAuthAccountRecord) -> bool {
    account
        .provider_id
        .eq_ignore_ascii_case(OPENAI_API_PROVIDER_ID)
}

pub(crate) fn is_openai_account(account: &OAuthAccountRecord) -> bool {
    account.provider_id.eq_ignore_ascii_case(OPENAI_PROVIDER_ID)
        || is_custom_openai_api_account(account)
        || (!is_anthropic_account(account) && !is_google_account(account))
}
