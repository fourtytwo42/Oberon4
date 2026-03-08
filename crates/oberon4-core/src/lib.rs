pub mod config;
pub mod domain;
pub mod error;
pub mod ids;
pub mod ipc;
pub mod mock;
pub mod persistence;
pub mod plugin_runtime;
pub mod result;
pub mod service;
pub mod state;

pub use config::{AppConfig, EndpointConfig, RuntimeConfig};
pub use domain::{
    AccountState, ChatMessage, ChatProviderKind, ChatRole, ChatToolCall, ChatToolCallDelta,
    DaemonStatus, OAuthProviderKind, SavedSession, SessionId, SessionSummary, StoredAccount,
};
pub use error::OberonError;
pub use ids::RequestId;
pub use ipc::{DaemonCommand, DaemonEvent};
pub use mock::{FakeChatProvider, FakeOAuthProvider, InMemoryConfigStore, InMemorySessionStore};
pub use persistence::{FileConfigStore, FileSessionStore, default_data_dir};
pub use plugin_runtime::{
    ApprovalDecisionPayload, ApprovalExpectation, ApprovalRequestPayload, ApprovalResourceLimits,
    ArtifactCapabilityManifest, ArtifactHandle, ArtifactReference, ArtifactRetention,
    BuildMetadata, CapabilityGrant, CapabilityGrantKind, CapabilityManifest, CommandDescription,
    CommandPlanPreview, CommandSchemaReference, EffectiveCapabilities, ErrorStatus, EstimatedCost,
    HandleOperation, HealthStatus, HostToolRuntime, InvocationContext, InvokeRequest,
    InvokeResponse, InvokeResult, LifecycleActions, LifecycleHooks, ManifestValidationIssue,
    ManifestValidationResult, NetworkPolicy, OperatingMode, PlanRequest, PlanResult, PlanStatus,
    PluginCommandManifest, PluginDescription, PluginExample, PluginManifest, PluginRegistry,
    PolicyInspectRequest, PolicyInspectResponse, PolicyResolution, PolicyResolver,
    PreconditionModel, PromptingManifest, ProvenanceManifest, ResourceBudgetRequest,
    ResourceLimits, SearchMatch, SearchRequest, SearchResponse, SecretClass, SecretHandle,
    SecurityLevel, ServiceHandle, SideEffectClass, SignatureMetadata, StorageCapabilityManifest,
    ToolDescribeRequest, ToolDescribeRequestEnvelope, ToolDescribeResponse,
    ToolDescribeResponseEnvelope, ToolError, ToolGuidance, ToolInvokeRequestEnvelope,
    ToolInvokeResponseEnvelope, ToolPlanRequestEnvelope, ToolPlanResponseEnvelope, ToolResult,
    ToolSearchRequestEnvelope, ToolSearchResponseEnvelope, ToolTier, WorkspaceCapabilityManifest,
    WorkspaceEntry, WorkspaceHandle, WorkspaceImpact, WorkspacePathKind,
    WorkspacePreconditionStatus, WorkspaceRevisionRef, WorkspaceWriteResult,
};
pub use result::ServiceEnvelope;
pub use service::{
    ChatProvider, ChatStreamEvent, ConfigStore, OAuthCallbackPayload, OAuthProvider, OAuthStart,
    PendingOAuthFlow, SessionStore, TokenBundle,
};
pub use state::{
    FileSecretStore, InMemorySecretStore, InMemoryStateStore, SecretStore, StateStore,
};
