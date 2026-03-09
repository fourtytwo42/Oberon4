use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use oberon4_core::{
    ArtifactBroker, ArtifactCreateRequest, ArtifactHandle, ArtifactMetadata, ArtifactReadResult,
    ArtifactRetention, ArtifactRetentionPolicy, OberonError, RequestId, WorkspaceBroker,
    WorkspaceConflict, WorkspaceEntry, WorkspaceHandle, WorkspaceListResult, WorkspaceMetadata,
    WorkspaceMutationResult, WorkspacePatchRequest, WorkspacePathKind, WorkspacePathMetadata,
    WorkspacePreconditionStatus, WorkspaceReadResult, WorkspaceWriteRequest, WorkspaceWriteResult,
};

#[derive(Debug, Clone)]
struct WorkspaceRecord {
    workspace_id: String,
    revision: u64,
    created_at: SystemTime,
    updated_at: SystemTime,
    last_mutation_request_id: Option<String>,
}

#[derive(Debug, Clone)]
struct StoredArtifact {
    metadata: ArtifactMetadata,
    path: PathBuf,
    created_at: SystemTime,
}

pub struct PluginRuntimeWorkspaceBroker {
    root: PathBuf,
    records: Mutex<BTreeMap<String, WorkspaceRecord>>,
}

impl PluginRuntimeWorkspaceBroker {
    pub fn new(root: PathBuf) -> Result<Self, OberonError> {
        fs::create_dir_all(&root)
            .map_err(|err| OberonError::Storage(format!("workspace root create failed: {err}")))?;
        Ok(Self {
            root,
            records: Mutex::new(BTreeMap::new()),
        })
    }

    pub fn root_for_handle(&self, workspace: &WorkspaceHandle) -> Result<PathBuf, OberonError> {
        let encoded = hex_name(&workspace.workspace_handle);
        let path = self.root.join(encoded);
        fs::create_dir_all(&path).map_err(|err| {
            OberonError::Storage(format!(
                "workspace directory create failed for '{}': {err}",
                workspace.workspace_handle
            ))
        })?;
        Ok(path)
    }

    fn ensure_record(&self, workspace: &WorkspaceHandle) -> Result<WorkspaceRecord, OberonError> {
        let now = SystemTime::now();
        let _ = self.root_for_handle(workspace)?;
        let mut records = self
            .records
            .lock()
            .map_err(|_| OberonError::Storage("workspace record lock poisoned".to_string()))?;
        let record = records
            .entry(workspace.workspace_handle.clone())
            .or_insert_with(|| WorkspaceRecord {
                workspace_id: workspace.workspace_handle.clone(),
                revision: 0,
                created_at: now,
                updated_at: now,
                last_mutation_request_id: None,
            });
        Ok(record.clone())
    }

    fn metadata_for(
        &self,
        workspace: &WorkspaceHandle,
        record: &WorkspaceRecord,
    ) -> WorkspaceMetadata {
        WorkspaceMetadata {
            workspace_handle: workspace.workspace_handle.clone(),
            workspace_id: record.workspace_id.clone(),
            workspace_revision: revision_string(record.revision),
            created_at: timestamp_string(record.created_at),
            updated_at: timestamp_string(record.updated_at),
            last_mutation_request_id: record.last_mutation_request_id.clone(),
        }
    }

    fn resolve_path(
        &self,
        workspace: &WorkspaceHandle,
        logical_path: &str,
        allow_root: bool,
    ) -> Result<(WorkspaceRecord, String, PathBuf), OberonError> {
        let record = self.ensure_record(workspace)?;
        let root = self.root_for_handle(workspace)?;
        let normalized = normalize_logical_path(logical_path, allow_root)?;
        let path = if normalized.is_empty() {
            root.clone()
        } else {
            root.join(pathbuf_from_logical(&normalized))
        };
        assert_contained(&root, &path)?;
        Ok((record, normalized, path))
    }

    fn update_record_after_write(
        &self,
        workspace_handle: &str,
        request_id: Option<String>,
    ) -> Result<WorkspaceRecord, OberonError> {
        let now = SystemTime::now();
        let mut records = self
            .records
            .lock()
            .map_err(|_| OberonError::Storage("workspace record lock poisoned".to_string()))?;
        let record = records
            .get_mut(workspace_handle)
            .ok_or_else(|| OberonError::NotFound(format!("workspace:{}", workspace_handle)))?;
        record.revision += 1;
        record.updated_at = now;
        record.last_mutation_request_id = request_id;
        Ok(record.clone())
    }

    fn conflict_result(
        &self,
        request: &WorkspaceWriteRequest,
        actual_revision: &str,
        reason: &str,
    ) -> WorkspaceMutationResult {
        WorkspaceMutationResult {
            result: None,
            conflict: Some(WorkspaceConflict {
                workspace_handle: request.workspace_handle.clone(),
                logical_path: request.logical_path.clone(),
                expected_revision: request.expected_revision.clone(),
                actual_revision: actual_revision.to_string(),
                reason: reason.to_string(),
            }),
        }
    }

    fn conflict_result_for_patch(
        &self,
        request: &WorkspacePatchRequest,
        actual_revision: &str,
        reason: &str,
    ) -> WorkspaceMutationResult {
        WorkspaceMutationResult {
            result: None,
            conflict: Some(WorkspaceConflict {
                workspace_handle: request.workspace_handle.clone(),
                logical_path: request.logical_path.clone(),
                expected_revision: request.expected_revision.clone(),
                actual_revision: actual_revision.to_string(),
                reason: reason.to_string(),
            }),
        }
    }
}

impl WorkspaceBroker for PluginRuntimeWorkspaceBroker {
    fn create_or_attach_workspace(
        &self,
        workspace: &WorkspaceHandle,
    ) -> Result<WorkspaceMetadata, OberonError> {
        let record = self.ensure_record(workspace)?;
        Ok(self.metadata_for(workspace, &record))
    }

    fn workspace_metadata(
        &self,
        workspace: &WorkspaceHandle,
    ) -> Result<WorkspaceMetadata, OberonError> {
        let record = self.ensure_record(workspace)?;
        Ok(self.metadata_for(workspace, &record))
    }

    fn list_dir(
        &self,
        workspace: &WorkspaceHandle,
        logical_path: &str,
    ) -> Result<WorkspaceListResult, OberonError> {
        let (record, normalized, path) = self.resolve_path(workspace, logical_path, true)?;
        let metadata = fs::metadata(&path)
            .map_err(|err| OberonError::Storage(format!("workspace metadata failed: {err}")))?;
        if !metadata.is_dir() {
            return Err(OberonError::Validation(format!(
                "logical path '{}' is not a directory",
                normalized
            )));
        }

        let mut entries = Vec::new();
        for entry in fs::read_dir(&path)
            .map_err(|err| OberonError::Storage(format!("workspace read_dir failed: {err}")))?
        {
            let entry = entry.map_err(|err| {
                OberonError::Storage(format!("workspace dir entry failed: {err}"))
            })?;
            let child_path = entry.path();
            assert_contained(&self.root_for_handle(workspace)?, &child_path)?;
            let name = entry.file_name().to_string_lossy().to_string();
            let child_logical = join_logical(&normalized, &name);
            let child_meta = fs::symlink_metadata(&child_path).map_err(|err| {
                OberonError::Storage(format!("workspace symlink metadata failed: {err}"))
            })?;
            let kind = if child_meta.is_dir() {
                WorkspacePathKind::Directory
            } else {
                WorkspacePathKind::File
            };
            entries.push(WorkspacePathMetadata {
                logical_path: child_logical,
                entry: WorkspaceEntry { name, kind },
                size_bytes: if child_meta.is_file() {
                    child_meta.len()
                } else {
                    0
                },
            });
        }
        entries.sort_by(|left, right| left.logical_path.cmp(&right.logical_path));

        Ok(WorkspaceListResult {
            workspace_handle: workspace.workspace_handle.clone(),
            workspace_revision: revision_string(record.revision),
            logical_path: normalized,
            entries,
        })
    }

    fn stat_path(
        &self,
        workspace: &WorkspaceHandle,
        logical_path: &str,
    ) -> Result<WorkspacePathMetadata, OberonError> {
        let (_record, normalized, path) = self.resolve_path(workspace, logical_path, true)?;
        let metadata = fs::symlink_metadata(&path)
            .map_err(|err| OberonError::Storage(format!("workspace stat failed: {err}")))?;
        let name = if normalized.is_empty() {
            ".".to_string()
        } else {
            normalized.rsplit('/').next().unwrap_or(".").to_string()
        };
        let kind = if metadata.is_dir() {
            WorkspacePathKind::Directory
        } else {
            WorkspacePathKind::File
        };
        Ok(WorkspacePathMetadata {
            logical_path: normalized,
            entry: WorkspaceEntry { name, kind },
            size_bytes: if metadata.is_file() {
                metadata.len()
            } else {
                0
            },
        })
    }

    fn read_file(
        &self,
        workspace: &WorkspaceHandle,
        logical_path: &str,
    ) -> Result<WorkspaceReadResult, OberonError> {
        let (record, normalized, path) = self.resolve_path(workspace, logical_path, false)?;
        let content = fs::read_to_string(&path)
            .map_err(|err| OberonError::Storage(format!("workspace read failed: {err}")))?;
        Ok(WorkspaceReadResult {
            workspace_handle: workspace.workspace_handle.clone(),
            workspace_revision: revision_string(record.revision),
            logical_path: normalized,
            bytes_read: content.len() as u64,
            content,
        })
    }

    fn write_file_atomic(
        &self,
        request: &WorkspaceWriteRequest,
    ) -> Result<WorkspaceMutationResult, OberonError> {
        let workspace = WorkspaceHandle {
            workspace_handle: request.workspace_handle.clone(),
        };
        let (record, normalized, path) =
            self.resolve_path(&workspace, &request.logical_path, false)?;
        let actual_revision = revision_string(record.revision);
        if let Some(expected) = &request.expected_revision {
            if expected != &actual_revision {
                return Ok(self.conflict_result(
                    request,
                    &actual_revision,
                    "workspace revision mismatch",
                ));
            }
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                OberonError::Storage(format!("workspace parent create failed: {err}"))
            })?;
            assert_contained(&self.root_for_handle(&workspace)?, parent)?;
        }
        let temp_path = path.with_extension(format!("tmp-{}", RequestId::new().as_str()));
        fs::write(&temp_path, request.content.as_bytes())
            .map_err(|err| OberonError::Storage(format!("workspace temp write failed: {err}")))?;
        fs::rename(&temp_path, &path)
            .map_err(|err| OberonError::Storage(format!("workspace rename failed: {err}")))?;

        let updated =
            self.update_record_after_write(&request.workspace_handle, request.request_id.clone())?;
        Ok(WorkspaceMutationResult {
            result: Some(WorkspaceWriteResult {
                workspace_handle: request.workspace_handle.clone(),
                workspace_revision: revision_string(updated.revision),
                logical_path: normalized,
                operation: "write_file_atomic".to_string(),
                bytes_written: request.content.len() as u64,
                precondition_status: WorkspacePreconditionStatus::Matched,
                touched: true,
            }),
            conflict: None,
        })
    }

    fn apply_patch(
        &self,
        request: &WorkspacePatchRequest,
    ) -> Result<WorkspaceMutationResult, OberonError> {
        let workspace = WorkspaceHandle {
            workspace_handle: request.workspace_handle.clone(),
        };
        let (record, normalized, path) =
            self.resolve_path(&workspace, &request.logical_path, false)?;
        let actual_revision = revision_string(record.revision);
        if let Some(expected) = &request.expected_revision {
            if expected != &actual_revision {
                return Ok(self.conflict_result_for_patch(
                    request,
                    &actual_revision,
                    "workspace revision mismatch",
                ));
            }
        }

        let current = fs::read_to_string(&path)
            .map_err(|err| OberonError::Storage(format!("workspace patch read failed: {err}")))?;
        if !current.contains(&request.search) {
            return Ok(self.conflict_result_for_patch(
                request,
                &actual_revision,
                "patch search text not found",
            ));
        }
        let updated_content = current.replacen(&request.search, &request.replace, 1);

        let temp_path = path.with_extension(format!("tmp-{}", RequestId::new().as_str()));
        fs::write(&temp_path, updated_content.as_bytes()).map_err(|err| {
            OberonError::Storage(format!("workspace temp patch write failed: {err}"))
        })?;
        fs::rename(&temp_path, &path)
            .map_err(|err| OberonError::Storage(format!("workspace patch rename failed: {err}")))?;

        let updated =
            self.update_record_after_write(&request.workspace_handle, request.request_id.clone())?;
        Ok(WorkspaceMutationResult {
            result: Some(WorkspaceWriteResult {
                workspace_handle: request.workspace_handle.clone(),
                workspace_revision: revision_string(updated.revision),
                logical_path: normalized,
                operation: "apply_patch".to_string(),
                bytes_written: request.replace.len() as u64,
                precondition_status: WorkspacePreconditionStatus::Matched,
                touched: true,
            }),
            conflict: None,
        })
    }
}

pub struct PluginRuntimeArtifactBroker {
    root: PathBuf,
    retention_policy: ArtifactRetentionPolicy,
    artifacts: Mutex<BTreeMap<String, StoredArtifact>>,
}

impl PluginRuntimeArtifactBroker {
    pub fn new(
        root: PathBuf,
        retention_policy: ArtifactRetentionPolicy,
    ) -> Result<Self, OberonError> {
        fs::create_dir_all(&root)
            .map_err(|err| OberonError::Storage(format!("artifact root create failed: {err}")))?;
        Ok(Self {
            root,
            retention_policy,
            artifacts: Mutex::new(BTreeMap::new()),
        })
    }

    pub fn retention_policy(&self) -> &ArtifactRetentionPolicy {
        &self.retention_policy
    }

    pub fn sweep_at(&self, now: SystemTime) -> Result<(), OberonError> {
        let mut artifacts = self
            .artifacts
            .lock()
            .map_err(|_| OberonError::Storage("artifact lock poisoned".to_string()))?;
        self.sweep_locked(&mut artifacts, now)
    }

    fn sweep_locked(
        &self,
        artifacts: &mut BTreeMap<String, StoredArtifact>,
        now: SystemTime,
    ) -> Result<(), OberonError> {
        let mut expired = artifacts
            .iter()
            .filter(|(_, stored)| is_expired(stored, &self.retention_policy, now))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        expired.sort();
        for artifact_id in expired {
            remove_artifact_file(artifacts.remove(&artifact_id));
        }

        while total_retained_bytes(artifacts) > self.retention_policy.max_retained_bytes {
            let Some(oldest_id) = artifacts
                .iter()
                .min_by_key(|(_, stored)| stored.created_at)
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            remove_artifact_file(artifacts.remove(&oldest_id));
        }
        Ok(())
    }
}

impl ArtifactBroker for PluginRuntimeArtifactBroker {
    fn create_artifact(
        &self,
        request: &ArtifactCreateRequest,
    ) -> Result<ArtifactMetadata, OberonError> {
        if request.bytes.len() as u64 > self.retention_policy.max_artifact_bytes {
            return Err(OberonError::Storage(format!(
                "artifact exceeds max size: {} > {}",
                request.bytes.len(),
                self.retention_policy.max_artifact_bytes
            )));
        }
        let now = SystemTime::now();
        let artifact_id = format!("art_{}", RequestId::new().as_str());
        let path = self.root.join(format!("{}.bin", hex_name(&artifact_id)));
        fs::write(&path, &request.bytes)
            .map_err(|err| OberonError::Storage(format!("artifact write failed: {err}")))?;
        let metadata = ArtifactMetadata {
            artifact_id: artifact_id.clone(),
            media_type: request.media_type.clone(),
            size: request.bytes.len() as u64,
            retention: request.retention.clone(),
            created_at: timestamp_string(now),
            producer_tool: request.producer_tool.clone(),
            producer_plugin: request.producer_plugin.clone(),
            checksum: None,
        };
        let mut artifacts = self
            .artifacts
            .lock()
            .map_err(|_| OberonError::Storage("artifact lock poisoned".to_string()))?;
        artifacts.insert(
            artifact_id,
            StoredArtifact {
                metadata: metadata.clone(),
                path,
                created_at: now,
            },
        );
        self.sweep_locked(&mut artifacts, now)?;
        if total_retained_bytes(&artifacts) > self.retention_policy.max_retained_bytes {
            return Err(OberonError::Storage(
                "artifact retention pressure exceeded retained budget".to_string(),
            ));
        }
        Ok(metadata)
    }

    fn resolve_artifact(&self, artifact: &ArtifactHandle) -> Result<ArtifactMetadata, OberonError> {
        let artifacts = self
            .artifacts
            .lock()
            .map_err(|_| OberonError::Storage("artifact lock poisoned".to_string()))?;
        artifacts
            .get(&artifact.artifact_id)
            .map(|stored| stored.metadata.clone())
            .ok_or_else(|| OberonError::NotFound(format!("artifact:{}", artifact.artifact_id)))
    }

    fn read_artifact(
        &self,
        artifact: &ArtifactHandle,
        max_bytes: u64,
    ) -> Result<ArtifactReadResult, OberonError> {
        let artifacts = self
            .artifacts
            .lock()
            .map_err(|_| OberonError::Storage("artifact lock poisoned".to_string()))?;
        let stored = artifacts
            .get(&artifact.artifact_id)
            .ok_or_else(|| OberonError::NotFound(format!("artifact:{}", artifact.artifact_id)))?;
        if stored.metadata.size > max_bytes {
            return Err(OberonError::PolicyDenied(format!(
                "artifact '{}' exceeds readable limit {}",
                artifact.artifact_id, max_bytes
            )));
        }
        let bytes = fs::read(&stored.path)
            .map_err(|err| OberonError::Storage(format!("artifact read failed: {err}")))?;
        Ok(ArtifactReadResult {
            metadata: stored.metadata.clone(),
            bytes,
        })
    }
}

fn total_retained_bytes(artifacts: &BTreeMap<String, StoredArtifact>) -> u64 {
    artifacts.values().map(|stored| stored.metadata.size).sum()
}

fn remove_artifact_file(stored: Option<StoredArtifact>) {
    if let Some(stored) = stored {
        let _ = fs::remove_file(stored.path);
    }
}

fn is_expired(stored: &StoredArtifact, policy: &ArtifactRetentionPolicy, now: SystemTime) -> bool {
    let ttl = match stored.metadata.retention {
        ArtifactRetention::Session => Some(Duration::from_secs(policy.session_ttl_seconds)),
        ArtifactRetention::ShortTerm => Some(Duration::from_secs(policy.short_term_ttl_seconds)),
        ArtifactRetention::Durable => None,
    };
    ttl.and_then(|ttl| stored.created_at.checked_add(ttl))
        .map(|deadline| now >= deadline)
        .unwrap_or(false)
}

fn normalize_logical_path(logical_path: &str, allow_root: bool) -> Result<String, OberonError> {
    let trimmed = logical_path.trim();
    if trimmed.is_empty() {
        return if allow_root {
            Ok(String::new())
        } else {
            Err(OberonError::Validation(
                "logical path must not be empty".to_string(),
            ))
        };
    }
    if trimmed.starts_with('/')
        || trimmed.starts_with('\\')
        || trimmed.starts_with("//")
        || trimmed.starts_with("\\\\")
        || is_windows_drive_path(trimmed)
    {
        return Err(OberonError::Validation(format!(
            "logical path '{}' must be relative",
            logical_path
        )));
    }

    let mut segments = Vec::new();
    for segment in trimmed.replace('\\', "/").split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            return Err(OberonError::Validation(format!(
                "logical path '{}' must not traverse upward",
                logical_path
            )));
        }
        if segment.contains(':') {
            return Err(OberonError::Validation(format!(
                "logical path '{}' contains unsupported separator syntax",
                logical_path
            )));
        }
        segments.push(segment.to_string());
    }

    if segments.is_empty() && !allow_root {
        return Err(OberonError::Validation(
            "logical path must not resolve to workspace root".to_string(),
        ));
    }
    Ok(segments.join("/"))
}

fn pathbuf_from_logical(logical_path: &str) -> PathBuf {
    logical_path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .fold(PathBuf::new(), |mut path, segment| {
            path.push(segment);
            path
        })
}

fn assert_contained(root: &Path, candidate: &Path) -> Result<(), OberonError> {
    let canonical_root = fs::canonicalize(root).map_err(|err| {
        OberonError::Storage(format!("workspace root canonicalize failed: {err}"))
    })?;
    let relative = candidate.strip_prefix(root).map_err(|_| {
        OberonError::PolicyDenied("workspace candidate escaped root prefix".to_string())
    })?;

    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        if let Ok(metadata) = fs::symlink_metadata(&current) {
            if metadata.file_type().is_symlink() {
                let resolved = fs::canonicalize(&current).map_err(|err| {
                    OberonError::Storage(format!("workspace symlink canonicalize failed: {err}"))
                })?;
                if !resolved.starts_with(&canonical_root) {
                    return Err(OberonError::PolicyDenied(
                        "workspace path escapes via symlink or reparse point".to_string(),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn join_logical(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

fn revision_string(revision: u64) -> String {
    format!("rev-{revision}")
}

fn timestamp_string(value: SystemTime) -> String {
    value
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn hex_name(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .flat_map(|byte| [nibble_to_hex(byte >> 4), nibble_to_hex(byte & 0x0f)])
        .collect()
}

fn nibble_to_hex(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'a' + (value - 10)) as char,
        _ => '0',
    }
}

fn is_windows_drive_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

#[cfg(test)]
mod tests {
    use super::{
        ArtifactRetentionPolicy, PluginRuntimeArtifactBroker, PluginRuntimeWorkspaceBroker,
    };
    use oberon4_core::{
        ArtifactBroker, ArtifactCreateRequest, ArtifactHandle, ArtifactRetention, WorkspaceBroker,
        WorkspaceHandle, WorkspacePatchRequest, WorkspaceWriteRequest,
    };
    use std::{
        fs,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("oberon4-wp4-{label}-{nonce}"))
    }

    #[test]
    fn workspace_rejects_absolute_and_traversal_paths() {
        let root = temp_dir("containment");
        let broker = PluginRuntimeWorkspaceBroker::new(root.clone()).unwrap();
        let workspace = WorkspaceHandle {
            workspace_handle: "ws".to_string(),
        };
        broker.create_or_attach_workspace(&workspace).unwrap();

        let absolute = broker.read_file(&workspace, "/tmp/x").unwrap_err();
        assert!(absolute.to_string().contains("must be relative"));

        let traversal = broker.read_file(&workspace, "../x").unwrap_err();
        assert!(traversal.to_string().contains("must not traverse"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn workspace_detects_stale_revision_conflict() {
        let root = temp_dir("revision");
        let broker = PluginRuntimeWorkspaceBroker::new(root.clone()).unwrap();
        let workspace = WorkspaceHandle {
            workspace_handle: "ws".to_string(),
        };
        broker.create_or_attach_workspace(&workspace).unwrap();

        let first = broker
            .write_file_atomic(&WorkspaceWriteRequest {
                workspace_handle: "ws".to_string(),
                logical_path: "notes/a.txt".to_string(),
                content: "hello".to_string(),
                expected_revision: Some("rev-0".to_string()),
                request_id: Some("req-1".to_string()),
            })
            .unwrap();
        assert_eq!(
            first.result.unwrap().workspace_revision,
            "rev-1".to_string()
        );

        let stale = broker
            .write_file_atomic(&WorkspaceWriteRequest {
                workspace_handle: "ws".to_string(),
                logical_path: "notes/a.txt".to_string(),
                content: "goodbye".to_string(),
                expected_revision: Some("rev-0".to_string()),
                request_id: Some("req-2".to_string()),
            })
            .unwrap();
        assert!(stale.result.is_none());
        assert_eq!(stale.conflict.unwrap().actual_revision, "rev-1");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn workspace_patch_updates_revision() {
        let root = temp_dir("patch");
        let broker = PluginRuntimeWorkspaceBroker::new(root.clone()).unwrap();
        let workspace = WorkspaceHandle {
            workspace_handle: "ws".to_string(),
        };
        broker.create_or_attach_workspace(&workspace).unwrap();
        broker
            .write_file_atomic(&WorkspaceWriteRequest {
                workspace_handle: "ws".to_string(),
                logical_path: "notes/a.txt".to_string(),
                content: "hello world".to_string(),
                expected_revision: Some("rev-0".to_string()),
                request_id: Some("req-1".to_string()),
            })
            .unwrap();

        let patched = broker
            .apply_patch(&WorkspacePatchRequest {
                workspace_handle: "ws".to_string(),
                logical_path: "notes/a.txt".to_string(),
                search: "world".to_string(),
                replace: "oberon".to_string(),
                expected_revision: Some("rev-1".to_string()),
                request_id: Some("req-2".to_string()),
            })
            .unwrap();
        assert_eq!(
            patched.result.unwrap().workspace_revision,
            "rev-2".to_string()
        );
        assert_eq!(
            broker.read_file(&workspace, "notes/a.txt").unwrap().content,
            "hello oberon"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn workspace_normalizes_backslash_paths() {
        let root = temp_dir("normalize");
        let broker = PluginRuntimeWorkspaceBroker::new(root.clone()).unwrap();
        let workspace = WorkspaceHandle {
            workspace_handle: "ws".to_string(),
        };
        broker.create_or_attach_workspace(&workspace).unwrap();
        broker
            .write_file_atomic(&WorkspaceWriteRequest {
                workspace_handle: "ws".to_string(),
                logical_path: "dir\\nested\\note.txt".to_string(),
                content: "normalized".to_string(),
                expected_revision: Some("rev-0".to_string()),
                request_id: Some("req-1".to_string()),
            })
            .unwrap();

        let stat = broker.stat_path(&workspace, "dir/nested/note.txt").unwrap();
        assert_eq!(stat.logical_path, "dir/nested/note.txt");
        assert_eq!(
            broker
                .read_file(&workspace, "dir/nested/note.txt")
                .unwrap()
                .content,
            "normalized"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn workspace_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let root = temp_dir("symlink");
        let broker = PluginRuntimeWorkspaceBroker::new(root.clone()).unwrap();
        let workspace = WorkspaceHandle {
            workspace_handle: "ws".to_string(),
        };
        broker.create_or_attach_workspace(&workspace).unwrap();
        let workspace_root = broker.root_for_handle(&workspace).unwrap();
        let outside_root = temp_dir("outside");
        fs::create_dir_all(&outside_root).unwrap();
        fs::write(outside_root.join("escape.txt"), "secret").unwrap();
        symlink(&outside_root, workspace_root.join("escape")).unwrap();

        let error = broker
            .read_file(&workspace, "escape/escape.txt")
            .unwrap_err();
        assert!(error.to_string().contains("escapes via symlink"));

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside_root);
    }

    #[test]
    fn artifact_broker_creates_reads_and_evicts() {
        let root = temp_dir("artifact");
        let broker = PluginRuntimeArtifactBroker::new(
            root.clone(),
            ArtifactRetentionPolicy {
                max_artifact_bytes: 64,
                max_retained_bytes: 32,
                session_ttl_seconds: 1,
                short_term_ttl_seconds: 3600,
            },
        )
        .unwrap();

        let first = broker
            .create_artifact(&ArtifactCreateRequest {
                media_type: "text/plain".to_string(),
                retention: ArtifactRetention::ShortTerm,
                bytes: b"first artifact".to_vec(),
                producer_tool: Some("tool".to_string()),
                producer_plugin: Some("plugin".to_string()),
            })
            .unwrap();
        let second = broker
            .create_artifact(&ArtifactCreateRequest {
                media_type: "text/plain".to_string(),
                retention: ArtifactRetention::ShortTerm,
                bytes: b"second artifact is bigger".to_vec(),
                producer_tool: Some("tool".to_string()),
                producer_plugin: Some("plugin".to_string()),
            })
            .unwrap();

        let read = broker
            .read_artifact(
                &ArtifactHandle {
                    artifact_id: second.artifact_id.clone(),
                },
                64,
            )
            .unwrap();
        assert_eq!(read.bytes, b"second artifact is bigger");

        let first_lookup = broker.resolve_artifact(&ArtifactHandle {
            artifact_id: first.artifact_id.clone(),
        });
        assert!(first_lookup.is_err());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn artifact_broker_expires_session_artifacts() {
        let root = temp_dir("artifact-expire");
        let broker = PluginRuntimeArtifactBroker::new(
            root.clone(),
            ArtifactRetentionPolicy {
                max_artifact_bytes: 128,
                max_retained_bytes: 256,
                session_ttl_seconds: 1,
                short_term_ttl_seconds: 3600,
            },
        )
        .unwrap();

        let metadata = broker
            .create_artifact(&ArtifactCreateRequest {
                media_type: "application/json".to_string(),
                retention: ArtifactRetention::Session,
                bytes: br#"{"ok":true}"#.to_vec(),
                producer_tool: None,
                producer_plugin: None,
            })
            .unwrap();
        broker
            .sweep_at(SystemTime::now() + Duration::from_secs(5))
            .unwrap();

        assert!(
            broker
                .resolve_artifact(&ArtifactHandle {
                    artifact_id: metadata.artifact_id,
                })
                .is_err()
        );

        let _ = fs::remove_dir_all(root);
    }
}
