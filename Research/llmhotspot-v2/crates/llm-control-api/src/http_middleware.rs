use super::*;
use axum::http::Uri;
pub(crate) async fn path_normalization_middleware(
    mut req: Request,
    next: Next,
) -> impl IntoResponse {
    let original = req.uri().clone();
    let normalized_path = normalize_incoming_path(original.path());
    if normalized_path != original.path() {
        let mut parts = original.into_parts();
        let with_query =
            if let Some(query) = parts.path_and_query.as_ref().and_then(|pq| pq.query()) {
                format!("{normalized_path}?{query}")
            } else {
                normalized_path
            };
        if let Ok(path_and_query) = with_query.parse() {
            parts.path_and_query = Some(path_and_query);
            if let Ok(uri) = Uri::from_parts(parts) {
                *req.uri_mut() = uri;
            }
        }
    }
    next.run(req).await
}

pub(crate) fn normalize_incoming_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut prev_was_slash = false;
    for ch in path.chars() {
        if ch == '/' {
            if !prev_was_slash {
                out.push(ch);
            }
            prev_was_slash = true;
        } else {
            out.push(ch);
            prev_was_slash = false;
        }
    }

    if out.len() > 1 && out.ends_with('/') {
        out.pop();
    }

    if out == "/v1/chat/completions/chat/completions" || out == "/v1/responses/chat/completions" {
        return "/v1/chat/completions".to_string();
    }
    if out == "/v1/responses/responses" || out == "/v1/chat/completions/responses" {
        return "/v1/responses".to_string();
    }

    out
}

pub(crate) async fn request_id_middleware(mut req: Request, next: Next) -> impl IntoResponse {
    let request_id = {
        req.headers()
            .get("x-request-id")
            .and_then(|h| h.to_str().ok())
            .filter(|v| !v.trim().is_empty())
            .unwrap_or("req-generated")
            .to_string()
    };
    if trace_io_enabled() {
        trace_io(
            "http_inbound",
            json!({
                "request_id": request_id,
                "method": req.method().to_string(),
                "path": req.uri().path().to_string(),
                "query": req.uri().query().unwrap_or(""),
                "headers": redact_header_map(req.headers()),
            }),
        );
    }
    req.extensions_mut().insert(request_id.clone());

    let mut resp = next.run(req).await;
    let rid = HeaderValue::from_str(&request_id)
        .unwrap_or_else(|_| HeaderValue::from_static("req-generated"));
    resp.headers_mut()
        .insert(HeaderName::from_static("x-request-id"), rid);
    resp
}
