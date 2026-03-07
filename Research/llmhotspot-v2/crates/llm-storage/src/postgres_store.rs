use async_trait::async_trait;
use sqlx::{postgres::PgPoolOptions, PgPool, Row};

use crate::{
    AdminSessionRecord, AppOAuthCodeRecord, AppSessionRecord, AuditLogRecord,
    BillingEventAuditRecord, BillingReminderRecord, BillingRevenueEventRecord, BrokerStore,
    CustomerRecord, DeviceBindingRecord, GiftEntitlementRecord, LicenseRecord, OtpChallengeRecord,
    OtpDeliveryEventRecord, StorageError, SubscriptionRecord, TunnelLeaseRecord,
};

#[derive(Debug, Clone)]
pub struct PostgresStore {
    pool: PgPool,
}

impl PostgresStore {
    pub async fn connect(database_url: &str) -> Result<Self, StorageError> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    async fn migrate(&self) -> Result<(), StorageError> {
        let stmts = [
            "CREATE TABLE IF NOT EXISTS tunnel_leases (
                device_id TEXT PRIMARY KEY,
                endpoint_slug TEXT NOT NULL,
                hostname TEXT NOT NULL,
                tunnel_id TEXT NULL,
                dns_record_id TEXT NULL,
                created_unix BIGINT NOT NULL,
                updated_unix BIGINT NOT NULL,
                last_provisioned_unix BIGINT NULL,
                last_seen_unix BIGINT NULL
            );",
            "CREATE INDEX IF NOT EXISTS idx_tunnel_leases_last_seen_unix ON tunnel_leases(last_seen_unix);",
            "CREATE TABLE IF NOT EXISTS customers (
                customer_id TEXT PRIMARY KEY,
                email TEXT NOT NULL UNIQUE,
                stripe_customer_id TEXT NULL,
                created_unix BIGINT NOT NULL,
                updated_unix BIGINT NOT NULL
            );",
            "CREATE TABLE IF NOT EXISTS subscriptions (
                subscription_id TEXT PRIMARY KEY,
                customer_id TEXT NOT NULL,
                stripe_subscription_id TEXT NULL UNIQUE,
                status TEXT NOT NULL,
                seat_limit BIGINT NOT NULL,
                current_period_end_unix BIGINT NULL,
                grace_expires_unix BIGINT NULL,
                created_unix BIGINT NOT NULL,
                updated_unix BIGINT NOT NULL
            );",
            "CREATE TABLE IF NOT EXISTS licenses (
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
            );",
            "CREATE INDEX IF NOT EXISTS idx_licenses_customer_id ON licenses(customer_id);",
            "CREATE TABLE IF NOT EXISTS device_bindings (
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
                country_code TEXT NULL,
                last_ip_hash TEXT NULL,
                status TEXT NOT NULL,
                first_seen_unix BIGINT NOT NULL,
                last_seen_unix BIGINT NOT NULL,
                retired_unix BIGINT NULL,
                UNIQUE(license_id, seat_index),
                UNIQUE(license_id, device_id)
            );",
            "ALTER TABLE device_bindings ADD COLUMN IF NOT EXISTS country_code TEXT NULL;",
            "ALTER TABLE device_bindings ADD COLUMN IF NOT EXISTS last_ip_hash TEXT NULL;",
            "CREATE INDEX IF NOT EXISTS idx_device_bindings_license_status ON device_bindings(license_id, status);",
            "CREATE TABLE IF NOT EXISTS otp_challenges (
                challenge_id TEXT PRIMARY KEY,
                email TEXT NOT NULL,
                code_hash TEXT NOT NULL,
                purpose TEXT NOT NULL,
                target_license_id TEXT NULL,
                expires_unix BIGINT NOT NULL,
                consumed_unix BIGINT NULL,
                created_unix BIGINT NOT NULL
            );",
            "CREATE INDEX IF NOT EXISTS idx_otp_challenges_email_purpose ON otp_challenges(email, purpose);",
            "CREATE TABLE IF NOT EXISTS billing_events_audit (
                event_id TEXT PRIMARY KEY,
                event_type TEXT NOT NULL,
                created_unix BIGINT NOT NULL,
                payload_json TEXT NOT NULL,
                received_unix BIGINT NOT NULL,
                processed_unix BIGINT NULL,
                process_status TEXT NOT NULL,
                error TEXT NULL
            );",
            "CREATE INDEX IF NOT EXISTS idx_billing_events_received ON billing_events_audit(received_unix);",
            "CREATE TABLE IF NOT EXISTS billing_revenue_events (
                event_id TEXT PRIMARY KEY,
                stripe_subscription_id TEXT NULL,
                customer_id TEXT NULL,
                amount_cents BIGINT NOT NULL,
                currency TEXT NOT NULL,
                kind TEXT NOT NULL,
                occurred_unix BIGINT NOT NULL
            );",
            "CREATE INDEX IF NOT EXISTS idx_billing_revenue_events_occurred
             ON billing_revenue_events(occurred_unix);",
            "CREATE TABLE IF NOT EXISTS billing_reminders (
                reminder_id TEXT PRIMARY KEY,
                subscription_id TEXT NOT NULL,
                license_id TEXT NULL,
                customer_email TEXT NOT NULL,
                reason TEXT NOT NULL,
                due_unix BIGINT NOT NULL,
                sent_unix BIGINT NULL,
                revoked_unix BIGINT NULL,
                attempt_count BIGINT NOT NULL DEFAULT 0,
                last_error TEXT NULL,
                created_unix BIGINT NOT NULL,
                updated_unix BIGINT NOT NULL,
                idempotency_key TEXT NOT NULL UNIQUE
            );",
            "CREATE INDEX IF NOT EXISTS idx_billing_reminders_due_pending
             ON billing_reminders(due_unix, sent_unix, revoked_unix);",
            "CREATE INDEX IF NOT EXISTS idx_billing_reminders_subscription
             ON billing_reminders(subscription_id);",
            "CREATE TABLE IF NOT EXISTS otp_delivery_events (
                event_id TEXT PRIMARY KEY,
                challenge_id TEXT NOT NULL,
                email TEXT NOT NULL,
                provider TEXT NOT NULL,
                status TEXT NOT NULL,
                error TEXT NULL,
                created_unix BIGINT NOT NULL
            );",
            "CREATE INDEX IF NOT EXISTS idx_otp_delivery_events_challenge ON otp_delivery_events(challenge_id);",
            "CREATE TABLE IF NOT EXISTS otp_rate_limits (
                bucket_key TEXT PRIMARY KEY,
                window_start_unix BIGINT NOT NULL,
                attempt_count BIGINT NOT NULL,
                updated_unix BIGINT NOT NULL
            );",
            "CREATE TABLE IF NOT EXISTS otp_verify_attempts (
                challenge_id TEXT PRIMARY KEY,
                attempt_count BIGINT NOT NULL,
                updated_unix BIGINT NOT NULL
            );",
            "CREATE TABLE IF NOT EXISTS admin_sessions (
                session_id TEXT PRIMARY KEY,
                issued_unix BIGINT NOT NULL,
                expires_unix BIGINT NOT NULL,
                revoked_unix BIGINT NULL,
                source_ip TEXT NULL,
                user_agent_hash TEXT NULL,
                created_by TEXT NOT NULL
            );",
            "CREATE INDEX IF NOT EXISTS idx_admin_sessions_expires ON admin_sessions(expires_unix);",
            "CREATE INDEX IF NOT EXISTS idx_admin_sessions_revoked ON admin_sessions(revoked_unix);",
            "CREATE TABLE IF NOT EXISTS app_oauth_codes (
                code_id TEXT PRIMARY KEY,
                customer_id TEXT NOT NULL,
                license_id TEXT NOT NULL,
                state TEXT NOT NULL,
                code_challenge TEXT NOT NULL,
                device_id TEXT NOT NULL,
                expires_unix BIGINT NOT NULL,
                consumed_unix BIGINT NULL,
                created_unix BIGINT NOT NULL
            );",
            "CREATE INDEX IF NOT EXISTS idx_app_oauth_codes_expires ON app_oauth_codes(expires_unix);",
            "CREATE TABLE IF NOT EXISTS app_sessions (
                session_id TEXT PRIMARY KEY,
                customer_id TEXT NOT NULL,
                device_id TEXT NOT NULL,
                license_id TEXT NOT NULL,
                seat_index BIGINT NOT NULL,
                issued_unix BIGINT NOT NULL,
                expires_unix BIGINT NOT NULL,
                revoked_unix BIGINT NULL
            );",
            "CREATE INDEX IF NOT EXISTS idx_app_sessions_expires ON app_sessions(expires_unix);",
            "CREATE INDEX IF NOT EXISTS idx_app_sessions_revoked ON app_sessions(revoked_unix);",
            "CREATE TABLE IF NOT EXISTS gift_entitlements (
                gift_id TEXT PRIMARY KEY,
                license_id TEXT NOT NULL,
                recipient_email TEXT NOT NULL,
                seat_count BIGINT NOT NULL,
                starts_unix BIGINT NOT NULL,
                expires_unix BIGINT NOT NULL,
                created_unix BIGINT NOT NULL,
                created_by_admin_session TEXT NULL,
                status TEXT NOT NULL
            );",
            "CREATE INDEX IF NOT EXISTS idx_gift_entitlements_license ON gift_entitlements(license_id);",
            "CREATE TABLE IF NOT EXISTS audit_logs (
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
            );",
            "CREATE INDEX IF NOT EXISTS idx_audit_logs_created ON audit_logs(created_unix);",
        ];

        for stmt in stmts {
            sqlx::query(stmt)
                .execute(&self.pool)
                .await
                .map_err(|e| StorageError::Database(e.to_string()))?;
        }
        Ok(())
    }
}

fn map_tunnel_lease(row: sqlx::postgres::PgRow) -> TunnelLeaseRecord {
    TunnelLeaseRecord {
        device_id: row.get("device_id"),
        endpoint_slug: row.get("endpoint_slug"),
        hostname: row.get("hostname"),
        tunnel_id: row.get("tunnel_id"),
        dns_record_id: row.get("dns_record_id"),
        created_unix: row.get::<i64, _>("created_unix") as u64,
        updated_unix: row.get::<i64, _>("updated_unix") as u64,
        last_provisioned_unix: row
            .get::<Option<i64>, _>("last_provisioned_unix")
            .map(|v| v as u64),
        last_seen_unix: row
            .get::<Option<i64>, _>("last_seen_unix")
            .map(|v| v as u64),
    }
}

fn map_customer(row: sqlx::postgres::PgRow) -> CustomerRecord {
    CustomerRecord {
        customer_id: row.get("customer_id"),
        email: row.get("email"),
        stripe_customer_id: row.get("stripe_customer_id"),
        created_unix: row.get::<i64, _>("created_unix") as u64,
        updated_unix: row.get::<i64, _>("updated_unix") as u64,
    }
}

fn map_subscription(row: sqlx::postgres::PgRow) -> SubscriptionRecord {
    SubscriptionRecord {
        subscription_id: row.get("subscription_id"),
        customer_id: row.get("customer_id"),
        stripe_subscription_id: row.get("stripe_subscription_id"),
        status: row.get("status"),
        seat_limit: row.get("seat_limit"),
        current_period_end_unix: row
            .get::<Option<i64>, _>("current_period_end_unix")
            .map(|v| v as u64),
        grace_expires_unix: row
            .get::<Option<i64>, _>("grace_expires_unix")
            .map(|v| v as u64),
        created_unix: row.get::<i64, _>("created_unix") as u64,
        updated_unix: row.get::<i64, _>("updated_unix") as u64,
    }
}

fn map_license(row: sqlx::postgres::PgRow) -> LicenseRecord {
    LicenseRecord {
        license_id: row.get("license_id"),
        customer_id: row.get("customer_id"),
        subscription_id: row.get("subscription_id"),
        license_key: row.get("license_key"),
        plan_name: row.get("plan_name"),
        status: row.get("status"),
        seat_limit: row.get("seat_limit"),
        source: row.get("source"),
        grace_expires_unix: row
            .get::<Option<i64>, _>("grace_expires_unix")
            .map(|v| v as u64),
        created_unix: row.get::<i64, _>("created_unix") as u64,
        updated_unix: row.get::<i64, _>("updated_unix") as u64,
    }
}

fn map_binding(row: sqlx::postgres::PgRow) -> DeviceBindingRecord {
    DeviceBindingRecord {
        binding_id: row.get("binding_id"),
        license_id: row.get("license_id"),
        seat_index: row.get("seat_index"),
        device_id: row.get("device_id"),
        device_name: row.get("device_name"),
        os: row.get("os"),
        app_version: row.get("app_version"),
        label: row.get("label"),
        tunnel_id: row.get("tunnel_id"),
        tunnel_hostname: row.get("tunnel_hostname"),
        country_code: row.get("country_code"),
        last_ip_hash: row.get("last_ip_hash"),
        status: row.get("status"),
        first_seen_unix: row.get::<i64, _>("first_seen_unix") as u64,
        last_seen_unix: row.get::<i64, _>("last_seen_unix") as u64,
        retired_unix: row.get::<Option<i64>, _>("retired_unix").map(|v| v as u64),
    }
}

fn map_billing_revenue_event(row: sqlx::postgres::PgRow) -> BillingRevenueEventRecord {
    BillingRevenueEventRecord {
        event_id: row.get("event_id"),
        stripe_subscription_id: row.get("stripe_subscription_id"),
        customer_id: row.get("customer_id"),
        amount_cents: row.get("amount_cents"),
        currency: row.get("currency"),
        kind: row.get("kind"),
        occurred_unix: row.get::<i64, _>("occurred_unix") as u64,
    }
}

fn map_otp_challenge(row: sqlx::postgres::PgRow) -> OtpChallengeRecord {
    OtpChallengeRecord {
        challenge_id: row.get("challenge_id"),
        email: row.get("email"),
        code_hash: row.get("code_hash"),
        purpose: row.get("purpose"),
        target_license_id: row.get("target_license_id"),
        expires_unix: row.get::<i64, _>("expires_unix") as u64,
        consumed_unix: row.get::<Option<i64>, _>("consumed_unix").map(|v| v as u64),
        created_unix: row.get::<i64, _>("created_unix") as u64,
    }
}

fn map_billing_event(row: sqlx::postgres::PgRow) -> BillingEventAuditRecord {
    BillingEventAuditRecord {
        event_id: row.get("event_id"),
        event_type: row.get("event_type"),
        created_unix: row.get::<i64, _>("created_unix") as u64,
        payload_json: row.get("payload_json"),
        received_unix: row.get::<i64, _>("received_unix") as u64,
        processed_unix: row
            .get::<Option<i64>, _>("processed_unix")
            .map(|v| v as u64),
        process_status: row.get("process_status"),
        error: row.get("error"),
    }
}

fn map_billing_reminder(row: sqlx::postgres::PgRow) -> BillingReminderRecord {
    BillingReminderRecord {
        reminder_id: row.get("reminder_id"),
        subscription_id: row.get("subscription_id"),
        license_id: row.get("license_id"),
        customer_email: row.get("customer_email"),
        reason: row.get("reason"),
        due_unix: row.get::<i64, _>("due_unix") as u64,
        sent_unix: row.get::<Option<i64>, _>("sent_unix").map(|v| v as u64),
        revoked_unix: row.get::<Option<i64>, _>("revoked_unix").map(|v| v as u64),
        attempt_count: row.get("attempt_count"),
        last_error: row.get("last_error"),
        created_unix: row.get::<i64, _>("created_unix") as u64,
        updated_unix: row.get::<i64, _>("updated_unix") as u64,
        idempotency_key: row.get("idempotency_key"),
    }
}

fn map_admin_session(row: sqlx::postgres::PgRow) -> AdminSessionRecord {
    AdminSessionRecord {
        session_id: row.get("session_id"),
        issued_unix: row.get::<i64, _>("issued_unix") as u64,
        expires_unix: row.get::<i64, _>("expires_unix") as u64,
        revoked_unix: row.get::<Option<i64>, _>("revoked_unix").map(|v| v as u64),
        source_ip: row.get("source_ip"),
        user_agent_hash: row.get("user_agent_hash"),
        created_by: row.get("created_by"),
    }
}

fn map_app_oauth_code(row: sqlx::postgres::PgRow) -> AppOAuthCodeRecord {
    AppOAuthCodeRecord {
        code_id: row.get("code_id"),
        customer_id: row.get("customer_id"),
        license_id: row.get("license_id"),
        state: row.get("state"),
        code_challenge: row.get("code_challenge"),
        device_id: row.get("device_id"),
        expires_unix: row.get::<i64, _>("expires_unix") as u64,
        consumed_unix: row.get::<Option<i64>, _>("consumed_unix").map(|v| v as u64),
        created_unix: row.get::<i64, _>("created_unix") as u64,
    }
}

fn map_app_session(row: sqlx::postgres::PgRow) -> AppSessionRecord {
    AppSessionRecord {
        session_id: row.get("session_id"),
        customer_id: row.get("customer_id"),
        device_id: row.get("device_id"),
        license_id: row.get("license_id"),
        seat_index: row.get("seat_index"),
        issued_unix: row.get::<i64, _>("issued_unix") as u64,
        expires_unix: row.get::<i64, _>("expires_unix") as u64,
        revoked_unix: row.get::<Option<i64>, _>("revoked_unix").map(|v| v as u64),
    }
}

fn map_audit_log(row: sqlx::postgres::PgRow) -> AuditLogRecord {
    AuditLogRecord {
        audit_id: row.get("audit_id"),
        actor_type: row.get("actor_type"),
        actor_id: row.get("actor_id"),
        action: row.get("action"),
        target_type: row.get("target_type"),
        target_id: row.get("target_id"),
        payload_json: row.get("payload_json"),
        created_unix: row.get::<i64, _>("created_unix") as u64,
        ip: row.get("ip"),
        outcome: row.get("outcome"),
    }
}

#[async_trait]
impl BrokerStore for PostgresStore {
    fn boxed_clone(&self) -> Box<dyn BrokerStore> {
        Box::new(self.clone())
    }

    async fn get_tunnel_lease(
        &self,
        device_id: &str,
    ) -> Result<Option<TunnelLeaseRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT device_id, endpoint_slug, hostname, tunnel_id, dns_record_id,
                    created_unix, updated_unix, last_provisioned_unix, last_seen_unix
             FROM tunnel_leases WHERE device_id = $1 LIMIT 1",
        )
        .bind(device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.map(map_tunnel_lease))
    }

    async fn get_tunnel_lease_by_slug(
        &self,
        endpoint_slug: &str,
    ) -> Result<Option<TunnelLeaseRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT device_id, endpoint_slug, hostname, tunnel_id, dns_record_id,
                    created_unix, updated_unix, last_provisioned_unix, last_seen_unix
             FROM tunnel_leases WHERE endpoint_slug = $1 LIMIT 1",
        )
        .bind(endpoint_slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.map(map_tunnel_lease))
    }

    async fn upsert_tunnel_lease(&self, lease: &TunnelLeaseRecord) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO tunnel_leases(
                device_id, endpoint_slug, hostname, tunnel_id, dns_record_id,
                created_unix, updated_unix, last_provisioned_unix, last_seen_unix
             ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)
             ON CONFLICT(device_id) DO UPDATE SET
                endpoint_slug=excluded.endpoint_slug,
                hostname=excluded.hostname,
                tunnel_id=excluded.tunnel_id,
                dns_record_id=excluded.dns_record_id,
                updated_unix=excluded.updated_unix,
                last_provisioned_unix=excluded.last_provisioned_unix,
                last_seen_unix=excluded.last_seen_unix",
        )
        .bind(&lease.device_id)
        .bind(&lease.endpoint_slug)
        .bind(&lease.hostname)
        .bind(lease.tunnel_id.as_deref())
        .bind(lease.dns_record_id.as_deref())
        .bind(lease.created_unix as i64)
        .bind(lease.updated_unix as i64)
        .bind(lease.last_provisioned_unix.map(|v| v as i64))
        .bind(lease.last_seen_unix.map(|v| v as i64))
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    async fn touch_tunnel_lease_last_seen(
        &self,
        device_id: &str,
        now_unix: u64,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "UPDATE tunnel_leases SET last_seen_unix = $2, updated_unix = $2 WHERE device_id = $1",
        )
        .bind(device_id)
        .bind(now_unix as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }

    async fn list_stale_tunnel_leases(
        &self,
        stale_before_unix: u64,
    ) -> Result<Vec<TunnelLeaseRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT device_id, endpoint_slug, hostname, tunnel_id, dns_record_id,
                    created_unix, updated_unix, last_provisioned_unix, last_seen_unix
             FROM tunnel_leases
             WHERE COALESCE(last_seen_unix, 0) < $1",
        )
        .bind(stale_before_unix as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(rows.into_iter().map(map_tunnel_lease).collect())
    }

    async fn clear_tunnel_lease_provisioning(
        &self,
        device_id: &str,
        now_unix: u64,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "UPDATE tunnel_leases
             SET tunnel_id = NULL, last_provisioned_unix = NULL, updated_unix = $2
             WHERE device_id = $1",
        )
        .bind(device_id)
        .bind(now_unix as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }

    async fn upsert_customer(&self, row: &CustomerRecord) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO customers(customer_id, email, stripe_customer_id, created_unix, updated_unix)
             VALUES($1, $2, $3, $4, $5)
             ON CONFLICT(email) DO UPDATE SET
                stripe_customer_id=excluded.stripe_customer_id,
                updated_unix=excluded.updated_unix",
        )
        .bind(&row.customer_id)
        .bind(&row.email)
        .bind(row.stripe_customer_id.as_deref())
        .bind(row.created_unix as i64)
        .bind(row.updated_unix as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    async fn get_customer_by_email(
        &self,
        email: &str,
    ) -> Result<Option<CustomerRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT customer_id, email, stripe_customer_id, created_unix, updated_unix
             FROM customers WHERE email = $1 LIMIT 1",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.map(map_customer))
    }

    async fn get_customer_by_id(
        &self,
        customer_id: &str,
    ) -> Result<Option<CustomerRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT customer_id, email, stripe_customer_id, created_unix, updated_unix
             FROM customers WHERE customer_id = $1 LIMIT 1",
        )
        .bind(customer_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.map(map_customer))
    }

    async fn get_customer_by_stripe_customer_id(
        &self,
        stripe_customer_id: &str,
    ) -> Result<Option<CustomerRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT customer_id, email, stripe_customer_id, created_unix, updated_unix
             FROM customers WHERE stripe_customer_id = $1 LIMIT 1",
        )
        .bind(stripe_customer_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.map(map_customer))
    }

    async fn upsert_subscription(&self, row: &SubscriptionRecord) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO subscriptions(
                subscription_id, customer_id, stripe_subscription_id, status, seat_limit,
                current_period_end_unix, grace_expires_unix, created_unix, updated_unix
             ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)
             ON CONFLICT(subscription_id) DO UPDATE SET
                customer_id=excluded.customer_id,
                stripe_subscription_id=excluded.stripe_subscription_id,
                status=excluded.status,
                seat_limit=excluded.seat_limit,
                current_period_end_unix=excluded.current_period_end_unix,
                grace_expires_unix=excluded.grace_expires_unix,
                updated_unix=excluded.updated_unix",
        )
        .bind(&row.subscription_id)
        .bind(&row.customer_id)
        .bind(row.stripe_subscription_id.as_deref())
        .bind(&row.status)
        .bind(row.seat_limit)
        .bind(row.current_period_end_unix.map(|v| v as i64))
        .bind(row.grace_expires_unix.map(|v| v as i64))
        .bind(row.created_unix as i64)
        .bind(row.updated_unix as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    async fn get_subscription_by_id(
        &self,
        subscription_id: &str,
    ) -> Result<Option<SubscriptionRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT subscription_id, customer_id, stripe_subscription_id, status, seat_limit,
                    current_period_end_unix, grace_expires_unix, created_unix, updated_unix
             FROM subscriptions WHERE subscription_id = $1 LIMIT 1",
        )
        .bind(subscription_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.map(map_subscription))
    }

    async fn get_subscription_by_stripe_subscription_id(
        &self,
        stripe_subscription_id: &str,
    ) -> Result<Option<SubscriptionRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT subscription_id, customer_id, stripe_subscription_id, status, seat_limit,
                    current_period_end_unix, grace_expires_unix, created_unix, updated_unix
             FROM subscriptions WHERE stripe_subscription_id = $1 LIMIT 1",
        )
        .bind(stripe_subscription_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.map(map_subscription))
    }

    async fn list_subscriptions(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SubscriptionRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT subscription_id, customer_id, stripe_subscription_id, status, seat_limit,
                    current_period_end_unix, grace_expires_unix, created_unix, updated_unix
             FROM subscriptions
             ORDER BY updated_unix DESC, subscription_id DESC
             LIMIT $1 OFFSET $2",
        )
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(rows.into_iter().map(map_subscription).collect())
    }

    async fn list_subscriptions_for_customer(
        &self,
        customer_id: &str,
    ) -> Result<Vec<SubscriptionRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT subscription_id, customer_id, stripe_subscription_id, status, seat_limit,
                    current_period_end_unix, grace_expires_unix, created_unix, updated_unix
             FROM subscriptions
             WHERE customer_id = $1
             ORDER BY updated_unix DESC, subscription_id DESC",
        )
        .bind(customer_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(rows.into_iter().map(map_subscription).collect())
    }

    async fn upsert_license(&self, row: &LicenseRecord) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO licenses(
                license_id, customer_id, subscription_id, license_key, plan_name, status,
                seat_limit, source, grace_expires_unix, created_unix, updated_unix
             ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
             ON CONFLICT(license_id) DO UPDATE SET
                customer_id=excluded.customer_id,
                subscription_id=excluded.subscription_id,
                license_key=excluded.license_key,
                plan_name=excluded.plan_name,
                status=excluded.status,
                seat_limit=excluded.seat_limit,
                source=excluded.source,
                grace_expires_unix=excluded.grace_expires_unix,
                updated_unix=excluded.updated_unix",
        )
        .bind(&row.license_id)
        .bind(&row.customer_id)
        .bind(row.subscription_id.as_deref())
        .bind(&row.license_key)
        .bind(&row.plan_name)
        .bind(&row.status)
        .bind(row.seat_limit)
        .bind(&row.source)
        .bind(row.grace_expires_unix.map(|v| v as i64))
        .bind(row.created_unix as i64)
        .bind(row.updated_unix as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    async fn get_license_by_key(
        &self,
        license_key: &str,
    ) -> Result<Option<LicenseRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT license_id, customer_id, subscription_id, license_key, plan_name, status,
                    seat_limit, source, grace_expires_unix, created_unix, updated_unix
             FROM licenses WHERE license_key = $1 LIMIT 1",
        )
        .bind(license_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.map(map_license))
    }

    async fn get_license_by_subscription_id(
        &self,
        subscription_id: &str,
    ) -> Result<Option<LicenseRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT license_id, customer_id, subscription_id, license_key, plan_name, status,
                    seat_limit, source, grace_expires_unix, created_unix, updated_unix
             FROM licenses WHERE subscription_id = $1 LIMIT 1",
        )
        .bind(subscription_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.map(map_license))
    }

    async fn get_license_by_id(
        &self,
        license_id: &str,
    ) -> Result<Option<LicenseRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT license_id, customer_id, subscription_id, license_key, plan_name, status,
                    seat_limit, source, grace_expires_unix, created_unix, updated_unix
             FROM licenses WHERE license_id = $1 LIMIT 1",
        )
        .bind(license_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.map(map_license))
    }

    async fn list_licenses(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<LicenseRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT license_id, customer_id, subscription_id, license_key, plan_name, status,
                    seat_limit, source, grace_expires_unix, created_unix, updated_unix
             FROM licenses
             ORDER BY updated_unix DESC, license_id DESC
             LIMIT $1 OFFSET $2",
        )
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(rows.into_iter().map(map_license).collect())
    }

    async fn list_licenses_for_customer(
        &self,
        customer_id: &str,
    ) -> Result<Vec<LicenseRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT license_id, customer_id, subscription_id, license_key, plan_name, status,
                    seat_limit, source, grace_expires_unix, created_unix, updated_unix
             FROM licenses
             WHERE customer_id = $1
             ORDER BY updated_unix DESC, license_id DESC",
        )
        .bind(customer_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(rows.into_iter().map(map_license).collect())
    }

    async fn upsert_device_binding(&self, row: &DeviceBindingRecord) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO device_bindings(
                binding_id, license_id, seat_index, device_id, device_name, os, app_version, label,
                tunnel_id, tunnel_hostname, country_code, last_ip_hash, status, first_seen_unix, last_seen_unix, retired_unix
             ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)
             ON CONFLICT(binding_id) DO UPDATE SET
                license_id=excluded.license_id,
                seat_index=excluded.seat_index,
                device_id=excluded.device_id,
                device_name=excluded.device_name,
                os=excluded.os,
                app_version=excluded.app_version,
                label=excluded.label,
                tunnel_id=excluded.tunnel_id,
                tunnel_hostname=excluded.tunnel_hostname,
                country_code=excluded.country_code,
                last_ip_hash=excluded.last_ip_hash,
                status=excluded.status,
                last_seen_unix=excluded.last_seen_unix,
                retired_unix=excluded.retired_unix",
        )
        .bind(&row.binding_id)
        .bind(&row.license_id)
        .bind(row.seat_index)
        .bind(&row.device_id)
        .bind(row.device_name.as_deref())
        .bind(row.os.as_deref())
        .bind(row.app_version.as_deref())
        .bind(row.label.as_deref())
        .bind(row.tunnel_id.as_deref())
        .bind(row.tunnel_hostname.as_deref())
        .bind(row.country_code.as_deref())
        .bind(row.last_ip_hash.as_deref())
        .bind(&row.status)
        .bind(row.first_seen_unix as i64)
        .bind(row.last_seen_unix as i64)
        .bind(row.retired_unix.map(|v| v as i64))
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    async fn list_active_bindings_for_license(
        &self,
        license_id: &str,
    ) -> Result<Vec<DeviceBindingRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT binding_id, license_id, seat_index, device_id, device_name, os, app_version,
                    label, tunnel_id, tunnel_hostname, country_code, last_ip_hash, status, first_seen_unix, last_seen_unix, retired_unix
             FROM device_bindings
             WHERE license_id = $1 AND status = 'active'
             ORDER BY seat_index ASC",
        )
        .bind(license_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(rows.into_iter().map(map_binding).collect())
    }

    async fn list_bindings_for_license(
        &self,
        license_id: &str,
    ) -> Result<Vec<DeviceBindingRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT binding_id, license_id, seat_index, device_id, device_name, os, app_version,
                    label, tunnel_id, tunnel_hostname, country_code, last_ip_hash, status, first_seen_unix, last_seen_unix, retired_unix
             FROM device_bindings
             WHERE license_id = $1
             ORDER BY seat_index ASC, first_seen_unix ASC",
        )
        .bind(license_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(rows.into_iter().map(map_binding).collect())
    }

    async fn get_active_binding_by_license_device(
        &self,
        license_id: &str,
        device_id: &str,
    ) -> Result<Option<DeviceBindingRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT binding_id, license_id, seat_index, device_id, device_name, os, app_version,
                    label, tunnel_id, tunnel_hostname, country_code, last_ip_hash, status, first_seen_unix, last_seen_unix, retired_unix
             FROM device_bindings
             WHERE license_id = $1 AND device_id = $2 AND status = 'active'
             LIMIT 1",
        )
        .bind(license_id)
        .bind(device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.map(map_binding))
    }

    async fn get_binding_by_id(
        &self,
        binding_id: &str,
    ) -> Result<Option<DeviceBindingRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT binding_id, license_id, seat_index, device_id, device_name, os, app_version,
                    label, tunnel_id, tunnel_hostname, country_code, last_ip_hash, status, first_seen_unix, last_seen_unix, retired_unix
             FROM device_bindings
             WHERE binding_id = $1
             LIMIT 1",
        )
        .bind(binding_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.map(map_binding))
    }

    async fn list_bindings(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<DeviceBindingRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT binding_id, license_id, seat_index, device_id, device_name, os, app_version,
                    label, tunnel_id, tunnel_hostname, country_code, last_ip_hash, status, first_seen_unix, last_seen_unix, retired_unix
             FROM device_bindings
             ORDER BY last_seen_unix DESC, binding_id DESC
             LIMIT $1 OFFSET $2",
        )
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(rows.into_iter().map(map_binding).collect())
    }

    async fn retire_binding(
        &self,
        binding_id: &str,
        retired_unix: u64,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "UPDATE device_bindings
             SET status = 'retired', retired_unix = $2, last_seen_unix = $2
             WHERE binding_id = $1 AND status = 'active'",
        )
        .bind(binding_id)
        .bind(retired_unix as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }

    async fn upsert_otp_challenge(&self, row: &OtpChallengeRecord) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO otp_challenges(
                challenge_id, email, code_hash, purpose, target_license_id,
                expires_unix, consumed_unix, created_unix
             ) VALUES($1,$2,$3,$4,$5,$6,$7,$8)
             ON CONFLICT(challenge_id) DO UPDATE SET
                email=excluded.email,
                code_hash=excluded.code_hash,
                purpose=excluded.purpose,
                target_license_id=excluded.target_license_id,
                expires_unix=excluded.expires_unix,
                consumed_unix=excluded.consumed_unix",
        )
        .bind(&row.challenge_id)
        .bind(&row.email)
        .bind(&row.code_hash)
        .bind(&row.purpose)
        .bind(row.target_license_id.as_deref())
        .bind(row.expires_unix as i64)
        .bind(row.consumed_unix.map(|v| v as i64))
        .bind(row.created_unix as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    async fn get_otp_challenge(
        &self,
        challenge_id: &str,
    ) -> Result<Option<OtpChallengeRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT challenge_id, email, code_hash, purpose, target_license_id,
                    expires_unix, consumed_unix, created_unix
             FROM otp_challenges WHERE challenge_id = $1",
        )
        .bind(challenge_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.map(map_otp_challenge))
    }

    async fn consume_otp_challenge(
        &self,
        challenge_id: &str,
        consumed_unix: u64,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "UPDATE otp_challenges
             SET consumed_unix = $2
             WHERE challenge_id = $1 AND consumed_unix IS NULL",
        )
        .bind(challenge_id)
        .bind(consumed_unix as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }

    async fn otp_rate_limit_attempt(
        &self,
        bucket_key: &str,
        now_unix: u64,
        window_secs: u64,
    ) -> Result<u64, StorageError> {
        let row = sqlx::query(
            "SELECT window_start_unix, attempt_count
             FROM otp_rate_limits WHERE bucket_key = $1 LIMIT 1",
        )
        .bind(bucket_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        let (window_start, attempt_count) = if let Some(row) = row {
            (
                row.get::<i64, _>("window_start_unix").max(0) as u64,
                row.get::<i64, _>("attempt_count").max(0) as u64,
            )
        } else {
            (now_unix, 0)
        };
        let in_window = now_unix.saturating_sub(window_start) <= window_secs;
        let next_window_start = if in_window { window_start } else { now_unix };
        let next_count = if in_window {
            attempt_count.saturating_add(1)
        } else {
            1
        };
        sqlx::query(
            "INSERT INTO otp_rate_limits(bucket_key, window_start_unix, attempt_count, updated_unix)
             VALUES($1, $2, $3, $4)
             ON CONFLICT(bucket_key) DO UPDATE SET
                window_start_unix=excluded.window_start_unix,
                attempt_count=excluded.attempt_count,
                updated_unix=excluded.updated_unix",
        )
        .bind(bucket_key)
        .bind(next_window_start as i64)
        .bind(next_count as i64)
        .bind(now_unix as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(next_count)
    }

    async fn otp_verify_attempt(
        &self,
        challenge_id: &str,
        now_unix: u64,
    ) -> Result<u64, StorageError> {
        let row = sqlx::query(
            "SELECT attempt_count FROM otp_verify_attempts
             WHERE challenge_id = $1 LIMIT 1",
        )
        .bind(challenge_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        let next = row
            .map(|r| r.get::<i64, _>("attempt_count").max(0) as u64 + 1)
            .unwrap_or(1);
        sqlx::query(
            "INSERT INTO otp_verify_attempts(challenge_id, attempt_count, updated_unix)
             VALUES($1, $2, $3)
             ON CONFLICT(challenge_id) DO UPDATE SET
                attempt_count=excluded.attempt_count,
                updated_unix=excluded.updated_unix",
        )
        .bind(challenge_id)
        .bind(next as i64)
        .bind(now_unix as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(next)
    }

    async fn clear_otp_verify_attempts(&self, challenge_id: &str) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM otp_verify_attempts WHERE challenge_id = $1")
            .bind(challenge_id)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    async fn insert_otp_delivery_event(
        &self,
        row: &OtpDeliveryEventRecord,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO otp_delivery_events(
                event_id, challenge_id, email, provider, status, error, created_unix
            ) VALUES($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&row.event_id)
        .bind(&row.challenge_id)
        .bind(&row.email)
        .bind(&row.provider)
        .bind(&row.status)
        .bind(row.error.as_deref())
        .bind(row.created_unix as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    async fn count_licenses(&self) -> Result<i64, StorageError> {
        let row = sqlx::query("SELECT COUNT(*) AS c FROM licenses")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.get("c"))
    }

    async fn count_active_device_bindings(&self) -> Result<i64, StorageError> {
        let row = sqlx::query("SELECT COUNT(*) AS c FROM device_bindings WHERE status = 'active'")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.get("c"))
    }

    async fn insert_billing_event_if_new(
        &self,
        row: &BillingEventAuditRecord,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "INSERT INTO billing_events_audit(
                event_id, event_type, created_unix, payload_json, received_unix,
                processed_unix, process_status, error
            ) VALUES($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT(event_id) DO NOTHING",
        )
        .bind(&row.event_id)
        .bind(&row.event_type)
        .bind(row.created_unix as i64)
        .bind(&row.payload_json)
        .bind(row.received_unix as i64)
        .bind(row.processed_unix.map(|v| v as i64))
        .bind(&row.process_status)
        .bind(row.error.as_deref())
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }

    async fn set_billing_event_result(
        &self,
        event_id: &str,
        processed_unix: u64,
        process_status: &str,
        error: Option<&str>,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "UPDATE billing_events_audit
             SET processed_unix = $2, process_status = $3, error = $4
             WHERE event_id = $1",
        )
        .bind(event_id)
        .bind(processed_unix as i64)
        .bind(process_status)
        .bind(error)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }

    async fn get_billing_event(
        &self,
        event_id: &str,
    ) -> Result<Option<BillingEventAuditRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT event_id, event_type, created_unix, payload_json, received_unix,
                    processed_unix, process_status, error
             FROM billing_events_audit WHERE event_id = $1 LIMIT 1",
        )
        .bind(event_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.map(map_billing_event))
    }

    async fn list_billing_events_by_status(
        &self,
        process_status: &str,
        limit: u32,
    ) -> Result<Vec<BillingEventAuditRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT event_id, event_type, created_unix, payload_json, received_unix,
                    processed_unix, process_status, error
             FROM billing_events_audit
             WHERE process_status = $1
             ORDER BY received_unix ASC
             LIMIT $2",
        )
        .bind(process_status)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(rows.into_iter().map(map_billing_event).collect())
    }

    async fn insert_billing_reminder_if_absent(
        &self,
        row: &BillingReminderRecord,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "INSERT INTO billing_reminders(
                reminder_id, subscription_id, license_id, customer_email, reason, due_unix,
                sent_unix, revoked_unix, attempt_count, last_error, created_unix, updated_unix,
                idempotency_key
            ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
            ON CONFLICT(idempotency_key) DO NOTHING",
        )
        .bind(&row.reminder_id)
        .bind(&row.subscription_id)
        .bind(row.license_id.as_deref())
        .bind(&row.customer_email)
        .bind(&row.reason)
        .bind(row.due_unix as i64)
        .bind(row.sent_unix.map(|v| v as i64))
        .bind(row.revoked_unix.map(|v| v as i64))
        .bind(row.attempt_count)
        .bind(row.last_error.as_deref())
        .bind(row.created_unix as i64)
        .bind(row.updated_unix as i64)
        .bind(&row.idempotency_key)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }

    async fn list_due_unsent_billing_reminders(
        &self,
        now_unix: u64,
        limit: u32,
    ) -> Result<Vec<BillingReminderRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT reminder_id, subscription_id, license_id, customer_email, reason, due_unix,
                    sent_unix, revoked_unix, attempt_count, last_error, created_unix, updated_unix,
                    idempotency_key
             FROM billing_reminders
             WHERE due_unix <= $1
               AND sent_unix IS NULL
               AND revoked_unix IS NULL
               AND attempt_count < 20
             ORDER BY due_unix ASC, created_unix ASC
             LIMIT $2",
        )
        .bind(now_unix as i64)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(rows.into_iter().map(map_billing_reminder).collect())
    }

    async fn mark_billing_reminder_sent(
        &self,
        reminder_id: &str,
        sent_unix: u64,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "UPDATE billing_reminders
             SET sent_unix = $2, updated_unix = $2
             WHERE reminder_id = $1 AND sent_unix IS NULL AND revoked_unix IS NULL",
        )
        .bind(reminder_id)
        .bind(sent_unix as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }

    async fn mark_billing_reminder_failed(
        &self,
        reminder_id: &str,
        failed_unix: u64,
        error: Option<&str>,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "UPDATE billing_reminders
             SET attempt_count = attempt_count + 1, last_error = $3, updated_unix = $2
             WHERE reminder_id = $1 AND sent_unix IS NULL AND revoked_unix IS NULL",
        )
        .bind(reminder_id)
        .bind(failed_unix as i64)
        .bind(error)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }

    async fn revoke_pending_billing_reminders(
        &self,
        subscription_id: &str,
        reason_prefixes: &[&str],
        revoked_unix: u64,
    ) -> Result<u64, StorageError> {
        if reason_prefixes.is_empty() {
            return Ok(0);
        }
        let rows = sqlx::query(
            "SELECT reminder_id, reason
             FROM billing_reminders
             WHERE subscription_id = $1
               AND sent_unix IS NULL
               AND revoked_unix IS NULL",
        )
        .bind(subscription_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        let mut revoked = 0_u64;
        for row in rows {
            let reason: String = row.get("reason");
            if !reason_prefixes.iter().any(|p| reason.starts_with(p)) {
                continue;
            }
            let reminder_id: String = row.get("reminder_id");
            let res = sqlx::query(
                "UPDATE billing_reminders
                 SET revoked_unix = $2, updated_unix = $2
                 WHERE reminder_id = $1 AND sent_unix IS NULL AND revoked_unix IS NULL",
            )
            .bind(&reminder_id)
            .bind(revoked_unix as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
            revoked = revoked.saturating_add(res.rows_affected());
        }
        Ok(revoked)
    }

    async fn count_due_billing_reminders(&self, now_unix: u64) -> Result<i64, StorageError> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS c
             FROM billing_reminders
             WHERE due_unix <= $1
               AND sent_unix IS NULL
               AND revoked_unix IS NULL
               AND attempt_count < 20",
        )
        .bind(now_unix as i64)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.get("c"))
    }

    async fn count_sent_billing_reminders_since(
        &self,
        since_unix: u64,
    ) -> Result<i64, StorageError> {
        let row = sqlx::query("SELECT COUNT(*) AS c FROM billing_reminders WHERE sent_unix >= $1")
            .bind(since_unix as i64)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.get("c"))
    }

    async fn count_failed_billing_reminders_since(
        &self,
        since_unix: u64,
    ) -> Result<i64, StorageError> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS c
             FROM billing_reminders
             WHERE sent_unix IS NULL
               AND revoked_unix IS NULL
               AND attempt_count > 0
               AND updated_unix >= $1",
        )
        .bind(since_unix as i64)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.get("c"))
    }

    async fn insert_billing_revenue_event_if_new(
        &self,
        row: &BillingRevenueEventRecord,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "INSERT INTO billing_revenue_events(
                event_id, stripe_subscription_id, customer_id, amount_cents, currency, kind, occurred_unix
            ) VALUES($1,$2,$3,$4,$5,$6,$7)
            ON CONFLICT(event_id) DO NOTHING",
        )
        .bind(&row.event_id)
        .bind(row.stripe_subscription_id.as_deref())
        .bind(row.customer_id.as_deref())
        .bind(row.amount_cents)
        .bind(&row.currency)
        .bind(&row.kind)
        .bind(row.occurred_unix as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }

    async fn list_billing_revenue_events_between(
        &self,
        start_unix: u64,
        end_unix: u64,
    ) -> Result<Vec<BillingRevenueEventRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT event_id, stripe_subscription_id, customer_id, amount_cents, currency, kind, occurred_unix
             FROM billing_revenue_events
             WHERE occurred_unix >= $1 AND occurred_unix <= $2
             ORDER BY occurred_unix ASC, event_id ASC",
        )
        .bind(start_unix as i64)
        .bind(end_unix as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(rows.into_iter().map(map_billing_revenue_event).collect())
    }

    async fn create_admin_session(&self, row: &AdminSessionRecord) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO admin_sessions(
                session_id, issued_unix, expires_unix, revoked_unix, source_ip, user_agent_hash, created_by
            ) VALUES($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&row.session_id)
        .bind(row.issued_unix as i64)
        .bind(row.expires_unix as i64)
        .bind(row.revoked_unix.map(|v| v as i64))
        .bind(row.source_ip.as_deref())
        .bind(row.user_agent_hash.as_deref())
        .bind(&row.created_by)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    async fn get_admin_session(
        &self,
        session_id: &str,
    ) -> Result<Option<AdminSessionRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT session_id, issued_unix, expires_unix, revoked_unix, source_ip, user_agent_hash, created_by
             FROM admin_sessions WHERE session_id = $1 LIMIT 1",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.map(map_admin_session))
    }

    async fn revoke_admin_session(
        &self,
        session_id: &str,
        revoked_unix: u64,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "UPDATE admin_sessions
             SET revoked_unix = $2
             WHERE session_id = $1 AND revoked_unix IS NULL",
        )
        .bind(session_id)
        .bind(revoked_unix as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }

    async fn prune_expired_admin_sessions(&self, now_unix: u64) -> Result<u64, StorageError> {
        let res = sqlx::query("DELETE FROM admin_sessions WHERE expires_unix < $1")
            .bind(now_unix as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected())
    }

    async fn create_app_oauth_code(&self, row: &AppOAuthCodeRecord) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO app_oauth_codes(
                code_id, customer_id, license_id, state, code_challenge, device_id,
                expires_unix, consumed_unix, created_unix
            ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        )
        .bind(&row.code_id)
        .bind(&row.customer_id)
        .bind(&row.license_id)
        .bind(&row.state)
        .bind(&row.code_challenge)
        .bind(&row.device_id)
        .bind(row.expires_unix as i64)
        .bind(row.consumed_unix.map(|v| v as i64))
        .bind(row.created_unix as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    async fn get_app_oauth_code(
        &self,
        code_id: &str,
    ) -> Result<Option<AppOAuthCodeRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT code_id, customer_id, license_id, state, code_challenge, device_id,
                    expires_unix, consumed_unix, created_unix
             FROM app_oauth_codes WHERE code_id = $1 LIMIT 1",
        )
        .bind(code_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.map(map_app_oauth_code))
    }

    async fn consume_app_oauth_code(
        &self,
        code_id: &str,
        consumed_unix: u64,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "UPDATE app_oauth_codes
             SET consumed_unix = $2
             WHERE code_id = $1 AND consumed_unix IS NULL",
        )
        .bind(code_id)
        .bind(consumed_unix as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }

    async fn create_app_session(&self, row: &AppSessionRecord) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO app_sessions(
                session_id, customer_id, device_id, license_id, seat_index, issued_unix, expires_unix, revoked_unix
            ) VALUES($1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(&row.session_id)
        .bind(&row.customer_id)
        .bind(&row.device_id)
        .bind(&row.license_id)
        .bind(row.seat_index)
        .bind(row.issued_unix as i64)
        .bind(row.expires_unix as i64)
        .bind(row.revoked_unix.map(|v| v as i64))
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    async fn get_app_session(
        &self,
        session_id: &str,
    ) -> Result<Option<AppSessionRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT session_id, customer_id, device_id, license_id, seat_index, issued_unix, expires_unix, revoked_unix
             FROM app_sessions WHERE session_id = $1 LIMIT 1",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.map(map_app_session))
    }

    async fn revoke_app_session(
        &self,
        session_id: &str,
        revoked_unix: u64,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "UPDATE app_sessions
             SET revoked_unix = $2
             WHERE session_id = $1 AND revoked_unix IS NULL",
        )
        .bind(session_id)
        .bind(revoked_unix as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }

    async fn prune_expired_app_sessions(&self, now_unix: u64) -> Result<u64, StorageError> {
        let res = sqlx::query("DELETE FROM app_sessions WHERE expires_unix < $1")
            .bind(now_unix as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected())
    }

    async fn insert_gift_entitlement(
        &self,
        row: &GiftEntitlementRecord,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO gift_entitlements(
                gift_id, license_id, recipient_email, seat_count, starts_unix, expires_unix, created_unix,
                created_by_admin_session, status
            ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        )
        .bind(&row.gift_id)
        .bind(&row.license_id)
        .bind(&row.recipient_email)
        .bind(row.seat_count)
        .bind(row.starts_unix as i64)
        .bind(row.expires_unix as i64)
        .bind(row.created_unix as i64)
        .bind(row.created_by_admin_session.as_deref())
        .bind(&row.status)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    async fn insert_audit_log(&self, row: &AuditLogRecord) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO audit_logs(
                audit_id, actor_type, actor_id, action, target_type, target_id, payload_json,
                created_unix, ip, outcome
            ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
        )
        .bind(&row.audit_id)
        .bind(&row.actor_type)
        .bind(&row.actor_id)
        .bind(&row.action)
        .bind(&row.target_type)
        .bind(&row.target_id)
        .bind(&row.payload_json)
        .bind(row.created_unix as i64)
        .bind(row.ip.as_deref())
        .bind(&row.outcome)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    async fn list_audit_logs(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<AuditLogRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT audit_id, actor_type, actor_id, action, target_type, target_id, payload_json,
                    created_unix, ip, outcome
             FROM audit_logs
             ORDER BY created_unix DESC, audit_id DESC
             LIMIT $1 OFFSET $2",
        )
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(rows.into_iter().map(map_audit_log).collect())
    }
}
