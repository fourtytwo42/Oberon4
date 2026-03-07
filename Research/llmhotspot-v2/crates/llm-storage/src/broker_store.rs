use async_trait::async_trait;

use crate::{
    AdminSessionRecord, AppOAuthCodeRecord, AppSessionRecord, AuditLogRecord,
    BillingEventAuditRecord, BillingReminderRecord, BillingRevenueEventRecord, CustomerRecord,
    DeviceBindingRecord, GiftEntitlementRecord, LicenseRecord, OtpChallengeRecord,
    OtpDeliveryEventRecord, StorageError, SubscriptionRecord, TunnelLeaseRecord,
};

#[async_trait]
pub trait BrokerStore: Send + Sync {
    fn boxed_clone(&self) -> Box<dyn BrokerStore>;

    async fn get_tunnel_lease(
        &self,
        device_id: &str,
    ) -> Result<Option<TunnelLeaseRecord>, StorageError>;
    async fn get_tunnel_lease_by_slug(
        &self,
        endpoint_slug: &str,
    ) -> Result<Option<TunnelLeaseRecord>, StorageError>;
    async fn upsert_tunnel_lease(&self, lease: &TunnelLeaseRecord) -> Result<(), StorageError>;
    async fn touch_tunnel_lease_last_seen(
        &self,
        device_id: &str,
        now_unix: u64,
    ) -> Result<bool, StorageError>;
    async fn list_stale_tunnel_leases(
        &self,
        stale_before_unix: u64,
    ) -> Result<Vec<TunnelLeaseRecord>, StorageError>;
    async fn clear_tunnel_lease_provisioning(
        &self,
        device_id: &str,
        now_unix: u64,
    ) -> Result<bool, StorageError>;

    async fn upsert_customer(&self, row: &CustomerRecord) -> Result<(), StorageError>;
    async fn get_customer_by_email(
        &self,
        email: &str,
    ) -> Result<Option<CustomerRecord>, StorageError>;
    async fn get_customer_by_id(
        &self,
        customer_id: &str,
    ) -> Result<Option<CustomerRecord>, StorageError>;
    async fn get_customer_by_stripe_customer_id(
        &self,
        stripe_customer_id: &str,
    ) -> Result<Option<CustomerRecord>, StorageError>;

    async fn upsert_subscription(&self, row: &SubscriptionRecord) -> Result<(), StorageError>;
    async fn get_subscription_by_id(
        &self,
        subscription_id: &str,
    ) -> Result<Option<SubscriptionRecord>, StorageError>;
    async fn get_subscription_by_stripe_subscription_id(
        &self,
        stripe_subscription_id: &str,
    ) -> Result<Option<SubscriptionRecord>, StorageError>;
    async fn list_subscriptions(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SubscriptionRecord>, StorageError>;
    async fn list_subscriptions_for_customer(
        &self,
        customer_id: &str,
    ) -> Result<Vec<SubscriptionRecord>, StorageError>;

    async fn upsert_license(&self, row: &LicenseRecord) -> Result<(), StorageError>;
    async fn get_license_by_key(
        &self,
        license_key: &str,
    ) -> Result<Option<LicenseRecord>, StorageError>;
    async fn get_license_by_subscription_id(
        &self,
        subscription_id: &str,
    ) -> Result<Option<LicenseRecord>, StorageError>;
    async fn get_license_by_id(
        &self,
        license_id: &str,
    ) -> Result<Option<LicenseRecord>, StorageError>;
    async fn list_licenses(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<LicenseRecord>, StorageError>;
    async fn list_licenses_for_customer(
        &self,
        customer_id: &str,
    ) -> Result<Vec<LicenseRecord>, StorageError>;

    async fn upsert_device_binding(&self, row: &DeviceBindingRecord) -> Result<(), StorageError>;
    async fn list_active_bindings_for_license(
        &self,
        license_id: &str,
    ) -> Result<Vec<DeviceBindingRecord>, StorageError>;
    async fn list_bindings_for_license(
        &self,
        license_id: &str,
    ) -> Result<Vec<DeviceBindingRecord>, StorageError>;
    async fn get_active_binding_by_license_device(
        &self,
        license_id: &str,
        device_id: &str,
    ) -> Result<Option<DeviceBindingRecord>, StorageError>;
    async fn get_binding_by_id(
        &self,
        binding_id: &str,
    ) -> Result<Option<DeviceBindingRecord>, StorageError>;
    async fn list_bindings(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<DeviceBindingRecord>, StorageError>;
    async fn retire_binding(
        &self,
        binding_id: &str,
        retired_unix: u64,
    ) -> Result<bool, StorageError>;

    async fn upsert_otp_challenge(&self, row: &OtpChallengeRecord) -> Result<(), StorageError>;
    async fn get_otp_challenge(
        &self,
        challenge_id: &str,
    ) -> Result<Option<OtpChallengeRecord>, StorageError>;
    async fn consume_otp_challenge(
        &self,
        challenge_id: &str,
        consumed_unix: u64,
    ) -> Result<bool, StorageError>;
    async fn otp_rate_limit_attempt(
        &self,
        bucket_key: &str,
        now_unix: u64,
        window_secs: u64,
    ) -> Result<u64, StorageError>;
    async fn otp_verify_attempt(
        &self,
        challenge_id: &str,
        now_unix: u64,
    ) -> Result<u64, StorageError>;
    async fn clear_otp_verify_attempts(&self, challenge_id: &str) -> Result<(), StorageError>;
    async fn insert_otp_delivery_event(
        &self,
        row: &OtpDeliveryEventRecord,
    ) -> Result<(), StorageError>;

    async fn count_licenses(&self) -> Result<i64, StorageError>;
    async fn count_active_device_bindings(&self) -> Result<i64, StorageError>;

    async fn insert_billing_event_if_new(
        &self,
        row: &BillingEventAuditRecord,
    ) -> Result<bool, StorageError>;
    async fn set_billing_event_result(
        &self,
        event_id: &str,
        processed_unix: u64,
        process_status: &str,
        error: Option<&str>,
    ) -> Result<bool, StorageError>;
    async fn get_billing_event(
        &self,
        event_id: &str,
    ) -> Result<Option<BillingEventAuditRecord>, StorageError>;
    async fn list_billing_events_by_status(
        &self,
        process_status: &str,
        limit: u32,
    ) -> Result<Vec<BillingEventAuditRecord>, StorageError>;
    async fn insert_billing_reminder_if_absent(
        &self,
        row: &BillingReminderRecord,
    ) -> Result<bool, StorageError>;
    async fn list_due_unsent_billing_reminders(
        &self,
        now_unix: u64,
        limit: u32,
    ) -> Result<Vec<BillingReminderRecord>, StorageError>;
    async fn mark_billing_reminder_sent(
        &self,
        reminder_id: &str,
        sent_unix: u64,
    ) -> Result<bool, StorageError>;
    async fn mark_billing_reminder_failed(
        &self,
        reminder_id: &str,
        failed_unix: u64,
        error: Option<&str>,
    ) -> Result<bool, StorageError>;
    async fn revoke_pending_billing_reminders(
        &self,
        subscription_id: &str,
        reason_prefixes: &[&str],
        revoked_unix: u64,
    ) -> Result<u64, StorageError>;
    async fn count_due_billing_reminders(&self, now_unix: u64) -> Result<i64, StorageError>;
    async fn count_sent_billing_reminders_since(
        &self,
        since_unix: u64,
    ) -> Result<i64, StorageError>;
    async fn count_failed_billing_reminders_since(
        &self,
        since_unix: u64,
    ) -> Result<i64, StorageError>;
    async fn insert_billing_revenue_event_if_new(
        &self,
        row: &BillingRevenueEventRecord,
    ) -> Result<bool, StorageError>;
    async fn list_billing_revenue_events_between(
        &self,
        start_unix: u64,
        end_unix: u64,
    ) -> Result<Vec<BillingRevenueEventRecord>, StorageError>;

    async fn create_admin_session(&self, row: &AdminSessionRecord) -> Result<(), StorageError>;
    async fn get_admin_session(
        &self,
        session_id: &str,
    ) -> Result<Option<AdminSessionRecord>, StorageError>;
    async fn revoke_admin_session(
        &self,
        session_id: &str,
        revoked_unix: u64,
    ) -> Result<bool, StorageError>;
    async fn prune_expired_admin_sessions(&self, now_unix: u64) -> Result<u64, StorageError>;

    async fn create_app_oauth_code(&self, row: &AppOAuthCodeRecord) -> Result<(), StorageError>;
    async fn get_app_oauth_code(
        &self,
        code_id: &str,
    ) -> Result<Option<AppOAuthCodeRecord>, StorageError>;
    async fn consume_app_oauth_code(
        &self,
        code_id: &str,
        consumed_unix: u64,
    ) -> Result<bool, StorageError>;
    async fn create_app_session(&self, row: &AppSessionRecord) -> Result<(), StorageError>;
    async fn get_app_session(
        &self,
        session_id: &str,
    ) -> Result<Option<AppSessionRecord>, StorageError>;
    async fn revoke_app_session(
        &self,
        session_id: &str,
        revoked_unix: u64,
    ) -> Result<bool, StorageError>;
    async fn prune_expired_app_sessions(&self, now_unix: u64) -> Result<u64, StorageError>;

    async fn insert_gift_entitlement(
        &self,
        row: &GiftEntitlementRecord,
    ) -> Result<(), StorageError>;

    async fn insert_audit_log(&self, row: &AuditLogRecord) -> Result<(), StorageError>;
    async fn list_audit_logs(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<AuditLogRecord>, StorageError>;
}

impl Clone for Box<dyn BrokerStore> {
    fn clone(&self) -> Self {
        self.boxed_clone()
    }
}
