use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use oberon4_core::{
    AllowlistStatus, AppConfig, ApprovalExpectation, ApprovalRequestPayload,
    ApprovalResourceLimits, ArtifactRetentionPolicy, BrokeredServiceResult, CapabilityManifest,
    ConformanceStatus, EffectiveCapabilities, ErrorStatus, GithubRequest, HostToolRuntime,
    InstalledPluginKind, InstalledPluginStatus, InvokeStatus, ManifestValidationIssue,
    ManifestValidationResult, NetworkPolicy, OauthRequest, OberonError, OperatingMode, PlanRequest,
    PlanResult, PlanStatus, PluginAdmissionStatus, PluginCommandManifest, PluginDescription,
    PluginManifest, PluginRegistry, PolicyInspectRequest, PolicyInspectResponse, PolicyResolution,
    PolicyResolver, PreconditionModel, RedactionPipeline, RedactionProfile, RedactionTargetKind,
    RequestId, ResourceLimits, SearchMatch, SearchRequest, SearchResponse, SecretBroker,
    SecretClass, SecurityLevel, ServiceBroker, SideEffectClass, ToolDescribeRequest,
    ToolDescribeResponse, ToolError, ToolGuidance, ToolInvokeResponseEnvelope, ToolTier,
    WorkspaceBroker, WorkspaceHandle, WorkspaceImpact,
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
    pub source_kind: RegisteredToolSource,
    pub package_id: Option<String>,
    pub host_trusted: bool,
    pub enabled: bool,
    pub archive_path: Option<PathBuf>,
    pub archive_entry_manifest_path: Option<String>,
    pub archive_entry_wasm_path: Option<String>,
    pub archive_checksum: Option<String>,
    pub generated_config_path: Option<PathBuf>,
    pub manifest_validation: ManifestValidationResult,
    pub conformance_status: ConformanceStatus,
    pub conformance_issues: Vec<String>,
}

impl RegisteredPlugin {
    fn source_kind_string(&self) -> String {
        match self.source_kind {
            RegisteredToolSource::OrdinaryPlugin => "ordinary-plugin".to_string(),
            RegisteredToolSource::FirstPartyPackage => "first-party-package".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisteredToolSource {
    OrdinaryPlugin,
    FirstPartyPackage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolAvailabilityMetadata {
    pub tool: String,
    pub tool_tier: ToolTier,
    pub source_kind: Option<RegisteredToolSource>,
    pub package_id: Option<String>,
    pub installed: bool,
    pub enabled: bool,
    pub host_trusted: bool,
    pub archive_kind: Option<String>,
    pub archive_checksum: Option<String>,
    pub reason_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginAdmissionMetadata {
    pub is_builtin: bool,
    pub is_host_trusted: bool,
    pub allowlist_status: AllowlistStatus,
    pub conformance_status: ConformanceStatus,
    pub production_eligible: bool,
    pub package_installed: Option<bool>,
    pub package_enabled: Option<bool>,
    pub archive_installed: Option<bool>,
    pub archive_enabled: Option<bool>,
    pub archive_kind: Option<String>,
    pub archive_checksum: Option<String>,
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

const ARCHIVE_MANIFEST_NAME: &str = "oberon-archive.json";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ArchiveKind {
    OrdinaryPlugin,
    FirstPartyPackage,
}

impl ArchiveKind {
    fn extension(&self) -> &'static str {
        match self {
            Self::OrdinaryPlugin => ".obplg.zip",
            Self::FirstPartyPackage => ".obpkg.zip",
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::OrdinaryPlugin => "ordinary-plugin",
            Self::FirstPartyPackage => "first-party-package",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct ArchiveManifest {
    pub id: String,
    pub version: String,
    pub kind: ArchiveKind,
    #[serde(default, rename = "pluginManifestPath")]
    pub plugin_manifest_path: Option<String>,
    #[serde(default, rename = "wasmPath")]
    pub wasm_path: Option<String>,
    #[serde(default, rename = "packageManifestPath")]
    pub package_manifest_path: Option<String>,
    #[serde(default)]
    pub settings: Vec<ArchiveSettingDefinition>,
    #[serde(default)]
    pub signature: Option<oberon4_core::SignatureMetadata>,
    #[serde(default)]
    pub build: Option<oberon4_core::BuildMetadata>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct ArchiveSettingDefinition {
    pub key: String,
    #[serde(rename = "type")]
    pub value_type: String,
    pub default: toml::Value,
    pub description: String,
    pub sensitivity: String,
    #[serde(default, rename = "operatorEditable")]
    pub operator_editable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct InstalledArchiveConfig {
    pub id: String,
    pub kind: String,
    pub version: String,
    #[serde(rename = "archivePath")]
    pub archive_path: String,
    #[serde(rename = "archiveSha256")]
    pub archive_sha256: String,
    pub installed: bool,
    #[serde(rename = "installedAt")]
    pub installed_at: String,
    #[serde(rename = "lastSeenAt")]
    pub last_seen_at: String,
    pub enabled: bool,
    #[serde(default, rename = "disabledReason")]
    pub disabled_reason: Option<String>,
    #[serde(rename = "loadStatus")]
    pub load_status: String,
    #[serde(default, rename = "lastError")]
    pub last_error: Option<String>,
    #[serde(default)]
    pub settings: BTreeMap<String, toml::Value>,
    #[serde(default, rename = "deprecatedSettings")]
    pub deprecated_settings: BTreeMap<String, toml::Value>,
    #[serde(default, rename = "runtimeOverrides")]
    pub runtime_overrides: Option<oberon4_core::ResourceBudgetRequest>,
}

impl InstalledArchiveConfig {
    fn new(
        archive: &DiscoveredArchive,
        settings: BTreeMap<String, toml::Value>,
        now: &str,
    ) -> Self {
        Self {
            id: archive.manifest.id.clone(),
            kind: archive.manifest.kind.as_str().to_string(),
            version: archive.manifest.version.clone(),
            archive_path: archive.path.display().to_string(),
            archive_sha256: archive.sha256.clone(),
            installed: true,
            installed_at: now.to_string(),
            last_seen_at: now.to_string(),
            enabled: true,
            disabled_reason: None,
            load_status: "ready".to_string(),
            last_error: None,
            settings,
            deprecated_settings: BTreeMap::new(),
            runtime_overrides: None,
        }
    }
}

#[derive(Debug, Clone)]
struct DiscoveredArchive {
    path: PathBuf,
    sha256: String,
    manifest: ArchiveManifest,
}

type SyncedArchiveConfigs = BTreeMap<String, (PathBuf, InstalledArchiveConfig)>;
type SyncedArchiveState = (BTreeMap<String, DiscoveredArchive>, SyncedArchiveConfigs);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePluginRegistry {
    runtime_api_version: String,
    plugins: BTreeMap<String, RegisteredPlugin>,
    index: Vec<IndexedCommand>,
    tool_availability: BTreeMap<String, ToolAvailabilityMetadata>,
}

impl FilePluginRegistry {
    pub fn load_from_roots(
        plugin_roots: &[PathBuf],
        runtime_api_version: &str,
    ) -> Result<Self, OberonError> {
        Self::load_from_sources(
            plugin_roots,
            &[],
            &BTreeSet::new(),
            &BTreeSet::new(),
            runtime_api_version,
        )
    }

    pub fn load_from_sources(
        plugin_roots: &[PathBuf],
        package_roots: &[PathBuf],
        enabled_packages: &BTreeSet<String>,
        disabled_packages: &BTreeSet<String>,
        runtime_api_version: &str,
    ) -> Result<Self, OberonError> {
        let mut plugins = BTreeMap::new();
        let mut index = Vec::new();
        let mut tool_availability = BTreeMap::new();
        let conformance_engine = ExecutionEngine::new(RuntimeBrokerFoundations::default())?;

        if package_roots.is_empty() {
            for registered in PluginRuntimeVersionedPackage::registered_plugins() {
                insert_registered_plugin(
                    &mut plugins,
                    &mut index,
                    &mut tool_availability,
                    registered,
                    runtime_api_version,
                    false,
                    Some(&conformance_engine),
                )?;
            }
        }

        for root in package_roots {
            discover_first_party_packages(
                root,
                &mut plugins,
                &mut index,
                &mut tool_availability,
                &BTreeSet::new(),
                enabled_packages,
                disabled_packages,
                runtime_api_version,
                &conformance_engine,
            )?;
        }

        for package_id in enabled_packages {
            if !tool_availability
                .values()
                .any(|metadata| metadata.package_id.as_deref() == Some(package_id.as_str()))
            {
                register_missing_first_party_package(package_id, &mut tool_availability);
            }
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
                    &mut tool_availability,
                    RegisteredPlugin {
                        manifest_path,
                        wasm_path,
                        manifest,
                        tool_tier: ToolTier::OrdinaryPlugin,
                        source_kind: RegisteredToolSource::OrdinaryPlugin,
                        package_id: None,
                        host_trusted: false,
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
                        conformance_status: ConformanceStatus::NotRun,
                        conformance_issues: Vec::new(),
                    },
                    runtime_api_version,
                    true,
                    Some(&conformance_engine),
                )?;
            }
        }

        Ok(Self {
            runtime_api_version: runtime_api_version.to_string(),
            plugins,
            index,
            tool_availability,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn load_from_runtime_sources(
        plugin_archive_roots: &[PathBuf],
        plugin_config_root: &Path,
        enabled_plugins: &BTreeSet<String>,
        disabled_plugins: &BTreeSet<String>,
        plugin_roots: &[PathBuf],
        package_roots: &[PathBuf],
        enabled_packages: &BTreeSet<String>,
        disabled_packages: &BTreeSet<String>,
        runtime_api_version: &str,
    ) -> Result<Self, OberonError> {
        let mut plugins = BTreeMap::new();
        let mut index = Vec::new();
        let mut tool_availability = BTreeMap::new();
        let conformance_engine = ExecutionEngine::new(RuntimeBrokerFoundations::default())?;

        if package_roots.is_empty() && plugin_archive_roots.is_empty() {
            for registered in PluginRuntimeVersionedPackage::registered_plugins() {
                insert_registered_plugin(
                    &mut plugins,
                    &mut index,
                    &mut tool_availability,
                    registered,
                    runtime_api_version,
                    false,
                    Some(&conformance_engine),
                )?;
            }
        }

        fs::create_dir_all(plugin_config_root).map_err(|err| {
            OberonError::Storage(format!(
                "plugin config root create failed ({}): {err}",
                plugin_config_root.display()
            ))
        })?;
        let existing_configs = load_installed_archive_configs(plugin_config_root)?;
        let mut seen_archive_ids = BTreeSet::new();
        for root in plugin_archive_roots {
            discover_archive_plugins(
                root,
                plugin_config_root,
                &existing_configs,
                &mut seen_archive_ids,
                &mut plugins,
                &mut index,
                &mut tool_availability,
                enabled_plugins,
                disabled_plugins,
                runtime_api_version,
                &conformance_engine,
            )?;
        }
        register_missing_archive_configs(
            &existing_configs,
            &seen_archive_ids,
            &mut tool_availability,
        );
        let archive_managed_ids = existing_configs
            .keys()
            .chain(seen_archive_ids.iter())
            .cloned()
            .collect::<BTreeSet<_>>();

        for root in package_roots {
            discover_first_party_packages(
                root,
                &mut plugins,
                &mut index,
                &mut tool_availability,
                &archive_managed_ids,
                enabled_packages,
                disabled_packages,
                runtime_api_version,
                &conformance_engine,
            )?;
        }

        for package_id in enabled_packages {
            if archive_managed_ids.contains(package_id) {
                continue;
            }
            if !tool_availability
                .values()
                .any(|metadata| metadata.package_id.as_deref() == Some(package_id.as_str()))
            {
                register_missing_first_party_package(package_id, &mut tool_availability);
            }
        }

        for root in plugin_roots {
            let manifest_paths = discover_manifest_paths(root)?;
            for manifest_path in manifest_paths {
                let manifest = load_manifest(&manifest_path)?;
                if archive_managed_ids.contains(&manifest.id) {
                    continue;
                }
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
                    &mut tool_availability,
                    RegisteredPlugin {
                        manifest_path,
                        wasm_path,
                        manifest,
                        tool_tier: ToolTier::OrdinaryPlugin,
                        source_kind: RegisteredToolSource::OrdinaryPlugin,
                        package_id: None,
                        host_trusted: false,
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
                        conformance_status: ConformanceStatus::NotRun,
                        conformance_issues: Vec::new(),
                    },
                    runtime_api_version,
                    true,
                    Some(&conformance_engine),
                )?;
            }
        }

        Ok(Self {
            runtime_api_version: runtime_api_version.to_string(),
            plugins,
            index,
            tool_availability,
        })
    }

    pub fn get_registered_plugin(&self, tool: &str) -> Result<&RegisteredPlugin, OberonError> {
        self.plugins
            .get(tool)
            .ok_or_else(|| OberonError::NotFound(format!("plugin:{tool}")))
    }

    pub fn all_registered_plugins(&self) -> Vec<RegisteredPlugin> {
        self.plugins.values().cloned().collect()
    }

    pub fn tool_availability(&self, tool: &str) -> Option<ToolAvailabilityMetadata> {
        self.tool_availability.get(tool).cloned()
    }

    pub fn runtime_api_version(&self) -> &str {
        &self.runtime_api_version
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct FirstPartyPackageManifest {
    pub id: String,
    pub version: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    pub description: String,
    pub tools: Vec<FirstPartyPackageToolManifest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct FirstPartyPackageToolManifest {
    pub tool: String,
    #[serde(rename = "manifestPath")]
    pub manifest_path: String,
}

fn now_timestamp() -> Result<String, OberonError> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| OberonError::Storage(format!("clock failure: {err}")))?
        .as_secs()
        .to_string())
}

fn archive_config_path(config_root: &Path, id: &str) -> PathBuf {
    config_root.join(format!("{id}.toml"))
}

fn discover_archive_paths(root: &Path) -> Result<Vec<PathBuf>, OberonError> {
    let mut archives = Vec::new();
    if !root.exists() {
        return Ok(archives);
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
            } else if let Some(name) = path.file_name().and_then(|value| value.to_str()) {
                if name.ends_with(".obplg.zip") || name.ends_with(".obpkg.zip") {
                    archives.push(path);
                }
            }
        }
    }

    archives.sort();
    Ok(archives)
}

fn read_archive_bytes(path: &Path) -> Result<Vec<u8>, OberonError> {
    fs::read(path).map_err(|err| OberonError::Storage(format!("archive read failed: {err}")))
}

fn compute_archive_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{:x}", digest)
}

fn open_zip_archive(bytes: &[u8]) -> Result<ZipArchive<Cursor<Vec<u8>>>, OberonError> {
    ZipArchive::new(Cursor::new(bytes.to_vec()))
        .map_err(|err| OberonError::ManifestValidation(format!("invalid archive zip: {err}")))
}

fn read_zip_entry_bytes(
    archive: &mut ZipArchive<Cursor<Vec<u8>>>,
    entry_path: &str,
) -> Result<Vec<u8>, OberonError> {
    let mut file = archive.by_name(entry_path).map_err(|err| {
        OberonError::ManifestValidation(format!("missing archive entry '{entry_path}': {err}"))
    })?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|err| OberonError::Storage(format!("archive entry read failed: {err}")))?;
    Ok(bytes)
}

fn read_zip_entry_string(
    archive: &mut ZipArchive<Cursor<Vec<u8>>>,
    entry_path: &str,
) -> Result<String, OberonError> {
    String::from_utf8(read_zip_entry_bytes(archive, entry_path)?)
        .map_err(|err| OberonError::ManifestValidation(format!("archive entry utf8 failed: {err}")))
}

fn load_archive_manifest(path: &Path) -> Result<DiscoveredArchive, OberonError> {
    let bytes = read_archive_bytes(path)?;
    let sha256 = compute_archive_sha256(&bytes);
    let mut archive = open_zip_archive(&bytes)?;
    let raw = read_zip_entry_string(&mut archive, ARCHIVE_MANIFEST_NAME)?;
    let manifest: ArchiveManifest = serde_json::from_str(&raw).map_err(|err| {
        OberonError::ManifestValidation(format!("invalid archive manifest: {err}"))
    })?;

    if !path
        .file_name()
        .and_then(|value| value.to_str())
        .map(|name| name.ends_with(manifest.kind.extension()))
        .unwrap_or(false)
    {
        return Err(OberonError::ManifestValidation(format!(
            "{}: archive extension does not match kind {}",
            path.display(),
            manifest.kind.as_str()
        )));
    }

    Ok(DiscoveredArchive {
        path: path.to_path_buf(),
        sha256,
        manifest,
    })
}

fn load_archive_plugin_manifest(
    archive_path: &Path,
    manifest_path: &str,
) -> Result<PluginManifest, OberonError> {
    let bytes = read_archive_bytes(archive_path)?;
    let mut archive = open_zip_archive(&bytes)?;
    let raw = read_zip_entry_string(&mut archive, manifest_path)?;
    serde_json::from_str(&raw).map_err(|err| {
        OberonError::ManifestValidation(format!("invalid plugin.json in archive: {err}"))
    })
}

fn load_archive_package_manifest(
    archive_path: &Path,
    manifest_path: &str,
) -> Result<FirstPartyPackageManifest, OberonError> {
    let bytes = read_archive_bytes(archive_path)?;
    let mut archive = open_zip_archive(&bytes)?;
    let raw = read_zip_entry_string(&mut archive, manifest_path)?;
    serde_json::from_str(&raw).map_err(|err| {
        OberonError::ManifestValidation(format!("invalid package.json in archive: {err}"))
    })
}

fn archive_setting_defaults(
    settings: &[ArchiveSettingDefinition],
) -> BTreeMap<String, toml::Value> {
    settings
        .iter()
        .map(|setting| (setting.key.clone(), setting.default.clone()))
        .collect()
}

fn validate_setting_value(setting: &ArchiveSettingDefinition, value: &toml::Value) -> bool {
    match setting.value_type.as_str() {
        "string" => value.is_str(),
        "integer" => value.as_integer().is_some(),
        "number" => value.as_float().is_some() || value.as_integer().is_some(),
        "boolean" => value.as_bool().is_some(),
        "array" => value.as_array().is_some(),
        "object" => value.as_table().is_some(),
        _ => false,
    }
}

fn load_installed_archive_configs(
    config_root: &Path,
) -> Result<BTreeMap<String, (PathBuf, InstalledArchiveConfig)>, OberonError> {
    let mut configs = BTreeMap::new();
    if !config_root.exists() {
        return Ok(configs);
    }

    for entry in fs::read_dir(config_root)
        .map_err(|err| OberonError::Storage(format!("config dir read failed: {err}")))?
    {
        let entry =
            entry.map_err(|err| OberonError::Storage(format!("config entry failed: {err}")))?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("toml") {
            continue;
        }
        let raw = fs::read_to_string(&path)
            .map_err(|err| OberonError::Storage(format!("config file read failed: {err}")))?;
        let config: InstalledArchiveConfig = toml::from_str(&raw).map_err(|err| {
            OberonError::ManifestValidation(format!("invalid generated config: {err}"))
        })?;
        configs.insert(config.id.clone(), (path, config));
    }

    Ok(configs)
}

fn write_installed_archive_config(
    config_path: &Path,
    config: &InstalledArchiveConfig,
) -> Result<(), OberonError> {
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| OberonError::Storage(format!("config dir create failed: {err}")))?;
    }
    let encoded = toml::to_string_pretty(config)
        .map_err(|err| OberonError::Storage(format!("config encode failed: {err}")))?;
    fs::write(config_path, encoded)
        .map_err(|err| OberonError::Storage(format!("config write failed: {err}")))
}

fn load_or_create_archive_config(
    config_root: &Path,
    archive: &DiscoveredArchive,
    existing: Option<&(PathBuf, InstalledArchiveConfig)>,
) -> Result<(PathBuf, InstalledArchiveConfig), OberonError> {
    let now = now_timestamp()?;
    let config_path = existing
        .map(|(path, _)| path.clone())
        .unwrap_or_else(|| archive_config_path(config_root, &archive.manifest.id));
    let setting_defaults = archive_setting_defaults(&archive.manifest.settings);

    let mut config = if let Some((_, prior)) = existing {
        let mut prior = prior.clone();
        prior.version = archive.manifest.version.clone();
        prior.kind = archive.manifest.kind.as_str().to_string();
        prior.archive_path = archive.path.display().to_string();
        prior.archive_sha256 = archive.sha256.clone();
        prior.installed = true;
        prior.last_seen_at = now.clone();
        prior.load_status = "ready".to_string();
        prior.last_error = None;

        for (key, value) in &setting_defaults {
            prior
                .settings
                .entry(key.clone())
                .or_insert_with(|| value.clone());
        }

        let mut deprecated = BTreeMap::new();
        let active_keys = archive
            .manifest
            .settings
            .iter()
            .map(|setting| setting.key.as_str())
            .collect::<BTreeSet<_>>();
        let prior_keys = prior.settings.keys().cloned().collect::<Vec<_>>();
        for key in prior_keys {
            if !active_keys.contains(key.as_str()) {
                if let Some(value) = prior.settings.remove(&key) {
                    deprecated.insert(key, value);
                }
            }
        }
        prior.deprecated_settings.extend(deprecated);
        prior
    } else {
        InstalledArchiveConfig::new(archive, setting_defaults, &now)
    };

    for setting in &archive.manifest.settings {
        if let Some(value) = config.settings.get(&setting.key) {
            if !validate_setting_value(setting, value) {
                config.load_status = "invalid-config".to_string();
                config.last_error = Some(format!(
                    "setting '{}' does not match declared type '{}'",
                    setting.key, setting.value_type
                ));
                write_installed_archive_config(&config_path, &config)?;
                return Err(OberonError::Validation(
                    config
                        .last_error
                        .clone()
                        .unwrap_or_else(|| "invalid generated config".to_string()),
                ));
            }
        }
    }

    write_installed_archive_config(&config_path, &config)?;
    Ok((config_path, config))
}

fn archive_roots_from_config(config: &AppConfig) -> Vec<PathBuf> {
    config
        .runtime
        .plugin_archive_roots
        .iter()
        .map(PathBuf::from)
        .collect()
}

fn archive_config_root_from_config(config: &AppConfig) -> PathBuf {
    PathBuf::from(&config.runtime.plugin_config_root)
}

pub(crate) fn normalize_plugin_enablement(
    config: &mut AppConfig,
    id: &str,
    enabled: bool,
) -> Result<(), OberonError> {
    config.runtime.enabled_plugins.retain(|entry| entry != id);
    config.runtime.disabled_plugins.retain(|entry| entry != id);

    if enabled {
        if !config.runtime.enabled_plugins.is_empty() {
            config.runtime.enabled_plugins.push(id.to_string());
            config.runtime.enabled_plugins.sort();
            config.runtime.enabled_plugins.dedup();
        }
    } else {
        config.runtime.disabled_plugins.push(id.to_string());
        config.runtime.disabled_plugins.sort();
        config.runtime.disabled_plugins.dedup();
    }

    Ok(())
}

fn discovered_archive_to_status(
    archive: &DiscoveredArchive,
    config_path: Option<&Path>,
    config: Option<&InstalledArchiveConfig>,
    enabled_plugins: &BTreeSet<String>,
    disabled_plugins: &BTreeSet<String>,
) -> InstalledPluginStatus {
    let enabled_by_generated_config = config.map(|entry| entry.enabled).unwrap_or(true);
    let explicitly_enabled =
        enabled_plugins.is_empty() || enabled_plugins.contains(archive.manifest.id.as_str());
    let enabled = enabled_by_generated_config
        && explicitly_enabled
        && !disabled_plugins.contains(archive.manifest.id.as_str());

    let mut reason_codes = Vec::new();
    if !enabled_by_generated_config || disabled_plugins.contains(archive.manifest.id.as_str()) {
        reason_codes.push("blocked_by_disable".to_string());
    }
    if !explicitly_enabled {
        reason_codes.push("blocked_by_disable".to_string());
    }

    InstalledPluginStatus {
        id: archive.manifest.id.clone(),
        version: archive.manifest.version.clone(),
        kind: match archive.manifest.kind {
            ArchiveKind::OrdinaryPlugin => InstalledPluginKind::OrdinaryPlugin,
            ArchiveKind::FirstPartyPackage => InstalledPluginKind::FirstPartyPackage,
        },
        host_trusted: matches!(archive.manifest.kind, ArchiveKind::FirstPartyPackage),
        installed: true,
        enabled,
        archive_path: Some(archive.path.display().to_string()),
        archive_checksum: Some(archive.sha256.clone()),
        config_path: config_path.map(|path| path.display().to_string()),
        load_status: config.map(|entry| entry.load_status.clone()),
        last_error: config.and_then(|entry| entry.last_error.clone()),
        reason_codes,
    }
}

fn missing_archive_status(
    config_path: &Path,
    config: &InstalledArchiveConfig,
    enabled_plugins: &BTreeSet<String>,
    disabled_plugins: &BTreeSet<String>,
) -> InstalledPluginStatus {
    let enabled_by_generated_config = config.enabled;
    let explicitly_enabled =
        enabled_plugins.is_empty() || enabled_plugins.contains(config.id.as_str());
    let enabled = enabled_by_generated_config
        && explicitly_enabled
        && !disabled_plugins.contains(config.id.as_str());

    let mut reason_codes = vec!["blocked_by_missing_archive".to_string()];
    if !enabled_by_generated_config || disabled_plugins.contains(config.id.as_str()) {
        reason_codes.push("blocked_by_disable".to_string());
    }
    if !explicitly_enabled {
        reason_codes.push("blocked_by_disable".to_string());
    }

    InstalledPluginStatus {
        id: config.id.clone(),
        version: config.version.clone(),
        kind: match config.kind.as_str() {
            "first-party-package" => InstalledPluginKind::FirstPartyPackage,
            _ => InstalledPluginKind::OrdinaryPlugin,
        },
        host_trusted: config.kind == "first-party-package",
        installed: false,
        enabled,
        archive_path: Some(config.archive_path.clone()),
        archive_checksum: Some(config.archive_sha256.clone()),
        config_path: Some(config_path.display().to_string()),
        load_status: Some(config.load_status.clone()),
        last_error: config.last_error.clone(),
        reason_codes,
    }
}

fn sync_archive_configs_for_roots(
    archive_roots: &[PathBuf],
    config_root: &Path,
) -> Result<SyncedArchiveState, OberonError> {
    let existing_configs = load_installed_archive_configs(config_root)?;
    let mut synced_configs = BTreeMap::new();
    let mut discovered_archives = BTreeMap::new();

    for root in archive_roots {
        for archive_path in discover_archive_paths(root)? {
            let archive = load_archive_manifest(&archive_path)?;
            let existing = existing_configs.get(&archive.manifest.id);
            let (config_path, config) =
                load_or_create_archive_config(config_root, &archive, existing)?;
            synced_configs.insert(archive.manifest.id.clone(), (config_path, config));
            discovered_archives.insert(archive.manifest.id.clone(), archive);
        }
    }

    for (id, value) in existing_configs {
        synced_configs.entry(id).or_insert(value);
    }

    Ok((discovered_archives, synced_configs))
}

pub fn list_installed_plugins(
    config: &AppConfig,
) -> Result<Vec<InstalledPluginStatus>, OberonError> {
    let archive_roots = archive_roots_from_config(config);
    let config_root = archive_config_root_from_config(config);
    let enabled_plugins = config
        .runtime
        .enabled_plugins
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let disabled_plugins = config
        .runtime
        .disabled_plugins
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();

    let (discovered, configs) = sync_archive_configs_for_roots(&archive_roots, &config_root)?;
    let mut statuses = Vec::new();

    for (id, archive) in &discovered {
        let (config_path, config) = configs
            .get(id)
            .ok_or_else(|| OberonError::NotFound(format!("plugin-config:{id}")))?;
        statuses.push(discovered_archive_to_status(
            archive,
            Some(config_path.as_path()),
            Some(config),
            &enabled_plugins,
            &disabled_plugins,
        ));
    }

    for (id, (config_path, installed_config)) in &configs {
        if discovered.contains_key(id) {
            continue;
        }
        statuses.push(missing_archive_status(
            config_path,
            installed_config,
            &enabled_plugins,
            &disabled_plugins,
        ));
    }

    statuses.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(statuses)
}

pub fn install_plugin_archive(
    config: &AppConfig,
    archive_path: &Path,
) -> Result<InstalledPluginStatus, OberonError> {
    let archive_roots = archive_roots_from_config(config);
    let config_root = archive_config_root_from_config(config);
    let target_root = archive_roots.first().cloned().ok_or_else(|| {
        OberonError::Validation(
            "runtime.plugin_archive_roots must contain at least one path".to_string(),
        )
    })?;
    fs::create_dir_all(&target_root)
        .map_err(|err| OberonError::Storage(format!("archive root create failed: {err}")))?;

    let source_archive = load_archive_manifest(archive_path)?;
    let extension = source_archive.manifest.kind.extension();
    if archive_path
        .file_name()
        .and_then(|value| value.to_str())
        .is_none_or(|name| !name.ends_with(extension))
    {
        return Err(OberonError::Validation(format!(
            "archive extension does not match manifest kind; expected {extension}"
        )));
    }

    for root in &archive_roots {
        for existing_archive_path in discover_archive_paths(root)? {
            let existing_archive = load_archive_manifest(&existing_archive_path)?;
            if existing_archive.manifest.id == source_archive.manifest.id {
                let _ = fs::remove_file(existing_archive_path);
            }
        }
    }

    let destination_path = target_root.join(format!(
        "{}-{}{}",
        source_archive.manifest.id, source_archive.manifest.version, extension
    ));
    if archive_path != destination_path {
        fs::copy(archive_path, &destination_path)
            .map_err(|err| OberonError::Storage(format!("archive copy failed: {err}")))?;
    }

    let installed_archive = load_archive_manifest(&destination_path)?;
    let existing_configs = load_installed_archive_configs(&config_root)?;
    let existing = existing_configs.get(&installed_archive.manifest.id);
    let (config_path, config_file) =
        load_or_create_archive_config(&config_root, &installed_archive, existing)?;

    Ok(discovered_archive_to_status(
        &installed_archive,
        Some(config_path.as_path()),
        Some(&config_file),
        &config
            .runtime
            .enabled_plugins
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>(),
        &config
            .runtime
            .disabled_plugins
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>(),
    ))
}

pub fn set_plugin_enabled(
    config: &AppConfig,
    id: &str,
    enabled: bool,
) -> Result<InstalledPluginStatus, OberonError> {
    let archive_roots = archive_roots_from_config(config);
    let config_root = archive_config_root_from_config(config);
    let discovered = sync_archive_configs_for_roots(&archive_roots, &config_root)?.0;
    let existing_configs = load_installed_archive_configs(&config_root)?;

    let (config_path, mut installed_config) = existing_configs
        .get(id)
        .cloned()
        .ok_or_else(|| OberonError::NotFound(format!("plugin:{id}")))?;
    installed_config.enabled = enabled;
    installed_config.disabled_reason = if enabled {
        None
    } else {
        Some("operator_disabled".to_string())
    };
    installed_config.last_error = None;
    write_installed_archive_config(&config_path, &installed_config)?;

    let mut effective_config = config.clone();
    normalize_plugin_enablement(&mut effective_config, id, enabled)?;
    let enabled_plugins = effective_config
        .runtime
        .enabled_plugins
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let disabled_plugins = effective_config
        .runtime
        .disabled_plugins
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();

    Ok(match discovered.get(id) {
        Some(archive) => discovered_archive_to_status(
            archive,
            Some(config_path.as_path()),
            Some(&installed_config),
            &enabled_plugins,
            &disabled_plugins,
        ),
        None => missing_archive_status(
            config_path.as_path(),
            &installed_config,
            &enabled_plugins,
            &disabled_plugins,
        ),
    })
}

pub fn unload_plugin_archive(
    config: &AppConfig,
    id: &str,
) -> Result<InstalledPluginStatus, OberonError> {
    let archive_roots = archive_roots_from_config(config);
    let config_root = archive_config_root_from_config(config);
    let existing_configs = load_installed_archive_configs(&config_root)?;
    let mut removed = false;

    for root in &archive_roots {
        for archive_path in discover_archive_paths(root)? {
            let archive = load_archive_manifest(&archive_path)?;
            if archive.manifest.id == id {
                fs::remove_file(&archive_path)
                    .map_err(|err| OberonError::Storage(format!("archive remove failed: {err}")))?;
                removed = true;
            }
        }
    }

    let (config_path, mut installed_config) = existing_configs
        .get(id)
        .cloned()
        .ok_or_else(|| OberonError::NotFound(format!("plugin:{id}")))?;
    installed_config.installed = false;
    installed_config.load_status = "missing-archive".to_string();
    installed_config.last_error = Some("archive removed by operator".to_string());
    installed_config.last_seen_at = now_timestamp()?;
    write_installed_archive_config(&config_path, &installed_config)?;

    if !removed {
        return Ok(missing_archive_status(
            config_path.as_path(),
            &installed_config,
            &config
                .runtime
                .enabled_plugins
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>(),
            &config
                .runtime
                .disabled_plugins
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>(),
        ));
    }

    Ok(missing_archive_status(
        config_path.as_path(),
        &installed_config,
        &config
            .runtime
            .enabled_plugins
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>(),
        &config
            .runtime
            .disabled_plugins
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>(),
    ))
}

#[allow(clippy::too_many_arguments)]
fn discover_archive_plugins(
    root: &Path,
    config_root: &Path,
    existing_configs: &BTreeMap<String, (PathBuf, InstalledArchiveConfig)>,
    seen_archive_ids: &mut BTreeSet<String>,
    plugins: &mut BTreeMap<String, RegisteredPlugin>,
    index: &mut Vec<IndexedCommand>,
    tool_availability: &mut BTreeMap<String, ToolAvailabilityMetadata>,
    enabled_plugins: &BTreeSet<String>,
    disabled_plugins: &BTreeSet<String>,
    runtime_api_version: &str,
    conformance_engine: &ExecutionEngine,
) -> Result<(), OberonError> {
    for archive_path in discover_archive_paths(root)? {
        let archive = load_archive_manifest(&archive_path)?;
        let archive_id = archive.manifest.id.clone();
        seen_archive_ids.insert(archive_id.clone());

        let existing = existing_configs.get(&archive_id);
        let config_result = load_or_create_archive_config(config_root, &archive, existing);
        let (config_path, config) = match config_result {
            Ok(value) => value,
            Err(err) => {
                register_invalid_archive_config(&archive, config_root, tool_availability, err)?;
                continue;
            }
        };

        let enabled_by_config = config.enabled && !disabled_plugins.contains(&archive_id);
        let explicitly_enabled =
            enabled_plugins.is_empty() || enabled_plugins.contains(&archive_id);
        let archive_enabled = enabled_by_config && explicitly_enabled;

        if !archive_enabled {
            register_archive_disabled_state(&archive, tool_availability);
            continue;
        }

        match archive.manifest.kind {
            ArchiveKind::OrdinaryPlugin => {
                let manifest_path =
                    archive
                        .manifest
                        .plugin_manifest_path
                        .clone()
                        .ok_or_else(|| {
                            OberonError::ManifestValidation(format!(
                                "{}: ordinary-plugin archive missing pluginManifestPath",
                                archive.path.display()
                            ))
                        })?;
                let wasm_path = archive.manifest.wasm_path.clone().ok_or_else(|| {
                    OberonError::ManifestValidation(format!(
                        "{}: ordinary-plugin archive missing wasmPath",
                        archive.path.display()
                    ))
                })?;
                let manifest = load_archive_plugin_manifest(&archive.path, &manifest_path)?;
                if manifest.id != archive.manifest.id
                    || manifest.version != archive.manifest.version
                {
                    return Err(OberonError::ManifestValidation(format!(
                        "{}: archive manifest id/version does not match plugin.json",
                        archive.path.display()
                    )));
                }
                insert_registered_plugin(
                    plugins,
                    index,
                    tool_availability,
                    RegisteredPlugin {
                        manifest_path: PathBuf::from(format!(
                            "{}::{manifest_path}",
                            archive.path.display()
                        )),
                        wasm_path: PathBuf::from(format!(
                            "{}::{wasm_path}",
                            archive.path.display()
                        )),
                        manifest,
                        tool_tier: ToolTier::OrdinaryPlugin,
                        source_kind: RegisteredToolSource::OrdinaryPlugin,
                        package_id: None,
                        host_trusted: false,
                        enabled: true,
                        archive_path: Some(archive.path.clone()),
                        archive_entry_manifest_path: Some(manifest_path),
                        archive_entry_wasm_path: Some(wasm_path),
                        archive_checksum: Some(archive.sha256.clone()),
                        generated_config_path: Some(config_path.clone()),
                        manifest_validation: ManifestValidationResult {
                            valid: true,
                            issues: Vec::new(),
                        },
                        conformance_status: ConformanceStatus::NotRun,
                        conformance_issues: Vec::new(),
                    },
                    runtime_api_version,
                    true,
                    Some(conformance_engine),
                )?;
            }
            ArchiveKind::FirstPartyPackage => {
                let manifest_path =
                    archive
                        .manifest
                        .package_manifest_path
                        .clone()
                        .ok_or_else(|| {
                            OberonError::ManifestValidation(format!(
                                "{}: first-party-package archive missing packageManifestPath",
                                archive.path.display()
                            ))
                        })?;
                let package = load_archive_package_manifest(&archive.path, &manifest_path)?;
                if package.id != archive.manifest.id || package.version != archive.manifest.version
                {
                    return Err(OberonError::ManifestValidation(format!(
                        "{}: archive manifest id/version does not match package.json",
                        archive.path.display()
                    )));
                }
                for tool in &package.tools {
                    let entry_path = tool.manifest_path.clone();
                    let manifest = load_archive_plugin_manifest(&archive.path, &entry_path)?;
                    insert_registered_plugin(
                        plugins,
                        index,
                        tool_availability,
                        RegisteredPlugin {
                            manifest_path: PathBuf::from(format!(
                                "{}::{entry_path}",
                                archive.path.display()
                            )),
                            wasm_path: archive.path.clone(),
                            manifest,
                            tool_tier: ToolTier::BrokeredPrivileged,
                            source_kind: RegisteredToolSource::FirstPartyPackage,
                            package_id: Some(package.id.clone()),
                            host_trusted: true,
                            enabled: true,
                            archive_path: Some(archive.path.clone()),
                            archive_entry_manifest_path: Some(entry_path),
                            archive_entry_wasm_path: None,
                            archive_checksum: Some(archive.sha256.clone()),
                            generated_config_path: Some(config_path.clone()),
                            manifest_validation: ManifestValidationResult {
                                valid: true,
                                issues: Vec::new(),
                            },
                            conformance_status: ConformanceStatus::Passed,
                            conformance_issues: Vec::new(),
                        },
                        runtime_api_version,
                        false,
                        Some(conformance_engine),
                    )?;
                }
            }
        }
    }

    Ok(())
}

fn register_archive_disabled_state(
    archive: &DiscoveredArchive,
    tool_availability: &mut BTreeMap<String, ToolAvailabilityMetadata>,
) {
    match archive.manifest.kind {
        ArchiveKind::OrdinaryPlugin => {
            tool_availability.insert(
                archive.manifest.id.clone(),
                ToolAvailabilityMetadata {
                    tool: archive.manifest.id.clone(),
                    tool_tier: ToolTier::OrdinaryPlugin,
                    source_kind: Some(RegisteredToolSource::OrdinaryPlugin),
                    package_id: None,
                    installed: true,
                    enabled: false,
                    host_trusted: false,
                    archive_kind: Some(archive.manifest.kind.as_str().to_string()),
                    archive_checksum: Some(archive.sha256.clone()),
                    reason_codes: vec!["blocked_by_disable".to_string()],
                },
            );
        }
        ArchiveKind::FirstPartyPackage => {
            register_package_tool_state(
                &FirstPartyPackageManifest {
                    id: archive.manifest.id.clone(),
                    version: archive.manifest.version.clone(),
                    display_name: archive.manifest.id.clone(),
                    description: String::new(),
                    tools: PluginRuntimeVersionedPackage::tool_ids()
                        .iter()
                        .map(|tool| FirstPartyPackageToolManifest {
                            tool: (*tool).to_string(),
                            manifest_path: String::new(),
                        })
                        .collect(),
                },
                tool_availability,
                true,
                false,
                vec!["blocked_by_disable".to_string()],
            );
            for tool in PluginRuntimeVersionedPackage::tool_ids() {
                if let Some(metadata) = tool_availability.get_mut(*tool) {
                    metadata.archive_kind = Some(archive.manifest.kind.as_str().to_string());
                    metadata.archive_checksum = Some(archive.sha256.clone());
                }
            }
        }
    }
}

fn register_invalid_archive_config(
    archive: &DiscoveredArchive,
    config_root: &Path,
    tool_availability: &mut BTreeMap<String, ToolAvailabilityMetadata>,
    err: OberonError,
) -> Result<(), OberonError> {
    let config_path = archive_config_path(config_root, &archive.manifest.id);
    let now = now_timestamp()?;
    let mut config = InstalledArchiveConfig::new(
        archive,
        archive_setting_defaults(&archive.manifest.settings),
        &now,
    );
    config.load_status = "invalid-config".to_string();
    config.last_error = Some(err.to_string());
    write_installed_archive_config(&config_path, &config)?;

    match archive.manifest.kind {
        ArchiveKind::OrdinaryPlugin => {
            tool_availability.insert(
                archive.manifest.id.clone(),
                ToolAvailabilityMetadata {
                    tool: archive.manifest.id.clone(),
                    tool_tier: ToolTier::OrdinaryPlugin,
                    source_kind: Some(RegisteredToolSource::OrdinaryPlugin),
                    package_id: None,
                    installed: true,
                    enabled: false,
                    host_trusted: false,
                    archive_kind: Some(archive.manifest.kind.as_str().to_string()),
                    archive_checksum: Some(archive.sha256.clone()),
                    reason_codes: vec!["blocked_by_invalid_config".to_string()],
                },
            );
        }
        ArchiveKind::FirstPartyPackage => {
            for tool in PluginRuntimeVersionedPackage::tool_ids() {
                tool_availability.insert(
                    (*tool).to_string(),
                    ToolAvailabilityMetadata {
                        tool: (*tool).to_string(),
                        tool_tier: ToolTier::BrokeredPrivileged,
                        source_kind: Some(RegisteredToolSource::FirstPartyPackage),
                        package_id: Some(archive.manifest.id.clone()),
                        installed: true,
                        enabled: false,
                        host_trusted: true,
                        archive_kind: Some(archive.manifest.kind.as_str().to_string()),
                        archive_checksum: Some(archive.sha256.clone()),
                        reason_codes: vec!["blocked_by_invalid_config".to_string()],
                    },
                );
            }
        }
    }

    Ok(())
}

fn register_missing_archive_configs(
    existing_configs: &BTreeMap<String, (PathBuf, InstalledArchiveConfig)>,
    seen_archive_ids: &BTreeSet<String>,
    tool_availability: &mut BTreeMap<String, ToolAvailabilityMetadata>,
) {
    for (id, (_, config)) in existing_configs {
        if seen_archive_ids.contains(id) || !config.enabled {
            continue;
        }
        if config.kind == "ordinary-plugin" {
            tool_availability.insert(
                id.clone(),
                ToolAvailabilityMetadata {
                    tool: id.clone(),
                    tool_tier: ToolTier::OrdinaryPlugin,
                    source_kind: Some(RegisteredToolSource::OrdinaryPlugin),
                    package_id: None,
                    installed: false,
                    enabled: true,
                    host_trusted: false,
                    archive_kind: Some(config.kind.clone()),
                    archive_checksum: Some(config.archive_sha256.clone()),
                    reason_codes: vec!["blocked_by_missing_archive".to_string()],
                },
            );
        } else if id == PluginRuntimeVersionedPackage::package_id() {
            for tool in PluginRuntimeVersionedPackage::tool_ids() {
                tool_availability.insert(
                    (*tool).to_string(),
                    ToolAvailabilityMetadata {
                        tool: (*tool).to_string(),
                        tool_tier: ToolTier::BrokeredPrivileged,
                        source_kind: Some(RegisteredToolSource::FirstPartyPackage),
                        package_id: Some(id.clone()),
                        installed: false,
                        enabled: true,
                        host_trusted: true,
                        archive_kind: Some(config.kind.clone()),
                        archive_checksum: Some(config.archive_sha256.clone()),
                        reason_codes: vec!["blocked_by_missing_archive".to_string()],
                    },
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn discover_first_party_packages(
    root: &Path,
    plugins: &mut BTreeMap<String, RegisteredPlugin>,
    index: &mut Vec<IndexedCommand>,
    tool_availability: &mut BTreeMap<String, ToolAvailabilityMetadata>,
    archive_managed_ids: &BTreeSet<String>,
    enabled_packages: &BTreeSet<String>,
    disabled_packages: &BTreeSet<String>,
    runtime_api_version: &str,
    conformance_engine: &ExecutionEngine,
) -> Result<(), OberonError> {
    for package_manifest_path in discover_package_manifest_paths(root)? {
        let package = load_package_manifest(&package_manifest_path)?;
        if archive_managed_ids.contains(&package.id) {
            continue;
        }
        let package_enabled = (enabled_packages.is_empty()
            || enabled_packages.contains(&package.id))
            && !disabled_packages.contains(&package.id);
        if !package_enabled {
            register_package_tool_state(
                &package,
                tool_availability,
                true,
                false,
                vec!["blocked_by_disable".to_string()],
            );
            continue;
        }

        let package_dir = package_manifest_path
            .parent()
            .ok_or_else(|| {
                OberonError::ManifestValidation(format!(
                    "{}: package manifest has no parent directory",
                    package_manifest_path.display()
                ))
            })?
            .to_path_buf();

        for tool in &package.tools {
            let manifest_path = package_dir.join(&tool.manifest_path);
            let manifest = load_manifest(&manifest_path)?;
            insert_registered_plugin(
                plugins,
                index,
                tool_availability,
                RegisteredPlugin {
                    manifest_path: manifest_path.clone(),
                    wasm_path: package_manifest_path.clone(),
                    manifest,
                    tool_tier: ToolTier::BrokeredPrivileged,
                    source_kind: RegisteredToolSource::FirstPartyPackage,
                    package_id: Some(package.id.clone()),
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
                runtime_api_version,
                false,
                Some(conformance_engine),
            )?;
        }
    }
    Ok(())
}

fn register_missing_first_party_package(
    package_id: &str,
    tool_availability: &mut BTreeMap<String, ToolAvailabilityMetadata>,
) {
    if package_id != PluginRuntimeVersionedPackage::package_id() {
        return;
    }
    for tool in PluginRuntimeVersionedPackage::tool_ids() {
        tool_availability.insert(
            tool.to_string(),
            ToolAvailabilityMetadata {
                tool: tool.to_string(),
                tool_tier: ToolTier::BrokeredPrivileged,
                source_kind: Some(RegisteredToolSource::FirstPartyPackage),
                package_id: Some(package_id.to_string()),
                installed: false,
                enabled: true,
                host_trusted: true,
                archive_kind: None,
                archive_checksum: None,
                reason_codes: vec!["blocked_by_missing_package".to_string()],
            },
        );
    }
}

fn register_package_tool_state(
    package: &FirstPartyPackageManifest,
    tool_availability: &mut BTreeMap<String, ToolAvailabilityMetadata>,
    installed: bool,
    enabled: bool,
    reason_codes: Vec<String>,
) {
    for tool in &package.tools {
        tool_availability.insert(
            tool.tool.clone(),
            ToolAvailabilityMetadata {
                tool: tool.tool.clone(),
                tool_tier: ToolTier::BrokeredPrivileged,
                source_kind: Some(RegisteredToolSource::FirstPartyPackage),
                package_id: Some(package.id.clone()),
                installed,
                enabled,
                host_trusted: true,
                archive_kind: None,
                archive_checksum: None,
                reason_codes: reason_codes.clone(),
            },
        );
    }
}

pub trait RegisteredPluginAccess: PluginRegistry {
    fn registered_plugin(&self, tool: &str) -> Result<RegisteredPlugin, OberonError>;
    fn tool_availability(&self, tool: &str) -> Option<ToolAvailabilityMetadata>;
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
                        &OperatingMode::Dev,
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

    fn tool_availability(&self, tool: &str) -> Option<ToolAvailabilityMetadata> {
        self.tool_availability(tool)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticPolicyResolver {
    operating_mode: OperatingMode,
    dev_limits: ResourceLimits,
    production_limits: ResourceLimits,
    allowlisted_checksums: BTreeSet<String>,
    plugin_admission: BTreeMap<String, PluginAdmissionMetadata>,
    blocked_capabilities: BTreeSet<String>,
}

impl Default for StaticPolicyResolver {
    fn default() -> Self {
        Self {
            operating_mode: OperatingMode::Dev,
            dev_limits: ResourceLimits {
                memory_bytes: Some(134_217_728),
                time_ms: Some(5_000),
                storage_bytes: Some(268_435_456),
                output_bytes: Some(131_072),
                network_policy: Some(NetworkPolicy::Denied),
            },
            production_limits: ResourceLimits {
                memory_bytes: Some(100_663_296),
                time_ms: Some(3_000),
                storage_bytes: Some(134_217_728),
                output_bytes: Some(65_536),
                network_policy: Some(NetworkPolicy::Denied),
            },
            allowlisted_checksums: BTreeSet::new(),
            plugin_admission: BTreeMap::new(),
            blocked_capabilities: BTreeSet::new(),
        }
    }
}

impl StaticPolicyResolver {
    pub fn new(
        operating_mode: OperatingMode,
        dev_limits: ResourceLimits,
        production_limits: ResourceLimits,
        allowlisted_checksums: BTreeSet<String>,
        plugin_admission: BTreeMap<String, PluginAdmissionMetadata>,
        blocked_capabilities: BTreeSet<String>,
    ) -> Self {
        Self {
            operating_mode,
            dev_limits,
            production_limits,
            allowlisted_checksums,
            plugin_admission,
            blocked_capabilities,
        }
    }

    pub fn for_registered_plugins(
        operating_mode: OperatingMode,
        allowlisted_checksums: impl IntoIterator<Item = String>,
        plugins: impl IntoIterator<Item = RegisteredPlugin>,
    ) -> Self {
        let defaults = Self::default();
        Self::with_limits_for_registered_plugins(
            operating_mode,
            defaults.dev_limits,
            defaults.production_limits,
            allowlisted_checksums,
            plugins,
            defaults.blocked_capabilities,
        )
    }

    pub fn with_limits_for_registered_plugins(
        operating_mode: OperatingMode,
        dev_limits: ResourceLimits,
        production_limits: ResourceLimits,
        allowlisted_checksums: impl IntoIterator<Item = String>,
        plugins: impl IntoIterator<Item = RegisteredPlugin>,
        blocked_capabilities: BTreeSet<String>,
    ) -> Self {
        let allowlisted_checksums = allowlisted_checksums.into_iter().collect::<BTreeSet<_>>();
        let plugin_admission = plugins
            .into_iter()
            .map(|plugin| {
                let allowlist_status = if plugin.host_trusted {
                    AllowlistStatus::ExemptBuiltIn
                } else if plugin.tool_tier == ToolTier::OrdinaryPlugin {
                    if allowlisted_checksums.contains(&plugin.manifest.provenance.checksum) {
                        AllowlistStatus::Allowlisted
                    } else if matches!(operating_mode, OperatingMode::Production) {
                        AllowlistStatus::Missing
                    } else {
                        AllowlistStatus::NotRequired
                    }
                } else {
                    AllowlistStatus::ExemptBuiltIn
                };
                let production_eligible = plugin.host_trusted
                    || plugin.tool_tier != ToolTier::OrdinaryPlugin
                    || (matches!(allowlist_status, AllowlistStatus::Allowlisted)
                        && matches!(plugin.conformance_status, ConformanceStatus::Passed)
                        && plugin.manifest_validation.valid);
                let archive_installed = plugin.archive_path.as_ref().map(|_| true);
                let archive_enabled = plugin.archive_path.as_ref().map(|_| plugin.enabled);
                let archive_kind = plugin
                    .archive_path
                    .as_ref()
                    .map(|_| plugin.source_kind_string());
                (
                    plugin.manifest.id.clone(),
                    PluginAdmissionMetadata {
                        is_builtin: false,
                        is_host_trusted: plugin.host_trusted,
                        allowlist_status,
                        conformance_status: plugin.conformance_status,
                        production_eligible,
                        package_installed: plugin.package_id.as_ref().map(|_| true),
                        package_enabled: plugin.package_id.as_ref().map(|_| plugin.enabled),
                        archive_installed,
                        archive_enabled,
                        archive_kind,
                        archive_checksum: plugin.archive_checksum.clone(),
                    },
                )
            })
            .collect();

        Self {
            operating_mode,
            dev_limits,
            production_limits,
            allowlisted_checksums,
            plugin_admission,
            blocked_capabilities,
        }
    }

    fn effective_limits(&self) -> &ResourceLimits {
        match self.operating_mode {
            OperatingMode::Dev => &self.dev_limits,
            OperatingMode::Production => &self.production_limits,
        }
    }

    fn admission_for(&self, tool: &str, manifest: &PluginManifest) -> PluginAdmissionMetadata {
        self.plugin_admission.get(tool).cloned().unwrap_or_else(|| {
            let is_builtin = false;
            let is_host_trusted = matches!(tool, "workspace" | "git" | "github");
            let allowlist_status = if is_builtin || is_host_trusted {
                AllowlistStatus::ExemptBuiltIn
            } else if self
                .allowlisted_checksums
                .contains(&manifest.provenance.checksum)
            {
                AllowlistStatus::Allowlisted
            } else if matches!(self.operating_mode, OperatingMode::Production) {
                AllowlistStatus::Missing
            } else {
                AllowlistStatus::NotRequired
            };
            let conformance_status = if is_host_trusted {
                ConformanceStatus::Passed
            } else {
                ConformanceStatus::NotRun
            };
            let production_eligible = is_host_trusted
                || (matches!(allowlist_status, AllowlistStatus::Allowlisted)
                    && matches!(conformance_status, ConformanceStatus::Passed));
            PluginAdmissionMetadata {
                is_builtin,
                is_host_trusted,
                allowlist_status,
                conformance_status,
                production_eligible,
                package_installed: None,
                package_enabled: None,
                archive_installed: None,
                archive_enabled: None,
                archive_kind: None,
                archive_checksum: None,
            }
        })
    }
}

impl PolicyResolver for StaticPolicyResolver {
    fn operating_mode(&self) -> OperatingMode {
        self.operating_mode.clone()
    }

    fn resolve(
        &self,
        tool: &str,
        command: &PluginCommandManifest,
        manifest: &PluginManifest,
    ) -> Result<PolicyResolution, OberonError> {
        let mut allowed = Vec::new();
        let mut blocked = Vec::new();
        let mut reason_codes = Vec::new();
        let mut blocking_issues = Vec::new();
        let admission = self.admission_for(tool, manifest);

        for capability in &command.required_capabilities {
            if self.blocked_capabilities.contains(capability) {
                blocked.push(capability.clone());
                reason_codes.push(format!("blocked_capability:{capability}"));
            } else {
                allowed.push(capability.clone());
            }
        }

        if matches!(self.operating_mode, OperatingMode::Production) && !admission.is_host_trusted {
            if !matches!(admission.allowlist_status, AllowlistStatus::Allowlisted) {
                reason_codes.push("blocked_by_allowlist".to_string());
                blocking_issues.push(format!(
                    "plugin '{}' is not allowlisted for production mode",
                    tool
                ));
            }
            if !matches!(admission.conformance_status, ConformanceStatus::Passed) {
                reason_codes.push("blocked_by_conformance".to_string());
                blocking_issues.push(format!(
                    "plugin '{}' conformance status is {:?}",
                    tool, admission.conformance_status
                ));
            }
            if !admission.production_eligible {
                reason_codes.push("blocked_by_mode".to_string());
            }
        }

        if matches!(self.operating_mode, OperatingMode::Production)
            && matches!(command.side_effects, SideEffectClass::Destructive)
        {
            reason_codes.push("blocked_by_contract".to_string());
            blocking_issues.push(format!(
                "destructive command '{}.{}' is blocked in production mode",
                tool, command.name
            ));
        }

        let approval_required = approval_required_for(
            &command.side_effects,
            command.approval_expectation.as_ref(),
            &self.operating_mode,
        );

        let resource_limits = merge_resource_limits(
            self.effective_limits(),
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
            plugin_admission_status: if blocking_issues.is_empty() {
                PluginAdmissionStatus::Allowed
            } else {
                PluginAdmissionStatus::Blocked
            },
            allowlist_status: admission.allowlist_status,
            conformance_status: admission.conformance_status,
            production_eligible: admission.production_eligible,
            package_installed: admission.package_installed,
            package_enabled: admission.package_enabled,
            archive_installed: admission.archive_installed,
            archive_enabled: admission.archive_enabled,
            archive_kind: admission.archive_kind,
            archive_checksum: admission.archive_checksum,
            reason_codes,
            blocking_issues,
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

    fn ensure_plugin_admitted(
        &self,
        resolution: &PolicyResolution,
        tool: &str,
        command: &str,
    ) -> Result<(), OberonError> {
        if matches!(
            resolution.plugin_admission_status,
            PluginAdmissionStatus::Allowed
        ) {
            return Ok(());
        }

        let reason = if resolution.reason_codes.is_empty() {
            "plugin blocked by operating mode policy".to_string()
        } else {
            resolution.reason_codes.join(", ")
        };
        Err(OberonError::PolicyDenied(format!(
            "plugin '{tool}.{command}' is not admitted: {reason}"
        )))
    }

    fn unavailable_tool_error(&self, tool: &str) -> Option<OberonError>
    where
        R: RegisteredPluginAccess,
    {
        let availability = self.registry.tool_availability(tool)?;
        if availability.installed && availability.enabled && availability.reason_codes.is_empty() {
            return None;
        }
        let reason = if availability.reason_codes.is_empty() {
            "tool is unavailable".to_string()
        } else {
            availability.reason_codes.join(", ")
        };
        Some(OberonError::PolicyDenied(format!(
            "tool '{tool}' is unavailable: {reason}"
        )))
    }
}

#[derive(Debug, Clone)]
struct HostToolDefinition {
    tool: &'static str,
    command: &'static str,
    summary: &'static str,
    description: &'static str,
    input_schema: serde_json::Value,
    output_schema: serde_json::Value,
    guidance: ToolGuidance,
}

fn host_tool_definitions() -> Vec<HostToolDefinition> {
    vec![
        HostToolDefinition {
            tool: "tool_search",
            command: "search",
            summary: "Search installed tools and commands by capability.",
            description: "Search installed tools and commands by capability.",
            input_schema: serde_json::json!({
                "type":"object",
                "properties":{"query":{"type":"string"},"limit":{"type":"integer"}},
                "required":["query"]
            }),
            output_schema: serde_json::json!({"type":"object"}),
            guidance: ToolGuidance {
                when_to_use: "Use before unfamiliar tool usage to discover tools and commands.".to_string(),
                when_not_to_use: "Do not guess command names when search can narrow the surface.".to_string(),
                examples: vec!["find git commands".to_string(), "find file write tools".to_string()],
                common_mistakes: vec!["Treating host tools as plugin ids".to_string()],
                failure_recovery: vec!["If results are too broad, search again with a narrower intent.".to_string()],
            },
        },
        HostToolDefinition {
            tool: "tool_describe",
            command: "describe",
            summary: "Describe one tool command with guidance and schemas.",
            description: "Describe one tool command with guidance and schemas.",
            input_schema: serde_json::json!({
                "type":"object",
                "properties":{"tool":{"type":"string"},"command":{"type":"string"}},
                "required":["tool","command"]
            }),
            output_schema: serde_json::json!({"type":"object"}),
            guidance: ToolGuidance {
                when_to_use: "Use after search and before first use of a tool command.".to_string(),
                when_not_to_use: "Do not invoke an unfamiliar command without inspecting its schema.".to_string(),
                examples: vec!["describe git push".to_string()],
                common_mistakes: vec!["Passing the host tool name as a plugin id".to_string()],
                failure_recovery: vec!["If describe fails, confirm the tool and command pair from tool_search.".to_string()],
            },
        },
        HostToolDefinition {
            tool: "tool_plan",
            command: "plan",
            summary: "Plan a tool command before invoking it.",
            description: "Plan a tool command before invoking it.",
            input_schema: serde_json::json!({
                "type":"object",
                "properties":{"tool":{"type":"string"},"command":{"type":"string"},"input":{"type":"object"}},
                "required":["tool","command","input"]
            }),
            output_schema: serde_json::json!({"type":"object"}),
            guidance: ToolGuidance {
                when_to_use: "Use before state-changing commands and before approval-gated actions.".to_string(),
                when_not_to_use: "Do not skip planning for write or approval-sensitive operations.".to_string(),
                examples: vec!["plan git push".to_string()],
                common_mistakes: vec!["Planning host tools as nested plugin commands".to_string()],
                failure_recovery: vec!["If planning is blocked, inspect policy and revise the request.".to_string()],
            },
        },
        HostToolDefinition {
            tool: "tool_invoke",
            command: "invoke",
            summary: "Invoke a specific tool command.",
            description: "Invoke a specific tool command.",
            input_schema: serde_json::json!({
                "type":"object",
                "properties":{"tool":{"type":"string"},"command":{"type":"string"},"input":{"type":"object"}},
                "required":["tool","command","input"]
            }),
            output_schema: serde_json::json!({"type":"object"}),
            guidance: ToolGuidance {
                when_to_use: "Use after search, describe, and plan when execution is intended.".to_string(),
                when_not_to_use: "Do not use for discovering tools; use tool_search first.".to_string(),
                examples: vec!["invoke workspace read_file".to_string()],
                common_mistakes: vec!["Using tool_invoke to call host tools recursively".to_string()],
                failure_recovery: vec!["If invoke fails, inspect tool_policy_inspect or re-run tool_describe.".to_string()],
            },
        },
        HostToolDefinition {
            tool: "tool_policy_inspect",
            command: "inspect",
            summary: "Inspect policy and approval requirements for a tool command.",
            description: "Inspect policy and approval requirements for a tool command.",
            input_schema: serde_json::json!({
                "type":"object",
                "properties":{"tool":{"type":"string"},"command":{"type":"string"}},
                "required":["tool","command"]
            }),
            output_schema: serde_json::json!({"type":"object"}),
            guidance: ToolGuidance {
                when_to_use: "Use when a command is blocked, requires approval, or needs budget visibility.".to_string(),
                when_not_to_use: "Do not use as a substitute for tool_describe when you need command semantics.".to_string(),
                examples: vec!["inspect git push".to_string()],
                common_mistakes: vec!["Assuming policy matches command behavior without inspection".to_string()],
                failure_recovery: vec!["If policy denies a command, adjust scope or obtain approval rather than retrying blindly.".to_string()],
            },
        },
    ]
}

fn find_host_tool_definition(tool: &str, command: &str) -> Option<HostToolDefinition> {
    let normalized_command = if command == tool {
        match tool {
            "tool_search" => "search",
            "tool_describe" => "describe",
            "tool_plan" => "plan",
            "tool_invoke" => "invoke",
            "tool_policy_inspect" => "inspect",
            _ => command,
        }
    } else {
        command
    };
    host_tool_definitions()
        .into_iter()
        .find(|definition| definition.tool == tool && definition.command == normalized_command)
}

impl<R, P> HostToolRuntime for PluginRuntimeHost<R, P>
where
    R: PluginRegistry + RegisteredPluginAccess,
    P: PolicyResolver,
{
    fn tool_search(&self, request: &SearchRequest) -> Result<SearchResponse, OberonError> {
        let mut response = self.registry.search(request)?;
        let query = request.query.trim().to_lowercase();
        let mut host_matches = host_tool_definitions()
            .into_iter()
            .filter(|definition| {
                let tool = definition.tool.to_lowercase();
                let command = definition.command.to_lowercase();
                let summary = definition.summary.to_lowercase();
                tool.contains(&query) || command.contains(&query) || summary.contains(&query)
            })
            .map(|definition| SearchMatch {
                tool: definition.tool.to_string(),
                command: definition.command.to_string(),
                summary: definition.summary.to_string(),
                tool_tier: ToolTier::CoreHost,
                side_effects: SideEffectClass::None,
                approval_required: false,
                capabilities_required: Vec::new(),
                confidence: if definition.tool.contains(request.query.trim()) {
                    "1.00".to_string()
                } else {
                    "0.90".to_string()
                },
            })
            .collect::<Vec<_>>();
        response.matches = response
            .matches
            .into_iter()
            .filter_map(|mut entry| {
                let manifest = self.registry.manifest(&entry.tool).ok()?;
                let command = self
                    .registry
                    .command_manifest(&entry.tool, &entry.command)
                    .ok()?;
                let resolution = self.policy.resolve(&entry.tool, &command, &manifest).ok()?;
                if !matches!(
                    resolution.plugin_admission_status,
                    PluginAdmissionStatus::Allowed
                ) {
                    return None;
                }
                entry.approval_required = resolution.approval_required;
                Some(entry)
            })
            .collect();
        response.matches.append(&mut host_matches);
        response.matches.sort_by(|left, right| {
            right
                .confidence
                .cmp(&left.confidence)
                .then_with(|| left.tool.cmp(&right.tool))
                .then_with(|| left.command.cmp(&right.command))
        });
        if let Some(limit) = request.limit {
            response.matches.truncate(limit as usize);
        }
        Ok(response)
    }

    fn tool_describe(
        &self,
        request: &ToolDescribeRequest,
    ) -> Result<ToolDescribeResponse, OberonError> {
        if let Some(definition) = find_host_tool_definition(&request.tool, &request.command) {
            return Ok(ToolDescribeResponse {
                tool: definition.tool.to_string(),
                command: definition.command.to_string(),
                summary: definition.description.to_string(),
                input_schema: oberon4_core::CommandSchemaReference {
                    id: format!("{}.{}.input", definition.tool, definition.command),
                    schema: definition.input_schema,
                },
                output_schema: oberon4_core::CommandSchemaReference {
                    id: format!("{}.{}.output", definition.tool, definition.command),
                    schema: definition.output_schema,
                },
                side_effects: SideEffectClass::None,
                approval_expectation: ApprovalExpectation::None,
                capabilities_required: Vec::new(),
                pagination_behavior: None,
                artifact_rules: None,
                guidance: definition.guidance,
            });
        }
        if let Some(error) = self.unavailable_tool_error(&request.tool) {
            return Err(error);
        }
        let manifest = self.registry.manifest(&request.tool)?;
        let command = self
            .registry
            .command_manifest(&request.tool, &request.command)?;
        let resolution = self.policy.resolve(&request.tool, &command, &manifest)?;
        self.ensure_plugin_admitted(&resolution, &request.tool, &request.command)?;
        self.registry.describe(request)
    }

    fn tool_plan(&self, request: &PlanRequest) -> Result<PlanResult, OberonError> {
        if request.command != request.context.command {
            return Err(OberonError::ContractMismatch(format!(
                "request command '{}' does not match invocation context command '{}'",
                request.command, request.context.command
            )));
        }
        if let Some(error) = self.unavailable_tool_error(&request.context.plugin_id) {
            return Err(error);
        }

        let manifest = self.registry.manifest(&request.context.plugin_id)?;
        let command = self
            .registry
            .command_manifest(&request.context.plugin_id, &request.command)?;
        validate_request_shape(&command, request.input.as_object().is_some())?;
        let resolution = self
            .policy
            .resolve(&request.context.plugin_id, &command, &manifest)?;
        self.ensure_plugin_admitted(&resolution, &request.context.plugin_id, &request.command)?;
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
        if let Some(error) = self.unavailable_tool_error(&request.context.plugin_id) {
            return Err(error);
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
        self.ensure_plugin_admitted(&resolution, &request.context.plugin_id, &request.command)?;
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
        if let Some(definition) = find_host_tool_definition(&request.tool, &request.command) {
            return Ok(PolicyInspectResponse {
                status: PlanStatus::Ok,
                tool: definition.tool.to_string(),
                command: definition.command.to_string(),
                capabilities: EffectiveCapabilities {
                    allowed: Vec::new(),
                    blocked: Vec::new(),
                },
                approval_required: false,
                resource_limits: ResourceLimits {
                    memory_bytes: None,
                    time_ms: None,
                    storage_bytes: None,
                    output_bytes: None,
                    network_policy: Some(NetworkPolicy::BrokeredOnly),
                },
                operating_mode: self.policy.operating_mode(),
                plugin_admission_status: PluginAdmissionStatus::Allowed,
                allowlist_status: AllowlistStatus::ExemptBuiltIn,
                conformance_status: ConformanceStatus::Passed,
                production_eligible: true,
                package_installed: None,
                package_enabled: None,
                archive_installed: None,
                archive_enabled: None,
                archive_kind: None,
                archive_checksum: None,
                reason_codes: vec!["core_host_tool".to_string()],
                service_handles_used: Vec::new(),
                secret_classes_involved: Vec::new(),
                security_levels: Vec::new(),
            });
        }
        if let Some(availability) = self.registry.tool_availability(&request.tool) {
            if !availability.installed || !availability.enabled {
                return Ok(PolicyInspectResponse {
                    status: PlanStatus::Ok,
                    tool: request.tool.clone(),
                    command: request.command.clone(),
                    capabilities: EffectiveCapabilities {
                        allowed: Vec::new(),
                        blocked: Vec::new(),
                    },
                    approval_required: false,
                    resource_limits: ResourceLimits {
                        memory_bytes: None,
                        time_ms: None,
                        storage_bytes: None,
                        output_bytes: None,
                        network_policy: Some(NetworkPolicy::Denied),
                    },
                    operating_mode: self.policy.operating_mode(),
                    plugin_admission_status: PluginAdmissionStatus::Blocked,
                    allowlist_status: if availability.host_trusted {
                        AllowlistStatus::ExemptBuiltIn
                    } else {
                        AllowlistStatus::NotRequired
                    },
                    conformance_status: ConformanceStatus::NotRun,
                    production_eligible: availability.host_trusted,
                    package_installed: availability
                        .package_id
                        .as_ref()
                        .map(|_| availability.installed),
                    package_enabled: availability
                        .package_id
                        .as_ref()
                        .map(|_| availability.enabled),
                    archive_installed: availability
                        .archive_kind
                        .as_ref()
                        .map(|_| availability.installed),
                    archive_enabled: availability
                        .archive_kind
                        .as_ref()
                        .map(|_| availability.enabled),
                    archive_kind: availability.archive_kind.clone(),
                    archive_checksum: availability.archive_checksum.clone(),
                    reason_codes: availability.reason_codes,
                    service_handles_used: Vec::new(),
                    secret_classes_involved: Vec::new(),
                    security_levels: Vec::new(),
                });
            }
        }
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
            plugin_admission_status: resolution.plugin_admission_status,
            allowlist_status: resolution.allowlist_status,
            conformance_status: resolution.conformance_status,
            production_eligible: resolution.production_eligible,
            package_installed: resolution.package_installed,
            package_enabled: resolution.package_enabled,
            archive_installed: resolution.archive_installed,
            archive_enabled: resolution.archive_enabled,
            archive_kind: resolution.archive_kind,
            archive_checksum: resolution.archive_checksum,
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

fn discover_package_manifest_paths(root: &Path) -> Result<Vec<PathBuf>, OberonError> {
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
            } else if path.file_name().and_then(|name| name.to_str()) == Some("package.json") {
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

fn load_package_manifest(path: &Path) -> Result<FirstPartyPackageManifest, OberonError> {
    let raw = fs::read_to_string(path)
        .map_err(|err| OberonError::Storage(format!("package manifest read failed: {err}")))?;
    serde_json::from_str(&raw)
        .map_err(|err| OberonError::ManifestValidation(format!("invalid package.json: {err}")))
}

fn insert_registered_plugin(
    plugins: &mut BTreeMap<String, RegisteredPlugin>,
    index: &mut Vec<IndexedCommand>,
    tool_availability: &mut BTreeMap<String, ToolAvailabilityMetadata>,
    mut registered: RegisteredPlugin,
    runtime_api_version: &str,
    require_executable: bool,
    conformance_engine: Option<&ExecutionEngine>,
) -> Result<(), OberonError> {
    let validation = validate_manifest(&registered.manifest, runtime_api_version);
    registered.manifest_validation = validation.clone();
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

    if require_executable {
        let executable_present = if let Some(archive_path) = &registered.archive_path {
            archive_path.is_file() && registered.archive_entry_wasm_path.is_some()
        } else {
            registered.wasm_path.is_file()
        };
        if !executable_present {
            return Err(OberonError::ManifestValidation(format!(
                "{}: missing plugin.wasm for plugin registration",
                registered.manifest_path.display()
            )));
        }
    }

    if registered.tool_tier == ToolTier::OrdinaryPlugin {
        let (conformance_status, conformance_issues) =
            evaluate_plugin_conformance(&registered, conformance_engine)?;
        registered.conformance_status = conformance_status;
        registered.conformance_issues = conformance_issues;
    } else {
        registered.conformance_status = ConformanceStatus::Passed;
        registered.conformance_issues.clear();
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

    tool_availability.insert(
        plugin_id.clone(),
        ToolAvailabilityMetadata {
            tool: plugin_id.clone(),
            tool_tier: registered.tool_tier.clone(),
            source_kind: Some(registered.source_kind.clone()),
            package_id: registered.package_id.clone(),
            installed: true,
            enabled: registered.enabled,
            host_trusted: registered.host_trusted,
            archive_kind: registered
                .archive_path
                .as_ref()
                .map(|_| registered.source_kind_string()),
            archive_checksum: registered.archive_checksum.clone(),
            reason_codes: Vec::new(),
        },
    );

    plugins.insert(plugin_id, registered);
    Ok(())
}

fn evaluate_plugin_conformance(
    registered: &RegisteredPlugin,
    conformance_engine: Option<&ExecutionEngine>,
) -> Result<(ConformanceStatus, Vec<String>), OberonError> {
    let mut issues = Vec::new();

    if !registered.manifest_validation.valid {
        issues.extend(
            registered
                .manifest_validation
                .issues
                .iter()
                .map(|issue| format!("manifest:{}", issue.code)),
        );
    }

    if registered.tool_tier == ToolTier::OrdinaryPlugin {
        let has_component = if let Some(archive_path) = &registered.archive_path {
            archive_path.is_file() && registered.archive_entry_wasm_path.is_some()
        } else {
            registered.wasm_path.is_file()
        };
        if !has_component {
            issues.push("component:missing_wasm".to_string());
        } else if let Some(engine) = conformance_engine {
            if let Err(err) = engine.validate_component_contract(registered) {
                issues.push(format!("component:{}", err));
            }
        } else {
            return Ok((
                ConformanceStatus::NotRun,
                vec!["conformance:not_run".to_string()],
            ));
        }
    }

    if issues.is_empty() {
        Ok((ConformanceStatus::Passed, Vec::new()))
    } else {
        Ok((ConformanceStatus::Failed, issues))
    }
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
    operating_mode: &OperatingMode,
) -> bool {
    if matches!(approval_expectation, Some(ApprovalExpectation::Required)) {
        return true;
    }

    if matches!(operating_mode, OperatingMode::Production)
        && matches!(side_effects, SideEffectClass::PluginPrivateWrite)
    {
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
        io::Write,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        FilePluginRegistry, HOST_RUNTIME_API_VERSION, PluginRuntimeHost, RegisteredPlugin,
        RuntimeBrokerFoundations, StaticPolicyResolver, install_plugin_archive,
        list_installed_plugins, set_plugin_enabled, unload_plugin_archive, validate_manifest,
    };
    use oberon4_core::{
        AllowlistStatus, AppConfig, ApprovalExpectation, ArtifactCapabilityManifest,
        CapabilityManifest, ConformanceStatus, HostToolRuntime, InstalledPluginKind,
        InvocationContext, InvokeRequest, LifecycleActions, LifecycleHooks, NetworkPolicy,
        OperatingMode, PlanRequest, PluginAdmissionStatus, PluginCommandManifest, PluginExample,
        PluginManifest, PolicyInspectRequest, PromptingManifest, ProvenanceManifest,
        ResourceBudgetRequest, ResourceLimits, SearchRequest, SignatureMetadata,
        StorageCapabilityManifest, ToolDescribeRequest, WorkspaceCapabilityManifest,
    };
    use oberon4_core::{BuildMetadata, CapabilityGrant, CapabilityGrantKind, HandleOperation};
    use oberon4_core::{ManifestValidationResult, PluginRegistry, SideEffectClass};
    use oberon4_core::{OberonError, RequestId};
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    use crate::plugin_runtime_versioned::PluginRuntimeVersionedPackage;

    const VALID_WASM: &[u8] = include_bytes!("../tests/fixtures/plugin-runtime/valid/plugin.wasm");

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

    fn write_manifest_with_wasm(root: &std::path::Path, manifest: &PluginManifest, wasm: &[u8]) {
        let plugin_dir = root.join(&manifest.id);
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(
            plugin_dir.join("plugin.json"),
            serde_json::to_vec_pretty(manifest).unwrap(),
        )
        .unwrap();
        fs::write(plugin_dir.join("plugin.wasm"), wasm).unwrap();
    }

    fn write_manifest(root: &std::path::Path, manifest: &PluginManifest) {
        write_manifest_with_wasm(root, manifest, b"not-a-real-component");
    }

    fn write_archive_plugin_with_settings(
        archive_root: &std::path::Path,
        manifest: &PluginManifest,
        wasm: &[u8],
        settings: serde_json::Value,
    ) -> std::path::PathBuf {
        fs::create_dir_all(archive_root).unwrap();
        let archive_path = archive_root.join(format!("{}.obplg.zip", manifest.id));
        let file = fs::File::create(&archive_path).unwrap();
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

        let archive_manifest = serde_json::json!({
            "id": manifest.id,
            "version": manifest.version,
            "kind": "ordinary-plugin",
            "pluginManifestPath": "plugin.json",
            "wasmPath": "plugin.wasm",
            "settings": settings,
        });

        zip.start_file("oberon-archive.json", options).unwrap();
        zip.write_all(&serde_json::to_vec_pretty(&archive_manifest).unwrap())
            .unwrap();
        zip.start_file("plugin.json", options).unwrap();
        zip.write_all(&serde_json::to_vec_pretty(manifest).unwrap())
            .unwrap();
        zip.start_file("plugin.wasm", options).unwrap();
        zip.write_all(wasm).unwrap();
        zip.finish().unwrap();
        archive_path
    }

    fn write_archive_plugin(
        archive_root: &std::path::Path,
        manifest: &PluginManifest,
        wasm: &[u8],
    ) -> std::path::PathBuf {
        write_archive_plugin_with_settings(archive_root, manifest, wasm, serde_json::json!([]))
    }

    fn load_archive_host(
        archive_root: &std::path::Path,
        config_root: &std::path::Path,
        enabled_plugins: Vec<String>,
        disabled_plugins: Vec<String>,
        operating_mode: OperatingMode,
        allowlisted_checksums: Vec<String>,
    ) -> PluginRuntimeHost<FilePluginRegistry, StaticPolicyResolver> {
        let registry = FilePluginRegistry::load_from_runtime_sources(
            &[archive_root.to_path_buf()],
            config_root,
            &enabled_plugins.into_iter().collect(),
            &disabled_plugins.into_iter().collect(),
            &[],
            &[],
            &BTreeSet::new(),
            &BTreeSet::new(),
            HOST_RUNTIME_API_VERSION,
        )
        .unwrap();
        let policy = StaticPolicyResolver::for_registered_plugins(
            operating_mode,
            allowlisted_checksums,
            registry.all_registered_plugins(),
        );
        PluginRuntimeHost::new(registry, policy).unwrap()
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
    fn archive_plugin_is_discoverable_and_generates_config_defaults() {
        let archive_root = temp_dir("archive-ordinary");
        let config_root = temp_dir("archive-config");
        let manifest = sample_manifest();
        write_archive_plugin_with_settings(
            &archive_root,
            &manifest,
            VALID_WASM,
            serde_json::json!([{
                "key": "mode",
                "type": "string",
                "default": "safe",
                "description": "Execution mode",
                "sensitivity": "low",
                "operatorEditable": true
            }]),
        );

        let host = load_archive_host(
            &archive_root,
            &config_root,
            Vec::new(),
            Vec::new(),
            OperatingMode::Dev,
            Vec::new(),
        );

        let search = host
            .tool_search(&SearchRequest {
                query: "write file".to_string(),
                limit: None,
                session_id: None,
            })
            .unwrap();
        assert!(
            search
                .matches
                .iter()
                .any(|entry| entry.tool == "workspace-git")
        );

        let config_path = config_root.join("workspace-git.toml");
        assert!(config_path.is_file());
        let config: toml::Value =
            toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(config["id"].as_str(), Some("workspace-git"));
        assert_eq!(config["kind"].as_str(), Some("ordinary-plugin"));
        assert_eq!(config["enabled"].as_bool(), Some(true));
        assert_eq!(config["settings"]["mode"].as_str(), Some("safe"));
        assert!(
            config["archiveSha256"]
                .as_str()
                .unwrap()
                .starts_with("sha256:")
        );

        let inspect = host
            .tool_policy_inspect(&PolicyInspectRequest {
                tool: "workspace-git".to_string(),
                command: "write_file_atomic".to_string(),
            })
            .unwrap();
        assert_eq!(inspect.archive_installed, Some(true));
        assert_eq!(inspect.archive_enabled, Some(true));
        assert_eq!(inspect.archive_kind.as_deref(), Some("ordinary-plugin"));
        assert!(
            inspect
                .archive_checksum
                .as_deref()
                .unwrap()
                .starts_with("sha256:")
        );

        let _ = fs::remove_dir_all(archive_root);
        let _ = fs::remove_dir_all(config_root);
    }

    #[test]
    fn archive_plugin_disable_and_missing_archive_states_are_visible() {
        let archive_root = temp_dir("archive-disable");
        let config_root = temp_dir("archive-disable-config");
        let manifest = sample_manifest();
        let archive_path = write_archive_plugin(&archive_root, &manifest, VALID_WASM);

        let _ = load_archive_host(
            &archive_root,
            &config_root,
            Vec::new(),
            Vec::new(),
            OperatingMode::Dev,
            Vec::new(),
        );

        let config_path = config_root.join("workspace-git.toml");
        let mut config: toml::Value =
            toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        config["enabled"] = toml::Value::Boolean(false);
        fs::write(&config_path, toml::to_string_pretty(&config).unwrap()).unwrap();

        let disabled_host = load_archive_host(
            &archive_root,
            &config_root,
            Vec::new(),
            Vec::new(),
            OperatingMode::Dev,
            Vec::new(),
        );
        let search = disabled_host
            .tool_search(&SearchRequest {
                query: "write file".to_string(),
                limit: None,
                session_id: None,
            })
            .unwrap();
        assert!(
            !search
                .matches
                .iter()
                .any(|entry| entry.tool == "workspace-git")
        );
        let inspect = disabled_host
            .tool_policy_inspect(&PolicyInspectRequest {
                tool: "workspace-git".to_string(),
                command: "write_file_atomic".to_string(),
            })
            .unwrap();
        assert_eq!(inspect.archive_installed, Some(true));
        assert_eq!(inspect.archive_enabled, Some(false));
        assert!(
            inspect
                .reason_codes
                .contains(&"blocked_by_disable".to_string())
        );

        config["enabled"] = toml::Value::Boolean(true);
        fs::write(&config_path, toml::to_string_pretty(&config).unwrap()).unwrap();
        fs::remove_file(archive_path).unwrap();

        let missing_host = load_archive_host(
            &archive_root,
            &config_root,
            Vec::new(),
            Vec::new(),
            OperatingMode::Dev,
            Vec::new(),
        );
        let missing_inspect = missing_host
            .tool_policy_inspect(&PolicyInspectRequest {
                tool: "workspace-git".to_string(),
                command: "write_file_atomic".to_string(),
            })
            .unwrap();
        assert_eq!(missing_inspect.archive_installed, Some(false));
        assert_eq!(missing_inspect.archive_enabled, Some(true));
        assert!(
            missing_inspect
                .reason_codes
                .contains(&"blocked_by_missing_archive".to_string())
        );

        let _ = fs::remove_dir_all(archive_root);
        let _ = fs::remove_dir_all(config_root);
    }

    #[test]
    fn operator_archive_config_edits_persist_across_restarts() {
        let archive_root = temp_dir("archive-persist");
        let config_root = temp_dir("archive-persist-config");
        let manifest = sample_manifest();
        write_archive_plugin_with_settings(
            &archive_root,
            &manifest,
            VALID_WASM,
            serde_json::json!([{
                "key": "mode",
                "type": "string",
                "default": "safe",
                "description": "Execution mode",
                "sensitivity": "low",
                "operatorEditable": true
            }]),
        );

        let _ = load_archive_host(
            &archive_root,
            &config_root,
            Vec::new(),
            Vec::new(),
            OperatingMode::Dev,
            Vec::new(),
        );
        let config_path = config_root.join("workspace-git.toml");
        let mut config: toml::Value =
            toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        config["settings"]["mode"] = toml::Value::String("turbo".to_string());
        fs::write(&config_path, toml::to_string_pretty(&config).unwrap()).unwrap();

        let _ = load_archive_host(
            &archive_root,
            &config_root,
            Vec::new(),
            Vec::new(),
            OperatingMode::Dev,
            Vec::new(),
        );

        let persisted: toml::Value =
            toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(persisted["settings"]["mode"].as_str(), Some("turbo"));

        let _ = fs::remove_dir_all(archive_root);
        let _ = fs::remove_dir_all(config_root);
    }

    #[test]
    fn archive_admin_helpers_install_disable_and_unload_plugin() {
        let source_root = temp_dir("archive-admin-source");
        let install_root = temp_dir("archive-admin-install");
        let config_root = temp_dir("archive-admin-config");
        let manifest = sample_manifest();
        let source_archive = write_archive_plugin(&source_root, &manifest, VALID_WASM);

        let mut config = AppConfig::default();
        config.runtime.plugin_archive_roots = vec![install_root.display().to_string()];
        config.runtime.plugin_config_root = config_root.display().to_string();
        config.runtime.enabled_plugins = Vec::new();
        config.runtime.disabled_plugins = Vec::new();

        let installed = install_plugin_archive(&config, &source_archive).unwrap();
        assert_eq!(installed.id, "workspace-git");
        assert_eq!(installed.kind, InstalledPluginKind::OrdinaryPlugin);
        assert!(installed.installed);
        assert!(installed.enabled);

        let listed = list_installed_plugins(&config).unwrap();
        assert!(listed.iter().any(|entry| entry.id == "workspace-git"));

        let disabled = set_plugin_enabled(&config, "workspace-git", false).unwrap();
        assert!(!disabled.enabled);
        assert!(
            disabled
                .reason_codes
                .contains(&"blocked_by_disable".to_string())
        );

        let unloaded = unload_plugin_archive(&config, "workspace-git").unwrap();
        assert!(!unloaded.installed);
        assert!(
            unloaded
                .reason_codes
                .contains(&"blocked_by_missing_archive".to_string())
        );

        let _ = fs::remove_dir_all(source_root);
        let _ = fs::remove_dir_all(install_root);
        let _ = fs::remove_dir_all(config_root);
    }

    #[test]
    fn first_party_package_archive_can_be_installed_with_archive_admin_helpers() {
        let source_root = temp_dir("archive-admin-package-source");
        let install_root = temp_dir("archive-admin-package-install");
        let config_root = temp_dir("archive-admin-package-config");
        PluginRuntimeVersionedPackage::materialize_package_archive(&source_root).unwrap();
        let source_archive = source_root.join("versioned-workspace.obpkg.zip");

        let mut config = AppConfig::default();
        config.runtime.plugin_archive_roots = vec![install_root.display().to_string()];
        config.runtime.plugin_config_root = config_root.display().to_string();
        config.runtime.enabled_plugins = Vec::new();
        config.runtime.disabled_plugins = Vec::new();

        let installed = install_plugin_archive(&config, &source_archive).unwrap();
        assert_eq!(installed.id, "versioned-workspace");
        assert_eq!(installed.kind, InstalledPluginKind::FirstPartyPackage);
        assert!(installed.host_trusted);
        assert!(installed.installed);

        let listed = list_installed_plugins(&config).unwrap();
        assert!(listed.iter().any(|entry| {
            entry.id == "versioned-workspace"
                && entry.kind == InstalledPluginKind::FirstPartyPackage
        }));

        let _ = fs::remove_dir_all(source_root);
        let _ = fs::remove_dir_all(install_root);
        let _ = fs::remove_dir_all(config_root);
    }

    #[test]
    fn versioned_first_party_package_archive_is_discoverable() {
        let archive_root = temp_dir("archive-package");
        let config_root = temp_dir("archive-package-config");
        PluginRuntimeVersionedPackage::materialize_package_archive(&archive_root).unwrap();

        let host = load_archive_host(
            &archive_root,
            &config_root,
            Vec::new(),
            Vec::new(),
            OperatingMode::Dev,
            Vec::new(),
        );

        let search = host
            .tool_search(&SearchRequest {
                query: "git push github".to_string(),
                limit: None,
                session_id: None,
            })
            .unwrap();
        assert!(search.matches.iter().any(|entry| entry.tool == "git"));
        assert!(search.matches.iter().any(|entry| entry.tool == "github"));

        let inspect = host
            .tool_policy_inspect(&PolicyInspectRequest {
                tool: "git".to_string(),
                command: "push".to_string(),
            })
            .unwrap();
        assert_eq!(inspect.archive_installed, Some(true));
        assert_eq!(inspect.archive_enabled, Some(true));
        assert_eq!(inspect.archive_kind.as_deref(), Some("first-party-package"));

        let package_config = config_root.join("versioned-workspace.toml");
        assert!(package_config.is_file());

        let _ = fs::remove_dir_all(archive_root);
        let _ = fs::remove_dir_all(config_root);
    }

    #[test]
    fn archived_ordinary_plugin_production_admission_uses_manifest_checksum_allowlist() {
        let archive_root = temp_dir("archive-prod-allowlist");
        let config_root = temp_dir("archive-prod-allowlist-config");
        let manifest = sample_manifest();
        write_archive_plugin(&archive_root, &manifest, VALID_WASM);

        let host = load_archive_host(
            &archive_root,
            &config_root,
            Vec::new(),
            Vec::new(),
            OperatingMode::Production,
            vec![manifest.provenance.checksum.clone()],
        );

        let inspect = host
            .tool_policy_inspect(&PolicyInspectRequest {
                tool: "workspace-git".to_string(),
                command: "write_file_atomic".to_string(),
            })
            .unwrap();
        assert_eq!(
            inspect.plugin_admission_status,
            PluginAdmissionStatus::Allowed
        );
        assert_eq!(inspect.allowlist_status, AllowlistStatus::Allowlisted);
        assert_eq!(inspect.archive_installed, Some(true));
        assert_eq!(inspect.archive_enabled, Some(true));

        let _ = fs::remove_dir_all(archive_root);
        let _ = fs::remove_dir_all(config_root);
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
        let policy = StaticPolicyResolver::for_registered_plugins(
            OperatingMode::Dev,
            Vec::<String>::new(),
            registry.all_registered_plugins(),
        );
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
        assert_eq!(inspect.operating_mode, OperatingMode::Dev);
        assert_eq!(
            inspect.plugin_admission_status,
            PluginAdmissionStatus::Allowed
        );
        assert_eq!(inspect.allowlist_status, AllowlistStatus::NotRequired);
        assert_eq!(inspect.conformance_status, ConformanceStatus::Failed);
        assert!(!inspect.production_eligible);
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
        write_manifest_with_wasm(&root, &manifest, VALID_WASM);
        let registry =
            FilePluginRegistry::load_from_roots(&[root.clone()], HOST_RUNTIME_API_VERSION).unwrap();
        let policy = StaticPolicyResolver::with_limits_for_registered_plugins(
            OperatingMode::Production,
            ResourceLimits {
                memory_bytes: Some(134_217_728),
                time_ms: Some(5_000),
                storage_bytes: Some(268_435_456),
                output_bytes: Some(131_072),
                network_policy: Some(NetworkPolicy::Denied),
            },
            ResourceLimits {
                memory_bytes: Some(100_663_296),
                time_ms: Some(3_000),
                storage_bytes: Some(134_217_728),
                output_bytes: Some(65_536),
                network_policy: Some(NetworkPolicy::Denied),
            },
            vec![manifest.provenance.checksum.clone()],
            registry.all_registered_plugins(),
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
    fn dev_mode_loads_valid_ordinary_plugins_without_allowlist_enforcement() {
        let root = temp_dir("dev-mode");
        let manifest = sample_manifest();
        write_manifest_with_wasm(&root, &manifest, VALID_WASM);
        let registry =
            FilePluginRegistry::load_from_roots(&[root.clone()], HOST_RUNTIME_API_VERSION).unwrap();
        let policy = StaticPolicyResolver::for_registered_plugins(
            OperatingMode::Dev,
            Vec::<String>::new(),
            registry.all_registered_plugins(),
        );
        let host = PluginRuntimeHost::new(registry, policy).unwrap();

        let search = host
            .tool_search(&SearchRequest {
                query: "write file".to_string(),
                limit: None,
                session_id: None,
            })
            .unwrap();
        assert!(
            search
                .matches
                .iter()
                .any(|entry| entry.tool == "workspace-git")
        );

        let inspect = host
            .tool_policy_inspect(&PolicyInspectRequest {
                tool: "workspace-git".to_string(),
                command: "write_file_atomic".to_string(),
            })
            .unwrap();
        assert_eq!(inspect.operating_mode, OperatingMode::Dev);
        assert_eq!(
            inspect.plugin_admission_status,
            PluginAdmissionStatus::Allowed
        );
        assert_eq!(inspect.allowlist_status, AllowlistStatus::NotRequired);
        assert_eq!(inspect.conformance_status, ConformanceStatus::Passed);
        assert!(!inspect.production_eligible);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn production_mode_blocks_non_allowlisted_ordinary_plugins() {
        let root = temp_dir("prod-allowlist");
        let manifest = sample_manifest();
        write_manifest_with_wasm(&root, &manifest, VALID_WASM);
        let registry =
            FilePluginRegistry::load_from_roots(&[root.clone()], HOST_RUNTIME_API_VERSION).unwrap();
        let policy = StaticPolicyResolver::for_registered_plugins(
            OperatingMode::Production,
            Vec::<String>::new(),
            registry.all_registered_plugins(),
        );
        let host = PluginRuntimeHost::new(registry, policy).unwrap();

        let search = host
            .tool_search(&SearchRequest {
                query: "write file".to_string(),
                limit: None,
                session_id: None,
            })
            .unwrap();
        assert!(
            !search
                .matches
                .iter()
                .any(|entry| entry.tool == "workspace-git")
        );

        let inspect = host
            .tool_policy_inspect(&PolicyInspectRequest {
                tool: "workspace-git".to_string(),
                command: "write_file_atomic".to_string(),
            })
            .unwrap();
        assert_eq!(
            inspect.plugin_admission_status,
            PluginAdmissionStatus::Blocked
        );
        assert_eq!(inspect.allowlist_status, AllowlistStatus::Missing);
        assert_eq!(inspect.conformance_status, ConformanceStatus::Passed);
        assert!(
            inspect
                .reason_codes
                .contains(&"blocked_by_allowlist".to_string())
        );
        assert!(
            inspect
                .reason_codes
                .contains(&"blocked_by_mode".to_string())
        );

        let describe = host.tool_describe(&ToolDescribeRequest {
            tool: "workspace-git".to_string(),
            command: "write_file_atomic".to_string(),
        });
        assert!(matches!(describe, Err(OberonError::PolicyDenied(_))));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn production_mode_blocks_plugins_with_failed_conformance() {
        let root = temp_dir("prod-conformance");
        let manifest = sample_manifest();
        write_manifest(&root, &manifest);
        let registry =
            FilePluginRegistry::load_from_roots(&[root.clone()], HOST_RUNTIME_API_VERSION).unwrap();
        let policy = StaticPolicyResolver::for_registered_plugins(
            OperatingMode::Production,
            vec![manifest.provenance.checksum.clone()],
            registry.all_registered_plugins(),
        );
        let host = PluginRuntimeHost::new(registry, policy).unwrap();

        let inspect = host
            .tool_policy_inspect(&PolicyInspectRequest {
                tool: "workspace-git".to_string(),
                command: "write_file_atomic".to_string(),
            })
            .unwrap();
        assert_eq!(
            inspect.plugin_admission_status,
            PluginAdmissionStatus::Blocked
        );
        assert_eq!(inspect.allowlist_status, AllowlistStatus::Allowlisted);
        assert_eq!(inspect.conformance_status, ConformanceStatus::Failed);
        assert!(
            inspect
                .reason_codes
                .contains(&"blocked_by_conformance".to_string())
        );
        assert!(
            inspect
                .reason_codes
                .contains(&"blocked_by_mode".to_string())
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn builtins_remain_available_in_production_with_stricter_limits() {
        let registry = FilePluginRegistry::load_from_roots(&[], HOST_RUNTIME_API_VERSION).unwrap();
        let policy = StaticPolicyResolver::for_registered_plugins(
            OperatingMode::Production,
            Vec::<String>::new(),
            registry.all_registered_plugins(),
        );
        let host = PluginRuntimeHost::new(registry, policy).unwrap();

        let describe = host
            .tool_describe(&ToolDescribeRequest {
                tool: "git".to_string(),
                command: "push".to_string(),
            })
            .unwrap();
        assert_eq!(describe.tool, "git");

        let inspect = host
            .tool_policy_inspect(&PolicyInspectRequest {
                tool: "git".to_string(),
                command: "push".to_string(),
            })
            .unwrap();
        assert_eq!(inspect.operating_mode, OperatingMode::Production);
        assert_eq!(
            inspect.plugin_admission_status,
            PluginAdmissionStatus::Allowed
        );
        assert_eq!(inspect.allowlist_status, AllowlistStatus::ExemptBuiltIn);
        assert_eq!(inspect.conformance_status, ConformanceStatus::Passed);
        assert!(inspect.production_eligible);
        assert_eq!(inspect.resource_limits.time_ms, Some(2_000));
        assert_eq!(inspect.resource_limits.output_bytes, Some(65_536));
    }

    #[test]
    fn host_tools_are_searchable_describable_and_policy_visible() {
        let registry = FilePluginRegistry::load_from_roots(&[], HOST_RUNTIME_API_VERSION).unwrap();
        let host = PluginRuntimeHost::new(registry, StaticPolicyResolver::default()).unwrap();

        let search = host
            .tool_search(&SearchRequest {
                query: "tools".to_string(),
                limit: Some(5),
                session_id: None,
            })
            .unwrap();
        assert!(
            search
                .matches
                .iter()
                .any(|entry| entry.tool == "tool_search")
        );

        let describe = host
            .tool_describe(&ToolDescribeRequest {
                tool: "tool_search".to_string(),
                command: "tool_search".to_string(),
            })
            .unwrap();
        assert_eq!(describe.tool, "tool_search");
        assert_eq!(describe.command, "search");

        let inspect = host
            .tool_policy_inspect(&PolicyInspectRequest {
                tool: "tool_search".to_string(),
                command: "tool_search".to_string(),
            })
            .unwrap();
        assert_eq!(
            inspect.plugin_admission_status,
            PluginAdmissionStatus::Allowed
        );
        assert_eq!(inspect.allowlist_status, AllowlistStatus::ExemptBuiltIn);
        assert!(inspect.reason_codes.contains(&"core_host_tool".to_string()));
    }

    #[test]
    fn first_party_package_can_be_disabled_and_missing_with_explicit_policy_diagnostics() {
        let package_root = temp_dir("first-party-package");
        PluginRuntimeVersionedPackage::materialize_package_bundle(&package_root).unwrap();

        let disabled_registry = FilePluginRegistry::load_from_sources(
            &[],
            &[package_root.clone()],
            &BTreeSet::new(),
            &BTreeSet::from([PluginRuntimeVersionedPackage::package_id().to_string()]),
            HOST_RUNTIME_API_VERSION,
        )
        .unwrap();
        let disabled_host =
            PluginRuntimeHost::new(disabled_registry, StaticPolicyResolver::default()).unwrap();
        let disabled_search = disabled_host
            .tool_search(&SearchRequest {
                query: "git".to_string(),
                limit: None,
                session_id: None,
            })
            .unwrap();
        assert!(
            !disabled_search
                .matches
                .iter()
                .any(|entry| entry.tool == "git")
        );
        let disabled_policy = disabled_host
            .tool_policy_inspect(&PolicyInspectRequest {
                tool: "git".to_string(),
                command: "push".to_string(),
            })
            .unwrap();
        assert_eq!(
            disabled_policy.plugin_admission_status,
            PluginAdmissionStatus::Blocked
        );
        assert_eq!(disabled_policy.package_installed, Some(true));
        assert_eq!(disabled_policy.package_enabled, Some(false));
        assert!(
            disabled_policy
                .reason_codes
                .contains(&"blocked_by_disable".to_string())
        );

        let missing_registry = FilePluginRegistry::load_from_sources(
            &[],
            &[temp_dir("missing-first-party")],
            &BTreeSet::from([PluginRuntimeVersionedPackage::package_id().to_string()]),
            &BTreeSet::new(),
            HOST_RUNTIME_API_VERSION,
        )
        .unwrap();
        let missing_host =
            PluginRuntimeHost::new(missing_registry, StaticPolicyResolver::default()).unwrap();
        let missing_policy = missing_host
            .tool_policy_inspect(&PolicyInspectRequest {
                tool: "git".to_string(),
                command: "push".to_string(),
            })
            .unwrap();
        assert_eq!(
            missing_policy.plugin_admission_status,
            PluginAdmissionStatus::Blocked
        );
        assert_eq!(missing_policy.package_installed, Some(false));
        assert_eq!(missing_policy.package_enabled, Some(true));
        assert!(
            missing_policy
                .reason_codes
                .contains(&"blocked_by_missing_package".to_string())
        );

        let _ = fs::remove_dir_all(package_root);
    }

    #[test]
    fn malformed_first_party_package_manifest_fails_activation_cleanly() {
        let root = temp_dir("malformed-package");
        let package_dir = root.join(PluginRuntimeVersionedPackage::package_id());
        fs::create_dir_all(&package_dir).unwrap();
        fs::write(package_dir.join("package.json"), b"{not-json").unwrap();

        let error = FilePluginRegistry::load_from_sources(
            &[],
            &[root.clone()],
            &BTreeSet::new(),
            &BTreeSet::new(),
            HOST_RUNTIME_API_VERSION,
        )
        .unwrap_err();
        assert!(matches!(error, OberonError::ManifestValidation(_)));

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
