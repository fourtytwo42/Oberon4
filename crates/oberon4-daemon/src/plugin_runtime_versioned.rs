#![allow(clippy::result_large_err)]

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use oberon4_core::plugin_runtime::DiffSummary;
use oberon4_core::{
    ApprovalExpectation, ArtifactBroker, ArtifactCreateRequest, ArtifactReference, BuildMetadata,
    CapabilityManifest, CommandPlanPreview, ConformanceStatus, GithubRequest, InvokeStatus,
    LifecycleActions, LifecycleHooks, ManifestValidationResult, OberonError, PlanRequest,
    PluginCommandManifest, PluginExample, PluginManifest, PreconditionModel, PromptingManifest,
    ProvenanceManifest, RequestId, ResourceBudgetRequest, SecurityLevel, ServiceBroker,
    SideEffectClass, SignatureMetadata, ToolError, ToolInvokeResponseEnvelope, ToolResult,
    ToolTier, WorkspaceBroker, WorkspaceHandle, WorkspaceImpact, WorkspacePatchRequest,
    WorkspacePreconditionStatus, WorkspaceWriteRequest,
};
use serde::{Deserialize, Serialize};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use crate::plugin_runtime::{HOST_RUNTIME_API_VERSION, RegisteredPlugin, RegisteredToolSource};
use crate::plugin_runtime_data::{PluginRuntimeWorkspaceBroker, hex_name};

const DEFAULT_GITHUB_SERVICE_HANDLE: &str = "svc_github";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RemoteBinding {
    provider: String,
    repo: String,
    service_handle: String,
    default_branch: String,
    private: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CommitRecord {
    id: String,
    message: String,
    branch: String,
    parent: Option<String>,
    created_at: String,
    snapshot: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct VersionedWorkspaceState {
    current_branch: String,
    branches: BTreeMap<String, Option<String>>,
    commits: Vec<CommitRecord>,
    remote: Option<RemoteBinding>,
}

impl Default for VersionedWorkspaceState {
    fn default() -> Self {
        let default_branch = "main".to_string();
        let mut branches = BTreeMap::new();
        branches.insert(default_branch.clone(), None);
        Self {
            current_branch: default_branch,
            branches,
            commits: Vec::new(),
            remote: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedWorkspacePlanDetails {
    pub estimated_steps: u32,
    pub blocking_issues: Vec<String>,
    pub approval_preview: Option<CommandPlanPreview>,
}

pub struct PluginRuntimeVersionedPackage {
    state_root: PathBuf,
    workspace_broker: Arc<PluginRuntimeWorkspaceBroker>,
    artifact_broker: Arc<dyn ArtifactBroker + Send + Sync>,
    service_broker: Arc<dyn ServiceBroker + Send + Sync>,
    _inline_output_threshold_bytes: u64,
}

impl PluginRuntimeVersionedPackage {
    pub fn package_id() -> &'static str {
        "versioned-workspace"
    }

    pub fn tool_ids() -> &'static [&'static str] {
        &["workspace", "git", "github"]
    }

    pub fn new(
        state_root: impl Into<PathBuf>,
        workspace_broker: Arc<PluginRuntimeWorkspaceBroker>,
        artifact_broker: Arc<dyn ArtifactBroker + Send + Sync>,
        service_broker: Arc<dyn ServiceBroker + Send + Sync>,
        inline_output_threshold_bytes: u64,
    ) -> Result<Self, OberonError> {
        let state_root = state_root.into();
        fs::create_dir_all(&state_root).map_err(|err| {
            OberonError::Storage(format!("versioned state root create failed: {err}"))
        })?;
        Ok(Self {
            state_root,
            workspace_broker,
            artifact_broker,
            service_broker,
            _inline_output_threshold_bytes: inline_output_threshold_bytes,
        })
    }

    pub fn registered_plugins() -> Vec<RegisteredPlugin> {
        vec![
            RegisteredPlugin {
                manifest_path: PathBuf::from("<builtin:workspace>"),
                wasm_path: PathBuf::from("<builtin:workspace>"),
                manifest: workspace_manifest(),
                tool_tier: ToolTier::BrokeredPrivileged,
                source_kind: RegisteredToolSource::FirstPartyPackage,
                package_id: Some(Self::package_id().to_string()),
                host_trusted: true,
                enabled: true,
                archive_path: None,
                archive_entry_manifest_path: None,
                archive_entry_wasm_path: None,
                archive_checksum: None,
                generated_config_path: None,
                manifest_validation: ManifestValidationResult {
                    valid: true,
                    issues: Vec::new(),
                },
                conformance_status: ConformanceStatus::Passed,
                conformance_issues: Vec::new(),
            },
            RegisteredPlugin {
                manifest_path: PathBuf::from("<builtin:git>"),
                wasm_path: PathBuf::from("<builtin:git>"),
                manifest: git_manifest(),
                tool_tier: ToolTier::BrokeredPrivileged,
                source_kind: RegisteredToolSource::FirstPartyPackage,
                package_id: Some(Self::package_id().to_string()),
                host_trusted: true,
                enabled: true,
                archive_path: None,
                archive_entry_manifest_path: None,
                archive_entry_wasm_path: None,
                archive_checksum: None,
                generated_config_path: None,
                manifest_validation: ManifestValidationResult {
                    valid: true,
                    issues: Vec::new(),
                },
                conformance_status: ConformanceStatus::Passed,
                conformance_issues: Vec::new(),
            },
            RegisteredPlugin {
                manifest_path: PathBuf::from("<builtin:github>"),
                wasm_path: PathBuf::from("<builtin:github>"),
                manifest: github_manifest(),
                tool_tier: ToolTier::BrokeredPrivileged,
                source_kind: RegisteredToolSource::FirstPartyPackage,
                package_id: Some(Self::package_id().to_string()),
                host_trusted: true,
                enabled: true,
                archive_path: None,
                archive_entry_manifest_path: None,
                archive_entry_wasm_path: None,
                archive_checksum: None,
                generated_config_path: None,
                manifest_validation: ManifestValidationResult {
                    valid: true,
                    issues: Vec::new(),
                },
                conformance_status: ConformanceStatus::Passed,
                conformance_issues: Vec::new(),
            },
        ]
    }

    pub fn is_builtin_tool(tool: &str) -> bool {
        matches!(tool, "workspace" | "git" | "github")
    }

    pub fn ensure_default_package_installed(package_root: &Path) -> Result<(), OberonError> {
        let bootstrap_marker = oberon4_core::default_data_dir()
            .join("plugin-runtime")
            .join("first-party-package-bootstrap.marker");
        if bootstrap_marker.exists() {
            return Ok(());
        }

        Self::materialize_package_bundle(package_root)?;

        if let Some(parent) = bootstrap_marker.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                OberonError::Storage(format!("bootstrap marker dir create failed: {err}"))
            })?;
        }
        fs::write(&bootstrap_marker, b"versioned-workspace-seeded")
            .map_err(|err| OberonError::Storage(format!("bootstrap marker write failed: {err}")))?;
        Ok(())
    }

    pub fn ensure_default_package_archive_installed(
        archive_root: &Path,
    ) -> Result<(), OberonError> {
        let bootstrap_marker = oberon4_core::default_data_dir()
            .join("plugin-runtime")
            .join("first-party-package-archive-bootstrap.marker");
        if bootstrap_marker.exists() {
            return Ok(());
        }

        Self::materialize_package_archive(archive_root)?;

        if let Some(parent) = bootstrap_marker.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                OberonError::Storage(format!("bootstrap marker dir create failed: {err}"))
            })?;
        }
        fs::write(&bootstrap_marker, b"versioned-workspace-archive-seeded")
            .map_err(|err| OberonError::Storage(format!("bootstrap marker write failed: {err}")))?;
        Ok(())
    }

    pub fn materialize_package_bundle(package_root: &Path) -> Result<(), OberonError> {
        let package_dir = package_root.join(Self::package_id());
        fs::create_dir_all(package_dir.join("tools"))
            .map_err(|err| OberonError::Storage(format!("package root create failed: {err}")))?;

        let package_manifest = serde_json::json!({
            "id": Self::package_id(),
            "version": "0.1.0",
            "displayName": "Versioned Workspace Package",
            "description": "First-party brokered workspace, git, and github tools.",
            "tools": [
                {"tool":"workspace","manifestPath":"tools/workspace.plugin.json"},
                {"tool":"git","manifestPath":"tools/git.plugin.json"},
                {"tool":"github","manifestPath":"tools/github.plugin.json"}
            ]
        });

        fs::write(
            package_dir.join("package.json"),
            serde_json::to_vec_pretty(&package_manifest).map_err(|err| {
                OberonError::Storage(format!("package manifest encode failed: {err}"))
            })?,
        )
        .map_err(|err| OberonError::Storage(format!("package manifest write failed: {err}")))?;

        write_package_tool_manifest(&package_dir, "workspace.plugin.json", &workspace_manifest())?;
        write_package_tool_manifest(&package_dir, "git.plugin.json", &git_manifest())?;
        write_package_tool_manifest(&package_dir, "github.plugin.json", &github_manifest())?;
        Ok(())
    }

    pub fn materialize_package_archive(archive_root: &Path) -> Result<(), OberonError> {
        fs::create_dir_all(archive_root)
            .map_err(|err| OberonError::Storage(format!("archive root create failed: {err}")))?;
        let archive_path = archive_root.join(format!("{}.obpkg.zip", Self::package_id()));
        let file = fs::File::create(&archive_path)
            .map_err(|err| OberonError::Storage(format!("archive create failed: {err}")))?;
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

        let archive_manifest = serde_json::json!({
            "id": Self::package_id(),
            "version": "0.1.0",
            "kind": "first-party-package",
            "packageManifestPath": "package.json",
            "settings": []
        });
        let package_manifest = serde_json::json!({
            "id": Self::package_id(),
            "version": "0.1.0",
            "displayName": "Versioned Workspace Package",
            "description": "First-party brokered workspace, git, and github tools.",
            "tools": [
                {"tool":"workspace","manifestPath":"tools/workspace.plugin.json"},
                {"tool":"git","manifestPath":"tools/git.plugin.json"},
                {"tool":"github","manifestPath":"tools/github.plugin.json"}
            ]
        });

        zip.start_file("oberon-archive.json", options)
            .map_err(|err| OberonError::Storage(format!("archive entry create failed: {err}")))?;
        zip.write_all(
            &serde_json::to_vec_pretty(&archive_manifest).map_err(|err| {
                OberonError::Storage(format!("archive manifest encode failed: {err}"))
            })?,
        )
        .map_err(|err| OberonError::Storage(format!("archive manifest write failed: {err}")))?;

        zip.start_file("package.json", options)
            .map_err(|err| OberonError::Storage(format!("package entry create failed: {err}")))?;
        zip.write_all(
            &serde_json::to_vec_pretty(&package_manifest).map_err(|err| {
                OberonError::Storage(format!("package manifest encode failed: {err}"))
            })?,
        )
        .map_err(|err| OberonError::Storage(format!("package manifest write failed: {err}")))?;

        write_package_tool_manifest_to_zip(
            &mut zip,
            "tools/workspace.plugin.json",
            &workspace_manifest(),
        )?;
        write_package_tool_manifest_to_zip(&mut zip, "tools/git.plugin.json", &git_manifest())?;
        write_package_tool_manifest_to_zip(
            &mut zip,
            "tools/github.plugin.json",
            &github_manifest(),
        )?;

        zip.finish()
            .map_err(|err| OberonError::Storage(format!("archive finalize failed: {err}")))?;
        Ok(())
    }

    pub fn plan(
        &self,
        tool: &str,
        command: &PluginCommandManifest,
        request: &PlanRequest,
    ) -> Result<VersionedWorkspacePlanDetails, OberonError> {
        let mut blocking_issues = Vec::new();
        let approval_preview = match (tool, command.name.as_str()) {
            ("workspace", "write_file_atomic" | "apply_patch") => {
                let workspace = request
                    .input
                    .get("workspaceHandle")
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown");
                let path = request
                    .input
                    .get("path")
                    .and_then(|value| value.as_str())
                    .map(|value| vec![value.to_string()])
                    .unwrap_or_default();
                Some(CommandPlanPreview {
                    summary: format!("workspace mutation on {workspace}"),
                    workspace_impact: Some(WorkspaceImpact {
                        workspace_handle: workspace.to_string(),
                        workspace_revision: request
                            .context
                            .workspace_revision
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string()),
                        touched_paths: path,
                    }),
                })
            }
            ("git", "push") => Some(CommandPlanPreview {
                summary: "git push will use brokered GitHub authentication".to_string(),
                workspace_impact: request
                    .input
                    .get("workspaceHandle")
                    .and_then(|value| value.as_str())
                    .map(|workspace| WorkspaceImpact {
                        workspace_handle: workspace.to_string(),
                        workspace_revision: request
                            .context
                            .workspace_revision
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string()),
                        touched_paths: Vec::new(),
                    }),
            }),
            ("github", "clone_to_workspace") => Some(CommandPlanPreview {
                summary: "github clone will attach a new logical workspace".to_string(),
                workspace_impact: None,
            }),
            _ => None,
        };

        match (tool, command.name.as_str()) {
            ("workspace", "write_file_atomic" | "apply_patch") => {
                if request.context.workspace_revision.is_none() {
                    blocking_issues
                        .push("workspace writes require current workspace revision".to_string());
                }
            }
            ("git", "push") => {
                let workspace = workspace_handle_from_input(&request.input)?;
                let state = self.load_state(&workspace)?;
                if state.remote.is_none() {
                    blocking_issues.push(
                        "git push requires a workspace cloned from github or configured remote"
                            .to_string(),
                    );
                }
            }
            ("github", "clone_to_workspace") => {
                self.default_github_service()?;
            }
            _ => {}
        }

        Ok(VersionedWorkspacePlanDetails {
            estimated_steps: match (tool, command.name.as_str()) {
                ("git", "push") => 2,
                ("github", "clone_to_workspace") => 2,
                _ => 1,
            },
            blocking_issues,
            approval_preview,
        })
    }

    pub fn invoke(
        &self,
        tool: &str,
        command: &PluginCommandManifest,
        request: &oberon4_core::InvokeRequest,
    ) -> Result<ToolInvokeResponseEnvelope, ToolError> {
        validate_input_schema(&command.input_schema, &request.input, request)?;
        validate_output_schema_shape(command)
            .map_err(|err| contract_error(request, &err.to_string()))?;

        let invoke = match (tool, command.name.as_str()) {
            ("workspace", "read_file") => self.invoke_workspace_read_file(request),
            ("workspace", "list_dir") => self.invoke_workspace_list_dir(request),
            ("workspace", "stat_path") => self.invoke_workspace_stat_path(request),
            ("workspace", "write_file_atomic") => self.invoke_workspace_write_file_atomic(request),
            ("workspace", "apply_patch") => self.invoke_workspace_apply_patch(request),
            ("git", "status") => self.invoke_git_status(request),
            ("git", "diff") => self.invoke_git_diff(request),
            ("git", "branch.create") => self.invoke_git_branch_create(request),
            ("git", "commit") => self.invoke_git_commit(request),
            ("git", "push") => self.invoke_git_push(request),
            ("github", "auth_status") => self.invoke_github_auth_status(request),
            ("github", "clone_to_workspace") => self.invoke_github_clone_to_workspace(request),
            ("github", "repo_info") => self.invoke_github_repo_info(request),
            _ => Err(contract_error(
                request,
                &format!("unknown built-in command '{}.{}'", tool, command.name),
            )),
        }?;

        validate_output_schema(&command.output_schema, &invoke.data, request)?;
        Ok(ToolInvokeResponseEnvelope {
            result: Some(invoke),
            error: None,
        })
    }

    fn invoke_workspace_read_file(
        &self,
        request: &oberon4_core::InvokeRequest,
    ) -> Result<ToolResult, ToolError> {
        let workspace = workspace_handle_from_input(&request.input)
            .map_err(|err| contract_error(request, &err.to_string()))?;
        let path = path_from_input(&request.input)
            .map_err(|err| contract_error(request, &err.to_string()))?;
        let result = self
            .workspace_broker
            .read_file(&workspace, &path)
            .map_err(|err| tool_error_from_oberon(request, err))?;
        Ok(ToolResult {
            status: InvokeStatus::Ok,
            tool: "workspace".to_string(),
            command: "read_file".to_string(),
            summary: format!("read {}", result.logical_path),
            data: serde_json::json!({
                "workspaceHandle": result.workspace_handle,
                "workspaceRevision": result.workspace_revision,
                "path": result.logical_path,
                "content": result.content,
                "truncated": false
            }),
            artifacts: Vec::new(),
            truncated: false,
            next_suggested_action: None,
        })
    }

    fn invoke_workspace_list_dir(
        &self,
        request: &oberon4_core::InvokeRequest,
    ) -> Result<ToolResult, ToolError> {
        let workspace = workspace_handle_from_input(&request.input)
            .map_err(|err| contract_error(request, &err.to_string()))?;
        let path = optional_path_from_input(&request.input);
        let result = self
            .workspace_broker
            .list_dir(&workspace, &path)
            .map_err(|err| tool_error_from_oberon(request, err))?;
        Ok(ToolResult {
            status: InvokeStatus::Ok,
            tool: "workspace".to_string(),
            command: "list_dir".to_string(),
            summary: format!("listed {}", result.logical_path),
            data: serde_json::json!({
                "workspaceHandle": result.workspace_handle,
                "workspaceRevision": result.workspace_revision,
                "path": result.logical_path,
                "entries": result.entries.into_iter().map(|entry| {
                    serde_json::json!({
                        "name": entry.entry.name,
                        "kind": match entry.entry.kind {
                            oberon4_core::WorkspacePathKind::File => "file",
                            oberon4_core::WorkspacePathKind::Directory => "directory",
                        }
                    })
                }).collect::<Vec<_>>()
            }),
            artifacts: Vec::new(),
            truncated: false,
            next_suggested_action: None,
        })
    }

    fn invoke_workspace_stat_path(
        &self,
        request: &oberon4_core::InvokeRequest,
    ) -> Result<ToolResult, ToolError> {
        let workspace = workspace_handle_from_input(&request.input)
            .map_err(|err| contract_error(request, &err.to_string()))?;
        let path = path_from_input(&request.input)
            .map_err(|err| contract_error(request, &err.to_string()))?;
        let metadata = self
            .workspace_broker
            .workspace_metadata(&workspace)
            .map_err(|err| tool_error_from_oberon(request, err))?;
        let result = self
            .workspace_broker
            .stat_path(&workspace, &path)
            .map_err(|err| tool_error_from_oberon(request, err))?;
        Ok(ToolResult {
            status: InvokeStatus::Ok,
            tool: "workspace".to_string(),
            command: "stat_path".to_string(),
            summary: format!("stat {}", result.logical_path),
            data: serde_json::json!({
                "workspaceHandle": workspace.workspace_handle,
                "workspaceRevision": metadata.workspace_revision,
                "path": result.logical_path,
                "kind": match result.entry.kind {
                    oberon4_core::WorkspacePathKind::File => "file",
                    oberon4_core::WorkspacePathKind::Directory => "directory",
                },
                "size": result.size_bytes
            }),
            artifacts: Vec::new(),
            truncated: false,
            next_suggested_action: None,
        })
    }

    fn invoke_workspace_write_file_atomic(
        &self,
        request: &oberon4_core::InvokeRequest,
    ) -> Result<ToolResult, ToolError> {
        let workspace = workspace_handle_from_input(&request.input)
            .map_err(|err| contract_error(request, &err.to_string()))?;
        let path = path_from_input(&request.input)
            .map_err(|err| contract_error(request, &err.to_string()))?;
        let content = required_string(&request.input, "content")
            .map_err(|err| contract_error(request, &err))?;
        let expected_revision = required_string(&request.input, "expectedRevision")
            .map_err(|err| tool_error_from_validation(request, &err))?;
        let result = self
            .workspace_broker
            .write_file_atomic(&WorkspaceWriteRequest {
                workspace_handle: workspace.workspace_handle,
                logical_path: path,
                content,
                expected_revision: Some(expected_revision),
                request_id: Some(request.context.request_id.as_str().to_string()),
            })
            .map_err(|err| tool_error_from_oberon(request, err))?;
        if let Some(conflict) = result.conflict {
            return Err(tool_error_conflict(
                request,
                &format!(
                    "{} (expected {:?}, actual {})",
                    conflict.reason, conflict.expected_revision, conflict.actual_revision
                ),
            ));
        }
        let write = result.result.ok_or_else(|| {
            contract_error(request, "workspace write did not return a write result")
        })?;
        Ok(ToolResult {
            status: InvokeStatus::Ok,
            tool: "workspace".to_string(),
            command: "write_file_atomic".to_string(),
            summary: format!("wrote {}", write.logical_path),
            data: serde_json::json!({
                "workspaceHandle": write.workspace_handle,
                "workspaceRevision": write.workspace_revision,
                "logicalPath": write.logical_path,
                "operation": write.operation,
                "bytesWritten": write.bytes_written,
                "preconditionStatus": workspace_precondition_status_name(&write.precondition_status),
                "touched": write.touched
            }),
            artifacts: Vec::new(),
            truncated: false,
            next_suggested_action: None,
        })
    }

    fn invoke_workspace_apply_patch(
        &self,
        request: &oberon4_core::InvokeRequest,
    ) -> Result<ToolResult, ToolError> {
        let workspace = workspace_handle_from_input(&request.input)
            .map_err(|err| contract_error(request, &err.to_string()))?;
        let expected_revision = required_string(&request.input, "expectedRevision")
            .map_err(|err| tool_error_from_validation(request, &err))?;
        let patch = required_string(&request.input, "patch")
            .map_err(|err| contract_error(request, &err))?;
        let (logical_path, search, replace) =
            parse_apply_patch_request(&patch).map_err(|err| contract_error(request, &err))?;
        let result = self
            .workspace_broker
            .apply_patch(&WorkspacePatchRequest {
                workspace_handle: workspace.workspace_handle,
                logical_path: logical_path.clone(),
                search,
                replace,
                expected_revision: Some(expected_revision),
                request_id: Some(request.context.request_id.as_str().to_string()),
            })
            .map_err(|err| tool_error_from_oberon(request, err))?;
        if let Some(conflict) = result.conflict {
            return Err(tool_error_conflict(
                request,
                &format!(
                    "{} (expected {:?}, actual {})",
                    conflict.reason, conflict.expected_revision, conflict.actual_revision
                ),
            ));
        }
        let write = result.result.ok_or_else(|| {
            contract_error(request, "workspace patch did not return a write result")
        })?;
        Ok(ToolResult {
            status: InvokeStatus::Ok,
            tool: "workspace".to_string(),
            command: "apply_patch".to_string(),
            summary: format!("patched {}", logical_path),
            data: serde_json::json!({
                "workspaceHandle": write.workspace_handle,
                "workspaceRevision": write.workspace_revision,
                "operation": "apply_patch",
                "preconditionStatus": workspace_precondition_status_name(&write.precondition_status),
                "touchedPaths": [write.logical_path]
            }),
            artifacts: Vec::new(),
            truncated: false,
            next_suggested_action: None,
        })
    }

    fn invoke_git_status(
        &self,
        request: &oberon4_core::InvokeRequest,
    ) -> Result<ToolResult, ToolError> {
        let workspace = workspace_handle_from_input(&request.input)
            .map_err(|err| contract_error(request, &err.to_string()))?;
        let metadata = self
            .workspace_broker
            .workspace_metadata(&workspace)
            .map_err(|err| tool_error_from_oberon(request, err))?;
        let state = self
            .load_state(&workspace)
            .map_err(|err| tool_error_from_oberon(request, err))?;
        let snapshot = self
            .workspace_snapshot(&workspace)
            .map_err(|err| tool_error_from_oberon(request, err))?;
        let head = self.head_snapshot(&state);
        let diff = diff_summary(&head, &snapshot);
        Ok(ToolResult {
            status: InvokeStatus::Ok,
            tool: "git".to_string(),
            command: "status".to_string(),
            summary: format!("git status on {}", state.current_branch),
            data: serde_json::json!({
                "workspaceHandle": workspace.workspace_handle,
                "workspaceRevision": metadata.workspace_revision,
                "branch": state.current_branch,
                "dirty": diff.files_changed > 0,
                "filesChanged": diff.files_changed
            }),
            artifacts: Vec::new(),
            truncated: false,
            next_suggested_action: None,
        })
    }

    fn invoke_git_diff(
        &self,
        request: &oberon4_core::InvokeRequest,
    ) -> Result<ToolResult, ToolError> {
        let workspace = workspace_handle_from_input(&request.input)
            .map_err(|err| contract_error(request, &err.to_string()))?;
        let metadata = self
            .workspace_broker
            .workspace_metadata(&workspace)
            .map_err(|err| tool_error_from_oberon(request, err))?;
        let state = self
            .load_state(&workspace)
            .map_err(|err| tool_error_from_oberon(request, err))?;
        let snapshot = self
            .workspace_snapshot(&workspace)
            .map_err(|err| tool_error_from_oberon(request, err))?;
        let head = self.head_snapshot(&state);
        let (summary, diff_text) = diff_text(&head, &snapshot);
        let artifact = self
            .artifact_broker
            .create_artifact(&ArtifactCreateRequest {
                media_type: "text/plain".to_string(),
                retention: oberon4_core::ArtifactRetention::ShortTerm,
                bytes: diff_text.into_bytes(),
                producer_tool: Some("git.diff".to_string()),
                producer_plugin: Some("git".to_string()),
            })
            .map_err(|err| tool_error_from_oberon(request, err))?;
        let artifact_ref = ArtifactReference {
            artifact_id: artifact.artifact_id.clone(),
            media_type: artifact.media_type.clone(),
            size: artifact.size,
            retention: Some(artifact.retention),
            truncated: false,
        };
        Ok(ToolResult {
            status: InvokeStatus::Ok,
            tool: "git".to_string(),
            command: "diff".to_string(),
            summary: format!("git diff on {}", state.current_branch),
            data: serde_json::json!({
                "workspaceHandle": workspace.workspace_handle,
                "workspaceRevision": metadata.workspace_revision,
                "branch": state.current_branch,
                "filesChanged": summary.files_changed,
                "diffArtifact": artifact.artifact_id
            }),
            artifacts: vec![artifact_ref],
            truncated: false,
            next_suggested_action: None,
        })
    }

    fn invoke_git_branch_create(
        &self,
        request: &oberon4_core::InvokeRequest,
    ) -> Result<ToolResult, ToolError> {
        let workspace = workspace_handle_from_input(&request.input)
            .map_err(|err| contract_error(request, &err.to_string()))?;
        let branch =
            required_string(&request.input, "name").map_err(|err| contract_error(request, &err))?;
        let mut state = self
            .load_state(&workspace)
            .map_err(|err| tool_error_from_oberon(request, err))?;
        let created = !state.branches.contains_key(&branch);
        let current_head = state
            .branches
            .get(&state.current_branch)
            .cloned()
            .unwrap_or(None);
        state.branches.entry(branch.clone()).or_insert(current_head);
        state.current_branch = branch.clone();
        self.save_state(&workspace, &state)
            .map_err(|err| tool_error_from_oberon(request, err))?;
        Ok(ToolResult {
            status: InvokeStatus::Ok,
            tool: "git".to_string(),
            command: "branch.create".to_string(),
            summary: format!("switched to {branch}"),
            data: serde_json::json!({
                "workspaceHandle": workspace.workspace_handle,
                "branch": branch,
                "created": created
            }),
            artifacts: Vec::new(),
            truncated: false,
            next_suggested_action: None,
        })
    }

    fn invoke_git_commit(
        &self,
        request: &oberon4_core::InvokeRequest,
    ) -> Result<ToolResult, ToolError> {
        let workspace = workspace_handle_from_input(&request.input)
            .map_err(|err| contract_error(request, &err.to_string()))?;
        let message = required_string(&request.input, "message")
            .map_err(|err| contract_error(request, &err))?;
        let mut state = self
            .load_state(&workspace)
            .map_err(|err| tool_error_from_oberon(request, err))?;
        let snapshot = self
            .workspace_snapshot(&workspace)
            .map_err(|err| tool_error_from_oberon(request, err))?;
        let head = self.head_snapshot(&state);
        let diff = diff_summary(&head, &snapshot);
        let commit_id = format!("c_{}", RequestId::new().as_str().to_lowercase());
        let parent = state
            .branches
            .get(&state.current_branch)
            .cloned()
            .unwrap_or(None);
        state.commits.push(CommitRecord {
            id: commit_id.clone(),
            message,
            branch: state.current_branch.clone(),
            parent: parent.clone(),
            created_at: now_iso8601(),
            snapshot,
        });
        state
            .branches
            .insert(state.current_branch.clone(), Some(commit_id.clone()));
        self.save_state(&workspace, &state)
            .map_err(|err| tool_error_from_oberon(request, err))?;
        Ok(ToolResult {
            status: InvokeStatus::Ok,
            tool: "git".to_string(),
            command: "commit".to_string(),
            summary: format!("created commit {commit_id}"),
            data: serde_json::json!({
                "workspaceHandle": workspace.workspace_handle,
                "branch": state.current_branch,
                "headCommit": commit_id,
                "dirty": false,
                "filesChanged": diff.files_changed
            }),
            artifacts: Vec::new(),
            truncated: false,
            next_suggested_action: None,
        })
    }

    fn invoke_git_push(
        &self,
        request: &oberon4_core::InvokeRequest,
    ) -> Result<ToolResult, ToolError> {
        let workspace = workspace_handle_from_input(&request.input)
            .map_err(|err| contract_error(request, &err.to_string()))?;
        let remote = required_string(&request.input, "remote")
            .map_err(|err| contract_error(request, &err))?;
        let branch = required_string(&request.input, "branch")
            .map_err(|err| contract_error(request, &err))?;
        let state = self
            .load_state(&workspace)
            .map_err(|err| tool_error_from_oberon(request, err))?;
        let binding = state.remote.clone().ok_or_else(|| {
            tool_error_from_validation(request, "git push requires a configured github remote")
        })?;
        let commit = state
            .branches
            .get(&branch)
            .cloned()
            .flatten()
            .ok_or_else(|| {
                tool_error_from_validation(
                    request,
                    "git push requires a local commit on the selected branch",
                )
            })?;
        let response = self
            .service_broker
            .execute_github_request(&GithubRequest {
                service_handle: binding.service_handle.clone(),
                method: "POST".to_string(),
                path: format!("/repos/{}/git/refs/heads/{}", binding.repo, branch),
                headers: Vec::new(),
                body_json: Some(serde_json::json!({ "commit": commit })),
            })
            .map_err(|err| tool_error_from_oberon(request, err))?;
        Ok(ToolResult {
            status: InvokeStatus::Ok,
            tool: "git".to_string(),
            command: "push".to_string(),
            summary: format!("pushed {branch} to {}", binding.repo),
            data: serde_json::json!({
                "remote": remote,
                "branch": branch,
                "commit": commit,
                "pushed": response.status_code >= 200 && response.status_code < 300,
                "provider": binding.provider,
                "remoteRef": format!("refs/heads/{}", state.current_branch)
            }),
            artifacts: Vec::new(),
            truncated: false,
            next_suggested_action: None,
        })
    }

    fn invoke_github_auth_status(
        &self,
        _request: &oberon4_core::InvokeRequest,
    ) -> Result<ToolResult, ToolError> {
        let metadata = self.default_github_service().ok();
        Ok(ToolResult {
            status: InvokeStatus::Ok,
            tool: "github".to_string(),
            command: "auth_status".to_string(),
            summary: if metadata.is_some() {
                "github auth available".to_string()
            } else {
                "github auth unavailable".to_string()
            },
            data: serde_json::json!({
                "provider": "github",
                "authenticated": metadata.is_some(),
                "securityLevel": metadata
                    .and_then(|value| value.security_level)
                    .map(security_level_name)
                    .unwrap_or("encrypted_local")
            }),
            artifacts: Vec::new(),
            truncated: false,
            next_suggested_action: None,
        })
    }

    fn invoke_github_clone_to_workspace(
        &self,
        request: &oberon4_core::InvokeRequest,
    ) -> Result<ToolResult, ToolError> {
        let repo =
            required_string(&request.input, "repo").map_err(|err| contract_error(request, &err))?;
        let service = self
            .default_github_service()
            .map_err(|err| tool_error_from_oberon(request, err))?;
        let response = self
            .service_broker
            .execute_github_request(&GithubRequest {
                service_handle: service.service_handle.clone(),
                method: "GET".to_string(),
                path: format!("/repos/{repo}"),
                headers: Vec::new(),
                body_json: None,
            })
            .map_err(|err| tool_error_from_oberon(request, err))?;
        let workspace = WorkspaceHandle {
            workspace_handle: format!("wrk_{}", RequestId::new().as_str().to_lowercase()),
        };
        self.workspace_broker
            .create_or_attach_workspace(&workspace)
            .map_err(|err| tool_error_from_oberon(request, err))?;
        let default_branch = response
            .body
            .get("defaultBranch")
            .or_else(|| response.body.get("default_branch"))
            .and_then(|value| value.as_str())
            .unwrap_or("main")
            .to_string();
        let private = response
            .body
            .get("private")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        let state = VersionedWorkspaceState {
            current_branch: default_branch.clone(),
            branches: BTreeMap::from([(default_branch.clone(), None)]),
            commits: Vec::new(),
            remote: Some(RemoteBinding {
                provider: "github".to_string(),
                repo: repo.clone(),
                service_handle: service.service_handle,
                default_branch: default_branch.clone(),
                private,
            }),
        };
        self.save_state(&workspace, &state)
            .map_err(|err| tool_error_from_oberon(request, err))?;
        Ok(ToolResult {
            status: InvokeStatus::Ok,
            tool: "github".to_string(),
            command: "clone_to_workspace".to_string(),
            summary: format!("attached workspace for {repo}"),
            data: serde_json::json!({
                "provider": "github",
                "workspaceHandle": workspace.workspace_handle,
                "defaultBranch": default_branch
            }),
            artifacts: Vec::new(),
            truncated: false,
            next_suggested_action: None,
        })
    }

    fn invoke_github_repo_info(
        &self,
        request: &oberon4_core::InvokeRequest,
    ) -> Result<ToolResult, ToolError> {
        let repo =
            required_string(&request.input, "repo").map_err(|err| contract_error(request, &err))?;
        let service = self
            .default_github_service()
            .map_err(|err| tool_error_from_oberon(request, err))?;
        let response = self
            .service_broker
            .execute_github_request(&GithubRequest {
                service_handle: service.service_handle,
                method: "GET".to_string(),
                path: format!("/repos/{repo}"),
                headers: Vec::new(),
                body_json: None,
            })
            .map_err(|err| tool_error_from_oberon(request, err))?;
        Ok(ToolResult {
            status: InvokeStatus::Ok,
            tool: "github".to_string(),
            command: "repo_info".to_string(),
            summary: format!("repo info for {repo}"),
            data: serde_json::json!({
                "provider": "github",
                "repo": repo,
                "defaultBranch": response
                    .body
                    .get("defaultBranch")
                    .or_else(|| response.body.get("default_branch"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("main"),
                "private": response
                    .body
                    .get("private")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(true)
            }),
            artifacts: Vec::new(),
            truncated: false,
            next_suggested_action: None,
        })
    }

    fn default_github_service(&self) -> Result<oberon4_core::ServiceMetadata, OberonError> {
        self.service_broker
            .resolve_service(DEFAULT_GITHUB_SERVICE_HANDLE)
    }

    fn workspace_snapshot(
        &self,
        workspace: &WorkspaceHandle,
    ) -> Result<BTreeMap<String, String>, OberonError> {
        let root = self.workspace_broker.workspace_root_path(workspace)?;
        let mut files = BTreeMap::new();
        collect_workspace_files(&root, "", &mut files)?;
        Ok(files)
    }

    fn load_state(
        &self,
        workspace: &WorkspaceHandle,
    ) -> Result<VersionedWorkspaceState, OberonError> {
        let path = self.state_path(workspace);
        if !path.exists() {
            return Ok(VersionedWorkspaceState::default());
        }
        let raw = fs::read_to_string(&path)
            .map_err(|err| OberonError::Storage(format!("versioned state read failed: {err}")))?;
        serde_json::from_str(&raw)
            .map_err(|err| OberonError::Validation(format!("versioned state decode failed: {err}")))
    }

    fn save_state(
        &self,
        workspace: &WorkspaceHandle,
        state: &VersionedWorkspaceState,
    ) -> Result<(), OberonError> {
        let path = self.state_path(workspace);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                OberonError::Storage(format!("versioned state parent create failed: {err}"))
            })?;
        }
        let encoded = serde_json::to_vec_pretty(state)
            .map_err(|err| OberonError::Storage(format!("versioned state encode failed: {err}")))?;
        fs::write(&path, encoded)
            .map_err(|err| OberonError::Storage(format!("versioned state write failed: {err}")))
    }

    fn state_path(&self, workspace: &WorkspaceHandle) -> PathBuf {
        self.state_root
            .join(format!("{}.json", hex_name(&workspace.workspace_handle)))
    }

    fn head_snapshot(&self, state: &VersionedWorkspaceState) -> BTreeMap<String, String> {
        let Some(head) = state.branches.get(&state.current_branch).cloned().flatten() else {
            return BTreeMap::new();
        };
        state
            .commits
            .iter()
            .find(|commit| commit.id == head)
            .map(|commit| commit.snapshot.clone())
            .unwrap_or_default()
    }
}

fn collect_workspace_files(
    root: &Path,
    prefix: &str,
    files: &mut BTreeMap<String, String>,
) -> Result<(), OberonError> {
    if !root.exists() {
        return Ok(());
    }
    let mut entries = fs::read_dir(root)
        .map_err(|err| OberonError::Storage(format!("workspace scan failed: {err}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| OberonError::Storage(format!("workspace scan failed: {err}")))?;
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let logical = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        };
        let file_type = entry
            .file_type()
            .map_err(|err| OberonError::Storage(format!("workspace file type failed: {err}")))?;
        if file_type.is_dir() {
            collect_workspace_files(&path, &logical, files)?;
        } else if file_type.is_file() {
            let content = fs::read_to_string(&path).map_err(|err| {
                OberonError::Storage(format!("workspace file read failed: {err}"))
            })?;
            files.insert(logical, content);
        }
    }
    Ok(())
}

fn diff_summary(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> DiffSummary {
    let mut changed = BTreeSet::new();
    for key in before.keys().chain(after.keys()) {
        if before.get(key) != after.get(key) {
            changed.insert(key.clone());
        }
    }
    DiffSummary {
        files_changed: changed.len() as u32,
        insertions: 0,
        deletions: 0,
    }
}

fn diff_text(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> (DiffSummary, String) {
    let summary = diff_summary(before, after);
    let mut diff = String::new();
    let changed = before
        .keys()
        .chain(after.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    for path in changed {
        match (before.get(&path), after.get(&path)) {
            (None, Some(content)) => {
                diff.push_str(&format!("+++ {path}\n{content}\n"));
            }
            (Some(_), None) => {
                diff.push_str(&format!("--- {path}\n"));
            }
            (Some(old), Some(new)) if old != new => {
                diff.push_str(&format!(
                    "--- {path}\n+++ {path}\n-old\n{old}\n+new\n{new}\n"
                ));
            }
            _ => {}
        }
    }
    (summary, diff)
}

fn validate_input_schema(
    schema: &serde_json::Value,
    data: &serde_json::Value,
    request: &oberon4_core::InvokeRequest,
) -> Result<(), ToolError> {
    let validator = jsonschema::validator_for(schema).map_err(|err| {
        contract_error(request, &format!("input schema compilation failed: {err}"))
    })?;
    if validator.validate(data).is_err() {
        return Err(contract_error(
            request,
            "command input does not match schema",
        ));
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

fn validate_output_schema(
    schema: &serde_json::Value,
    data: &serde_json::Value,
    request: &oberon4_core::InvokeRequest,
) -> Result<(), ToolError> {
    let validator = jsonschema::validator_for(schema).map_err(|err| {
        contract_error(request, &format!("output schema compilation failed: {err}"))
    })?;
    if validator.validate(data).is_err() {
        return Err(contract_error(
            request,
            "built-in command output does not match schema",
        ));
    }
    Ok(())
}

fn contract_error(request: &oberon4_core::InvokeRequest, message: &str) -> ToolError {
    ToolError {
        status: oberon4_core::ErrorStatus::Error,
        code: "RUNTIME_CONTRACT_MISMATCH".to_string(),
        message: message.to_string(),
        retryable: false,
        request_id: request.context.request_id.clone(),
        plugin_id: request.context.plugin_id.clone(),
        plugin_version: request.context.plugin_version.clone(),
        command: request.command.clone(),
        next_suggested_action: Some("tool_describe".to_string()),
    }
}

fn tool_error_from_validation(request: &oberon4_core::InvokeRequest, message: &str) -> ToolError {
    ToolError {
        status: oberon4_core::ErrorStatus::Error,
        code: "INVALID_INPUT".to_string(),
        message: message.to_string(),
        retryable: true,
        request_id: request.context.request_id.clone(),
        plugin_id: request.context.plugin_id.clone(),
        plugin_version: request.context.plugin_version.clone(),
        command: request.command.clone(),
        next_suggested_action: Some("tool_describe".to_string()),
    }
}

fn tool_error_conflict(request: &oberon4_core::InvokeRequest, message: &str) -> ToolError {
    ToolError {
        status: oberon4_core::ErrorStatus::Error,
        code: "CONFLICT".to_string(),
        message: message.to_string(),
        retryable: true,
        request_id: request.context.request_id.clone(),
        plugin_id: request.context.plugin_id.clone(),
        plugin_version: request.context.plugin_version.clone(),
        command: request.command.clone(),
        next_suggested_action: Some("tool_plan".to_string()),
    }
}

fn tool_error_from_oberon(request: &oberon4_core::InvokeRequest, err: OberonError) -> ToolError {
    let code = match err {
        OberonError::NotFound(_) => "NOT_FOUND",
        OberonError::PolicyDenied(_) => "CAPABILITY_DENIED",
        OberonError::Validation(_) => "INVALID_INPUT",
        OberonError::ContractMismatch(_) => "RUNTIME_CONTRACT_MISMATCH",
        OberonError::ManifestValidation(_) | OberonError::UnsupportedRuntimeApiVersion(_) => {
            "RUNTIME_CONTRACT_MISMATCH"
        }
        OberonError::ConfigParse(_) | OberonError::ConfigSerialize(_) => "INVALID_INPUT",
        OberonError::Storage(ref message) if message.contains("conflict") => "CONFLICT",
        OberonError::Storage(_) => "STORAGE_ERROR",
    };
    ToolError {
        status: oberon4_core::ErrorStatus::Error,
        code: code.to_string(),
        message: err.to_string(),
        retryable: !matches!(code, "CAPABILITY_DENIED" | "RUNTIME_CONTRACT_MISMATCH"),
        request_id: request.context.request_id.clone(),
        plugin_id: request.context.plugin_id.clone(),
        plugin_version: request.context.plugin_version.clone(),
        command: request.command.clone(),
        next_suggested_action: Some("tool_plan".to_string()),
    }
}

fn workspace_handle_from_input(input: &serde_json::Value) -> Result<WorkspaceHandle, OberonError> {
    let workspace_handle = input
        .get("workspaceHandle")
        .and_then(|value| value.as_str())
        .ok_or_else(|| OberonError::Validation("workspaceHandle is required".to_string()))?;
    Ok(WorkspaceHandle {
        workspace_handle: workspace_handle.to_string(),
    })
}

fn path_from_input(input: &serde_json::Value) -> Result<String, OberonError> {
    input
        .get("path")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| OberonError::Validation("path is required".to_string()))
}

fn optional_path_from_input(input: &serde_json::Value) -> String {
    input
        .get("path")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string()
}

fn required_string(input: &serde_json::Value, field: &str) -> Result<String, String> {
    input
        .get(field)
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("{field} is required"))
}

fn parse_apply_patch_request(patch: &str) -> Result<(String, String, String), String> {
    let lines = patch.lines().collect::<Vec<_>>();
    if lines.len() < 7
        || lines.first().copied() != Some("*** Begin Patch")
        || !lines[1].starts_with("*** Update File: ")
        || lines.last().copied() != Some("*** End Patch")
    {
        return Err("patch format is not supported by WP5 apply_patch".to_string());
    }
    let logical_path = lines[1]
        .strip_prefix("*** Update File: ")
        .ok_or_else(|| "patch is missing update file header".to_string())?
        .to_string();
    let mut removed = None;
    let mut added = None;
    for line in lines {
        if let Some(value) = line.strip_prefix('-') {
            removed = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix('+') {
            added = Some(value.to_string());
        }
    }
    match (removed, added) {
        (Some(search), Some(replace)) => Ok((logical_path, search, replace)),
        _ => Err("patch must include one removed line and one added line".to_string()),
    }
}

fn workspace_precondition_status_name(status: &WorkspacePreconditionStatus) -> &'static str {
    match status {
        WorkspacePreconditionStatus::Matched => "matched",
        WorkspacePreconditionStatus::Conflict => "conflict",
    }
}

fn security_level_name(level: SecurityLevel) -> &'static str {
    match level {
        SecurityLevel::HardwareBacked => "hardware_backed",
        SecurityLevel::OsProtected => "os_protected",
        SecurityLevel::EncryptedLocal => "encrypted_local",
    }
}

fn now_iso8601() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{seconds}Z")
}

fn workspace_manifest() -> PluginManifest {
    PluginManifest {
        id: "workspace".to_string(),
        version: "0.1.0".to_string(),
        display_name: "Workspace".to_string(),
        description: "Logical workspace access and safe edits.".to_string(),
        runtime_api_version: HOST_RUNTIME_API_VERSION.to_string(),
        commands: vec![
            builtin_command(
                "read_file",
                "Read a file from a logical workspace.",
                serde_json::json!({
                    "type": "object",
                    "required": ["workspaceHandle", "path"],
                    "properties": {
                        "workspaceHandle": {"type": "string"},
                        "path": {"type": "string"}
                    }
                }),
                serde_json::json!({
                    "type": "object",
                    "required": ["workspaceHandle", "workspaceRevision", "path", "content", "truncated"],
                    "properties": {
                        "workspaceHandle": {"type": "string"},
                        "workspaceRevision": {"type": "string"},
                        "path": {"type": "string"},
                        "content": {"type": "string"},
                        "truncated": {"type": "boolean"}
                    }
                }),
                SideEffectClass::None,
                vec!["workspace.read"],
                ApprovalExpectation::None,
                false,
                false,
                None,
            ),
            builtin_command(
                "list_dir",
                "List directory entries in a logical workspace.",
                serde_json::json!({
                    "type": "object",
                    "required": ["workspaceHandle"],
                    "properties": {
                        "workspaceHandle": {"type": "string"},
                        "path": {"type": "string"}
                    }
                }),
                serde_json::json!({
                    "type": "object",
                    "required": ["workspaceHandle", "workspaceRevision", "path", "entries"],
                    "properties": {
                        "workspaceHandle": {"type": "string"},
                        "workspaceRevision": {"type": "string"},
                        "path": {"type": "string"},
                        "entries": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "required": ["name", "kind"],
                                "properties": {
                                    "name": {"type": "string"},
                                    "kind": {"type": "string"}
                                }
                            }
                        }
                    }
                }),
                SideEffectClass::None,
                vec!["workspace.read"],
                ApprovalExpectation::None,
                false,
                false,
                None,
            ),
            builtin_command(
                "stat_path",
                "Read path metadata from a logical workspace.",
                serde_json::json!({
                    "type": "object",
                    "required": ["workspaceHandle", "path"],
                    "properties": {
                        "workspaceHandle": {"type": "string"},
                        "path": {"type": "string"}
                    }
                }),
                serde_json::json!({
                    "type": "object",
                    "required": ["workspaceHandle", "workspaceRevision", "path", "kind", "size"],
                    "properties": {
                        "workspaceHandle": {"type": "string"},
                        "workspaceRevision": {"type": "string"},
                        "path": {"type": "string"},
                        "kind": {"type": "string"},
                        "size": {"type": "integer", "minimum": 0}
                    }
                }),
                SideEffectClass::None,
                vec!["workspace.read"],
                ApprovalExpectation::None,
                false,
                false,
                None,
            ),
            builtin_command(
                "write_file_atomic",
                "Write a file atomically into a logical workspace.",
                serde_json::json!({
                    "type": "object",
                    "required": ["workspaceHandle", "path", "content", "expectedRevision"],
                    "properties": {
                        "workspaceHandle": {"type": "string"},
                        "path": {"type": "string"},
                        "content": {"type": "string"},
                        "expectedRevision": {"type": "string"}
                    }
                }),
                serde_json::json!({
                    "type": "object",
                    "required": ["workspaceHandle", "workspaceRevision", "logicalPath", "operation", "bytesWritten", "preconditionStatus", "touched"],
                    "properties": {
                        "workspaceHandle": {"type": "string"},
                        "workspaceRevision": {"type": "string"},
                        "logicalPath": {"type": "string"},
                        "operation": {"type": "string"},
                        "bytesWritten": {"type": "integer", "minimum": 0},
                        "preconditionStatus": {"type": "string"},
                        "touched": {"type": "boolean"}
                    }
                }),
                SideEffectClass::ExternalWrite,
                vec!["workspace.write"],
                ApprovalExpectation::Recommended,
                false,
                false,
                Some(PreconditionModel::WorkspaceRevision),
            ),
            builtin_command(
                "apply_patch",
                "Apply a structured patch to a logical workspace.",
                serde_json::json!({
                    "type": "object",
                    "required": ["workspaceHandle", "patch", "expectedRevision"],
                    "properties": {
                        "workspaceHandle": {"type": "string"},
                        "patch": {"type": "string"},
                        "expectedRevision": {"type": "string"}
                    }
                }),
                serde_json::json!({
                    "type": "object",
                    "required": ["workspaceHandle", "workspaceRevision", "operation", "preconditionStatus", "touchedPaths"],
                    "properties": {
                        "workspaceHandle": {"type": "string"},
                        "workspaceRevision": {"type": "string"},
                        "operation": {"type": "string"},
                        "preconditionStatus": {"type": "string"},
                        "touchedPaths": {"type": "array", "items": {"type": "string"}}
                    }
                }),
                SideEffectClass::ExternalWrite,
                vec!["workspace.write"],
                ApprovalExpectation::Recommended,
                false,
                false,
                Some(PreconditionModel::WorkspaceRevision),
            ),
        ],
        capabilities: CapabilityManifest {
            storage: Default::default(),
            workspace: oberon4_core::WorkspaceCapabilityManifest {
                read: true,
                write: true,
            },
            artifacts: Default::default(),
            services: Vec::new(),
        },
        prompting: builtin_prompting(
            "Use for reading or editing logical workspace files.",
            "Use when a task needs file reads, directory listing, metadata inspection, or contained edits.",
            "Do not use for version control metadata or GitHub remote operations.",
            vec![
                PluginExample {
                    intent: "Read docs/notes.md".to_string(),
                    command: "read_file".to_string(),
                },
                PluginExample {
                    intent: "Apply a patch with revision control".to_string(),
                    command: "apply_patch".to_string(),
                },
            ],
        ),
        provenance: builtin_provenance(),
        lifecycle_actions: Some(builtin_lifecycle()),
        requested_resources: Some(ResourceBudgetRequest {
            memory_bytes: None,
            time_ms: Some(1_000),
            storage_bytes: None,
            output_bytes: None,
            artifact_bytes: None,
            network_policy: None,
        }),
    }
}

fn write_package_tool_manifest(
    package_dir: &Path,
    file_name: &str,
    manifest: &PluginManifest,
) -> Result<(), OberonError> {
    fs::write(
        package_dir.join("tools").join(file_name),
        serde_json::to_vec_pretty(manifest)
            .map_err(|err| OberonError::Storage(format!("tool manifest encode failed: {err}")))?,
    )
    .map_err(|err| OberonError::Storage(format!("tool manifest write failed: {err}")))
}

fn write_package_tool_manifest_to_zip(
    zip: &mut ZipWriter<fs::File>,
    entry_name: &str,
    manifest: &PluginManifest,
) -> Result<(), OberonError> {
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file(entry_name, options)
        .map_err(|err| OberonError::Storage(format!("zip entry create failed: {err}")))?;
    zip.write_all(
        &serde_json::to_vec_pretty(manifest)
            .map_err(|err| OberonError::Storage(format!("tool manifest encode failed: {err}")))?,
    )
    .map_err(|err| OberonError::Storage(format!("tool manifest write failed: {err}")))
}

fn git_manifest() -> PluginManifest {
    PluginManifest {
        id: "git".to_string(),
        version: "0.1.0".to_string(),
        display_name: "Git".to_string(),
        description: "Brokered versioning operations on logical workspaces.".to_string(),
        runtime_api_version: HOST_RUNTIME_API_VERSION.to_string(),
        commands: vec![
            builtin_command(
                "status",
                "Report branch and dirty state for a workspace.",
                serde_json::json!({
                    "type": "object",
                    "required": ["workspaceHandle"],
                    "properties": {"workspaceHandle": {"type": "string"}}
                }),
                serde_json::json!({
                    "type": "object",
                    "required": ["workspaceHandle", "workspaceRevision", "branch", "dirty", "filesChanged"],
                    "properties": {
                        "workspaceHandle": {"type": "string"},
                        "workspaceRevision": {"type": "string"},
                        "branch": {"type": "string"},
                        "dirty": {"type": "boolean"},
                        "filesChanged": {"type": "integer", "minimum": 0}
                    }
                }),
                SideEffectClass::None,
                vec!["workspace.read"],
                ApprovalExpectation::None,
                false,
                false,
                None,
            ),
            builtin_command(
                "diff",
                "Generate a diff artifact for the current workspace state.",
                serde_json::json!({
                    "type": "object",
                    "required": ["workspaceHandle"],
                    "properties": {"workspaceHandle": {"type": "string"}}
                }),
                serde_json::json!({
                    "type": "object",
                    "required": ["workspaceHandle", "workspaceRevision", "branch", "filesChanged", "diffArtifact"],
                    "properties": {
                        "workspaceHandle": {"type": "string"},
                        "workspaceRevision": {"type": "string"},
                        "branch": {"type": "string"},
                        "filesChanged": {"type": "integer", "minimum": 0},
                        "diffArtifact": {"type": "string"}
                    }
                }),
                SideEffectClass::None,
                vec!["workspace.read", "artifact.create"],
                ApprovalExpectation::None,
                false,
                true,
                None,
            ),
            builtin_command(
                "branch.create",
                "Create and switch to a local branch.",
                serde_json::json!({
                    "type": "object",
                    "required": ["workspaceHandle", "name"],
                    "properties": {
                        "workspaceHandle": {"type": "string"},
                        "name": {"type": "string"}
                    }
                }),
                serde_json::json!({
                    "type": "object",
                    "required": ["workspaceHandle", "branch", "created"],
                    "properties": {
                        "workspaceHandle": {"type": "string"},
                        "branch": {"type": "string"},
                        "created": {"type": "boolean"}
                    }
                }),
                SideEffectClass::PluginPrivateWrite,
                vec!["workspace.read"],
                ApprovalExpectation::None,
                false,
                false,
                None,
            ),
            builtin_command(
                "commit",
                "Create a local commit from the workspace state.",
                serde_json::json!({
                    "type": "object",
                    "required": ["workspaceHandle", "message"],
                    "properties": {
                        "workspaceHandle": {"type": "string"},
                        "message": {"type": "string"}
                    }
                }),
                serde_json::json!({
                    "type": "object",
                    "required": ["workspaceHandle", "branch", "headCommit", "dirty", "filesChanged"],
                    "properties": {
                        "workspaceHandle": {"type": "string"},
                        "branch": {"type": "string"},
                        "headCommit": {"type": "string"},
                        "dirty": {"type": "boolean"},
                        "filesChanged": {"type": "integer", "minimum": 0}
                    }
                }),
                SideEffectClass::ExternalWrite,
                vec!["workspace.read"],
                ApprovalExpectation::Recommended,
                false,
                false,
                None,
            ),
            builtin_command(
                "push",
                "Push the current branch through the brokered GitHub remote path.",
                serde_json::json!({
                    "type": "object",
                    "required": ["workspaceHandle", "remote", "branch"],
                    "properties": {
                        "workspaceHandle": {"type": "string"},
                        "remote": {"type": "string"},
                        "branch": {"type": "string"}
                    }
                }),
                serde_json::json!({
                    "type": "object",
                    "required": ["remote", "branch", "commit", "pushed", "provider", "remoteRef"],
                    "properties": {
                        "remote": {"type": "string"},
                        "branch": {"type": "string"},
                        "commit": {"type": "string"},
                        "pushed": {"type": "boolean"},
                        "provider": {"type": "string"},
                        "remoteRef": {"type": "string"}
                    }
                }),
                SideEffectClass::ExternalWrite,
                vec!["workspace.read", "service.github.request"],
                ApprovalExpectation::Required,
                false,
                false,
                None,
            ),
        ],
        capabilities: CapabilityManifest {
            storage: Default::default(),
            workspace: oberon4_core::WorkspaceCapabilityManifest {
                read: true,
                write: false,
            },
            artifacts: oberon4_core::ArtifactCapabilityManifest {
                read: false,
                create: true,
            },
            services: vec!["service.github.request".to_string()],
        },
        prompting: builtin_prompting(
            "Use for brokered version-control operations on a workspace.",
            "Use when you need branch, diff, commit, or push behavior tied to a logical workspace.",
            "Do not use for raw file editing or GitHub metadata lookup.",
            vec![
                PluginExample {
                    intent: "See if the workspace is dirty".to_string(),
                    command: "status".to_string(),
                },
                PluginExample {
                    intent: "Push a reviewed branch".to_string(),
                    command: "push".to_string(),
                },
            ],
        ),
        provenance: builtin_provenance(),
        lifecycle_actions: Some(builtin_lifecycle()),
        requested_resources: Some(ResourceBudgetRequest {
            memory_bytes: None,
            time_ms: Some(2_000),
            storage_bytes: None,
            output_bytes: None,
            artifact_bytes: None,
            network_policy: Some(oberon4_core::NetworkPolicy::BrokeredOnly),
        }),
    }
}

fn github_manifest() -> PluginManifest {
    PluginManifest {
        id: "github".to_string(),
        version: "0.1.0".to_string(),
        display_name: "GitHub".to_string(),
        description: "Brokered GitHub auth and repository metadata operations.".to_string(),
        runtime_api_version: HOST_RUNTIME_API_VERSION.to_string(),
        commands: vec![
            builtin_command(
                "auth_status",
                "Report whether brokered GitHub auth is available.",
                serde_json::json!({"type": "object"}),
                serde_json::json!({
                    "type": "object",
                    "required": ["provider", "authenticated", "securityLevel"],
                    "properties": {
                        "provider": {"type": "string"},
                        "authenticated": {"type": "boolean"},
                        "securityLevel": {"type": "string"}
                    }
                }),
                SideEffectClass::None,
                vec!["service.github.request"],
                ApprovalExpectation::None,
                false,
                false,
                None,
            ),
            builtin_command(
                "clone_to_workspace",
                "Attach a logical workspace to a brokered GitHub repository.",
                serde_json::json!({
                    "type": "object",
                    "required": ["repo"],
                    "properties": {
                        "repo": {"type": "string"}
                    }
                }),
                serde_json::json!({
                    "type": "object",
                    "required": ["provider", "workspaceHandle", "defaultBranch"],
                    "properties": {
                        "provider": {"type": "string"},
                        "workspaceHandle": {"type": "string"},
                        "defaultBranch": {"type": "string"}
                    }
                }),
                SideEffectClass::ExternalWrite,
                vec!["service.github.request", "workspace.write"],
                ApprovalExpectation::Recommended,
                false,
                false,
                None,
            ),
            builtin_command(
                "repo_info",
                "Fetch GitHub repository metadata through the broker.",
                serde_json::json!({
                    "type": "object",
                    "required": ["repo"],
                    "properties": {
                        "repo": {"type": "string"}
                    }
                }),
                serde_json::json!({
                    "type": "object",
                    "required": ["provider", "repo", "defaultBranch", "private"],
                    "properties": {
                        "provider": {"type": "string"},
                        "repo": {"type": "string"},
                        "defaultBranch": {"type": "string"},
                        "private": {"type": "boolean"}
                    }
                }),
                SideEffectClass::ExternalRead,
                vec!["service.github.request"],
                ApprovalExpectation::None,
                false,
                false,
                None,
            ),
        ],
        capabilities: CapabilityManifest {
            storage: Default::default(),
            workspace: oberon4_core::WorkspaceCapabilityManifest {
                read: false,
                write: true,
            },
            artifacts: Default::default(),
            services: vec!["service.github.request".to_string()],
        },
        prompting: builtin_prompting(
            "Use for brokered GitHub authentication and repository metadata.",
            "Use when you need auth status, repository metadata, or to attach a repo-backed workspace.",
            "Do not use for local workspace reads or git diff/commit operations.",
            vec![
                PluginExample {
                    intent: "Check GitHub auth".to_string(),
                    command: "auth_status".to_string(),
                },
                PluginExample {
                    intent: "Attach owner/repo to a workspace".to_string(),
                    command: "clone_to_workspace".to_string(),
                },
            ],
        ),
        provenance: builtin_provenance(),
        lifecycle_actions: Some(builtin_lifecycle()),
        requested_resources: Some(ResourceBudgetRequest {
            memory_bytes: None,
            time_ms: Some(2_000),
            storage_bytes: None,
            output_bytes: None,
            artifact_bytes: None,
            network_policy: Some(oberon4_core::NetworkPolicy::BrokeredOnly),
        }),
    }
}

#[allow(clippy::too_many_arguments)]
fn builtin_command(
    name: &str,
    description: &str,
    input_schema: serde_json::Value,
    output_schema: serde_json::Value,
    side_effects: SideEffectClass,
    required_capabilities: Vec<&str>,
    approval_expectation: ApprovalExpectation,
    supports_planning: bool,
    supports_artifacts: bool,
    precondition_model: Option<PreconditionModel>,
) -> PluginCommandManifest {
    PluginCommandManifest {
        name: name.to_string(),
        version: "1".to_string(),
        description: description.to_string(),
        input_schema,
        output_schema,
        tags: vec!["wp5".to_string(), name.to_string()],
        side_effects,
        required_capabilities: required_capabilities
            .into_iter()
            .map(str::to_string)
            .collect(),
        approval_expectation: Some(approval_expectation),
        supports_planning,
        supports_pagination: false,
        supports_artifacts,
        precondition_model,
        requested_resources: None,
    }
}

fn builtin_prompting(
    summary: &str,
    when_to_use: &str,
    when_not_to_use: &str,
    examples: Vec<PluginExample>,
) -> PromptingManifest {
    PromptingManifest {
        prompt_version: "1".to_string(),
        summary: summary.to_string(),
        when_to_use: when_to_use.to_string(),
        when_not_to_use: when_not_to_use.to_string(),
        examples,
        failure_recovery: vec!["Re-run tool_plan if workspace or auth state changed.".to_string()],
        safety_notes: vec!["All operations are host-brokered and policy-checked.".to_string()],
    }
}

fn builtin_provenance() -> ProvenanceManifest {
    ProvenanceManifest {
        publisher: "oberon4".to_string(),
        source: "first-party-builtin".to_string(),
        checksum: "sha256:first-party-builtin".to_string(),
        signature: Some(SignatureMetadata {
            format: "builtin".to_string(),
            value: "builtin".to_string(),
            key_id: Some("oberon4".to_string()),
        }),
        build: Some(BuildMetadata {
            target: "builtin".to_string(),
            created_at: "2026-03-09T00:00:00Z".to_string(),
        }),
    }
}

fn builtin_lifecycle() -> LifecycleActions {
    LifecycleActions {
        install: false,
        validate: true,
        activate: true,
        deactivate: false,
        upgrade: true,
        uninstall: false,
        hooks: LifecycleHooks {
            on_install: false,
            on_upgrade: false,
            health_check: true,
        },
    }
}
