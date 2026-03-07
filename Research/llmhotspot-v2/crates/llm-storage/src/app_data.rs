use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OAuthAccountRecord {
    pub account_id: String,
    pub provider_id: String,
    pub display_name: String,
    pub email: String,
    pub priority: i64,
    pub enabled: bool,
    pub last_used_unix: Option<u64>,
    pub last_error: Option<String>,
    pub upstream_base_url: Option<String>,
    pub chat_path: Option<String>,
    pub responses_path: Option<String>,
    pub org_id: Option<String>,
    pub project_id: Option<String>,
    pub extra_headers_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OAuthUsageRecord {
    pub account_id: String,
    pub window_type: String,
    pub used: i64,
    pub limit: i64,
    pub reset_unix: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiKeyRecord {
    pub key_id: String,
    pub label: String,
    pub key_hash: String,
    pub tool_choice_policy: String,
    pub created_unix: u64,
    pub expires_unix: Option<u64>,
    pub revoked_unix: Option<u64>,
    pub last_used_unix: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RemoteAssignment {
    pub remote_base_url: Option<String>,
    pub remote_console_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TunnelLeaseRecord {
    pub device_id: String,
    pub endpoint_slug: String,
    pub hostname: String,
    pub tunnel_id: Option<String>,
    pub dns_record_id: Option<String>,
    pub created_unix: u64,
    pub updated_unix: u64,
    pub last_provisioned_unix: Option<u64>,
    pub last_seen_unix: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomerRecord {
    pub customer_id: String,
    pub email: String,
    pub stripe_customer_id: Option<String>,
    pub created_unix: u64,
    pub updated_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubscriptionRecord {
    pub subscription_id: String,
    pub customer_id: String,
    pub stripe_subscription_id: Option<String>,
    pub status: String,
    pub seat_limit: i64,
    pub current_period_end_unix: Option<u64>,
    pub grace_expires_unix: Option<u64>,
    pub created_unix: u64,
    pub updated_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LicenseRecord {
    pub license_id: String,
    pub customer_id: String,
    pub subscription_id: Option<String>,
    pub license_key: String,
    pub plan_name: String,
    pub status: String,
    pub seat_limit: i64,
    pub source: String,
    pub grace_expires_unix: Option<u64>,
    pub created_unix: u64,
    pub updated_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceBindingRecord {
    pub binding_id: String,
    pub license_id: String,
    pub seat_index: i64,
    pub device_id: String,
    pub device_name: Option<String>,
    pub os: Option<String>,
    pub app_version: Option<String>,
    pub label: Option<String>,
    pub tunnel_id: Option<String>,
    pub tunnel_hostname: Option<String>,
    pub country_code: Option<String>,
    pub last_ip_hash: Option<String>,
    pub status: String,
    pub first_seen_unix: u64,
    pub last_seen_unix: u64,
    pub retired_unix: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OtpChallengeRecord {
    pub challenge_id: String,
    pub email: String,
    pub code_hash: String,
    pub purpose: String,
    pub target_license_id: Option<String>,
    pub expires_unix: u64,
    pub consumed_unix: Option<u64>,
    pub created_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BillingEventAuditRecord {
    pub event_id: String,
    pub event_type: String,
    pub created_unix: u64,
    pub payload_json: String,
    pub received_unix: u64,
    pub processed_unix: Option<u64>,
    pub process_status: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BillingReminderRecord {
    pub reminder_id: String,
    pub subscription_id: String,
    pub license_id: Option<String>,
    pub customer_email: String,
    pub reason: String,
    pub due_unix: u64,
    pub sent_unix: Option<u64>,
    pub revoked_unix: Option<u64>,
    pub attempt_count: i64,
    pub last_error: Option<String>,
    pub created_unix: u64,
    pub updated_unix: u64,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BillingRevenueEventRecord {
    pub event_id: String,
    pub stripe_subscription_id: Option<String>,
    pub customer_id: Option<String>,
    pub amount_cents: i64,
    pub currency: String,
    pub kind: String,
    pub occurred_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OtpDeliveryEventRecord {
    pub event_id: String,
    pub challenge_id: String,
    pub email: String,
    pub provider: String,
    pub status: String,
    pub error: Option<String>,
    pub created_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdminSessionRecord {
    pub session_id: String,
    pub issued_unix: u64,
    pub expires_unix: u64,
    pub revoked_unix: Option<u64>,
    pub source_ip: Option<String>,
    pub user_agent_hash: Option<String>,
    pub created_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppOAuthCodeRecord {
    pub code_id: String,
    pub customer_id: String,
    pub license_id: String,
    pub state: String,
    pub code_challenge: String,
    pub device_id: String,
    pub expires_unix: u64,
    pub consumed_unix: Option<u64>,
    pub created_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppSessionRecord {
    pub session_id: String,
    pub customer_id: String,
    pub device_id: String,
    pub license_id: String,
    pub seat_index: i64,
    pub issued_unix: u64,
    pub expires_unix: u64,
    pub revoked_unix: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GiftEntitlementRecord {
    pub gift_id: String,
    pub license_id: String,
    pub recipient_email: String,
    pub seat_count: i64,
    pub starts_unix: u64,
    pub expires_unix: u64,
    pub created_unix: u64,
    pub created_by_admin_session: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditLogRecord {
    pub audit_id: String,
    pub actor_type: String,
    pub actor_id: String,
    pub action: String,
    pub target_type: String,
    pub target_id: String,
    pub payload_json: String,
    pub created_unix: u64,
    pub ip: Option<String>,
    pub outcome: String,
}
