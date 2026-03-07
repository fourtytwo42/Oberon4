use crate::{
    AdminSessionRecord, ApiKeyRecord, AppOAuthCodeRecord, AppSessionRecord, AuditLogRecord,
    BillingEventAuditRecord, BillingReminderRecord, BillingRevenueEventRecord, BrokerStore,
    CustomerRecord, DeviceBindingRecord, GiftEntitlementRecord, LicenseRecord, OAuthAccountRecord,
    OAuthUsageRecord, OtpChallengeRecord, OtpDeliveryEventRecord, RemoteAssignment, StorageError,
    StoredTokenRecord, SubscriptionRecord, TunnelLeaseRecord,
};
use async_trait::async_trait;
use sqlx::{sqlite::SqlitePoolOptions, Row, SqlitePool};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct SqliteStore {
    pub(crate) pool: SqlitePool,
}

async fn sqlite_column_exists(
    pool: &SqlitePool,
    table: &str,
    column: &str,
) -> Result<bool, StorageError> {
    let pragma = format!("PRAGMA table_info({table})");
    let rows = sqlx::query(&pragma)
        .fetch_all(pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
    Ok(rows
        .iter()
        .any(|row| row.get::<String, _>("name").eq_ignore_ascii_case(column)))
}

#[async_trait]
impl BrokerStore for SqliteStore {
    fn boxed_clone(&self) -> Box<dyn BrokerStore> {
        Box::new(self.clone())
    }

    async fn get_tunnel_lease(
        &self,
        device_id: &str,
    ) -> Result<Option<TunnelLeaseRecord>, StorageError> {
        self.get_tunnel_lease(device_id).await
    }
    async fn get_tunnel_lease_by_slug(
        &self,
        endpoint_slug: &str,
    ) -> Result<Option<TunnelLeaseRecord>, StorageError> {
        self.get_tunnel_lease_by_slug(endpoint_slug).await
    }
    async fn upsert_tunnel_lease(&self, lease: &TunnelLeaseRecord) -> Result<(), StorageError> {
        self.upsert_tunnel_lease(lease).await
    }
    async fn touch_tunnel_lease_last_seen(
        &self,
        device_id: &str,
        now_unix: u64,
    ) -> Result<bool, StorageError> {
        self.touch_tunnel_lease_last_seen(device_id, now_unix).await
    }
    async fn list_stale_tunnel_leases(
        &self,
        stale_before_unix: u64,
    ) -> Result<Vec<TunnelLeaseRecord>, StorageError> {
        self.list_stale_tunnel_leases(stale_before_unix).await
    }
    async fn clear_tunnel_lease_provisioning(
        &self,
        device_id: &str,
        now_unix: u64,
    ) -> Result<bool, StorageError> {
        self.clear_tunnel_lease_provisioning(device_id, now_unix)
            .await
    }
    async fn upsert_customer(&self, row: &CustomerRecord) -> Result<(), StorageError> {
        self.upsert_customer(row).await
    }
    async fn get_customer_by_email(
        &self,
        email: &str,
    ) -> Result<Option<CustomerRecord>, StorageError> {
        self.get_customer_by_email(email).await
    }
    async fn get_customer_by_id(
        &self,
        customer_id: &str,
    ) -> Result<Option<CustomerRecord>, StorageError> {
        self.get_customer_by_id(customer_id).await
    }
    async fn get_customer_by_stripe_customer_id(
        &self,
        stripe_customer_id: &str,
    ) -> Result<Option<CustomerRecord>, StorageError> {
        self.get_customer_by_stripe_customer_id(stripe_customer_id)
            .await
    }
    async fn upsert_subscription(&self, row: &SubscriptionRecord) -> Result<(), StorageError> {
        self.upsert_subscription(row).await
    }
    async fn get_subscription_by_id(
        &self,
        subscription_id: &str,
    ) -> Result<Option<SubscriptionRecord>, StorageError> {
        self.get_subscription_by_id(subscription_id).await
    }
    async fn get_subscription_by_stripe_subscription_id(
        &self,
        stripe_subscription_id: &str,
    ) -> Result<Option<SubscriptionRecord>, StorageError> {
        self.get_subscription_by_stripe_subscription_id(stripe_subscription_id)
            .await
    }
    async fn list_subscriptions(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SubscriptionRecord>, StorageError> {
        self.list_subscriptions(limit, offset).await
    }
    async fn list_subscriptions_for_customer(
        &self,
        customer_id: &str,
    ) -> Result<Vec<SubscriptionRecord>, StorageError> {
        self.list_subscriptions_for_customer(customer_id).await
    }
    async fn upsert_license(&self, row: &LicenseRecord) -> Result<(), StorageError> {
        self.upsert_license(row).await
    }
    async fn get_license_by_key(
        &self,
        license_key: &str,
    ) -> Result<Option<LicenseRecord>, StorageError> {
        self.get_license_by_key(license_key).await
    }
    async fn get_license_by_subscription_id(
        &self,
        subscription_id: &str,
    ) -> Result<Option<LicenseRecord>, StorageError> {
        self.get_license_by_subscription_id(subscription_id).await
    }
    async fn get_license_by_id(
        &self,
        license_id: &str,
    ) -> Result<Option<LicenseRecord>, StorageError> {
        self.get_license_by_id(license_id).await
    }
    async fn list_licenses(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<LicenseRecord>, StorageError> {
        self.list_licenses(limit, offset).await
    }
    async fn list_licenses_for_customer(
        &self,
        customer_id: &str,
    ) -> Result<Vec<LicenseRecord>, StorageError> {
        self.list_licenses_for_customer(customer_id).await
    }
    async fn upsert_device_binding(&self, row: &DeviceBindingRecord) -> Result<(), StorageError> {
        self.upsert_device_binding(row).await
    }
    async fn list_active_bindings_for_license(
        &self,
        license_id: &str,
    ) -> Result<Vec<DeviceBindingRecord>, StorageError> {
        self.list_active_bindings_for_license(license_id).await
    }
    async fn list_bindings_for_license(
        &self,
        license_id: &str,
    ) -> Result<Vec<DeviceBindingRecord>, StorageError> {
        self.list_bindings_for_license(license_id).await
    }
    async fn get_active_binding_by_license_device(
        &self,
        license_id: &str,
        device_id: &str,
    ) -> Result<Option<DeviceBindingRecord>, StorageError> {
        self.get_active_binding_by_license_device(license_id, device_id)
            .await
    }
    async fn get_binding_by_id(
        &self,
        binding_id: &str,
    ) -> Result<Option<DeviceBindingRecord>, StorageError> {
        self.get_binding_by_id(binding_id).await
    }
    async fn list_bindings(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<DeviceBindingRecord>, StorageError> {
        self.list_bindings(limit, offset).await
    }
    async fn retire_binding(
        &self,
        binding_id: &str,
        retired_unix: u64,
    ) -> Result<bool, StorageError> {
        self.retire_binding(binding_id, retired_unix).await
    }
    async fn upsert_otp_challenge(&self, row: &OtpChallengeRecord) -> Result<(), StorageError> {
        self.upsert_otp_challenge(row).await
    }
    async fn get_otp_challenge(
        &self,
        challenge_id: &str,
    ) -> Result<Option<OtpChallengeRecord>, StorageError> {
        self.get_otp_challenge(challenge_id).await
    }
    async fn consume_otp_challenge(
        &self,
        challenge_id: &str,
        consumed_unix: u64,
    ) -> Result<bool, StorageError> {
        self.consume_otp_challenge(challenge_id, consumed_unix)
            .await
    }
    async fn otp_rate_limit_attempt(
        &self,
        bucket_key: &str,
        now_unix: u64,
        window_secs: u64,
    ) -> Result<u64, StorageError> {
        self.otp_rate_limit_attempt(bucket_key, now_unix, window_secs)
            .await
    }
    async fn otp_verify_attempt(
        &self,
        challenge_id: &str,
        now_unix: u64,
    ) -> Result<u64, StorageError> {
        self.otp_verify_attempt(challenge_id, now_unix).await
    }
    async fn clear_otp_verify_attempts(&self, challenge_id: &str) -> Result<(), StorageError> {
        self.clear_otp_verify_attempts(challenge_id).await
    }
    async fn insert_otp_delivery_event(
        &self,
        row: &OtpDeliveryEventRecord,
    ) -> Result<(), StorageError> {
        self.insert_otp_delivery_event(row).await
    }
    async fn count_licenses(&self) -> Result<i64, StorageError> {
        self.count_licenses().await
    }
    async fn count_active_device_bindings(&self) -> Result<i64, StorageError> {
        self.count_active_device_bindings().await
    }
    async fn insert_billing_event_if_new(
        &self,
        row: &BillingEventAuditRecord,
    ) -> Result<bool, StorageError> {
        self.insert_billing_event_if_new(row).await
    }
    async fn set_billing_event_result(
        &self,
        event_id: &str,
        processed_unix: u64,
        process_status: &str,
        error: Option<&str>,
    ) -> Result<bool, StorageError> {
        self.set_billing_event_result(event_id, processed_unix, process_status, error)
            .await
    }
    async fn get_billing_event(
        &self,
        event_id: &str,
    ) -> Result<Option<BillingEventAuditRecord>, StorageError> {
        self.get_billing_event(event_id).await
    }
    async fn list_billing_events_by_status(
        &self,
        process_status: &str,
        limit: u32,
    ) -> Result<Vec<BillingEventAuditRecord>, StorageError> {
        self.list_billing_events_by_status(process_status, limit)
            .await
    }
    async fn insert_billing_reminder_if_absent(
        &self,
        row: &BillingReminderRecord,
    ) -> Result<bool, StorageError> {
        self.insert_billing_reminder_if_absent(row).await
    }
    async fn list_due_unsent_billing_reminders(
        &self,
        now_unix: u64,
        limit: u32,
    ) -> Result<Vec<BillingReminderRecord>, StorageError> {
        self.list_due_unsent_billing_reminders(now_unix, limit)
            .await
    }
    async fn mark_billing_reminder_sent(
        &self,
        reminder_id: &str,
        sent_unix: u64,
    ) -> Result<bool, StorageError> {
        self.mark_billing_reminder_sent(reminder_id, sent_unix)
            .await
    }
    async fn mark_billing_reminder_failed(
        &self,
        reminder_id: &str,
        failed_unix: u64,
        error: Option<&str>,
    ) -> Result<bool, StorageError> {
        self.mark_billing_reminder_failed(reminder_id, failed_unix, error)
            .await
    }
    async fn revoke_pending_billing_reminders(
        &self,
        subscription_id: &str,
        reason_prefixes: &[&str],
        revoked_unix: u64,
    ) -> Result<u64, StorageError> {
        self.revoke_pending_billing_reminders(subscription_id, reason_prefixes, revoked_unix)
            .await
    }
    async fn count_due_billing_reminders(&self, now_unix: u64) -> Result<i64, StorageError> {
        self.count_due_billing_reminders(now_unix).await
    }
    async fn count_sent_billing_reminders_since(
        &self,
        since_unix: u64,
    ) -> Result<i64, StorageError> {
        self.count_sent_billing_reminders_since(since_unix).await
    }
    async fn count_failed_billing_reminders_since(
        &self,
        since_unix: u64,
    ) -> Result<i64, StorageError> {
        self.count_failed_billing_reminders_since(since_unix).await
    }
    async fn insert_billing_revenue_event_if_new(
        &self,
        row: &BillingRevenueEventRecord,
    ) -> Result<bool, StorageError> {
        self.insert_billing_revenue_event_if_new(row).await
    }
    async fn list_billing_revenue_events_between(
        &self,
        start_unix: u64,
        end_unix: u64,
    ) -> Result<Vec<BillingRevenueEventRecord>, StorageError> {
        self.list_billing_revenue_events_between(start_unix, end_unix)
            .await
    }
    async fn create_admin_session(&self, row: &AdminSessionRecord) -> Result<(), StorageError> {
        self.create_admin_session(row).await
    }
    async fn get_admin_session(
        &self,
        session_id: &str,
    ) -> Result<Option<AdminSessionRecord>, StorageError> {
        self.get_admin_session(session_id).await
    }
    async fn revoke_admin_session(
        &self,
        session_id: &str,
        revoked_unix: u64,
    ) -> Result<bool, StorageError> {
        self.revoke_admin_session(session_id, revoked_unix).await
    }
    async fn prune_expired_admin_sessions(&self, now_unix: u64) -> Result<u64, StorageError> {
        self.prune_expired_admin_sessions(now_unix).await
    }
    async fn create_app_oauth_code(&self, row: &AppOAuthCodeRecord) -> Result<(), StorageError> {
        self.create_app_oauth_code(row).await
    }
    async fn get_app_oauth_code(
        &self,
        code_id: &str,
    ) -> Result<Option<AppOAuthCodeRecord>, StorageError> {
        self.get_app_oauth_code(code_id).await
    }
    async fn consume_app_oauth_code(
        &self,
        code_id: &str,
        consumed_unix: u64,
    ) -> Result<bool, StorageError> {
        self.consume_app_oauth_code(code_id, consumed_unix).await
    }
    async fn create_app_session(&self, row: &AppSessionRecord) -> Result<(), StorageError> {
        self.create_app_session(row).await
    }
    async fn get_app_session(
        &self,
        session_id: &str,
    ) -> Result<Option<AppSessionRecord>, StorageError> {
        self.get_app_session(session_id).await
    }
    async fn revoke_app_session(
        &self,
        session_id: &str,
        revoked_unix: u64,
    ) -> Result<bool, StorageError> {
        self.revoke_app_session(session_id, revoked_unix).await
    }
    async fn prune_expired_app_sessions(&self, now_unix: u64) -> Result<u64, StorageError> {
        self.prune_expired_app_sessions(now_unix).await
    }
    async fn insert_gift_entitlement(
        &self,
        row: &GiftEntitlementRecord,
    ) -> Result<(), StorageError> {
        self.insert_gift_entitlement(row).await
    }
    async fn insert_audit_log(&self, row: &AuditLogRecord) -> Result<(), StorageError> {
        self.insert_audit_log(row).await
    }
    async fn list_audit_logs(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<AuditLogRecord>, StorageError> {
        self.list_audit_logs(limit, offset).await
    }
}

impl SqliteStore {
    pub async fn connect(database_url: &str) -> Result<Self, StorageError> {
        let max_connections = std::env::var("LLMHOTSPOT_SQLITE_MAX_CONNECTIONS")
            .ok()
            .and_then(|v| v.trim().parse::<u32>().ok())
            .filter(|v| *v >= 2)
            .unwrap_or(8);
        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .min_connections(1)
            .acquire_timeout(Duration::from_secs(3))
            .after_connect(|conn, _meta| {
                Box::pin(async move {
                    // Reduce lock contention for mixed read/write workloads (console + streaming diagnostics).
                    sqlx::query("PRAGMA journal_mode = WAL;")
                        .execute(&mut *conn)
                        .await?;
                    sqlx::query("PRAGMA synchronous = NORMAL;")
                        .execute(&mut *conn)
                        .await?;
                    sqlx::query("PRAGMA busy_timeout = 3000;")
                        .execute(&mut *conn)
                        .await?;
                    Ok(())
                })
            })
            .connect(database_url)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    async fn migrate(&self) -> Result<(), StorageError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS kv_state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                expires_unix INTEGER NULL
            );",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS oauth_tokens (
                workspace_id TEXT NOT NULL,
                connection_id TEXT NOT NULL,
                payload TEXT NOT NULL,
                PRIMARY KEY(workspace_id, connection_id)
            );",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS oauth_accounts (
                account_id TEXT PRIMARY KEY,
                provider_id TEXT NOT NULL DEFAULT 'openai',
                display_name TEXT NOT NULL,
                email TEXT NOT NULL,
                priority INTEGER NOT NULL,
                enabled INTEGER NOT NULL,
                last_used_unix INTEGER NULL,
                last_error TEXT NULL,
                upstream_base_url TEXT NULL,
                chat_path TEXT NULL,
                responses_path TEXT NULL,
                org_id TEXT NULL,
                project_id TEXT NULL,
                extra_headers_json TEXT NULL
            );",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;

        self.add_column_if_missing("oauth_accounts", "upstream_base_url", "TEXT NULL")
            .await?;
        self.add_column_if_missing("oauth_accounts", "chat_path", "TEXT NULL")
            .await?;
        self.add_column_if_missing("oauth_accounts", "responses_path", "TEXT NULL")
            .await?;
        self.add_column_if_missing("oauth_accounts", "org_id", "TEXT NULL")
            .await?;
        self.add_column_if_missing("oauth_accounts", "project_id", "TEXT NULL")
            .await?;
        self.add_column_if_missing("oauth_accounts", "extra_headers_json", "TEXT NULL")
            .await?;
        self.add_column_if_missing(
            "oauth_accounts",
            "provider_id",
            "TEXT NOT NULL DEFAULT 'openai'",
        )
        .await?;
        sqlx::query(
            "UPDATE oauth_accounts
             SET provider_id = CASE
               WHEN COALESCE(upstream_base_url, '') = 'https://api.anthropic.com' THEN 'anthropic'
               ELSE COALESCE(NULLIF(provider_id, ''), 'openai')
             END",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS oauth_usage (
                account_id TEXT NOT NULL,
                window_type TEXT NOT NULL,
                used INTEGER NOT NULL,
                limit_value INTEGER NOT NULL,
                reset_unix INTEGER NOT NULL,
                PRIMARY KEY(account_id, window_type)
            );",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS api_keys (
                key_id TEXT PRIMARY KEY,
                label TEXT NOT NULL,
                key_hash TEXT NOT NULL,
                tool_choice_policy TEXT NOT NULL DEFAULT 'auto',
                created_unix INTEGER NOT NULL,
                expires_unix INTEGER NULL,
                revoked_unix INTEGER NULL,
                last_used_unix INTEGER NULL
            );",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        self.add_column_if_missing(
            "api_keys",
            "tool_choice_policy",
            "TEXT NOT NULL DEFAULT 'auto'",
        )
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_api_keys_key_hash
             ON api_keys(key_hash);",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS app_state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS tunnel_leases (
                device_id TEXT PRIMARY KEY,
                endpoint_slug TEXT NOT NULL,
                hostname TEXT NOT NULL,
                tunnel_id TEXT NULL,
                dns_record_id TEXT NULL,
                created_unix INTEGER NOT NULL,
                updated_unix INTEGER NOT NULL,
                last_provisioned_unix INTEGER NULL,
                last_seen_unix INTEGER NULL
            );",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS customers (
                customer_id TEXT PRIMARY KEY,
                email TEXT NOT NULL UNIQUE,
                stripe_customer_id TEXT NULL,
                created_unix INTEGER NOT NULL,
                updated_unix INTEGER NOT NULL
            );",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS subscriptions (
                subscription_id TEXT PRIMARY KEY,
                customer_id TEXT NOT NULL,
                stripe_subscription_id TEXT NULL UNIQUE,
                status TEXT NOT NULL,
                seat_limit INTEGER NOT NULL,
                current_period_end_unix INTEGER NULL,
                grace_expires_unix INTEGER NULL,
                created_unix INTEGER NOT NULL,
                updated_unix INTEGER NOT NULL
            );",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS licenses (
                license_id TEXT PRIMARY KEY,
                customer_id TEXT NOT NULL,
                subscription_id TEXT NULL,
                license_key TEXT NOT NULL UNIQUE,
                plan_name TEXT NOT NULL,
                status TEXT NOT NULL,
                seat_limit INTEGER NOT NULL,
                source TEXT NOT NULL,
                grace_expires_unix INTEGER NULL,
                created_unix INTEGER NOT NULL,
                updated_unix INTEGER NOT NULL
            );",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_licenses_customer_id
             ON licenses(customer_id);",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS device_bindings (
                binding_id TEXT PRIMARY KEY,
                license_id TEXT NOT NULL,
                seat_index INTEGER NOT NULL,
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
                first_seen_unix INTEGER NOT NULL,
                last_seen_unix INTEGER NOT NULL,
                retired_unix INTEGER NULL,
                UNIQUE(license_id, seat_index),
                UNIQUE(license_id, device_id)
            );",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        if !sqlite_column_exists(&self.pool, "device_bindings", "country_code").await? {
            sqlx::query("ALTER TABLE device_bindings ADD COLUMN country_code TEXT NULL;")
                .execute(&self.pool)
                .await
                .map_err(|e| StorageError::Database(e.to_string()))?;
        }
        if !sqlite_column_exists(&self.pool, "device_bindings", "last_ip_hash").await? {
            sqlx::query("ALTER TABLE device_bindings ADD COLUMN last_ip_hash TEXT NULL;")
                .execute(&self.pool)
                .await
                .map_err(|e| StorageError::Database(e.to_string()))?;
        }
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_device_bindings_license_status
             ON device_bindings(license_id, status);",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS otp_challenges (
                challenge_id TEXT PRIMARY KEY,
                email TEXT NOT NULL,
                code_hash TEXT NOT NULL,
                purpose TEXT NOT NULL,
                target_license_id TEXT NULL,
                expires_unix INTEGER NOT NULL,
                consumed_unix INTEGER NULL,
                created_unix INTEGER NOT NULL
            );",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_otp_challenges_email_purpose
             ON otp_challenges(email, purpose);",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS billing_events_audit (
                event_id TEXT PRIMARY KEY,
                event_type TEXT NOT NULL,
                created_unix INTEGER NOT NULL,
                payload_json TEXT NOT NULL,
                received_unix INTEGER NOT NULL,
                processed_unix INTEGER NULL,
                process_status TEXT NOT NULL,
                error TEXT NULL
            );",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_billing_events_received
             ON billing_events_audit(received_unix);",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS billing_revenue_events (
                event_id TEXT PRIMARY KEY,
                stripe_subscription_id TEXT NULL,
                customer_id TEXT NULL,
                amount_cents INTEGER NOT NULL,
                currency TEXT NOT NULL,
                kind TEXT NOT NULL,
                occurred_unix INTEGER NOT NULL
            );",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_billing_revenue_events_occurred
             ON billing_revenue_events(occurred_unix);",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS billing_reminders (
                reminder_id TEXT PRIMARY KEY,
                subscription_id TEXT NOT NULL,
                license_id TEXT NULL,
                customer_email TEXT NOT NULL,
                reason TEXT NOT NULL,
                due_unix INTEGER NOT NULL,
                sent_unix INTEGER NULL,
                revoked_unix INTEGER NULL,
                attempt_count INTEGER NOT NULL DEFAULT 0,
                last_error TEXT NULL,
                created_unix INTEGER NOT NULL,
                updated_unix INTEGER NOT NULL,
                idempotency_key TEXT NOT NULL UNIQUE
            );",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_billing_reminders_due_pending
             ON billing_reminders(due_unix, sent_unix, revoked_unix);",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_billing_reminders_subscription
             ON billing_reminders(subscription_id);",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS otp_delivery_events (
                event_id TEXT PRIMARY KEY,
                challenge_id TEXT NOT NULL,
                email TEXT NOT NULL,
                provider TEXT NOT NULL,
                status TEXT NOT NULL,
                error TEXT NULL,
                created_unix INTEGER NOT NULL
            );",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_otp_delivery_events_challenge
             ON otp_delivery_events(challenge_id);",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS otp_rate_limits (
                bucket_key TEXT PRIMARY KEY,
                window_start_unix INTEGER NOT NULL,
                attempt_count INTEGER NOT NULL,
                updated_unix INTEGER NOT NULL
            );",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS otp_verify_attempts (
                challenge_id TEXT PRIMARY KEY,
                attempt_count INTEGER NOT NULL,
                updated_unix INTEGER NOT NULL
            );",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS admin_sessions (
                session_id TEXT PRIMARY KEY,
                issued_unix INTEGER NOT NULL,
                expires_unix INTEGER NOT NULL,
                revoked_unix INTEGER NULL,
                source_ip TEXT NULL,
                user_agent_hash TEXT NULL,
                created_by TEXT NOT NULL
            );",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_admin_sessions_expires
             ON admin_sessions(expires_unix);",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_admin_sessions_revoked
             ON admin_sessions(revoked_unix);",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS app_oauth_codes (
                code_id TEXT PRIMARY KEY,
                customer_id TEXT NOT NULL,
                license_id TEXT NOT NULL,
                state TEXT NOT NULL,
                code_challenge TEXT NOT NULL,
                device_id TEXT NOT NULL,
                expires_unix INTEGER NOT NULL,
                consumed_unix INTEGER NULL,
                created_unix INTEGER NOT NULL
            );",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_app_oauth_codes_expires
             ON app_oauth_codes(expires_unix);",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS app_sessions (
                session_id TEXT PRIMARY KEY,
                customer_id TEXT NOT NULL,
                device_id TEXT NOT NULL,
                license_id TEXT NOT NULL,
                seat_index INTEGER NOT NULL,
                issued_unix INTEGER NOT NULL,
                expires_unix INTEGER NOT NULL,
                revoked_unix INTEGER NULL
            );",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_app_sessions_expires
             ON app_sessions(expires_unix);",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_app_sessions_revoked
             ON app_sessions(revoked_unix);",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS gift_entitlements (
                gift_id TEXT PRIMARY KEY,
                license_id TEXT NOT NULL,
                recipient_email TEXT NOT NULL,
                seat_count INTEGER NOT NULL,
                starts_unix INTEGER NOT NULL,
                expires_unix INTEGER NOT NULL,
                created_unix INTEGER NOT NULL,
                created_by_admin_session TEXT NULL,
                status TEXT NOT NULL
            );",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_gift_entitlements_license
             ON gift_entitlements(license_id);",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS audit_logs (
                audit_id TEXT PRIMARY KEY,
                actor_type TEXT NOT NULL,
                actor_id TEXT NOT NULL,
                action TEXT NOT NULL,
                target_type TEXT NOT NULL,
                target_id TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                created_unix INTEGER NOT NULL,
                ip TEXT NULL,
                outcome TEXT NOT NULL
            );",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_audit_logs_created
             ON audit_logs(created_unix);",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_tunnel_leases_last_seen_unix
             ON tunnel_leases(last_seen_unix);",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;

        Ok(())
    }

    async fn add_column_if_missing(
        &self,
        table: &str,
        column: &str,
        sql_type: &str,
    ) -> Result<(), StorageError> {
        let stmt = format!("ALTER TABLE {table} ADD COLUMN {column} {sql_type};");
        let result = sqlx::query(&stmt).execute(&self.pool).await;
        if let Err(err) = result {
            let text = err.to_string();
            if !text.contains("duplicate column name") {
                return Err(StorageError::Database(text));
            }
        }
        Ok(())
    }

    pub async fn put_state(
        &self,
        key: &str,
        value: &str,
        expires_unix: Option<u64>,
    ) -> Result<(), StorageError> {
        if key.trim().is_empty() {
            return Err(StorageError::InvalidKey);
        }

        sqlx::query(
            "INSERT INTO kv_state(key, value, expires_unix)
             VALUES(?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET
               value=excluded.value,
               expires_unix=excluded.expires_unix;",
        )
        .bind(key)
        .bind(value)
        .bind(expires_unix.map(|v| v as i64))
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;

        Ok(())
    }

    pub async fn get_state(&self, key: &str, now_unix: u64) -> Result<String, StorageError> {
        let row = sqlx::query("SELECT value, expires_unix FROM kv_state WHERE key = ?1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let Some(row) = row else {
            return Err(StorageError::NotFound);
        };

        let value: String = row.get("value");
        let expires_unix: Option<i64> = row.get("expires_unix");
        if let Some(expires) = expires_unix {
            if expires <= now_unix as i64 {
                self.delete_state(key).await?;
                return Err(StorageError::NotFound);
            }
        }

        Ok(value)
    }

    pub async fn delete_state(&self, key: &str) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM kv_state WHERE key = ?1")
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    pub async fn save_token(&self, record: &StoredTokenRecord) -> Result<(), StorageError> {
        let payload = serde_json::to_string(record)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        sqlx::query(
            "INSERT INTO oauth_tokens(workspace_id, connection_id, payload)
             VALUES(?1, ?2, ?3)
             ON CONFLICT(workspace_id, connection_id) DO UPDATE SET
               payload=excluded.payload;",
        )
        .bind(&record.workspace_id)
        .bind(&record.connection_id)
        .bind(payload)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;

        Ok(())
    }

    pub async fn load_token(
        &self,
        workspace_id: &str,
        connection_id: &str,
    ) -> Result<Option<StoredTokenRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT payload FROM oauth_tokens WHERE workspace_id = ?1 AND connection_id = ?2",
        )
        .bind(workspace_id)
        .bind(connection_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;

        let Some(row) = row else {
            return Ok(None);
        };

        let payload: String = row.get("payload");
        let record = serde_json::from_str::<StoredTokenRecord>(&payload)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        Ok(Some(record))
    }

    pub async fn delete_token(
        &self,
        workspace_id: &str,
        connection_id: &str,
    ) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM oauth_tokens WHERE workspace_id = ?1 AND connection_id = ?2")
            .bind(workspace_id)
            .bind(connection_id)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    pub async fn upsert_oauth_account(&self, row: &OAuthAccountRecord) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO oauth_accounts(
                account_id, provider_id, display_name, email, priority, enabled, last_used_unix, last_error,
                upstream_base_url, chat_path, responses_path, org_id, project_id, extra_headers_json
             )
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT(account_id) DO UPDATE SET
               provider_id=excluded.provider_id,
               display_name=excluded.display_name,
               email=excluded.email,
               priority=excluded.priority,
               enabled=excluded.enabled,
               last_used_unix=excluded.last_used_unix,
               last_error=excluded.last_error,
               upstream_base_url=excluded.upstream_base_url,
               chat_path=excluded.chat_path,
               responses_path=excluded.responses_path,
               org_id=excluded.org_id,
               project_id=excluded.project_id,
               extra_headers_json=excluded.extra_headers_json;",
        )
        .bind(&row.account_id)
        .bind(&row.provider_id)
        .bind(&row.display_name)
        .bind(&row.email)
        .bind(row.priority)
        .bind(row.enabled as i64)
        .bind(row.last_used_unix.map(|v| v as i64))
        .bind(row.last_error.as_deref())
        .bind(row.upstream_base_url.as_deref())
        .bind(row.chat_path.as_deref())
        .bind(row.responses_path.as_deref())
        .bind(row.org_id.as_deref())
        .bind(row.project_id.as_deref())
        .bind(row.extra_headers_json.as_deref())
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    pub async fn list_oauth_accounts(&self) -> Result<Vec<OAuthAccountRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT account_id, provider_id, display_name, email, priority, enabled, last_used_unix, last_error,
                    upstream_base_url, chat_path, responses_path, org_id, project_id, extra_headers_json
             FROM oauth_accounts
             ORDER BY priority ASC, account_id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|row| OAuthAccountRecord {
                account_id: row.get("account_id"),
                provider_id: row.get("provider_id"),
                display_name: row.get("display_name"),
                email: row.get("email"),
                priority: row.get("priority"),
                enabled: row.get::<i64, _>("enabled") != 0,
                last_used_unix: row
                    .get::<Option<i64>, _>("last_used_unix")
                    .map(|v| v as u64),
                last_error: row.get("last_error"),
                upstream_base_url: row.get("upstream_base_url"),
                chat_path: row.get("chat_path"),
                responses_path: row.get("responses_path"),
                org_id: row.get("org_id"),
                project_id: row.get("project_id"),
                extra_headers_json: row.get("extra_headers_json"),
            })
            .collect())
    }

    pub async fn get_oauth_account(
        &self,
        account_id: &str,
    ) -> Result<Option<OAuthAccountRecord>, StorageError> {
        let items = self.list_oauth_accounts().await?;
        Ok(items.into_iter().find(|a| a.account_id == account_id))
    }

    pub async fn delete_oauth_account(&self, account_id: &str) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM oauth_usage WHERE account_id = ?1")
            .bind(account_id)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        sqlx::query("DELETE FROM oauth_accounts WHERE account_id = ?1")
            .bind(account_id)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    pub async fn update_oauth_enabled(
        &self,
        account_id: &str,
        enabled: bool,
    ) -> Result<(), StorageError> {
        sqlx::query("UPDATE oauth_accounts SET enabled = ?2 WHERE account_id = ?1")
            .bind(account_id)
            .bind(enabled as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    pub async fn set_oauth_priority(
        &self,
        account_id: &str,
        priority: i64,
    ) -> Result<(), StorageError> {
        sqlx::query("UPDATE oauth_accounts SET priority = ?2 WHERE account_id = ?1")
            .bind(account_id)
            .bind(priority)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    pub async fn upsert_oauth_usage(&self, row: &OAuthUsageRecord) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO oauth_usage(account_id, window_type, used, limit_value, reset_unix)
             VALUES(?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(account_id, window_type) DO UPDATE SET
               used=excluded.used,
               limit_value=excluded.limit_value,
               reset_unix=excluded.reset_unix;",
        )
        .bind(&row.account_id)
        .bind(&row.window_type)
        .bind(row.used)
        .bind(row.limit)
        .bind(row.reset_unix)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    pub async fn list_oauth_usage(
        &self,
        account_id: &str,
    ) -> Result<Vec<OAuthUsageRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT account_id, window_type, used, limit_value, reset_unix
             FROM oauth_usage WHERE account_id = ?1 ORDER BY window_type ASC",
        )
        .bind(account_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|row| OAuthUsageRecord {
                account_id: row.get("account_id"),
                window_type: row.get("window_type"),
                used: row.get("used"),
                limit: row.get("limit_value"),
                reset_unix: row.get("reset_unix"),
            })
            .collect())
    }

    pub async fn create_api_key(&self, key: &ApiKeyRecord) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO api_keys(key_id, label, key_hash, tool_choice_policy, created_unix, expires_unix, revoked_unix, last_used_unix)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8);",
        )
        .bind(&key.key_id)
        .bind(&key.label)
        .bind(&key.key_hash)
        .bind(&key.tool_choice_policy)
        .bind(key.created_unix as i64)
        .bind(key.expires_unix.map(|v| v as i64))
        .bind(key.revoked_unix.map(|v| v as i64))
        .bind(key.last_used_unix.map(|v| v as i64))
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    pub async fn list_api_keys(&self) -> Result<Vec<ApiKeyRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT key_id, label, key_hash, tool_choice_policy, created_unix, expires_unix, revoked_unix, last_used_unix
             FROM api_keys ORDER BY created_unix DESC, key_id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|row| ApiKeyRecord {
                key_id: row.get("key_id"),
                label: row.get("label"),
                key_hash: row.get("key_hash"),
                tool_choice_policy: row.get("tool_choice_policy"),
                created_unix: row.get::<i64, _>("created_unix") as u64,
                expires_unix: row.get::<Option<i64>, _>("expires_unix").map(|v| v as u64),
                revoked_unix: row.get::<Option<i64>, _>("revoked_unix").map(|v| v as u64),
                last_used_unix: row
                    .get::<Option<i64>, _>("last_used_unix")
                    .map(|v| v as u64),
            })
            .collect())
    }

    pub async fn count_api_keys(&self) -> Result<u64, StorageError> {
        let row = sqlx::query("SELECT COUNT(*) AS c FROM api_keys")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.get::<i64, _>("c").max(0) as u64)
    }

    pub async fn get_api_key_by_hash(
        &self,
        key_hash: &str,
    ) -> Result<Option<ApiKeyRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT key_id, label, key_hash, tool_choice_policy, created_unix, expires_unix, revoked_unix, last_used_unix
             FROM api_keys
             WHERE key_hash = ?1
             ORDER BY created_unix DESC, key_id ASC
             LIMIT 1",
        )
        .bind(key_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.map(|row| ApiKeyRecord {
            key_id: row.get("key_id"),
            label: row.get("label"),
            key_hash: row.get("key_hash"),
            tool_choice_policy: row.get("tool_choice_policy"),
            created_unix: row.get::<i64, _>("created_unix") as u64,
            expires_unix: row.get::<Option<i64>, _>("expires_unix").map(|v| v as u64),
            revoked_unix: row.get::<Option<i64>, _>("revoked_unix").map(|v| v as u64),
            last_used_unix: row
                .get::<Option<i64>, _>("last_used_unix")
                .map(|v| v as u64),
        }))
    }

    pub async fn revoke_api_key(
        &self,
        key_id: &str,
        revoked_unix: u64,
    ) -> Result<(), StorageError> {
        sqlx::query("UPDATE api_keys SET revoked_unix = ?2 WHERE key_id = ?1")
            .bind(key_id)
            .bind(revoked_unix as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    pub async fn update_api_key_revoked(
        &self,
        key_id: &str,
        revoked_unix: Option<u64>,
    ) -> Result<(), StorageError> {
        sqlx::query("UPDATE api_keys SET revoked_unix = ?2 WHERE key_id = ?1")
            .bind(key_id)
            .bind(revoked_unix.map(|v| v as i64))
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    pub async fn delete_api_key(&self, key_id: &str) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM api_keys WHERE key_id = ?1")
            .bind(key_id)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    pub async fn touch_api_key_last_used(
        &self,
        key_id: &str,
        last_used_unix: u64,
    ) -> Result<(), StorageError> {
        sqlx::query("UPDATE api_keys SET last_used_unix = ?2 WHERE key_id = ?1")
            .bind(key_id)
            .bind(last_used_unix as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    pub async fn update_api_key_tool_choice_policy(
        &self,
        key_id: &str,
        tool_choice_policy: &str,
    ) -> Result<(), StorageError> {
        sqlx::query("UPDATE api_keys SET tool_choice_policy = ?2 WHERE key_id = ?1")
            .bind(key_id)
            .bind(tool_choice_policy)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    pub async fn set_remote_assignment(
        &self,
        remote: &RemoteAssignment,
    ) -> Result<(), StorageError> {
        self.set_app_state(
            "remote_base_url",
            remote.remote_base_url.as_deref().unwrap_or(""),
        )
        .await?;
        self.set_app_state(
            "remote_console_url",
            remote.remote_console_url.as_deref().unwrap_or(""),
        )
        .await?;
        Ok(())
    }

    pub async fn get_remote_assignment(&self) -> Result<RemoteAssignment, StorageError> {
        let base = self.get_app_state("remote_base_url").await?;
        let console = self.get_app_state("remote_console_url").await?;
        Ok(RemoteAssignment {
            remote_base_url: if base.is_empty() { None } else { Some(base) },
            remote_console_url: if console.is_empty() {
                None
            } else {
                Some(console)
            },
        })
    }

    pub async fn set_app_state(&self, key: &str, value: &str) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO app_state(key, value)
             VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value;",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    pub async fn get_app_state(&self, key: &str) -> Result<String, StorageError> {
        let row = sqlx::query("SELECT value FROM app_state WHERE key = ?1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        let Some(row) = row else {
            return Ok(String::new());
        };
        Ok(row.get("value"))
    }

    pub async fn delete_app_state(&self, key: &str) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM app_state WHERE key = ?1")
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    pub async fn get_tunnel_lease(
        &self,
        device_id: &str,
    ) -> Result<Option<TunnelLeaseRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT device_id, endpoint_slug, hostname, tunnel_id, dns_record_id,
                    created_unix, updated_unix, last_provisioned_unix, last_seen_unix
             FROM tunnel_leases WHERE device_id = ?1",
        )
        .bind(device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(TunnelLeaseRecord {
            device_id: row.get("device_id"),
            endpoint_slug: row.get("endpoint_slug"),
            hostname: row.get("hostname"),
            tunnel_id: row.get::<Option<String>, _>("tunnel_id"),
            dns_record_id: row.get::<Option<String>, _>("dns_record_id"),
            created_unix: row.get::<i64, _>("created_unix") as u64,
            updated_unix: row.get::<i64, _>("updated_unix") as u64,
            last_provisioned_unix: row
                .get::<Option<i64>, _>("last_provisioned_unix")
                .map(|v| v as u64),
            last_seen_unix: row
                .get::<Option<i64>, _>("last_seen_unix")
                .map(|v| v as u64),
        }))
    }

    pub async fn get_tunnel_lease_by_slug(
        &self,
        endpoint_slug: &str,
    ) -> Result<Option<TunnelLeaseRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT device_id, endpoint_slug, hostname, tunnel_id, dns_record_id,
                    created_unix, updated_unix, last_provisioned_unix, last_seen_unix
             FROM tunnel_leases WHERE endpoint_slug = ?1 LIMIT 1",
        )
        .bind(endpoint_slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(TunnelLeaseRecord {
            device_id: row.get("device_id"),
            endpoint_slug: row.get("endpoint_slug"),
            hostname: row.get("hostname"),
            tunnel_id: row.get::<Option<String>, _>("tunnel_id"),
            dns_record_id: row.get::<Option<String>, _>("dns_record_id"),
            created_unix: row.get::<i64, _>("created_unix") as u64,
            updated_unix: row.get::<i64, _>("updated_unix") as u64,
            last_provisioned_unix: row
                .get::<Option<i64>, _>("last_provisioned_unix")
                .map(|v| v as u64),
            last_seen_unix: row
                .get::<Option<i64>, _>("last_seen_unix")
                .map(|v| v as u64),
        }))
    }

    pub async fn upsert_tunnel_lease(&self, lease: &TunnelLeaseRecord) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO tunnel_leases(
                device_id, endpoint_slug, hostname, tunnel_id, dns_record_id,
                created_unix, updated_unix, last_provisioned_unix, last_seen_unix
             )
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(device_id) DO UPDATE SET
                endpoint_slug=excluded.endpoint_slug,
                hostname=excluded.hostname,
                tunnel_id=excluded.tunnel_id,
                dns_record_id=excluded.dns_record_id,
                updated_unix=excluded.updated_unix,
                last_provisioned_unix=excluded.last_provisioned_unix,
                last_seen_unix=excluded.last_seen_unix;",
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

    pub async fn touch_tunnel_lease_last_seen(
        &self,
        device_id: &str,
        now_unix: u64,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "UPDATE tunnel_leases SET last_seen_unix = ?2, updated_unix = ?2 WHERE device_id = ?1",
        )
        .bind(device_id)
        .bind(now_unix as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn list_stale_tunnel_leases(
        &self,
        stale_before_unix: u64,
    ) -> Result<Vec<TunnelLeaseRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT device_id, endpoint_slug, hostname, tunnel_id, dns_record_id,
                    created_unix, updated_unix, last_provisioned_unix, last_seen_unix
             FROM tunnel_leases
             WHERE tunnel_id IS NOT NULL AND COALESCE(last_seen_unix, last_provisioned_unix, 0) > 0
               AND COALESCE(last_seen_unix, last_provisioned_unix, 0) < ?1",
        )
        .bind(stale_before_unix as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(TunnelLeaseRecord {
                device_id: row.get("device_id"),
                endpoint_slug: row.get("endpoint_slug"),
                hostname: row.get("hostname"),
                tunnel_id: row.get::<Option<String>, _>("tunnel_id"),
                dns_record_id: row.get::<Option<String>, _>("dns_record_id"),
                created_unix: row.get::<i64, _>("created_unix") as u64,
                updated_unix: row.get::<i64, _>("updated_unix") as u64,
                last_provisioned_unix: row
                    .get::<Option<i64>, _>("last_provisioned_unix")
                    .map(|v| v as u64),
                last_seen_unix: row
                    .get::<Option<i64>, _>("last_seen_unix")
                    .map(|v| v as u64),
            });
        }
        Ok(out)
    }

    pub async fn clear_tunnel_lease_provisioning(
        &self,
        device_id: &str,
        now_unix: u64,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "UPDATE tunnel_leases
             SET tunnel_id = NULL, dns_record_id = dns_record_id,
                 last_provisioned_unix = NULL, updated_unix = ?2
             WHERE device_id = ?1",
        )
        .bind(device_id)
        .bind(now_unix as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn upsert_customer(&self, row: &CustomerRecord) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO customers(customer_id, email, stripe_customer_id, created_unix, updated_unix)
             VALUES(?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(email) DO UPDATE SET
                stripe_customer_id=excluded.stripe_customer_id,
                updated_unix=excluded.updated_unix;",
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

    pub async fn get_customer_by_email(
        &self,
        email: &str,
    ) -> Result<Option<CustomerRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT customer_id, email, stripe_customer_id, created_unix, updated_unix
             FROM customers WHERE email = ?1 LIMIT 1",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(CustomerRecord {
            customer_id: row.get("customer_id"),
            email: row.get("email"),
            stripe_customer_id: row.get("stripe_customer_id"),
            created_unix: row.get::<i64, _>("created_unix") as u64,
            updated_unix: row.get::<i64, _>("updated_unix") as u64,
        }))
    }

    pub async fn get_customer_by_id(
        &self,
        customer_id: &str,
    ) -> Result<Option<CustomerRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT customer_id, email, stripe_customer_id, created_unix, updated_unix
             FROM customers WHERE customer_id = ?1 LIMIT 1",
        )
        .bind(customer_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(CustomerRecord {
            customer_id: row.get("customer_id"),
            email: row.get("email"),
            stripe_customer_id: row.get("stripe_customer_id"),
            created_unix: row.get::<i64, _>("created_unix") as u64,
            updated_unix: row.get::<i64, _>("updated_unix") as u64,
        }))
    }

    pub async fn get_customer_by_stripe_customer_id(
        &self,
        stripe_customer_id: &str,
    ) -> Result<Option<CustomerRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT customer_id, email, stripe_customer_id, created_unix, updated_unix
             FROM customers WHERE stripe_customer_id = ?1 LIMIT 1",
        )
        .bind(stripe_customer_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(CustomerRecord {
            customer_id: row.get("customer_id"),
            email: row.get("email"),
            stripe_customer_id: row.get("stripe_customer_id"),
            created_unix: row.get::<i64, _>("created_unix") as u64,
            updated_unix: row.get::<i64, _>("updated_unix") as u64,
        }))
    }

    pub async fn upsert_subscription(&self, row: &SubscriptionRecord) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO subscriptions(
                subscription_id, customer_id, stripe_subscription_id, status, seat_limit,
                current_period_end_unix, grace_expires_unix, created_unix, updated_unix
             )
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(subscription_id) DO UPDATE SET
                customer_id=excluded.customer_id,
                stripe_subscription_id=excluded.stripe_subscription_id,
                status=excluded.status,
                seat_limit=excluded.seat_limit,
                current_period_end_unix=excluded.current_period_end_unix,
                grace_expires_unix=excluded.grace_expires_unix,
                updated_unix=excluded.updated_unix;",
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

    pub async fn get_subscription_by_id(
        &self,
        subscription_id: &str,
    ) -> Result<Option<SubscriptionRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT subscription_id, customer_id, stripe_subscription_id, status, seat_limit,
                    current_period_end_unix, grace_expires_unix, created_unix, updated_unix
             FROM subscriptions WHERE subscription_id = ?1 LIMIT 1",
        )
        .bind(subscription_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(SubscriptionRecord {
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
        }))
    }

    pub async fn get_subscription_by_stripe_subscription_id(
        &self,
        stripe_subscription_id: &str,
    ) -> Result<Option<SubscriptionRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT subscription_id, customer_id, stripe_subscription_id, status, seat_limit,
                    current_period_end_unix, grace_expires_unix, created_unix, updated_unix
             FROM subscriptions WHERE stripe_subscription_id = ?1 LIMIT 1",
        )
        .bind(stripe_subscription_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(SubscriptionRecord {
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
        }))
    }

    pub async fn list_subscriptions(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SubscriptionRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT subscription_id, customer_id, stripe_subscription_id, status, seat_limit,
                    current_period_end_unix, grace_expires_unix, created_unix, updated_unix
             FROM subscriptions
             ORDER BY updated_unix DESC, subscription_id DESC
             LIMIT ?1 OFFSET ?2",
        )
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(|row| SubscriptionRecord {
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
            })
            .collect())
    }

    pub async fn list_subscriptions_for_customer(
        &self,
        customer_id: &str,
    ) -> Result<Vec<SubscriptionRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT subscription_id, customer_id, stripe_subscription_id, status, seat_limit,
                    current_period_end_unix, grace_expires_unix, created_unix, updated_unix
             FROM subscriptions
             WHERE customer_id = ?1
             ORDER BY updated_unix DESC, subscription_id DESC",
        )
        .bind(customer_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(|row| SubscriptionRecord {
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
            })
            .collect())
    }

    pub async fn upsert_license(&self, row: &LicenseRecord) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO licenses(
                license_id, customer_id, subscription_id, license_key, plan_name, status,
                seat_limit, source, grace_expires_unix, created_unix, updated_unix
             )
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(license_id) DO UPDATE SET
                customer_id=excluded.customer_id,
                subscription_id=excluded.subscription_id,
                license_key=excluded.license_key,
                plan_name=excluded.plan_name,
                status=excluded.status,
                seat_limit=excluded.seat_limit,
                source=excluded.source,
                grace_expires_unix=excluded.grace_expires_unix,
                updated_unix=excluded.updated_unix;",
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

    pub async fn get_license_by_key(
        &self,
        license_key: &str,
    ) -> Result<Option<LicenseRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT license_id, customer_id, subscription_id, license_key, plan_name, status,
                    seat_limit, source, grace_expires_unix, created_unix, updated_unix
             FROM licenses WHERE license_key = ?1",
        )
        .bind(license_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(LicenseRecord {
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
        }))
    }

    pub async fn get_license_by_subscription_id(
        &self,
        subscription_id: &str,
    ) -> Result<Option<LicenseRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT license_id, customer_id, subscription_id, license_key, plan_name, status,
                    seat_limit, source, grace_expires_unix, created_unix, updated_unix
             FROM licenses WHERE subscription_id = ?1 LIMIT 1",
        )
        .bind(subscription_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(LicenseRecord {
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
        }))
    }

    pub async fn get_license_by_id(
        &self,
        license_id: &str,
    ) -> Result<Option<LicenseRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT license_id, customer_id, subscription_id, license_key, plan_name, status,
                    seat_limit, source, grace_expires_unix, created_unix, updated_unix
             FROM licenses WHERE license_id = ?1 LIMIT 1",
        )
        .bind(license_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(LicenseRecord {
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
        }))
    }

    pub async fn list_licenses(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<LicenseRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT license_id, customer_id, subscription_id, license_key, plan_name, status,
                    seat_limit, source, grace_expires_unix, created_unix, updated_unix
             FROM licenses
             ORDER BY updated_unix DESC, license_id DESC
             LIMIT ?1 OFFSET ?2",
        )
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(|row| LicenseRecord {
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
            })
            .collect())
    }

    pub async fn list_licenses_for_customer(
        &self,
        customer_id: &str,
    ) -> Result<Vec<LicenseRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT license_id, customer_id, subscription_id, license_key, plan_name, status,
                    seat_limit, source, grace_expires_unix, created_unix, updated_unix
             FROM licenses
             WHERE customer_id = ?1
             ORDER BY updated_unix DESC, license_id DESC",
        )
        .bind(customer_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(|row| LicenseRecord {
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
            })
            .collect())
    }

    pub async fn upsert_device_binding(
        &self,
        row: &DeviceBindingRecord,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO device_bindings(
                binding_id, license_id, seat_index, device_id, device_name, os, app_version, label,
                tunnel_id, tunnel_hostname, country_code, last_ip_hash, status, first_seen_unix, last_seen_unix, retired_unix
             )
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
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
                retired_unix=excluded.retired_unix;",
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

    pub async fn list_active_bindings_for_license(
        &self,
        license_id: &str,
    ) -> Result<Vec<DeviceBindingRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT binding_id, license_id, seat_index, device_id, device_name, os, app_version,
                    label, tunnel_id, tunnel_hostname, country_code, last_ip_hash, status, first_seen_unix, last_seen_unix, retired_unix
             FROM device_bindings
             WHERE license_id = ?1 AND status = 'active'
             ORDER BY seat_index ASC",
        )
        .bind(license_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(|row| DeviceBindingRecord {
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
            })
            .collect())
    }

    pub async fn list_bindings_for_license(
        &self,
        license_id: &str,
    ) -> Result<Vec<DeviceBindingRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT binding_id, license_id, seat_index, device_id, device_name, os, app_version,
                    label, tunnel_id, tunnel_hostname, country_code, last_ip_hash, status, first_seen_unix, last_seen_unix, retired_unix
             FROM device_bindings
             WHERE license_id = ?1
             ORDER BY seat_index ASC, first_seen_unix ASC",
        )
        .bind(license_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(|row| DeviceBindingRecord {
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
            })
            .collect())
    }

    pub async fn retire_binding(
        &self,
        binding_id: &str,
        retired_unix: u64,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "UPDATE device_bindings
             SET status = 'retired', retired_unix = ?2, last_seen_unix = ?2
             WHERE binding_id = ?1 AND status = 'active'",
        )
        .bind(binding_id)
        .bind(retired_unix as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn get_binding_by_id(
        &self,
        binding_id: &str,
    ) -> Result<Option<DeviceBindingRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT binding_id, license_id, seat_index, device_id, device_name, os, app_version,
                    label, tunnel_id, tunnel_hostname, country_code, last_ip_hash, status, first_seen_unix, last_seen_unix, retired_unix
             FROM device_bindings
             WHERE binding_id = ?1
             LIMIT 1",
        )
        .bind(binding_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(DeviceBindingRecord {
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
        }))
    }

    pub async fn list_bindings(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<DeviceBindingRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT binding_id, license_id, seat_index, device_id, device_name, os, app_version,
                    label, tunnel_id, tunnel_hostname, country_code, last_ip_hash, status, first_seen_unix, last_seen_unix, retired_unix
             FROM device_bindings
             ORDER BY last_seen_unix DESC, binding_id DESC
             LIMIT ?1 OFFSET ?2",
        )
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(|row| DeviceBindingRecord {
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
            })
            .collect())
    }

    pub async fn upsert_otp_challenge(&self, row: &OtpChallengeRecord) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO otp_challenges(
                challenge_id, email, code_hash, purpose, target_license_id,
                expires_unix, consumed_unix, created_unix
             )
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(challenge_id) DO UPDATE SET
                email=excluded.email,
                code_hash=excluded.code_hash,
                purpose=excluded.purpose,
                target_license_id=excluded.target_license_id,
                expires_unix=excluded.expires_unix,
                consumed_unix=excluded.consumed_unix;",
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

    pub async fn get_otp_challenge(
        &self,
        challenge_id: &str,
    ) -> Result<Option<OtpChallengeRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT challenge_id, email, code_hash, purpose, target_license_id,
                    expires_unix, consumed_unix, created_unix
             FROM otp_challenges WHERE challenge_id = ?1",
        )
        .bind(challenge_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(OtpChallengeRecord {
            challenge_id: row.get("challenge_id"),
            email: row.get("email"),
            code_hash: row.get("code_hash"),
            purpose: row.get("purpose"),
            target_license_id: row.get("target_license_id"),
            expires_unix: row.get::<i64, _>("expires_unix") as u64,
            consumed_unix: row.get::<Option<i64>, _>("consumed_unix").map(|v| v as u64),
            created_unix: row.get::<i64, _>("created_unix") as u64,
        }))
    }

    pub async fn consume_otp_challenge(
        &self,
        challenge_id: &str,
        consumed_unix: u64,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "UPDATE otp_challenges
             SET consumed_unix = ?2
             WHERE challenge_id = ?1 AND consumed_unix IS NULL",
        )
        .bind(challenge_id)
        .bind(consumed_unix as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn get_active_binding_by_license_device(
        &self,
        license_id: &str,
        device_id: &str,
    ) -> Result<Option<DeviceBindingRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT binding_id, license_id, seat_index, device_id, device_name, os, app_version,
                    label, tunnel_id, tunnel_hostname, country_code, last_ip_hash, status, first_seen_unix, last_seen_unix, retired_unix
             FROM device_bindings
             WHERE license_id = ?1 AND device_id = ?2 AND status = 'active'
             LIMIT 1",
        )
        .bind(license_id)
        .bind(device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(DeviceBindingRecord {
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
        }))
    }

    pub async fn touch_active_binding_last_seen(
        &self,
        license_id: &str,
        device_id: &str,
        now_unix: u64,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "UPDATE device_bindings
             SET last_seen_unix = ?3
             WHERE license_id = ?1 AND device_id = ?2 AND status = 'active'",
        )
        .bind(license_id)
        .bind(device_id)
        .bind(now_unix as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn count_licenses(&self) -> Result<i64, StorageError> {
        let row = sqlx::query("SELECT COUNT(*) AS c FROM licenses")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.get("c"))
    }

    pub async fn count_active_device_bindings(&self) -> Result<i64, StorageError> {
        let row = sqlx::query("SELECT COUNT(*) AS c FROM device_bindings WHERE status = 'active'")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.get("c"))
    }

    pub async fn insert_billing_event_if_new(
        &self,
        row: &BillingEventAuditRecord,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "INSERT OR IGNORE INTO billing_events_audit(
                event_id, event_type, created_unix, payload_json, received_unix,
                processed_unix, process_status, error
            ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
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

    pub async fn set_billing_event_result(
        &self,
        event_id: &str,
        processed_unix: u64,
        process_status: &str,
        error: Option<&str>,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "UPDATE billing_events_audit
             SET processed_unix = ?2, process_status = ?3, error = ?4
             WHERE event_id = ?1",
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

    pub async fn get_billing_event(
        &self,
        event_id: &str,
    ) -> Result<Option<BillingEventAuditRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT event_id, event_type, created_unix, payload_json, received_unix,
                    processed_unix, process_status, error
             FROM billing_events_audit WHERE event_id = ?1 LIMIT 1",
        )
        .bind(event_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(BillingEventAuditRecord {
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
        }))
    }

    pub async fn list_billing_events_by_status(
        &self,
        process_status: &str,
        limit: u32,
    ) -> Result<Vec<BillingEventAuditRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT event_id, event_type, created_unix, payload_json, received_unix,
                    processed_unix, process_status, error
             FROM billing_events_audit
             WHERE process_status = ?1
             ORDER BY received_unix ASC
             LIMIT ?2",
        )
        .bind(process_status)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(|row| BillingEventAuditRecord {
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
            })
            .collect())
    }

    pub async fn insert_billing_reminder_if_absent(
        &self,
        row: &BillingReminderRecord,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "INSERT OR IGNORE INTO billing_reminders(
                reminder_id, subscription_id, license_id, customer_email, reason,
                due_unix, sent_unix, revoked_unix, attempt_count, last_error,
                created_unix, updated_unix, idempotency_key
            ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
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

    pub async fn list_due_unsent_billing_reminders(
        &self,
        now_unix: u64,
        limit: u32,
    ) -> Result<Vec<BillingReminderRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT reminder_id, subscription_id, license_id, customer_email, reason,
                    due_unix, sent_unix, revoked_unix, attempt_count, last_error,
                    created_unix, updated_unix, idempotency_key
             FROM billing_reminders
             WHERE due_unix <= ?1
               AND sent_unix IS NULL
               AND revoked_unix IS NULL
               AND attempt_count < 20
             ORDER BY due_unix ASC
             LIMIT ?2",
        )
        .bind(now_unix as i64)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(|row| BillingReminderRecord {
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
            })
            .collect())
    }

    pub async fn mark_billing_reminder_sent(
        &self,
        reminder_id: &str,
        sent_unix: u64,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "UPDATE billing_reminders
             SET sent_unix = ?2, updated_unix = ?2
             WHERE reminder_id = ?1",
        )
        .bind(reminder_id)
        .bind(sent_unix as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn mark_billing_reminder_failed(
        &self,
        reminder_id: &str,
        failed_unix: u64,
        error: Option<&str>,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "UPDATE billing_reminders
             SET attempt_count = attempt_count + 1,
                 last_error = ?3,
                 updated_unix = ?2
             WHERE reminder_id = ?1
               AND sent_unix IS NULL
               AND revoked_unix IS NULL",
        )
        .bind(reminder_id)
        .bind(failed_unix as i64)
        .bind(error)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn revoke_pending_billing_reminders(
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
             WHERE subscription_id = ?1
               AND sent_unix IS NULL
               AND revoked_unix IS NULL",
        )
        .bind(subscription_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        let mut changed: u64 = 0;
        for row in rows {
            let reminder_id: String = row.get("reminder_id");
            let reason: String = row.get("reason");
            if !reason_prefixes.iter().any(|p| reason.starts_with(p)) {
                continue;
            }
            let res = sqlx::query(
                "UPDATE billing_reminders
                 SET revoked_unix = ?2, updated_unix = ?2
                 WHERE reminder_id = ?1
                   AND sent_unix IS NULL
                   AND revoked_unix IS NULL",
            )
            .bind(&reminder_id)
            .bind(revoked_unix as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
            changed = changed.saturating_add(res.rows_affected());
        }
        Ok(changed)
    }

    pub async fn count_due_billing_reminders(&self, now_unix: u64) -> Result<i64, StorageError> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS c
             FROM billing_reminders
             WHERE due_unix <= ?1 AND sent_unix IS NULL AND revoked_unix IS NULL",
        )
        .bind(now_unix as i64)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.get("c"))
    }

    pub async fn count_sent_billing_reminders_since(
        &self,
        since_unix: u64,
    ) -> Result<i64, StorageError> {
        let row = sqlx::query("SELECT COUNT(*) AS c FROM billing_reminders WHERE sent_unix >= ?1")
            .bind(since_unix as i64)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.get("c"))
    }

    pub async fn count_failed_billing_reminders_since(
        &self,
        since_unix: u64,
    ) -> Result<i64, StorageError> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS c
             FROM billing_reminders
             WHERE sent_unix IS NULL
               AND revoked_unix IS NULL
               AND attempt_count > 0
               AND updated_unix >= ?1",
        )
        .bind(since_unix as i64)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(row.get("c"))
    }

    pub async fn insert_billing_revenue_event_if_new(
        &self,
        row: &BillingRevenueEventRecord,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "INSERT OR IGNORE INTO billing_revenue_events(
                event_id, stripe_subscription_id, customer_id, amount_cents, currency, kind, occurred_unix
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
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

    pub async fn list_billing_revenue_events_between(
        &self,
        start_unix: u64,
        end_unix: u64,
    ) -> Result<Vec<BillingRevenueEventRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT event_id, stripe_subscription_id, customer_id, amount_cents, currency, kind, occurred_unix
             FROM billing_revenue_events
             WHERE occurred_unix >= ?1 AND occurred_unix <= ?2
             ORDER BY occurred_unix ASC, event_id ASC",
        )
        .bind(start_unix as i64)
        .bind(end_unix as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(|row| BillingRevenueEventRecord {
                event_id: row.get("event_id"),
                stripe_subscription_id: row.get("stripe_subscription_id"),
                customer_id: row.get("customer_id"),
                amount_cents: row.get("amount_cents"),
                currency: row.get("currency"),
                kind: row.get("kind"),
                occurred_unix: row.get::<i64, _>("occurred_unix") as u64,
            })
            .collect())
    }

    pub async fn insert_otp_delivery_event(
        &self,
        row: &OtpDeliveryEventRecord,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO otp_delivery_events(
                event_id, challenge_id, email, provider, status, error, created_unix
            ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
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

    pub async fn otp_rate_limit_attempt(
        &self,
        bucket_key: &str,
        now_unix: u64,
        window_secs: u64,
    ) -> Result<u64, StorageError> {
        let row = sqlx::query(
            "SELECT window_start_unix, attempt_count
             FROM otp_rate_limits WHERE bucket_key = ?1 LIMIT 1",
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
             VALUES(?1, ?2, ?3, ?4)
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

    pub async fn otp_verify_attempt(
        &self,
        challenge_id: &str,
        now_unix: u64,
    ) -> Result<u64, StorageError> {
        let row = sqlx::query(
            "SELECT attempt_count FROM otp_verify_attempts
             WHERE challenge_id = ?1 LIMIT 1",
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
             VALUES(?1, ?2, ?3)
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

    pub async fn clear_otp_verify_attempts(&self, challenge_id: &str) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM otp_verify_attempts WHERE challenge_id = ?1")
            .bind(challenge_id)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    pub async fn create_admin_session(&self, row: &AdminSessionRecord) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO admin_sessions(
                session_id, issued_unix, expires_unix, revoked_unix, source_ip, user_agent_hash, created_by
            ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
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

    pub async fn get_admin_session(
        &self,
        session_id: &str,
    ) -> Result<Option<AdminSessionRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT session_id, issued_unix, expires_unix, revoked_unix, source_ip, user_agent_hash, created_by
             FROM admin_sessions WHERE session_id = ?1 LIMIT 1",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(AdminSessionRecord {
            session_id: row.get("session_id"),
            issued_unix: row.get::<i64, _>("issued_unix") as u64,
            expires_unix: row.get::<i64, _>("expires_unix") as u64,
            revoked_unix: row.get::<Option<i64>, _>("revoked_unix").map(|v| v as u64),
            source_ip: row.get("source_ip"),
            user_agent_hash: row.get("user_agent_hash"),
            created_by: row.get("created_by"),
        }))
    }

    pub async fn revoke_admin_session(
        &self,
        session_id: &str,
        revoked_unix: u64,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "UPDATE admin_sessions
             SET revoked_unix = ?2
             WHERE session_id = ?1 AND revoked_unix IS NULL",
        )
        .bind(session_id)
        .bind(revoked_unix as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn prune_expired_admin_sessions(&self, now_unix: u64) -> Result<u64, StorageError> {
        let res = sqlx::query("DELETE FROM admin_sessions WHERE expires_unix < ?1")
            .bind(now_unix as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected())
    }

    pub async fn create_app_oauth_code(
        &self,
        row: &AppOAuthCodeRecord,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO app_oauth_codes(
                code_id, customer_id, license_id, state, code_challenge, device_id,
                expires_unix, consumed_unix, created_unix
            ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
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

    pub async fn get_app_oauth_code(
        &self,
        code_id: &str,
    ) -> Result<Option<AppOAuthCodeRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT code_id, customer_id, license_id, state, code_challenge, device_id,
                    expires_unix, consumed_unix, created_unix
             FROM app_oauth_codes WHERE code_id = ?1 LIMIT 1",
        )
        .bind(code_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(AppOAuthCodeRecord {
            code_id: row.get("code_id"),
            customer_id: row.get("customer_id"),
            license_id: row.get("license_id"),
            state: row.get("state"),
            code_challenge: row.get("code_challenge"),
            device_id: row.get("device_id"),
            expires_unix: row.get::<i64, _>("expires_unix") as u64,
            consumed_unix: row.get::<Option<i64>, _>("consumed_unix").map(|v| v as u64),
            created_unix: row.get::<i64, _>("created_unix") as u64,
        }))
    }

    pub async fn consume_app_oauth_code(
        &self,
        code_id: &str,
        consumed_unix: u64,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "UPDATE app_oauth_codes
             SET consumed_unix = ?2
             WHERE code_id = ?1 AND consumed_unix IS NULL",
        )
        .bind(code_id)
        .bind(consumed_unix as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn create_app_session(&self, row: &AppSessionRecord) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO app_sessions(
                session_id, customer_id, device_id, license_id, seat_index, issued_unix, expires_unix, revoked_unix
            ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
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

    pub async fn get_app_session(
        &self,
        session_id: &str,
    ) -> Result<Option<AppSessionRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT session_id, customer_id, device_id, license_id, seat_index, issued_unix, expires_unix, revoked_unix
             FROM app_sessions WHERE session_id = ?1 LIMIT 1",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(AppSessionRecord {
            session_id: row.get("session_id"),
            customer_id: row.get("customer_id"),
            device_id: row.get("device_id"),
            license_id: row.get("license_id"),
            seat_index: row.get("seat_index"),
            issued_unix: row.get::<i64, _>("issued_unix") as u64,
            expires_unix: row.get::<i64, _>("expires_unix") as u64,
            revoked_unix: row.get::<Option<i64>, _>("revoked_unix").map(|v| v as u64),
        }))
    }

    pub async fn revoke_app_session(
        &self,
        session_id: &str,
        revoked_unix: u64,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query(
            "UPDATE app_sessions
             SET revoked_unix = ?2
             WHERE session_id = ?1 AND revoked_unix IS NULL",
        )
        .bind(session_id)
        .bind(revoked_unix as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn prune_expired_app_sessions(&self, now_unix: u64) -> Result<u64, StorageError> {
        let res = sqlx::query("DELETE FROM app_sessions WHERE expires_unix < ?1")
            .bind(now_unix as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected())
    }

    pub async fn insert_gift_entitlement(
        &self,
        row: &GiftEntitlementRecord,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO gift_entitlements(
                gift_id, license_id, recipient_email, seat_count, starts_unix, expires_unix, created_unix,
                created_by_admin_session, status
            ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
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

    pub async fn list_gift_entitlements_for_license(
        &self,
        license_id: &str,
    ) -> Result<Vec<GiftEntitlementRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT gift_id, license_id, recipient_email, seat_count, starts_unix, expires_unix, created_unix,
                    created_by_admin_session, status
             FROM gift_entitlements
             WHERE license_id = ?1
             ORDER BY created_unix DESC",
        )
        .bind(license_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(|row| GiftEntitlementRecord {
                gift_id: row.get("gift_id"),
                license_id: row.get("license_id"),
                recipient_email: row.get("recipient_email"),
                seat_count: row.get("seat_count"),
                starts_unix: row.get::<i64, _>("starts_unix") as u64,
                expires_unix: row.get::<i64, _>("expires_unix") as u64,
                created_unix: row.get::<i64, _>("created_unix") as u64,
                created_by_admin_session: row.get("created_by_admin_session"),
                status: row.get("status"),
            })
            .collect())
    }

    pub async fn set_gift_entitlement_status(
        &self,
        gift_id: &str,
        status: &str,
    ) -> Result<bool, StorageError> {
        let res = sqlx::query("UPDATE gift_entitlements SET status = ?2 WHERE gift_id = ?1")
            .bind(gift_id)
            .bind(status)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn insert_audit_log(&self, row: &AuditLogRecord) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO audit_logs(
                audit_id, actor_type, actor_id, action, target_type, target_id, payload_json,
                created_unix, ip, outcome
            ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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

    pub async fn list_audit_logs(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<AuditLogRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT audit_id, actor_type, actor_id, action, target_type, target_id, payload_json,
                    created_unix, ip, outcome
             FROM audit_logs
             ORDER BY created_unix DESC
             LIMIT ?1 OFFSET ?2",
        )
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(|row| AuditLogRecord {
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
            })
            .collect())
    }
}
