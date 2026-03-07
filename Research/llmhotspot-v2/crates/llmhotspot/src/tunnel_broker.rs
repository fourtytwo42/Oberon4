use axum::{
    extract::{Path, Query, Request},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::IntoResponse,
    routing::{get, post},
    Extension, Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hmac::{Hmac, Mac};
use llm_storage::{
    AdminSessionRecord, AppOAuthCodeRecord, AppSessionRecord, AuditLogRecord,
    BillingEventAuditRecord, BillingReminderRecord, BrokerStore, CustomerRecord,
    DeviceBindingRecord, GiftEntitlementRecord, LicenseRecord, OtpChallengeRecord,
    OtpDeliveryEventRecord, StorageError, SubscriptionRecord, TunnelLeaseRecord,
};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::time::{sleep, Duration};

use crate::{env_nonempty, env_truthy, now_unix};

mod cloudflare;

const WEEK_SECS: u64 = 7 * 24 * 60 * 60;
const BILLING_GRACE_SECS: u64 = 30 * 24 * 60 * 60;
const OTP_REQUEST_WINDOW_SECS: u64 = 10 * 60;
const OTP_REQUEST_MAX_PER_EMAIL_WINDOW: u64 = 5;
const OTP_REQUEST_MAX_PER_IP_WINDOW: u64 = 20;
const OTP_VERIFY_MAX_ATTEMPTS: u64 = 5;
const ADMIN_LOGIN_WINDOW_SECS: u64 = 10 * 60;
const ADMIN_LOGIN_MAX_PER_IP_WINDOW: u64 = 20;
const ADMIN_SESSION_COOKIE_NAME: &str = "llmhotspot_admin_session";
const APP_OAUTH_CODE_TTL_SECS: u64 = 5 * 60;
const APP_SESSION_TTL_SECS: u64 = 30 * 24 * 60 * 60;
const REMINDER_BATCH_LIMIT: u32 = 100;

pub fn enabled() -> bool {
    env_truthy("LLMHOTSPOT_TUNNEL_BROKER")
}

pub fn router(store: Arc<dyn BrokerStore>) -> Router {
    Router::new()
        .route("/api/tunnels/provision", post(provision))
        .route("/api/tunnels/heartbeat", post(heartbeat))
        .route("/api/jobs/tunnels-reap", post(reap))
        .route(
            "/api/jobs/billing-reminders-dispatch",
            post(billing_reminders_dispatch),
        )
        .route("/api/endpoints/details", post(endpoint_details))
        .route("/api/endpoints/register", post(endpoint_register))
        .route("/api/licenses/activate", post(activate_license))
        .route("/api/app/oauth/authorize", post(app_oauth_authorize))
        .route("/api/app/oauth/token", post(app_oauth_token))
        .route("/api/app/session/status", post(app_session_status))
        .route("/api/customer/context", post(customer_context))
        .route(
            "/api/customer/portal-session",
            post(customer_portal_session),
        )
        .route("/api/customer/device/retire", post(customer_device_retire))
        .route(
            "/api/customer/device/reassign",
            post(customer_device_reassign),
        )
        .route(
            "/api/licenses/{license_key}/devices",
            get(list_license_devices),
        )
        .route(
            "/api/licenses/{license_key}/devices/{binding_id}/retire",
            post(retire_license_device),
        )
        .route("/api/otp/request", post(request_otp))
        .route("/api/otp/verify", post(verify_otp))
        .route("/admin/auth/login", post(admin_auth_login))
        .route("/admin/auth/logout", post(admin_auth_logout))
        .route("/admin/metrics", get(admin_metrics))
        .route("/admin/customers", get(admin_list_customers))
        .route("/admin/customer-tree", get(admin_customer_tree))
        .route("/admin/analytics/overview", get(admin_analytics_overview))
        .route("/admin/health", get(admin_health))
        .route("/admin/subscriptions", get(admin_list_subscriptions))
        .route("/admin/licenses", get(admin_list_licenses))
        .route("/admin/seats", get(admin_list_seats))
        .route("/admin/audit-logs", get(admin_list_audit_logs))
        .route(
            "/admin/export/customers.csv",
            get(admin_export_customers_csv),
        )
        .route(
            "/admin/export/subscriptions.csv",
            get(admin_export_subscriptions_csv),
        )
        .route("/admin/export/devices.csv", get(admin_export_devices_csv))
        .route("/admin/export/revenue.csv", get(admin_export_revenue_csv))
        .route(
            "/admin/subscriptions/{id}/update",
            post(admin_subscription_update),
        )
        .route(
            "/admin/subscriptions/{id}/cancel",
            post(admin_subscription_cancel),
        )
        .route(
            "/admin/subscriptions/{id}/suspend",
            post(admin_subscription_suspend),
        )
        .route(
            "/admin/subscriptions/{id}/unsuspend",
            post(admin_subscription_unsuspend),
        )
        .route("/admin/licenses/{id}/disable", post(admin_license_disable))
        .route("/admin/licenses/{id}/enable", post(admin_license_enable))
        .route("/admin/licenses/{id}/rotate", post(admin_license_rotate))
        .route("/admin/seats/{seat_id}/unassign", post(admin_seat_unassign))
        .route("/admin/seats/{seat_id}/reassign", post(admin_seat_reassign))
        .route("/admin/seats/gift", post(admin_gift_license))
        .route("/admin/licenses/gift", post(admin_gift_license))
        .route(
            "/admin/maintenance/reconcile",
            post(admin_reconcile_billing),
        )
        .route(
            "/admin/maintenance/replay-failed-events",
            post(admin_replay_failed_billing_events),
        )
        .route("/billing/create-checkout", post(create_checkout))
        .route(
            "/billing/create-portal-session",
            post(create_portal_session),
        )
        .route("/billing/webhook", post(stripe_webhook))
        // If Cloudflare Tunnel maps a path prefix like `/keys/*` to this service,
        // the prefix is not stripped. Serve the broker API under that prefix too.
        .route("/keys/api/tunnels/provision", post(provision))
        .route("/keys/api/tunnels/heartbeat", post(heartbeat))
        .route("/keys/api/jobs/tunnels-reap", post(reap))
        .route(
            "/keys/api/jobs/billing-reminders-dispatch",
            post(billing_reminders_dispatch),
        )
        .layer(middleware::from_fn(auth_middleware))
        .layer(Extension(store))
}

async fn auth_middleware(req: Request, next: Next) -> impl IntoResponse {
    let path = req.uri().path().to_string();
    let public_paths = ["/api/app/oauth/token", "/api/app/session/status"];
    if public_paths.iter().any(|p| path == *p) {
        return next.run(req).await;
    }
    if let Some(secret) = env_nonempty("LLMHOTSPOT_BROKER_SHARED_SECRET") {
        let provided = req
            .headers()
            .get("x-broker-secret")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if provided != secret {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"ok":false,"error":"unauthorized"})),
            )
                .into_response();
        }
    }
    next.run(req).await
}

fn hex_encode(bytes: &[u8]) -> String {
    const LUT: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(LUT[(b >> 4) as usize] as char);
        out.push(LUT[(b & 0x0f) as usize] as char);
    }
    out
}

fn default_slug_for_device(device_id: &str) -> String {
    let digest = Sha256::digest(device_id.as_bytes());
    let hex = hex_encode(digest.as_slice());
    format!("hs-{}", &hex[..10])
}

fn endpoints_base_domain() -> String {
    std::env::var("LLMHOTSPOT_ENDPOINTS_BASE_DOMAIN")
        .ok()
        .or_else(|| std::env::var("ENDPOINTS_BASE_DOMAIN").ok())
        .or_else(|| std::env::var("CF_BASE_DOMAIN").ok())
        .unwrap_or_else(|| "llmhotspot.com".to_string())
        .trim()
        .trim_start_matches('.')
        .to_string()
}

fn admin_secret() -> Option<String> {
    env_nonempty("LLMHOTSPOT_ADMIN_SECRET")
}

fn stripe_secret_key() -> Option<String> {
    env_nonempty("STRIPE_SECRET_KEY")
}

fn stripe_webhook_secret() -> Option<String> {
    env_nonempty("STRIPE_WEBHOOK_SECRET")
}

fn stripe_price_id() -> Option<String> {
    env_nonempty("STRIPE_PRICE_ID")
}

fn stripe_default_success_url() -> Option<String> {
    env_nonempty("BILLING_SUCCESS_URL")
}

fn stripe_default_cancel_url() -> Option<String> {
    env_nonempty("BILLING_CANCEL_URL")
}

fn reminder_webhook_base_url() -> String {
    env_nonempty("LLMHOTSPOT_REMINDER_WEBHOOK_URL")
        .unwrap_or_else(|| "https://llmhotspot.com".to_string())
}

fn reminder_internal_secret() -> Option<String> {
    env_nonempty("LLMHOTSPOT_REMINDER_INTERNAL_SECRET")
}

fn stripe_sandbox_mode() -> bool {
    env_truthy("LLMHOTSPOT_STRIPE_SANDBOX_MODE")
}

fn validate_stripe_secret_for_mode(secret: &str) -> Result<(), &'static str> {
    if stripe_sandbox_mode() && secret.starts_with("sk_live_") {
        return Err("stripe_mode_mismatch_live_key_in_sandbox");
    }
    Ok(())
}

fn stripe_api_base() -> String {
    env_nonempty("STRIPE_API_BASE").unwrap_or_else(|| "https://api.stripe.com".to_string())
}

fn otp_provider() -> String {
    env_nonempty("LLMHOTSPOT_OTP_PROVIDER").unwrap_or_else(|| "resend".to_string())
}

fn otp_from_email() -> Option<String> {
    env_nonempty("LLMHOTSPOT_OTP_FROM_EMAIL")
}

fn resend_api_key() -> Option<String> {
    env_nonempty("RESEND_API_KEY")
}

fn otp_dev_allow_no_sender() -> bool {
    env_truthy("LLMHOTSPOT_OTP_DEV_ALLOW_NO_SENDER")
}

fn admin_session_ttl_secs() -> u64 {
    env_nonempty("LLMHOTSPOT_ADMIN_SESSION_TTL_SECS")
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(8 * 60 * 60)
}

fn admin_allow_secret_fallback() -> bool {
    env_truthy("LLMHOTSPOT_ADMIN_ALLOW_SECRET_FALLBACK")
}

fn is_valid_license_key(raw: &str) -> bool {
    let v = raw.trim();
    !v.is_empty() && v.len() >= 8
}

fn hash_text(raw: &str) -> String {
    let digest = Sha256::digest(raw.as_bytes());
    hex_encode(digest.as_slice())
}

fn pkce_s256_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

async fn pick_effective_active_license_for_customer(
    store: &Arc<dyn BrokerStore>,
    customer_id: &str,
    now: u64,
) -> Result<Option<LicenseRecord>, StorageError> {
    let mut licenses = store.list_licenses_for_customer(customer_id).await?;
    licenses.sort_by(|a, b| b.updated_unix.cmp(&a.updated_unix));
    Ok(licenses
        .into_iter()
        .find(|l| license_is_effective_active(l, now)))
}

fn verify_stripe_webhook_signature(sig_header: &str, body: &str, secret: &str, now: u64) -> bool {
    let mut ts: Option<i64> = None;
    let mut v1: Option<&str> = None;
    for part in sig_header.split(',') {
        let mut it = part.splitn(2, '=');
        let key = it.next().unwrap_or("").trim();
        let val = it.next().unwrap_or("").trim();
        if key == "t" {
            if let Ok(v) = val.parse::<i64>() {
                ts = Some(v);
            }
        } else if key == "v1" {
            v1 = Some(val);
        }
    }
    let (Some(ts), Some(v1)) = (ts, v1) else {
        return false;
    };
    let age = (now as i64).saturating_sub(ts).abs();
    if age > 300 {
        return false;
    }

    let payload = format!("{ts}.{body}");
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = match HmacSha256::new_from_slice(secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(payload.as_bytes());
    let expected = hex_encode(&mac.finalize().into_bytes());
    expected.eq_ignore_ascii_case(v1)
}

fn gen_license_key() -> String {
    format!("LLMH-{}", uuid::Uuid::new_v4().simple())
}

fn ensure_plan_from_seat_limit(seat_limit: i64) -> String {
    if seat_limit <= 0 {
        "free".to_string()
    } else {
        "pro".to_string()
    }
}

fn period_seconds(raw: &str) -> u64 {
    match raw {
        "1d" => 24 * 60 * 60,
        "1w" => 7 * 24 * 60 * 60,
        "1m" => 30 * 24 * 60 * 60,
        "1y" => 365 * 24 * 60 * 60,
        _ => 7 * 24 * 60 * 60,
    }
}

fn period_bucket_seconds(raw: &str) -> u64 {
    match raw {
        "1d" => 60 * 60,
        "1w" => 24 * 60 * 60,
        "1m" => 24 * 60 * 60,
        "1y" => 7 * 24 * 60 * 60,
        _ => 24 * 60 * 60,
    }
}

fn rate_cents_for_seats(seats: i64) -> i64 {
    match seats {
        1 => 2000,
        2..=5 => 1800,
        6..=10 => 1700,
        11..=20 => 1600,
        21..=30 => 1400,
        31..=50 => 1200,
        _ => 1000,
    }
}

fn contracted_mrr_cents_for_seats(seats: i64) -> i64 {
    let bounded = seats.clamp(1, 100);
    bounded.saturating_mul(rate_cents_for_seats(bounded))
}

fn license_is_effective_active(license: &LicenseRecord, now: u64) -> bool {
    let status = license.status.to_ascii_lowercase();
    if status != "active" && status != "grace" {
        return false;
    }
    license.grace_expires_unix.map(|v| v >= now).unwrap_or(true)
}

fn generate_otp_code() -> String {
    let mut rng = rand::thread_rng();
    format!("{:06}", rng.gen_range(0..1_000_000))
}

fn client_ip_from_headers(headers: &HeaderMap) -> String {
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        let first = xff.split(',').next().unwrap_or("").trim();
        if !first.is_empty() {
            return first.to_string();
        }
    }
    headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn country_code_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get("cf-ipcountry")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().to_uppercase())
        .filter(|v| v.len() == 2 && v.chars().all(|c| c.is_ascii_alphabetic()))
}

fn ip_hash_from_headers(headers: &HeaderMap) -> Option<String> {
    let ip = client_ip_from_headers(headers);
    if ip == "unknown" {
        return None;
    }
    Some(hash_text(&ip))
}

fn is_unique_constraint(err: &str, table_col: &str) -> bool {
    if err.contains("UNIQUE constraint failed") && err.contains(table_col) {
        return true;
    }
    if err.contains("duplicate key value violates unique constraint") {
        if table_col.contains("device_bindings.license_id, device_id")
            && err.contains("device_bindings_license_id_device_id_key")
        {
            return true;
        }
        if table_col.contains("device_bindings.license_id, seat_index")
            && err.contains("device_bindings_license_id_seat_index_key")
        {
            return true;
        }
    }
    false
}

#[allow(clippy::too_many_arguments)] // Inputs are explicit request/context fields for provisioning logic.
async fn ensure_active_binding_for_device(
    store: &Arc<dyn BrokerStore>,
    license: &LicenseRecord,
    device_id: &str,
    device_name: Option<String>,
    os: Option<String>,
    app_version: Option<String>,
    label: Option<String>,
    country_code: Option<String>,
    last_ip_hash: Option<String>,
) -> Result<DeviceBindingRecord, (StatusCode, Json<Value>)> {
    let now = now_unix();
    let all_for_license = match store.list_bindings_for_license(&license.license_id).await {
        Ok(v) => v,
        Err(err) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            ));
        }
    };
    let existing_same_device = match store
        .get_active_binding_by_license_device(&license.license_id, device_id)
        .await
    {
        Ok(v) => v,
        Err(err) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            ));
        }
    };
    if let Some(existing) = existing_same_device {
        let mut updated = existing.clone();
        updated.last_seen_unix = now;
        if device_name.is_some() {
            updated.device_name = device_name;
        }
        if os.is_some() {
            updated.os = os;
        }
        if app_version.is_some() {
            updated.app_version = app_version;
        }
        if label.is_some() {
            updated.label = label;
        }
        if country_code.is_some() {
            updated.country_code = country_code;
        }
        if last_ip_hash.is_some() {
            updated.last_ip_hash = last_ip_hash;
        }
        if let Err(err) = store.upsert_device_binding(&updated).await {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            ));
        }
        return Ok(updated);
    }

    if let Some(existing_retired) = all_for_license
        .iter()
        .find(|b| b.device_id == device_id)
        .cloned()
    {
        let active = match store
            .list_active_bindings_for_license(&license.license_id)
            .await
        {
            Ok(v) => v,
            Err(err) => {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"ok":false,"error":format!("store error: {err}")})),
                ));
            }
        };
        let mut used = std::collections::BTreeSet::<i64>::new();
        for a in &active {
            used.insert(a.seat_index);
        }
        let mut reactivated = existing_retired;
        reactivated.status = "active".to_string();
        reactivated.retired_unix = None;
        reactivated.last_seen_unix = now;
        if device_name.is_some() {
            reactivated.device_name = device_name.clone();
        }
        if os.is_some() {
            reactivated.os = os.clone();
        }
        if app_version.is_some() {
            reactivated.app_version = app_version.clone();
        }
        if label.is_some() {
            reactivated.label = label.clone();
        }
        if country_code.is_some() {
            reactivated.country_code = country_code.clone();
        }
        if last_ip_hash.is_some() {
            reactivated.last_ip_hash = last_ip_hash.clone();
        }
        if reactivated.seat_index < 1
            || reactivated.seat_index > license.seat_limit
            || used.contains(&reactivated.seat_index)
        {
            let mut chosen = 1_i64;
            while used.contains(&chosen) && chosen <= license.seat_limit {
                chosen += 1;
            }
            if chosen > license.seat_limit {
                return Err((
                    StatusCode::CONFLICT,
                    Json(json!({"ok":false,"error":"seat_full"})),
                ));
            }
            reactivated.seat_index = chosen;
        }
        if let Err(err) = store.upsert_device_binding(&reactivated).await {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            ));
        }
        return Ok(reactivated);
    }

    const SEAT_ALLOC_RETRIES: usize = 4;
    for _ in 0..SEAT_ALLOC_RETRIES {
        let active = match store
            .list_active_bindings_for_license(&license.license_id)
            .await
        {
            Ok(v) => v,
            Err(err) => {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"ok":false,"error":format!("store error: {err}")})),
                ));
            }
        };
        if (active.len() as i64) >= license.seat_limit {
            return Err((
                StatusCode::CONFLICT,
                Json(json!({
                    "ok": false,
                    "error": "seat_full",
                    "activeSeats": active.len(),
                    "seatLimit": license.seat_limit
                })),
            ));
        }

        let mut used = std::collections::BTreeSet::<i64>::new();
        for a in &active {
            used.insert(a.seat_index);
        }
        let mut chosen = 1_i64;
        while used.contains(&chosen) && chosen <= license.seat_limit {
            chosen += 1;
        }
        if chosen > license.seat_limit {
            return Err((
                StatusCode::CONFLICT,
                Json(json!({"ok":false,"error":"seat_full"})),
            ));
        }

        let rec = DeviceBindingRecord {
            binding_id: uuid::Uuid::new_v4().to_string(),
            license_id: license.license_id.clone(),
            seat_index: chosen,
            device_id: device_id.to_string(),
            device_name: device_name.clone(),
            os: os.clone(),
            app_version: app_version.clone(),
            label: label.clone(),
            tunnel_id: None,
            tunnel_hostname: None,
            country_code: country_code.clone(),
            last_ip_hash: last_ip_hash.clone(),
            status: "active".to_string(),
            first_seen_unix: now,
            last_seen_unix: now,
            retired_unix: None,
        };
        match store.upsert_device_binding(&rec).await {
            Ok(()) => return Ok(rec),
            Err(err) => {
                let msg = err.to_string();
                if is_unique_constraint(&msg, "device_bindings.license_id, device_id") {
                    match store
                        .get_active_binding_by_license_device(&license.license_id, device_id)
                        .await
                    {
                        Ok(Some(existing)) => return Ok(existing),
                        Ok(None) => continue,
                        Err(err) => {
                            return Err((
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(json!({"ok":false,"error":format!("store error: {err}")})),
                            ));
                        }
                    }
                }
                if is_unique_constraint(&msg, "device_bindings.license_id, seat_index") {
                    continue;
                }
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"ok":false,"error":format!("store error: {err}")})),
                ));
            }
        }
    }
    Err((
        StatusCode::CONFLICT,
        Json(json!({"ok":false,"error":"seat_allocation_retry_exhausted"})),
    ))
}

async fn authorize_license_management(
    headers: &HeaderMap,
    store: &Arc<dyn BrokerStore>,
    license_id: &str,
) -> Result<(), (StatusCode, Json<Value>)> {
    if let Some(secret) = admin_secret() {
        let provided = headers
            .get("x-admin-secret")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if provided == secret {
            return Ok(());
        }
    }
    let challenge_id = headers
        .get("x-otp-challenge-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .trim();
    let otp_code = headers
        .get("x-otp-code")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .trim();
    if challenge_id.is_empty() || otp_code.is_empty() {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"management_auth_required"})),
        ));
    }
    let now = now_unix();
    let Some(challenge) = (match store.get_otp_challenge(challenge_id).await {
        Ok(v) => v,
        Err(err) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            ));
        }
    }) else {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"invalid_management_otp"})),
        ));
    };
    if challenge.consumed_unix.is_some() || challenge.expires_unix < now {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"invalid_management_otp"})),
        ));
    }
    let purpose = challenge.purpose.to_ascii_lowercase();
    if purpose != "management" && purpose != "license_management" {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"invalid_management_otp"})),
        ));
    }
    if let Some(target_license) = challenge.target_license_id.as_deref() {
        if !target_license.is_empty() && target_license != license_id {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({"ok":false,"error":"invalid_management_otp"})),
            ));
        }
    }
    if hash_text(otp_code) != challenge.code_hash {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"invalid_management_otp"})),
        ));
    }
    match store
        .consume_otp_challenge(&challenge.challenge_id, now)
        .await
    {
        Ok(true) => Ok(()),
        Ok(false) => Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"invalid_management_otp"})),
        )),
        Err(err) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":format!("store error: {err}")})),
        )),
    }
}

fn management_actor_from_headers(headers: &HeaderMap) -> (&'static str, String) {
    if headers.get("x-admin-secret").is_some() {
        ("admin_secret", "secret_fallback".to_string())
    } else if let Some(challenge) = headers
        .get("x-otp-challenge-id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        ("management_otp", challenge.to_string())
    } else {
        ("unknown", "unknown".to_string())
    }
}

fn cloudflare_client() -> cloudflare::CloudflareClient {
    let token = std::env::var("CF_API_TOKEN").unwrap_or_default();
    let account_id = std::env::var("CF_ACCOUNT_ID").unwrap_or_default();
    let zone_id = std::env::var("CF_ZONE_ID").unwrap_or_default();
    cloudflare::CloudflareClient::new(token, account_id, zone_id)
}

#[derive(Debug, Deserialize)]
struct ProvisionRequest {
    #[serde(default)]
    device_id: String,
    #[serde(default, rename = "deviceId")]
    device_id_compat: String,
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    device_name: Option<String>,
    #[serde(default, rename = "deviceName")]
    device_name_compat: Option<String>,
    // Compatibility: clients may send licenseKey (ignored in broker mode).
    #[serde(default, rename = "licenseKey")]
    license_key_compat: Option<String>,
    #[serde(default)]
    session_token: Option<String>,
    #[serde(default, rename = "sessionToken")]
    session_token_compat: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EndpointDetailsRequest {
    #[serde(default)]
    device_id: String,
    #[serde(default, rename = "deviceId")]
    device_id_compat: String,
    #[serde(default, rename = "licenseKey")]
    license_key_compat: Option<String>,
    #[serde(default)]
    session_token: Option<String>,
    #[serde(default, rename = "sessionToken")]
    session_token_compat: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EndpointRegisterRequest {
    #[serde(default)]
    device_id: String,
    #[serde(default, rename = "deviceId")]
    device_id_compat: String,
    slug: String,
    #[serde(default, rename = "licenseKey")]
    license_key_compat: Option<String>,
    #[serde(default)]
    session_token: Option<String>,
    #[serde(default, rename = "sessionToken")]
    session_token_compat: Option<String>,
}

fn request_device_id(device_id: &str, device_id_camel: &str) -> String {
    if !device_id.trim().is_empty() {
        device_id.trim().to_string()
    } else {
        device_id_camel.trim().to_string()
    }
}

fn request_session_token(session_token: Option<&str>, session_token_camel: Option<&str>) -> String {
    session_token
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .or_else(|| session_token_camel.map(str::trim).filter(|v| !v.is_empty()))
        .unwrap_or("")
        .to_string()
}

async fn resolve_effective_license_for_endpoint_access(
    store: &Arc<dyn BrokerStore>,
    device_id: &str,
    license_key: Option<&str>,
    session_token: Option<&str>,
) -> Result<LicenseRecord, (StatusCode, Json<Value>)> {
    let now = now_unix();
    let token = session_token
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("");
    if !token.is_empty() {
        let Some(session) = (match store.get_app_session(token).await {
            Ok(v) => v,
            Err(err) => {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"ok":false,"error":format!("store error: {err}")})),
                ))
            }
        }) else {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({"ok":false,"error":"unauthorized"})),
            ));
        };
        if session.revoked_unix.is_some()
            || session.expires_unix < now
            || session.device_id != device_id
        {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({"ok":false,"error":"unauthorized"})),
            ));
        }
        let Some(license) = (match store.get_license_by_id(&session.license_id).await {
            Ok(v) => v,
            Err(err) => {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"ok":false,"error":format!("store error: {err}")})),
                ))
            }
        }) else {
            return Err((
                StatusCode::NOT_FOUND,
                Json(json!({"ok":false,"error":"license_not_found"})),
            ));
        };
        if !license_is_effective_active(&license, now) {
            return Err((
                StatusCode::PAYMENT_REQUIRED,
                Json(json!({"ok":false,"error":"subscription_inactive"})),
            ));
        }
        return Ok(license);
    }

    let provided_key = license_key.unwrap_or("").trim().to_string();
    if !is_valid_license_key(&provided_key) {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"license_key_required"})),
        ));
    }
    let Some(license) = (match store.get_license_by_key(&provided_key).await {
        Ok(v) => v,
        Err(err) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            ))
        }
    }) else {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"license_not_found"})),
        ));
    };
    if !license_is_effective_active(&license, now) {
        return Err((
            StatusCode::PAYMENT_REQUIRED,
            Json(json!({"ok":false,"error":"subscription_inactive"})),
        ));
    }
    Ok(license)
}

fn endpoint_payload_for_hostname(hostname: &str) -> Value {
    let base = format!("https://{}", hostname.trim());
    json!({
        "publicBaseUrl": base,
        "remoteConsoleUrl": format!("{}/remote", base)
    })
}

fn sanitize_endpoint_slug(slug: &str) -> Option<String> {
    let normalized: String = slug
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    if normalized.len() < 3 || normalized.len() > 32 {
        return None;
    }
    if normalized.starts_with('-') || normalized.ends_with('-') {
        return None;
    }
    Some(normalized)
}

#[derive(Debug, Serialize)]
struct ProvisionResponse {
    ok: bool,
    tunnel: Value,
    lease: Value,
}

async fn provision(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
    Json(req): Json<ProvisionRequest>,
) -> impl IntoResponse {
    let device_id = request_device_id(&req.device_id, &req.device_id_compat);
    let device_name = req
        .device_name
        .as_deref()
        .or(req.device_name_compat.as_deref())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string());
    if device_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"deviceId is required"})),
        );
    }

    let mut seat_binding: Option<DeviceBindingRecord> = None;
    let country_code = country_code_from_headers(&headers);
    let last_ip_hash = ip_hash_from_headers(&headers);
    if env_truthy("LLMHOTSPOT_ENFORCE_LICENSES") {
        let session_token = request_session_token(
            req.session_token.as_deref(),
            req.session_token_compat.as_deref(),
        );
        let license = match resolve_effective_license_for_endpoint_access(
            &store,
            &device_id,
            req.license_key_compat.as_deref(),
            Some(&session_token),
        )
        .await
        {
            Ok(v) => v,
            Err(resp) => return resp,
        };
        if license.seat_limit <= 0 {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"ok":false,"error":"seat_limit_zero"})),
            );
        }
        let binding = match ensure_active_binding_for_device(
            &store,
            &license,
            &device_id,
            device_name,
            None,
            None,
            None,
            country_code,
            last_ip_hash,
        )
        .await
        {
            Ok(v) => v,
            Err(resp) => return resp,
        };
        seat_binding = Some(binding);
    }

    let cf = cloudflare_client();
    if !cf.is_configured() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":"cloudflare not configured"})),
        );
    }

    let now = now_unix();
    let existing = store.get_tunnel_lease(&device_id).await.ok().flatten();

    let requested_slug = req
        .slug
        .as_deref()
        .map(|v| v.trim().to_lowercase())
        .filter(|v| !v.is_empty());
    let seat_hostname = seat_binding
        .as_ref()
        .and_then(|b| b.tunnel_hostname.as_ref())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let seat_slug = seat_hostname
        .as_ref()
        .and_then(|hostname| sanitize_endpoint_slug(hostname.split('.').next().unwrap_or("")));
    let slug = requested_slug
        .clone()
        .or(seat_slug)
        .or_else(|| existing.as_ref().map(|l| l.endpoint_slug.clone()))
        .unwrap_or_else(|| default_slug_for_device(&device_id));
    let hostname = if requested_slug.is_none() {
        seat_hostname.unwrap_or_else(|| format!("{}.{}", slug, endpoints_base_domain()))
    } else {
        format!("{}.{}", slug, endpoints_base_domain())
    };

    let mut lease = TunnelLeaseRecord {
        device_id: device_id.clone(),
        endpoint_slug: slug.clone(),
        hostname: hostname.clone(),
        tunnel_id: seat_binding
            .as_ref()
            .and_then(|b| b.tunnel_id.clone())
            .or_else(|| existing.as_ref().and_then(|l| l.tunnel_id.clone())),
        dns_record_id: existing.as_ref().and_then(|l| l.dns_record_id.clone()),
        created_unix: existing.as_ref().map(|l| l.created_unix).unwrap_or(now),
        updated_unix: now,
        last_provisioned_unix: existing.as_ref().and_then(|l| l.last_provisioned_unix),
        last_seen_unix: existing.as_ref().and_then(|l| l.last_seen_unix),
    };

    let digest = Sha256::digest(device_id.as_bytes());
    let device_hash_full = hex_encode(digest.as_slice());
    let device_hash = &device_hash_full[..8];
    let tunnel_name = format!("llmhotspot-{}-{}", slug, device_hash);

    let mut token: Option<String> = None;
    let mut was_reprovisioned = false;

    if let Some(tunnel_id) = lease.tunnel_id.as_deref() {
        match cf.get_tunnel_token(tunnel_id).await {
            Ok(t) => token = Some(t),
            Err(_) => lease.tunnel_id = None,
        }
    }

    if lease.tunnel_id.is_none() {
        match cf.create_tunnel(&tunnel_name).await {
            Ok(created) => {
                lease.tunnel_id = Some(created.id.clone());
                token = Some(created.token);
                was_reprovisioned = true;
            }
            Err(err) => {
                // If the broker DB was lost but tunnels still exist, Cloudflare may reject
                // creating a tunnel with a duplicate name. In that case, recover by finding
                // the existing tunnel id and issuing a fresh token.
                let err_text = err.to_string();
                if err_text.contains("status=409")
                    || err_text.contains("already have a tunnel with this name")
                {
                    match cf.find_tunnel_id_by_name(&tunnel_name).await {
                        Ok(Some(existing_id)) => match cf.get_tunnel_token(&existing_id).await {
                            Ok(t) => {
                                lease.tunnel_id = Some(existing_id);
                                token = Some(t);
                                was_reprovisioned = true;
                            }
                            Err(err) => {
                                return (
                                    StatusCode::BAD_GATEWAY,
                                    Json(
                                        json!({"ok":false,"error":format!("cloudflare get token failed: {err}")}),
                                    ),
                                );
                            }
                        },
                        Ok(None) => {
                            return (
                                StatusCode::BAD_GATEWAY,
                                Json(
                                    json!({"ok":false,"error":format!("cloudflare create tunnel failed (name conflict) and lookup returned none: {err_text}")}),
                                ),
                            );
                        }
                        Err(err) => {
                            return (
                                StatusCode::BAD_GATEWAY,
                                Json(
                                    json!({"ok":false,"error":format!("cloudflare create tunnel failed (name conflict) and lookup failed: {err}")}),
                                ),
                            );
                        }
                    }
                } else {
                    return (
                        StatusCode::BAD_GATEWAY,
                        Json(
                            json!({"ok":false,"error":format!("cloudflare create tunnel failed: {err}")}),
                        ),
                    );
                }
            }
        }
    }

    let tunnel_id = lease.tunnel_id.clone().unwrap();
    match cf
        .upsert_tunnel_dns_record(&hostname, &tunnel_id, lease.dns_record_id.as_deref())
        .await
    {
        Ok(dns_id) => lease.dns_record_id = Some(dns_id),
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"ok":false,"error":format!("cloudflare dns upsert failed: {err}")})),
            );
        }
    }

    lease.last_provisioned_unix = Some(now);
    lease.last_seen_unix = Some(now);
    if let Err(err) = store.upsert_tunnel_lease(&lease).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":format!("store error: {err}")})),
        );
    }
    if let Some(mut binding) = seat_binding {
        binding.tunnel_id = Some(tunnel_id.clone());
        binding.tunnel_hostname = Some(hostname.clone());
        binding.last_seen_unix = now;
        if let Err(err) = store.upsert_device_binding(&binding).await {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    }

    let Some(token) = token else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":"missing tunnel token"})),
        );
    };

    let resp = ProvisionResponse {
        ok: true,
        tunnel: json!({
            "hostname": hostname,
            "tunnelId": tunnel_id,
            "tunnelToken": token,
        }),
        lease: json!({
            "deviceId": device_id,
            "endpointSlug": slug,
            "lastSeenUnix": now,
            "staleAfterSeconds": WEEK_SECS,
            "wasReprovisioned": was_reprovisioned,
        }),
    };
    (
        StatusCode::OK,
        Json(serde_json::to_value(resp).unwrap_or(json!({"ok":true}))),
    )
}

async fn endpoint_details(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    Json(req): Json<EndpointDetailsRequest>,
) -> impl IntoResponse {
    let device_id = request_device_id(&req.device_id, &req.device_id_compat);
    if device_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"deviceId is required"})),
        );
    }
    if env_truthy("LLMHOTSPOT_ENFORCE_LICENSES") {
        let session_token = request_session_token(
            req.session_token.as_deref(),
            req.session_token_compat.as_deref(),
        );
        if let Err(resp) = resolve_effective_license_for_endpoint_access(
            &store,
            &device_id,
            req.license_key_compat.as_deref(),
            Some(&session_token),
        )
        .await
        {
            return resp;
        }
    }
    let lease = match store.get_tunnel_lease(&device_id).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    };
    let endpoint = lease
        .as_ref()
        .map(|l| endpoint_payload_for_hostname(&l.hostname))
        .unwrap_or(Value::Null);
    (StatusCode::OK, Json(json!({"ok":true,"endpoint":endpoint})))
}

async fn endpoint_register(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    Json(req): Json<EndpointRegisterRequest>,
) -> impl IntoResponse {
    let device_id = request_device_id(&req.device_id, &req.device_id_compat);
    if device_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"deviceId is required"})),
        );
    }
    let Some(slug) = sanitize_endpoint_slug(&req.slug) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"reason":"invalid_slug"})),
        );
    };
    if env_truthy("LLMHOTSPOT_ENFORCE_LICENSES") {
        let session_token = request_session_token(
            req.session_token.as_deref(),
            req.session_token_compat.as_deref(),
        );
        if let Err(resp) = resolve_effective_license_for_endpoint_access(
            &store,
            &device_id,
            req.license_key_compat.as_deref(),
            Some(&session_token),
        )
        .await
        {
            return resp;
        }
    }

    let existing_for_slug = match store.get_tunnel_lease_by_slug(&slug).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    };
    if let Some(existing) = existing_for_slug {
        if existing.device_id != device_id {
            return (
                StatusCode::CONFLICT,
                Json(json!({"ok":false,"reason":"slug_taken"})),
            );
        }
    }

    let now = now_unix();
    let existing = match store.get_tunnel_lease(&device_id).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    };
    let hostname = format!("{}.{}", slug, endpoints_base_domain());
    let lease = TunnelLeaseRecord {
        device_id: device_id.clone(),
        endpoint_slug: slug.clone(),
        hostname: hostname.clone(),
        tunnel_id: existing.as_ref().and_then(|l| l.tunnel_id.clone()),
        dns_record_id: existing.as_ref().and_then(|l| l.dns_record_id.clone()),
        created_unix: existing.as_ref().map(|l| l.created_unix).unwrap_or(now),
        updated_unix: now,
        last_provisioned_unix: existing.as_ref().and_then(|l| l.last_provisioned_unix),
        last_seen_unix: existing
            .as_ref()
            .and_then(|l| l.last_seen_unix)
            .or(Some(now)),
    };
    if let Err(err) = store.upsert_tunnel_lease(&lease).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":format!("store error: {err}")})),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "endpoint": endpoint_payload_for_hostname(&hostname)
        })),
    )
}

#[derive(Debug, Deserialize)]
struct ActivateLicenseRequest {
    license_key: String,
    device_id: String,
    #[serde(default)]
    device_name: Option<String>,
    #[serde(default)]
    os: Option<String>,
    #[serde(default)]
    app_version: Option<String>,
    #[serde(default)]
    label: Option<String>,
}

async fn activate_license(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
    Json(req): Json<ActivateLicenseRequest>,
) -> impl IntoResponse {
    let now = now_unix();
    let license_key = req.license_key.trim();
    let device_id = req.device_id.trim();
    if !is_valid_license_key(license_key) || device_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"invalid_request"})),
        );
    }
    let Some(license) = (match store.get_license_by_key(license_key).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    }) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"license_not_found"})),
        );
    };
    if !license_is_effective_active(&license, now) {
        return (
            StatusCode::PAYMENT_REQUIRED,
            Json(json!({"ok":false,"error":"subscription_inactive"})),
        );
    }
    let rec = match ensure_active_binding_for_device(
        &store,
        &license,
        device_id,
        req.device_name.clone(),
        req.os.clone(),
        req.app_version.clone(),
        req.label.clone(),
        country_code_from_headers(&headers),
        ip_hash_from_headers(&headers),
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "licenseId": rec.license_id,
            "bindingId": rec.binding_id,
            "seatIndex": rec.seat_index,
            "seatLimit": license.seat_limit,
            "plan": license.plan_name
        })),
    )
}

#[derive(Debug, Deserialize)]
struct AppOAuthAuthorizeRequest {
    email: String,
    state: String,
    code_challenge: String,
    device_id: String,
}

async fn app_oauth_authorize(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    Json(req): Json<AppOAuthAuthorizeRequest>,
) -> impl IntoResponse {
    let email = req.email.trim().to_ascii_lowercase();
    let state = req.state.trim();
    let challenge = req.code_challenge.trim();
    let device_id = req.device_id.trim();
    if email.is_empty() || state.is_empty() || challenge.is_empty() || device_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"invalid_request"})),
        );
    }
    let now = now_unix();
    let Some(customer) = (match store.get_customer_by_email(&email).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            )
        }
    }) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"customer_not_found"})),
        );
    };
    let Some(license) =
        (match pick_effective_active_license_for_customer(&store, &customer.customer_id, now).await
        {
            Ok(v) => v,
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"ok":false,"error":format!("store error: {err}")})),
                )
            }
        })
    else {
        return (
            StatusCode::PAYMENT_REQUIRED,
            Json(json!({"ok":false,"error":"no_active_subscription"})),
        );
    };
    let code_id = format!("ac_{}", uuid::Uuid::new_v4().simple());
    let rec = AppOAuthCodeRecord {
        code_id: code_id.clone(),
        customer_id: customer.customer_id.clone(),
        license_id: license.license_id,
        state: state.to_string(),
        code_challenge: challenge.to_string(),
        device_id: device_id.to_string(),
        expires_unix: now.saturating_add(APP_OAUTH_CODE_TTL_SECS),
        consumed_unix: None,
        created_unix: now,
    };
    if let Err(err) = store.create_app_oauth_code(&rec).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":format!("store error: {err}")})),
        );
    }
    (
        StatusCode::OK,
        Json(json!({"ok":true,"code":code_id,"state":state})),
    )
}

#[derive(Debug, Deserialize)]
struct AppOAuthTokenRequest {
    code: String,
    code_verifier: String,
    device_id: String,
    #[serde(default)]
    device_name: Option<String>,
    #[serde(default)]
    os: Option<String>,
}

async fn app_oauth_token(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
    Json(req): Json<AppOAuthTokenRequest>,
) -> impl IntoResponse {
    let code = req.code.trim();
    let code_verifier = req.code_verifier.trim();
    let device_id = req.device_id.trim();
    let device_name = req
        .device_name
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string());
    let os = req
        .os
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string());
    if code.is_empty() || code_verifier.is_empty() || device_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"invalid_request"})),
        );
    }
    let now = now_unix();
    let Some(code_row) = (match store.get_app_oauth_code(code).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            )
        }
    }) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"invalid_or_expired_code"})),
        );
    };
    if code_row.consumed_unix.is_some() || code_row.expires_unix < now {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"invalid_or_expired_code"})),
        );
    }
    if code_row.device_id != device_id {
        return (
            StatusCode::CONFLICT,
            Json(json!({"ok":false,"error":"device_mismatch"})),
        );
    }
    let expected = pkce_s256_challenge(code_verifier);
    if expected != code_row.code_challenge {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"invalid_or_expired_code"})),
        );
    }
    let Some(license) = (match store.get_license_by_id(&code_row.license_id).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            )
        }
    }) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"license_not_found"})),
        );
    };
    if !license_is_effective_active(&license, now) {
        return (
            StatusCode::PAYMENT_REQUIRED,
            Json(json!({"ok":false,"error":"no_active_subscription"})),
        );
    }
    let binding = match ensure_active_binding_for_device(
        &store,
        &license,
        device_id,
        device_name,
        os,
        None,
        None,
        country_code_from_headers(&headers),
        ip_hash_from_headers(&headers),
    )
    .await
    {
        Ok(v) => v,
        Err((status, body)) => {
            let err = body
                .0
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("binding_failed");
            if err == "seat_limit_reached" {
                return (
                    StatusCode::CONFLICT,
                    Json(json!({"ok":false,"error":"seat_full"})),
                );
            }
            return (status, body);
        }
    };
    let _ = store.consume_app_oauth_code(code, now).await;
    let session_id = format!("aps_{}", uuid::Uuid::new_v4().simple());
    let expires = now.saturating_add(APP_SESSION_TTL_SECS);
    let session = AppSessionRecord {
        session_id: session_id.clone(),
        customer_id: code_row.customer_id,
        device_id: device_id.to_string(),
        license_id: license.license_id.clone(),
        seat_index: binding.seat_index,
        issued_unix: now,
        expires_unix: expires,
        revoked_unix: None,
    };
    if let Err(err) = store.create_app_session(&session).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":format!("store error: {err}")})),
        );
    }
    let _ = store.prune_expired_app_sessions(now).await;
    (
        StatusCode::OK,
        Json(json!({
            "ok":true,
            "sessionToken": session_id,
            "expiresUnix": expires,
            "tier":"pro",
            "licenseId": license.license_id,
            "seatIndex": binding.seat_index,
            "seatLimit": license.seat_limit,
            "capabilities": {
                "managed_tunnel": true,
                "max_enabled_oauth_accounts": -1,
                "max_api_keys": -1
            }
        })),
    )
}

#[derive(Debug, Deserialize)]
struct AppSessionStatusRequest {
    session_token: String,
    device_id: String,
}

async fn app_session_status(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    Json(req): Json<AppSessionStatusRequest>,
) -> impl IntoResponse {
    let session_token = req.session_token.trim();
    let device_id = req.device_id.trim();
    if session_token.is_empty() || device_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"invalid_request"})),
        );
    }
    let now = now_unix();
    let Some(session) = (match store.get_app_session(session_token).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            )
        }
    }) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"unauthorized"})),
        );
    };
    if session.revoked_unix.is_some()
        || session.expires_unix < now
        || session.device_id != device_id
    {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"unauthorized"})),
        );
    }
    let Some(license) = (match store.get_license_by_id(&session.license_id).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            )
        }
    }) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"license_not_found"})),
        );
    };
    // Session remains valid only while its device->seat binding is still active.
    let Some(binding) = (match store
        .get_active_binding_by_license_device(&session.license_id, device_id)
        .await
    {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            )
        }
    }) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"unauthorized"})),
        );
    };
    if binding.seat_index != session.seat_index {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"unauthorized"})),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "ok":true,
            "tier": if license_is_effective_active(&license, now) { "pro" } else { "free" },
            "licenseId": session.license_id,
            "seatIndex": session.seat_index,
            "seatLimit": license.seat_limit,
            "capabilities": {
                "managed_tunnel": license_is_effective_active(&license, now),
                "max_enabled_oauth_accounts": if license_is_effective_active(&license, now) { -1 } else { 1 },
                "max_api_keys": if license_is_effective_active(&license, now) { -1 } else { 1 }
            }
        })),
    )
}

async fn list_license_devices(
    Path(license_key): Path<String>,
    headers: HeaderMap,
    Extension(store): Extension<Arc<dyn BrokerStore>>,
) -> impl IntoResponse {
    let Some(license) = (match store.get_license_by_key(&license_key).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    }) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"license_not_found"})),
        );
    };
    if let Err(resp) = authorize_license_management(&headers, &store, &license.license_id).await {
        return resp;
    }
    let rows = match store.list_bindings_for_license(&license.license_id).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    };
    let (actor_type, actor_id) = management_actor_from_headers(&headers);
    append_audit_log(
        &store,
        actor_type,
        &actor_id,
        "license.devices.list",
        "license",
        &license.license_id,
        json!({}),
        source_ip_from_headers(&headers),
        "success",
    )
    .await;
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "licenseId": license.license_id,
            "seatLimit": license.seat_limit,
            "devices": rows
        })),
    )
}

async fn retire_license_device(
    Path((license_key, binding_id)): Path<(String, String)>,
    headers: HeaderMap,
    Extension(store): Extension<Arc<dyn BrokerStore>>,
) -> impl IntoResponse {
    let Some(license) = (match store.get_license_by_key(&license_key).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    }) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"license_not_found"})),
        );
    };
    if let Err(resp) = authorize_license_management(&headers, &store, &license.license_id).await {
        return resp;
    }
    let rows = match store.list_bindings_for_license(&license.license_id).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    };
    let binding_device_id = rows
        .iter()
        .find(|r| r.binding_id == binding_id)
        .map(|r| r.device_id.clone());
    if binding_device_id.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"binding_not_found"})),
        );
    }
    let now = now_unix();
    match store.retire_binding(&binding_id, now).await {
        Ok(true) => {
            if let Some(device_id) = binding_device_id {
                let _ = store.clear_tunnel_lease_provisioning(&device_id, now).await;
            }
            let (actor_type, actor_id) = management_actor_from_headers(&headers);
            append_audit_log(
                &store,
                actor_type,
                &actor_id,
                "license.devices.retire",
                "binding",
                &binding_id,
                json!({"license_id": license.license_id}),
                source_ip_from_headers(&headers),
                "success",
            )
            .await;
            (StatusCode::OK, Json(json!({"ok":true})))
        }
        Ok(false) => (
            StatusCode::CONFLICT,
            Json(json!({"ok":false,"error":"binding_not_active"})),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":format!("store error: {err}")})),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct OtpRequestPayload {
    email: String,
    #[serde(default)]
    purpose: Option<String>,
    #[serde(default)]
    target_license_id: Option<String>,
}

async fn send_otp_via_resend(email: &str, code: &str) -> Result<(), String> {
    let api_key = resend_api_key().ok_or_else(|| "RESEND_API_KEY not configured".to_string())?;
    let from =
        otp_from_email().ok_or_else(|| "LLMHOTSPOT_OTP_FROM_EMAIL not configured".to_string())?;
    let body = json!({
        "from": from,
        "to": [email],
        "subject": "Your LLM Hotspot verification code",
        "text": format!("Your verification code is: {code}\n\nThis code expires in 10 minutes."),
    });
    let url = "https://api.resend.com/emails";
    let client = reqwest::Client::new();
    let resp = client
        .post(url)
        .header("authorization", format!("Bearer {api_key}"))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("resend request failed: {e}"))?;
    if resp.status().is_success() {
        return Ok(());
    }
    let status = resp.status().as_u16();
    let txt = resp.text().await.unwrap_or_default();
    Err(format!("resend error status={status} body={txt}"))
}

async fn deliver_otp_email(
    store: &Arc<dyn BrokerStore>,
    challenge_id: &str,
    email: &str,
    code: &str,
    now: u64,
) -> Result<(), String> {
    let provider = otp_provider().to_ascii_lowercase();
    let delivery_res = match provider.as_str() {
        "resend" => send_otp_via_resend(email, code).await,
        _ => Err(format!("unsupported otp provider: {provider}")),
    };
    let event = OtpDeliveryEventRecord {
        event_id: format!("ode_{}", uuid::Uuid::new_v4().simple()),
        challenge_id: challenge_id.to_string(),
        email: email.to_string(),
        provider: provider.clone(),
        status: if delivery_res.is_ok() {
            "sent".to_string()
        } else {
            "failed".to_string()
        },
        error: delivery_res.as_ref().err().cloned(),
        created_unix: now,
    };
    let _ = store.insert_otp_delivery_event(&event).await;
    delivery_res
}

async fn request_otp(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
    Json(req): Json<OtpRequestPayload>,
) -> impl IntoResponse {
    let email = req.email.trim().to_ascii_lowercase();
    if email.is_empty() || !email.contains('@') {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"invalid_email"})),
        );
    }
    let now = now_unix();
    let ip = client_ip_from_headers(&headers);
    let email_bucket = format!("otp_req_email:{email}");
    let ip_bucket = format!("otp_req_ip:{ip}");
    let email_attempts = match store
        .otp_rate_limit_attempt(&email_bucket, now, OTP_REQUEST_WINDOW_SECS)
        .await
    {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    };
    let ip_attempts = match store
        .otp_rate_limit_attempt(&ip_bucket, now, OTP_REQUEST_WINDOW_SECS)
        .await
    {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    };
    if email_attempts > OTP_REQUEST_MAX_PER_EMAIL_WINDOW
        || ip_attempts > OTP_REQUEST_MAX_PER_IP_WINDOW
    {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"ok":false,"error":"otp_rate_limited"})),
        );
    }

    let code = generate_otp_code();
    let rec = OtpChallengeRecord {
        challenge_id: uuid::Uuid::new_v4().to_string(),
        email,
        code_hash: hash_text(&code),
        purpose: req
            .purpose
            .as_deref()
            .unwrap_or("management")
            .trim()
            .to_string(),
        target_license_id: req.target_license_id.clone(),
        expires_unix: now.saturating_add(10 * 60),
        consumed_unix: None,
        created_unix: now,
    };
    if let Err(err) = store.upsert_otp_challenge(&rec).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":format!("store error: {err}")})),
        );
    }
    if let Err(err) = deliver_otp_email(&store, &rec.challenge_id, &rec.email, &code, now).await {
        if !otp_dev_allow_no_sender() {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"ok":false,"error":format!("otp_delivery_failed: {err}")})),
            );
        }
    }
    (
        StatusCode::OK,
        Json(json!({"ok":true,"challengeId":rec.challenge_id,"expiresUnix":rec.expires_unix})),
    )
}

#[derive(Debug, Deserialize)]
struct OtpVerifyPayload {
    challenge_id: String,
    code: String,
}

async fn verify_otp(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    Json(req): Json<OtpVerifyPayload>,
) -> impl IntoResponse {
    let now = now_unix();
    let Some(challenge) = (match store.get_otp_challenge(&req.challenge_id).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    }) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"challenge_not_found"})),
        );
    };
    if challenge.consumed_unix.is_some() {
        return (
            StatusCode::CONFLICT,
            Json(json!({"ok":false,"error":"challenge_consumed"})),
        );
    }
    if challenge.expires_unix < now {
        return (
            StatusCode::GONE,
            Json(json!({"ok":false,"error":"challenge_expired"})),
        );
    }
    let attempt_count = match store.otp_verify_attempt(&challenge.challenge_id, now).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    };
    if attempt_count > OTP_VERIFY_MAX_ATTEMPTS {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"ok":false,"error":"otp_attempt_limit_exceeded"})),
        );
    }
    if hash_text(req.code.trim()) != challenge.code_hash {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"invalid_code"})),
        );
    }
    match store
        .consume_otp_challenge(&challenge.challenge_id, now)
        .await
    {
        Ok(true) => {
            let _ = store
                .clear_otp_verify_attempts(&challenge.challenge_id)
                .await;
            (
                StatusCode::OK,
                Json(json!({"ok":true,"email":challenge.email,"purpose":challenge.purpose})),
            )
        }
        Ok(false) => (
            StatusCode::CONFLICT,
            Json(json!({"ok":false,"error":"challenge_consumed"})),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":format!("store error: {err}")})),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct CustomerOtpPayload {
    email: String,
    #[serde(default)]
    challenge_id: Option<String>,
    #[serde(default)]
    code: Option<String>,
}

async fn verify_customer_otp(
    store: &Arc<dyn BrokerStore>,
    payload: &CustomerOtpPayload,
) -> Result<(), (StatusCode, Json<Value>)> {
    let email = payload.email.trim().to_ascii_lowercase();
    let challenge_id = payload.challenge_id.as_deref().unwrap_or("").trim();
    let code = payload.code.as_deref().unwrap_or("").trim();
    if email.is_empty() || challenge_id.is_empty() || code.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"invalid_request"})),
        ));
    }
    let Some(challenge) = (match store.get_otp_challenge(challenge_id).await {
        Ok(v) => v,
        Err(err) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            ));
        }
    }) else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"challenge_not_found"})),
        ));
    };
    if challenge.email.to_ascii_lowercase() != email {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"invalid_code"})),
        ));
    }
    let purpose = challenge.purpose.trim().to_ascii_lowercase();
    if purpose != "management" && purpose != "license_management" {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"invalid_code"})),
        ));
    }
    let now = now_unix();
    if challenge.expires_unix < now || challenge.consumed_unix.is_some() {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"challenge_expired_or_consumed"})),
        ));
    }
    let attempts = match store.otp_verify_attempt(&challenge.challenge_id, now).await {
        Ok(v) => v,
        Err(err) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            ));
        }
    };
    if attempts > OTP_VERIFY_MAX_ATTEMPTS {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"ok":false,"error":"otp_attempt_limit_exceeded"})),
        ));
    }
    if hash_text(code) != challenge.code_hash {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"invalid_code"})),
        ));
    }
    match store
        .consume_otp_challenge(&challenge.challenge_id, now)
        .await
    {
        Ok(true) => {
            let _ = store
                .clear_otp_verify_attempts(&challenge.challenge_id)
                .await;
            Ok(())
        }
        Ok(false) => Err((
            StatusCode::CONFLICT,
            Json(json!({"ok":false,"error":"challenge_consumed"})),
        )),
        Err(err) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":format!("store error: {err}")})),
        )),
    }
}

async fn customer_context(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    Json(payload): Json<CustomerOtpPayload>,
) -> impl IntoResponse {
    let challenge_id = payload.challenge_id.as_deref().unwrap_or("").trim();
    let code = payload.code.as_deref().unwrap_or("").trim();
    if !challenge_id.is_empty() || !code.is_empty() {
        if let Err(resp) = verify_customer_otp(&store, &payload).await {
            return resp;
        }
    }
    let email = payload.email.trim().to_ascii_lowercase();
    let Some(customer) = (match store.get_customer_by_email(&email).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    }) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"customer_not_found"})),
        );
    };

    let licenses = match store
        .list_licenses_for_customer(&customer.customer_id)
        .await
    {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    };
    let subscriptions = match store
        .list_subscriptions_for_customer(&customer.customer_id)
        .await
    {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    };

    let mut license_items = Vec::<Value>::new();
    for license in licenses {
        let bindings = match store.list_bindings_for_license(&license.license_id).await {
            Ok(v) => v,
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"ok":false,"error":format!("store error: {err}")})),
                );
            }
        };
        let active_seats = bindings.iter().filter(|b| b.status == "active").count();
        license_items.push(json!({
            "licenseId": license.license_id,
            "licenseKey": license.license_key,
            "status": license.status,
            "plan": license.plan_name,
            "seatLimit": license.seat_limit,
            "activeSeats": active_seats,
            "subscriptionId": license.subscription_id,
            "graceExpiresUnix": license.grace_expires_unix,
            "createdUnix": license.created_unix,
            "updatedUnix": license.updated_unix,
            "bindings": bindings.iter().map(|b| json!({
                "bindingId": b.binding_id,
                "seatIndex": b.seat_index,
                "deviceId": b.device_id,
                "deviceName": b.device_name,
                "os": b.os,
                "countryCode": b.country_code,
                "appVersion": b.app_version,
                "label": b.label,
                "status": b.status,
                "firstSeenUnix": b.first_seen_unix,
                "lastSeenUnix": b.last_seen_unix
            })).collect::<Vec<_>>()
        }));
    }

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "customer": {
                "customerId": customer.customer_id,
                "email": customer.email,
                "stripeCustomerId": customer.stripe_customer_id
            },
            "subscriptions": subscriptions.iter().map(|s| json!({
                "subscriptionId": s.subscription_id,
                "status": s.status,
                "seatLimit": s.seat_limit,
                "currentPeriodEndUnix": s.current_period_end_unix,
                "graceExpiresUnix": s.grace_expires_unix,
                "createdUnix": s.created_unix,
                "updatedUnix": s.updated_unix
            })).collect::<Vec<_>>(),
            "licenses": license_items
        })),
    )
}

#[derive(Debug, Deserialize)]
struct CustomerPortalSessionPayload {
    email: String,
    #[serde(default)]
    challenge_id: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    return_url: Option<String>,
}

async fn customer_portal_session(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    Json(payload): Json<CustomerPortalSessionPayload>,
) -> impl IntoResponse {
    let challenge_id = payload.challenge_id.as_deref().unwrap_or("").trim();
    let code = payload.code.as_deref().unwrap_or("").trim();
    if !challenge_id.is_empty() || !code.is_empty() {
        if let Err(resp) = verify_customer_otp(
            &store,
            &CustomerOtpPayload {
                email: payload.email.clone(),
                challenge_id: payload.challenge_id.clone(),
                code: payload.code.clone(),
            },
        )
        .await
        {
            return resp;
        }
    }

    let email = payload.email.trim().to_ascii_lowercase();
    let Some(customer) = (match store.get_customer_by_email(&email).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    }) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"customer_not_found"})),
        );
    };
    let Some(customer_id) = customer.stripe_customer_id.as_deref() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"stripe_customer_missing"})),
        );
    };
    let Some(secret) = stripe_secret_key() else {
        return (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({"ok":false,"error":"stripe_not_configured"})),
        );
    };
    if let Err(code) = validate_stripe_secret_for_mode(&secret) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "error": code,
                "hint": "Set STRIPE_SECRET_KEY to a test key (sk_test_...) or disable LLMHOTSPOT_STRIPE_SANDBOX_MODE."
            })),
        );
    }
    let return_url = payload
        .return_url
        .or_else(stripe_default_cancel_url)
        .unwrap_or_default();
    if return_url.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"missing_return_url"})),
        );
    }

    let form = vec![
        ("customer".to_string(), customer_id.to_string()),
        ("return_url".to_string(), return_url),
    ];
    let url = format!("{}/v1/billing_portal/sessions", stripe_api_base());
    let client = reqwest::Client::new();
    let resp = match client
        .post(url)
        .bearer_auth(secret)
        .form(&form)
        .send()
        .await
    {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"ok":false,"error":format!("stripe_request_failed: {err}")})),
            );
        }
    };
    let status = resp.status();
    let value: Value = match resp.json().await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"ok":false,"error":format!("stripe_decode_failed: {err}")})),
            );
        }
    };
    if !status.is_success() {
        return (
            StatusCode::BAD_GATEWAY,
            Json(
                json!({"ok":false,"error":"stripe_portal_failed","status":status.as_u16(),"stripe":value}),
            ),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "portalUrl": value.get("url").and_then(Value::as_str).unwrap_or("")
        })),
    )
}

#[derive(Debug, Deserialize)]
struct CustomerDeviceRetirePayload {
    email: String,
    binding_id: String,
}

async fn customer_device_retire(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    Json(payload): Json<CustomerDeviceRetirePayload>,
) -> impl IntoResponse {
    let email = payload.email.trim().to_ascii_lowercase();
    let binding_id = payload.binding_id.trim();
    if email.is_empty() || binding_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"invalid_request"})),
        );
    }
    let Some(customer) = (match store.get_customer_by_email(&email).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    }) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"customer_not_found"})),
        );
    };
    let Some(binding) = (match store.get_binding_by_id(binding_id).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    }) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"binding_not_found"})),
        );
    };
    let Some(license) = (match store.get_license_by_id(&binding.license_id).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    }) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"license_not_found"})),
        );
    };
    if license.customer_id != customer.customer_id {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"ownership_mismatch"})),
        );
    }
    let now = now_unix();
    match store.retire_binding(binding_id, now).await {
        Ok(true) => {
            let _ = store
                .clear_tunnel_lease_provisioning(&binding.device_id, now)
                .await;
            (StatusCode::OK, Json(json!({"ok":true})))
        }
        Ok(false) => (
            StatusCode::CONFLICT,
            Json(json!({"ok":false,"error":"binding_not_active"})),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":format!("store error: {err}")})),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct CustomerDeviceReassignPayload {
    email: String,
    binding_id: String,
    device_id: String,
    #[serde(default)]
    device_name: Option<String>,
    #[serde(default)]
    os: Option<String>,
    #[serde(default)]
    app_version: Option<String>,
    #[serde(default)]
    label: Option<String>,
}

async fn customer_device_reassign(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    Json(payload): Json<CustomerDeviceReassignPayload>,
) -> impl IntoResponse {
    let email = payload.email.trim().to_ascii_lowercase();
    let binding_id = payload.binding_id.trim();
    let device_id = payload.device_id.trim();
    if email.is_empty() || binding_id.is_empty() || device_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"invalid_request"})),
        );
    }
    let Some(customer) = (match store.get_customer_by_email(&email).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    }) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"customer_not_found"})),
        );
    };
    let Some(mut binding) = (match store.get_binding_by_id(binding_id).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    }) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"binding_not_found"})),
        );
    };
    let Some(license) = (match store.get_license_by_id(&binding.license_id).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    }) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"license_not_found"})),
        );
    };
    if license.customer_id != customer.customer_id {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"ownership_mismatch"})),
        );
    }
    if !license_is_effective_active(&license, now_unix()) {
        return (
            StatusCode::CONFLICT,
            Json(json!({"ok":false,"error":"license_not_active"})),
        );
    }
    if let Ok(Some(other)) = store
        .get_active_binding_by_license_device(&binding.license_id, device_id)
        .await
    {
        if other.binding_id != binding_id {
            return (
                StatusCode::CONFLICT,
                Json(json!({"ok":false,"error":"device_already_assigned"})),
            );
        }
    }

    binding.device_id = device_id.to_string();
    binding.device_name = payload.device_name;
    binding.os = payload.os;
    binding.app_version = payload.app_version;
    binding.label = payload.label;
    binding.status = "active".to_string();
    binding.last_seen_unix = now_unix();
    binding.retired_unix = None;
    if let Err(err) = store.upsert_device_binding(&binding).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":format!("store error: {err}")})),
        );
    }
    (
        StatusCode::OK,
        Json(
            json!({"ok":true,"bindingId":binding_id,"deviceId":binding.device_id,"status":"active"}),
        ),
    )
}

#[derive(Debug, Clone)]
struct AdminAuthContext {
    actor_type: String,
    actor_id: String,
    source_ip: Option<String>,
    session_id: Option<String>,
}

fn source_ip_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(',').next().unwrap_or("").trim().to_string())
        .filter(|v| !v.is_empty())
}

fn user_agent_hash_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(hash_text)
}

fn admin_session_from_headers(headers: &HeaderMap) -> Option<String> {
    if let Some(token) = headers
        .get("x-admin-session")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        return Some(token.to_string());
    }
    let cookie_header = headers.get(header::COOKIE).and_then(|v| v.to_str().ok())?;
    for part in cookie_header.split(';') {
        let mut it = part.trim().splitn(2, '=');
        let key = it.next().unwrap_or("").trim();
        let val = it.next().unwrap_or("").trim();
        if key == ADMIN_SESSION_COOKIE_NAME && !val.is_empty() {
            return Some(val.to_string());
        }
    }
    None
}

#[allow(clippy::too_many_arguments)] // Audit rows are assembled from explicit actor/target fields at call sites.
async fn append_audit_log(
    store: &Arc<dyn BrokerStore>,
    actor_type: &str,
    actor_id: &str,
    action: &str,
    target_type: &str,
    target_id: &str,
    payload_json: Value,
    ip: Option<String>,
    outcome: &str,
) {
    let row = AuditLogRecord {
        audit_id: format!("aud_{}", uuid::Uuid::new_v4().simple()),
        actor_type: actor_type.to_string(),
        actor_id: actor_id.to_string(),
        action: action.to_string(),
        target_type: target_type.to_string(),
        target_id: target_id.to_string(),
        payload_json: payload_json.to_string(),
        created_unix: now_unix(),
        ip,
        outcome: outcome.to_string(),
    };
    let _ = store.insert_audit_log(&row).await;
}

async fn require_admin(
    headers: &HeaderMap,
    store: &Arc<dyn BrokerStore>,
) -> Result<AdminAuthContext, (StatusCode, Json<Value>)> {
    if let Some(session_id) = admin_session_from_headers(headers) {
        let now = now_unix();
        if let Ok(Some(session)) = store.get_admin_session(&session_id).await {
            if session.revoked_unix.is_none() && session.expires_unix >= now {
                return Ok(AdminAuthContext {
                    actor_type: "admin_session".to_string(),
                    actor_id: session_id.clone(),
                    source_ip: source_ip_from_headers(headers),
                    session_id: Some(session_id),
                });
            }
        }
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"unauthorized"})),
        ));
    }
    if admin_allow_secret_fallback() {
        let Some(secret) = admin_secret() else {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({"ok":false,"error":"admin_secret_not_configured"})),
            ));
        };
        let provided = headers
            .get("x-admin-secret")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if provided == secret {
            return Ok(AdminAuthContext {
                actor_type: "admin_secret".to_string(),
                actor_id: "secret_fallback".to_string(),
                source_ip: source_ip_from_headers(headers),
                session_id: None,
            });
        }
    }
    Err((
        StatusCode::UNAUTHORIZED,
        Json(json!({"ok":false,"error":"unauthorized"})),
    ))
}

#[derive(Debug, Deserialize)]
struct AdminAuthLoginPayload {
    admin_secret: String,
}

async fn admin_auth_login(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
    Json(payload): Json<AdminAuthLoginPayload>,
) -> axum::response::Response {
    let ip = source_ip_from_headers(&headers).unwrap_or_else(|| "unknown".to_string());
    let now = now_unix();
    let bucket = format!("admin_login_ip:{ip}");
    let attempts = match store
        .otp_rate_limit_attempt(&bucket, now, ADMIN_LOGIN_WINDOW_SECS)
        .await
    {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            )
                .into_response();
        }
    };
    if attempts > ADMIN_LOGIN_MAX_PER_IP_WINDOW {
        append_audit_log(
            &store,
            "admin_auth",
            "login",
            "admin.auth.login",
            "admin_session",
            "",
            json!({"ip": ip, "reason":"rate_limited"}),
            Some(ip),
            "rate_limited",
        )
        .await;
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"ok":false,"error":"admin_login_rate_limited"})),
        )
            .into_response();
    }
    let Some(secret) = admin_secret() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"admin_secret_not_configured"})),
        )
            .into_response();
    };
    if payload.admin_secret != secret {
        append_audit_log(
            &store,
            "admin_auth",
            "login",
            "admin.auth.login",
            "admin_session",
            "",
            json!({"ip": ip, "reason":"invalid_secret"}),
            Some(ip),
            "unauthorized",
        )
        .await;
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"unauthorized"})),
        )
            .into_response();
    }
    let ttl = admin_session_ttl_secs();
    let session_id = format!("as_{}", uuid::Uuid::new_v4().simple());
    let row = AdminSessionRecord {
        session_id: session_id.clone(),
        issued_unix: now,
        expires_unix: now.saturating_add(ttl),
        revoked_unix: None,
        source_ip: Some(ip.clone()),
        user_agent_hash: user_agent_hash_from_headers(&headers),
        created_by: "bootstrap_secret".to_string(),
    };
    if let Err(err) = store.create_admin_session(&row).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":format!("store error: {err}")})),
        )
            .into_response();
    }
    let _ = store.prune_expired_admin_sessions(now).await;
    append_audit_log(
        &store,
        "admin_auth",
        "login",
        "admin.auth.login",
        "admin_session",
        &session_id,
        json!({"expires_unix": row.expires_unix}),
        Some(ip),
        "success",
    )
    .await;

    let cookie = format!(
        "{name}={value}; HttpOnly; Path=/; SameSite=Strict",
        name = ADMIN_SESSION_COOKIE_NAME,
        value = session_id
    );
    let mut headers_out = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(&cookie) {
        headers_out.insert(header::SET_COOKIE, v);
    }
    (
        StatusCode::OK,
        headers_out,
        Json(json!({"ok":true,"sessionId":session_id,"expiresUnix":row.expires_unix})),
    )
        .into_response()
}

async fn admin_auth_logout(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
) -> axum::response::Response {
    let Ok(auth) = require_admin(&headers, &store).await else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"unauthorized"})),
        )
            .into_response();
    };
    if let Some(session_id) = auth.session_id.as_deref() {
        let _ = store.revoke_admin_session(session_id, now_unix()).await;
    }
    append_audit_log(
        &store,
        &auth.actor_type,
        &auth.actor_id,
        "admin.auth.logout",
        "admin_session",
        auth.session_id.as_deref().unwrap_or(""),
        json!({}),
        auth.source_ip.clone(),
        "success",
    )
    .await;
    let mut headers_out = HeaderMap::new();
    headers_out.insert(
        header::SET_COOKIE,
        HeaderValue::from_static(
            "llmhotspot_admin_session=; HttpOnly; Path=/; SameSite=Strict; Max-Age=0",
        ),
    );
    (StatusCode::OK, headers_out, Json(json!({"ok":true}))).into_response()
}

async fn admin_metrics(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(resp) = require_admin(&headers, &store).await {
        return resp;
    }
    let licenses = store.count_licenses().await.unwrap_or(0);
    let active_bindings = store.count_active_device_bindings().await.unwrap_or(0);
    let now = now_unix();
    let subs = store.list_subscriptions(5000, 0).await.unwrap_or_default();
    let mut customers = std::collections::BTreeSet::<String>::new();
    let total_seats: i64 = subs.iter().map(|s| s.seat_limit).sum();
    let contracted_mrr_cents: i64 = subs
        .iter()
        .filter(|s| matches!(s.status.as_str(), "active" | "grace"))
        .map(|s| contracted_mrr_cents_for_seats(s.seat_limit))
        .sum();
    for s in &subs {
        customers.insert(s.customer_id.clone());
    }
    let avg_seats_per_customer = if customers.is_empty() {
        0.0
    } else {
        total_seats as f64 / customers.len() as f64
    };
    let rev_24h = store
        .list_billing_revenue_events_between(now.saturating_sub(24 * 60 * 60), now)
        .await
        .unwrap_or_default();
    let collected_cash_24h_cents: i64 = rev_24h
        .iter()
        .filter(|e| e.kind == "invoice_paid")
        .map(|e| e.amount_cents)
        .sum();
    let reminders_due = store.count_due_billing_reminders(now).await.unwrap_or(0);
    let reminders_sent_24h = store
        .count_sent_billing_reminders_since(now.saturating_sub(24 * 60 * 60))
        .await
        .unwrap_or(0);
    let reminders_failed_24h = store
        .count_failed_billing_reminders_since(now.saturating_sub(24 * 60 * 60))
        .await
        .unwrap_or(0);
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "metrics": {
                "licenses_total": licenses,
                "active_bindings_total": active_bindings,
                "total_customers": customers.len(),
                "total_seats": total_seats,
                "avg_seats_per_customer": avg_seats_per_customer,
                "contracted_mrr_cents": contracted_mrr_cents,
                "collected_cash_24h_cents": collected_cash_24h_cents,
                "reminders_due": reminders_due,
                "reminders_sent_24h": reminders_sent_24h,
                "reminders_failed_24h": reminders_failed_24h
            }
        })),
    )
}

#[derive(Debug, Deserialize, Default)]
struct PaginationQuery {
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    offset: Option<u32>,
}

fn pagination_params(query: &PaginationQuery) -> (u32, u32) {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0);
    (limit, offset)
}

fn csv_escape(input: &str) -> String {
    if input.contains([',', '"', '\n']) {
        format!("\"{}\"", input.replace('"', "\"\""))
    } else {
        input.to_string()
    }
}

async fn admin_list_subscriptions(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
    Query(query): Query<PaginationQuery>,
) -> impl IntoResponse {
    if let Err(resp) = require_admin(&headers, &store).await {
        return resp;
    }
    let (limit, offset) = pagination_params(&query);
    let rows = match store.list_subscriptions(limit, offset).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    };
    let mut items = Vec::new();
    for s in &rows {
        let customer = store
            .get_customer_by_id(&s.customer_id)
            .await
            .ok()
            .flatten();
        let seat_in_use = if let Some(lic) = store
            .get_license_by_subscription_id(&s.subscription_id)
            .await
            .ok()
            .flatten()
        {
            store
                .list_active_bindings_for_license(&lic.license_id)
                .await
                .map(|v| v.len() as i64)
                .unwrap_or(0)
        } else {
            0
        };
        items.push(json!({
            "subscriptionId": s.subscription_id,
            "customerId": s.customer_id,
            "email": customer.as_ref().map(|c| c.email.as_str()).unwrap_or("unknown"),
            "stripeSubscriptionId": s.stripe_subscription_id,
            "status": s.status,
            "seatLimit": s.seat_limit,
            "seatInUse": seat_in_use,
            "currentPeriodEndUnix": s.current_period_end_unix,
            "graceExpiresUnix": s.grace_expires_unix,
            "createdUnix": s.created_unix,
            "updatedUnix": s.updated_unix
        }));
    }
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "items": items,
            "pagination": { "limit": limit, "offset": offset }
        })),
    )
}

async fn admin_list_licenses(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
    Query(query): Query<PaginationQuery>,
) -> impl IntoResponse {
    if let Err(resp) = require_admin(&headers, &store).await {
        return resp;
    }
    let (limit, offset) = pagination_params(&query);
    let rows = match store.list_licenses(limit, offset).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    };
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "items": rows.iter().map(|l| json!({
                "licenseId": l.license_id,
                "customerId": l.customer_id,
                "subscriptionId": l.subscription_id,
                "licenseKey": l.license_key,
                "plan": l.plan_name,
                "status": l.status,
                "seatLimit": l.seat_limit,
                "source": l.source,
                "graceExpiresUnix": l.grace_expires_unix,
                "createdUnix": l.created_unix,
                "updatedUnix": l.updated_unix
            })).collect::<Vec<_>>(),
            "pagination": { "limit": limit, "offset": offset }
        })),
    )
}

async fn admin_list_seats(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
    Query(query): Query<PaginationQuery>,
) -> impl IntoResponse {
    if let Err(resp) = require_admin(&headers, &store).await {
        return resp;
    }
    let (limit, offset) = pagination_params(&query);
    let rows = match store.list_bindings(limit, offset).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    };
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "items": rows.iter().map(|b| json!({
                "bindingId": b.binding_id,
                "licenseId": b.license_id,
                "seatIndex": b.seat_index,
                "deviceId": b.device_id,
                "deviceName": b.device_name,
                "os": b.os,
                "countryCode": b.country_code,
                "appVersion": b.app_version,
                "label": b.label,
                "status": b.status,
                "tunnelId": b.tunnel_id,
                "tunnelHostname": b.tunnel_hostname,
                "firstSeenUnix": b.first_seen_unix,
                "lastSeenUnix": b.last_seen_unix,
                "retiredUnix": b.retired_unix
            })).collect::<Vec<_>>(),
            "pagination": { "limit": limit, "offset": offset }
        })),
    )
}

#[derive(Debug, Deserialize, Default)]
struct PeriodPaginationQuery {
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    offset: Option<u32>,
}

async fn admin_list_customers(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
    Query(query): Query<PeriodPaginationQuery>,
) -> impl IntoResponse {
    if let Err(resp) = require_admin(&headers, &store).await {
        return resp;
    }
    let (limit, offset) = pagination_params(&PaginationQuery {
        limit: query.limit,
        offset: query.offset,
    });
    let rows = match store.list_subscriptions(limit, offset).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    };
    let mut grouped: std::collections::BTreeMap<String, Value> = std::collections::BTreeMap::new();
    for sub in rows {
        let customer = store
            .get_customer_by_id(&sub.customer_id)
            .await
            .ok()
            .flatten();
        let email = customer
            .as_ref()
            .map(|c| c.email.clone())
            .unwrap_or_else(|| "unknown".to_string());
        let licenses = store
            .list_licenses_for_customer(&sub.customer_id)
            .await
            .unwrap_or_default();
        let mut active_seats: i64 = 0;
        for l in &licenses {
            active_seats += store
                .list_active_bindings_for_license(&l.license_id)
                .await
                .map(|b| b.len() as i64)
                .unwrap_or(0);
        }
        let entry = grouped.entry(email.clone()).or_insert_with(|| {
            json!({
                "customerId": sub.customer_id,
                "email": email,
                "subscriptionCount": 0,
                "seatTotal": 0,
                "activeSeatTotal": 0
            })
        });
        let next_sub_count = entry
            .get("subscriptionCount")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            + 1;
        let next_seat_total =
            entry.get("seatTotal").and_then(Value::as_i64).unwrap_or(0) + sub.seat_limit;
        let next_active = entry
            .get("activeSeatTotal")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            + active_seats;
        *entry = json!({
            "customerId": sub.customer_id,
            "email": email,
            "subscriptionCount": next_sub_count,
            "seatTotal": next_seat_total,
            "activeSeatTotal": next_active
        });
    }
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "items": grouped.into_values().collect::<Vec<_>>(),
            "pagination": { "limit": limit, "offset": offset }
        })),
    )
}

async fn admin_customer_tree(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
    Query(query): Query<PeriodPaginationQuery>,
) -> impl IntoResponse {
    if let Err(resp) = require_admin(&headers, &store).await {
        return resp;
    }
    let (limit, offset) = pagination_params(&PaginationQuery {
        limit: query.limit,
        offset: query.offset,
    });
    let customers = match store.list_subscriptions(limit, offset).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    };
    let mut out = Vec::<Value>::new();
    let mut seen = std::collections::BTreeSet::<String>::new();
    for sub in customers {
        if !seen.insert(sub.customer_id.clone()) {
            continue;
        }
        let customer = store
            .get_customer_by_id(&sub.customer_id)
            .await
            .ok()
            .flatten();
        let subs = store
            .list_subscriptions_for_customer(&sub.customer_id)
            .await
            .unwrap_or_default();
        let mut sub_nodes = Vec::<Value>::new();
        for s in subs {
            let lic_opt = store
                .get_license_by_subscription_id(&s.subscription_id)
                .await
                .unwrap_or(None);
            let mut licenses = Vec::<Value>::new();
            if let Some(lic) = lic_opt {
                let seats = store
                    .list_bindings_for_license(&lic.license_id)
                    .await
                    .unwrap_or_default();
                licenses.push(json!({
                    "licenseId": lic.license_id,
                    "licenseKey": lic.license_key,
                    "status": lic.status,
                    "seatLimit": lic.seat_limit,
                    "bindings": seats.iter().map(|b| json!({
                        "bindingId": b.binding_id,
                        "seatIndex": b.seat_index,
                        "deviceId": b.device_id,
                        "deviceName": b.device_name,
                        "os": b.os,
                        "countryCode": b.country_code,
                        "status": b.status,
                        "firstSeenUnix": b.first_seen_unix,
                        "lastSeenUnix": b.last_seen_unix
                    })).collect::<Vec<_>>()
                }));
            }
            sub_nodes.push(json!({
                "subscriptionId": s.subscription_id,
                "status": s.status,
                "seatLimit": s.seat_limit,
                "currentPeriodEndUnix": s.current_period_end_unix,
                "graceExpiresUnix": s.grace_expires_unix,
                "licenses": licenses
            }));
        }
        out.push(json!({
            "customerId": sub.customer_id,
            "email": customer.as_ref().map(|c| c.email.clone()).unwrap_or_else(|| "unknown".to_string()),
            "subscriptions": sub_nodes
        }));
    }
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "items": out,
            "pagination": {"limit": limit, "offset": offset}
        })),
    )
}

async fn admin_analytics_overview(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    if let Err(resp) = require_admin(&headers, &store).await {
        return resp;
    }
    let now = now_unix();
    let period = query
        .get("period")
        .map(|v| v.as_str())
        .filter(|v| matches!(*v, "1d" | "1w" | "1m" | "1y"))
        .unwrap_or("1w");
    let period_start = now.saturating_sub(period_seconds(period));
    let bucket_secs = period_bucket_seconds(period);

    let mut all_subs: Vec<SubscriptionRecord> = Vec::new();
    let mut offset = 0_u32;
    let page = 200_u32;
    loop {
        let chunk = store
            .list_subscriptions(page, offset)
            .await
            .unwrap_or_default();
        if chunk.is_empty() {
            break;
        }
        let len = chunk.len() as u32;
        all_subs.extend(chunk);
        if len < page {
            break;
        }
        offset = offset.saturating_add(page);
    }
    let mut customer_ids = std::collections::BTreeSet::<String>::new();
    let mut total_seats: i64 = 0;
    let mut active_seats: i64 = 0;
    let mut contracted_mrr_cents: i64 = 0;
    let mut churn_cancel_count: i64 = 0;
    let mut seats_over_time: std::collections::BTreeMap<u64, i64> =
        std::collections::BTreeMap::new();
    let mut customers_over_time: std::collections::BTreeMap<u64, i64> =
        std::collections::BTreeMap::new();
    for s in &all_subs {
        customer_ids.insert(s.customer_id.clone());
        total_seats += s.seat_limit;
        if matches!(s.status.as_str(), "active" | "grace") {
            active_seats += s.seat_limit;
            contracted_mrr_cents += contracted_mrr_cents_for_seats(s.seat_limit);
        }
        if s.status == "canceled" || s.status == "suspended" {
            churn_cancel_count += 1;
        }
        if s.created_unix >= period_start {
            let b = (s.created_unix / bucket_secs) * bucket_secs;
            *seats_over_time.entry(b).or_insert(0) += s.seat_limit;
            *customers_over_time.entry(b).or_insert(0) += 1;
        }
    }
    let avg = if customer_ids.is_empty() {
        0.0
    } else {
        total_seats as f64 / customer_ids.len() as f64
    };

    let revenue_events = store
        .list_billing_revenue_events_between(period_start, now)
        .await
        .unwrap_or_default();
    let mut collected_cash_cents_period: i64 = 0;
    let mut refunds_cents_period: i64 = 0;
    let mut revenue_over_time: std::collections::BTreeMap<u64, i64> =
        std::collections::BTreeMap::new();
    for ev in revenue_events {
        match ev.kind.as_str() {
            "invoice_paid" => collected_cash_cents_period += ev.amount_cents,
            "refund" | "chargeback" => refunds_cents_period += ev.amount_cents.abs(),
            _ => {}
        }
        let b = (ev.occurred_unix / bucket_secs) * bucket_secs;
        *revenue_over_time.entry(b).or_insert(0) += ev.amount_cents;
    }

    let mut os_distribution: std::collections::BTreeMap<String, i64> =
        std::collections::BTreeMap::new();
    let mut country_distribution: std::collections::BTreeMap<String, i64> =
        std::collections::BTreeMap::new();
    let all_bindings = store.list_bindings(5000, 0).await.unwrap_or_default();
    for b in all_bindings {
        let os = b.os.unwrap_or_else(|| "unknown".to_string());
        *os_distribution.entry(os).or_insert(0) += 1;
        let cc = b.country_code.unwrap_or_else(|| "unknown".to_string());
        *country_distribution.entry(cc).or_insert(0) += 1;
    }

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "period": period,
            "totals": {
                "total_customers": customer_ids.len(),
                "total_seats": total_seats,
                "active_seats": active_seats,
                "avg_seats_per_customer": avg,
                "contracted_mrr_cents": contracted_mrr_cents,
                "collected_cash_cents_period": collected_cash_cents_period,
                "refunds_cents_period": refunds_cents_period,
                "net_cents_period": collected_cash_cents_period - refunds_cents_period,
                "churn_cancel_count_period": churn_cancel_count
            },
            "series": {
                "seats_over_time": seats_over_time.into_iter().map(|(ts, value)| json!({"ts": ts, "value": value})).collect::<Vec<_>>(),
                "customers_over_time": customers_over_time.into_iter().map(|(ts, value)| json!({"ts": ts, "value": value})).collect::<Vec<_>>(),
                "revenue_over_time": revenue_over_time.into_iter().map(|(ts, value)| json!({"ts": ts, "value": value})).collect::<Vec<_>>(),
                "churn_cancel_over_time": vec![json!({"ts": now, "value": churn_cancel_count})],
                "os_distribution": os_distribution.into_iter().map(|(label, value)| json!({"label": label, "value": value})).collect::<Vec<_>>(),
                "country_distribution": country_distribution.into_iter().map(|(label, value)| json!({"label": label, "value": value})).collect::<Vec<_>>()
            }
        })),
    )
}

async fn admin_health(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(resp) = require_admin(&headers, &store).await {
        return resp;
    }
    let now = now_unix();
    let due = store.count_due_billing_reminders(now).await.unwrap_or(0);
    let failed = store
        .count_failed_billing_reminders_since(now.saturating_sub(24 * 60 * 60))
        .await
        .unwrap_or(0);
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "health": {
                "broker_up": true,
                "timestamp_unix": now,
                "reminders_due": due,
                "reminders_failed_24h": failed
            }
        })),
    )
}

async fn admin_export_customers_csv(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Err(resp) = require_admin(&headers, &store).await {
        return resp.into_response();
    }
    let mut csv =
        "email,customer_id,subscription_count,seat_total,active_seat_total,created_unix\n"
            .to_string();
    let subs = store.list_subscriptions(5000, 0).await.unwrap_or_default();
    let mut grouped: std::collections::BTreeMap<String, (String, i64, i64, i64, u64)> =
        std::collections::BTreeMap::new();
    for s in subs {
        let customer = store
            .get_customer_by_id(&s.customer_id)
            .await
            .ok()
            .flatten();
        let email = customer
            .as_ref()
            .map(|c| c.email.clone())
            .unwrap_or_else(|| "unknown".to_string());
        let created = customer.as_ref().map(|c| c.created_unix).unwrap_or(0);
        let entry =
            grouped
                .entry(email.clone())
                .or_insert((s.customer_id.clone(), 0, 0, 0, created));
        entry.1 += 1;
        entry.2 += s.seat_limit;
        let lics = store
            .list_licenses_for_customer(&s.customer_id)
            .await
            .unwrap_or_default();
        let mut active: i64 = 0;
        for l in &lics {
            active += store
                .list_active_bindings_for_license(&l.license_id)
                .await
                .map(|x| x.len() as i64)
                .unwrap_or(0);
        }
        entry.3 += active;
    }
    for (email, (customer_id, sub_count, seat_total, active_total, created_unix)) in grouped {
        csv.push_str(&format!(
            "{},{},{},{},{},{}\n",
            csv_escape(&email),
            csv_escape(&customer_id),
            sub_count,
            seat_total,
            active_total,
            created_unix
        ));
    }
    let mut headers_out = HeaderMap::new();
    headers_out.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    (StatusCode::OK, headers_out, csv).into_response()
}

async fn admin_export_subscriptions_csv(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Err(resp) = require_admin(&headers, &store).await {
        return resp.into_response();
    }
    let mut csv = "subscription_id,customer_id,email,status,seat_limit,seat_in_use,current_period_end_unix,grace_expires_unix,created_unix\n".to_string();
    let rows = store.list_subscriptions(5000, 0).await.unwrap_or_default();
    for s in rows {
        let customer = store
            .get_customer_by_id(&s.customer_id)
            .await
            .ok()
            .flatten();
        let lic = store
            .get_license_by_subscription_id(&s.subscription_id)
            .await
            .ok()
            .flatten();
        let seat_in_use = if let Some(l) = lic {
            store
                .list_active_bindings_for_license(&l.license_id)
                .await
                .map(|v| v.len() as i64)
                .unwrap_or(0)
        } else {
            0
        };
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{}\n",
            csv_escape(&s.subscription_id),
            csv_escape(&s.customer_id),
            csv_escape(
                customer
                    .as_ref()
                    .map(|c| c.email.as_str())
                    .unwrap_or("unknown")
            ),
            csv_escape(&s.status),
            s.seat_limit,
            seat_in_use,
            s.current_period_end_unix.unwrap_or(0),
            s.grace_expires_unix.unwrap_or(0),
            s.created_unix
        ));
    }
    let mut headers_out = HeaderMap::new();
    headers_out.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    (StatusCode::OK, headers_out, csv).into_response()
}

async fn admin_export_devices_csv(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Err(resp) = require_admin(&headers, &store).await {
        return resp.into_response();
    }
    let mut csv = "binding_id,subscription_id,license_id,email,seat_index,device_id,device_name,os,country_code,status,first_seen_unix,last_seen_unix\n".to_string();
    let rows = store.list_bindings(5000, 0).await.unwrap_or_default();
    for b in rows {
        let lic = store.get_license_by_id(&b.license_id).await.ok().flatten();
        let (sub_id, email) = if let Some(l) = lic {
            let customer = store
                .get_customer_by_id(&l.customer_id)
                .await
                .ok()
                .flatten();
            (
                l.subscription_id.unwrap_or_default(),
                customer
                    .map(|c| c.email)
                    .unwrap_or_else(|| "unknown".to_string()),
            )
        } else {
            (String::new(), "unknown".to_string())
        };
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{}\n",
            csv_escape(&b.binding_id),
            csv_escape(&sub_id),
            csv_escape(&b.license_id),
            csv_escape(&email),
            b.seat_index,
            csv_escape(&b.device_id),
            csv_escape(b.device_name.as_deref().unwrap_or("")),
            csv_escape(b.os.as_deref().unwrap_or("")),
            csv_escape(b.country_code.as_deref().unwrap_or("unknown")),
            csv_escape(&b.status),
            b.first_seen_unix,
            b.last_seen_unix
        ));
    }
    let mut headers_out = HeaderMap::new();
    headers_out.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    (StatusCode::OK, headers_out, csv).into_response()
}

async fn admin_export_revenue_csv(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    if let Err(resp) = require_admin(&headers, &store).await {
        return resp.into_response();
    }
    let now = now_unix();
    let period = query
        .get("period")
        .map(|v| v.as_str())
        .filter(|v| matches!(*v, "1d" | "1w" | "1m" | "1y"))
        .unwrap_or("1m");
    let start = now.saturating_sub(period_seconds(period));
    let events = store
        .list_billing_revenue_events_between(start, now)
        .await
        .unwrap_or_default();
    let mut buckets: std::collections::BTreeMap<u64, (i64, i64, i64, i64)> =
        std::collections::BTreeMap::new();
    let bucket_secs = period_bucket_seconds(period);
    for ev in events {
        let b = (ev.occurred_unix / bucket_secs) * bucket_secs;
        let entry = buckets.entry(b).or_insert((0, 0, 0, 0));
        match ev.kind.as_str() {
            "invoice_paid" => entry.1 += ev.amount_cents,
            "refund" | "chargeback" => entry.2 += ev.amount_cents.abs(),
            _ => {}
        }
    }
    let subs = store.list_subscriptions(5000, 0).await.unwrap_or_default();
    let contracted: i64 = subs
        .iter()
        .filter(|s| matches!(s.status.as_str(), "active" | "grace"))
        .map(|s| contracted_mrr_cents_for_seats(s.seat_limit))
        .sum();
    let mut csv = "date_bucket,contracted_mrr_cents,collected_cash_cents,refunds_cents,net_cents\n"
        .to_string();
    for (bucket, (_mrr, collected, refunds, _net)) in buckets {
        let net = collected - refunds;
        csv.push_str(&format!(
            "{},{},{},{},{}\n",
            bucket, contracted, collected, refunds, net
        ));
    }
    let mut headers_out = HeaderMap::new();
    headers_out.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    (StatusCode::OK, headers_out, csv).into_response()
}

async fn admin_list_audit_logs(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
    Query(query): Query<PaginationQuery>,
) -> impl IntoResponse {
    if let Err(resp) = require_admin(&headers, &store).await {
        return resp;
    }
    let (limit, offset) = pagination_params(&query);
    let rows = match store.list_audit_logs(limit, offset).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    };
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "items": rows.iter().map(|a| json!({
                "auditId": a.audit_id,
                "actorType": a.actor_type,
                "actorId": a.actor_id,
                "action": a.action,
                "targetType": a.target_type,
                "targetId": a.target_id,
                "payloadJson": a.payload_json,
                "createdUnix": a.created_unix,
                "ip": a.ip,
                "outcome": a.outcome
            })).collect::<Vec<_>>(),
            "pagination": { "limit": limit, "offset": offset }
        })),
    )
}

#[derive(Debug, Deserialize)]
struct AdminGiftLicensePayload {
    email: String,
    seats: i64,
    duration_days: i64,
    #[serde(default)]
    note: Option<String>,
}

async fn admin_gift_license(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
    Json(payload): Json<AdminGiftLicensePayload>,
) -> impl IntoResponse {
    let auth = match require_admin(&headers, &store).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let email = payload.email.trim().to_ascii_lowercase();
    if email.is_empty() || !email.contains('@') {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"invalid_email"})),
        );
    }
    if payload.seats <= 0 || payload.seats > 100 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"invalid_seats"})),
        );
    }
    if payload.duration_days <= 0 || payload.duration_days > 3650 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"invalid_duration_days"})),
        );
    }
    let now = now_unix();
    let customer = match store.get_customer_by_email(&email).await {
        Ok(Some(mut existing)) => {
            existing.updated_unix = now;
            existing
        }
        Ok(None) => llm_storage::CustomerRecord {
            customer_id: format!("cust_{}", uuid::Uuid::new_v4().simple()),
            email: email.clone(),
            stripe_customer_id: None,
            created_unix: now,
            updated_unix: now,
        },
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    };
    if let Err(err) = store.upsert_customer(&customer).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":format!("store error: {err}")})),
        );
    }
    let license = LicenseRecord {
        license_id: format!("lic_{}", uuid::Uuid::new_v4().simple()),
        customer_id: customer.customer_id.clone(),
        subscription_id: None,
        license_key: gen_license_key(),
        plan_name: ensure_plan_from_seat_limit(payload.seats),
        status: "active".to_string(),
        seat_limit: payload.seats,
        source: "gift".to_string(),
        grace_expires_unix: Some(
            now.saturating_add((payload.duration_days as u64).saturating_mul(24 * 60 * 60)),
        ),
        created_unix: now,
        updated_unix: now,
    };
    if let Err(err) = store.upsert_license(&license).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":format!("store error: {err}")})),
        );
    }
    let gift_id = format!("gift_{}", uuid::Uuid::new_v4().simple());
    let gift = GiftEntitlementRecord {
        gift_id: gift_id.clone(),
        license_id: license.license_id.clone(),
        recipient_email: email.clone(),
        seat_count: payload.seats,
        starts_unix: now,
        expires_unix: license.grace_expires_unix.unwrap_or(now),
        created_unix: now,
        created_by_admin_session: auth.session_id.clone(),
        status: "active".to_string(),
    };
    if let Err(err) = store.insert_gift_entitlement(&gift).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":format!("store error: {err}")})),
        );
    }
    append_audit_log(
        &store,
        &auth.actor_type,
        &auth.actor_id,
        "admin.seats.gift",
        "gift_entitlement",
        &gift_id,
        json!({
            "email": email,
            "license_id": license.license_id,
            "seats": payload.seats,
            "duration_days": payload.duration_days
        }),
        auth.source_ip.clone(),
        "success",
    )
    .await;
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "email": email,
            "giftId": gift_id,
            "licenseKey": license.license_key,
            "seats": license.seat_limit,
            "expiresUnix": license.grace_expires_unix,
            "note": payload.note.unwrap_or_default()
        })),
    )
}

#[derive(Debug, Deserialize)]
struct AdminSubscriptionUpdatePayload {
    #[serde(default)]
    seat_limit: Option<i64>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    current_period_end_unix: Option<u64>,
    #[serde(default)]
    grace_expires_unix: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
struct AdminSubscriptionCancelPayload {
    #[serde(default)]
    effective_unix: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct AdminSeatReassignPayload {
    device_id: String,
    #[serde(default)]
    device_name: Option<String>,
    #[serde(default)]
    os: Option<String>,
    #[serde(default)]
    app_version: Option<String>,
    #[serde(default)]
    label: Option<String>,
}

fn valid_subscription_status(status: &str) -> bool {
    matches!(
        status,
        "active" | "grace" | "canceled" | "past_due" | "trialing" | "incomplete" | "suspended"
    )
}

async fn sync_license_from_subscription(
    store: &Arc<dyn BrokerStore>,
    subscription: &SubscriptionRecord,
    now: u64,
) -> Result<(), String> {
    let Some(mut license) = store
        .get_license_by_subscription_id(&subscription.subscription_id)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(());
    };
    license.seat_limit = subscription.seat_limit;
    license.status = subscription.status.clone();
    license.grace_expires_unix = subscription.grace_expires_unix;
    license.updated_unix = now;
    store
        .upsert_license(&license)
        .await
        .map_err(|e| e.to_string())
}

async fn admin_subscription_update(
    Path(id): Path<String>,
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
    Json(payload): Json<AdminSubscriptionUpdatePayload>,
) -> impl IntoResponse {
    let auth = match require_admin(&headers, &store).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let Some(mut sub) = (match store.get_subscription_by_id(&id).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    }) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"subscription_not_found"})),
        );
    };
    if let Some(v) = payload.seat_limit {
        if v <= 0 || v > 10_000 {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"ok":false,"error":"invalid_seat_limit"})),
            );
        }
        sub.seat_limit = v;
    }
    if let Some(v) = payload.status.as_deref() {
        let status = v.trim().to_ascii_lowercase();
        if !valid_subscription_status(&status) {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"ok":false,"error":"invalid_status"})),
            );
        }
        sub.status = status;
    }
    if payload.current_period_end_unix.is_some() {
        sub.current_period_end_unix = payload.current_period_end_unix;
    }
    if payload.grace_expires_unix.is_some() {
        sub.grace_expires_unix = payload.grace_expires_unix;
    }
    sub.updated_unix = now_unix();
    if let Err(err) = store.upsert_subscription(&sub).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":format!("store error: {err}")})),
        );
    }
    let _ = sync_license_from_subscription(&store, &sub, sub.updated_unix).await;
    append_audit_log(
        &store,
        &auth.actor_type,
        &auth.actor_id,
        "admin.subscriptions.update",
        "subscription",
        &id,
        json!({
            "seat_limit": sub.seat_limit,
            "status": sub.status,
            "current_period_end_unix": sub.current_period_end_unix,
            "grace_expires_unix": sub.grace_expires_unix
        }),
        auth.source_ip.clone(),
        "success",
    )
    .await;
    (
        StatusCode::OK,
        Json(json!({"ok":true,"subscription":{
            "subscriptionId": sub.subscription_id,
            "status": sub.status,
            "seatLimit": sub.seat_limit,
            "currentPeriodEndUnix": sub.current_period_end_unix,
            "graceExpiresUnix": sub.grace_expires_unix
        }})),
    )
}

async fn admin_subscription_cancel(
    Path(id): Path<String>,
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
    Json(payload): Json<AdminSubscriptionCancelPayload>,
) -> impl IntoResponse {
    let auth = match require_admin(&headers, &store).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let Some(mut sub) = (match store.get_subscription_by_id(&id).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    }) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"subscription_not_found"})),
        );
    };
    let now = now_unix();
    sub.status = "canceled".to_string();
    sub.grace_expires_unix = Some(payload.effective_unix.unwrap_or(now));
    sub.updated_unix = now;
    if let Err(err) = store.upsert_subscription(&sub).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":format!("store error: {err}")})),
        );
    }
    let _ = sync_license_from_subscription(&store, &sub, now).await;
    append_audit_log(
        &store,
        &auth.actor_type,
        &auth.actor_id,
        "admin.subscriptions.cancel",
        "subscription",
        &id,
        json!({"effective_unix": sub.grace_expires_unix}),
        auth.source_ip.clone(),
        "success",
    )
    .await;
    (
        StatusCode::OK,
        Json(
            json!({"ok":true,"subscriptionId": id, "status":"canceled","effectiveUnix":sub.grace_expires_unix}),
        ),
    )
}

async fn admin_subscription_suspend(
    Path(id): Path<String>,
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let auth = match require_admin(&headers, &store).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let Some(mut sub) = (match store.get_subscription_by_id(&id).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    }) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"subscription_not_found"})),
        );
    };
    let now = now_unix();
    sub.status = "suspended".to_string();
    sub.updated_unix = now;
    if let Err(err) = store.upsert_subscription(&sub).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":format!("store error: {err}")})),
        );
    }
    let _ = sync_license_from_subscription(&store, &sub, now).await;
    append_audit_log(
        &store,
        &auth.actor_type,
        &auth.actor_id,
        "admin.subscriptions.suspend",
        "subscription",
        &id,
        json!({}),
        auth.source_ip.clone(),
        "success",
    )
    .await;
    (
        StatusCode::OK,
        Json(json!({"ok":true,"subscriptionId":id,"status":"suspended"})),
    )
}

async fn admin_subscription_unsuspend(
    Path(id): Path<String>,
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let auth = match require_admin(&headers, &store).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let Some(mut sub) = (match store.get_subscription_by_id(&id).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    }) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"subscription_not_found"})),
        );
    };
    let now = now_unix();
    sub.status = "active".to_string();
    sub.updated_unix = now;
    if let Err(err) = store.upsert_subscription(&sub).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":format!("store error: {err}")})),
        );
    }
    let _ = sync_license_from_subscription(&store, &sub, now).await;
    append_audit_log(
        &store,
        &auth.actor_type,
        &auth.actor_id,
        "admin.subscriptions.unsuspend",
        "subscription",
        &id,
        json!({}),
        auth.source_ip.clone(),
        "success",
    )
    .await;
    (
        StatusCode::OK,
        Json(json!({"ok":true,"subscriptionId":id,"status":"active"})),
    )
}

async fn admin_license_disable(
    Path(id): Path<String>,
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let auth = match require_admin(&headers, &store).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let Some(mut lic) = (match store.get_license_by_id(&id).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    }) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"license_not_found"})),
        );
    };
    lic.status = "disabled".to_string();
    lic.updated_unix = now_unix();
    if let Err(err) = store.upsert_license(&lic).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":format!("store error: {err}")})),
        );
    }
    append_audit_log(
        &store,
        &auth.actor_type,
        &auth.actor_id,
        "admin.licenses.disable",
        "license",
        &id,
        json!({}),
        auth.source_ip.clone(),
        "success",
    )
    .await;
    (
        StatusCode::OK,
        Json(json!({"ok":true,"licenseId":id,"status":"disabled"})),
    )
}

async fn admin_license_enable(
    Path(id): Path<String>,
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let auth = match require_admin(&headers, &store).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let Some(mut lic) = (match store.get_license_by_id(&id).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    }) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"license_not_found"})),
        );
    };
    lic.status = "active".to_string();
    lic.updated_unix = now_unix();
    if let Err(err) = store.upsert_license(&lic).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":format!("store error: {err}")})),
        );
    }
    append_audit_log(
        &store,
        &auth.actor_type,
        &auth.actor_id,
        "admin.licenses.enable",
        "license",
        &id,
        json!({}),
        auth.source_ip.clone(),
        "success",
    )
    .await;
    (
        StatusCode::OK,
        Json(json!({"ok":true,"licenseId":id,"status":"active"})),
    )
}

async fn admin_license_rotate(
    Path(id): Path<String>,
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let auth = match require_admin(&headers, &store).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let Some(mut lic) = (match store.get_license_by_id(&id).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    }) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"license_not_found"})),
        );
    };
    let old_key = lic.license_key.clone();
    lic.license_key = gen_license_key();
    lic.updated_unix = now_unix();
    if let Err(err) = store.upsert_license(&lic).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":format!("store error: {err}")})),
        );
    }
    append_audit_log(
        &store,
        &auth.actor_type,
        &auth.actor_id,
        "admin.licenses.rotate",
        "license",
        &id,
        json!({"old_key_hash": hash_text(&old_key)}),
        auth.source_ip.clone(),
        "success",
    )
    .await;
    (
        StatusCode::OK,
        Json(json!({"ok":true,"licenseId":id,"licenseKey":lic.license_key})),
    )
}

async fn admin_seat_unassign(
    Path(seat_id): Path<String>,
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let auth = match require_admin(&headers, &store).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let binding_device_id = match store.get_binding_by_id(&seat_id).await {
        Ok(v) => v.map(|binding| binding.device_id),
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    };
    let now = now_unix();
    match store.retire_binding(&seat_id, now).await {
        Ok(true) => {
            if let Some(device_id) = binding_device_id {
                let _ = store.clear_tunnel_lease_provisioning(&device_id, now).await;
            }
            append_audit_log(
                &store,
                &auth.actor_type,
                &auth.actor_id,
                "admin.seats.unassign",
                "seat",
                &seat_id,
                json!({}),
                auth.source_ip.clone(),
                "success",
            )
            .await;
            (
                StatusCode::OK,
                Json(json!({"ok":true,"seatId":seat_id,"status":"retired"})),
            )
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"seat_not_found_or_not_active"})),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":format!("store error: {err}")})),
        ),
    }
}

async fn admin_seat_reassign(
    Path(seat_id): Path<String>,
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
    Json(payload): Json<AdminSeatReassignPayload>,
) -> impl IntoResponse {
    let auth = match require_admin(&headers, &store).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let device_id = payload.device_id.trim();
    if device_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"device_id_required"})),
        );
    }
    let Some(mut binding) = (match store.get_binding_by_id(&seat_id).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    }) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"seat_not_found"})),
        );
    };
    let Some(license) = (match store.get_license_by_id(&binding.license_id).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    }) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"license_not_found"})),
        );
    };
    if !license_is_effective_active(&license, now_unix()) {
        return (
            StatusCode::CONFLICT,
            Json(json!({"ok":false,"error":"license_not_active"})),
        );
    }
    if let Ok(Some(other)) = store
        .get_active_binding_by_license_device(&binding.license_id, device_id)
        .await
    {
        if other.binding_id != seat_id {
            return (
                StatusCode::CONFLICT,
                Json(json!({"ok":false,"error":"device_already_assigned"})),
            );
        }
    }
    binding.device_id = device_id.to_string();
    binding.device_name = payload.device_name;
    binding.os = payload.os;
    binding.app_version = payload.app_version;
    binding.label = payload.label;
    binding.status = "active".to_string();
    binding.last_seen_unix = now_unix();
    binding.retired_unix = None;
    if let Err(err) = store.upsert_device_binding(&binding).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":format!("store error: {err}")})),
        );
    }
    append_audit_log(
        &store,
        &auth.actor_type,
        &auth.actor_id,
        "admin.seats.reassign",
        "seat",
        &seat_id,
        json!({"device_id": binding.device_id}),
        auth.source_ip.clone(),
        "success",
    )
    .await;
    (
        StatusCode::OK,
        Json(json!({"ok":true,"seatId":seat_id,"deviceId":binding.device_id,"status":"active"})),
    )
}

#[derive(Debug, Deserialize)]
struct AdminReconcilePayload {
    #[serde(default)]
    stripe_subscription_id: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}

async fn fetch_stripe_subscription(
    secret: &str,
    stripe_subscription_id: &str,
) -> Result<Value, String> {
    let url = format!(
        "{}/v1/subscriptions/{}",
        stripe_api_base(),
        stripe_subscription_id
    );
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .bearer_auth(secret)
        .send()
        .await
        .map_err(|e| format!("stripe_request_failed: {e}"))?;
    let status = resp.status();
    let value: Value = resp
        .json()
        .await
        .map_err(|e| format!("stripe_decode_failed: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "stripe_subscription_fetch_failed status={} body={value}",
            status.as_u16()
        ));
    }
    Ok(value)
}

async fn list_stripe_subscriptions(secret: &str, limit: u32) -> Result<Vec<Value>, String> {
    let url = format!(
        "{}/v1/subscriptions?limit={}&status=all",
        stripe_api_base(),
        limit
    );
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .bearer_auth(secret)
        .send()
        .await
        .map_err(|e| format!("stripe_request_failed: {e}"))?;
    let status = resp.status();
    let value: Value = resp
        .json()
        .await
        .map_err(|e| format!("stripe_decode_failed: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "stripe_subscription_list_failed status={} body={value}",
            status.as_u16()
        ));
    }
    let arr = value
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(arr)
}

async fn admin_reconcile_billing(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
    Json(payload): Json<AdminReconcilePayload>,
) -> impl IntoResponse {
    let auth = match require_admin(&headers, &store).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let Some(secret) = stripe_secret_key() else {
        return (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({"ok":false,"error":"stripe_not_configured"})),
        );
    };

    let mut processed: u64 = 0;
    let mut failed: u64 = 0;
    let mut failed_ids: Vec<String> = Vec::new();

    if let Some(stripe_sub_id) = payload
        .stripe_subscription_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        match fetch_stripe_subscription(&secret, stripe_sub_id).await {
            Ok(obj) => {
                match process_stripe_event(&store, "customer.subscription.updated", &obj).await {
                    Ok(()) => processed = 1,
                    Err(_) => {
                        failed = 1;
                        failed_ids.push(stripe_sub_id.to_string());
                    }
                }
            }
            Err(err) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({"ok":false,"error":err})),
                );
            }
        }
    } else {
        let limit = payload.limit.unwrap_or(20).clamp(1, 100);
        let subs = match list_stripe_subscriptions(&secret, limit).await {
            Ok(v) => v,
            Err(err) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({"ok":false,"error":err})),
                );
            }
        };
        for obj in subs {
            let sub_id = obj
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            match process_stripe_event(&store, "customer.subscription.updated", &obj).await {
                Ok(()) => processed = processed.saturating_add(1),
                Err(_) => {
                    failed = failed.saturating_add(1);
                    if !sub_id.is_empty() {
                        failed_ids.push(sub_id);
                    }
                }
            }
        }
    }

    append_audit_log(
        &store,
        &auth.actor_type,
        &auth.actor_id,
        "admin.maintenance.reconcile",
        "billing",
        "",
        json!({"processed": processed, "failed": failed, "failedIds": failed_ids}),
        auth.source_ip.clone(),
        "success",
    )
    .await;
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "processed": processed,
            "failed": failed,
            "failedIds": failed_ids,
        })),
    )
}

#[derive(Debug, Deserialize)]
struct AdminReplayFailedPayload {
    #[serde(default)]
    limit: Option<u32>,
}

async fn process_audited_billing_payload(
    store: &Arc<dyn BrokerStore>,
    payload_json: &str,
) -> Result<(), String> {
    let value: Value =
        serde_json::from_str(payload_json).map_err(|e| format!("invalid_payload_json: {e}"))?;
    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing event type".to_string())?;
    let obj = value
        .get("data")
        .and_then(|v| v.get("object"))
        .cloned()
        .ok_or_else(|| "missing data.object".to_string())?;
    process_stripe_event(store, event_type, &obj).await
}

async fn admin_replay_failed_billing_events(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
    Json(payload): Json<AdminReplayFailedPayload>,
) -> impl IntoResponse {
    let auth = match require_admin(&headers, &store).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let limit = payload.limit.unwrap_or(50).clamp(1, 200);
    let failed_rows = match store.list_billing_events_by_status("failed", limit).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    };

    let mut processed: u64 = 0;
    let mut failed: u64 = 0;
    let mut failed_ids: Vec<String> = Vec::new();
    for row in failed_rows {
        let now = now_unix();
        match process_audited_billing_payload(&store, &row.payload_json).await {
            Ok(()) => {
                let _ = store
                    .set_billing_event_result(&row.event_id, now, "processed", None)
                    .await;
                processed = processed.saturating_add(1);
            }
            Err(err) => {
                let _ = store
                    .set_billing_event_result(&row.event_id, now, "failed", Some(&err))
                    .await;
                failed = failed.saturating_add(1);
                failed_ids.push(row.event_id);
            }
        }
    }
    append_audit_log(
        &store,
        &auth.actor_type,
        &auth.actor_id,
        "admin.maintenance.replay_failed_events",
        "billing_events",
        "",
        json!({"processed": processed, "failed": failed, "failedIds": failed_ids}),
        auth.source_ip.clone(),
        "success",
    )
    .await;
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "processed": processed,
            "failed": failed,
            "failedIds": failed_ids
        })),
    )
}

#[derive(Debug, Deserialize)]
struct CreateCheckoutPayload {
    email: String,
    seats: i64,
    #[serde(default)]
    success_url: Option<String>,
    #[serde(default)]
    cancel_url: Option<String>,
}

async fn create_checkout(Json(payload): Json<CreateCheckoutPayload>) -> impl IntoResponse {
    let Some(secret) = stripe_secret_key() else {
        return (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({"ok":false,"error":"stripe_not_configured"})),
        );
    };
    if let Err(code) = validate_stripe_secret_for_mode(&secret) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "error": code,
                "hint": "Set STRIPE_SECRET_KEY to a test key (sk_test_...) or disable LLMHOTSPOT_STRIPE_SANDBOX_MODE."
            })),
        );
    }
    let Some(price_id) = stripe_price_id() else {
        return (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({"ok":false,"error":"stripe_price_not_configured"})),
        );
    };
    let email = payload.email.trim().to_ascii_lowercase();
    if email.is_empty() || !email.contains('@') {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"invalid_email"})),
        );
    }
    if payload.seats <= 0 || payload.seats > 100 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"invalid_seats"})),
        );
    }
    let success_url = payload
        .success_url
        .or_else(stripe_default_success_url)
        .unwrap_or_default();
    let cancel_url = payload
        .cancel_url
        .or_else(stripe_default_cancel_url)
        .unwrap_or_default();
    if success_url.is_empty() || cancel_url.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"missing_success_or_cancel_url"})),
        );
    }

    let form = vec![
        ("mode".to_string(), "subscription".to_string()),
        ("customer_email".to_string(), email),
        ("line_items[0][price]".to_string(), price_id),
        (
            "line_items[0][quantity]".to_string(),
            payload.seats.to_string(),
        ),
        ("success_url".to_string(), success_url),
        ("cancel_url".to_string(), cancel_url),
        (
            "metadata[requested_seats]".to_string(),
            payload.seats.to_string(),
        ),
    ];
    let url = format!("{}/v1/checkout/sessions", stripe_api_base());
    let client = reqwest::Client::new();
    let resp = match client
        .post(url)
        .bearer_auth(secret)
        .form(&form)
        .send()
        .await
    {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"ok":false,"error":format!("stripe_request_failed: {err}")})),
            );
        }
    };
    let status = resp.status();
    let value: Value = match resp.json().await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"ok":false,"error":format!("stripe_decode_failed: {err}")})),
            );
        }
    };
    if !status.is_success() {
        return (
            StatusCode::BAD_GATEWAY,
            Json(
                json!({"ok":false,"error":"stripe_checkout_failed","status":status.as_u16(),"stripe":value}),
            ),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "checkoutSessionId": value.get("id").and_then(Value::as_str).unwrap_or(""),
            "checkoutUrl": value.get("url").and_then(Value::as_str).unwrap_or("")
        })),
    )
}

#[derive(Debug, Deserialize)]
struct CreatePortalSessionPayload {
    customer_id: String,
    #[serde(default)]
    return_url: Option<String>,
}

async fn create_portal_session(
    Json(payload): Json<CreatePortalSessionPayload>,
) -> impl IntoResponse {
    let Some(secret) = stripe_secret_key() else {
        return (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({"ok":false,"error":"stripe_not_configured"})),
        );
    };
    if let Err(code) = validate_stripe_secret_for_mode(&secret) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "error": code,
                "hint": "Set STRIPE_SECRET_KEY to a test key (sk_test_...) or disable LLMHOTSPOT_STRIPE_SANDBOX_MODE."
            })),
        );
    }
    let customer_id = payload.customer_id.trim();
    if customer_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"missing_customer_id"})),
        );
    }
    let return_url = payload
        .return_url
        .or_else(stripe_default_cancel_url)
        .unwrap_or_default();
    if return_url.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"missing_return_url"})),
        );
    }

    let form = vec![
        ("customer".to_string(), customer_id.to_string()),
        ("return_url".to_string(), return_url),
    ];
    let url = format!("{}/v1/billing_portal/sessions", stripe_api_base());
    let client = reqwest::Client::new();
    let resp = match client
        .post(url)
        .bearer_auth(secret)
        .form(&form)
        .send()
        .await
    {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"ok":false,"error":format!("stripe_request_failed: {err}")})),
            );
        }
    };
    let status = resp.status();
    let value: Value = match resp.json().await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"ok":false,"error":format!("stripe_decode_failed: {err}")})),
            );
        }
    };
    if !status.is_success() {
        return (
            StatusCode::BAD_GATEWAY,
            Json(
                json!({"ok":false,"error":"stripe_portal_failed","status":status.as_u16(),"stripe":value}),
            ),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "portalUrl": value.get("url").and_then(Value::as_str).unwrap_or("")
        })),
    )
}

async fn ensure_customer_for_billing(
    store: &Arc<dyn BrokerStore>,
    email: &str,
    stripe_customer_id: Option<&str>,
    now: u64,
) -> Result<CustomerRecord, String> {
    if let Some(scid) = stripe_customer_id {
        if let Some(mut existing) = store
            .get_customer_by_stripe_customer_id(scid)
            .await
            .map_err(|e| e.to_string())?
        {
            existing.updated_unix = now;
            if existing.email != email {
                existing.email = email.to_string();
            }
            store
                .upsert_customer(&existing)
                .await
                .map_err(|e| e.to_string())?;
            return Ok(existing);
        }
    }
    if let Some(mut existing) = store
        .get_customer_by_email(email)
        .await
        .map_err(|e| e.to_string())?
    {
        existing.updated_unix = now;
        if existing.stripe_customer_id.is_none() {
            existing.stripe_customer_id = stripe_customer_id.map(ToString::to_string);
        }
        store
            .upsert_customer(&existing)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(existing);
    }
    let customer = CustomerRecord {
        customer_id: format!("cust_{}", uuid::Uuid::new_v4().simple()),
        email: email.to_string(),
        stripe_customer_id: stripe_customer_id.map(ToString::to_string),
        created_unix: now,
        updated_unix: now,
    };
    store
        .upsert_customer(&customer)
        .await
        .map_err(|e| e.to_string())?;
    Ok(customer)
}

async fn upsert_license_for_subscription(
    store: &Arc<dyn BrokerStore>,
    subscription: &SubscriptionRecord,
    source: &str,
    now: u64,
) -> Result<(), String> {
    if let Some(mut existing) = store
        .get_license_by_subscription_id(&subscription.subscription_id)
        .await
        .map_err(|e| e.to_string())?
    {
        existing.customer_id = subscription.customer_id.clone();
        existing.status = subscription.status.clone();
        existing.seat_limit = subscription.seat_limit;
        existing.grace_expires_unix = subscription.grace_expires_unix;
        existing.updated_unix = now;
        store
            .upsert_license(&existing)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    let row = LicenseRecord {
        license_id: format!("lic_{}", uuid::Uuid::new_v4().simple()),
        customer_id: subscription.customer_id.clone(),
        subscription_id: Some(subscription.subscription_id.clone()),
        license_key: gen_license_key(),
        plan_name: ensure_plan_from_seat_limit(subscription.seat_limit),
        status: subscription.status.clone(),
        seat_limit: subscription.seat_limit,
        source: source.to_string(),
        grace_expires_unix: subscription.grace_expires_unix,
        created_unix: now,
        updated_unix: now,
    };
    store.upsert_license(&row).await.map_err(|e| e.to_string())
}

fn normalize_subscription_status_for_entitlement(status: &str, now: u64) -> (String, Option<u64>) {
    let lower = status.to_ascii_lowercase();
    match lower.as_str() {
        "active" | "trialing" => ("active".to_string(), None),
        "past_due" | "unpaid" | "incomplete" => (
            "grace".to_string(),
            Some(now.saturating_add(BILLING_GRACE_SECS)),
        ),
        "canceled" | "incomplete_expired" => ("canceled".to_string(), Some(now)),
        _ => (
            "grace".to_string(),
            Some(now.saturating_add(BILLING_GRACE_SECS)),
        ),
    }
}

fn parse_checkout_completed(obj: &Value) -> Result<(String, Option<String>, String, i64), String> {
    let object_kind = obj.get("object").and_then(Value::as_str).unwrap_or("");
    if object_kind != "checkout.session" {
        return Err("invalid object kind for checkout.session.completed".to_string());
    }
    let email = obj
        .get("customer_details")
        .and_then(|v| v.get("email"))
        .and_then(Value::as_str)
        .or_else(|| obj.get("customer_email").and_then(Value::as_str))
        .map(|v| v.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if email.is_empty() || !email.contains('@') {
        return Err("missing customer email on checkout.session.completed".to_string());
    }
    let stripe_customer_id = obj
        .get("customer")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let stripe_subscription_id = obj
        .get("subscription")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| "missing subscription id on checkout.session.completed".to_string())?;
    let seats = obj
        .get("metadata")
        .and_then(|m| m.get("requested_seats"))
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(1);
    Ok((email, stripe_customer_id, stripe_subscription_id, seats))
}

fn parse_subscription_object(
    event_type: &str,
    obj: &Value,
) -> Result<(String, i64, String, Option<u64>), String> {
    let object_kind = obj.get("object").and_then(Value::as_str).unwrap_or("");
    if object_kind != "subscription" {
        return Err("invalid object kind for subscription event".to_string());
    }
    let stripe_sub_id = obj
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing subscription id".to_string())?
        .to_string();
    let quantity_direct = obj.get("quantity").and_then(Value::as_i64);
    let quantity_item = obj
        .get("items")
        .and_then(|v| v.get("data"))
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("quantity"))
        .and_then(Value::as_i64);
    let quantity = quantity_direct
        .or(quantity_item)
        .filter(|v| *v > 0)
        .unwrap_or(1);
    let raw_status = if event_type == "customer.subscription.deleted" {
        "canceled".to_string()
    } else {
        obj.get("status")
            .and_then(Value::as_str)
            .unwrap_or("active")
            .to_string()
    };
    let current_period_end = obj
        .get("current_period_end")
        .and_then(Value::as_i64)
        .map(|v| v.max(0) as u64);
    Ok((stripe_sub_id, quantity, raw_status, current_period_end))
}

fn parse_invoice_subscription_id(obj: &Value) -> Result<String, String> {
    let object_kind = obj.get("object").and_then(Value::as_str).unwrap_or("");
    if object_kind != "invoice" {
        return Err("invalid object kind for invoice event".to_string());
    }
    let stripe_sub_id = obj
        .get("subscription")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing subscription id on invoice event".to_string())?;
    Ok(stripe_sub_id.to_string())
}

async fn schedule_billing_reminder(
    store: &Arc<dyn BrokerStore>,
    subscription: &SubscriptionRecord,
    license_id: Option<&str>,
    customer_email: &str,
    reason: &str,
    due_unix: u64,
    now: u64,
) -> Result<bool, String> {
    let idempotency_key = format!("{}:{reason}:{due_unix}", subscription.subscription_id);
    let row = BillingReminderRecord {
        reminder_id: format!("brm_{}", uuid::Uuid::new_v4().simple()),
        subscription_id: subscription.subscription_id.clone(),
        license_id: license_id.map(|s| s.to_string()),
        customer_email: customer_email.to_string(),
        reason: reason.to_string(),
        due_unix,
        sent_unix: None,
        revoked_unix: None,
        attempt_count: 0,
        last_error: None,
        created_unix: now,
        updated_unix: now,
        idempotency_key,
    };
    store
        .insert_billing_reminder_if_absent(&row)
        .await
        .map_err(|e| e.to_string())
}

async fn schedule_payment_failed_reminders(
    store: &Arc<dyn BrokerStore>,
    subscription: &SubscriptionRecord,
    customer_email: &str,
    now: u64,
) -> Result<(), String> {
    let license_id = store
        .get_license_by_subscription_id(&subscription.subscription_id)
        .await
        .map_err(|e| e.to_string())?
        .map(|v| v.license_id);
    let _ = schedule_billing_reminder(
        store,
        subscription,
        license_id.as_deref(),
        customer_email,
        "payment_failed_immediate",
        now,
        now,
    )
    .await?;
    let Some(grace_end) = subscription.grace_expires_unix else {
        return Ok(());
    };
    let checkpoints = [
        ("payment_failed_7d", 7 * 24 * 60 * 60),
        ("payment_failed_3d", 3 * 24 * 60 * 60),
        ("payment_failed_1d", 24 * 60 * 60),
    ];
    for (reason, offset) in checkpoints {
        let due = grace_end.saturating_sub(offset);
        if due > now {
            let _ = schedule_billing_reminder(
                store,
                subscription,
                license_id.as_deref(),
                customer_email,
                reason,
                due,
                now,
            )
            .await?;
        }
    }
    Ok(())
}

async fn schedule_cancellation_reminders(
    store: &Arc<dyn BrokerStore>,
    subscription: &SubscriptionRecord,
    customer_email: &str,
    now: u64,
) -> Result<(), String> {
    let license_id = store
        .get_license_by_subscription_id(&subscription.subscription_id)
        .await
        .map_err(|e| e.to_string())?
        .map(|v| v.license_id);
    let _ = schedule_billing_reminder(
        store,
        subscription,
        license_id.as_deref(),
        customer_email,
        "cancel_immediate",
        now,
        now,
    )
    .await?;
    let expiry = subscription
        .current_period_end_unix
        .or(subscription.grace_expires_unix)
        .unwrap_or(now);
    let checkpoints = [
        ("cancel_7d", 7 * 24 * 60 * 60),
        ("cancel_3d", 3 * 24 * 60 * 60),
        ("cancel_1d", 24 * 60 * 60),
    ];
    for (reason, offset) in checkpoints {
        let due = expiry.saturating_sub(offset);
        if due > now {
            let _ = schedule_billing_reminder(
                store,
                subscription,
                license_id.as_deref(),
                customer_email,
                reason,
                due,
                now,
            )
            .await?;
        }
    }
    Ok(())
}

async fn revoke_pending_recovery_reminders(
    store: &Arc<dyn BrokerStore>,
    subscription_id: &str,
    now: u64,
) {
    let _ = store
        .revoke_pending_billing_reminders(subscription_id, &["payment_failed_", "cancel_"], now)
        .await;
}

#[derive(Debug, Serialize)]
struct ReminderDispatchRequestItem {
    reminder_id: String,
    email: String,
    subscription_id: String,
    license_id: Option<String>,
    reason: String,
    due_unix: u64,
    grace_expires_unix: Option<u64>,
    current_period_end_unix: Option<u64>,
    seats: i64,
    status: String,
}

#[derive(Debug, Deserialize)]
struct ReminderDispatchResponseItem {
    reminder_id: String,
    ok: bool,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReminderDispatchResponse {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    results: Vec<ReminderDispatchResponseItem>,
}

async fn process_stripe_event(
    store: &Arc<dyn BrokerStore>,
    event_type: &str,
    obj: &Value,
) -> Result<(), String> {
    let now = now_unix();
    match event_type {
        "checkout.session.completed" => {
            let (email, stripe_customer_id, stripe_subscription_id, seats) =
                parse_checkout_completed(obj)?;
            let customer =
                ensure_customer_for_billing(store, &email, stripe_customer_id.as_deref(), now)
                    .await?;
            let sub = SubscriptionRecord {
                subscription_id: stripe_subscription_id.clone(),
                customer_id: customer.customer_id.clone(),
                stripe_subscription_id: Some(stripe_subscription_id),
                status: "active".to_string(),
                seat_limit: seats,
                current_period_end_unix: None,
                grace_expires_unix: None,
                created_unix: now,
                updated_unix: now,
            };
            store
                .upsert_subscription(&sub)
                .await
                .map_err(|e| e.to_string())?;
            upsert_license_for_subscription(store, &sub, "stripe", now).await?;
            Ok(())
        }
        "customer.subscription.updated" | "customer.subscription.deleted" => {
            let (stripe_sub_id, quantity, raw_status, current_period_end) =
                parse_subscription_object(event_type, obj)?;
            let existing = if let Some(s) = store
                .get_subscription_by_stripe_subscription_id(&stripe_sub_id)
                .await
                .map_err(|e| e.to_string())?
            {
                Some(s)
            } else {
                store
                    .get_subscription_by_id(&stripe_sub_id)
                    .await
                    .map_err(|e| e.to_string())?
            };
            let Some(mut sub) = existing else {
                return Ok(());
            };
            let (status, mut grace) =
                normalize_subscription_status_for_entitlement(&raw_status, now);
            if status == "canceled" {
                grace = current_period_end.or(grace);
            }
            sub.status = status;
            sub.seat_limit = quantity;
            sub.current_period_end_unix = current_period_end;
            sub.grace_expires_unix = grace;
            sub.updated_unix = now;
            store
                .upsert_subscription(&sub)
                .await
                .map_err(|e| e.to_string())?;
            upsert_license_for_subscription(store, &sub, "stripe", now).await?;
            if let Some(customer) = store
                .get_customer_by_id(&sub.customer_id)
                .await
                .map_err(|e| e.to_string())?
            {
                if sub.status == "canceled" {
                    schedule_cancellation_reminders(store, &sub, &customer.email, now).await?;
                } else {
                    let _ = store
                        .revoke_pending_billing_reminders(&sub.subscription_id, &["cancel_"], now)
                        .await;
                }
            }
            Ok(())
        }
        "invoice.paid" | "invoice.payment_failed" => {
            let stripe_sub_id = parse_invoice_subscription_id(obj)?;
            let invoice_id = obj.get("id").and_then(Value::as_str).unwrap_or("");
            let Some(mut sub) = store
                .get_subscription_by_stripe_subscription_id(&stripe_sub_id)
                .await
                .map_err(|e| e.to_string())?
            else {
                return Ok(());
            };
            if event_type == "invoice.paid" {
                sub.status = "active".to_string();
                sub.grace_expires_unix = None;
                revoke_pending_recovery_reminders(store, &sub.subscription_id, now).await;
            } else {
                sub.status = "grace".to_string();
                sub.grace_expires_unix = Some(now.saturating_add(BILLING_GRACE_SECS));
            }
            sub.updated_unix = now;
            store
                .upsert_subscription(&sub)
                .await
                .map_err(|e| e.to_string())?;
            upsert_license_for_subscription(store, &sub, "stripe", now).await?;
            if event_type == "invoice.payment_failed" {
                if let Some(customer) = store
                    .get_customer_by_id(&sub.customer_id)
                    .await
                    .map_err(|e| e.to_string())?
                {
                    schedule_payment_failed_reminders(store, &sub, &customer.email, now).await?;
                }
            } else {
                let amount_cents = obj
                    .get("amount_paid")
                    .and_then(Value::as_i64)
                    .or_else(|| obj.get("amount_due").and_then(Value::as_i64))
                    .unwrap_or(0);
                let currency = obj
                    .get("currency")
                    .and_then(Value::as_str)
                    .unwrap_or("usd")
                    .to_ascii_lowercase();
                if amount_cents > 0 && !invoice_id.is_empty() {
                    let _ = store
                        .insert_billing_revenue_event_if_new(
                            &llm_storage::BillingRevenueEventRecord {
                                event_id: format!("rev_invoice_paid_{invoice_id}"),
                                stripe_subscription_id: Some(stripe_sub_id.clone()),
                                customer_id: Some(sub.customer_id.clone()),
                                amount_cents,
                                currency,
                                kind: "invoice_paid".to_string(),
                                occurred_unix: now,
                            },
                        )
                        .await;
                }
            }
            Ok(())
        }
        "charge.refunded" | "charge.dispute.created" => {
            let charge_id = obj.get("id").and_then(Value::as_str).unwrap_or("");
            if !charge_id.is_empty() {
                let amount_cents = obj
                    .get("amount_refunded")
                    .and_then(Value::as_i64)
                    .or_else(|| obj.get("amount").and_then(Value::as_i64))
                    .unwrap_or(0)
                    .abs();
                let currency = obj
                    .get("currency")
                    .and_then(Value::as_str)
                    .unwrap_or("usd")
                    .to_ascii_lowercase();
                if amount_cents > 0 {
                    let kind = if event_type == "charge.refunded" {
                        "refund"
                    } else {
                        "chargeback"
                    };
                    let _ = store
                        .insert_billing_revenue_event_if_new(
                            &llm_storage::BillingRevenueEventRecord {
                                event_id: format!("rev_{kind}_{charge_id}"),
                                stripe_subscription_id: None,
                                customer_id: None,
                                amount_cents,
                                currency,
                                kind: kind.to_string(),
                                occurred_unix: now,
                            },
                        )
                        .await;
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

async fn stripe_webhook(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let Some(secret) = stripe_webhook_secret() else {
        return (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({"ok":false,"error":"stripe_webhook_not_configured"})),
        );
    };
    let sig_header = headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !verify_stripe_webhook_signature(sig_header, &body, &secret, now_unix()) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"invalid_webhook_signature"})),
        );
    }
    let value: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"ok":false,"error":format!("invalid_json: {err}")})),
            );
        }
    };
    let event_id = value.get("id").and_then(Value::as_str).unwrap_or("").trim();
    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if event_id.is_empty() || event_type.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"missing_event_id_or_type"})),
        );
    }
    let created_unix = value
        .get("created")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        .max(0) as u64;
    let rec = BillingEventAuditRecord {
        event_id: event_id.to_string(),
        event_type: event_type.to_string(),
        created_unix,
        payload_json: body.clone(),
        received_unix: now_unix(),
        processed_unix: None,
        process_status: "received".to_string(),
        error: None,
    };
    let inserted = match store.insert_billing_event_if_new(&rec).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    };
    if !inserted {
        return (StatusCode::OK, Json(json!({"ok":true,"duplicate":true})));
    }
    let obj = value
        .get("data")
        .and_then(|v| v.get("object"))
        .cloned()
        .unwrap_or(Value::Null);
    let process_res = process_stripe_event(&store, event_type, &obj).await;
    let now = now_unix();
    match process_res {
        Ok(()) => {
            let _ = store
                .set_billing_event_result(event_id, now, "processed", None)
                .await;
            (StatusCode::OK, Json(json!({"ok":true})))
        }
        Err(err) => {
            let _ = store
                .set_billing_event_result(event_id, now, "failed", Some(&err))
                .await;
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":err})),
            )
        }
    }
}

#[derive(Debug, Deserialize)]
struct HeartbeatRequest {
    #[serde(default)]
    device_id: String,
    #[serde(default, rename = "deviceId")]
    device_id_compat: String,
}

async fn heartbeat(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    Json(req): Json<HeartbeatRequest>,
) -> impl IntoResponse {
    let device_id = if !req.device_id.trim().is_empty() {
        req.device_id.trim().to_string()
    } else {
        req.device_id_compat.trim().to_string()
    };
    if device_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"deviceId is required"})),
        );
    }
    let now = now_unix();
    match store.touch_tunnel_lease_last_seen(&device_id, now).await {
        Ok(true) => (StatusCode::OK, Json(json!({"ok":true,"lastSeenUnix":now}))),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":"lease_not_found"})),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":format!("store error: {err}")})),
        ),
    }
}

async fn reap(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    req: Request,
) -> impl IntoResponse {
    if let Err(resp) = require_cron_secret(&req) {
        return resp;
    }

    let cf = cloudflare_client();
    if !cf.is_configured() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":"cloudflare not configured"})),
        );
    }

    let now = now_unix();
    let stale_before = now.saturating_sub(WEEK_SECS);
    let stale = match store.list_stale_tunnel_leases(stale_before).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            )
        }
    };

    let mut deleted = 0_u64;
    for lease in stale {
        let Some(tunnel_id) = lease.tunnel_id.as_deref() else {
            continue;
        };
        let _ = cf.delete_tunnel(tunnel_id).await;
        let _ = store
            .clear_tunnel_lease_provisioning(&lease.device_id, now)
            .await;
        deleted += 1;
        sleep(Duration::from_millis(50)).await;
    }

    (StatusCode::OK, Json(json!({"ok":true,"deleted":deleted})))
}

fn require_cron_secret(req: &Request) -> Result<(), (StatusCode, Json<Value>)> {
    let Some(secret) = env_nonempty("CRON_SECRET") else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":"CRON_SECRET not configured"})),
        ));
    };
    let provided = req
        .headers()
        .get("x-cron-secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if provided != secret {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok":false,"error":"unauthorized"})),
        ));
    }
    Ok(())
}

async fn billing_reminders_dispatch(
    Extension(store): Extension<Arc<dyn BrokerStore>>,
    req: Request,
) -> impl IntoResponse {
    if let Err(resp) = require_cron_secret(&req) {
        return resp;
    }
    let Some(internal_secret) = reminder_internal_secret() else {
        return (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({"ok":false,"error":"reminder_internal_secret_not_configured"})),
        );
    };

    let now = now_unix();
    let due = match store
        .list_due_unsent_billing_reminders(now, REMINDER_BATCH_LIMIT)
        .await
    {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":format!("store error: {err}")})),
            );
        }
    };
    let scanned = due.len() as u64;
    if due.is_empty() {
        return (
            StatusCode::OK,
            Json(json!({"ok":true,"scanned":0,"attempted":0,"sent":0,"failed":0})),
        );
    }

    let mut reminders_payload: Vec<ReminderDispatchRequestItem> = Vec::with_capacity(due.len());
    for reminder in &due {
        let sub = store
            .get_subscription_by_id(&reminder.subscription_id)
            .await
            .ok()
            .flatten();
        reminders_payload.push(ReminderDispatchRequestItem {
            reminder_id: reminder.reminder_id.clone(),
            email: reminder.customer_email.clone(),
            subscription_id: reminder.subscription_id.clone(),
            license_id: reminder.license_id.clone(),
            reason: reminder.reason.clone(),
            due_unix: reminder.due_unix,
            grace_expires_unix: sub.as_ref().and_then(|v| v.grace_expires_unix),
            current_period_end_unix: sub.as_ref().and_then(|v| v.current_period_end_unix),
            seats: sub.as_ref().map(|v| v.seat_limit).unwrap_or(1),
            status: sub
                .as_ref()
                .map(|v| v.status.clone())
                .unwrap_or_else(|| "unknown".to_string()),
        });
    }

    let url = format!(
        "{}/api/internal/reminders/send",
        reminder_webhook_base_url().trim_end_matches('/')
    );
    let client = reqwest::Client::new();
    let response = match client
        .post(url)
        .header("x-internal-reminder-secret", &internal_secret)
        .json(&json!({ "reminders": reminders_payload }))
        .send()
        .await
    {
        Ok(v) => v,
        Err(err) => {
            let msg = format!("dispatch_request_failed: {err}");
            let mut failed = 0_u64;
            for reminder in &due {
                if store
                    .mark_billing_reminder_failed(&reminder.reminder_id, now, Some(&msg))
                    .await
                    .unwrap_or(false)
                {
                    failed = failed.saturating_add(1);
                }
            }
            return (
                StatusCode::BAD_GATEWAY,
                Json(
                    json!({"ok":false,"error":"dispatch_request_failed","scanned":scanned,"attempted":scanned,"sent":0,"failed":failed}),
                ),
            );
        }
    };
    let status = response.status();
    let payload: ReminderDispatchResponse = match response.json().await {
        Ok(v) => v,
        Err(err) => {
            let msg = format!("dispatch_decode_failed: {err}");
            let mut failed = 0_u64;
            for reminder in &due {
                if store
                    .mark_billing_reminder_failed(&reminder.reminder_id, now, Some(&msg))
                    .await
                    .unwrap_or(false)
                {
                    failed = failed.saturating_add(1);
                }
            }
            return (
                StatusCode::BAD_GATEWAY,
                Json(
                    json!({"ok":false,"error":"dispatch_decode_failed","scanned":scanned,"attempted":scanned,"sent":0,"failed":failed}),
                ),
            );
        }
    };
    if !status.is_success() || !payload.ok {
        let msg = format!("dispatch_rejected_status_{}", status.as_u16());
        let mut failed = 0_u64;
        for reminder in &due {
            if store
                .mark_billing_reminder_failed(&reminder.reminder_id, now, Some(&msg))
                .await
                .unwrap_or(false)
            {
                failed = failed.saturating_add(1);
            }
        }
        return (
            StatusCode::BAD_GATEWAY,
            Json(
                json!({"ok":false,"error":"dispatch_rejected","scanned":scanned,"attempted":scanned,"sent":0,"failed":failed}),
            ),
        );
    }

    let mut by_id: std::collections::HashMap<&str, &ReminderDispatchResponseItem> =
        std::collections::HashMap::new();
    for item in &payload.results {
        by_id.insert(item.reminder_id.as_str(), item);
    }
    let mut sent = 0_u64;
    let mut failed = 0_u64;
    for reminder in &due {
        let outcome = by_id.get(reminder.reminder_id.as_str());
        if outcome.map(|v| v.ok).unwrap_or(false) {
            if store
                .mark_billing_reminder_sent(&reminder.reminder_id, now)
                .await
                .unwrap_or(false)
            {
                sent = sent.saturating_add(1);
            }
        } else {
            let err_msg = outcome
                .and_then(|v| v.error.as_deref())
                .unwrap_or("send_failed");
            if store
                .mark_billing_reminder_failed(&reminder.reminder_id, now, Some(err_msg))
                .await
                .unwrap_or(false)
            {
                failed = failed.saturating_add(1);
            }
        }
    }

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "scanned": scanned,
            "attempted": scanned,
            "sent": sent,
            "failed": failed
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        generate_otp_code, license_is_effective_active, process_stripe_event, router,
        verify_stripe_webhook_signature,
    };
    use axum::{
        body::{self, Body},
        http::Request,
    };
    use hmac::{Hmac, Mac};
    use llm_storage::{
        BillingEventAuditRecord, BrokerStore, CustomerRecord, DeviceBindingRecord, LicenseRecord,
        PostgresStore, SqliteStore, SubscriptionRecord,
    };
    use serde_json::json;
    use sha2::Sha256;
    use std::sync::{Arc, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tower::util::ServiceExt;

    fn env_lock() -> &'static tokio::sync::Mutex<()> {
        static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
    }

    fn sign_webhook(secret: &str, body: &str, ts: i64) -> String {
        type HmacSha256 = Hmac<Sha256>;
        let payload = format!("{ts}.{body}");
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac");
        mac.update(payload.as_bytes());
        let sig = super::hex_encode(&mac.finalize().into_bytes());
        format!("t={ts},v1={sig}")
    }

    async fn login_admin(app: &axum::Router) -> String {
        let req = Request::builder()
            .method("POST")
            .uri("/admin/auth/login")
            .header("content-type", "application/json")
            .header("x-forwarded-for", "127.0.0.1")
            .body(Body::from(r#"{"admin_secret":"adm_test"}"#))
            .expect("request");
        let resp = app.clone().oneshot(req).await.expect("response");
        assert_eq!(resp.status(), 200);
        let bytes = body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body");
        let out: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        out.get("sessionId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }

    #[test]
    fn license_effective_active_respects_expiry() {
        let now = 1_000;
        let active_expired = LicenseRecord {
            license_id: "lic_1".to_string(),
            customer_id: "cust_1".to_string(),
            subscription_id: None,
            license_key: "LLMH-abc".to_string(),
            plan_name: "pro".to_string(),
            status: "active".to_string(),
            seat_limit: 1,
            source: "gift".to_string(),
            grace_expires_unix: Some(999),
            created_unix: 1,
            updated_unix: 1,
        };
        assert!(!license_is_effective_active(&active_expired, now));

        let active_no_expiry = LicenseRecord {
            grace_expires_unix: None,
            ..active_expired.clone()
        };
        assert!(license_is_effective_active(&active_no_expiry, now));

        let grace_valid = LicenseRecord {
            status: "grace".to_string(),
            grace_expires_unix: Some(1001),
            ..active_expired.clone()
        };
        assert!(license_is_effective_active(&grace_valid, now));

        let canceled = LicenseRecord {
            status: "canceled".to_string(),
            grace_expires_unix: Some(10_000),
            ..active_expired
        };
        assert!(!license_is_effective_active(&canceled, now));
    }

    #[test]
    fn generated_otp_code_is_six_digits() {
        let code = generate_otp_code();
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn stripe_signature_verification_accepts_valid_signature() {
        type HmacSha256 = Hmac<Sha256>;
        let body = "{\"id\":\"evt_1\"}";
        let ts = 1_000_i64;
        let payload = format!("{ts}.{body}");
        let mut mac = HmacSha256::new_from_slice(b"whsec_test").expect("hmac");
        mac.update(payload.as_bytes());
        let sig = super::hex_encode(&mac.finalize().into_bytes());
        let header = format!("t={ts},v1={sig}");
        assert!(verify_stripe_webhook_signature(
            &header,
            body,
            "whsec_test",
            ts as u64
        ));
    }

    #[tokio::test]
    async fn process_checkout_session_completed_creates_subscription_and_license() {
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        let obj = json!({
            "object": "checkout.session",
            "customer": "cus_1",
            "subscription": "sub_1",
            "customer_details": {"email":"buyer@example.com"},
            "metadata": {"requested_seats":"3"}
        });
        process_stripe_event(
            &(Arc::new(store.clone()) as Arc<dyn BrokerStore>),
            "checkout.session.completed",
            &obj,
        )
        .await
        .expect("process");
        let sub = store
            .get_subscription_by_stripe_subscription_id("sub_1")
            .await
            .expect("lookup")
            .expect("subscription");
        assert_eq!(sub.seat_limit, 3);
        assert_eq!(sub.status, "active");
        let lic = store
            .get_license_by_subscription_id("sub_1")
            .await
            .expect("lookup")
            .expect("license");
        assert_eq!(lic.seat_limit, 3);
        assert_eq!(lic.status, "active");
    }

    #[tokio::test]
    async fn process_invoice_payment_failed_sets_grace() {
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        let sub = SubscriptionRecord {
            subscription_id: "sub_2".to_string(),
            customer_id: "cust_2".to_string(),
            stripe_subscription_id: Some("sub_2".to_string()),
            status: "active".to_string(),
            seat_limit: 2,
            current_period_end_unix: None,
            grace_expires_unix: None,
            created_unix: 1,
            updated_unix: 1,
        };
        store.upsert_subscription(&sub).await.expect("upsert sub");
        let obj = json!({"object":"invoice","subscription":"sub_2"});
        process_stripe_event(
            &(Arc::new(store.clone()) as Arc<dyn BrokerStore>),
            "invoice.payment_failed",
            &obj,
        )
        .await
        .expect("process");
        let updated = store
            .get_subscription_by_stripe_subscription_id("sub_2")
            .await
            .expect("lookup")
            .expect("subscription");
        assert_eq!(updated.status, "grace");
        assert!(updated.grace_expires_unix.is_some());
    }

    #[tokio::test]
    async fn process_invoice_payment_failed_schedules_reminders() {
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        let now = crate::now_unix();
        store
            .upsert_customer(&CustomerRecord {
                customer_id: "cust_r_1".to_string(),
                email: "reminders@example.com".to_string(),
                stripe_customer_id: None,
                created_unix: now,
                updated_unix: now,
            })
            .await
            .expect("upsert customer");
        store
            .upsert_subscription(&SubscriptionRecord {
                subscription_id: "sub_r_1".to_string(),
                customer_id: "cust_r_1".to_string(),
                stripe_subscription_id: Some("sub_r_1".to_string()),
                status: "active".to_string(),
                seat_limit: 2,
                current_period_end_unix: None,
                grace_expires_unix: None,
                created_unix: now,
                updated_unix: now,
            })
            .await
            .expect("upsert sub");

        let obj = json!({"object":"invoice","subscription":"sub_r_1"});
        process_stripe_event(
            &(Arc::new(store.clone()) as Arc<dyn BrokerStore>),
            "invoice.payment_failed",
            &obj,
        )
        .await
        .expect("process");

        let due = store
            .list_due_unsent_billing_reminders(i64::MAX as u64, 20)
            .await
            .expect("list due");
        assert!(due.iter().any(|r| r.reason == "payment_failed_immediate"));
        assert!(due.iter().any(|r| r.reason == "payment_failed_7d"));
        assert!(due.iter().any(|r| r.reason == "payment_failed_3d"));
        assert!(due.iter().any(|r| r.reason == "payment_failed_1d"));
    }

    #[tokio::test]
    async fn process_invoice_paid_revokes_pending_failure_reminders() {
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        let now = crate::now_unix();
        store
            .upsert_customer(&CustomerRecord {
                customer_id: "cust_r_2".to_string(),
                email: "recovery@example.com".to_string(),
                stripe_customer_id: None,
                created_unix: now,
                updated_unix: now,
            })
            .await
            .expect("upsert customer");
        store
            .upsert_subscription(&SubscriptionRecord {
                subscription_id: "sub_r_2".to_string(),
                customer_id: "cust_r_2".to_string(),
                stripe_subscription_id: Some("sub_r_2".to_string()),
                status: "active".to_string(),
                seat_limit: 1,
                current_period_end_unix: None,
                grace_expires_unix: None,
                created_unix: now,
                updated_unix: now,
            })
            .await
            .expect("upsert sub");

        let obj = json!({"object":"invoice","subscription":"sub_r_2"});
        process_stripe_event(
            &(Arc::new(store.clone()) as Arc<dyn BrokerStore>),
            "invoice.payment_failed",
            &obj,
        )
        .await
        .expect("process failed");

        process_stripe_event(
            &(Arc::new(store.clone()) as Arc<dyn BrokerStore>),
            "invoice.paid",
            &obj,
        )
        .await
        .expect("process paid");

        let due = store
            .list_due_unsent_billing_reminders(i64::MAX as u64, 20)
            .await
            .expect("list due");
        assert_eq!(due.len(), 0);
    }

    #[tokio::test]
    async fn billing_reminders_dispatch_route_requires_cron_secret() {
        let _g = env_lock().lock().await;
        std::env::remove_var("CRON_SECRET");
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        let app = router(Arc::new(store));

        let req = Request::builder()
            .method("POST")
            .uri("/api/jobs/billing-reminders-dispatch")
            .body(Body::empty())
            .expect("request");
        let resp = app.clone().oneshot(req).await.expect("response");
        assert_eq!(resp.status(), 400);

        std::env::set_var("CRON_SECRET", "cron_test");
        let req2 = Request::builder()
            .method("POST")
            .uri("/api/jobs/billing-reminders-dispatch")
            .header("x-cron-secret", "wrong")
            .body(Body::empty())
            .expect("request");
        let resp2 = app.oneshot(req2).await.expect("response");
        assert_eq!(resp2.status(), 401);
        std::env::remove_var("CRON_SECRET");
    }

    #[tokio::test]
    async fn process_subscription_updated_rejects_wrong_object_kind() {
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        let obj = json!({"object":"invoice","id":"sub_3","status":"active"});
        let err = process_stripe_event(
            &(Arc::new(store.clone()) as Arc<dyn BrokerStore>),
            "customer.subscription.updated",
            &obj,
        )
        .await
        .expect_err("should fail");
        assert!(err.contains("invalid object kind"));
    }

    #[tokio::test]
    async fn process_subscription_deleted_schedules_cancellation_reminders() {
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        let now = crate::now_unix();
        store
            .upsert_customer(&CustomerRecord {
                customer_id: "cust_cancel_1".to_string(),
                email: "cancel@example.com".to_string(),
                stripe_customer_id: None,
                created_unix: now,
                updated_unix: now,
            })
            .await
            .expect("upsert customer");
        store
            .upsert_subscription(&SubscriptionRecord {
                subscription_id: "sub_cancel_1".to_string(),
                customer_id: "cust_cancel_1".to_string(),
                stripe_subscription_id: Some("sub_cancel_1".to_string()),
                status: "active".to_string(),
                seat_limit: 1,
                current_period_end_unix: Some(now + 10 * 24 * 60 * 60),
                grace_expires_unix: None,
                created_unix: now,
                updated_unix: now,
            })
            .await
            .expect("upsert sub");

        let obj = json!({
            "object":"subscription",
            "id":"sub_cancel_1",
            "status":"canceled",
            "quantity": 1,
            "current_period_end": (now + 10 * 24 * 60 * 60) as i64
        });
        process_stripe_event(
            &(Arc::new(store.clone()) as Arc<dyn BrokerStore>),
            "customer.subscription.deleted",
            &obj,
        )
        .await
        .expect("process");

        let due = store
            .list_due_unsent_billing_reminders(i64::MAX as u64, 20)
            .await
            .expect("list due");
        assert!(due.iter().any(|r| r.reason == "cancel_immediate"));
        assert!(due.iter().any(|r| r.reason == "cancel_7d"));
        assert!(due.iter().any(|r| r.reason == "cancel_3d"));
        assert!(due.iter().any(|r| r.reason == "cancel_1d"));
    }

    #[tokio::test]
    async fn webhook_route_is_idempotent_and_creates_entitlement_state() {
        let _g = env_lock().lock().await;
        std::env::set_var("STRIPE_WEBHOOK_SECRET", "whsec_route");
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        let app = router(Arc::new(store.clone()));
        let body_json = json!({
            "id":"evt_route_1",
            "type":"checkout.session.completed",
            "created": 123,
            "data": {
                "object": {
                    "object":"checkout.session",
                    "customer":"cus_route",
                    "subscription":"sub_route",
                    "customer_details":{"email":"route@example.com"},
                    "metadata":{"requested_seats":"4"}
                }
            }
        });
        let body = serde_json::to_string(&body_json).expect("json");
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_secs() as i64;
        let sig = sign_webhook("whsec_route", &body, ts);

        let req = Request::builder()
            .method("POST")
            .uri("/billing/webhook")
            .header("content-type", "application/json")
            .header("stripe-signature", sig.clone())
            .body(Body::from(body.clone()))
            .expect("request");
        let resp = app.clone().oneshot(req).await.expect("response");
        assert_eq!(resp.status(), 200);

        let sub = store
            .get_subscription_by_stripe_subscription_id("sub_route")
            .await
            .expect("lookup")
            .expect("subscription");
        assert_eq!(sub.seat_limit, 4);
        let lic = store
            .get_license_by_subscription_id("sub_route")
            .await
            .expect("lookup")
            .expect("license");
        assert_eq!(lic.seat_limit, 4);

        let req2 = Request::builder()
            .method("POST")
            .uri("/billing/webhook")
            .header("content-type", "application/json")
            .header("stripe-signature", sig)
            .body(Body::from(body))
            .expect("request");
        let resp2 = app.oneshot(req2).await.expect("response");
        assert_eq!(resp2.status(), 200);
        let bytes = body::to_bytes(resp2.into_body(), usize::MAX)
            .await
            .expect("body");
        let out: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(out.get("duplicate").and_then(|v| v.as_bool()), Some(true));

        std::env::remove_var("STRIPE_WEBHOOK_SECRET");
    }

    #[tokio::test]
    async fn replay_failed_events_route_reprocesses_audited_events() {
        let _g = env_lock().lock().await;
        std::env::set_var("LLMHOTSPOT_ADMIN_SECRET", "adm_test");
        std::env::set_var("LLMHOTSPOT_ADMIN_ALLOW_SECRET_FALLBACK", "1");
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        let app = router(Arc::new(store.clone()));

        let payload = json!({
            "id":"evt_replay_payload",
            "type":"checkout.session.completed",
            "created": 123,
            "data": {
                "object": {
                    "object":"checkout.session",
                    "customer":"cus_replay",
                    "subscription":"sub_replay",
                    "customer_details":{"email":"replay@example.com"},
                    "metadata":{"requested_seats":"2"}
                }
            }
        });
        let row = BillingEventAuditRecord {
            event_id: "evt_failed_1".to_string(),
            event_type: "checkout.session.completed".to_string(),
            created_unix: 123,
            payload_json: payload.to_string(),
            received_unix: 123,
            processed_unix: Some(124),
            process_status: "failed".to_string(),
            error: Some("prior failure".to_string()),
        };
        store
            .insert_billing_event_if_new(&row)
            .await
            .expect("insert");

        let req = Request::builder()
            .method("POST")
            .uri("/admin/maintenance/replay-failed-events")
            .header("content-type", "application/json")
            .header("x-admin-secret", "adm_test")
            .body(Body::from(r#"{"limit":10}"#))
            .expect("request");
        let resp = app.oneshot(req).await.expect("response");
        assert_eq!(resp.status(), 200);

        let bytes = body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body");
        let out: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(out.get("processed").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(out.get("failed").and_then(|v| v.as_u64()), Some(0));

        let ev = store
            .get_billing_event("evt_failed_1")
            .await
            .expect("get")
            .expect("row");
        assert_eq!(ev.process_status, "processed");
        let sub = store
            .get_subscription_by_stripe_subscription_id("sub_replay")
            .await
            .expect("lookup")
            .expect("subscription");
        assert_eq!(sub.seat_limit, 2);

        std::env::remove_var("LLMHOTSPOT_ADMIN_SECRET");
        std::env::remove_var("LLMHOTSPOT_ADMIN_ALLOW_SECRET_FALLBACK");
    }

    #[tokio::test]
    async fn request_otp_route_enforces_rate_limits() {
        let _g = env_lock().lock().await;
        std::env::set_var("LLMHOTSPOT_OTP_DEV_ALLOW_NO_SENDER", "1");
        std::env::set_var("LLMHOTSPOT_OTP_PROVIDER", "disabled");
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        let app = router(Arc::new(store));
        for i in 0..6 {
            let req = Request::builder()
                .method("POST")
                .uri("/api/otp/request")
                .header("content-type", "application/json")
                .header("x-forwarded-for", "10.0.0.1")
                .body(Body::from(
                    r#"{"email":"limit@example.com","purpose":"management"}"#,
                ))
                .expect("request");
            let resp = app.clone().oneshot(req).await.expect("response");
            if i < 5 {
                assert_eq!(resp.status(), 200);
            } else {
                assert_eq!(resp.status(), 429);
            }
        }
        std::env::remove_var("LLMHOTSPOT_OTP_DEV_ALLOW_NO_SENDER");
        std::env::remove_var("LLMHOTSPOT_OTP_PROVIDER");
    }

    #[tokio::test]
    async fn verify_otp_route_enforces_attempt_limits() {
        let _g = env_lock().lock().await;
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        let app = router(Arc::new(store.clone()));
        let challenge = llm_storage::OtpChallengeRecord {
            challenge_id: "ch_limit_1".to_string(),
            email: "user@example.com".to_string(),
            code_hash: super::hash_text("123456"),
            purpose: "management".to_string(),
            target_license_id: None,
            expires_unix: crate::now_unix().saturating_add(600),
            consumed_unix: None,
            created_unix: crate::now_unix(),
        };
        store
            .upsert_otp_challenge(&challenge)
            .await
            .expect("upsert");

        for i in 0..6 {
            let req = Request::builder()
                .method("POST")
                .uri("/api/otp/verify")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"challenge_id":"ch_limit_1","code":"000000"}"#,
                ))
                .expect("request");
            let resp = app.clone().oneshot(req).await.expect("response");
            if i < 5 {
                assert_eq!(resp.status(), 401);
            } else {
                assert_eq!(resp.status(), 429);
            }
        }
    }

    #[tokio::test]
    async fn admin_auth_session_login_logout_works() {
        let _g = env_lock().lock().await;
        std::env::set_var("LLMHOTSPOT_ADMIN_SECRET", "adm_test");
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        let app = router(Arc::new(store));
        let session = login_admin(&app).await;
        assert!(!session.is_empty());

        let req_metrics = Request::builder()
            .method("GET")
            .uri("/admin/metrics")
            .header("x-admin-session", &session)
            .body(Body::empty())
            .expect("request");
        let resp_metrics = app.clone().oneshot(req_metrics).await.expect("response");
        assert_eq!(resp_metrics.status(), 200);

        let req_logout = Request::builder()
            .method("POST")
            .uri("/admin/auth/logout")
            .header("x-admin-session", &session)
            .body(Body::empty())
            .expect("request");
        let resp_logout = app.clone().oneshot(req_logout).await.expect("response");
        assert_eq!(resp_logout.status(), 200);

        let req_metrics_2 = Request::builder()
            .method("GET")
            .uri("/admin/metrics")
            .header("x-admin-session", &session)
            .body(Body::empty())
            .expect("request");
        let resp_metrics_2 = app.oneshot(req_metrics_2).await.expect("response");
        assert_eq!(resp_metrics_2.status(), 401);
        std::env::remove_var("LLMHOTSPOT_ADMIN_SECRET");
    }

    #[tokio::test]
    async fn admin_subscription_and_license_mutations_preserve_entitlement_flow() {
        let _g = env_lock().lock().await;
        std::env::set_var("LLMHOTSPOT_ADMIN_SECRET", "adm_test");
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        let now = crate::now_unix();
        store
            .upsert_customer(&CustomerRecord {
                customer_id: "cust_admin_1".to_string(),
                email: "admin-sub@example.com".to_string(),
                stripe_customer_id: None,
                created_unix: now,
                updated_unix: now,
            })
            .await
            .expect("upsert customer");
        store
            .upsert_subscription(&SubscriptionRecord {
                subscription_id: "sub_admin_1".to_string(),
                customer_id: "cust_admin_1".to_string(),
                stripe_subscription_id: Some("sub_stripe_admin_1".to_string()),
                status: "active".to_string(),
                seat_limit: 3,
                current_period_end_unix: None,
                grace_expires_unix: None,
                created_unix: now,
                updated_unix: now,
            })
            .await
            .expect("upsert sub");
        store
            .upsert_license(&LicenseRecord {
                license_id: "lic_admin_1".to_string(),
                customer_id: "cust_admin_1".to_string(),
                subscription_id: Some("sub_admin_1".to_string()),
                license_key: "LLMH-admin-initial".to_string(),
                plan_name: "pro".to_string(),
                status: "active".to_string(),
                seat_limit: 3,
                source: "stripe".to_string(),
                grace_expires_unix: None,
                created_unix: now,
                updated_unix: now,
            })
            .await
            .expect("upsert license");

        let app = router(Arc::new(store.clone()));
        let session = login_admin(&app).await;

        let req_update = Request::builder()
            .method("POST")
            .uri("/admin/subscriptions/sub_admin_1/update")
            .header("x-admin-session", &session)
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"seat_limit":7,"status":"grace","grace_expires_unix":9999999999}"#,
            ))
            .expect("request");
        let resp_update = app.clone().oneshot(req_update).await.expect("response");
        assert_eq!(resp_update.status(), 200);
        let lic_after_update = store
            .get_license_by_id("lic_admin_1")
            .await
            .expect("lookup")
            .expect("license");
        assert_eq!(lic_after_update.seat_limit, 7);
        assert_eq!(lic_after_update.status, "grace");

        let req_cancel = Request::builder()
            .method("POST")
            .uri("/admin/subscriptions/sub_admin_1/cancel")
            .header("x-admin-session", &session)
            .header("content-type", "application/json")
            .body(Body::from(r#"{}"#))
            .expect("request");
        let resp_cancel = app.clone().oneshot(req_cancel).await.expect("response");
        assert_eq!(resp_cancel.status(), 200);
        let sub_after_cancel = store
            .get_subscription_by_id("sub_admin_1")
            .await
            .expect("lookup")
            .expect("sub");
        assert_eq!(sub_after_cancel.status, "canceled");

        let req_rotate = Request::builder()
            .method("POST")
            .uri("/admin/licenses/lic_admin_1/rotate")
            .header("x-admin-session", &session)
            .body(Body::empty())
            .expect("request");
        let resp_rotate = app.clone().oneshot(req_rotate).await.expect("response");
        assert_eq!(resp_rotate.status(), 200);
        let lic_after_rotate = store
            .get_license_by_id("lic_admin_1")
            .await
            .expect("lookup")
            .expect("license");
        assert_ne!(lic_after_rotate.license_key, "LLMH-admin-initial");

        let req_disable = Request::builder()
            .method("POST")
            .uri("/admin/licenses/lic_admin_1/disable")
            .header("x-admin-session", &session)
            .body(Body::empty())
            .expect("request");
        let resp_disable = app.clone().oneshot(req_disable).await.expect("response");
        assert_eq!(resp_disable.status(), 200);

        let req_activate_disabled = Request::builder()
            .method("POST")
            .uri("/api/licenses/activate")
            .header("content-type", "application/json")
            .body(Body::from(format!(
                "{{\"license_key\":\"{}\",\"device_id\":\"dev_admin_1\"}}",
                lic_after_rotate.license_key
            )))
            .expect("request");
        let resp_activate_disabled = app
            .clone()
            .oneshot(req_activate_disabled)
            .await
            .expect("response");
        assert_eq!(resp_activate_disabled.status(), 402);

        let req_enable = Request::builder()
            .method("POST")
            .uri("/admin/licenses/lic_admin_1/enable")
            .header("x-admin-session", &session)
            .body(Body::empty())
            .expect("request");
        let resp_enable = app.clone().oneshot(req_enable).await.expect("response");
        assert_eq!(resp_enable.status(), 200);

        let req_activate_enabled = Request::builder()
            .method("POST")
            .uri("/api/licenses/activate")
            .header("content-type", "application/json")
            .body(Body::from(format!(
                "{{\"license_key\":\"{}\",\"device_id\":\"dev_admin_1\"}}",
                lic_after_rotate.license_key
            )))
            .expect("request");
        let resp_activate_enabled = app.oneshot(req_activate_enabled).await.expect("response");
        assert_eq!(resp_activate_enabled.status(), 200);
        std::env::remove_var("LLMHOTSPOT_ADMIN_SECRET");
    }

    #[tokio::test]
    async fn admin_seat_routes_and_gift_alias_work() {
        let _g = env_lock().lock().await;
        std::env::set_var("LLMHOTSPOT_ADMIN_SECRET", "adm_test");
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        let now = crate::now_unix();
        store
            .upsert_customer(&CustomerRecord {
                customer_id: "cust_admin_2".to_string(),
                email: "admin-seat@example.com".to_string(),
                stripe_customer_id: None,
                created_unix: now,
                updated_unix: now,
            })
            .await
            .expect("upsert customer");
        store
            .upsert_license(&LicenseRecord {
                license_id: "lic_admin_2".to_string(),
                customer_id: "cust_admin_2".to_string(),
                subscription_id: None,
                license_key: "LLMH-admin-seat".to_string(),
                plan_name: "pro".to_string(),
                status: "active".to_string(),
                seat_limit: 2,
                source: "gift".to_string(),
                grace_expires_unix: Some(now + 3600),
                created_unix: now,
                updated_unix: now,
            })
            .await
            .expect("upsert license");
        store
            .upsert_device_binding(&DeviceBindingRecord {
                binding_id: "bind_admin_1".to_string(),
                license_id: "lic_admin_2".to_string(),
                seat_index: 1,
                device_id: "dev_old".to_string(),
                device_name: None,
                os: None,
                country_code: None,
                last_ip_hash: None,
                app_version: None,
                label: None,
                tunnel_id: None,
                tunnel_hostname: None,
                status: "active".to_string(),
                first_seen_unix: now,
                last_seen_unix: now,
                retired_unix: None,
            })
            .await
            .expect("upsert binding");

        let app = router(Arc::new(store.clone()));
        let session = login_admin(&app).await;

        let req_unassign = Request::builder()
            .method("POST")
            .uri("/admin/seats/bind_admin_1/unassign")
            .header("x-admin-session", &session)
            .body(Body::empty())
            .expect("request");
        let resp_unassign = app.clone().oneshot(req_unassign).await.expect("response");
        assert_eq!(resp_unassign.status(), 200);

        let req_reassign = Request::builder()
            .method("POST")
            .uri("/admin/seats/bind_admin_1/reassign")
            .header("x-admin-session", &session)
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"device_id":"dev_new","device_name":"New Device","os":"linux"}"#,
            ))
            .expect("request");
        let resp_reassign = app.clone().oneshot(req_reassign).await.expect("response");
        assert_eq!(resp_reassign.status(), 200);

        let req_gift_new = Request::builder()
            .method("POST")
            .uri("/admin/seats/gift")
            .header("x-admin-session", &session)
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"email":"gift-new@example.com","seats":1,"duration_days":7}"#,
            ))
            .expect("request");
        let resp_gift_new = app.clone().oneshot(req_gift_new).await.expect("response");
        assert_eq!(resp_gift_new.status(), 200);

        let req_gift_alias = Request::builder()
            .method("POST")
            .uri("/admin/licenses/gift")
            .header("x-admin-session", &session)
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"email":"gift-alias@example.com","seats":1,"duration_days":7}"#,
            ))
            .expect("request");
        let resp_gift_alias = app.oneshot(req_gift_alias).await.expect("response");
        assert_eq!(resp_gift_alias.status(), 200);
        std::env::remove_var("LLMHOTSPOT_ADMIN_SECRET");
    }

    #[tokio::test]
    async fn activate_license_reactivates_retired_same_device_binding() {
        let _g = env_lock().lock().await;
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        let now = crate::now_unix();
        store
            .upsert_customer(&CustomerRecord {
                customer_id: "cust_reactivate_1".to_string(),
                email: "reactivate@example.com".to_string(),
                stripe_customer_id: None,
                created_unix: now,
                updated_unix: now,
            })
            .await
            .expect("upsert customer");
        store
            .upsert_license(&LicenseRecord {
                license_id: "lic_reactivate_1".to_string(),
                customer_id: "cust_reactivate_1".to_string(),
                subscription_id: None,
                license_key: "LLMH-reactivate-1".to_string(),
                plan_name: "pro".to_string(),
                status: "active".to_string(),
                seat_limit: 1,
                source: "gift".to_string(),
                grace_expires_unix: Some(now + 3600),
                created_unix: now,
                updated_unix: now,
            })
            .await
            .expect("upsert license");
        store
            .upsert_device_binding(&DeviceBindingRecord {
                binding_id: "bind_reactivate_1".to_string(),
                license_id: "lic_reactivate_1".to_string(),
                seat_index: 1,
                device_id: "dev_reactivate_1".to_string(),
                device_name: None,
                os: None,
                country_code: None,
                last_ip_hash: None,
                app_version: None,
                label: None,
                tunnel_id: None,
                tunnel_hostname: None,
                status: "retired".to_string(),
                first_seen_unix: now.saturating_sub(100),
                last_seen_unix: now.saturating_sub(10),
                retired_unix: Some(now.saturating_sub(5)),
            })
            .await
            .expect("upsert retired binding");

        let app = router(Arc::new(store.clone()));
        let req_activate = Request::builder()
            .method("POST")
            .uri("/api/licenses/activate")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"license_key":"LLMH-reactivate-1","device_id":"dev_reactivate_1"}"#,
            ))
            .expect("request");
        let resp_activate = app.clone().oneshot(req_activate).await.expect("response");
        assert_eq!(resp_activate.status(), 200);
        let body = body::to_bytes(resp_activate.into_body(), usize::MAX)
            .await
            .expect("body");
        let out: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(out.get("ok").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(out.get("seatIndex").and_then(|v| v.as_i64()), Some(1));

        let active = store
            .get_active_binding_by_license_device("lic_reactivate_1", "dev_reactivate_1")
            .await
            .expect("lookup active")
            .expect("active binding");
        assert_eq!(active.binding_id, "bind_reactivate_1");
        assert_eq!(active.status, "active");
        assert!(active.retired_unix.is_none());
    }

    #[tokio::test]
    async fn endpoint_register_and_details_contract_work() {
        let _g = env_lock().lock().await;
        std::env::set_var("LLMHOTSPOT_ENFORCE_LICENSES", "1");
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        let now = crate::now_unix();
        store
            .upsert_customer(&CustomerRecord {
                customer_id: "cust_ep_1".to_string(),
                email: "endpoint@example.com".to_string(),
                stripe_customer_id: None,
                created_unix: now,
                updated_unix: now,
            })
            .await
            .expect("upsert customer");
        store
            .upsert_license(&LicenseRecord {
                license_id: "lic_ep_1".to_string(),
                customer_id: "cust_ep_1".to_string(),
                subscription_id: None,
                license_key: "LLMH-endpoint-1".to_string(),
                plan_name: "pro".to_string(),
                status: "active".to_string(),
                seat_limit: 2,
                source: "gift".to_string(),
                grace_expires_unix: Some(now + 3600),
                created_unix: now,
                updated_unix: now,
            })
            .await
            .expect("upsert license");
        let app = router(Arc::new(store));

        let req_register = Request::builder()
            .method("POST")
            .uri("/api/endpoints/register")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"deviceId":"dev_ep_1","licenseKey":"LLMH-endpoint-1","slug":"acme-demo"}"#,
            ))
            .expect("request");
        let resp_register = app.clone().oneshot(req_register).await.expect("response");
        assert_eq!(resp_register.status(), 200);

        let req_details = Request::builder()
            .method("POST")
            .uri("/api/endpoints/details")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"deviceId":"dev_ep_1","licenseKey":"LLMH-endpoint-1"}"#,
            ))
            .expect("request");
        let resp_details = app.clone().oneshot(req_details).await.expect("response");
        assert_eq!(resp_details.status(), 200);
        let bytes = body::to_bytes(resp_details.into_body(), usize::MAX)
            .await
            .expect("body");
        let out: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(
            out.pointer("/endpoint/publicBaseUrl")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            "https://acme-demo.llmhotspot.com"
        );

        let req_taken = Request::builder()
            .method("POST")
            .uri("/api/endpoints/register")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"deviceId":"dev_ep_2","licenseKey":"LLMH-endpoint-1","slug":"acme-demo"}"#,
            ))
            .expect("request");
        let resp_taken = app.oneshot(req_taken).await.expect("response");
        assert_eq!(resp_taken.status(), 409);
        std::env::remove_var("LLMHOTSPOT_ENFORCE_LICENSES");
    }

    #[tokio::test]
    async fn customer_context_and_admin_lists_work() {
        let _g = env_lock().lock().await;
        std::env::set_var("LLMHOTSPOT_ADMIN_SECRET", "adm_test");
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        let now = crate::now_unix();
        store
            .upsert_customer(&CustomerRecord {
                customer_id: "cust_view_1".to_string(),
                email: "view@example.com".to_string(),
                stripe_customer_id: Some("cus_view_1".to_string()),
                created_unix: now,
                updated_unix: now,
            })
            .await
            .expect("upsert customer");
        store
            .upsert_subscription(&SubscriptionRecord {
                subscription_id: "sub_view_1".to_string(),
                customer_id: "cust_view_1".to_string(),
                stripe_subscription_id: Some("sub_view_1".to_string()),
                status: "active".to_string(),
                seat_limit: 3,
                current_period_end_unix: None,
                grace_expires_unix: None,
                created_unix: now,
                updated_unix: now,
            })
            .await
            .expect("upsert sub");
        store
            .upsert_license(&LicenseRecord {
                license_id: "lic_view_1".to_string(),
                customer_id: "cust_view_1".to_string(),
                subscription_id: Some("sub_view_1".to_string()),
                license_key: "LLMH-view-1".to_string(),
                plan_name: "pro".to_string(),
                status: "active".to_string(),
                seat_limit: 3,
                source: "stripe".to_string(),
                grace_expires_unix: None,
                created_unix: now,
                updated_unix: now,
            })
            .await
            .expect("upsert license");
        store
            .upsert_device_binding(&DeviceBindingRecord {
                binding_id: "bind_view_1".to_string(),
                license_id: "lic_view_1".to_string(),
                seat_index: 1,
                device_id: "dev_view_1".to_string(),
                device_name: Some("Device 1".to_string()),
                os: Some("linux".to_string()),
                country_code: None,
                last_ip_hash: None,
                app_version: Some("1.0.0".to_string()),
                label: Some("main".to_string()),
                tunnel_id: None,
                tunnel_hostname: None,
                status: "active".to_string(),
                first_seen_unix: now,
                last_seen_unix: now,
                retired_unix: None,
            })
            .await
            .expect("upsert binding");
        store
            .upsert_otp_challenge(&llm_storage::OtpChallengeRecord {
                challenge_id: "ch_view_1".to_string(),
                email: "view@example.com".to_string(),
                code_hash: super::hash_text("123456"),
                purpose: "management".to_string(),
                target_license_id: None,
                expires_unix: now + 600,
                consumed_unix: None,
                created_unix: now,
            })
            .await
            .expect("otp");
        let app = router(Arc::new(store));
        let session = login_admin(&app).await;

        let req_context = Request::builder()
            .method("POST")
            .uri("/api/customer/context")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"email":"view@example.com","challenge_id":"ch_view_1","code":"123456"}"#,
            ))
            .expect("request");
        let resp_context = app.clone().oneshot(req_context).await.expect("response");
        assert_eq!(resp_context.status(), 200);

        for uri in [
            "/admin/subscriptions",
            "/admin/licenses",
            "/admin/seats",
            "/admin/audit-logs",
        ] {
            let req = Request::builder()
                .method("GET")
                .uri(uri)
                .header("x-admin-session", &session)
                .body(Body::empty())
                .expect("request");
            let resp = app.clone().oneshot(req).await.expect("response");
            assert_eq!(resp.status(), 200, "uri={uri}");
        }
        std::env::remove_var("LLMHOTSPOT_ADMIN_SECRET");
    }

    #[tokio::test]
    async fn postgres_broker_smoke_login_and_metrics() {
        let database_url = match std::env::var("LLMHOTSPOT_TEST_POSTGRES_URL") {
            Ok(v) if !v.trim().is_empty() => v,
            _ => return,
        };
        let _g = env_lock().lock().await;
        std::env::set_var("LLMHOTSPOT_ADMIN_SECRET", "adm_test");
        let store = PostgresStore::connect(&database_url)
            .await
            .expect("connect postgres");
        let app = router(Arc::new(store));
        let session = login_admin(&app).await;
        let req_metrics = Request::builder()
            .method("GET")
            .uri("/admin/metrics")
            .header("x-admin-session", &session)
            .body(Body::empty())
            .expect("request");
        let resp_metrics = app.oneshot(req_metrics).await.expect("response");
        assert_eq!(resp_metrics.status(), 200);
        std::env::remove_var("LLMHOTSPOT_ADMIN_SECRET");
    }
}
