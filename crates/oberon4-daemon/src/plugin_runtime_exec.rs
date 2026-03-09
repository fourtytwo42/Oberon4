use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use jsonschema::validator_for;
use oberon4_core::plugin_runtime::{InvokeMetrics, InvokeResult, InvokeStatus};
use oberon4_core::{
    ErrorStatus, InvocationContext, NetworkPolicy, OberonError, PlanRequest, PlanResult,
    PluginCommandManifest, RequestId, ToolError, default_data_dir,
};
use wasmtime::{
    Config, Engine, Store, StoreLimits, StoreLimitsBuilder, Trap,
    component::{Component, Linker, ResourceTable},
};
use wasmtime_wasi::{
    DirPerms, FilePerms,
    p2::{IoView, WasiCtx, WasiCtxBuilder, WasiView, add_to_linker_sync, pipe::MemoryOutputPipe},
};

use crate::plugin_runtime::{RegisteredPlugin, RuntimeBrokerFoundations};

wasmtime::component::bindgen!({
    path: "../../wit/oberon-plugin-runtime",
    world: "oberon-plugin",
    async: false,
});

use self::exports::oberon::plugin_runtime::plugin_runtime as guest;

pub struct ExecutionEngine {
    engine: Engine,
    cache: Mutex<BTreeMap<String, CachedComponent>>,
    shutdown: Arc<AtomicBool>,
    ticker: Option<thread::JoinHandle<()>>,
}

#[derive(Clone)]
struct CachedComponent {
    mtime_ms: u128,
    component: Component,
}

struct ExecutionStoreState {
    table: ResourceTable,
    wasi: WasiCtx,
    limits: StoreLimits,
}

impl IoView for ExecutionStoreState {
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

impl WasiView for ExecutionStoreState {
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi
    }
}

impl ExecutionEngine {
    #[allow(clippy::result_large_err)]
    pub fn new(_broker_foundations: RuntimeBrokerFoundations) -> Result<Self, OberonError> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.epoch_interruption(true);
        let engine = Engine::new(&config)
            .map_err(|err| OberonError::Storage(format!("wasmtime engine init failed: {err}")))?;

        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_engine = engine.clone();
        let thread_shutdown = shutdown.clone();
        let ticker = thread::spawn(move || {
            while !thread_shutdown.load(Ordering::Relaxed) {
                thread_engine.increment_epoch();
                thread::sleep(Duration::from_millis(1));
            }
        });

        Ok(Self {
            engine,
            cache: Mutex::new(BTreeMap::new()),
            shutdown,
            ticker: Some(ticker),
        })
    }

    #[allow(clippy::result_large_err)]
    pub fn plan(
        &self,
        plugin: &RegisteredPlugin,
        command: &PluginCommandManifest,
        request: &PlanRequest,
    ) -> Result<PlanResult, ToolError> {
        let component = self
            .load_component(plugin)
            .map_err(|err| self.load_error(err, request))?;
        let mut linker = Linker::new(&self.engine);
        add_to_linker_sync(&mut linker)
            .map_err(|err| self.load_error(OberonError::Storage(err.to_string()), request))?;
        let mut store = self
            .build_store(plugin, command, &request.context)
            .map_err(|err| self.load_error(err, request))?;
        let plugin_instance = OberonPlugin::instantiate(&mut store, &component, &linker)
            .map_err(|err| self.runtime_error("RUNTIME_COMPONENT_LOAD_FAILED", err, request))?;
        let input_json = serde_json::to_string(&request.input)
            .map_err(|err| self.load_error(OberonError::Storage(err.to_string()), request))?;
        let context = self.to_guest_context(&request.context);

        let result = plugin_instance
            .oberon_plugin_runtime_plugin_runtime()
            .call_plan(&mut store, &request.command, &input_json, &context)
            .map_err(|err| self.trap_error(err, request, start_instant()))?
            .map_err(|err| self.guest_error(err, request))?;

        self.enforce_storage_limit(plugin, &request.context)
            .map_err(|err| self.limit_error(err, request))?;

        let plan = PlanResult {
            status: oberon4_core::PlanStatus::Ok,
            side_effects: convert_side_effects(result.side_effects),
            approval_required: result.approval_required,
            capabilities_required: result.capabilities_required,
            estimated_cost: match result.estimated_cost.as_str() {
                "high" => oberon4_core::EstimatedCost::High,
                "medium" => oberon4_core::EstimatedCost::Medium,
                _ => oberon4_core::EstimatedCost::Low,
            },
            estimated_steps: result.estimated_steps,
            preconditions: result.preconditions,
            blocking_issues: result.blocking_issues,
            resource_limits: None,
            approval_preview: None,
        };
        Ok(plan)
    }

    #[allow(clippy::result_large_err)]
    pub fn invoke(
        &self,
        plugin: &RegisteredPlugin,
        command: &PluginCommandManifest,
        request: &oberon4_core::InvokeRequest,
    ) -> Result<InvokeResult, ToolError> {
        let started = Instant::now();
        let component = self
            .load_component(plugin)
            .map_err(|err| self.load_error(err, request))?;
        let mut linker = Linker::new(&self.engine);
        add_to_linker_sync(&mut linker)
            .map_err(|err| self.load_error(OberonError::Storage(err.to_string()), request))?;
        let mut store = self
            .build_store(plugin, command, &request.context)
            .map_err(|err| self.load_error(err, request))?;
        let plugin_instance = OberonPlugin::instantiate(&mut store, &component, &linker)
            .map_err(|err| self.runtime_error("RUNTIME_COMPONENT_LOAD_FAILED", err, request))?;
        let input_json = serde_json::to_string(&request.input)
            .map_err(|err| self.load_error(OberonError::Storage(err.to_string()), request))?;
        let context = self.to_guest_context(&request.context);

        let result = plugin_instance
            .oberon_plugin_runtime_plugin_runtime()
            .call_invoke(&mut store, &request.command, &input_json, &context)
            .map_err(|err| self.trap_error(err, request, started))?
            .map_err(|err| self.guest_error(err, request))?;

        self.enforce_output_limit(&request.context, &result.summary, &result.data_json)
            .map_err(|err| self.limit_error(err, request))?;
        self.enforce_storage_limit(plugin, &request.context)
            .map_err(|err| self.limit_error(err, request))?;

        if !result.artifacts.is_empty() {
            return Err(self.contract_error(
                "RUNTIME_CONTRACT_MISMATCH",
                "artifacts are not supported until WP4",
                request,
            ));
        }

        let data = serde_json::from_str(&result.data_json).map_err(|err| {
            self.contract_error(
                "RUNTIME_CONTRACT_MISMATCH",
                &format!("plugin returned invalid json payload: {err}"),
                request,
            )
        })?;
        self.validate_schema(&command.output_schema, &data, request)?;

        Ok(InvokeResult {
            status: match result.status {
                guest::InvokeStatus::Pending => InvokeStatus::Pending,
                guest::InvokeStatus::Ok => InvokeStatus::Ok,
            },
            summary: result.summary,
            data,
            artifacts: Vec::new(),
            metrics: InvokeMetrics {
                duration_ms: started.elapsed().as_millis() as u64,
            },
            next_suggested_action: result.next_suggested_action,
        })
    }

    fn load_component(&self, plugin: &RegisteredPlugin) -> Result<Component, OberonError> {
        let key = format!(
            "{}@{}:{}:{}",
            plugin.manifest.id,
            plugin.manifest.version,
            plugin.manifest_path.display(),
            plugin.wasm_path.display()
        );
        let mtime_ms = fs::metadata(&plugin.wasm_path)
            .and_then(|meta| meta.modified())
            .map_err(|err| OberonError::Storage(format!("plugin.wasm metadata failed: {err}")))?
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|err| OberonError::Storage(format!("mtime conversion failed: {err}")))?
            .as_millis();

        let mut cache = self
            .cache
            .lock()
            .map_err(|_| OberonError::Storage("component cache lock poisoned".to_string()))?;
        if let Some(cached) = cache.get(&key) {
            if cached.mtime_ms == mtime_ms {
                return Ok(cached.component.clone());
            }
        }
        let component = Component::from_file(&self.engine, &plugin.wasm_path).map_err(|err| {
            OberonError::Storage(format!(
                "component load failed for {}: {err}",
                plugin.wasm_path.display()
            ))
        })?;
        cache.insert(
            key,
            CachedComponent {
                mtime_ms,
                component: component.clone(),
            },
        );
        Ok(component)
    }

    fn build_store(
        &self,
        plugin: &RegisteredPlugin,
        command: &PluginCommandManifest,
        context: &InvocationContext,
    ) -> Result<Store<ExecutionStoreState>, OberonError> {
        let output_limit = usize::try_from(context.output_limit_bytes)
            .map_err(|_| OberonError::Validation("output limit exceeds usize".to_string()))?;
        let memory_limit = usize::try_from(context.memory_limit)
            .map_err(|_| OberonError::Validation("memory limit exceeds usize".to_string()))?;
        let stdout = MemoryOutputPipe::new(output_limit);
        let stderr = MemoryOutputPipe::new(output_limit);
        let state = ExecutionStoreState {
            table: ResourceTable::new(),
            wasi: self.build_wasi(plugin, command, context, stdout.clone(), stderr.clone())?,
            limits: StoreLimitsBuilder::new().memory_size(memory_limit).build(),
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|state| &mut state.limits);
        store.set_epoch_deadline(context.time_limit_ms.max(1));
        store.epoch_deadline_trap();
        Ok(store)
    }

    fn build_wasi(
        &self,
        plugin: &RegisteredPlugin,
        command: &PluginCommandManifest,
        context: &InvocationContext,
        stdout: MemoryOutputPipe,
        stderr: MemoryOutputPipe,
    ) -> Result<WasiCtx, OberonError> {
        let mut builder = WasiCtxBuilder::new();
        builder.stdout(stdout).stderr(stderr);
        builder.arg(&plugin.manifest.id);
        builder.arg(&command.name);

        for mount in self.mounts_for(plugin, context)? {
            builder
                .preopened_dir(
                    &mount.host_path,
                    &mount.guest_path,
                    mount.dir_perms,
                    mount.file_perms,
                )
                .map_err(|err| OberonError::Storage(format!("preopened_dir failed: {err}")))?;
        }

        if !matches!(context.network_policy, NetworkPolicy::Denied) {
            return Err(OberonError::Validation(
                "raw network policies are not supported in WP2".to_string(),
            ));
        }

        Ok(builder.build())
    }

    fn mounts_for(
        &self,
        plugin: &RegisteredPlugin,
        context: &InvocationContext,
    ) -> Result<Vec<StorageMount>, OberonError> {
        let mut mounts = Vec::new();
        let data_root = default_data_dir().join("plugin-runtime");
        if plugin.manifest.capabilities.storage.private {
            let host_path = data_root.join("private").join(&plugin.manifest.id);
            fs::create_dir_all(&host_path).map_err(|err| {
                OberonError::Storage(format!("create private storage failed: {err}"))
            })?;
            mounts.push(StorageMount::read_write(host_path, "/private"));
        }
        if plugin.manifest.capabilities.storage.cache {
            let host_path = data_root.join("cache").join(&plugin.manifest.id);
            fs::create_dir_all(&host_path).map_err(|err| {
                OberonError::Storage(format!("create cache storage failed: {err}"))
            })?;
            mounts.push(StorageMount::read_write(host_path, "/cache"));
        }
        if plugin.manifest.capabilities.storage.session {
            let host_path = data_root
                .join("session")
                .join(&context.session_id)
                .join(&plugin.manifest.id);
            fs::create_dir_all(&host_path).map_err(|err| {
                OberonError::Storage(format!("create session storage failed: {err}"))
            })?;
            mounts.push(StorageMount::read_write(host_path, "/session"));
        }
        Ok(mounts)
    }

    fn enforce_output_limit(
        &self,
        context: &InvocationContext,
        summary: &str,
        data_json: &str,
    ) -> Result<(), OberonError> {
        let total = summary.len() + data_json.len();
        if total > context.output_limit_bytes as usize {
            return Err(OberonError::Validation(format!(
                "RESOURCE_LIMIT_OUTPUT: {} bytes exceeds {}",
                total, context.output_limit_bytes
            )));
        }
        Ok(())
    }

    fn enforce_storage_limit(
        &self,
        plugin: &RegisteredPlugin,
        context: &InvocationContext,
    ) -> Result<(), OberonError> {
        let roots = self.mounts_for(plugin, context)?;
        let total: u64 = roots
            .iter()
            .map(|mount| dir_size(&mount.host_path))
            .sum::<Result<u64, _>>()?;
        if total > context.storage_limit_bytes {
            return Err(OberonError::Storage(format!(
                "RESOURCE_LIMIT_STORAGE: {} bytes exceeds {}",
                total, context.storage_limit_bytes
            )));
        }
        Ok(())
    }

    #[allow(clippy::result_large_err)]
    fn validate_schema(
        &self,
        schema: &serde_json::Value,
        data: &serde_json::Value,
        request: &oberon4_core::InvokeRequest,
    ) -> Result<(), ToolError> {
        let validator = validator_for(schema).map_err(|err| {
            self.contract_error(
                "RUNTIME_CONTRACT_MISMATCH",
                &format!("schema compilation failed: {err}"),
                request,
            )
        })?;
        if validator.validate(data).is_err() {
            return Err(self.contract_error(
                "RUNTIME_CONTRACT_MISMATCH",
                "plugin output does not match command output schema",
                request,
            ));
        }
        Ok(())
    }

    fn to_guest_context(&self, context: &InvocationContext) -> guest::InvocationContext {
        guest::InvocationContext {
            request_id: context.request_id.as_str().to_string(),
            session_id: context.session_id.clone(),
            plugin_id: context.plugin_id.clone(),
            plugin_version: context.plugin_version.clone(),
            command: context.command.clone(),
            deadline: context.deadline.clone(),
            runtime_api_version: context.runtime_api_version.clone(),
            capability_grants: context
                .capability_grants
                .iter()
                .map(|grant| guest::CapabilityGrant {
                    kind: match grant.kind {
                        oberon4_core::CapabilityGrantKind::StorageHandle => {
                            guest::CapabilityGrantKind::StorageHandle
                        }
                        oberon4_core::CapabilityGrantKind::ArtifactHandle => {
                            guest::CapabilityGrantKind::ArtifactHandle
                        }
                        oberon4_core::CapabilityGrantKind::WorkspaceHandle => {
                            guest::CapabilityGrantKind::WorkspaceHandle
                        }
                        oberon4_core::CapabilityGrantKind::ServiceHandle => {
                            guest::CapabilityGrantKind::ServiceHandle
                        }
                        oberon4_core::CapabilityGrantKind::SecretUseHandle => {
                            guest::CapabilityGrantKind::SecretUseHandle
                        }
                        oberon4_core::CapabilityGrantKind::SecretHandle => {
                            guest::CapabilityGrantKind::SecretHandle
                        }
                    },
                    id: grant.id.clone(),
                    operations: grant
                        .operations
                        .iter()
                        .map(|op| format!("{op:?}").to_lowercase())
                        .collect(),
                })
                .collect(),
            workspace_revision: context.workspace_revision.clone(),
            memory_limit: context.memory_limit,
            time_limit_ms: context.time_limit_ms,
            output_limit_bytes: context.output_limit_bytes,
            storage_limit_bytes: context.storage_limit_bytes,
            network_policy: match context.network_policy {
                NetworkPolicy::Denied => guest::NetworkPolicy::Denied,
                NetworkPolicy::BrokeredOnly => guest::NetworkPolicy::BrokeredOnly,
                NetworkPolicy::Allowlisted => guest::NetworkPolicy::Allowlisted,
            },
        }
    }

    fn load_error<T>(&self, err: OberonError, request: &T) -> ToolError
    where
        T: RequestView,
    {
        self.error_from_request(
            "RUNTIME_COMPONENT_LOAD_FAILED",
            &err.to_string(),
            false,
            request,
        )
    }

    fn runtime_error<T>(&self, code: &str, err: impl std::fmt::Display, request: &T) -> ToolError
    where
        T: RequestView,
    {
        self.error_from_request(code, &err.to_string(), false, request)
    }

    fn limit_error<T>(&self, err: OberonError, request: &T) -> ToolError
    where
        T: RequestView,
    {
        let message = err.to_string();
        let code = if message.contains("RESOURCE_LIMIT_OUTPUT") {
            "RESOURCE_LIMIT_OUTPUT"
        } else if message.contains("RESOURCE_LIMIT_STORAGE") {
            "RESOURCE_LIMIT_STORAGE"
        } else if message.contains("RESOURCE_LIMIT_MEMORY") {
            "RESOURCE_LIMIT_MEMORY"
        } else if message.contains("RESOURCE_LIMIT_TIMEOUT") {
            "RESOURCE_LIMIT_TIMEOUT"
        } else {
            "RUNTIME_COMPONENT_LOAD_FAILED"
        };
        self.error_from_request(code, &message, false, request)
    }

    fn guest_error<T>(&self, err: guest::InvokeError, request: &T) -> ToolError
    where
        T: RequestView,
    {
        ToolError {
            status: ErrorStatus::Error,
            code: err.code,
            message: err.message,
            retryable: err.retryable,
            request_id: request.request_id().clone(),
            plugin_id: request.context().plugin_id.clone(),
            plugin_version: request.context().plugin_version.clone(),
            command: request.command().to_string(),
            next_suggested_action: err.next_suggested_action,
        }
    }

    fn trap_error<T>(&self, err: wasmtime::Error, request: &T, started: Instant) -> ToolError
    where
        T: RequestView,
    {
        let code = if err.downcast_ref::<Trap>().is_some()
            && started.elapsed().as_millis() as u64 >= request.context().time_limit_ms
        {
            "RESOURCE_LIMIT_TIMEOUT"
        } else {
            "RESOURCE_LIMIT_MEMORY"
        };
        self.error_from_request(code, &err.to_string(), false, request)
    }

    fn contract_error<T>(&self, code: &str, message: &str, request: &T) -> ToolError
    where
        T: RequestView,
    {
        self.error_from_request(code, message, false, request)
    }

    fn error_from_request<T>(
        &self,
        code: &str,
        message: &str,
        retryable: bool,
        request: &T,
    ) -> ToolError
    where
        T: RequestView,
    {
        ToolError {
            status: ErrorStatus::Error,
            code: code.to_string(),
            message: message.to_string(),
            retryable,
            request_id: request.request_id().clone(),
            plugin_id: request.context().plugin_id.clone(),
            plugin_version: request.context().plugin_version.clone(),
            command: request.command().to_string(),
            next_suggested_action: Some("tool_policy_inspect".to_string()),
        }
    }
}

impl Drop for ExecutionEngine {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(handle) = self.ticker.take() {
            let _ = handle.join();
        }
    }
}

trait RequestView {
    fn context(&self) -> &InvocationContext;
    fn request_id(&self) -> &RequestId;
    fn command(&self) -> &str;
}

impl RequestView for PlanRequest {
    fn context(&self) -> &InvocationContext {
        &self.context
    }

    fn request_id(&self) -> &RequestId {
        &self.context.request_id
    }

    fn command(&self) -> &str {
        &self.command
    }
}

impl RequestView for oberon4_core::InvokeRequest {
    fn context(&self) -> &InvocationContext {
        &self.context
    }

    fn request_id(&self) -> &RequestId {
        &self.context.request_id
    }

    fn command(&self) -> &str {
        &self.command
    }
}

struct StorageMount {
    host_path: PathBuf,
    guest_path: String,
    dir_perms: DirPerms,
    file_perms: FilePerms,
}

impl StorageMount {
    fn read_write(host_path: PathBuf, guest_path: &str) -> Self {
        Self {
            host_path,
            guest_path: guest_path.to_string(),
            dir_perms: DirPerms::READ | DirPerms::MUTATE,
            file_perms: FilePerms::READ | FilePerms::WRITE,
        }
    }
}

fn dir_size(path: &Path) -> Result<u64, OberonError> {
    let mut total = 0;
    if !path.exists() {
        return Ok(0);
    }
    for entry in
        fs::read_dir(path).map_err(|err| OberonError::Storage(format!("read_dir failed: {err}")))?
    {
        let entry =
            entry.map_err(|err| OberonError::Storage(format!("dir entry failed: {err}")))?;
        let meta = entry
            .metadata()
            .map_err(|err| OberonError::Storage(format!("metadata failed: {err}")))?;
        if meta.is_dir() {
            total += dir_size(&entry.path())?;
        } else {
            total += meta.len();
        }
    }
    Ok(total)
}

fn start_instant() -> Instant {
    Instant::now()
}

fn convert_side_effects(value: guest::SideEffectClass) -> oberon4_core::SideEffectClass {
    match value {
        guest::SideEffectClass::None => oberon4_core::SideEffectClass::None,
        guest::SideEffectClass::Ephemeral => oberon4_core::SideEffectClass::Ephemeral,
        guest::SideEffectClass::PluginPrivateWrite => {
            oberon4_core::SideEffectClass::PluginPrivateWrite
        }
        guest::SideEffectClass::ExternalRead => oberon4_core::SideEffectClass::ExternalRead,
        guest::SideEffectClass::ExternalWrite => oberon4_core::SideEffectClass::ExternalWrite,
        guest::SideEffectClass::Destructive => oberon4_core::SideEffectClass::Destructive,
    }
}
