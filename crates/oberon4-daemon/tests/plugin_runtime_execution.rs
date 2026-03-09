use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use oberon4_core::{
    ApprovalExpectation, ArtifactCapabilityManifest, BuildMetadata, CapabilityGrant,
    CapabilityGrantKind, CapabilityManifest, HandleOperation, HostToolRuntime, InMemorySecretStore,
    InvocationContext, InvokeRequest, LifecycleActions, LifecycleHooks, NetworkPolicy,
    OperatingMode, PlanRequest, PluginCommandManifest, PluginExample, PluginManifest,
    PromptingManifest, ProvenanceManifest, RequestId, ResourceBudgetRequest, ResourceLimits,
    SecretStore, SideEffectClass, SignatureMetadata, StorageCapabilityManifest,
    WorkspaceCapabilityManifest,
};
use oberon4_daemon::{
    EncryptedLocalSecretBackend, FilePluginRegistry, HOST_RUNTIME_API_VERSION,
    InMemoryServiceTransport, PluginRuntimeHost, PluginRuntimeRedactor, PluginRuntimeSecretBroker,
    PluginRuntimeServiceBroker, SecretBinding, ServiceBinding, StaticPolicyResolver,
};

const VALID_WASM: &[u8] = include_bytes!("fixtures/plugin-runtime/valid/plugin.wasm");
const INVALID_WASM: &[u8] = include_bytes!("fixtures/plugin-runtime/invalid/plugin.wasm");

fn temp_dir(label: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("oberon4-wp2-exec-{label}-{nonce}"))
}

fn command_manifest(
    name: &str,
    side_effects: SideEffectClass,
    required_capabilities: Vec<&str>,
    input_schema: serde_json::Value,
    output_schema: serde_json::Value,
) -> PluginCommandManifest {
    PluginCommandManifest {
        name: name.to_string(),
        version: "1".to_string(),
        description: format!("{name} command"),
        input_schema,
        output_schema,
        tags: vec!["wp2".to_string(), name.to_string()],
        side_effects,
        required_capabilities: required_capabilities
            .into_iter()
            .map(str::to_string)
            .collect(),
        approval_expectation: Some(ApprovalExpectation::None),
        supports_planning: true,
        supports_pagination: false,
        supports_artifacts: false,
        precondition_model: None,
        requested_resources: None,
    }
}

fn fixture_manifest(plugin_id: &str) -> PluginManifest {
    PluginManifest {
        id: plugin_id.to_string(),
        version: "0.1.0".to_string(),
        display_name: "WP2 Fixture".to_string(),
        description: "Execution fixture for WP2.".to_string(),
        runtime_api_version: HOST_RUNTIME_API_VERSION.to_string(),
        commands: vec![
            command_manifest(
                "echo",
                SideEffectClass::None,
                vec![],
                serde_json::json!({
                    "type": "object",
                    "required": ["message"],
                    "properties": { "message": { "type": "string" } }
                }),
                serde_json::json!({
                    "type": "object",
                    "required": ["message"],
                    "properties": { "message": { "type": "string" } }
                }),
            ),
            command_manifest(
                "big_output",
                SideEffectClass::None,
                vec![],
                serde_json::json!({
                    "type": "object",
                    "required": ["bytes"],
                    "properties": { "bytes": { "type": "integer", "minimum": 1 } }
                }),
                serde_json::json!({
                    "type": "object",
                    "required": ["payload"],
                    "properties": { "payload": { "type": "string" } }
                }),
            ),
            command_manifest(
                "allocate",
                SideEffectClass::None,
                vec![],
                serde_json::json!({
                    "type": "object",
                    "required": ["bytes"],
                    "properties": { "bytes": { "type": "integer", "minimum": 1 } }
                }),
                serde_json::json!({
                    "type": "object",
                    "required": ["allocated"],
                    "properties": { "allocated": { "type": "integer", "minimum": 0 } }
                }),
            ),
            command_manifest(
                "write_storage",
                SideEffectClass::PluginPrivateWrite,
                vec!["storage.private"],
                serde_json::json!({
                    "type": "object",
                    "required": ["bytes"],
                    "properties": { "bytes": { "type": "integer", "minimum": 1 } }
                }),
                serde_json::json!({
                    "type": "object",
                    "required": ["path", "bytes_written"],
                    "properties": {
                        "path": { "type": "string" },
                        "bytes_written": { "type": "integer", "minimum": 0 }
                    }
                }),
            ),
            command_manifest(
                "counter",
                SideEffectClass::None,
                vec![],
                serde_json::json!({ "type": "object" }),
                serde_json::json!({
                    "type": "object",
                    "required": ["value"],
                    "properties": { "value": { "type": "integer", "minimum": 1 } }
                }),
            ),
            command_manifest(
                "spin",
                SideEffectClass::None,
                vec![],
                serde_json::json!({
                    "type": "object",
                    "required": ["bytes"],
                    "properties": { "bytes": { "type": "integer", "minimum": 1 } }
                }),
                serde_json::json!({
                    "type": "object",
                    "required": ["value"],
                    "properties": { "value": { "type": "integer", "minimum": 0 } }
                }),
            ),
            command_manifest(
                "bad_json",
                SideEffectClass::None,
                vec![],
                serde_json::json!({ "type": "object" }),
                serde_json::json!({
                    "type": "object",
                    "required": ["message"],
                    "properties": { "message": { "type": "string" } }
                }),
            ),
            command_manifest(
                "needs_service",
                SideEffectClass::ExternalRead,
                vec!["service.github.request"],
                serde_json::json!({ "type": "object" }),
                serde_json::json!({ "type": "object" }),
            ),
        ],
        capabilities: CapabilityManifest {
            storage: StorageCapabilityManifest {
                private: true,
                session: true,
                cache: true,
            },
            workspace: WorkspaceCapabilityManifest {
                read: false,
                write: false,
            },
            artifacts: ArtifactCapabilityManifest {
                read: false,
                create: false,
            },
            services: vec!["service.github.request".to_string()],
        },
        prompting: PromptingManifest {
            prompt_version: "1".to_string(),
            summary: "Fixture".to_string(),
            when_to_use: "Use for execution tests.".to_string(),
            when_not_to_use: "Do not use outside execution tests.".to_string(),
            examples: vec![PluginExample {
                intent: "Echo a message".to_string(),
                command: "echo".to_string(),
            }],
            failure_recovery: vec!["Retry with smaller input".to_string()],
            safety_notes: vec!["Fixture only".to_string()],
        },
        provenance: ProvenanceManifest {
            publisher: "oberon4".to_string(),
            source: "fixture".to_string(),
            checksum: "sha256:fixture".to_string(),
            signature: Some(SignatureMetadata {
                format: "fixture".to_string(),
                value: "fixture".to_string(),
                key_id: Some("fixture".to_string()),
            }),
            build: Some(BuildMetadata {
                target: "wasm32-wasip2".to_string(),
                created_at: "2026-03-09T00:00:00Z".to_string(),
            }),
        },
        lifecycle_actions: Some(LifecycleActions {
            install: false,
            validate: true,
            activate: true,
            deactivate: true,
            upgrade: false,
            uninstall: false,
            hooks: LifecycleHooks {
                on_install: false,
                on_upgrade: false,
                health_check: true,
            },
        }),
        requested_resources: Some(ResourceBudgetRequest {
            memory_bytes: None,
            time_ms: None,
            storage_bytes: None,
            output_bytes: None,
            artifact_bytes: None,
            network_policy: Some(NetworkPolicy::Denied),
        }),
    }
}

fn write_bundle(root: &Path, manifest: &PluginManifest, wasm: &[u8], include_wasm: bool) {
    let plugin_dir = root.join(&manifest.id);
    fs::create_dir_all(&plugin_dir).unwrap();
    fs::write(
        plugin_dir.join("plugin.json"),
        serde_json::to_vec_pretty(manifest).unwrap(),
    )
    .unwrap();
    if include_wasm {
        fs::write(plugin_dir.join("plugin.wasm"), wasm).unwrap();
    }
}

fn context(plugin_id: &str, command: &str) -> InvocationContext {
    InvocationContext {
        request_id: RequestId::new(),
        session_id: "sess_wp2".to_string(),
        plugin_id: plugin_id.to_string(),
        plugin_version: "0.1.0".to_string(),
        command: command.to_string(),
        deadline: "2026-03-09T00:00:05Z".to_string(),
        runtime_api_version: HOST_RUNTIME_API_VERSION.to_string(),
        capability_grants: vec![CapabilityGrant {
            kind: CapabilityGrantKind::StorageHandle,
            id: "stor_fixture".to_string(),
            operations: vec![
                HandleOperation::Read,
                HandleOperation::Write,
                HandleOperation::List,
            ],
        }],
        workspace_revision: None,
        memory_limit: 134_217_728,
        time_limit_ms: 5_000,
        output_limit_bytes: 131_072,
        storage_limit_bytes: 268_435_456,
        network_policy: NetworkPolicy::Denied,
    }
}

fn policy_with_limits(
    limits: ResourceLimits,
    blocked_capabilities: BTreeSet<String>,
) -> StaticPolicyResolver {
    StaticPolicyResolver::new(OperatingMode::Production, limits, blocked_capabilities)
}

fn host_with_github_service(
    registry: FilePluginRegistry,
) -> PluginRuntimeHost<FilePluginRegistry, StaticPolicyResolver> {
    let mut store = InMemorySecretStore::default();
    store
        .set_secret("oauth.github.main", "gh-secret-token")
        .unwrap();
    let secret_broker = Arc::new(PluginRuntimeSecretBroker::new(
        EncryptedLocalSecretBackend::new(store),
        vec![SecretBinding {
            secret_handle: "sec_github".to_string(),
            store_key: "oauth.github.main".to_string(),
            secret_class: oberon4_core::SecretClass::OauthCredential,
            allowed_operations: vec![HandleOperation::Invoke],
        }],
    ));
    let service_broker = Arc::new(PluginRuntimeServiceBroker::new(
        PluginRuntimeSecretBroker::new(
            EncryptedLocalSecretBackend::new({
                let mut store = InMemorySecretStore::default();
                store
                    .set_secret("oauth.github.main", "gh-secret-token")
                    .unwrap();
                store
            }),
            vec![SecretBinding {
                secret_handle: "sec_github".to_string(),
                store_key: "oauth.github.main".to_string(),
                secret_class: oberon4_core::SecretClass::OauthCredential,
                allowed_operations: vec![HandleOperation::Invoke],
            }],
        ),
        vec![ServiceBinding {
            service_handle: "svc_github".to_string(),
            service_name: "github.request".to_string(),
            secret_handle: "sec_github".to_string(),
            base_url: "https://api.github.test".to_string(),
            allowed_operations: vec![HandleOperation::Invoke],
        }],
        InMemoryServiceTransport::default().with_response(
            "github.request",
            "GET",
            "/repos/oberon4/status",
            oberon4_daemon::plugin_runtime_broker::StubbedServiceResponse {
                status_code: 200,
                headers: vec![oberon4_core::HttpHeader {
                    name: "authorization".to_string(),
                    value: "Bearer gh-secret-token".to_string(),
                }],
                body: serde_json::json!({
                    "ok": true,
                    "tokenEcho": "gh-secret-token",
                }),
            },
        ),
    ));
    PluginRuntimeHost::with_runtime_services(
        registry,
        StaticPolicyResolver::default(),
        oberon4_daemon::RuntimeBrokerFoundations::default(),
        secret_broker,
        service_broker,
        Arc::new(PluginRuntimeRedactor),
    )
    .unwrap()
}

#[test]
fn valid_component_can_be_planned_and_invoked() {
    let root = temp_dir("valid-plan-invoke");
    let manifest = fixture_manifest("wp2-fixture");
    write_bundle(&root, &manifest, VALID_WASM, true);

    let registry =
        FilePluginRegistry::load_from_roots(&[root.clone()], HOST_RUNTIME_API_VERSION).unwrap();
    let host = PluginRuntimeHost::new(registry, StaticPolicyResolver::default()).unwrap();

    let plan = host
        .tool_plan(&PlanRequest {
            command: "echo".to_string(),
            input: serde_json::json!({ "message": "hello" }),
            context: context("wp2-fixture", "echo"),
        })
        .unwrap();
    assert!(plan.blocking_issues.is_empty());
    assert_eq!(plan.estimated_steps, 1);

    let response = host
        .tool_invoke(&InvokeRequest {
            command: "echo".to_string(),
            input: serde_json::json!({ "message": "hello" }),
            context: context("wp2-fixture", "echo"),
        })
        .unwrap();

    let result = response.result.expect("result expected");
    assert_eq!(result.summary, "ok");
    assert_eq!(result.data["message"], "hello");
    assert!(response.error.is_none());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn missing_plugin_wasm_is_rejected() {
    let root = temp_dir("missing-wasm");
    let manifest = fixture_manifest("wp2-missing");
    write_bundle(&root, &manifest, VALID_WASM, false);

    let error =
        FilePluginRegistry::load_from_roots(&[root.clone()], HOST_RUNTIME_API_VERSION).unwrap_err();
    assert!(error.to_string().contains("missing plugin.wasm"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn invalid_component_returns_load_failed() {
    let root = temp_dir("invalid-component");
    let manifest = fixture_manifest("wp2-invalid");
    write_bundle(&root, &manifest, INVALID_WASM, true);

    let registry =
        FilePluginRegistry::load_from_roots(&[root.clone()], HOST_RUNTIME_API_VERSION).unwrap();
    let host = PluginRuntimeHost::new(registry, StaticPolicyResolver::default()).unwrap();

    let response = host
        .tool_invoke(&InvokeRequest {
            command: "echo".to_string(),
            input: serde_json::json!({ "message": "hello" }),
            context: context("wp2-invalid", "echo"),
        })
        .unwrap();

    assert!(response.result.is_none());
    assert_eq!(
        response.error.expect("error expected").code,
        "RUNTIME_COMPONENT_LOAD_FAILED"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn output_schema_mismatch_returns_contract_error() {
    let root = temp_dir("schema-mismatch");
    let manifest = fixture_manifest("wp2-schema");
    write_bundle(&root, &manifest, VALID_WASM, true);

    let registry =
        FilePluginRegistry::load_from_roots(&[root.clone()], HOST_RUNTIME_API_VERSION).unwrap();
    let host = PluginRuntimeHost::new(registry, StaticPolicyResolver::default()).unwrap();

    let response = host
        .tool_invoke(&InvokeRequest {
            command: "bad_json".to_string(),
            input: serde_json::json!({}),
            context: context("wp2-schema", "bad_json"),
        })
        .unwrap();

    assert!(response.result.is_none());
    assert_eq!(
        response.error.expect("error expected").code,
        "RUNTIME_CONTRACT_MISMATCH"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn denied_capability_is_blocked_before_invocation() {
    let root = temp_dir("denied-capability");
    let manifest = fixture_manifest("wp2-denied");
    write_bundle(&root, &manifest, VALID_WASM, true);

    let registry =
        FilePluginRegistry::load_from_roots(&[root.clone()], HOST_RUNTIME_API_VERSION).unwrap();
    let host = PluginRuntimeHost::new(
        registry,
        policy_with_limits(
            ResourceLimits {
                memory_bytes: Some(134_217_728),
                time_ms: Some(5_000),
                storage_bytes: Some(268_435_456),
                output_bytes: Some(131_072),
                network_policy: Some(NetworkPolicy::Denied),
            },
            BTreeSet::from(["storage.private".to_string()]),
        ),
    )
    .unwrap();

    let response = host
        .tool_invoke(&InvokeRequest {
            command: "write_storage".to_string(),
            input: serde_json::json!({ "bytes": 1024 }),
            context: context("wp2-denied", "write_storage"),
        })
        .unwrap();

    assert!(response.result.is_none());
    assert_eq!(
        response.error.expect("error expected").code,
        "CAPABILITY_DENIED"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn brokered_service_without_service_handle_is_denied_before_invocation() {
    let root = temp_dir("unsupported-capability");
    let manifest = fixture_manifest("wp2-unsupported");
    write_bundle(&root, &manifest, VALID_WASM, true);

    let registry =
        FilePluginRegistry::load_from_roots(&[root.clone()], HOST_RUNTIME_API_VERSION).unwrap();
    let host = PluginRuntimeHost::new(registry, StaticPolicyResolver::default()).unwrap();

    let response = host
        .tool_invoke(&InvokeRequest {
            command: "needs_service".to_string(),
            input: serde_json::json!({}),
            context: context("wp2-unsupported", "needs_service"),
        })
        .unwrap();

    assert!(response.result.is_none());
    assert_eq!(
        response.error.expect("error expected").code,
        "SERVICE_ACCESS_DENIED"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn brokered_github_request_succeeds_without_exposing_secret() {
    let root = temp_dir("github-broker");
    let manifest = fixture_manifest("wp3-broker");
    write_bundle(&root, &manifest, VALID_WASM, true);

    let registry =
        FilePluginRegistry::load_from_roots(&[root.clone()], HOST_RUNTIME_API_VERSION).unwrap();
    let host = host_with_github_service(registry);

    let response = host
        .tool_invoke(&InvokeRequest {
            command: "needs_service".to_string(),
            input: serde_json::json!({
                "serviceHandle": "svc_github",
                "method": "GET",
                "path": "/repos/oberon4/status"
            }),
            context: context("wp3-broker", "needs_service"),
        })
        .unwrap();

    let result = response.result.expect("result expected");
    assert_eq!(result.data["statusCode"], 200);
    assert_eq!(result.data["body"]["ok"], true);
    let encoded = serde_json::to_string(&result).unwrap();
    assert!(!encoded.contains("gh-secret-token"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn brokered_service_output_is_redacted_before_agent_visibility() {
    let root = temp_dir("github-redaction");
    let manifest = fixture_manifest("wp3-redaction");
    write_bundle(&root, &manifest, VALID_WASM, true);

    let registry =
        FilePluginRegistry::load_from_roots(&[root.clone()], HOST_RUNTIME_API_VERSION).unwrap();
    let host = host_with_github_service(registry);

    let response = host
        .tool_invoke(&InvokeRequest {
            command: "needs_service".to_string(),
            input: serde_json::json!({
                "serviceHandle": "svc_github",
                "method": "GET",
                "path": "/repos/oberon4/status"
            }),
            context: context("wp3-redaction", "needs_service"),
        })
        .unwrap();

    let result = response.result.expect("result expected");
    assert_eq!(result.data["body"]["tokenEcho"], "<redacted>");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn output_limit_returns_structured_error() {
    let root = temp_dir("output-limit");
    let manifest = fixture_manifest("wp2-output");
    write_bundle(&root, &manifest, VALID_WASM, true);

    let registry =
        FilePluginRegistry::load_from_roots(&[root.clone()], HOST_RUNTIME_API_VERSION).unwrap();
    let host = PluginRuntimeHost::new(
        registry,
        policy_with_limits(
            ResourceLimits {
                memory_bytes: Some(134_217_728),
                time_ms: Some(5_000),
                storage_bytes: Some(268_435_456),
                output_bytes: Some(512),
                network_policy: Some(NetworkPolicy::Denied),
            },
            BTreeSet::new(),
        ),
    )
    .unwrap();

    let response = host
        .tool_invoke(&InvokeRequest {
            command: "big_output".to_string(),
            input: serde_json::json!({ "bytes": 10_000 }),
            context: context("wp2-output", "big_output"),
        })
        .unwrap();

    assert!(response.result.is_none());
    assert_eq!(
        response.error.expect("error expected").code,
        "RESOURCE_LIMIT_OUTPUT"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn storage_limit_returns_structured_error() {
    let root = temp_dir("storage-limit");
    let manifest = fixture_manifest("wp2-storage");
    write_bundle(&root, &manifest, VALID_WASM, true);

    let registry =
        FilePluginRegistry::load_from_roots(&[root.clone()], HOST_RUNTIME_API_VERSION).unwrap();
    let host = PluginRuntimeHost::new(
        registry,
        policy_with_limits(
            ResourceLimits {
                memory_bytes: Some(134_217_728),
                time_ms: Some(5_000),
                storage_bytes: Some(512),
                output_bytes: Some(131_072),
                network_policy: Some(NetworkPolicy::Denied),
            },
            BTreeSet::new(),
        ),
    )
    .unwrap();

    let response = host
        .tool_invoke(&InvokeRequest {
            command: "write_storage".to_string(),
            input: serde_json::json!({ "bytes": 4096 }),
            context: context("wp2-storage", "write_storage"),
        })
        .unwrap();

    assert!(response.result.is_none());
    assert_eq!(
        response.error.expect("error expected").code,
        "RESOURCE_LIMIT_STORAGE"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn fresh_store_per_invocation_prevents_state_leak() {
    let root = temp_dir("fresh-store");
    let manifest = fixture_manifest("wp2-fresh");
    write_bundle(&root, &manifest, VALID_WASM, true);

    let registry =
        FilePluginRegistry::load_from_roots(&[root.clone()], HOST_RUNTIME_API_VERSION).unwrap();
    let host = PluginRuntimeHost::new(registry, StaticPolicyResolver::default()).unwrap();

    let first = host
        .tool_invoke(&InvokeRequest {
            command: "counter".to_string(),
            input: serde_json::json!({}),
            context: context("wp2-fresh", "counter"),
        })
        .unwrap()
        .result
        .expect("first result");
    let second = host
        .tool_invoke(&InvokeRequest {
            command: "counter".to_string(),
            input: serde_json::json!({}),
            context: context("wp2-fresh", "counter"),
        })
        .unwrap()
        .result
        .expect("second result");

    assert_eq!(first.data["value"], 1);
    assert_eq!(second.data["value"], 1);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn timeout_returns_structured_error() {
    let root = temp_dir("timeout");
    let manifest = fixture_manifest("wp2-timeout");
    write_bundle(&root, &manifest, VALID_WASM, true);

    let registry =
        FilePluginRegistry::load_from_roots(&[root.clone()], HOST_RUNTIME_API_VERSION).unwrap();
    let host = PluginRuntimeHost::new(
        registry,
        policy_with_limits(
            ResourceLimits {
                memory_bytes: Some(134_217_728),
                time_ms: Some(1),
                storage_bytes: Some(268_435_456),
                output_bytes: Some(131_072),
                network_policy: Some(NetworkPolicy::Denied),
            },
            BTreeSet::new(),
        ),
    )
    .unwrap();

    let response = host
        .tool_invoke(&InvokeRequest {
            command: "spin".to_string(),
            input: serde_json::json!({ "bytes": 5_000_000_000u64 }),
            context: context("wp2-timeout", "spin"),
        })
        .unwrap();

    assert!(response.result.is_none());
    assert_eq!(
        response.error.expect("error expected").code,
        "RESOURCE_LIMIT_TIMEOUT"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn memory_limit_returns_structured_error() {
    let root = temp_dir("memory");
    let manifest = fixture_manifest("wp2-memory");
    write_bundle(&root, &manifest, VALID_WASM, true);

    let registry =
        FilePluginRegistry::load_from_roots(&[root.clone()], HOST_RUNTIME_API_VERSION).unwrap();
    let host = PluginRuntimeHost::new(
        registry,
        policy_with_limits(
            ResourceLimits {
                memory_bytes: Some(8_388_608),
                time_ms: Some(20_000),
                storage_bytes: Some(268_435_456),
                output_bytes: Some(131_072),
                network_policy: Some(NetworkPolicy::Denied),
            },
            BTreeSet::new(),
        ),
    )
    .unwrap();

    let response = host
        .tool_invoke(&InvokeRequest {
            command: "allocate".to_string(),
            input: serde_json::json!({ "bytes": 64_000_000 }),
            context: context("wp2-memory", "allocate"),
        })
        .unwrap();

    assert!(response.result.is_none());
    assert_eq!(
        response.error.expect("error expected").code,
        "RESOURCE_LIMIT_MEMORY"
    );

    let _ = fs::remove_dir_all(root);
}
