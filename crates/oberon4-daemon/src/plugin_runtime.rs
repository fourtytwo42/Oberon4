use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use oberon4_core::{
    ApprovalExpectation, ApprovalRequestPayload, ApprovalResourceLimits, ArtifactRetentionPolicy,
    BrokeredServiceResult, CapabilityManifest, EffectiveCapabilities, ErrorStatus, GithubRequest,
    HostToolRuntime, InvokeStatus, ManifestValidationIssue, ManifestValidationResult,
    NetworkPolicy, OauthRequest, OberonError, OperatingMode, PlanRequest, PlanResult, PlanStatus,
    PluginCommandManifest, PluginDescription, PluginManifest, PluginRegistry, PolicyInspectRequest,
    PolicyInspectResponse, PolicyResolution, PolicyResolver, PreconditionModel, RedactionPipeline,
    RedactionProfile, RedactionTargetKind, RequestId, ResourceLimits, SearchMatch, SearchRequest,
    SearchResponse, SecretBroker, SecretClass, SecurityLevel, ServiceBroker, SideEffectClass,
    ToolDescribeRequest, ToolDescribeResponse, ToolError, ToolGuidance, ToolInvokeResponseEnvelope,
    ToolTier, WorkspaceBroker, WorkspaceHandle, WorkspaceImpact,
};

use crate::plugin_runtime_broker::{
    InMemoryServiceTransport, NativeSecretBackendUnavailable, PluginRuntimeRedactor,
    PluginRuntimeSecretBroker, PluginRuntimeServiceBroker,
};
use crate::plugin_runtime_data::{PluginRuntimeArtifactBroker, PluginRuntimeWorkspaceBroker};
use crate::plugin_runtime_exec::ExecutionEngine;
use crate::plugin_runtime_versioned::PluginRuntimeVersionedPackage;

pub const HOST_RUNTIME_API_VERSION: &str = "1.0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactBrokerFoundation {
    pub inline_output_threshold_bytes: u64,
    pub max_artifact_bytes: u64,
    pub retained_artifact_bytes: u64,
}

impl Default for ArtifactBrokerFoundation {
    fn default() -> Self {
        Self {
            inline_output_threshold_bytes: 131_072,
            max_artifact_bytes: 33_554_432,
            retained_artifact_bytes: 268_435_456,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageBrokerFoundation {
    pub private_storage_bytes: u64,
    pub session_storage_bytes: u64,
    pub cache_storage_bytes: u64,
    pub workspace_handles_required: bool,
}

impl Default for StorageBrokerFoundation {
    fn default() -> Self {
        Self {
            private_storage_bytes: 67_108_864,
            session_storage_bytes: 268_435_456,
            cache_storage_bytes: 536_870_912,
            workspace_handles_required: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RuntimeBrokerFoundations {
    pub artifact: ArtifactBrokerFoundation,
    pub storage: StorageBrokerFoundation,
}

type ServicePolicyMetadata = (Vec<String>, Vec<SecretClass>, Vec<SecurityLevel>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredPlugin {
    pub manifest_path: PathBuf,
    pub wasm_path: PathBuf,
    pub manifest: PluginManifest,
    pub tool_tier: ToolTier,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexedCommand {
    tool: String,
    command: String,
    summary: String,
    tool_tier: ToolTier,
    tags: Vec<String>,
    side_effects: SideEffectClass,
    capabilities_required: Vec<String>,
    approval_expectation: Option<ApprovalExpectation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePluginRegistry {
    runtime_api_version: String,
    plugins: BTreeMap<String, RegisteredPlugin>,
    index: Vec<IndexedCommand>,
}

impl FilePluginRegistry {
    pub fn load_from_roots(
        plugin_roots: &[PathBuf],
        runtime_api_version: &str,
    ) -> Result<Self, OberonError> {
        let mut plugins = BTreeMap::new();
        let mut index = Vec::new();

        for registered in PluginRuntimeVersionedPackage::registered_plugins() {
            insert_registered_plugin(
                &mut plugins,
                &mut index,
                registered,
                runtime_api_version,
                false,
            )?;
        }

        for root in plugin_roots {
            let manifest_paths = discover_manifest_paths(root)?;
            for manifest_path in manifest_paths {
                let manifest = load_manifest(&manifest_path)?;
                let wasm_path = manifest_path.with_file_name("plugin.wasm");
                if !wasm_path.is_file() {
                    return Err(OberonError::ManifestValidation(format!(
                        "{}: missing plugin.wasm adjacent to plugin.json",
                        manifest_path.display()
                    )));
                }
                insert_registered_plugin(
                    &mut plugins,
                    &mut index,
                    RegisteredPlugin {
                        manifest_path,
                        wasm_path,
                        manifest,
                        tool_tier: ToolTier::OrdinaryPlugin,
                    },
                    runtime_api_version,
                    true,
                )?;
            }
        }

        Ok(Self {
            runtime_api_version: runtime_api_version.to_string(),
            plugins,
            index,
        })
    }

    pub fn get_registered_plugin(&self, tool: &str) -> Result<&RegisteredPlugin, OberonError> {
        self.plugins
            .get(tool)
            .ok_or_else(|| OberonError::NotFound(format!("plugin:{tool}")))
    }
}

pub trait RegisteredPluginAccess: PluginRegistry {
    fn registered_plugin(&self, tool: &str) -> Result<RegisteredPlugin, OberonError>;
}

impl PluginRegistry for FilePluginRegistry {
    fn list_plugins(&self) -> Vec<PluginDescription> {
        self.plugins
            .values()
            .map(|registered| PluginDescription {
                id: registered.manifest.id.clone(),
                version: registered.manifest.version.clone(),
                display_name: registered.manifest.display_name.clone(),
                description: registered.manifest.description.clone(),
            })
            .collect()
    }

    fn search(&self, request: &SearchRequest) -> Result<SearchResponse, OberonError> {
        let query = request.query.trim().to_lowercase();
        if query.is_empty() {
            return Err(OberonError::Validation(
                "tool_search query must not be empty".to_string(),
            ));
        }

        let tokens = query
            .split_whitespace()
            .filter(|token| !token.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        let mut matches = self
            .index
            .iter()
            .filter_map(|entry| {
                score_entry(&query, &tokens, entry).map(|score| SearchMatch {
                    tool: entry.tool.clone(),
                    command: entry.command.clone(),
                    summary: entry.summary.clone(),
                    tool_tier: entry.tool_tier.clone(),
                    side_effects: entry.side_effects.clone(),
                    approval_required: approval_required_for(
                        &entry.side_effects,
                        entry.approval_expectation.as_ref(),
                    ),
                    capabilities_required: entry.capabilities_required.clone(),
                    confidence: format!("{:.2}", (score.min(100) as f32) / 100.0),
                })
            })
            .collect::<Vec<_>>();

        matches.sort_by(|left, right| {
            right
                .confidence
                .cmp(&left.confidence)
                .then_with(|| left.tool.cmp(&right.tool))
                .then_with(|| left.command.cmp(&right.command))
        });

        if let Some(limit) = request.limit {
            matches.truncate(limit as usize);
        }

        Ok(SearchResponse { matches })
    }

    fn describe(&self, request: &ToolDescribeRequest) -> Result<ToolDescribeResponse, OberonError> {
        let manifest = self.manifest(&request.tool)?;
        let command = self.command_manifest(&request.tool, &request.command)?;

        Ok(ToolDescribeResponse {
            tool: manifest.id.clone(),
            command: command.name.clone(),
            summary: command.description.clone(),
            input_schema: oberon4_core::CommandSchemaReference {
                id: format!("{}.{}.input", manifest.id, command.name),
                schema: command.input_schema.clone(),
            },
            output_schema: oberon4_core::CommandSchemaReference {
                id: format!("{}.{}.output", manifest.id, command.name),
                schema: command.output_schema.clone(),
            },
            side_effects: command.side_effects.clone(),
            approval_expectation: command
                .approval_expectation
                .clone()
                .unwrap_or(ApprovalExpectation::None),
            capabilities_required: command.required_capabilities.clone(),
            pagination_behavior: command.supports_pagination.then(|| "supported".to_string()),
            artifact_rules: command
                .supports_artifacts
                .then(|| "artifacts_supported".to_string()),
            guidance: ToolGuidance {
                when_to_use: manifest.prompting.when_to_use,
                when_not_to_use: manifest.prompting.when_not_to_use,
                examples: manifest
                    .prompting
                    .examples
                    .into_iter()
                    .filter(|example| example.command == command.name)
                    .map(|example| example.intent)
                    .collect(),
                common_mistakes: match command.precondition_model {
                    Some(PreconditionModel::WorkspaceRevision) => {
                        vec!["Reusing a stale workspace revision".to_string()]
                    }
                    Some(PreconditionModel::FileHash) => {
                        vec!["Skipping the expected file hash precondition".to_string()]
                    }
                    Some(PreconditionModel::PatchContext) => {
                        vec!["Applying a patch without current context".to_string()]
                    }
                    _ => Vec::new(),
                },
                failure_recovery: manifest.prompting.failure_recovery,
            },
        })
    }

    fn manifest(&self, tool: &str) -> Result<PluginManifest, OberonError> {
        Ok(self.get_registered_plugin(tool)?.manifest.clone())
    }

    fn command_manifest(
        &self,
        tool: &str,
        command: &str,
    ) -> Result<PluginCommandManifest, OberonError> {
        let registered = self.get_registered_plugin(tool)?;
        registered
            .manifest
            .commands
            .iter()
            .find(|entry| entry.name == command)
            .cloned()
            .ok_or_else(|| OberonError::NotFound(format!("plugin_command:{tool}.{command}")))
    }
}

impl RegisteredPluginAccess for FilePluginRegistry {
    fn registered_plugin(&self, tool: &str) -> Result<RegisteredPlugin, OberonError> {
        self.get_registered_plugin(tool).cloned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticPolicyResolver {
    operating_mode: OperatingMode,
    default_limits: ResourceLimits,
    blocked_capabilities: BTreeSet<String>,
}

impl Default for StaticPolicyResolver {
    fn default() -> Self {
        Self {
            operating_mode: OperatingMode::Production,
            default_limits: ResourceLimits {
                memory_bytes: Some(134_217_728),
                time_ms: Some(5_000),
                storage_bytes: Some(268_435_456),
                output_bytes: Some(131_072),
                network_policy: Some(NetworkPolicy::Denied),
            },
            blocked_capabilities: BTreeSet::new(),
        }
    }
}

impl StaticPolicyResolver {
    pub fn new(
        operating_mode: OperatingMode,
        default_limits: ResourceLimits,
        blocked_capabilities: BTreeSet<String>,
    ) -> Self {
        Self {
            operating_mode,
            default_limits,
            blocked_capabilities,
        }
    }
}

impl PolicyResolver for StaticPolicyResolver {
    fn resolve(
        &self,
        tool: &str,
        command: &PluginCommandManifest,
        manifest: &PluginManifest,
    ) -> Result<PolicyResolution, OberonError> {
        let mut allowed = Vec::new();
        let mut blocked = Vec::new();
        let mut reason_codes = Vec::new();

        for capability in &command.required_capabilities {
            if self.blocked_capabilities.contains(capability) {
                blocked.push(capability.clone());
                reason_codes.push(format!("blocked_capability:{capability}"));
            } else {
                allowed.push(capability.clone());
            }
        }

        let approval_required =
            approval_required_for(&command.side_effects, command.approval_expectation.as_ref());

        let resource_limits = merge_resource_limits(
            &self.default_limits,
            manifest.requested_resources.as_ref(),
            command.requested_resources.as_ref(),
        );

        Ok(PolicyResolution {
            tool: tool.to_string(),
            command: command.name.clone(),
            capabilities: EffectiveCapabilities { allowed, blocked },
            approval_required,
            resource_limits,
            operating_mode: self.operating_mode.clone(),
            reason_codes,
            blocking_issues: Vec::new(),
        })
    }
}

pub struct PluginRuntimeHost<R, P> {
    registry: R,
    policy: P,
    broker_foundations: RuntimeBrokerFoundations,
    execution: ExecutionEngine,
    _secret_broker: Arc<dyn SecretBroker + Send + Sync>,
    service_broker: Arc<dyn ServiceBroker + Send + Sync>,
    redactor: Arc<dyn RedactionPipeline + Send + Sync>,
    workspace_broker: Arc<PluginRuntimeWorkspaceBroker>,
    artifact_broker: Arc<PluginRuntimeArtifactBroker>,
    versioned_package: Arc<PluginRuntimeVersionedPackage>,
}

impl<R, P> PluginRuntimeHost<R, P> {
    pub fn new(registry: R, policy: P) -> Result<Self, OberonError> {
        Self::with_foundations(registry, policy, RuntimeBrokerFoundations::default())
    }

    pub fn with_foundations(
        registry: R,
        policy: P,
        broker_foundations: RuntimeBrokerFoundations,
    ) -> Result<Self, OberonError> {
        let secret_broker = Arc::new(PluginRuntimeSecretBroker::new(
            NativeSecretBackendUnavailable,
            Vec::new(),
        ));
        let service_broker = Arc::new(PluginRuntimeServiceBroker::new(
            PluginRuntimeSecretBroker::new(NativeSecretBackendUnavailable, Vec::new()),
            Vec::new(),
            InMemoryServiceTransport::default(),
        ));
        Self::with_runtime_services(
            registry,
            policy,
            broker_foundations,
            secret_broker,
            service_broker,
            Arc::new(PluginRuntimeRedactor),
        )
    }

    pub fn with_runtime_services(
        registry: R,
        policy: P,
        broker_foundations: RuntimeBrokerFoundations,
        secret_broker: Arc<dyn SecretBroker + Send + Sync>,
        service_broker: Arc<dyn ServiceBroker + Send + Sync>,
        redactor: Arc<dyn RedactionPipeline + Send + Sync>,
    ) -> Result<Self, OberonError> {
        let data_root = oberon4_core::default_data_dir().join("plugin-runtime");
        let workspace_broker = Arc::new(PluginRuntimeWorkspaceBroker::new(
            data_root.join("workspaces"),
        )?);
        let artifact_broker = Arc::new(PluginRuntimeArtifactBroker::new(
            data_root.join("artifacts"),
            ArtifactRetentionPolicy {
                max_artifact_bytes: broker_foundations.artifact.max_artifact_bytes,
                max_retained_bytes: broker_foundations.artifact.retained_artifact_bytes,
                session_ttl_seconds: 3600,
                short_term_ttl_seconds: 86_400,
            },
        )?);
        Self::with_runtime_infrastructure(
            registry,
            policy,
            broker_foundations,
            secret_broker,
            service_broker,
            redactor,
            workspace_broker,
            artifact_broker,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_runtime_infrastructure(
        registry: R,
        policy: P,
        broker_foundations: RuntimeBrokerFoundations,
        secret_broker: Arc<dyn SecretBroker + Send + Sync>,
        service_broker: Arc<dyn ServiceBroker + Send + Sync>,
        redactor: Arc<dyn RedactionPipeline + Send + Sync>,
        workspace_broker: Arc<PluginRuntimeWorkspaceBroker>,
        artifact_broker: Arc<PluginRuntimeArtifactBroker>,
    ) -> Result<Self, OberonError> {
        let execution = ExecutionEngine::new(broker_foundations.clone())?;
        let versioned_package = Arc::new(PluginRuntimeVersionedPackage::new(
            oberon4_core::default_data_dir()
                .join("plugin-runtime")
                .join("versioned-state"),
            workspace_broker.clone(),
            artifact_broker.clone(),
            service_broker.clone(),
            broker_foundations.artifact.inline_output_threshold_bytes,
        )?);
        Ok(Self {
            registry,
            policy,
            broker_foundations,
            execution,
            _secret_broker: secret_broker,
            service_broker,
            redactor,
            workspace_broker,
            artifact_broker,
            versioned_package,
        })
    }

    pub fn broker_foundations(&self) -> &RuntimeBrokerFoundations {
        &self.broker_foundations
    }
}

impl<R, P> PluginRuntimeHost<R, P>
where
    R: PluginRegistry,
    P: PolicyResolver,
{
    fn is_builtin_tool(tool: &str) -> bool {
        PluginRuntimeVersionedPackage::is_builtin_tool(tool)
    }

    fn service_handle_from_input(input: &serde_json::Value) -> Option<String> {
        input
            .get("serviceHandle")
            .and_then(|value| value.as_str())
            .map(str::to_string)
    }

    fn workspace_handle_from_context(
        context: &oberon4_core::InvocationContext,
    ) -> Option<WorkspaceHandle> {
        context
            .capability_grants
            .iter()
            .find(|grant| {
                matches!(
                    grant.kind,
                    oberon4_core::CapabilityGrantKind::WorkspaceHandle
                )
            })
            .map(|grant| WorkspaceHandle {
                workspace_handle: grant.id.clone(),
            })
    }

    fn command_uses_workspace(command: &PluginCommandManifest) -> bool {
        command
            .required_capabilities
            .iter()
            .any(|capability| matches!(capability.as_str(), "workspace.read" | "workspace.write"))
    }

    fn command_requires_existing_workspace_handle(
        tool: &str,
        command: &PluginCommandManifest,
    ) -> bool {
        !(tool == "github" && command.name == "clone_to_workspace")
            && Self::command_uses_workspace(command)
    }

    fn command_uses_artifacts(command: &PluginCommandManifest) -> bool {
        command
            .required_capabilities
            .iter()
            .any(|capability| matches!(capability.as_str(), "artifact.read" | "artifact.create"))
            || command.supports_artifacts
    }

    fn validate_workspace_access(
        &self,
        command: &PluginCommandManifest,
        context: &oberon4_core::InvocationContext,
    ) -> Result<Option<oberon4_core::WorkspaceMetadata>, OberonError> {
        if !Self::command_requires_existing_workspace_handle(&context.plugin_id, command) {
            return Ok(None);
        }
        let workspace = Self::workspace_handle_from_context(context).ok_or_else(|| {
            OberonError::PolicyDenied(
                "workspace-capable commands require a granted workspace handle".to_string(),
            )
        })?;
        self.workspace_broker
            .create_or_attach_workspace(&workspace)
            .map(Some)
    }

    fn service_metadata_from_request(
        &self,
        command: &PluginCommandManifest,
        input: &serde_json::Value,
    ) -> Result<ServicePolicyMetadata, OberonError> {
        let Some(service_handle) = Self::service_handle_from_input(input) else {
            return Ok((Vec::new(), inferred_secret_classes(command), Vec::new()));
        };
        let metadata = self.service_broker.resolve_service(&service_handle)?;
        Ok((
            vec![metadata.service_handle],
            metadata.secret_class.into_iter().collect(),
            metadata.security_level.into_iter().collect(),
        ))
    }

    fn redaction_profile_for_input(
        &self,
        input: &serde_json::Value,
    ) -> Result<RedactionProfile, OberonError> {
        let mut exact_values = Vec::new();
        if let Some(service_handle) = Self::service_handle_from_input(input) {
            if let Ok(values) = self
                .service_broker
                .redaction_values_for_service(&service_handle)
            {
                exact_values.extend(values);
            }
        }
        Ok(RedactionProfile {
            exact_values,
            fail_closed: true,
        })
    }

    fn redact_tool_error(
        &self,
        mut error: ToolError,
        profile: &RedactionProfile,
    ) -> Result<ToolError, OberonError> {
        let redacted = self.redactor.redact_text(
            RedactionTargetKind::ErrorPayload,
            &error.message,
            profile,
        )?;
        if redacted.blocked {
            error.code = "REDACTION_BLOCKED_OUTPUT".to_string();
            error.message =
                "runtime output was blocked because it could not be safely redacted".to_string();
            error.retryable = false;
        } else {
            error.message = redacted.redacted_text;
        }
        Ok(error)
    }

    fn redact_tool_result(
        &self,
        summary: &str,
        data: &serde_json::Value,
        profile: &RedactionProfile,
    ) -> Result<(String, serde_json::Value), OberonError> {
        let summary_result =
            self.redactor
                .redact_text(RedactionTargetKind::ToolResult, summary, profile)?;
        let (data_value, data_result) =
            self.redactor
                .redact_json(RedactionTargetKind::ToolResult, data, profile)?;
        if summary_result.blocked || data_result.blocked {
            return Err(OberonError::PolicyDenied(
                "runtime output was blocked because it could not be safely redacted".to_string(),
            ));
        }
        Ok((summary_result.redacted_text, data_value))
    }

    fn brokered_service_result(
        &self,
        command: &PluginCommandManifest,
        input: &serde_json::Value,
    ) -> Result<Option<BrokeredServiceResult>, OberonError> {
        let Some(service_name) = brokered_service_kind(command) else {
            return Ok(None);
        };
        let service_handle = Self::service_handle_from_input(input).ok_or_else(|| {
            OberonError::PolicyDenied(
                "brokered service commands require serviceHandle input".to_string(),
            )
        })?;
        match service_name {
            "github.request" => self
                .service_broker
                .execute_github_request(&GithubRequest {
                    service_handle,
                    method: input
                        .get("method")
                        .and_then(|value| value.as_str())
                        .unwrap_or("GET")
                        .to_string(),
                    path: input
                        .get("path")
                        .and_then(|value| value.as_str())
                        .unwrap_or("/")
                        .to_string(),
                    headers: headers_from_input(input),
                    body_json: input.get("bodyJson").cloned(),
                })
                .map(|response| BrokeredServiceResult {
                    summary: format!("github.request {}", response.status_code),
                    data: serde_json::json!({
                        "statusCode": response.status_code,
                        "headers": response.headers,
                        "body": response.body,
                    }),
                    next_suggested_action: None,
                })
                .map(Some),
            "oauth.request" => self
                .service_broker
                .execute_oauth_request(&OauthRequest {
                    service_handle,
                    method: input
                        .get("method")
                        .and_then(|value| value.as_str())
                        .unwrap_or("GET")
                        .to_string(),
                    path: input
                        .get("path")
                        .and_then(|value| value.as_str())
                        .unwrap_or("/")
                        .to_string(),
                    headers: headers_from_input(input),
                    body_json: input.get("bodyJson").cloned(),
                })
                .map(|response| BrokeredServiceResult {
                    summary: format!("oauth.request {}", response.status_code),
                    data: serde_json::json!({
                        "statusCode": response.status_code,
                        "headers": response.headers,
                        "body": response.body,
                    }),
                    next_suggested_action: None,
                })
                .map(Some),
            "ssh.sign" => Err(OberonError::PolicyDenied(
                "ssh signing is not configured in WP3".to_string(),
            )),
            "wallet.sign" => Err(OberonError::PolicyDenied(
                "wallet signing is not configured in WP3".to_string(),
            )),
            _ => Ok(None),
        }
    }

    pub fn approval_payload_for_plan(
        &self,
        request: &PlanRequest,
    ) -> Result<Option<ApprovalRequestPayload>, OberonError> {
        let manifest = self.registry.manifest(&request.context.plugin_id)?;
        let command = self
            .registry
            .command_manifest(&request.context.plugin_id, &request.command)?;
        let resolution = self
            .policy
            .resolve(&request.context.plugin_id, &command, &manifest)?;

        if !resolution.approval_required {
            return Ok(None);
        }

        let (service_handles_used, secret_classes_involved, _) =
            self.service_metadata_from_request(&command, &request.input)?;

        Ok(Some(ApprovalRequestPayload {
            approval_id: format!(
                "appr_{}_{}",
                request.context.request_id.as_str(),
                request.command
            ),
            tool: manifest.id,
            command: command.name.clone(),
            summary: format!("Approve {}.{}", request.context.plugin_id, command.name),
            side_effects: command.side_effects,
            workspace_impact: workspace_impact_from_plan(request),
            remote_impact: None,
            artifacts: Vec::new(),
            service_handles_used,
            secret_classes_involved,
            resource_limits: ApprovalResourceLimits {
                time_ms: resolution.resource_limits.time_ms,
                memory_bytes: resolution.resource_limits.memory_bytes,
            },
            diff_summary: None,
        }))
    }
}

impl<R, P> HostToolRuntime for PluginRuntimeHost<R, P>
where
    R: PluginRegistry + RegisteredPluginAccess,
    P: PolicyResolver,
{
    fn tool_search(&self, request: &SearchRequest) -> Result<SearchResponse, OberonError> {
        self.registry.search(request)
    }

    fn tool_describe(
        &self,
        request: &ToolDescribeRequest,
    ) -> Result<ToolDescribeResponse, OberonError> {
        self.registry.describe(request)
    }

    fn tool_plan(&self, request: &PlanRequest) -> Result<PlanResult, OberonError> {
        if request.command != request.context.command {
            return Err(OberonError::ContractMismatch(format!(
                "request command '{}' does not match invocation context command '{}'",
                request.command, request.context.command
            )));
        }

        let manifest = self.registry.manifest(&request.context.plugin_id)?;
        let command = self
            .registry
            .command_manifest(&request.context.plugin_id, &request.command)?;
        validate_request_shape(&command, request.input.as_object().is_some())?;
        let resolution = self
            .policy
            .resolve(&request.context.plugin_id, &command, &manifest)?;
        let effective_request = PlanRequest {
            command: request.command.clone(),
            input: request.input.clone(),
            context: apply_policy_to_context(&request.context, &resolution.resource_limits),
        };
        let unsupported = unsupported_capabilities_after_wp4(&command);
        let approval_preview = self
            .approval_payload_for_plan(&effective_request)?
            .map(|payload| oberon4_core::CommandPlanPreview {
                summary: payload.summary,
                workspace_impact: payload.workspace_impact,
            });

        let mut blocking_issues = resolution
            .capabilities
            .blocked
            .iter()
            .map(|capability| format!("blocked capability: {capability}"))
            .collect::<Vec<_>>();
        blocking_issues.extend(resolution.blocking_issues);
        if !unsupported.is_empty() {
            blocking_issues.push(format!(
                "WP4 does not support required capabilities: {}",
                unsupported.join(", ")
            ));
        }
        if brokered_service_kind(&command).is_some()
            && Self::service_handle_from_input(&request.input).is_none()
        {
            blocking_issues
                .push("brokered service commands require serviceHandle input".to_string());
        }
        if Self::command_uses_workspace(&command) {
            match self.validate_workspace_access(&command, &effective_request.context) {
                Ok(Some(metadata)) => {
                    if matches!(
                        command.precondition_model,
                        Some(PreconditionModel::WorkspaceRevision)
                    ) && effective_request.context.workspace_revision.is_none()
                    {
                        blocking_issues.push(format!(
                            "workspace writes require current workspace revision for {}",
                            metadata.workspace_handle
                        ));
                    }
                }
                Ok(None) => {}
                Err(err) => blocking_issues.push(err.to_string()),
            }
        }

        if !resolution.capabilities.blocked.is_empty() || !unsupported.is_empty() {
            return Ok(PlanResult {
                status: PlanStatus::Ok,
                side_effects: command.side_effects.clone(),
                approval_required: resolution.approval_required,
                capabilities_required: command.required_capabilities.clone(),
                estimated_cost: estimate_cost(&command.side_effects),
                estimated_steps: 0,
                preconditions: preconditions_for(&command),
                blocking_issues,
                resource_limits: Some(resolution.resource_limits),
                approval_preview,
            });
        }

        if Self::is_builtin_tool(&request.context.plugin_id) {
            let builtin = self.versioned_package.plan(
                &request.context.plugin_id,
                &command,
                &effective_request,
            )?;
            blocking_issues.extend(builtin.blocking_issues);
            return Ok(PlanResult {
                status: PlanStatus::Ok,
                side_effects: command.side_effects.clone(),
                approval_required: resolution.approval_required,
                capabilities_required: command.required_capabilities.clone(),
                estimated_cost: estimate_cost(&command.side_effects),
                estimated_steps: builtin.estimated_steps,
                preconditions: preconditions_for(&command),
                blocking_issues,
                resource_limits: Some(resolution.resource_limits),
                approval_preview: builtin.approval_preview.or(approval_preview),
            });
        }

        if brokered_service_kind(&command).is_some() {
            return Ok(PlanResult {
                status: PlanStatus::Ok,
                side_effects: command.side_effects.clone(),
                approval_required: resolution.approval_required,
                capabilities_required: command.required_capabilities.clone(),
                estimated_cost: estimate_cost(&command.side_effects),
                estimated_steps: 1,
                preconditions: preconditions_for(&command),
                blocking_issues,
                resource_limits: Some(resolution.resource_limits),
                approval_preview,
            });
        }

        let registered = self
            .registry
            .registered_plugin(&request.context.plugin_id)?;
        let plugin_plan = if command.supports_planning {
            Some(
                self.execution
                    .plan(
                        &registered,
                        &command,
                        &effective_request,
                        self.workspace_broker.as_ref(),
                    )
                    .map_err(map_tool_error_to_oberon_error)?,
            )
        } else {
            None
        };
        if let Some(plugin_plan) = plugin_plan.as_ref() {
            validate_plugin_plan_contract(plugin_plan, &command, &effective_request)?;
            blocking_issues.extend(plugin_plan.blocking_issues.clone());
        }

        Ok(PlanResult {
            status: PlanStatus::Ok,
            side_effects: command.side_effects.clone(),
            approval_required: resolution.approval_required,
            capabilities_required: command.required_capabilities.clone(),
            estimated_cost: plugin_plan
                .as_ref()
                .map(|plan| plan.estimated_cost.clone())
                .unwrap_or_else(|| estimate_cost(&command.side_effects)),
            estimated_steps: plugin_plan
                .as_ref()
                .map(|plan| plan.estimated_steps)
                .unwrap_or(1),
            preconditions: plugin_plan
                .as_ref()
                .map(|plan| plan.preconditions.clone())
                .unwrap_or_else(|| preconditions_for(&command)),
            blocking_issues,
            resource_limits: Some(resolution.resource_limits),
            approval_preview,
        })
    }

    fn tool_invoke(
        &self,
        request: &oberon4_core::InvokeRequest,
    ) -> Result<ToolInvokeResponseEnvelope, OberonError> {
        if request.command != request.context.command {
            return Err(OberonError::ContractMismatch(format!(
                "request command '{}' does not match invocation context command '{}'",
                request.command, request.context.command
            )));
        }

        let manifest = self.registry.manifest(&request.context.plugin_id)?;
        let command = self
            .registry
            .command_manifest(&request.context.plugin_id, &request.command)?;
        validate_request_shape(&command, request.input.as_object().is_some())?;
        validate_output_schema_shape(&command)?;
        let resolution = self
            .policy
            .resolve(&request.context.plugin_id, &command, &manifest)?;
        let effective_request = oberon4_core::InvokeRequest {
            command: request.command.clone(),
            input: request.input.clone(),
            context: apply_policy_to_context(&request.context, &resolution.resource_limits),
        };
        let unsupported = unsupported_capabilities_after_wp4(&command);

        if !resolution.capabilities.blocked.is_empty() {
            let error = self.redact_tool_error(
                ToolError {
                    status: ErrorStatus::Error,
                    code: "CAPABILITY_DENIED".to_string(),
                    message: format!(
                        "blocked capabilities: {}",
                        resolution.capabilities.blocked.join(", ")
                    ),
                    retryable: false,
                    request_id: request.context.request_id.clone(),
                    plugin_id: request.context.plugin_id.clone(),
                    plugin_version: request.context.plugin_version.clone(),
                    command: request.command.clone(),
                    next_suggested_action: Some("tool_policy_inspect".to_string()),
                },
                &self.redaction_profile_for_input(&request.input)?,
            )?;
            return Ok(ToolInvokeResponseEnvelope {
                result: None,
                error: Some(error),
            });
        }
        if !unsupported.is_empty() {
            let error = self.redact_tool_error(
                ToolError {
                    status: ErrorStatus::Error,
                    code: "NOT_YET_SUPPORTED_IN_WP2".to_string(),
                    message: format!(
                        "WP4 does not support required capabilities: {}",
                        unsupported.join(", ")
                    ),
                    retryable: false,
                    request_id: request.context.request_id.clone(),
                    plugin_id: request.context.plugin_id.clone(),
                    plugin_version: request.context.plugin_version.clone(),
                    command: request.command.clone(),
                    next_suggested_action: Some("tool_policy_inspect".to_string()),
                },
                &self.redaction_profile_for_input(&request.input)?,
            )?;
            return Ok(ToolInvokeResponseEnvelope {
                result: None,
                error: Some(error),
            });
        }

        if let Err(err) = self.validate_workspace_access(&command, &effective_request.context) {
            let error = self.redact_tool_error(
                ToolError {
                    status: ErrorStatus::Error,
                    code: "CAPABILITY_DENIED".to_string(),
                    message: err.to_string(),
                    retryable: false,
                    request_id: request.context.request_id.clone(),
                    plugin_id: request.context.plugin_id.clone(),
                    plugin_version: request.context.plugin_version.clone(),
                    command: request.command.clone(),
                    next_suggested_action: Some("tool_policy_inspect".to_string()),
                },
                &self.redaction_profile_for_input(&request.input)?,
            )?;
            return Ok(ToolInvokeResponseEnvelope {
                result: None,
                error: Some(error),
            });
        }

        if Self::is_builtin_tool(&request.context.plugin_id) {
            let profile = self.redaction_profile_for_input(&request.input)?;
            return match self.versioned_package.invoke(
                &request.context.plugin_id,
                &command,
                &effective_request,
            ) {
                Ok(result) => match result.result {
                    Some(tool_result) => {
                        let (summary, data) = match self.redact_tool_result(
                            &tool_result.summary,
                            &tool_result.data,
                            &profile,
                        ) {
                            Ok(result) => result,
                            Err(_) => {
                                return Ok(ToolInvokeResponseEnvelope {
                                    result: None,
                                    error: Some(ToolError {
                                        status: ErrorStatus::Error,
                                        code: "REDACTION_BLOCKED_OUTPUT".to_string(),
                                        message: "runtime output was blocked because it could not be safely redacted".to_string(),
                                        retryable: false,
                                        request_id: request.context.request_id.clone(),
                                        plugin_id: request.context.plugin_id.clone(),
                                        plugin_version: request.context.plugin_version.clone(),
                                        command: request.command.clone(),
                                        next_suggested_action: Some("tool_policy_inspect".to_string()),
                                    }),
                                });
                            }
                        };
                        Ok(ToolInvokeResponseEnvelope {
                            result: Some(oberon4_core::ToolResult {
                                summary,
                                data,
                                ..tool_result
                            }),
                            error: None,
                        })
                    }
                    None => Ok(ToolInvokeResponseEnvelope {
                        result: None,
                        error: result
                            .error
                            .map(|error| self.redact_tool_error(error, &profile))
                            .transpose()?,
                    }),
                },
                Err(error) => Ok(ToolInvokeResponseEnvelope {
                    result: None,
                    error: Some(self.redact_tool_error(error, &profile)?),
                }),
            };
        }

        if brokered_service_kind(&command).is_some() {
            let profile = self.redaction_profile_for_input(&request.input)?;
            let invoke_result = match self.brokered_service_result(&command, &request.input) {
                Ok(Some(result)) => result,
                Ok(None) => {
                    return Ok(ToolInvokeResponseEnvelope {
                        result: None,
                        error: Some(self.redact_tool_error(
                            ToolError {
                                status: ErrorStatus::Error,
                                code: "SERVICE_REQUEST_FAILED".to_string(),
                                message: "brokered service resolution failed".to_string(),
                                retryable: false,
                                request_id: request.context.request_id.clone(),
                                plugin_id: request.context.plugin_id.clone(),
                                plugin_version: request.context.plugin_version.clone(),
                                command: request.command.clone(),
                                next_suggested_action: Some("tool_policy_inspect".to_string()),
                            },
                            &profile,
                        )?),
                    });
                }
                Err(error) => {
                    let mut error = broker_error_from_oberon_error(error, &command);
                    error.request_id = request.context.request_id.clone();
                    error.plugin_id = request.context.plugin_id.clone();
                    error.plugin_version = request.context.plugin_version.clone();
                    return Ok(ToolInvokeResponseEnvelope {
                        result: None,
                        error: Some(self.redact_tool_error(error, &profile)?),
                    });
                }
            };
            let (summary, data) = match self.redact_tool_result(
                &invoke_result.summary,
                &invoke_result.data,
                &profile,
            ) {
                Ok(result) => result,
                Err(_) => {
                    return Ok(ToolInvokeResponseEnvelope {
                        result: None,
                        error: Some(ToolError {
                            status: ErrorStatus::Error,
                            code: "REDACTION_BLOCKED_OUTPUT".to_string(),
                            message:
                                "runtime output was blocked because it could not be safely redacted"
                                    .to_string(),
                            retryable: false,
                            request_id: request.context.request_id.clone(),
                            plugin_id: request.context.plugin_id.clone(),
                            plugin_version: request.context.plugin_version.clone(),
                            command: request.command.clone(),
                            next_suggested_action: Some("tool_policy_inspect".to_string()),
                        }),
                    });
                }
            };
            return Ok(ToolInvokeResponseEnvelope {
                result: Some(oberon4_core::ToolResult {
                    status: InvokeStatus::Ok,
                    tool: request.context.plugin_id.clone(),
                    command: request.command.clone(),
                    summary,
                    data,
                    artifacts: Vec::new(),
                    truncated: false,
                    next_suggested_action: invoke_result.next_suggested_action,
                }),
                error: None,
            });
        }

        let registered = self
            .registry
            .registered_plugin(&request.context.plugin_id)?;
        let invoke_result = match self.execution.invoke(
            &registered,
            &command,
            &effective_request,
            self.workspace_broker.as_ref(),
            self.artifact_broker.as_ref(),
        ) {
            Ok(result) => result,
            Err(error) => {
                let profile = self.redaction_profile_for_input(&request.input)?;
                return Ok(ToolInvokeResponseEnvelope {
                    result: None,
                    error: Some(self.redact_tool_error(error, &profile)?),
                });
            }
        };

        let profile = self.redaction_profile_for_input(&request.input)?;
        let (summary, data) =
            match self.redact_tool_result(&invoke_result.summary, &invoke_result.data, &profile) {
                Ok(result) => result,
                Err(_) => {
                    return Ok(ToolInvokeResponseEnvelope {
                        result: None,
                        error: Some(ToolError {
                            status: ErrorStatus::Error,
                            code: "REDACTION_BLOCKED_OUTPUT".to_string(),
                            message:
                                "runtime output was blocked because it could not be safely redacted"
                                    .to_string(),
                            retryable: false,
                            request_id: request.context.request_id.clone(),
                            plugin_id: request.context.plugin_id.clone(),
                            plugin_version: request.context.plugin_version.clone(),
                            command: request.command.clone(),
                            next_suggested_action: Some("tool_policy_inspect".to_string()),
                        }),
                    });
                }
            };

        Ok(ToolInvokeResponseEnvelope {
            result: Some(oberon4_core::ToolResult {
                status: invoke_result.status,
                tool: request.context.plugin_id.clone(),
                command: request.command.clone(),
                summary,
                data,
                artifacts: invoke_result.artifacts,
                truncated: false,
                next_suggested_action: invoke_result.next_suggested_action,
            }),
            error: None,
        })
    }

    fn tool_policy_inspect(
        &self,
        request: &PolicyInspectRequest,
    ) -> Result<PolicyInspectResponse, OberonError> {
        let manifest = self.registry.manifest(&request.tool)?;
        let command = self
            .registry
            .command_manifest(&request.tool, &request.command)?;
        let resolution = self.policy.resolve(&request.tool, &command, &manifest)?;
        let workspace_allowed = Self::command_uses_workspace(&command);
        let artifact_allowed = Self::command_uses_artifacts(&command);
        let service_handles_used = service_capabilities(&command)
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        Ok(PolicyInspectResponse {
            status: PlanStatus::Ok,
            tool: request.tool.clone(),
            command: request.command.clone(),
            capabilities: resolution.capabilities,
            approval_required: resolution.approval_required,
            resource_limits: resolution.resource_limits,
            operating_mode: resolution.operating_mode,
            reason_codes: {
                let mut reason_codes = resolution.reason_codes;
                if workspace_allowed {
                    reason_codes.push("workspace_capability_declared".to_string());
                }
                if artifact_allowed {
                    reason_codes.push("artifact_capability_declared".to_string());
                }
                reason_codes
            },
            service_handles_used,
            secret_classes_involved: inferred_secret_classes(&command),
            security_levels: Vec::new(),
        })
    }
}

pub fn validate_manifest(
    manifest: &PluginManifest,
    runtime_api_version: &str,
) -> ManifestValidationResult {
    let mut issues = Vec::new();

    if manifest.runtime_api_version != runtime_api_version {
        issues.push(ManifestValidationIssue {
            code: "unsupported_runtime_api_version".to_string(),
            message: format!(
                "plugin targets runtime API version '{}' but host supports '{}'",
                manifest.runtime_api_version, runtime_api_version
            ),
            field: Some("runtimeApiVersion".to_string()),
        });
    }

    if manifest.commands.is_empty() {
        issues.push(ManifestValidationIssue {
            code: "missing_commands".to_string(),
            message: "plugin manifest must declare at least one command".to_string(),
            field: Some("commands".to_string()),
        });
    }

    let mut names = BTreeSet::new();
    let declared_capabilities = declared_capabilities(&manifest.capabilities);
    for (index, command) in manifest.commands.iter().enumerate() {
        if !names.insert(command.name.clone()) {
            issues.push(ManifestValidationIssue {
                code: "duplicate_command".to_string(),
                message: format!("duplicate command '{}'", command.name),
                field: Some(format!("commands[{index}].name")),
            });
        }

        if !command.input_schema.is_object() {
            issues.push(ManifestValidationIssue {
                code: "invalid_input_schema".to_string(),
                message: "command input schema must be a JSON object".to_string(),
                field: Some(format!("commands[{index}].inputSchema")),
            });
        }

        if !command.output_schema.is_object() {
            issues.push(ManifestValidationIssue {
                code: "invalid_output_schema".to_string(),
                message: "command output schema must be a JSON object".to_string(),
                field: Some(format!("commands[{index}].outputSchema")),
            });
        }

        for capability in &command.required_capabilities {
            if !declared_capabilities.contains(capability) {
                issues.push(ManifestValidationIssue {
                    code: "undeclared_capability".to_string(),
                    message: format!(
                        "command '{}' requires undeclared capability '{}'",
                        command.name, capability
                    ),
                    field: Some(format!("commands[{index}].requiredCapabilities")),
                });
            }
        }
    }

    if manifest.provenance.publisher.trim().is_empty()
        || manifest.provenance.source.trim().is_empty()
        || manifest.provenance.checksum.trim().is_empty()
    {
        issues.push(ManifestValidationIssue {
            code: "invalid_provenance".to_string(),
            message: "publisher, source, and checksum are required provenance scaffolding"
                .to_string(),
            field: Some("provenance".to_string()),
        });
    }

    ManifestValidationResult {
        valid: issues.is_empty(),
        issues,
    }
}

fn discover_manifest_paths(root: &Path) -> Result<Vec<PathBuf>, OberonError> {
    let mut manifests = Vec::new();
    if !root.exists() {
        return Ok(manifests);
    }

    let mut stack = vec![root.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(&current)
            .map_err(|err| OberonError::Storage(format!("read_dir failed: {err}")))?
        {
            let entry =
                entry.map_err(|err| OberonError::Storage(format!("dir entry failed: {err}")))?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().and_then(|name| name.to_str()) == Some("plugin.json") {
                manifests.push(path);
            }
        }
    }

    manifests.sort();
    Ok(manifests)
}

fn load_manifest(path: &Path) -> Result<PluginManifest, OberonError> {
    let raw = fs::read_to_string(path)
        .map_err(|err| OberonError::Storage(format!("manifest read failed: {err}")))?;
    serde_json::from_str(&raw)
        .map_err(|err| OberonError::ManifestValidation(format!("invalid plugin.json: {err}")))
}

fn insert_registered_plugin(
    plugins: &mut BTreeMap<String, RegisteredPlugin>,
    index: &mut Vec<IndexedCommand>,
    registered: RegisteredPlugin,
    runtime_api_version: &str,
    require_executable: bool,
) -> Result<(), OberonError> {
    let validation = validate_manifest(&registered.manifest, runtime_api_version);
    if !validation.valid {
        let issues = validation
            .issues
            .into_iter()
            .map(|issue| issue.message)
            .collect::<Vec<_>>()
            .join("; ");
        return Err(OberonError::ManifestValidation(format!(
            "{}: {}",
            registered.manifest_path.display(),
            issues
        )));
    }

    if require_executable && !registered.wasm_path.is_file() {
        return Err(OberonError::ManifestValidation(format!(
            "{}: missing plugin.wasm adjacent to plugin.json",
            registered.manifest_path.display()
        )));
    }

    let plugin_id = registered.manifest.id.clone();
    if plugins.contains_key(&plugin_id) {
        return Err(OberonError::ManifestValidation(format!(
            "duplicate plugin id '{}'",
            plugin_id
        )));
    }

    index.extend(
        registered
            .manifest
            .commands
            .iter()
            .map(|command| IndexedCommand {
                tool: plugin_id.clone(),
                command: command.name.clone(),
                summary: command.description.clone(),
                tool_tier: registered.tool_tier.clone(),
                tags: command.tags.clone(),
                side_effects: command.side_effects.clone(),
                capabilities_required: command.required_capabilities.clone(),
                approval_expectation: command.approval_expectation.clone(),
            }),
    );

    plugins.insert(plugin_id, registered);
    Ok(())
}

fn declared_capabilities(capabilities: &CapabilityManifest) -> BTreeSet<String> {
    let mut declared = BTreeSet::new();
    if capabilities.storage.private {
        declared.insert("storage.private".to_string());
    }
    if capabilities.storage.session {
        declared.insert("storage.session".to_string());
    }
    if capabilities.storage.cache {
        declared.insert("storage.cache".to_string());
    }
    if capabilities.workspace.read {
        declared.insert("workspace.read".to_string());
    }
    if capabilities.workspace.write {
        declared.insert("workspace.write".to_string());
    }
    if capabilities.artifacts.read {
        declared.insert("artifact.read".to_string());
    }
    if capabilities.artifacts.create {
        declared.insert("artifact.create".to_string());
    }
    declared.extend(capabilities.services.iter().cloned());
    declared
}

fn merge_resource_limits(
    defaults: &ResourceLimits,
    manifest: Option<&oberon4_core::ResourceBudgetRequest>,
    command: Option<&oberon4_core::ResourceBudgetRequest>,
) -> ResourceLimits {
    let memory_bytes = min_optional(
        defaults.memory_bytes,
        command
            .and_then(|requested| requested.memory_bytes)
            .or_else(|| manifest.and_then(|requested| requested.memory_bytes)),
    );
    let time_ms = min_optional(
        defaults.time_ms,
        command
            .and_then(|requested| requested.time_ms)
            .or_else(|| manifest.and_then(|requested| requested.time_ms)),
    );
    let storage_bytes = min_optional(
        defaults.storage_bytes,
        command
            .and_then(|requested| requested.storage_bytes)
            .or_else(|| manifest.and_then(|requested| requested.storage_bytes)),
    );
    let output_bytes = min_optional(
        defaults.output_bytes,
        command
            .and_then(|requested| requested.output_bytes)
            .or_else(|| manifest.and_then(|requested| requested.output_bytes)),
    );
    let requested_network = command
        .and_then(|requested| requested.network_policy.clone())
        .or_else(|| manifest.and_then(|requested| requested.network_policy.clone()));
    let network_policy = Some(stricter_network_policy(
        defaults
            .network_policy
            .clone()
            .unwrap_or(NetworkPolicy::Denied),
        requested_network.unwrap_or(NetworkPolicy::Denied),
    ));

    ResourceLimits {
        memory_bytes,
        time_ms,
        storage_bytes,
        output_bytes,
        network_policy,
    }
}

fn min_optional(default: Option<u64>, requested: Option<u64>) -> Option<u64> {
    match (default, requested) {
        (Some(default), Some(requested)) => Some(default.min(requested)),
        (Some(default), None) => Some(default),
        (None, Some(requested)) => Some(requested),
        (None, None) => None,
    }
}

fn stricter_network_policy(default: NetworkPolicy, requested: NetworkPolicy) -> NetworkPolicy {
    match (default, requested) {
        (NetworkPolicy::Denied, _) | (_, NetworkPolicy::Denied) => NetworkPolicy::Denied,
        (NetworkPolicy::BrokeredOnly, _) | (_, NetworkPolicy::BrokeredOnly) => {
            NetworkPolicy::BrokeredOnly
        }
        _ => NetworkPolicy::Allowlisted,
    }
}

fn approval_required_for(
    side_effects: &SideEffectClass,
    approval_expectation: Option<&ApprovalExpectation>,
) -> bool {
    if matches!(approval_expectation, Some(ApprovalExpectation::Required)) {
        return true;
    }

    matches!(
        side_effects,
        SideEffectClass::ExternalWrite | SideEffectClass::Destructive
    )
}

fn preconditions_for(command: &PluginCommandManifest) -> Vec<String> {
    match command.precondition_model {
        Some(PreconditionModel::WorkspaceRevision) => vec!["workspaceRevision".to_string()],
        Some(PreconditionModel::FileHash) => vec!["fileHash".to_string()],
        Some(PreconditionModel::PatchContext) => vec!["patchContext".to_string()],
        _ => Vec::new(),
    }
}

fn estimate_cost(side_effects: &SideEffectClass) -> oberon4_core::EstimatedCost {
    match side_effects {
        SideEffectClass::Destructive => oberon4_core::EstimatedCost::High,
        SideEffectClass::ExternalWrite => oberon4_core::EstimatedCost::Medium,
        _ => oberon4_core::EstimatedCost::Low,
    }
}

fn validate_request_shape(
    command: &PluginCommandManifest,
    input_is_object: bool,
) -> Result<(), OberonError> {
    if command
        .input_schema
        .get("type")
        .and_then(|value| value.as_str())
        == Some("object")
        && !input_is_object
    {
        return Err(OberonError::ContractMismatch(format!(
            "command '{}.{}' expects object-shaped input",
            command.name, command.version
        )));
    }
    Ok(())
}

fn validate_output_schema_shape(command: &PluginCommandManifest) -> Result<(), OberonError> {
    if !command.output_schema.is_object() {
        return Err(OberonError::ContractMismatch(format!(
            "command '{}.{}' output schema must be a JSON object",
            command.name, command.version
        )));
    }
    Ok(())
}

fn validate_plugin_plan_contract(
    plugin_plan: &PlanResult,
    command: &PluginCommandManifest,
    request: &PlanRequest,
) -> Result<(), OberonError> {
    if plugin_plan.side_effects != command.side_effects {
        return Err(OberonError::ContractMismatch(format!(
            "plugin plan side effects for '{}.{}' did not match manifest declaration in request {}",
            request.context.plugin_id,
            request.command,
            request.context.request_id.as_str()
        )));
    }
    if plugin_plan.capabilities_required != command.required_capabilities {
        return Err(OberonError::ContractMismatch(format!(
            "plugin plan capabilities for '{}.{}' did not match manifest declaration in request {}",
            request.context.plugin_id,
            request.command,
            request.context.request_id.as_str()
        )));
    }
    Ok(())
}

fn map_tool_error_to_oberon_error(error: ToolError) -> OberonError {
    match error.code.as_str() {
        "CAPABILITY_DENIED" => OberonError::PolicyDenied(error.message),
        "SERVICE_ACCESS_DENIED" | "SECRET_ACCESS_DENIED" => {
            OberonError::PolicyDenied(error.message)
        }
        "RUNTIME_CONTRACT_MISMATCH" => OberonError::ContractMismatch(error.message),
        "NOT_YET_SUPPORTED_IN_WP2" => OberonError::Validation(error.message),
        "SERVICE_REQUEST_FAILED" | "REDACTION_BLOCKED_OUTPUT" => {
            OberonError::Storage(error.message)
        }
        "RUNTIME_COMPONENT_LOAD_FAILED"
        | "RESOURCE_LIMIT_TIMEOUT"
        | "RESOURCE_LIMIT_MEMORY"
        | "RESOURCE_LIMIT_OUTPUT"
        | "RESOURCE_LIMIT_STORAGE" => OberonError::Storage(error.message),
        _ => OberonError::Validation(error.message),
    }
}

fn apply_policy_to_context(
    context: &oberon4_core::InvocationContext,
    limits: &ResourceLimits,
) -> oberon4_core::InvocationContext {
    oberon4_core::InvocationContext {
        request_id: context.request_id.clone(),
        session_id: context.session_id.clone(),
        plugin_id: context.plugin_id.clone(),
        plugin_version: context.plugin_version.clone(),
        command: context.command.clone(),
        deadline: context.deadline.clone(),
        runtime_api_version: context.runtime_api_version.clone(),
        capability_grants: context.capability_grants.clone(),
        workspace_revision: context.workspace_revision.clone(),
        memory_limit: limits.memory_bytes.unwrap_or(context.memory_limit),
        time_limit_ms: limits.time_ms.unwrap_or(context.time_limit_ms),
        output_limit_bytes: limits.output_bytes.unwrap_or(context.output_limit_bytes),
        storage_limit_bytes: limits.storage_bytes.unwrap_or(context.storage_limit_bytes),
        network_policy: limits
            .network_policy
            .clone()
            .unwrap_or_else(|| context.network_policy.clone()),
    }
}

fn unsupported_capabilities_after_wp4(command: &PluginCommandManifest) -> Vec<String> {
    command
        .required_capabilities
        .iter()
        .filter(|capability| {
            !matches!(
                capability.as_str(),
                "storage.private"
                    | "storage.session"
                    | "storage.cache"
                    | "workspace.read"
                    | "workspace.write"
                    | "artifact.read"
                    | "artifact.create"
                    | "github.request"
                    | "service.github.request"
                    | "oauth.request"
                    | "service.oauth.request"
                    | "ssh.sign"
                    | "service.ssh.sign"
                    | "wallet.sign_tx"
                    | "service.wallet.sign_tx"
                    | "wallet.sign_message"
                    | "service.wallet.sign_message"
            )
        })
        .cloned()
        .collect()
}

fn brokered_service_kind(command: &PluginCommandManifest) -> Option<&'static str> {
    if command.required_capabilities.iter().any(|capability| {
        matches!(
            capability.as_str(),
            "github.request" | "service.github.request"
        )
    }) {
        Some("github.request")
    } else if command.required_capabilities.iter().any(|capability| {
        matches!(
            capability.as_str(),
            "oauth.request" | "service.oauth.request"
        )
    }) {
        Some("oauth.request")
    } else if command
        .required_capabilities
        .iter()
        .any(|capability| matches!(capability.as_str(), "ssh.sign" | "service.ssh.sign"))
    {
        Some("ssh.sign")
    } else if command.required_capabilities.iter().any(|capability| {
        matches!(
            capability.as_str(),
            "wallet.sign_tx"
                | "service.wallet.sign_tx"
                | "wallet.sign_message"
                | "service.wallet.sign_message"
        )
    }) {
        Some("wallet.sign")
    } else {
        None
    }
}

fn service_capabilities(command: &PluginCommandManifest) -> Vec<&str> {
    command
        .required_capabilities
        .iter()
        .filter_map(|capability| {
            matches!(
                capability.as_str(),
                "github.request"
                    | "service.github.request"
                    | "oauth.request"
                    | "service.oauth.request"
                    | "ssh.sign"
                    | "service.ssh.sign"
                    | "wallet.sign_tx"
                    | "service.wallet.sign_tx"
                    | "wallet.sign_message"
                    | "service.wallet.sign_message"
            )
            .then_some(capability.as_str())
        })
        .collect()
}

fn inferred_secret_classes(command: &PluginCommandManifest) -> Vec<SecretClass> {
    if brokered_service_kind(command).is_some() {
        vec![SecretClass::OauthCredential]
    } else {
        Vec::new()
    }
}

fn headers_from_input(input: &serde_json::Value) -> Vec<oberon4_core::HttpHeader> {
    input
        .get("headers")
        .and_then(|value| value.as_object())
        .map(|headers| {
            headers
                .iter()
                .filter_map(|(name, value)| {
                    value.as_str().map(|value| oberon4_core::HttpHeader {
                        name: name.clone(),
                        value: value.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn broker_error_from_oberon_error(err: OberonError, command: &PluginCommandManifest) -> ToolError {
    let (code, retryable) = match err {
        OberonError::PolicyDenied(_) => ("SERVICE_ACCESS_DENIED", false),
        OberonError::NotFound(_) => ("SERVICE_ACCESS_DENIED", false),
        _ => ("SERVICE_REQUEST_FAILED", true),
    };
    ToolError {
        status: ErrorStatus::Error,
        code: code.to_string(),
        message: err.to_string(),
        retryable,
        request_id: RequestId::new(),
        plugin_id: String::new(),
        plugin_version: String::new(),
        command: command.name.clone(),
        next_suggested_action: Some("tool_policy_inspect".to_string()),
    }
}

fn workspace_impact_from_plan(request: &PlanRequest) -> Option<WorkspaceImpact> {
    let workspace_handle = request
        .input
        .get("workspaceHandle")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .or_else(|| {
            request
                .context
                .capability_grants
                .iter()
                .find(|grant| {
                    matches!(
                        grant.kind,
                        oberon4_core::CapabilityGrantKind::WorkspaceHandle
                    )
                })
                .map(|grant| grant.id.clone())
        })?;
    Some(WorkspaceImpact {
        workspace_handle,
        workspace_revision: request
            .context
            .workspace_revision
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        touched_paths: request
            .input
            .get("path")
            .and_then(|value| value.as_str())
            .map(|path| vec![path.to_string()])
            .unwrap_or_default(),
    })
}

fn score_entry(query: &str, tokens: &[String], entry: &IndexedCommand) -> Option<u32> {
    let tool = entry.tool.to_lowercase();
    let command = entry.command.to_lowercase();
    let summary = entry.summary.to_lowercase();
    let tags = entry
        .tags
        .iter()
        .map(|tag| tag.to_lowercase())
        .collect::<Vec<_>>();

    let mut score = 0;
    if tool == query || command == query {
        score += 100;
    }

    for token in tokens {
        if command.contains(token) {
            score += 40;
        } else if tool.contains(token) {
            score += 30;
        } else if summary.contains(token) {
            score += 20;
        } else if tags.iter().any(|tag| tag.contains(token)) {
            score += 10;
        }
    }

    (score > 0).then_some(score)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        FilePluginRegistry, HOST_RUNTIME_API_VERSION, PluginRuntimeHost, RegisteredPlugin,
        RuntimeBrokerFoundations, StaticPolicyResolver, validate_manifest,
    };
    use oberon4_core::{
        ApprovalExpectation, ArtifactCapabilityManifest, CapabilityManifest, HostToolRuntime,
        InvocationContext, InvokeRequest, LifecycleActions, LifecycleHooks, NetworkPolicy,
        OperatingMode, PlanRequest, PluginCommandManifest, PluginExample, PluginManifest,
        PolicyInspectRequest, PromptingManifest, ProvenanceManifest, ResourceBudgetRequest,
        ResourceLimits, SearchRequest, SignatureMetadata, StorageCapabilityManifest,
        ToolDescribeRequest, WorkspaceCapabilityManifest,
    };
    use oberon4_core::{BuildMetadata, CapabilityGrant, CapabilityGrantKind, HandleOperation};
    use oberon4_core::{ManifestValidationResult, PluginRegistry, SideEffectClass};
    use oberon4_core::{OberonError, RequestId};

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("oberon4-plugin-runtime-{label}-{nonce}"))
    }

    fn sample_manifest() -> PluginManifest {
        PluginManifest {
            id: "workspace-git".to_string(),
            version: "0.1.0".to_string(),
            display_name: "Versioned Workspace".to_string(),
            description: "Workspace editing and versioned git operations.".to_string(),
            runtime_api_version: HOST_RUNTIME_API_VERSION.to_string(),
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
                supports_planning: false,
                supports_pagination: false,
                supports_artifacts: false,
                precondition_model: Some(oberon4_core::PreconditionModel::WorkspaceRevision),
                requested_resources: Some(ResourceBudgetRequest {
                    memory_bytes: Some(100_000_000),
                    time_ms: Some(4_000),
                    storage_bytes: None,
                    output_bytes: None,
                    artifact_bytes: None,
                    network_policy: Some(NetworkPolicy::Denied),
                }),
            }],
            capabilities: CapabilityManifest {
                storage: StorageCapabilityManifest {
                    private: true,
                    session: true,
                    cache: false,
                },
                workspace: WorkspaceCapabilityManifest {
                    read: true,
                    write: true,
                },
                artifacts: ArtifactCapabilityManifest {
                    read: true,
                    create: false,
                },
                services: vec!["github.request".to_string()],
            },
            prompting: PromptingManifest {
                prompt_version: "1".to_string(),
                summary: "Use for versioned workspace operations.".to_string(),
                when_to_use: "Use when reading or editing the workspace.".to_string(),
                when_not_to_use: "Do not use for remote-only metadata.".to_string(),
                examples: vec![PluginExample {
                    intent: "Update a file".to_string(),
                    command: "write_file_atomic".to_string(),
                }],
                failure_recovery: vec!["If blocked, re-plan.".to_string()],
                safety_notes: vec!["Writes may require approval.".to_string()],
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
                    on_install: false,
                    on_upgrade: false,
                    health_check: true,
                },
            }),
            requested_resources: Some(ResourceBudgetRequest {
                memory_bytes: Some(120_000_000),
                time_ms: Some(4_500),
                storage_bytes: Some(200_000_000),
                output_bytes: Some(100_000),
                artifact_bytes: Some(20_000_000),
                network_policy: Some(NetworkPolicy::Denied),
            }),
        }
    }

    fn write_manifest(root: &std::path::Path, manifest: &PluginManifest) {
        let plugin_dir = root.join(&manifest.id);
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(
            plugin_dir.join("plugin.json"),
            serde_json::to_vec_pretty(manifest).unwrap(),
        )
        .unwrap();
        fs::write(plugin_dir.join("plugin.wasm"), b"not-a-real-component").unwrap();
    }

    fn sample_context() -> InvocationContext {
        InvocationContext {
            request_id: RequestId::new(),
            session_id: "sess_123".to_string(),
            plugin_id: "workspace-git".to_string(),
            plugin_version: "0.1.0".to_string(),
            command: "write_file_atomic".to_string(),
            deadline: "2026-03-08T12:00:05Z".to_string(),
            runtime_api_version: HOST_RUNTIME_API_VERSION.to_string(),
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
        }
    }

    #[test]
    fn valid_manifest_discovery_and_search_work_without_execution() {
        let root = temp_dir("registry-valid");
        let manifest = sample_manifest();
        write_manifest(&root, &manifest);

        let registry =
            FilePluginRegistry::load_from_roots(&[root.clone()], HOST_RUNTIME_API_VERSION).unwrap();
        let plugins = registry.list_plugins();
        assert!(plugins.iter().any(|plugin| plugin.id == "workspace-git"));
        assert!(plugins.iter().any(|plugin| plugin.id == "workspace"));
        assert!(plugins.iter().any(|plugin| plugin.id == "git"));
        assert!(plugins.iter().any(|plugin| plugin.id == "github"));

        let results = registry
            .search(&SearchRequest {
                query: "write file".to_string(),
                limit: None,
                session_id: None,
            })
            .unwrap();
        assert!(results.matches.iter().any(|entry| {
            entry.tool == "workspace-git" && entry.command == "write_file_atomic"
        }));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn malformed_manifest_is_rejected() {
        let root = temp_dir("registry-malformed");
        let plugin_dir = root.join("broken");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(plugin_dir.join("plugin.json"), b"{not-json").unwrap();

        let error = FilePluginRegistry::load_from_roots(&[root.clone()], HOST_RUNTIME_API_VERSION)
            .unwrap_err();
        assert!(matches!(error, OberonError::ManifestValidation(_)));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn duplicate_command_names_are_rejected() {
        let mut manifest = sample_manifest();
        manifest.commands.push(manifest.commands[0].clone());
        let validation = validate_manifest(&manifest, HOST_RUNTIME_API_VERSION);
        assert!(!validation.valid);
        assert!(
            validation
                .issues
                .iter()
                .any(|issue| issue.code == "duplicate_command")
        );
    }

    #[test]
    fn unsupported_runtime_api_is_rejected() {
        let mut manifest = sample_manifest();
        manifest.runtime_api_version = "99.0".to_string();
        let validation = validate_manifest(&manifest, HOST_RUNTIME_API_VERSION);
        assert!(!validation.valid);
        assert!(
            validation
                .issues
                .iter()
                .any(|issue| issue.code == "unsupported_runtime_api_version")
        );
    }

    #[test]
    fn tool_describe_returns_schema_and_guidance() {
        let root = temp_dir("describe");
        let manifest = sample_manifest();
        write_manifest(&root, &manifest);
        let registry =
            FilePluginRegistry::load_from_roots(&[root.clone()], HOST_RUNTIME_API_VERSION).unwrap();

        let response = registry
            .describe(&ToolDescribeRequest {
                tool: "workspace-git".to_string(),
                command: "write_file_atomic".to_string(),
            })
            .unwrap();
        assert_eq!(response.tool, "workspace-git");
        assert_eq!(response.command, "write_file_atomic");
        assert_eq!(
            response.input_schema.id,
            "workspace-git.write_file_atomic.input"
        );
        assert_eq!(
            response.capabilities_required,
            vec!["workspace.write".to_string()]
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn plan_and_policy_inspect_resolve_approval_and_budgets() {
        let root = temp_dir("plan");
        let manifest = sample_manifest();
        write_manifest(&root, &manifest);
        let registry =
            FilePluginRegistry::load_from_roots(&[root.clone()], HOST_RUNTIME_API_VERSION).unwrap();
        let policy = StaticPolicyResolver::default();
        let host = PluginRuntimeHost::new(registry, policy).unwrap();

        let plan = host
            .tool_plan(&PlanRequest {
                command: "write_file_atomic".to_string(),
                input: serde_json::json!({
                    "workspaceHandle": "wrk_123",
                    "path": "docs/notes.md",
                    "content": "hello"
                }),
                context: sample_context(),
            })
            .unwrap();
        assert!(plan.approval_required);
        assert!(plan.blocking_issues.is_empty());
        assert_eq!(plan.resource_limits.unwrap().time_ms, Some(4_000));

        let inspect = host
            .tool_policy_inspect(&PolicyInspectRequest {
                tool: "workspace-git".to_string(),
                command: "write_file_atomic".to_string(),
            })
            .unwrap();
        assert!(inspect.approval_required);
        assert_eq!(inspect.operating_mode, OperatingMode::Production);
        assert_eq!(
            inspect.capabilities.allowed,
            vec!["workspace.write".to_string()]
        );

        let approval = host
            .approval_payload_for_plan(&PlanRequest {
                command: "write_file_atomic".to_string(),
                input: serde_json::json!({
                    "workspaceHandle": "wrk_123",
                    "path": "docs/notes.md",
                    "content": "hello"
                }),
                context: sample_context(),
            })
            .unwrap()
            .expect("approval payload should exist");
        assert_eq!(approval.tool, "workspace-git");
        assert_eq!(
            approval.workspace_impact.unwrap().touched_paths,
            vec!["docs/notes.md".to_string()]
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn denied_capability_returns_deterministic_policy_error() {
        let root = temp_dir("policy-block");
        let manifest = sample_manifest();
        write_manifest(&root, &manifest);
        let registry =
            FilePluginRegistry::load_from_roots(&[root.clone()], HOST_RUNTIME_API_VERSION).unwrap();
        let policy = StaticPolicyResolver::new(
            OperatingMode::Production,
            ResourceLimits {
                memory_bytes: Some(134_217_728),
                time_ms: Some(5_000),
                storage_bytes: Some(268_435_456),
                output_bytes: Some(131_072),
                network_policy: Some(NetworkPolicy::Denied),
            },
            BTreeSet::from(["workspace.write".to_string()]),
        );
        let host = PluginRuntimeHost::new(registry, policy).unwrap();

        let response = host
            .tool_invoke(&InvokeRequest {
                command: "write_file_atomic".to_string(),
                input: serde_json::json!({
                    "workspaceHandle": "wrk_123",
                    "path": "docs/notes.md",
                    "content": "hello"
                }),
                context: sample_context(),
            })
            .unwrap();
        assert!(response.result.is_none());
        assert_eq!(
            response.error.unwrap().code,
            "CAPABILITY_DENIED".to_string()
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn invoke_fails_closed_for_invalid_component_after_wp4_support() {
        let root = temp_dir("invoke");
        let manifest = sample_manifest();
        write_manifest(&root, &manifest);
        let registry =
            FilePluginRegistry::load_from_roots(&[root.clone()], HOST_RUNTIME_API_VERSION).unwrap();
        let host = PluginRuntimeHost::new(registry, StaticPolicyResolver::default()).unwrap();

        let response = host
            .tool_invoke(&InvokeRequest {
                command: "write_file_atomic".to_string(),
                input: serde_json::json!({
                    "workspaceHandle": "wrk_123",
                    "path": "docs/notes.md",
                    "content": "hello"
                }),
                context: sample_context(),
            })
            .unwrap();
        assert!(response.result.is_none());
        let error = response.error.expect("error should be present");
        assert_eq!(error.code, "RUNTIME_COMPONENT_LOAD_FAILED");
        assert!(!error.retryable);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn manifest_only_indexing_never_needs_wasm_artifact() {
        let root = temp_dir("index-only");
        let manifest = sample_manifest();
        write_manifest(&root, &manifest);

        let registry =
            FilePluginRegistry::load_from_roots(&[root.clone()], HOST_RUNTIME_API_VERSION).unwrap();
        let RegisteredPlugin { manifest_path, .. } = registry
            .get_registered_plugin("workspace-git")
            .unwrap()
            .clone();
        assert!(manifest_path.ends_with("plugin.json"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn validation_result_shape_is_serializable() {
        let result = ManifestValidationResult {
            valid: false,
            issues: vec![oberon4_core::ManifestValidationIssue {
                code: "duplicate_command".to_string(),
                message: "duplicate command".to_string(),
                field: Some("commands[1].name".to_string()),
            }],
        };
        let encoded = serde_json::to_string(&result).unwrap();
        let decoded: ManifestValidationResult = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, result);
    }

    #[test]
    fn broker_foundations_default_to_host_managed_limits() {
        let foundations = RuntimeBrokerFoundations::default();
        assert_eq!(foundations.artifact.inline_output_threshold_bytes, 131_072);
        assert_eq!(foundations.artifact.max_artifact_bytes, 33_554_432);
        assert_eq!(foundations.storage.private_storage_bytes, 67_108_864);
        assert!(foundations.storage.workspace_handles_required);
    }
}
