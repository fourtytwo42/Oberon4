use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::Value;

pub fn decode_jwt_payload(token: &str) -> Option<serde_json::Value> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let bytes = URL_SAFE_NO_PAD.decode(parts.get(1)?.trim()).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn openai_chatgpt_account_id(access_token: &str, claim_path: &str) -> Option<String> {
    let payload = decode_jwt_payload(access_token)?;
    if let Some(account_id) = account_id_from_claim_path(&payload, claim_path) {
        return Some(account_id);
    }
    find_account_id_recursive(&payload)
}

fn account_id_from_claim_path(payload: &Value, claim_path: &str) -> Option<String> {
    let mut candidates = Vec::new();
    let raw = claim_path.trim();
    if !raw.is_empty() {
        candidates.push(raw.to_string());
        candidates.push(raw.trim_end_matches('/').to_string());
        candidates.push(format!("{}/", raw.trim_end_matches('/')));
        if raw.starts_with('/') {
            candidates.push(raw.to_string());
        } else {
            candidates.push(format!("/{}", raw));
        }
    }
    candidates.push("https://api.openai.com/auth".to_string());
    candidates.push("/https://api.openai.com/auth".to_string());
    candidates.push("auth".to_string());
    candidates.push("/auth".to_string());

    for candidate in candidates {
        if candidate.is_empty() {
            continue;
        }
        if let Some(v) = payload.get(&candidate) {
            if let Some(id) = extract_account_id_from_value(v) {
                return Some(id);
            }
        }
        if candidate.starts_with('/') {
            if let Some(v) = payload.pointer(&candidate) {
                if let Some(id) = extract_account_id_from_value(v) {
                    return Some(id);
                }
            }
        }
    }
    None
}

fn extract_account_id_from_value(v: &Value) -> Option<String> {
    let id = v.get("chatgpt_account_id")?.as_str()?.trim();
    if id.is_empty() {
        return None;
    }
    Some(id.to_string())
}

fn find_account_id_recursive(v: &Value) -> Option<String> {
    match v {
        Value::Object(obj) => {
            if let Some(id) = extract_account_id_from_value(v) {
                return Some(id);
            }
            for child in obj.values() {
                if let Some(id) = find_account_id_recursive(child) {
                    return Some(id);
                }
            }
            None
        }
        Value::Array(items) => {
            for item in items {
                if let Some(id) = find_account_id_recursive(item) {
                    return Some(id);
                }
            }
            None
        }
        _ => None,
    }
}

pub fn openai_email_from_access_token(access_token: &str) -> Option<String> {
    let payload = decode_jwt_payload(access_token)?;
    let email = payload.get("email")?.as_str()?;
    if email.trim().is_empty() {
        return None;
    }
    Some(email.to_string())
}
