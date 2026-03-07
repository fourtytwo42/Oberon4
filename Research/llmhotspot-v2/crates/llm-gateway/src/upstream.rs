use crate::{GatewayError, GatewayErrorDetail};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::{json, Value};
use std::{collections::HashMap, str::FromStr, time::Duration};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpstreamRoutingProfile {
    pub base_url: Option<String>,
    pub chat_path: Option<String>,
    pub responses_path: Option<String>,
    pub org_id: Option<String>,
    pub project_id: Option<String>,
    pub extra_headers: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamRequestConfig {
    pub base_url: String,
    pub path: String,
    pub org_id: Option<String>,
    pub project_id: Option<String>,
    pub extra_headers: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct UpstreamJsonResponse {
    pub status: u16,
    pub body: Value,
}

#[derive(Debug)]
pub struct UpstreamSseResponse {
    pub status: u16,
    pub response: reqwest::Response,
}

pub fn resolve_request_config(
    default_base_url: &str,
    default_path: &str,
    is_responses: bool,
    profile: Option<&UpstreamRoutingProfile>,
) -> UpstreamRequestConfig {
    let default_chat_path = std::env::var("LLMHOTSPOT_OPENAI_CHAT_PATH")
        .unwrap_or_else(|_| "/v1/chat/completions".to_string());
    let default_responses_path = std::env::var("LLMHOTSPOT_OPENAI_RESPONSES_PATH")
        .unwrap_or_else(|_| "/v1/responses".to_string());
    let path = if let Some(p) = profile {
        if is_responses {
            p.responses_path.clone()
        } else {
            p.chat_path.clone()
        }
    } else {
        None
    }
    .unwrap_or_else(|| {
        if is_responses {
            if default_path == "/v1/responses" {
                default_responses_path
            } else {
                default_path.to_string()
            }
        } else if default_path == "/v1/chat/completions" {
            default_chat_path
        } else {
            default_path.to_string()
        }
    });

    let mut extra = HashMap::new();
    if let Some(p) = profile {
        extra.extend(p.extra_headers.clone());
    }

    UpstreamRequestConfig {
        base_url: profile
            .and_then(|p| p.base_url.clone())
            .unwrap_or_else(|| default_base_url.to_string()),
        path,
        org_id: profile.and_then(|p| p.org_id.clone()),
        project_id: profile.and_then(|p| p.project_id.clone()),
        extra_headers: extra,
    }
}

pub fn resolve_codex_oauth_config() -> UpstreamRequestConfig {
    let base_url = std::env::var("LLMHOTSPOT_CODEX_BASE_URL")
        .unwrap_or_else(|_| "https://chatgpt.com/backend-api".to_string());
    let path = std::env::var("LLMHOTSPOT_CODEX_RESPONSES_PATH")
        .unwrap_or_else(|_| "/codex/responses".to_string());
    UpstreamRequestConfig {
        base_url,
        path,
        org_id: None,
        project_id: None,
        extra_headers: HashMap::new(),
    }
}

pub async fn forward_openai_json(
    config: &UpstreamRequestConfig,
    access_token: &str,
    payload: &Value,
    request_headers: Option<&HashMap<String, String>>,
) -> Result<UpstreamJsonResponse, GatewayErrorDetail> {
    let url = format!("{}{}", config.base_url.trim_end_matches('/'), config.path);
    let headers = build_headers(config, access_token, request_headers)
        .map_err(|e| GatewayErrorDetail::new(e).with_url(url.clone()))?;
    trace_upstream(
        "openai_json_request",
        json!({
            "url": url,
            "headers": header_map_to_json(&headers),
            "payload": payload,
        }),
    );
    let response = build_client()
        .map_err(|e| GatewayErrorDetail::new(e).with_url(url.clone()))?
        .post(url.clone())
        .headers(headers)
        .json(payload)
        .send()
        .await
        .map_err(|e| map_transport_error(e, &url))?;

    let status = response.status().as_u16();
    let raw_body = response.text().await.map_err(|e| {
        GatewayErrorDetail::new(GatewayError::BadGateway)
            .with_url(url.clone())
            .with_transport_message(format!("failed reading upstream body: {e}"))
    })?;
    trace_upstream(
        "openai_json_response",
        json!({
            "url": url,
            "status": status,
            "body_raw": raw_body,
        }),
    );
    if status == 401 {
        return Err(err_with_body(GatewayError::Auth, &url, status, &raw_body));
    }
    if status == 429 {
        return Err(err_with_body(
            GatewayError::RateLimit,
            &url,
            status,
            &raw_body,
        ));
    }
    if status == 503 {
        return Err(err_with_body(
            GatewayError::TenantOffline,
            &url,
            status,
            &raw_body,
        ));
    }
    if status >= 500 {
        return Err(err_with_body(
            GatewayError::BadGateway,
            &url,
            status,
            &raw_body,
        ));
    }
    if !(200..300).contains(&status) {
        return Err(err_with_body(
            GatewayError::BadGateway,
            &url,
            status,
            &raw_body,
        ));
    }

    let body = serde_json::from_str::<Value>(&raw_body).map_err(|e| {
        err_with_body(
            GatewayError::BadGateway,
            &url,
            status,
            &format!("json parse error: {e}; body={raw_body}"),
        )
    })?;

    Ok(UpstreamJsonResponse { status, body })
}

pub async fn forward_openai_sse(
    config: &UpstreamRequestConfig,
    access_token: &str,
    payload: &Value,
    request_headers: Option<&HashMap<String, String>>,
) -> Result<UpstreamSseResponse, GatewayErrorDetail> {
    let url = format!("{}{}", config.base_url.trim_end_matches('/'), config.path);
    let headers = build_headers(config, access_token, request_headers)
        .map_err(|e| GatewayErrorDetail::new(e).with_url(url.clone()))?;
    trace_upstream(
        "openai_sse_request",
        json!({
            "url": url,
            "headers": header_map_to_json(&headers),
            "payload": payload,
        }),
    );
    let response = build_client()
        .map_err(|e| GatewayErrorDetail::new(e).with_url(url.clone()))?
        .post(url.clone())
        .headers(headers)
        .json(payload)
        .send()
        .await
        .map_err(|e| map_transport_error(e, &url))?;

    let status = response.status().as_u16();
    trace_upstream(
        "openai_sse_response",
        json!({
            "url": url,
            "status": status,
        }),
    );
    if !response.status().is_success() {
        let raw_body = response.text().await.unwrap_or_default();
        trace_upstream(
            "openai_sse_error_response",
            json!({
                "url": url,
                "status": status,
                "body_raw": raw_body,
            }),
        );
        if status == 401 {
            return Err(err_with_body(GatewayError::Auth, &url, status, &raw_body));
        }
        if status == 429 {
            return Err(err_with_body(
                GatewayError::RateLimit,
                &url,
                status,
                &raw_body,
            ));
        }
        if status == 503 {
            return Err(err_with_body(
                GatewayError::TenantOffline,
                &url,
                status,
                &raw_body,
            ));
        }
        if status >= 500 {
            return Err(err_with_body(
                GatewayError::BadGateway,
                &url,
                status,
                &raw_body,
            ));
        }
        return Err(err_with_body(
            GatewayError::BadGateway,
            &url,
            status,
            &raw_body,
        ));
    }

    Ok(UpstreamSseResponse { status, response })
}

fn env_u64(key: &str) -> Option<u64> {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
}

fn build_client() -> Result<reqwest::Client, GatewayError> {
    // IMPORTANT: This timeout applies to the full request, including streaming SSE bodies.
    // If it's too small, Codex tool-call streams (notably Cursor ApplyPatch custom tools)
    // get truncated, causing client "reconnecting" loops and invalid patch errors.
    let timeout_secs = env_u64("LLMHOTSPOT_UPSTREAM_TIMEOUT_SECS").unwrap_or(600);
    reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|_| GatewayError::BadGateway)
}

fn build_headers(
    config: &UpstreamRequestConfig,
    access_token: &str,
    request_headers: Option<&HashMap<String, String>>,
) -> Result<HeaderMap, GatewayError> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {access_token}"))
            .map_err(|_| GatewayError::BadGateway)?,
    );
    if let Some(org) = &config.org_id {
        headers.insert(
            HeaderName::from_static("openai-organization"),
            HeaderValue::from_str(org).map_err(|_| GatewayError::BadGateway)?,
        );
    }
    if let Some(project) = &config.project_id {
        headers.insert(
            HeaderName::from_static("openai-project"),
            HeaderValue::from_str(project).map_err(|_| GatewayError::BadGateway)?,
        );
    }

    for (key, value) in &config.extra_headers {
        if key.eq_ignore_ascii_case("authorization") {
            continue;
        }
        if let (Ok(name), Ok(val)) = (HeaderName::from_str(key), HeaderValue::from_str(value)) {
            headers.insert(name, val);
        }
    }
    if let Some(request_headers) = request_headers {
        for (key, value) in request_headers {
            if key.eq_ignore_ascii_case("authorization") {
                continue;
            }
            if let (Ok(name), Ok(val)) = (HeaderName::from_str(key), HeaderValue::from_str(value)) {
                headers.insert(name, val);
            }
        }
    }
    Ok(headers)
}

fn map_transport_error(err: reqwest::Error, url: &str) -> GatewayErrorDetail {
    trace_upstream(
        "openai_transport_error",
        json!({
            "is_timeout": err.is_timeout(),
            "message": err.to_string(),
        }),
    );
    if err.is_timeout() {
        GatewayErrorDetail::new(GatewayError::Timeout)
            .with_url(url.to_string())
            .with_transport_message(err.to_string())
    } else {
        GatewayErrorDetail::new(GatewayError::BadGateway)
            .with_url(url.to_string())
            .with_transport_message(err.to_string())
    }
}

fn snippet(s: &str) -> String {
    // Keep this safe for diagnostics: short, single-line-ish.
    let mut out = s.replace(['\r', '\n'], " ");
    out = out.trim().to_string();
    if out.len() > 240 {
        out.truncate(240);
        out.push_str("...");
    }
    out
}

fn err_with_body(kind: GatewayError, url: &str, status: u16, raw_body: &str) -> GatewayErrorDetail {
    GatewayErrorDetail::new(kind)
        .with_url(url.to_string())
        .with_status(status)
        .with_body_snippet(snippet(raw_body))
}

fn trace_upstream_enabled() -> bool {
    std::env::var("LLMHOTSPOT_TRACE_IO")
        .ok()
        .map(|v| {
            let n = v.trim().to_ascii_lowercase();
            n == "1" || n == "true" || n == "yes" || n == "on"
        })
        .unwrap_or(false)
}

fn trace_upstream(event: &str, payload: Value) {
    if !trace_upstream_enabled() {
        return;
    }
    eprintln!(
        "[llmhotspot-trace] {}",
        json!({
            "event": event,
            "payload": payload,
        })
    );
}

fn header_map_to_json(headers: &HeaderMap) -> Value {
    let mut out = serde_json::Map::new();
    for (name, value) in headers {
        let key = name.as_str().to_string();
        let raw = value.to_str().unwrap_or("<non-utf8>").to_string();
        let val = if key.eq_ignore_ascii_case("authorization") {
            if raw.len() <= 16 {
                "***".to_string()
            } else {
                format!("{}***{}", &raw[..8], &raw[raw.len() - 4..])
            }
        } else {
            raw
        };
        out.insert(key, Value::String(val));
    }
    Value::Object(out)
}
