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
    AllowlistStatus, ApprovalDecisionPayload, ApprovalExpectation, ApprovalRequestPayload,
    ApprovalResourceLimits, ArtifactBroker, ArtifactCapabilityManifest, ArtifactCreateRequest,
    ArtifactHandle, ArtifactMetadata, ArtifactOffloadResult, ArtifactReadResult, ArtifactReference,
    ArtifactRetention, ArtifactRetentionPolicy, BrokeredServiceResult, BuildMetadata,
    CapabilityGrant, CapabilityGrantKind, CapabilityManifest, CommandDescription,
    CommandPlanPreview, CommandSchemaReference, ConformanceStatus, EffectiveCapabilities,
    ErrorStatus, EstimatedCost, GithubRequest, GithubResponse, HandleOperation, HealthStatus,
    HostToolRuntime, HttpHeader, InstalledPluginKind, InstalledPluginStatus, InvocationContext,
    InvokeRequest, InvokeResponse, InvokeResult, InvokeStatus, LifecycleActions, LifecycleHooks,
    ManifestValidationIssue, ManifestValidationResult, NetworkPolicy, OauthRequest, OauthResponse,
    OperatingMode, PlanRequest, PlanResult, PlanStatus, PluginAdmissionStatus,
    PluginCommandManifest, PluginDescription, PluginExample, PluginManifest, PluginRegistry,
    PolicyInspectRequest, PolicyInspectResponse, PolicyResolution, PolicyResolver,
    PreconditionModel, PromptingManifest, ProvenanceManifest, RedactionPipeline, RedactionProfile,
    RedactionResult, RedactionTargetKind, ResourceBudgetRequest, ResourceLimits, SearchMatch,
    SearchRequest, SearchResponse, SecretBroker, SecretClass, SecretHandle, SecretMetadata,
    SecretUseGrant, SecurityLevel, ServiceBroker, ServiceHandle, ServiceMetadata, SideEffectClass,
    SignatureMetadata, SshSignRequest, StorageCapabilityManifest, ToolDescribeRequest,
    ToolDescribeRequestEnvelope, ToolDescribeResponse, ToolDescribeResponseEnvelope, ToolError,
    ToolGuidance, ToolInvokeRequestEnvelope, ToolInvokeResponseEnvelope, ToolPlanRequestEnvelope,
    ToolPlanResponseEnvelope, ToolResult, ToolSearchRequestEnvelope, ToolSearchResponseEnvelope,
    ToolTier, WalletSignMessageRequest, WalletSignTxRequest, WorkspaceBroker,
    WorkspaceCapabilityManifest, WorkspaceConflict, WorkspaceEntry, WorkspaceHandle,
    WorkspaceImpact, WorkspaceListResult, WorkspaceMetadata, WorkspaceMutationResult,
    WorkspacePatchRequest, WorkspacePathKind, WorkspacePathMetadata, WorkspacePreconditionStatus,
    WorkspaceReadResult, WorkspaceRevisionRef, WorkspaceWriteRequest, WorkspaceWriteResult,
};
pub use result::ServiceEnvelope;
pub use service::{
    ChatProvider, ChatStreamEvent, ConfigStore, OAuthCallbackPayload, OAuthProvider, OAuthStart,
    PendingOAuthFlow, ProviderCapabilityMatrix, ProviderFinishState, ProviderToolDefinition,
    SessionStore, TokenBundle, ToolAwareChatProvider, ToolAwareProviderResponse,
};
pub use state::{
    FileSecretStore, InMemorySecretStore, InMemoryStateStore, SecretStore, StateStore,
};
