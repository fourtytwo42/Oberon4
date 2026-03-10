# Oberon4 Plugin Runtime Contract

Version: 0.1
Owner: Oberon4
Status: draft

## 1. Purpose
Define the canonical host/plugin runtime contract for Oberon4's WASM/WASI tool system, including the typed transport boundary, invocation envelopes, handle model, and versioning rules.

This document specifies the runtime boundary below the agent-facing tools and above the provider adapter layer.

Companion documents:
- `docs/prd/PRD-wasm-tool-runtime.md`
- `docs/prd/architecture-wasm-tool-runtime.md`
- `docs/prd/tool-interaction-contract.md`
- `docs/prd/provider-tool-compatibility.md`
- `docs/prd/plugin-archive-schema.md`

## 2. Boundary Model
The runtime has three separate contract layers:
- agent-facing tool contract
- host/plugin runtime contract
- provider adapter contract

Rules:
- the host/plugin runtime contract must be typed and portable
- the agent-facing contract may use JSON-shaped schemas optimized for model tool use
- provider adapters must never leak provider-native wire quirks into plugin exports

## 3. Portability Rule
The runtime contract is Oberon-native, not OS-native.

Rules:
- target portable `wasm32-wasip2` or component-model style plugins
- ordinary plugins must not require shell availability
- ordinary plugins must not depend on POSIX or Windows path syntax
- host adapters own platform-native filesystem, secret store, browser, and network details

## 4. Plugin Package Inputs
Primary install shape:
- ordinary plugin archive:
  - `oberon-archive.json`
  - `plugin.json`
  - `plugin.wasm`
- first-party package archive:
  - `oberon-archive.json`
  - `package.json`
  - referenced tool manifests such as `tools/*.plugin.json`

Compatibility path:
- unpacked `plugin.json + plugin.wasm`
- unpacked `package.json + tools/*.plugin.json`

The archive wrapper and inner manifests are read by the host before plugin activation.

Minimum manifest fields:
- `id`
- `version`
- `displayName`
- `description`
- `runtimeApiVersion`
- `commands`
- `capabilities`
- `prompting`
- `provenance`

Generated config rule:
- the host should generate and maintain a separate operator-editable config file for each discovered archive
- generated config should preserve operator settings while keeping install metadata authoritative and host-managed

## 5. Typed Runtime Surface
The preferred direction is a typed WIT or component-model contract for host/plugin transport.

The plugin should expose:
- `describe_plugin() -> PluginDescription`
- `list_commands() -> list<CommandDescription>`
- `plan(command, input, context) -> PlanResult` when supported
- `invoke(command, input, context) -> InvokeResult`
- `health_check() -> HealthStatus` optionally

The host should expose narrow broker interfaces for:
- logging
- time
- randomness
- capability handle resolution
- storage operations
- artifact operations
- brokered service operations
- cancellation or deadline checks

Rules:
- plugins should not receive raw ambient authority through the typed surface
- host imports should be narrow and capability-checked
- the runtime contract should be versioned independently from individual plugin versions

Best-practice default:
- use typed WIT or component-model transport for host/plugin interaction
- reserve JSON for the agent-facing schema boundary rather than the internal runtime boundary

## 6. JSON Boundary Rule
Use typed transport internally and JSON schemas externally.

Rules:
- host/plugin transport should be typed
- command input and output contracts exposed to agents should remain JSON-schema-defined
- host must validate agent input against command schema before entering plugin runtime
- host must validate plugin output against command schema before returning results to the agent

## 7. Invocation Context
Each invocation should receive a host-issued context with:
- `requestId`
- `sessionId`
- `pluginId`
- `pluginVersion`
- `command`
- `deadline`
- `capabilityGrants`
- `runtimeApiVersion`

Rules:
- invocation context is authoritative for the active call only
- capability grants should be invocation-scoped by default
- plugins must not mint or widen their own grants

Recommended invocation budget metadata:
- `memory_limit`
- `time_limit_ms`
- `output_limit_bytes`
- `storage_limit_bytes`
- `network_policy`

Configuration rule:
- these budget fields should be resolved by host policy and passed into the invocation context as effective limits
- plugins may observe granted limits, but must not assume the documented defaults are constant across deployments

Best-practice default:
- ordinary plugins should execute in fresh per-invocation instances for the first runtime slice
- invocation context and grants should be destroyed when the invocation completes or is cancelled

## 8. Capability Handle Model
Capabilities should be conveyed as opaque handles, not broad ambient permissions.

Recommended handle categories:
- `storage_handle`
- `artifact_handle`
- `workspace_handle`
- `service_handle`
- `secret_use_handle`
- `secret_handle`

Rules:
- handles are opaque to the plugin
- handles are scoped and revocable
- handles should not expose host filesystem layout, credential values, or provider-native connection details
- plugins should use broker APIs with handles rather than raw OS resources

Best-practice default:
- prefer single-invocation handles unless a specific workflow requires a longer lifetime
- service and secret-use handles should be narrower than generic network or environment access
- workspace handles should expose only a logical sandbox root, never the underlying host path
- multiple plugins may receive distinct handles to the same logical workspace, but the host remains responsible for shared-workspace coordination and conflict control

## 9. Storage Contract
The runtime should support:
- `private_durable`
- `session_ephemeral`
- `cache_rebuildable`

Rules:
- plugins may use logical relative paths inside their own storage scope
- plugins must not observe or depend on the real host storage path
- the host may map storage differently on Linux, macOS, and Windows
- plugins must not receive raw outer host filesystem paths as ordinary runtime inputs
- host mediation must reject absolute paths, parent-directory traversal, and comparable escape forms before any workspace-oriented filesystem operation
- host mediation must prevent symlink or reparse-point escape outside the granted logical workspace scope
- host mediation must enforce quotas and size limits without exposing host-native filesystem layout

Recommended default storage quotas:
- private durable storage per plugin: `64 MB`
- session scratch storage per invocation or session: `256 MB`
- rebuildable cache storage per plugin: `512 MB`

Configuration rule:
- storage quotas should be configurable by host policy at global, plugin, and command scope
- plugin manifests may declare requested budgets, but the host decides effective granted budgets

Recommended storage operations:
- open logical file
- read bytes
- write bytes
- list logical entries
- delete logical entry
- stat logical entry

Recommended workspace-oriented operations:
- read file by logical relative path
- write file atomically by logical relative path
- list directory by logical relative path
- stat logical path
- apply patch with preconditions

Recommended shared-workspace metadata:
- `workspace_revision`
- `expected_revision`
- `precondition_status`
- `conflict_reason`

These operations remain subject to capability and size policy.

## 10. Artifact Contract
Artifacts are host-managed handles for large payloads.

Artifact metadata should include:
- `artifactId`
- `mediaType`
- `size`
- `producer`
- `retention`
- `truncated`

Rules:
- large files, media, patches, and similar outputs should use artifacts instead of oversized inline JSON
- artifact ids are opaque
- artifact access must remain capability-checked

Best-practice default:
- artifacts should be immutable once created
- inline result payloads should stay small and summary-oriented
- retention should be explicit and host-controlled

Recommended default artifact budgets:
- retained artifacts per plugin: `256 MB`
- single artifact size: `32 MB`
- inline result fallback threshold: `128 KB`

Configuration rule:
- artifact retention and size budgets should be configurable by host policy
- deployments may tune artifact thresholds without changing plugin contracts or result envelope semantics

## 11. Brokered Service Contract
Brokered privileged services should be exposed as host-owned operations, not as raw secret plus raw network access.

Examples:
- `github.request`
- `slack.send`
- `db.query`
- `browser.navigate`

Rules:
- ordinary plugins do not receive raw tokens or connection strings
- service requests should reference a service or secret-use handle, not secret material
- the host remains responsible for auth, redaction, auditing, and policy enforcement

Network rule:
- ordinary plugins should have raw network access denied by default
- any future network capability should be explicitly allowlisted, budgeted, and policy-controlled

## 12. Secret Custody Contract
Secrets remain host-custodied.

Rules:
- agents must not read raw secret values
- ordinary plugins must not read raw secret values
- brokered operations should use secret references or service handles only
- logs, traces, stdout, stderr, and error envelopes must be redacted before agent visibility
- the host must preserve one stable secret-handle and broker contract across supported operating systems even when the underlying secure-store backend differs

Best-practice default:
- decrypted secret material should remain inside host-managed custody paths for as short a time as possible
- ordinary plugins should use brokered service calls instead of direct secret-backed network access

Recommended secret-use pattern:
1. host policy grants a `secret_use_handle` or `service_handle`
2. plugin invokes a narrow brokered operation
3. host performs the secret-backed action
4. host returns only the sanitized result

Cross-platform backend rule:
- host adapters may resolve secret and signing operations through native secure stores, SSH agents, wallet providers, OS crypto APIs, or encrypted local fallback storage
- those backend differences must not leak into plugin contracts, tool prompting, or handle semantics

## 12.1 Secret Class Policy Defaults
Recommended default handling by secret class:
- API credential:
  - brokered request operations allowed by policy
  - no raw secret reads
- OAuth credential:
  - brokered request operations allowed by policy
  - no raw token or refresh-token reads
- SSH key:
  - sign-only operations by default
  - no raw private-key export
- wallet key:
  - sign-only operations
  - approval required for signing by default
  - no raw private-key or seed export

## 13. Result Envelopes
Successful invocation envelopes should include:
- `status`
- `summary`
- `data`
- `artifacts`
- `metrics`
- `nextSuggestedAction`

Error envelopes should include:
- `status`
- `code`
- `message`
- `retryable`
- `requestId`
- `pluginId`
- `pluginVersion`
- `command`
- `nextSuggestedAction`

Recommended resource and policy error codes:
- `RESOURCE_LIMIT_STORAGE`
- `RESOURCE_LIMIT_MEMORY`
- `RESOURCE_LIMIT_TIMEOUT`
- `RESOURCE_LIMIT_OUTPUT`
- `NETWORK_BLOCKED`
- `NETWORK_QUOTA_EXCEEDED`

Plan envelopes should include:
- `status`
- `sideEffects`
- `approvalRequired`
- `capabilitiesRequired`
- `estimatedCost`
- `estimatedSteps`
- `preconditions`
- `blockingIssues`

## 14. Lifecycle Contract
The runtime should support explicit lifecycle actions:
- install
- validate
- activate
- deactivate
- upgrade
- uninstall

Optional lifecycle hooks may include:
- `on_install`
- `on_upgrade`
- `health_check`

Rules:
- lifecycle actions are host-controlled
- activation must fail closed on contract mismatch, capability policy failure, or conformance failure
- lifecycle state should be inspectable without loading the plugin into agent context

Best-practice default:
- install and activate only allowlisted or otherwise validated plugins in the initial shipping slice
- keep third-party distribution broader than local or allowlisted installs out of scope until provenance and revocation are fully implemented

## 14.1 Operating Modes
The runtime should support at least two operating modes:
- `dev`
- `production`

Recommended expectations:
- `dev`
  - local or unsigned plugin iteration may be allowed by policy
  - budgets and debugging visibility may be looser
  - still no raw secret reads for agents or ordinary plugins
- `production`
  - validated or allowlisted plugins only by default
  - stricter budgets and approvals
  - stricter audit and redaction expectations

## 15. Compatibility and Versioning
Required version dimensions:
- `runtimeApiVersion`
- `pluginVersion`
- `schemaVersion` per command when needed

Rules:
- host must reject plugins targeting unsupported runtime API versions
- plugins must not silently activate against incompatible host contracts
- compatibility errors must be explicit and machine-usable

## 16. Conformance Requirements
Production-eligible plugins should pass a host-run conformance suite covering:
- manifest validation
- typed contract compatibility
- schema validation
- denied capability behavior
- storage isolation
- artifact handling
- timeout behavior
- error envelope behavior
- secret redaction behavior
- cross-platform portability checks

## 17. Acceptance Rules
- `PRC-101`: host/plugin transport remains typed and portable
- `PRC-102`: agent-facing JSON schemas are validated at both ingress and egress
- `PRC-103`: capabilities are conveyed as narrow opaque handles
- `PRC-104`: artifacts and secrets use host-managed custody paths
- `PRC-105`: plugins remain portable across supported Oberon hosts without OS-specific contract changes
