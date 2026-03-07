pub mod app_data;
pub mod broker_store;
pub mod postgres_store;
pub mod secrets;
pub mod sqlite_store;
pub mod state_store;
pub mod token_store;

pub use app_data::{
    AdminSessionRecord, ApiKeyRecord, AppOAuthCodeRecord, AppSessionRecord, AuditLogRecord,
    BillingEventAuditRecord, BillingReminderRecord, BillingRevenueEventRecord, CustomerRecord,
    DeviceBindingRecord, GiftEntitlementRecord, LicenseRecord, OAuthAccountRecord,
    OAuthUsageRecord, OtpChallengeRecord, OtpDeliveryEventRecord, RemoteAssignment,
    SubscriptionRecord, TunnelLeaseRecord,
};
pub use broker_store::BrokerStore;
pub use postgres_store::PostgresStore;
pub use secrets::{InMemorySecretStore, SecretStore};
pub use sqlite_store::SqliteStore;
pub use state_store::{InMemoryStateStore, StateStore};
pub use token_store::{InMemoryTokenStore, KeyringTokenStore, StoredTokenRecord, TokenStore};

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("not found")]
    NotFound,
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("keyring error: {0}")]
    Keyring(String),
    #[error("database error: {0}")]
    Database(String),
    #[error("invalid key")]
    InvalidKey,
}
