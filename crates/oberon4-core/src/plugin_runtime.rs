use serde::{Deserialize, Serialize};

use crate::RequestId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub version: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    pub description: String,
    #[serde(rename = "runtimeApiVersion")]
    pub runtime_api_version: String,
    pub commands: Vec<PluginCommandManifest>,
    pub capabilities: CapabilityManifest,
    pub prompting: PromptingManifest,
    pub provenance: ProvenanceManifest,
    #[serde(default, rename = "lifecycle")]
    pub lifecycle_actions: Option<LifecycleActions>,
    #[serde(default, rename = "requestedResources")]
    pub requested_resources: Option<ResourceBudgetRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginCommandManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: serde_json::Value,
    #[serde(rename = "outputSchema")]
    pub output_schema: serde_json::Value,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(rename = "sideEffects")]
    pub side_effects: SideEffectClass,
    #[serde(rename = "requiredCapabilities")]
    pub required_capabilities: Vec<String>,
    #[serde(default, rename = "approvalExpectation")]
    pub approval_expectation: Option<ApprovalExpectation>,
    #[serde(default, rename = "supportsPlanning")]
    pub supports_planning: bool,
    #[serde(default, rename = "supportsPagination")]
    pub supports_pagination: bool,
    #[serde(default, rename = "supportsArtifacts")]
    pub supports_artifacts: bool,
    #[serde(default, rename = "preconditionModel")]
    pub precondition_model: Option<PreconditionModel>,
    #[serde(default, rename = "requestedResources")]
    pub requested_resources: Option<ResourceBudgetRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CapabilityManifest {
    #[serde(default)]
    pub storage: StorageCapabilityManifest,
    #[serde(default)]
    pub workspace: WorkspaceCapabilityManifest,
    #[serde(default)]
    pub artifacts: ArtifactCapabilityManifest,
    #[serde(default)]
    pub services: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct StorageCapabilityManifest {
    #[serde(default)]
    pub private: bool,
    #[serde(default)]
    pub session: bool,
    #[serde(default)]
    pub cache: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WorkspaceCapabilityManifest {
    #[serde(default)]
    pub read: bool,
    #[serde(default)]
    pub write: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ArtifactCapabilityManifest {
    #[serde(default)]
    pub read: bool,
    #[serde(default)]
    pub create: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptingManifest {
    #[serde(rename = "promptVersion")]
    pub prompt_version: String,
    pub summary: String,
    #[serde(rename = "whenToUse")]
    pub when_to_use: String,
    #[serde(rename = "whenNotToUse")]
    pub when_not_to_use: String,
    #[serde(default)]
    pub examples: Vec<PluginExample>,
    #[serde(default, rename = "failureRecovery")]
    pub failure_recovery: Vec<String>,
    #[serde(default, rename = "safetyNotes")]
    pub safety_notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginExample {
    pub intent: String,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceManifest {
    pub publisher: String,
    pub source: String,
    pub checksum: String,
    #[serde(default)]
    pub signature: Option<SignatureMetadata>,
    #[serde(default)]
    pub build: Option<BuildMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignatureMetadata {
    pub format: String,
    pub value: String,
    #[serde(default)]
    pub key_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildMetadata {
    pub target: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LifecycleActions {
    #[serde(default)]
    pub install: bool,
    #[serde(default)]
    pub validate: bool,
    #[serde(default)]
    pub activate: bool,
    #[serde(default)]
    pub deactivate: bool,
    #[serde(default)]
    pub upgrade: bool,
    #[serde(default)]
    pub uninstall: bool,
    #[serde(default)]
    pub hooks: LifecycleHooks,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LifecycleHooks {
    #[serde(default, rename = "onInstall")]
    pub on_install: bool,
    #[serde(default, rename = "onUpgrade")]
    pub on_upgrade: bool,
    #[serde(default, rename = "healthCheck")]
    pub health_check: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ResourceBudgetRequest {
    #[serde(default, rename = "memoryBytes")]
    pub memory_bytes: Option<u64>,
    #[serde(default, rename = "timeMs")]
    pub time_ms: Option<u64>,
    #[serde(default, rename = "storageBytes")]
    pub storage_bytes: Option<u64>,
    #[serde(default, rename = "outputBytes")]
    pub output_bytes: Option<u64>,
    #[serde(default, rename = "artifactBytes")]
    pub artifact_bytes: Option<u64>,
    #[serde(default, rename = "networkPolicy")]
    pub network_policy: Option<NetworkPolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginDescription {
    pub id: String,
    pub version: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandDescription {
    pub name: String,
    pub version: String,
    pub description: String,
    #[serde(rename = "sideEffects")]
    pub side_effects: SideEffectClass,
    #[serde(rename = "requiredCapabilities")]
    pub required_capabilities: Vec<String>,
    #[serde(rename = "supportsPlanning")]
    pub supports_planning: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default, rename = "sessionId")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResponse {
    pub matches: Vec<SearchMatch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchMatch {
    pub tool: String,
    pub command: String,
    pub summary: String,
    #[serde(rename = "toolTier")]
    pub tool_tier: ToolTier,
    #[serde(rename = "sideEffects")]
    pub side_effects: SideEffectClass,
    #[serde(rename = "approvalRequired")]
    pub approval_required: bool,
    #[serde(rename = "capabilitiesRequired")]
    pub capabilities_required: Vec<String>,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDescribeRequest {
    pub tool: String,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDescribeResponse {
    pub tool: String,
    pub command: String,
    pub summary: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: CommandSchemaReference,
    #[serde(rename = "outputSchema")]
    pub output_schema: CommandSchemaReference,
    #[serde(rename = "sideEffects")]
    pub side_effects: SideEffectClass,
    #[serde(rename = "approvalExpectation")]
    pub approval_expectation: ApprovalExpectation,
    #[serde(rename = "capabilitiesRequired")]
    pub capabilities_required: Vec<String>,
    #[serde(default, rename = "paginationBehavior")]
    pub pagination_behavior: Option<String>,
    #[serde(default, rename = "artifactRules")]
    pub artifact_rules: Option<String>,
    pub guidance: ToolGuidance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSearchRequestEnvelope {
    pub request: SearchRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSearchResponseEnvelope {
    pub response: SearchResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDescribeRequestEnvelope {
    pub request: ToolDescribeRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDescribeResponseEnvelope {
    pub response: ToolDescribeResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandSchemaReference {
    pub id: String,
    pub schema: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolGuidance {
    #[serde(rename = "whenToUse")]
    pub when_to_use: String,
    #[serde(rename = "whenNotToUse")]
    pub when_not_to_use: String,
    #[serde(default)]
    pub examples: Vec<String>,
    #[serde(default, rename = "commonMistakes")]
    pub common_mistakes: Vec<String>,
    #[serde(default, rename = "failureRecovery")]
    pub failure_recovery: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRequest {
    pub command: String,
    pub input: serde_json::Value,
    pub context: InvocationContext,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanResult {
    pub status: PlanStatus,
    #[serde(rename = "sideEffects")]
    pub side_effects: SideEffectClass,
    #[serde(rename = "approvalRequired")]
    pub approval_required: bool,
    #[serde(rename = "capabilitiesRequired")]
    pub capabilities_required: Vec<String>,
    #[serde(rename = "estimatedCost")]
    pub estimated_cost: EstimatedCost,
    #[serde(rename = "estimatedSteps")]
    pub estimated_steps: u32,
    #[serde(default)]
    pub preconditions: Vec<String>,
    #[serde(default, rename = "blockingIssues")]
    pub blocking_issues: Vec<String>,
    #[serde(default, rename = "resourceLimits")]
    pub resource_limits: Option<ResourceLimits>,
    #[serde(default, rename = "approvalPreview")]
    pub approval_preview: Option<CommandPlanPreview>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPlanRequestEnvelope {
    pub request: PlanRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPlanResponseEnvelope {
    pub response: PlanResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandPlanPreview {
    pub summary: String,
    #[serde(rename = "workspaceImpact")]
    pub workspace_impact: Option<WorkspaceImpact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvocationContext {
    #[serde(rename = "requestId")]
    pub request_id: RequestId,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "pluginId")]
    pub plugin_id: String,
    #[serde(rename = "pluginVersion")]
    pub plugin_version: String,
    pub command: String,
    pub deadline: String,
    #[serde(rename = "runtimeApiVersion")]
    pub runtime_api_version: String,
    #[serde(default, rename = "capabilityGrants")]
    pub capability_grants: Vec<CapabilityGrant>,
    #[serde(default, rename = "workspaceRevision")]
    pub workspace_revision: Option<String>,
    #[serde(rename = "memoryLimit")]
    pub memory_limit: u64,
    #[serde(rename = "timeLimitMs")]
    pub time_limit_ms: u64,
    #[serde(rename = "outputLimitBytes")]
    pub output_limit_bytes: u64,
    #[serde(rename = "storageLimitBytes")]
    pub storage_limit_bytes: u64,
    #[serde(rename = "networkPolicy")]
    pub network_policy: NetworkPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityGrant {
    pub kind: CapabilityGrantKind,
    pub id: String,
    pub operations: Vec<HandleOperation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvokeRequest {
    pub command: String,
    pub input: serde_json::Value,
    pub context: InvocationContext,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvokeResponse {
    pub result: InvokeResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInvokeRequestEnvelope {
    pub request: InvokeRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInvokeResponseEnvelope {
    #[serde(default)]
    pub result: Option<ToolResult>,
    #[serde(default)]
    pub error: Option<ToolError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvokeResult {
    pub status: InvokeStatus,
    pub summary: String,
    pub data: serde_json::Value,
    #[serde(default)]
    pub artifacts: Vec<ArtifactReference>,
    #[serde(default)]
    pub metrics: InvokeMetrics,
    #[serde(default, rename = "nextSuggestedAction")]
    pub next_suggested_action: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct InvokeMetrics {
    #[serde(default, rename = "durationMs")]
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResult {
    pub status: InvokeStatus,
    pub tool: String,
    pub command: String,
    pub summary: String,
    pub data: serde_json::Value,
    #[serde(default)]
    pub artifacts: Vec<ArtifactReference>,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default, rename = "nextSuggestedAction")]
    pub next_suggested_action: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolError {
    pub status: ErrorStatus,
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(rename = "requestId")]
    pub request_id: RequestId,
    #[serde(rename = "pluginId")]
    pub plugin_id: String,
    #[serde(rename = "pluginVersion")]
    pub plugin_version: String,
    pub command: String,
    #[serde(default, rename = "nextSuggestedAction")]
    pub next_suggested_action: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthStatus {
    pub status: HealthState,
    #[serde(rename = "pluginId")]
    pub plugin_id: String,
    #[serde(rename = "pluginVersion")]
    pub plugin_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyInspectRequest {
    pub tool: String,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyInspectResponse {
    pub status: PlanStatus,
    pub tool: String,
    pub command: String,
    pub capabilities: EffectiveCapabilities,
    #[serde(rename = "approvalRequired")]
    pub approval_required: bool,
    #[serde(rename = "resourceLimits")]
    pub resource_limits: ResourceLimits,
    #[serde(rename = "operatingMode")]
    pub operating_mode: OperatingMode,
    #[serde(rename = "pluginAdmissionStatus")]
    pub plugin_admission_status: PluginAdmissionStatus,
    #[serde(rename = "allowlistStatus")]
    pub allowlist_status: AllowlistStatus,
    #[serde(rename = "conformanceStatus")]
    pub conformance_status: ConformanceStatus,
    #[serde(rename = "productionEligible")]
    pub production_eligible: bool,
    #[serde(default, rename = "reasonCodes")]
    pub reason_codes: Vec<String>,
    #[serde(default, rename = "serviceHandlesUsed")]
    pub service_handles_used: Vec<String>,
    #[serde(default, rename = "secretClassesInvolved")]
    pub secret_classes_involved: Vec<SecretClass>,
    #[serde(default, rename = "securityLevels")]
    pub security_levels: Vec<SecurityLevel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPolicyInspectRequestEnvelope {
    pub request: PolicyInspectRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPolicyInspectResponseEnvelope {
    pub response: PolicyInspectResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveCapabilities {
    pub allowed: Vec<String>,
    pub blocked: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRequestPayload {
    #[serde(rename = "approvalId")]
    pub approval_id: String,
    pub tool: String,
    pub command: String,
    pub summary: String,
    #[serde(rename = "sideEffects")]
    pub side_effects: SideEffectClass,
    #[serde(rename = "workspaceImpact")]
    pub workspace_impact: Option<WorkspaceImpact>,
    #[serde(rename = "remoteImpact")]
    pub remote_impact: Option<RemoteImpact>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactReference>,
    #[serde(default, rename = "serviceHandlesUsed")]
    pub service_handles_used: Vec<String>,
    #[serde(default, rename = "secretClassesInvolved")]
    pub secret_classes_involved: Vec<SecretClass>,
    #[serde(rename = "resourceLimits")]
    pub resource_limits: ApprovalResourceLimits,
    #[serde(default, rename = "diffSummary")]
    pub diff_summary: Option<DiffSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalDecisionPayload {
    #[serde(rename = "approvalId")]
    pub approval_id: String,
    pub decision: ApprovalDecision,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceImpact {
    #[serde(rename = "workspaceHandle")]
    pub workspace_handle: String,
    #[serde(rename = "workspaceRevision")]
    pub workspace_revision: String,
    #[serde(default, rename = "touchedPaths")]
    pub touched_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteImpact {
    pub provider: String,
    pub remote: String,
    pub branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalResourceLimits {
    #[serde(default, rename = "timeMs")]
    pub time_ms: Option<u64>,
    #[serde(default, rename = "memoryBytes")]
    pub memory_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffSummary {
    #[serde(rename = "filesChanged")]
    pub files_changed: u32,
    pub insertions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLimits {
    #[serde(default, rename = "memoryBytes")]
    pub memory_bytes: Option<u64>,
    #[serde(default, rename = "timeMs")]
    pub time_ms: Option<u64>,
    #[serde(default, rename = "storageBytes")]
    pub storage_bytes: Option<u64>,
    #[serde(default, rename = "outputBytes")]
    pub output_bytes: Option<u64>,
    #[serde(default, rename = "networkPolicy")]
    pub network_policy: Option<NetworkPolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactReference {
    #[serde(rename = "artifactId")]
    pub artifact_id: String,
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub size: u64,
    #[serde(default)]
    pub retention: Option<ArtifactRetention>,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactHandle {
    #[serde(rename = "artifactId")]
    pub artifact_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactMetadata {
    #[serde(rename = "artifactId")]
    pub artifact_id: String,
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub size: u64,
    pub retention: ArtifactRetention,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(default, rename = "producerTool")]
    pub producer_tool: Option<String>,
    #[serde(default, rename = "producerPlugin")]
    pub producer_plugin: Option<String>,
    #[serde(default)]
    pub checksum: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRetentionPolicy {
    #[serde(rename = "maxArtifactBytes")]
    pub max_artifact_bytes: u64,
    #[serde(rename = "maxRetainedBytes")]
    pub max_retained_bytes: u64,
    #[serde(rename = "sessionTtlSeconds")]
    pub session_ttl_seconds: u64,
    #[serde(rename = "shortTermTtlSeconds")]
    pub short_term_ttl_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactCreateRequest {
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub retention: ArtifactRetention,
    pub bytes: Vec<u8>,
    #[serde(default, rename = "producerTool")]
    pub producer_tool: Option<String>,
    #[serde(default, rename = "producerPlugin")]
    pub producer_plugin: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactReadResult {
    pub metadata: ArtifactMetadata,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactOffloadResult {
    pub artifact: ArtifactReference,
    #[serde(rename = "inlineReplaced")]
    pub inline_replaced: bool,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceHandle {
    #[serde(rename = "workspaceHandle")]
    pub workspace_handle: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceHandle {
    #[serde(rename = "serviceHandle")]
    pub service_handle: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretHandle {
    #[serde(rename = "secretHandle")]
    pub secret_handle: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceRevisionRef {
    #[serde(rename = "workspaceHandle")]
    pub workspace_handle: String,
    #[serde(rename = "workspaceRevision")]
    pub workspace_revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceMetadata {
    #[serde(rename = "workspaceHandle")]
    pub workspace_handle: String,
    #[serde(rename = "workspaceId")]
    pub workspace_id: String,
    #[serde(rename = "workspaceRevision")]
    pub workspace_revision: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
    #[serde(default, rename = "lastMutationRequestId")]
    pub last_mutation_request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    pub name: String,
    pub kind: WorkspacePathKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspacePathMetadata {
    #[serde(rename = "logicalPath")]
    pub logical_path: String,
    pub entry: WorkspaceEntry,
    #[serde(rename = "sizeBytes")]
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceListResult {
    #[serde(rename = "workspaceHandle")]
    pub workspace_handle: String,
    #[serde(rename = "workspaceRevision")]
    pub workspace_revision: String,
    #[serde(rename = "logicalPath")]
    pub logical_path: String,
    pub entries: Vec<WorkspacePathMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceReadResult {
    #[serde(rename = "workspaceHandle")]
    pub workspace_handle: String,
    #[serde(rename = "workspaceRevision")]
    pub workspace_revision: String,
    #[serde(rename = "logicalPath")]
    pub logical_path: String,
    pub content: String,
    #[serde(rename = "bytesRead")]
    pub bytes_read: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceConflict {
    #[serde(rename = "workspaceHandle")]
    pub workspace_handle: String,
    #[serde(rename = "logicalPath")]
    pub logical_path: String,
    #[serde(default, rename = "expectedRevision")]
    pub expected_revision: Option<String>,
    #[serde(rename = "actualRevision")]
    pub actual_revision: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceWriteRequest {
    #[serde(rename = "workspaceHandle")]
    pub workspace_handle: String,
    #[serde(rename = "logicalPath")]
    pub logical_path: String,
    pub content: String,
    #[serde(default, rename = "expectedRevision")]
    pub expected_revision: Option<String>,
    #[serde(default, rename = "requestId")]
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspacePatchRequest {
    #[serde(rename = "workspaceHandle")]
    pub workspace_handle: String,
    #[serde(rename = "logicalPath")]
    pub logical_path: String,
    pub search: String,
    pub replace: String,
    #[serde(default, rename = "expectedRevision")]
    pub expected_revision: Option<String>,
    #[serde(default, rename = "requestId")]
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceWriteResult {
    #[serde(rename = "workspaceHandle")]
    pub workspace_handle: String,
    #[serde(rename = "workspaceRevision")]
    pub workspace_revision: String,
    #[serde(rename = "logicalPath")]
    pub logical_path: String,
    pub operation: String,
    #[serde(rename = "bytesWritten")]
    pub bytes_written: u64,
    #[serde(rename = "preconditionStatus")]
    pub precondition_status: WorkspacePreconditionStatus,
    pub touched: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceMutationResult {
    #[serde(default)]
    pub result: Option<WorkspaceWriteResult>,
    #[serde(default)]
    pub conflict: Option<WorkspaceConflict>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestValidationIssue {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub field: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestValidationResult {
    pub valid: bool,
    #[serde(default)]
    pub issues: Vec<ManifestValidationIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyResolution {
    pub tool: String,
    pub command: String,
    pub capabilities: EffectiveCapabilities,
    #[serde(rename = "approvalRequired")]
    pub approval_required: bool,
    #[serde(rename = "resourceLimits")]
    pub resource_limits: ResourceLimits,
    #[serde(rename = "operatingMode")]
    pub operating_mode: OperatingMode,
    #[serde(rename = "pluginAdmissionStatus")]
    pub plugin_admission_status: PluginAdmissionStatus,
    #[serde(rename = "allowlistStatus")]
    pub allowlist_status: AllowlistStatus,
    #[serde(rename = "conformanceStatus")]
    pub conformance_status: ConformanceStatus,
    #[serde(rename = "productionEligible")]
    pub production_eligible: bool,
    #[serde(default, rename = "reasonCodes")]
    pub reason_codes: Vec<String>,
    #[serde(default, rename = "blockingIssues")]
    pub blocking_issues: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretMetadata {
    #[serde(rename = "secretHandle")]
    pub secret_handle: String,
    #[serde(rename = "secretClass")]
    pub secret_class: SecretClass,
    #[serde(rename = "securityLevel")]
    pub security_level: SecurityLevel,
    #[serde(default, rename = "allowedOperations")]
    pub allowed_operations: Vec<HandleOperation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretUseGrant {
    #[serde(rename = "secretUseHandle")]
    pub secret_use_handle: String,
    #[serde(rename = "secretHandle")]
    pub secret_handle: String,
    #[serde(rename = "secretClass")]
    pub secret_class: SecretClass,
    #[serde(rename = "securityLevel")]
    pub security_level: SecurityLevel,
    #[serde(default, rename = "allowedOperations")]
    pub allowed_operations: Vec<HandleOperation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceMetadata {
    #[serde(rename = "serviceHandle")]
    pub service_handle: String,
    #[serde(rename = "serviceName")]
    pub service_name: String,
    #[serde(default, rename = "secretClass")]
    pub secret_class: Option<SecretClass>,
    #[serde(default, rename = "securityLevel")]
    pub security_level: Option<SecurityLevel>,
    #[serde(default, rename = "allowedOperations")]
    pub allowed_operations: Vec<HandleOperation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubRequest {
    #[serde(rename = "serviceHandle")]
    pub service_handle: String,
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub headers: Vec<HttpHeader>,
    #[serde(default, rename = "bodyJson")]
    pub body_json: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubResponse {
    #[serde(rename = "statusCode")]
    pub status_code: u16,
    #[serde(default)]
    pub headers: Vec<HttpHeader>,
    pub body: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OauthRequest {
    #[serde(rename = "serviceHandle")]
    pub service_handle: String,
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub headers: Vec<HttpHeader>,
    #[serde(default, rename = "bodyJson")]
    pub body_json: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OauthResponse {
    #[serde(rename = "statusCode")]
    pub status_code: u16,
    #[serde(default)]
    pub headers: Vec<HttpHeader>,
    pub body: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshSignRequest {
    #[serde(rename = "serviceHandle")]
    pub service_handle: String,
    pub payload: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletSignTxRequest {
    #[serde(rename = "serviceHandle")]
    pub service_handle: String,
    pub tx: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletSignMessageRequest {
    #[serde(rename = "serviceHandle")]
    pub service_handle: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokeredServiceResult {
    pub summary: String,
    pub data: serde_json::Value,
    #[serde(default, rename = "nextSuggestedAction")]
    pub next_suggested_action: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RedactionTargetKind {
    ToolResult,
    ErrorPayload,
    Stdout,
    Stderr,
    Log,
    Trace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RedactionProfile {
    #[serde(default, rename = "exactValues")]
    pub exact_values: Vec<String>,
    #[serde(default, rename = "failClosed")]
    pub fail_closed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RedactionResult {
    #[serde(rename = "redactedText")]
    pub redacted_text: String,
    #[serde(default)]
    pub applied: bool,
    #[serde(default)]
    pub blocked: bool,
    #[serde(default)]
    pub markers: Vec<String>,
}

pub trait PluginRegistry {
    fn list_plugins(&self) -> Vec<PluginDescription>;
    fn search(&self, request: &SearchRequest) -> Result<SearchResponse, crate::OberonError>;
    fn describe(
        &self,
        request: &ToolDescribeRequest,
    ) -> Result<ToolDescribeResponse, crate::OberonError>;
    fn manifest(&self, tool: &str) -> Result<PluginManifest, crate::OberonError>;
    fn command_manifest(
        &self,
        tool: &str,
        command: &str,
    ) -> Result<PluginCommandManifest, crate::OberonError>;
}

pub trait PolicyResolver {
    fn resolve(
        &self,
        tool: &str,
        command: &PluginCommandManifest,
        manifest: &PluginManifest,
    ) -> Result<PolicyResolution, crate::OberonError>;
}

pub trait SecretBroker {
    fn resolve_secret(&self, secret_handle: &str) -> Result<SecretMetadata, crate::OberonError>;
    fn grant_secret_use(
        &self,
        secret_handle: &str,
        operation: HandleOperation,
    ) -> Result<SecretUseGrant, crate::OberonError>;
}

pub trait ServiceBroker {
    fn resolve_service(&self, service_handle: &str) -> Result<ServiceMetadata, crate::OberonError>;
    fn redaction_values_for_service(
        &self,
        service_handle: &str,
    ) -> Result<Vec<String>, crate::OberonError>;
    fn execute_github_request(
        &self,
        request: &GithubRequest,
    ) -> Result<GithubResponse, crate::OberonError>;
    fn execute_oauth_request(
        &self,
        request: &OauthRequest,
    ) -> Result<OauthResponse, crate::OberonError>;
    fn sign_ssh(&self, request: &SshSignRequest) -> Result<String, crate::OberonError>;
    fn sign_wallet_tx(&self, request: &WalletSignTxRequest) -> Result<String, crate::OberonError>;
    fn sign_wallet_message(
        &self,
        request: &WalletSignMessageRequest,
    ) -> Result<String, crate::OberonError>;
}

pub trait WorkspaceBroker {
    fn create_or_attach_workspace(
        &self,
        workspace: &WorkspaceHandle,
    ) -> Result<WorkspaceMetadata, crate::OberonError>;
    fn workspace_metadata(
        &self,
        workspace: &WorkspaceHandle,
    ) -> Result<WorkspaceMetadata, crate::OberonError>;
    fn list_dir(
        &self,
        workspace: &WorkspaceHandle,
        logical_path: &str,
    ) -> Result<WorkspaceListResult, crate::OberonError>;
    fn stat_path(
        &self,
        workspace: &WorkspaceHandle,
        logical_path: &str,
    ) -> Result<WorkspacePathMetadata, crate::OberonError>;
    fn read_file(
        &self,
        workspace: &WorkspaceHandle,
        logical_path: &str,
    ) -> Result<WorkspaceReadResult, crate::OberonError>;
    fn write_file_atomic(
        &self,
        request: &WorkspaceWriteRequest,
    ) -> Result<WorkspaceMutationResult, crate::OberonError>;
    fn apply_patch(
        &self,
        request: &WorkspacePatchRequest,
    ) -> Result<WorkspaceMutationResult, crate::OberonError>;
}

pub trait ArtifactBroker {
    fn create_artifact(
        &self,
        request: &ArtifactCreateRequest,
    ) -> Result<ArtifactMetadata, crate::OberonError>;
    fn resolve_artifact(
        &self,
        artifact: &ArtifactHandle,
    ) -> Result<ArtifactMetadata, crate::OberonError>;
    fn read_artifact(
        &self,
        artifact: &ArtifactHandle,
        max_bytes: u64,
    ) -> Result<ArtifactReadResult, crate::OberonError>;
}

pub trait RedactionPipeline {
    fn redact_text(
        &self,
        target: RedactionTargetKind,
        text: &str,
        profile: &RedactionProfile,
    ) -> Result<RedactionResult, crate::OberonError>;
    fn redact_json(
        &self,
        target: RedactionTargetKind,
        value: &serde_json::Value,
        profile: &RedactionProfile,
    ) -> Result<(serde_json::Value, RedactionResult), crate::OberonError>;
}

pub trait HostToolRuntime {
    fn tool_search(&self, request: &SearchRequest) -> Result<SearchResponse, crate::OberonError>;
    fn tool_describe(
        &self,
        request: &ToolDescribeRequest,
    ) -> Result<ToolDescribeResponse, crate::OberonError>;
    fn tool_plan(&self, request: &PlanRequest) -> Result<PlanResult, crate::OberonError>;
    fn tool_invoke(
        &self,
        request: &InvokeRequest,
    ) -> Result<ToolInvokeResponseEnvelope, crate::OberonError>;
    fn tool_policy_inspect(
        &self,
        request: &PolicyInspectRequest,
    ) -> Result<PolicyInspectResponse, crate::OberonError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolTier {
    CoreHost,
    BrokeredPrivileged,
    OrdinaryPlugin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SideEffectClass {
    None,
    Ephemeral,
    PluginPrivateWrite,
    ExternalRead,
    ExternalWrite,
    Destructive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalExpectation {
    None,
    Recommended,
    Required,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PreconditionModel {
    None,
    WorkspaceRevision,
    FileHash,
    PatchContext,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkPolicy {
    Denied,
    BrokeredOnly,
    Allowlisted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatingMode {
    Dev,
    Production,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginAdmissionStatus {
    Allowed,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AllowlistStatus {
    NotRequired,
    Allowlisted,
    Missing,
    ExemptBuiltIn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConformanceStatus {
    Passed,
    Failed,
    NotRun,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretClass {
    ApiCredential,
    OauthCredential,
    SshKey,
    WalletKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SecurityLevel {
    HardwareBacked,
    OsProtected,
    EncryptedLocal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityGrantKind {
    StorageHandle,
    ArtifactHandle,
    WorkspaceHandle,
    ServiceHandle,
    SecretUseHandle,
    SecretHandle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandleOperation {
    Read,
    Write,
    List,
    Stat,
    Create,
    Delete,
    Invoke,
    Sign,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstimatedCost {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Ok,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvokeStatus {
    Ok,
    Pending,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorStatus {
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Ok,
    Degraded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approved,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactRetention {
    Session,
    ShortTerm,
    Durable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspacePathKind {
    File,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspacePreconditionStatus {
    Matched,
    Conflict,
}

#[cfg(test)]
mod tests {
    use super::{
        AllowlistStatus, ApprovalDecision, ApprovalDecisionPayload, ApprovalExpectation,
        ApprovalRequestPayload, ApprovalResourceLimits, ArtifactReference, BuildMetadata,
        CapabilityGrant, CapabilityGrantKind, CapabilityManifest, CommandSchemaReference,
        ConformanceStatus, DiffSummary, EffectiveCapabilities, EstimatedCost, HandleOperation,
        InvocationContext, InvokeMetrics, InvokeRequest, InvokeResult, InvokeStatus,
        LifecycleActions, LifecycleHooks, ManifestValidationIssue, ManifestValidationResult,
        NetworkPolicy, OperatingMode, PlanRequest, PlanResult, PlanStatus, PluginAdmissionStatus,
        PluginCommandManifest, PluginDescription, PluginExample, PluginManifest,
        PolicyInspectResponse, PolicyResolution, PreconditionModel, PromptingManifest,
        ProvenanceManifest, RemoteImpact, ResourceBudgetRequest, ResourceLimits, SearchMatch,
        SearchRequest, SearchResponse, SecretClass, SecurityLevel, SideEffectClass,
        SignatureMetadata, ToolDescribeRequest, ToolDescribeRequestEnvelope, ToolDescribeResponse,
        ToolDescribeResponseEnvelope, ToolError, ToolGuidance, ToolInvokeRequestEnvelope,
        ToolInvokeResponseEnvelope, ToolPlanRequestEnvelope, ToolPlanResponseEnvelope, ToolResult,
        ToolSearchRequestEnvelope, ToolSearchResponseEnvelope, ToolTier, WorkspaceImpact,
    };
    use crate::RequestId;

    fn sample_manifest() -> PluginManifest {
        PluginManifest {
            id: "workspace-git".to_string(),
            version: "0.1.0".to_string(),
            display_name: "Versioned Workspace".to_string(),
            description: "Workspace editing and versioned git operations.".to_string(),
            runtime_api_version: "1.0".to_string(),
            commands: vec![PluginCommandManifest {
                name: "write_file_atomic".to_string(),
                version: "1".to_string(),
                description: "Write file contents atomically.".to_string(),
                input_schema: serde_json::json!({"type":"object"}),
                output_schema: serde_json::json!({"type":"object"}),
                tags: vec!["workspace".to_string(), "write".to_string()],
                side_effects: SideEffectClass::ExternalWrite,
                required_capabilities: vec!["workspace.write".to_string()],
                approval_expectation: Some(ApprovalExpectation::Required),
                supports_planning: true,
                supports_pagination: false,
                supports_artifacts: false,
                precondition_model: Some(PreconditionModel::WorkspaceRevision),
                requested_resources: Some(ResourceBudgetRequest {
                    memory_bytes: Some(134_217_728),
                    time_ms: Some(5_000),
                    storage_bytes: None,
                    output_bytes: None,
                    artifact_bytes: None,
                    network_policy: Some(NetworkPolicy::Denied),
                }),
            }],
            capabilities: CapabilityManifest::default(),
            prompting: PromptingManifest {
                prompt_version: "1".to_string(),
                summary: "Use for safe versioned workspace operations.".to_string(),
                when_to_use: "Use when reading or editing the granted workspace.".to_string(),
                when_not_to_use: "Do not use for remote-only metadata operations.".to_string(),
                examples: vec![PluginExample {
                    intent: "Update a file and commit the change".to_string(),
                    command: "write_file_atomic".to_string(),
                }],
                failure_recovery: vec![
                    "If a revision conflict occurs, re-read the target and re-plan.".to_string(),
                ],
                safety_notes: vec![
                    "Writes require a valid workspace handle and may require approval.".to_string(),
                ],
            },
            provenance: ProvenanceManifest {
                publisher: "oberon4".to_string(),
                source: "local-bundle".to_string(),
                checksum: "sha256:abc".to_string(),
                signature: Some(SignatureMetadata {
                    format: "sigstore".to_string(),
                    value: "abc".to_string(),
                    key_id: Some("key-1".to_string()),
                }),
                build: Some(BuildMetadata {
                    target: "wasm32-wasip2".to_string(),
                    created_at: "2026-03-08T00:00:00Z".to_string(),
                }),
            },
            lifecycle_actions: Some(LifecycleActions {
                install: true,
                validate: true,
                activate: true,
                deactivate: true,
                upgrade: true,
                uninstall: true,
                hooks: LifecycleHooks {
                    on_install: true,
                    on_upgrade: true,
                    health_check: true,
                },
            }),
            requested_resources: Some(ResourceBudgetRequest {
                memory_bytes: Some(134_217_728),
                time_ms: Some(5_000),
                storage_bytes: Some(268_435_456),
                output_bytes: Some(131_072),
                artifact_bytes: Some(33_554_432),
                network_policy: Some(NetworkPolicy::Denied),
            }),
        }
    }

    #[test]
    fn plugin_manifest_round_trips() {
        let manifest = sample_manifest();
        let encoded = serde_json::to_string(&manifest).unwrap();
        let decoded: PluginManifest = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, manifest);
    }

    #[test]
    fn search_envelopes_round_trip() {
        let request = SearchRequest {
            query: "find workspace writer".to_string(),
            limit: Some(5),
            session_id: Some("sess_123".to_string()),
        };
        let request_json = serde_json::to_string(&request).unwrap();
        let decoded_request: SearchRequest = serde_json::from_str(&request_json).unwrap();
        assert_eq!(decoded_request, request);

        let response = SearchResponse {
            matches: vec![SearchMatch {
                tool: "workspace".to_string(),
                command: "write_file_atomic".to_string(),
                summary: "Write file contents atomically.".to_string(),
                tool_tier: ToolTier::OrdinaryPlugin,
                side_effects: SideEffectClass::ExternalWrite,
                approval_required: true,
                capabilities_required: vec!["workspace.write".to_string()],
                confidence: "0.92".to_string(),
            }],
        };
        let response_json = serde_json::to_string(&response).unwrap();
        let decoded_response: SearchResponse = serde_json::from_str(&response_json).unwrap();
        assert_eq!(decoded_response, response);
    }

    #[test]
    fn describe_envelopes_round_trip() {
        let request = ToolDescribeRequest {
            tool: "workspace".to_string(),
            command: "write_file_atomic".to_string(),
        };
        let encoded_request = serde_json::to_string(&request).unwrap();
        let decoded_request: ToolDescribeRequest = serde_json::from_str(&encoded_request).unwrap();
        assert_eq!(decoded_request, request);

        let response = ToolDescribeResponse {
            tool: "workspace".to_string(),
            command: "write_file_atomic".to_string(),
            summary: "Write file contents atomically.".to_string(),
            input_schema: CommandSchemaReference {
                id: "workspace.write_file_atomic.input".to_string(),
                schema: serde_json::json!({"type":"object"}),
            },
            output_schema: CommandSchemaReference {
                id: "workspace.write_file_atomic.output".to_string(),
                schema: serde_json::json!({"type":"object"}),
            },
            side_effects: SideEffectClass::ExternalWrite,
            approval_expectation: ApprovalExpectation::Required,
            capabilities_required: vec!["workspace.write".to_string()],
            pagination_behavior: None,
            artifact_rules: None,
            guidance: ToolGuidance {
                when_to_use: "Use when editing a workspace file.".to_string(),
                when_not_to_use: "Do not use for remote-only operations.".to_string(),
                examples: vec!["Update docs/notes.md".to_string()],
                common_mistakes: vec!["Passing an absolute path".to_string()],
                failure_recovery: vec!["Retry after re-reading workspace revision".to_string()],
            },
        };
        let encoded_response = serde_json::to_string(&response).unwrap();
        let decoded_response: ToolDescribeResponse =
            serde_json::from_str(&encoded_response).unwrap();
        assert_eq!(decoded_response, response);
    }

    #[test]
    fn plan_and_invoke_envelopes_round_trip() {
        let context = InvocationContext {
            request_id: RequestId::new(),
            session_id: "sess_123".to_string(),
            plugin_id: "workspace-git".to_string(),
            plugin_version: "0.1.0".to_string(),
            command: "write_file_atomic".to_string(),
            deadline: "2026-03-08T12:00:05Z".to_string(),
            runtime_api_version: "1.0".to_string(),
            capability_grants: vec![CapabilityGrant {
                kind: CapabilityGrantKind::WorkspaceHandle,
                id: "wrk_123".to_string(),
                operations: vec![HandleOperation::Read, HandleOperation::Write],
            }],
            workspace_revision: Some("wsr_123".to_string()),
            memory_limit: 134_217_728,
            time_limit_ms: 5_000,
            output_limit_bytes: 131_072,
            storage_limit_bytes: 268_435_456,
            network_policy: NetworkPolicy::Denied,
        };

        let plan = PlanRequest {
            command: "write_file_atomic".to_string(),
            input: serde_json::json!({"workspaceHandle":"wrk_123","path":"docs/notes.md"}),
            context: context.clone(),
        };
        let encoded_plan = serde_json::to_string(&plan).unwrap();
        let decoded_plan: PlanRequest = serde_json::from_str(&encoded_plan).unwrap();
        assert_eq!(decoded_plan, plan);

        let plan_result = PlanResult {
            status: PlanStatus::Ok,
            side_effects: SideEffectClass::ExternalWrite,
            approval_required: true,
            capabilities_required: vec!["workspace.write".to_string()],
            estimated_cost: EstimatedCost::Low,
            estimated_steps: 1,
            preconditions: vec!["workspaceRevision".to_string()],
            blocking_issues: Vec::new(),
            resource_limits: Some(ResourceLimits {
                memory_bytes: Some(134_217_728),
                time_ms: Some(5_000),
                storage_bytes: None,
                output_bytes: None,
                network_policy: Some(NetworkPolicy::Denied),
            }),
            approval_preview: None,
        };
        let encoded_plan_result = serde_json::to_string(&plan_result).unwrap();
        let decoded_plan_result: PlanResult = serde_json::from_str(&encoded_plan_result).unwrap();
        assert_eq!(decoded_plan_result, plan_result);

        let invoke_result = InvokeResult {
            status: InvokeStatus::Ok,
            summary: "Wrote docs/notes.md".to_string(),
            data: serde_json::json!({"logicalPath":"docs/notes.md"}),
            artifacts: vec![ArtifactReference {
                artifact_id: "art_123".to_string(),
                media_type: "text/x-diff".to_string(),
                size: 120,
                retention: None,
                truncated: false,
            }],
            metrics: InvokeMetrics { duration_ms: 43 },
            next_suggested_action: Some("git.diff".to_string()),
        };
        let encoded_invoke_result = serde_json::to_string(&invoke_result).unwrap();
        let decoded_invoke_result: InvokeResult =
            serde_json::from_str(&encoded_invoke_result).unwrap();
        assert_eq!(decoded_invoke_result, invoke_result);
    }

    #[test]
    fn policy_and_approval_envelopes_round_trip() {
        let policy = PolicyInspectResponse {
            status: PlanStatus::Ok,
            tool: "git".to_string(),
            command: "push".to_string(),
            capabilities: EffectiveCapabilities {
                allowed: vec![
                    "workspace.read".to_string(),
                    "workspace.write".to_string(),
                    "github.request".to_string(),
                ],
                blocked: Vec::new(),
            },
            approval_required: true,
            resource_limits: ResourceLimits {
                memory_bytes: Some(134_217_728),
                time_ms: Some(5_000),
                storage_bytes: Some(268_435_456),
                output_bytes: Some(131_072),
                network_policy: Some(NetworkPolicy::BrokeredOnly),
            },
            operating_mode: OperatingMode::Production,
            plugin_admission_status: PluginAdmissionStatus::Allowed,
            allowlist_status: AllowlistStatus::Allowlisted,
            conformance_status: ConformanceStatus::Passed,
            production_eligible: true,
            reason_codes: Vec::new(),
            service_handles_used: vec!["svc_github_main".to_string()],
            secret_classes_involved: vec![SecretClass::OauthCredential],
            security_levels: vec![SecurityLevel::OsProtected],
        };
        let encoded_policy = serde_json::to_string(&policy).unwrap();
        let decoded_policy: PolicyInspectResponse = serde_json::from_str(&encoded_policy).unwrap();
        assert_eq!(decoded_policy, policy);

        let approval = ApprovalRequestPayload {
            approval_id: "appr_123".to_string(),
            tool: "git".to_string(),
            command: "push".to_string(),
            summary: "Push branch feature/docs to origin".to_string(),
            side_effects: SideEffectClass::ExternalWrite,
            workspace_impact: Some(WorkspaceImpact {
                workspace_handle: "wrk_123".to_string(),
                workspace_revision: "wsr_123".to_string(),
                touched_paths: vec!["docs/notes.md".to_string()],
            }),
            remote_impact: Some(RemoteImpact {
                provider: "github".to_string(),
                remote: "origin".to_string(),
                branch: "feature/docs".to_string(),
            }),
            artifacts: Vec::new(),
            service_handles_used: vec!["svc_github_main".to_string()],
            secret_classes_involved: vec![SecretClass::OauthCredential],
            resource_limits: ApprovalResourceLimits {
                time_ms: Some(5_000),
                memory_bytes: None,
            },
            diff_summary: Some(DiffSummary {
                files_changed: 1,
                insertions: 10,
                deletions: 2,
            }),
        };
        let encoded_approval = serde_json::to_string(&approval).unwrap();
        let decoded_approval: ApprovalRequestPayload =
            serde_json::from_str(&encoded_approval).unwrap();
        assert_eq!(decoded_approval, approval);

        let decision = ApprovalDecisionPayload {
            approval_id: "appr_123".to_string(),
            decision: ApprovalDecision::Approved,
            reason: None,
        };
        let encoded_decision = serde_json::to_string(&decision).unwrap();
        let decoded_decision: ApprovalDecisionPayload =
            serde_json::from_str(&encoded_decision).unwrap();
        assert_eq!(decoded_decision, decision);
    }

    #[test]
    fn tool_result_and_error_round_trip() {
        let result = ToolResult {
            status: InvokeStatus::Ok,
            tool: "workspace".to_string(),
            command: "write_file_atomic".to_string(),
            summary: "Wrote docs/notes.md".to_string(),
            data: serde_json::json!({"bytesWritten":5}),
            artifacts: Vec::new(),
            truncated: false,
            next_suggested_action: Some("git.diff".to_string()),
        };
        let encoded_result = serde_json::to_string(&result).unwrap();
        let decoded_result: ToolResult = serde_json::from_str(&encoded_result).unwrap();
        assert_eq!(decoded_result, result);

        let error = ToolError {
            status: super::ErrorStatus::Error,
            code: "RESOURCE_LIMIT_STORAGE".to_string(),
            message: "Workspace quota exceeded.".to_string(),
            retryable: false,
            request_id: RequestId::new(),
            plugin_id: "workspace-git".to_string(),
            plugin_version: "0.1.0".to_string(),
            command: "write_file_atomic".to_string(),
            next_suggested_action: Some("reduce output or request a higher quota".to_string()),
        };
        let encoded_error = serde_json::to_string(&error).unwrap();
        let decoded_error: ToolError = serde_json::from_str(&encoded_error).unwrap();
        assert_eq!(decoded_error, error);
    }

    #[test]
    fn enum_serialization_is_stable() {
        assert_eq!(
            serde_json::to_string(&SideEffectClass::ExternalWrite).unwrap(),
            "\"external-write\""
        );
        assert_eq!(
            serde_json::to_string(&NetworkPolicy::BrokeredOnly).unwrap(),
            "\"brokered-only\""
        );
        assert_eq!(
            serde_json::to_string(&SecretClass::OauthCredential).unwrap(),
            "\"oauth_credential\""
        );
    }

    #[test]
    fn plugin_manifest_ignores_unknown_fields_for_forward_compatibility() {
        let mut value = serde_json::to_value(sample_manifest()).unwrap();
        value["futureField"] = serde_json::json!("ok");
        let decoded: PluginManifest = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.id, "workspace-git");
    }

    #[test]
    fn plugin_description_round_trips() {
        let description = PluginDescription {
            id: "workspace-git".to_string(),
            version: "0.1.0".to_string(),
            display_name: "Versioned Workspace".to_string(),
            description: "Workspace editing and versioned git operations.".to_string(),
        };
        let encoded = serde_json::to_string(&description).unwrap();
        let decoded: PluginDescription = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, description);
    }

    #[test]
    fn host_tool_envelopes_round_trip() {
        let search_request = ToolSearchRequestEnvelope {
            request: SearchRequest {
                query: "workspace writer".to_string(),
                limit: Some(5),
                session_id: Some("sess_123".to_string()),
            },
        };
        let encoded_search_request = serde_json::to_string(&search_request).unwrap();
        let decoded_search_request: ToolSearchRequestEnvelope =
            serde_json::from_str(&encoded_search_request).unwrap();
        assert_eq!(decoded_search_request, search_request);

        let search_response = ToolSearchResponseEnvelope {
            response: SearchResponse {
                matches: vec![SearchMatch {
                    tool: "workspace".to_string(),
                    command: "write_file_atomic".to_string(),
                    summary: "Write file contents atomically.".to_string(),
                    tool_tier: ToolTier::OrdinaryPlugin,
                    side_effects: SideEffectClass::ExternalWrite,
                    approval_required: true,
                    capabilities_required: vec!["workspace.write".to_string()],
                    confidence: "0.92".to_string(),
                }],
            },
        };
        let encoded_search_response = serde_json::to_string(&search_response).unwrap();
        let decoded_search_response: ToolSearchResponseEnvelope =
            serde_json::from_str(&encoded_search_response).unwrap();
        assert_eq!(decoded_search_response, search_response);

        let describe_request = ToolDescribeRequestEnvelope {
            request: ToolDescribeRequest {
                tool: "workspace".to_string(),
                command: "write_file_atomic".to_string(),
            },
        };
        let encoded_describe_request = serde_json::to_string(&describe_request).unwrap();
        let decoded_describe_request: ToolDescribeRequestEnvelope =
            serde_json::from_str(&encoded_describe_request).unwrap();
        assert_eq!(decoded_describe_request, describe_request);

        let describe_response = ToolDescribeResponseEnvelope {
            response: ToolDescribeResponse {
                tool: "workspace".to_string(),
                command: "write_file_atomic".to_string(),
                summary: "Write file contents atomically.".to_string(),
                input_schema: CommandSchemaReference {
                    id: "workspace.write_file_atomic.input".to_string(),
                    schema: serde_json::json!({"type":"object"}),
                },
                output_schema: CommandSchemaReference {
                    id: "workspace.write_file_atomic.output".to_string(),
                    schema: serde_json::json!({"type":"object"}),
                },
                side_effects: SideEffectClass::ExternalWrite,
                approval_expectation: ApprovalExpectation::Required,
                capabilities_required: vec!["workspace.write".to_string()],
                pagination_behavior: None,
                artifact_rules: None,
                guidance: ToolGuidance {
                    when_to_use: "Use when editing a workspace file.".to_string(),
                    when_not_to_use: "Do not use for remote-only operations.".to_string(),
                    examples: vec!["Update docs/notes.md".to_string()],
                    common_mistakes: vec!["Passing an absolute path".to_string()],
                    failure_recovery: vec!["Retry after re-reading workspace revision".to_string()],
                },
            },
        };
        let encoded_describe_response = serde_json::to_string(&describe_response).unwrap();
        let decoded_describe_response: ToolDescribeResponseEnvelope =
            serde_json::from_str(&encoded_describe_response).unwrap();
        assert_eq!(decoded_describe_response, describe_response);

        let context = InvocationContext {
            request_id: RequestId::new(),
            session_id: "sess_123".to_string(),
            plugin_id: "workspace-git".to_string(),
            plugin_version: "0.1.0".to_string(),
            command: "write_file_atomic".to_string(),
            deadline: "2026-03-08T12:00:05Z".to_string(),
            runtime_api_version: "1.0".to_string(),
            capability_grants: vec![CapabilityGrant {
                kind: CapabilityGrantKind::WorkspaceHandle,
                id: "wrk_123".to_string(),
                operations: vec![HandleOperation::Read, HandleOperation::Write],
            }],
            workspace_revision: Some("wsr_123".to_string()),
            memory_limit: 134_217_728,
            time_limit_ms: 5_000,
            output_limit_bytes: 131_072,
            storage_limit_bytes: 268_435_456,
            network_policy: NetworkPolicy::Denied,
        };

        let plan_request = ToolPlanRequestEnvelope {
            request: PlanRequest {
                command: "write_file_atomic".to_string(),
                input: serde_json::json!({"workspaceHandle":"wrk_123","path":"docs/notes.md"}),
                context: context.clone(),
            },
        };
        let encoded_plan_request = serde_json::to_string(&plan_request).unwrap();
        let decoded_plan_request: ToolPlanRequestEnvelope =
            serde_json::from_str(&encoded_plan_request).unwrap();
        assert_eq!(decoded_plan_request, plan_request);

        let plan_response = ToolPlanResponseEnvelope {
            response: PlanResult {
                status: PlanStatus::Ok,
                side_effects: SideEffectClass::ExternalWrite,
                approval_required: true,
                capabilities_required: vec!["workspace.write".to_string()],
                estimated_cost: EstimatedCost::Low,
                estimated_steps: 1,
                preconditions: vec!["workspaceRevision".to_string()],
                blocking_issues: Vec::new(),
                resource_limits: Some(ResourceLimits {
                    memory_bytes: Some(134_217_728),
                    time_ms: Some(5_000),
                    storage_bytes: None,
                    output_bytes: None,
                    network_policy: Some(NetworkPolicy::Denied),
                }),
                approval_preview: None,
            },
        };
        let encoded_plan_response = serde_json::to_string(&plan_response).unwrap();
        let decoded_plan_response: ToolPlanResponseEnvelope =
            serde_json::from_str(&encoded_plan_response).unwrap();
        assert_eq!(decoded_plan_response, plan_response);

        let invoke_request = ToolInvokeRequestEnvelope {
            request: InvokeRequest {
                command: "write_file_atomic".to_string(),
                input: serde_json::json!({"workspaceHandle":"wrk_123","path":"docs/notes.md"}),
                context,
            },
        };
        let encoded_invoke_request = serde_json::to_string(&invoke_request).unwrap();
        let decoded_invoke_request: ToolInvokeRequestEnvelope =
            serde_json::from_str(&encoded_invoke_request).unwrap();
        assert_eq!(decoded_invoke_request, invoke_request);

        let invoke_response = ToolInvokeResponseEnvelope {
            result: None,
            error: Some(ToolError {
                status: super::ErrorStatus::Error,
                code: "NOT_YET_EXECUTABLE".to_string(),
                message: "plugin execution is not available until WP2".to_string(),
                retryable: false,
                request_id: RequestId::new(),
                plugin_id: "workspace-git".to_string(),
                plugin_version: "0.1.0".to_string(),
                command: "write_file_atomic".to_string(),
                next_suggested_action: Some("tool_plan".to_string()),
            }),
        };
        let encoded_invoke_response = serde_json::to_string(&invoke_response).unwrap();
        let decoded_invoke_response: ToolInvokeResponseEnvelope =
            serde_json::from_str(&encoded_invoke_response).unwrap();
        assert_eq!(decoded_invoke_response, invoke_response);
    }

    #[test]
    fn manifest_validation_and_policy_models_round_trip() {
        let validation = ManifestValidationResult {
            valid: false,
            issues: vec![ManifestValidationIssue {
                code: "duplicate_command".to_string(),
                message: "command names must be unique".to_string(),
                field: Some("commands[1].name".to_string()),
            }],
        };
        let encoded_validation = serde_json::to_string(&validation).unwrap();
        let decoded_validation: ManifestValidationResult =
            serde_json::from_str(&encoded_validation).unwrap();
        assert_eq!(decoded_validation, validation);

        let resolution = PolicyResolution {
            tool: "workspace-git".to_string(),
            command: "write_file_atomic".to_string(),
            capabilities: EffectiveCapabilities {
                allowed: vec!["workspace.write".to_string()],
                blocked: Vec::new(),
            },
            approval_required: true,
            resource_limits: ResourceLimits {
                memory_bytes: Some(134_217_728),
                time_ms: Some(5_000),
                storage_bytes: Some(268_435_456),
                output_bytes: Some(131_072),
                network_policy: Some(NetworkPolicy::Denied),
            },
            operating_mode: OperatingMode::Production,
            plugin_admission_status: PluginAdmissionStatus::Allowed,
            allowlist_status: AllowlistStatus::Allowlisted,
            conformance_status: ConformanceStatus::Passed,
            production_eligible: true,
            reason_codes: Vec::new(),
            blocking_issues: Vec::new(),
        };
        let encoded_resolution = serde_json::to_string(&resolution).unwrap();
        let decoded_resolution: PolicyResolution =
            serde_json::from_str(&encoded_resolution).unwrap();
        assert_eq!(decoded_resolution, resolution);
    }
}
