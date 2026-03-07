-- Broker/website Postgres schema for monetization + entitlements.

CREATE TABLE IF NOT EXISTS tunnel_leases (
    device_id TEXT PRIMARY KEY,
    endpoint_slug TEXT NOT NULL,
    hostname TEXT NOT NULL,
    tunnel_id TEXT NULL,
    dns_record_id TEXT NULL,
    created_unix BIGINT NOT NULL,
    updated_unix BIGINT NOT NULL,
    last_provisioned_unix BIGINT NULL,
    last_seen_unix BIGINT NULL
);
CREATE INDEX IF NOT EXISTS idx_tunnel_leases_last_seen_unix ON tunnel_leases(last_seen_unix);

CREATE TABLE IF NOT EXISTS customers (
    customer_id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    stripe_customer_id TEXT NULL,
    created_unix BIGINT NOT NULL,
    updated_unix BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS subscriptions (
    subscription_id TEXT PRIMARY KEY,
    customer_id TEXT NOT NULL,
    stripe_subscription_id TEXT NULL UNIQUE,
    status TEXT NOT NULL,
    seat_limit BIGINT NOT NULL,
    current_period_end_unix BIGINT NULL,
    grace_expires_unix BIGINT NULL,
    created_unix BIGINT NOT NULL,
    updated_unix BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS licenses (
    license_id TEXT PRIMARY KEY,
    customer_id TEXT NOT NULL,
    subscription_id TEXT NULL,
    license_key TEXT NOT NULL UNIQUE,
    plan_name TEXT NOT NULL,
    status TEXT NOT NULL,
    seat_limit BIGINT NOT NULL,
    source TEXT NOT NULL,
    grace_expires_unix BIGINT NULL,
    created_unix BIGINT NOT NULL,
    updated_unix BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_licenses_customer_id ON licenses(customer_id);

CREATE TABLE IF NOT EXISTS device_bindings (
    binding_id TEXT PRIMARY KEY,
    license_id TEXT NOT NULL,
    seat_index BIGINT NOT NULL,
    device_id TEXT NOT NULL,
    device_name TEXT NULL,
    os TEXT NULL,
    app_version TEXT NULL,
    label TEXT NULL,
    tunnel_id TEXT NULL,
    tunnel_hostname TEXT NULL,
    status TEXT NOT NULL,
    first_seen_unix BIGINT NOT NULL,
    last_seen_unix BIGINT NOT NULL,
    retired_unix BIGINT NULL,
    UNIQUE(license_id, seat_index),
    UNIQUE(license_id, device_id)
);
CREATE INDEX IF NOT EXISTS idx_device_bindings_license_status ON device_bindings(license_id, status);

CREATE TABLE IF NOT EXISTS otp_challenges (
    challenge_id TEXT PRIMARY KEY,
    email TEXT NOT NULL,
    code_hash TEXT NOT NULL,
    purpose TEXT NOT NULL,
    target_license_id TEXT NULL,
    expires_unix BIGINT NOT NULL,
    consumed_unix BIGINT NULL,
    created_unix BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_otp_challenges_email_purpose ON otp_challenges(email, purpose);

CREATE TABLE IF NOT EXISTS billing_events_audit (
    event_id TEXT PRIMARY KEY,
    event_type TEXT NOT NULL,
    created_unix BIGINT NOT NULL,
    payload_json TEXT NOT NULL,
    received_unix BIGINT NOT NULL,
    processed_unix BIGINT NULL,
    process_status TEXT NOT NULL,
    error TEXT NULL
);
CREATE INDEX IF NOT EXISTS idx_billing_events_received ON billing_events_audit(received_unix);

CREATE TABLE IF NOT EXISTS otp_delivery_events (
    event_id TEXT PRIMARY KEY,
    challenge_id TEXT NOT NULL,
    email TEXT NOT NULL,
    provider TEXT NOT NULL,
    status TEXT NOT NULL,
    error TEXT NULL,
    created_unix BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_otp_delivery_events_challenge ON otp_delivery_events(challenge_id);

CREATE TABLE IF NOT EXISTS otp_rate_limits (
    bucket_key TEXT PRIMARY KEY,
    window_start_unix BIGINT NOT NULL,
    attempt_count BIGINT NOT NULL,
    updated_unix BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS otp_verify_attempts (
    challenge_id TEXT PRIMARY KEY,
    attempt_count BIGINT NOT NULL,
    updated_unix BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS admin_sessions (
    session_id TEXT PRIMARY KEY,
    issued_unix BIGINT NOT NULL,
    expires_unix BIGINT NOT NULL,
    revoked_unix BIGINT NULL,
    source_ip TEXT NULL,
    user_agent_hash TEXT NULL,
    created_by TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_admin_sessions_expires ON admin_sessions(expires_unix);
CREATE INDEX IF NOT EXISTS idx_admin_sessions_revoked ON admin_sessions(revoked_unix);

CREATE TABLE IF NOT EXISTS gift_entitlements (
    gift_id TEXT PRIMARY KEY,
    license_id TEXT NOT NULL,
    recipient_email TEXT NOT NULL,
    seat_count BIGINT NOT NULL,
    starts_unix BIGINT NOT NULL,
    expires_unix BIGINT NOT NULL,
    created_unix BIGINT NOT NULL,
    created_by_admin_session TEXT NULL,
    status TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_gift_entitlements_license ON gift_entitlements(license_id);

CREATE TABLE IF NOT EXISTS audit_logs (
    audit_id TEXT PRIMARY KEY,
    actor_type TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    action TEXT NOT NULL,
    target_type TEXT NOT NULL,
    target_id TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_unix BIGINT NOT NULL,
    ip TEXT NULL,
    outcome TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_audit_logs_created ON audit_logs(created_unix);
