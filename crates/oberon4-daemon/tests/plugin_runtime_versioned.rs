use std::{collections::BTreeSet, sync::Arc};

use oberon4_core::{
    CapabilityGrant, CapabilityGrantKind, HandleOperation, HostToolRuntime, InMemorySecretStore,
    InvocationContext, InvokeRequest, NetworkPolicy, OperatingMode, PlanRequest,
    PolicyInspectRequest, RequestId, ResourceLimits, SearchRequest, SecretStore,
    ToolDescribeRequest,
};
use oberon4_daemon::{
    EncryptedLocalSecretBackend, FilePluginRegistry, HOST_RUNTIME_API_VERSION,
    InMemoryServiceTransport, PluginRuntimeHost, PluginRuntimeRedactor, PluginRuntimeSecretBroker,
    PluginRuntimeServiceBroker, RuntimeBrokerFoundations, SecretBinding, ServiceBinding,
    StaticPolicyResolver,
};

fn context(plugin_id: &str, command: &str) -> InvocationContext {
    InvocationContext {
        request_id: RequestId::new(),
        session_id: "sess_wp5".to_string(),
        plugin_id: plugin_id.to_string(),
        plugin_version: "0.1.0".to_string(),
        command: command.to_string(),
        deadline: "2026-03-09T00:00:05Z".to_string(),
        runtime_api_version: HOST_RUNTIME_API_VERSION.to_string(),
        capability_grants: Vec::new(),
        workspace_revision: None,
        memory_limit: 134_217_728,
        time_limit_ms: 5_000,
        output_limit_bytes: 131_072,
        storage_limit_bytes: 268_435_456,
        network_policy: NetworkPolicy::Denied,
    }
}

fn context_with_workspace(
    plugin_id: &str,
    command: &str,
    workspace_handle: &str,
    workspace_revision: Option<&str>,
) -> InvocationContext {
    let mut context = context(plugin_id, command);
    context.capability_grants.push(CapabilityGrant {
        kind: CapabilityGrantKind::WorkspaceHandle,
        id: workspace_handle.to_string(),
        operations: vec![
            HandleOperation::Read,
            HandleOperation::Write,
            HandleOperation::List,
            HandleOperation::Stat,
        ],
    });
    context.workspace_revision = workspace_revision.map(str::to_string);
    context
}

fn builtin_registry() -> FilePluginRegistry {
    FilePluginRegistry::load_from_roots(&[], HOST_RUNTIME_API_VERSION).unwrap()
}

fn host_with_github_service() -> PluginRuntimeHost<FilePluginRegistry, StaticPolicyResolver> {
    let mut secret_store = InMemorySecretStore::default();
    secret_store
        .set_secret("oauth.github.main", "gh-secret-token")
        .unwrap();

    let secret_broker = Arc::new(PluginRuntimeSecretBroker::new(
        EncryptedLocalSecretBackend::new(secret_store),
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
        InMemoryServiceTransport::default()
            .with_response(
                "github.request",
                "GET",
                "/repos/oberon4/demo",
                oberon4_daemon::plugin_runtime_broker::StubbedServiceResponse {
                    status_code: 200,
                    headers: Vec::new(),
                    body: serde_json::json!({
                        "defaultBranch": "main",
                        "private": true
                    }),
                },
            )
            .with_response(
                "github.request",
                "POST",
                "/repos/oberon4/demo/git/refs/heads/main",
                oberon4_daemon::plugin_runtime_broker::StubbedServiceResponse {
                    status_code: 200,
                    headers: vec![oberon4_core::HttpHeader {
                        name: "authorization".to_string(),
                        value: "Bearer gh-secret-token".to_string(),
                    }],
                    body: serde_json::json!({
                        "ok": true,
                        "tokenEcho": "gh-secret-token"
                    }),
                },
            ),
    ));

    PluginRuntimeHost::with_runtime_services(
        builtin_registry(),
        StaticPolicyResolver::default(),
        RuntimeBrokerFoundations::default(),
        secret_broker,
        service_broker,
        Arc::new(PluginRuntimeRedactor),
    )
    .unwrap()
}

fn policy_with_blocked(capability: &str) -> StaticPolicyResolver {
    StaticPolicyResolver::new(
        OperatingMode::Production,
        ResourceLimits {
            memory_bytes: Some(134_217_728),
            time_ms: Some(5_000),
            storage_bytes: Some(268_435_456),
            output_bytes: Some(131_072),
            network_policy: Some(NetworkPolicy::Denied),
        },
        BTreeSet::from([capability.to_string()]),
    )
}

#[test]
fn builtin_versioned_commands_are_searchable_describable_and_policy_visible() {
    let registry = builtin_registry();
    let host = PluginRuntimeHost::new(registry, StaticPolicyResolver::default()).unwrap();

    let search = host
        .tool_search(&SearchRequest {
            query: "git push github repo".to_string(),
            limit: None,
            session_id: None,
        })
        .unwrap();
    assert!(
        search
            .matches
            .iter()
            .any(|entry| { entry.tool == "git" && entry.command == "push" })
    );
    assert!(
        search
            .matches
            .iter()
            .any(|entry| { entry.tool == "github" && entry.command == "clone_to_workspace" })
    );

    let describe = host
        .tool_describe(&ToolDescribeRequest {
            tool: "git".to_string(),
            command: "push".to_string(),
        })
        .unwrap();
    assert_eq!(describe.tool, "git");
    assert_eq!(describe.command, "push");
    assert_eq!(
        describe.capabilities_required,
        vec![
            "workspace.read".to_string(),
            "service.github.request".to_string()
        ]
    );

    let inspect = host
        .tool_policy_inspect(&PolicyInspectRequest {
            tool: "git".to_string(),
            command: "push".to_string(),
        })
        .unwrap();
    assert!(inspect.approval_required);
    assert!(
        inspect
            .capabilities
            .allowed
            .contains(&"service.github.request".to_string())
    );
}

#[test]
fn workspace_commands_enforce_revision_and_support_patching() {
    let host = host_with_github_service();

    let clone = host
        .tool_invoke(&InvokeRequest {
            command: "clone_to_workspace".to_string(),
            input: serde_json::json!({ "repo": "oberon4/demo" }),
            context: context("github", "clone_to_workspace"),
        })
        .unwrap();
    assert!(clone.error.is_none(), "{:?}", clone.error);
    let clone = clone.result.expect("clone result");
    let workspace = clone.data["workspaceHandle"].as_str().unwrap().to_string();

    let initial_status = host
        .tool_invoke(&InvokeRequest {
            command: "status".to_string(),
            input: serde_json::json!({ "workspaceHandle": workspace }),
            context: context_with_workspace("git", "status", &workspace, None),
        })
        .unwrap()
        .result
        .expect("status result");
    let revision = initial_status.data["workspaceRevision"]
        .as_str()
        .unwrap()
        .to_string();

    let write = host
        .tool_invoke(&InvokeRequest {
            command: "write_file_atomic".to_string(),
            input: serde_json::json!({
                "workspaceHandle": workspace,
                "path": "docs/notes.md",
                "content": "hello world\n",
                "expectedRevision": revision
            }),
            context: context_with_workspace("workspace", "write_file_atomic", &workspace, None),
        })
        .unwrap()
        .result
        .expect("write result");
    assert_eq!(write.data["preconditionStatus"], "matched");
    let write_revision = write.data["workspaceRevision"]
        .as_str()
        .unwrap()
        .to_string();

    let read = host
        .tool_invoke(&InvokeRequest {
            command: "read_file".to_string(),
            input: serde_json::json!({
                "workspaceHandle": workspace,
                "path": "docs/notes.md"
            }),
            context: context_with_workspace("workspace", "read_file", &workspace, None),
        })
        .unwrap()
        .result
        .expect("read result");
    assert_eq!(read.data["content"], "hello world\n");

    let patch = host
        .tool_invoke(&InvokeRequest {
            command: "apply_patch".to_string(),
            input: serde_json::json!({
                "workspaceHandle": workspace,
                "expectedRevision": write_revision,
                "patch": "*** Begin Patch\n*** Update File: docs/notes.md\n@@\n-hello world\n+hello oberon\n@@\n*** End Patch\n"
            }),
            context: context_with_workspace("workspace", "apply_patch", &workspace, None),
        })
        .unwrap();
    assert!(patch.error.is_none(), "{:?}", patch.error);
    let patch = patch.result.expect("patch result");
    assert_eq!(patch.data["preconditionStatus"], "matched");

    let reread = host
        .tool_invoke(&InvokeRequest {
            command: "read_file".to_string(),
            input: serde_json::json!({
                "workspaceHandle": workspace,
                "path": "docs/notes.md"
            }),
            context: context_with_workspace("workspace", "read_file", &workspace, None),
        })
        .unwrap()
        .result
        .expect("reread result");
    assert_eq!(reread.data["content"], "hello oberon\n");

    let stale = host
        .tool_invoke(&InvokeRequest {
            command: "write_file_atomic".to_string(),
            input: serde_json::json!({
                "workspaceHandle": workspace,
                "path": "docs/notes.md",
                "content": "stale write\n",
                "expectedRevision": "rev-0"
            }),
            context: context_with_workspace("workspace", "write_file_atomic", &workspace, None),
        })
        .unwrap();
    assert!(stale.result.is_none());
    assert_eq!(stale.error.expect("stale conflict").code, "CONFLICT");
}

#[test]
fn versioned_workspace_flow_supports_diff_commit_and_push_with_brokered_auth() {
    let host = host_with_github_service();

    let clone = host
        .tool_invoke(&InvokeRequest {
            command: "clone_to_workspace".to_string(),
            input: serde_json::json!({ "repo": "oberon4/demo" }),
            context: context("github", "clone_to_workspace"),
        })
        .unwrap();
    assert!(clone.error.is_none(), "{:?}", clone.error);
    let clone = clone.result.expect("clone result");
    let workspace = clone.data["workspaceHandle"].as_str().unwrap().to_string();

    let revision = host
        .tool_invoke(&InvokeRequest {
            command: "status".to_string(),
            input: serde_json::json!({ "workspaceHandle": workspace }),
            context: context_with_workspace("git", "status", &workspace, None),
        })
        .unwrap()
        .result
        .expect("status result")
        .data["workspaceRevision"]
        .as_str()
        .unwrap()
        .to_string();

    host.tool_invoke(&InvokeRequest {
        command: "write_file_atomic".to_string(),
        input: serde_json::json!({
            "workspaceHandle": workspace,
            "path": "README.md",
            "content": "wp5\n",
            "expectedRevision": revision
        }),
        context: context_with_workspace("workspace", "write_file_atomic", &workspace, None),
    })
    .unwrap();

    let diff = host
        .tool_invoke(&InvokeRequest {
            command: "diff".to_string(),
            input: serde_json::json!({ "workspaceHandle": workspace }),
            context: context_with_workspace("git", "diff", &workspace, None),
        })
        .unwrap()
        .result
        .expect("diff result");
    assert_eq!(diff.artifacts.len(), 1);
    assert_eq!(
        diff.data["diffArtifact"],
        serde_json::Value::String(diff.artifacts[0].artifact_id.clone())
    );

    let commit_plan = host
        .tool_plan(&PlanRequest {
            command: "commit".to_string(),
            input: serde_json::json!({
                "workspaceHandle": workspace,
                "message": "WP5 commit"
            }),
            context: context_with_workspace("git", "commit", &workspace, None),
        })
        .unwrap();
    assert!(commit_plan.approval_required);

    let commit = host
        .tool_invoke(&InvokeRequest {
            command: "commit".to_string(),
            input: serde_json::json!({
                "workspaceHandle": workspace,
                "message": "WP5 commit"
            }),
            context: context_with_workspace("git", "commit", &workspace, None),
        })
        .unwrap()
        .result
        .expect("commit result");
    let commit_id = commit.data["headCommit"].as_str().unwrap().to_string();

    let push_plan = host
        .tool_plan(&PlanRequest {
            command: "push".to_string(),
            input: serde_json::json!({
                "workspaceHandle": workspace,
                "remote": "origin",
                "branch": "main"
            }),
            context: context_with_workspace("git", "push", &workspace, None),
        })
        .unwrap();
    assert!(push_plan.approval_required);
    let approval = push_plan.approval_preview.expect("approval preview");
    assert!(approval.summary.contains("GitHub"));

    let push = host
        .tool_invoke(&InvokeRequest {
            command: "push".to_string(),
            input: serde_json::json!({
                "workspaceHandle": workspace,
                "remote": "origin",
                "branch": "main"
            }),
            context: context_with_workspace("git", "push", &workspace, None),
        })
        .unwrap()
        .result
        .expect("push result");
    assert_eq!(push.data["commit"], commit_id);
    assert_eq!(push.data["pushed"], true);

    let encoded = serde_json::to_string(&push).unwrap();
    assert!(!encoded.contains("gh-secret-token"));
}

#[test]
fn github_commands_and_policy_blocking_are_brokered_and_safe() {
    let host = host_with_github_service();

    let auth = host
        .tool_invoke(&InvokeRequest {
            command: "auth_status".to_string(),
            input: serde_json::json!({}),
            context: context("github", "auth_status"),
        })
        .unwrap()
        .result
        .expect("auth result");
    assert_eq!(auth.data["authenticated"], true);
    assert_eq!(auth.data["securityLevel"], "encrypted_local");

    let repo = host
        .tool_invoke(&InvokeRequest {
            command: "repo_info".to_string(),
            input: serde_json::json!({ "repo": "oberon4/demo" }),
            context: context("github", "repo_info"),
        })
        .unwrap()
        .result
        .expect("repo info result");
    assert_eq!(repo.data["defaultBranch"], "main");
    assert_eq!(repo.data["private"], true);

    let blocked = PluginRuntimeHost::with_runtime_services(
        builtin_registry(),
        policy_with_blocked("service.github.request"),
        RuntimeBrokerFoundations::default(),
        Arc::new(PluginRuntimeSecretBroker::new(
            EncryptedLocalSecretBackend::new(InMemorySecretStore::default()),
            Vec::new(),
        )),
        Arc::new(PluginRuntimeServiceBroker::new(
            PluginRuntimeSecretBroker::new(
                EncryptedLocalSecretBackend::new(InMemorySecretStore::default()),
                Vec::new(),
            ),
            Vec::new(),
            InMemoryServiceTransport::default(),
        )),
        Arc::new(PluginRuntimeRedactor),
    )
    .unwrap();

    let denied = blocked
        .tool_invoke(&InvokeRequest {
            command: "repo_info".to_string(),
            input: serde_json::json!({ "repo": "oberon4/demo" }),
            context: context("github", "repo_info"),
        })
        .unwrap();
    assert!(denied.result.is_none());
    assert_eq!(
        denied.error.expect("policy denial").code,
        "CAPABILITY_DENIED"
    );
}
