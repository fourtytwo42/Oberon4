# Oberon4 WASM/WASI Tool Runtime Architecture

Companion specifications:
- `docs/prd/tool-interaction-contract.md`
- `docs/prd/provider-tool-compatibility.md`
- `docs/prd/plugin-runtime-contract.md`
- `docs/prd/plugin-archive-schema.md`

## 1. System Shape
The WASM/WASI tool runtime is a host-managed subsystem inside Oberon4.

Primary layers:
- host tool runtime
- plugin registry and manifest index
- prompt and discovery broker
- WASM/WASI execution engine
- capability and storage broker
- secret custody and brokered integration broker
- audit/logging pipeline

The agent should not interact with raw plugins directly. The agent interacts with a stable host-facing tool surface, and the host routes execution to plugins.

Portability rule:
- the plugin contract should be Oberon-native, not OS-native
- plugins target Oberon runtime abstractions and portable WASM/WASI contracts
- operating-system-specific behavior is implemented only in host adapter layers

## 1.1 Tool Tiers
The system should distinguish three tool tiers:

- `Tier 1: Core host tools`
  - discovery, describe, plan, invoke, policy inspection, and other trust-anchor functions
- `Tier 2: Brokered privileged service tools`
  - integrations that need external auth, risky side effects, or sensitive host capabilities
- `Tier 3: Ordinary WASM plugins`
  - self-contained tools that operate within the standard capability envelope

Examples:
- core host tools:
  - `tool_search`
  - `tool_describe`
  - `tool_plan`
  - `tool_invoke`
- brokered privileged services:
  - GitHub
  - Slack or Discord messaging
  - browser control
  - database access
- ordinary WASM plugins:
  - local search
  - parsers
  - transforms
  - media processing

Portable ordinary plugins should run unchanged across supported Oberon hosts when they stay within the portable capability envelope.

First-party package rule:
- brokered privileged packages such as `workspace`, `git`, and `github` may be installable and removable from package roots without becoming ordinary WASM plugins
- those packages stay host-trusted and host-brokered even when their registration is file-based and modular

## 2. Agent-Facing Contract
Recommended host-native tool surface:
- `tool_search`
  - searches installed tools, commands, tags, examples, and side-effect metadata
- `tool_describe`
  - returns verbose command help, examples, constraints, and schemas for one tool or command
- `tool_plan`
  - returns an execution plan, side-effect summary, approval requirements, and estimated cost or scope when supported
- `tool_invoke`
  - invokes `<tool>.<command>` with validated structured input
- optional host-native inspection tools:
  - `tool_policy_inspect`
  - `tool_artifact_describe`

`tool_policy_inspect` should expose:
- effective capabilities
- blocked capabilities or commands
- approval requirements
- effective resource budgets
- operating mode
- reason codes for policy denials where safe to disclose

Design rule:
- do not inject every plugin command schema into the root system prompt
- use progressive disclosure:
  - root prompt explains the workflow
  - `tool_search` returns compact matches
  - `tool_describe` returns verbose usage guidance
  - `tool_plan` returns preflight execution understanding for risky or stateful commands
  - `tool_invoke` performs execution
- keep the agent-facing contract free of platform-specific assumptions such as shell syntax, platform-native paths, or host process-manager semantics

## 3. Runtime Flow
1. host scans configured archive roots for `.obplg.zip` and `.obpkg.zip` artifacts, plus compatibility plugin and package roots when enabled
2. host validates manifest structure, ids, versions, capability declarations, and schemas
3. host generates or refreshes `plugin-config/<id>.toml` for each discovered archive
4. host builds a search index from manifest metadata without executing plugins
5. agent calls `tool_search`
6. host returns compact ranked matches
7. agent calls `tool_describe`
8. host returns verbose plugin prompting, schema, examples, and safety notes
9. agent optionally calls `tool_plan`
10. host returns side-effect class, required capabilities, approval expectations, and estimated execution details
11. agent calls `tool_invoke`
12. host validates input schema and policy
13. host prepares isolated WASI execution context and storage mounts
14. host invokes the plugin command
15. host validates output schema and returns structured results or structured errors

For brokered privileged services, steps 12 and 13 may route to a host-owned integration adapter instead of a generic ordinary plugin runtime.

On different operating systems, those host-owned adapters may have different implementations while preserving the same runtime contract.

## 3.1 Recommended Runtime Defaults
Recommended defaults for the first implementation:
- canonical Oberon-native tool model internally, provider adapters at the edges
- one ordinary plugin instance per invocation
- typed WIT or component-model host/plugin transport
- JSON-schema validation at the agent-facing boundary
- invocation-scoped opaque capability handles
- immutable host-managed artifacts for large payloads
- host-brokered service operations for sensitive integrations
- side-effect-driven approval decisions

These defaults bias toward simpler isolation, clearer auditing, and easier cross-platform behavior.

## 4. Host Responsibilities
- registry management
- manifest validation
- search indexing
- prompt assembly and staged disclosure
- capability enforcement
- capability handle issuance and revocation
- storage mount assignment
- workspace scope assignment
- artifact/blob handle management
- input and output schema validation
- resource limits
- error normalization
- structured logs/traces
- lifecycle and version compatibility policy
- provenance verification and conformance gating
- secret custody and redaction enforcement

Platform-specific responsibilities that stay in host adapters:
- native secret store integration
- host filesystem mapping
- browser runtime integration
- platform-authenticated external service adapters
- OS-specific logging or keychain plumbing

The host is the trust anchor. Plugins are not trusted to self-police capability use.

## 5. Plugin Responsibilities
- implement tool logic only
- expose stable command handlers
- return structured results and errors
- use only granted host services and storage
- keep internal implementation self-contained

Plugins do not own:
- top-level registration policy
- host path selection
- host shell access
- direct secrets access
- prompt injection into the root system prompt
- direct authenticated access to external services unless explicitly exposed through a brokered host API
- platform-specific code paths as part of the ordinary portable plugin contract

## 6. Plugin Package Contract
Primary install contract:
- archive wrapper `oberon-archive.json`
- inner runtime manifest(s)
- a WASM module for ordinary plugins
- optional bundled docs/examples
- optional test fixtures and migration metadata

Generated config contract:
- every discovered archive gets a generated `plugin-config/<id>.toml`
- generated config stores install metadata, enabled state, load status, last error, and operator-editable settings
- generated config remains after archive removal so the host can report missing-installed state explicitly

Minimum manifest fields:
- `id`
- `version`
- `displayName`
- `description`
- `commands`
- `capabilities`
- `prompting`

Each command should declare:
- stable command name
- stable command version or schema version
- description
- input schema
- output schema
- tags
- side-effect classification
- required capabilities

Recommended additional command metadata:
- approval expectation
- supports planning
- supports pagination
- supports artifact handles
- precondition model for stateful edits

Manifest and command metadata should remain portable:
- avoid requiring platform-specific config keys for ordinary plugins
- use optional platform gating only when a plugin genuinely cannot be portable

## 7. Prompting Architecture
Prompting should be split into three layers:

### 7.1 Root agent guidance
Teaches the universal workflow:
- search before use
- describe before first invoke
- do not guess command names or argument shapes
- inspect side effects before stateful commands
- use structured tool errors to retry

### 7.2 Runtime guidance
Explains system behavior:
- tools are host-mediated
- plugins execute in isolated runtimes
- capabilities are explicit and limited
- search results are compact by design
- verbose detail must be fetched with `tool_describe`
- risky or stateful work should use `tool_plan` before `tool_invoke`
- secrets are not directly readable; use brokered operations instead
- large file or media payloads may be represented as artifact handles instead of inline text

### 7.3 Plugin guidance
Returned lazily per tool/command:
- when to use
- when not to use
- usage steps
- examples
- common mistakes
- safety notes

## 8. Storage Architecture
Default storage model:
- plugin private durable storage
- plugin invocation scratch storage
- plugin rebuildable cache storage

Adjacent data plane:
- host-managed artifact/blob store

Rules:
- each plugin gets a durable private namespace
- each invocation gets ephemeral scratch space
- cache storage is optional, rebuildable, and safe to purge without violating correctness
- no raw arbitrary host path access
- shared storage is not present by default
- workspace mounts are future capability additions, not part of the initial runtime slice
- large binary or document payloads should prefer artifact handles over inline JSON transport
- plugins should not depend on host path formats such as POSIX paths or Windows drive-letter paths

Filesystem silo rule:
- workspace-oriented plugins should not receive direct access to the outer host filesystem
- when workspace access is enabled, the plugin should operate only inside a host-issued logical workspace sandbox backed by scoped handles
- the logical sandbox must remain portable across Linux, macOS, and Windows regardless of how the host maps it internally

Recommended default storage budgets:
- private durable storage per plugin: `64 MB`
- session scratch storage per invocation or session: `256 MB`
- rebuildable cache storage per plugin: `512 MB`
- retained artifacts per plugin: `256 MB`
- single artifact size: `32 MB`

Budget configuration rule:
- these values are recommended defaults only
- the host should resolve effective budgets through policy layers such as:
  - global runtime defaults
  - deployment or admin overrides
  - plugin-level overrides
  - command-level overrides
- resolved budgets must remain bounded by host-controlled ceilings

Recommended cleanup and eviction order:
1. expired session scratch
2. rebuildable cache by retention or LRU policy
3. expired artifacts by retention policy
4. reject further durable writes once durable quota is exhausted until policy or cleanup resolves the condition

## 9. Security Model
Security boundary assumptions:
- WASM/WASI improves isolation but is not sufficient alone
- the host capability broker remains the decisive control point

Security rules:
- deny by default
- no ambient filesystem access
- no ambient environment access
- no ambient network access
- no shell/process spawn
- no broad persistent capability grants when narrower invocation-scoped handles can be used
- explicit resource budgets
- fail closed on policy mismatch
- validate manifests before activation
- validate inputs before invocation
- validate outputs before returning results
- secrets remain host-custodied and should not be copied into agent-visible or plugin-visible general outputs
- portable plugins should not assume a particular host shell, init system, keychain backend, or filesystem layout

Best-practice policy defaults:
- ordinary plugins should not receive decrypted secret material
- brokered service calls should be preferred over combining raw network access with secret access
- output redaction should run on logs, traces, stdout, stderr, and tool result envelopes before any agent-visible rendering
- capability grants should expire automatically with the invocation unless a stronger policy artifact allows otherwise
- ordinary plugins should have raw network access denied by default
- invocations should run with explicit memory ceilings, wall-clock timeouts, and output size limits

Configuration rule:
- memory, timeout, output, storage, artifact, and network budgets should be tunable through host policy without changing the agent-facing or plugin-facing contract

## 10. Capability Model
Initial capability classes:
- `storage.private`
- `storage.session`
- `storage.cache`
- `clock.read`
- `random.get`

Future capability classes:
- `workspace.read`
- `workspace.write`
- `network.fetch`
- `secrets.access`
- `subtools.invoke`

Capability rules:
- capability declarations live in manifests
- host policy decides whether a declared capability is allowed
- undeclared capabilities are unavailable
- declared but disallowed capabilities block activation or invocation
- granted capabilities should be exposed to plugins through narrow brokered handles rather than broad ambient mounts when practical

For sensitive integrations, prefer brokered service capabilities over raw primitives:
- prefer `github.request` over raw `network.fetch` + raw token access
- prefer `slack.send` over raw webhook secret exposure
- prefer `db.query` over raw secret + raw socket access

## 10.1 Workspace and Artifact Model
Workspace-oriented tools require explicit host mediation.

Recommended future workspace capabilities:
- `workspace.read`
- `workspace.write`

Recommended future artifact capabilities:
- `artifact.create`
- `artifact.read`
- `artifact.describe`

Rules:
- workspace access should be granted through scoped host-issued handles, not unrestricted path mounts
- artifact/blob handles should be used for large documents, media, diffs, and other payloads that do not fit well in prompt-sized inline tool results
- stateful edit commands should support explicit preconditions such as file hashes, expected content windows, or patch context
- workspace-oriented contracts should use normalized Oberon path and handle semantics rather than leaking platform-native path syntax into ordinary plugins
- host path validation should reject absolute paths, parent-directory traversal, and other escape attempts before any filesystem operation is performed
- host workspace mediation should prevent symlink or reparse-point escape outside the granted logical workspace scope
- host workspace mediation should enforce per-invocation and per-plugin quotas such as bytes written, file count, and maximum file size
- file write operations should be performed through host-brokered APIs rather than by exposing raw writable mounts to ordinary plugins
- multiple plugins may operate on the same logical workspace only through separate host-issued handles with shared-workspace coordination controlled by the host
- shared-workspace writes should use workspace revision ids, preconditions, or equivalent optimistic concurrency checks

Best-practice defaults:
- artifact handles should be immutable once produced
- inline tool payloads should remain small enough to avoid prompt bloat and accidental secret leakage
- workspace writes should use atomic replacement semantics by default
- stateful edits should fail if preconditions do not match current workspace state
- ordinary plugin contracts should use normalized relative paths only, not host-native absolute paths
- write-style commands should return structured post-write metadata such as touched logical path, bytes written, and precondition outcome
- shared-workspace write results should include revision or conflict metadata so agents can re-read and re-plan safely

Recommended workspace snapshot metadata:
- `workspace_revision`
- `snapshot_id`
- `base_revision`
- `conflict_reason`

Snapshots or revisions should let agents re-read, re-plan, and recover safely when concurrent changes happen.

## 11. Search and Discovery Index
The registry index should include:
- tool id
- tool version
- tool tier
- tool description
- command name
- command description
- tags
- side-effect classification
- capability summary
- approval expectation
- example intents
- prompting summary

Search should run against the index, not by loading every module at query time.

## 12. Error Contract
All host-mediated errors should be structured.

Recommended fields:
- `status`
- `code`
- `message`
- `retryable`
- `hint`
- `tool`
- `command`
- `pluginVersion`
- `requestId`
- `nextSuggestedAction`

Sensitive-data rule:
- error payloads must never include raw secret values or raw secret-bearing request headers

The agent should be able to repair a bad invocation using only the returned error plus `tool_describe`.

Best-practice behavior:
- identical failing invocations should not be retried automatically more than once
- timeouts, memory limits, policy denials, and schema mismatches should have distinct error codes
- pending or delegated execution should be a first-class status, not encoded as a generic failure

Recommended resource-limit error codes:
- `RESOURCE_LIMIT_STORAGE`
- `RESOURCE_LIMIT_MEMORY`
- `RESOURCE_LIMIT_TIMEOUT`
- `RESOURCE_LIMIT_OUTPUT`
- `NETWORK_BLOCKED`
- `NETWORK_QUOTA_EXCEEDED`

## 13. Compatibility Rules
- manifest versioning must be explicit
- host runtime major-version incompatibilities must fail clearly
- command names and schema contracts are stable public interfaces
- host-owned discovery and invoke tools should remain stable even as plugin inventory changes
- provenance format and lifecycle states must remain stable enough for operational tooling and audits

## 14. Provenance, Lifecycle, and Conformance
Each plugin should carry provenance metadata such as:
- publisher or owner id
- source uri or package source
- artifact checksum
- signature metadata when enabled
- build or packaging metadata

Lifecycle states should include:
- installed
- validated
- active
- disabled
- upgrade-pending
- failed

Best-practice lifecycle default:
- the runtime should instantiate ordinary plugins per invocation and tear them down after completion
- any future pooling or warm-instance reuse should require dedicated benchmarking, leak checks, and policy review

Operating modes should influence:
- plugin activation strictness
- resource budgets
- approval defaults
- audit expectations
- debug visibility

The host should provide a conformance suite covering:
- manifest validation
- schema validation
- denied capability behavior
- storage isolation
- timeout and memory budget behavior
- structured error contract compliance
- secret redaction compliance
- artifact handle behavior where applicable
- output-size limit behavior
- cleanup or eviction behavior for scratch, cache, and artifact storage

Cross-platform conformance should run on:
- Linux
- macOS
- Windows

Portable plugins should not require manifest changes across those platforms.

## 15. Secret Custody Best Practice
Recommended rule: agents and plugins should never receive raw secret values when an equivalent brokered host operation is possible.

Best-practice secret model:
- store secrets only in a host-owned secret provider
- identify secrets by opaque ids or handles
- let tools request secret use, not secret reads
- perform authenticated external requests in host-owned brokers whenever possible
- redact secrets from logs, traces, stderr/stdout capture, tool results, and error payloads
- minimize secret exposure windows in memory and avoid copying secrets into general plugin config structures

Preferred pattern:
- plugin asks host to perform `github.request` using credential handle `cred_github_default`
- host resolves and applies the credential internally
- plugin receives only the API response and redacted diagnostics

Avoid:
- injecting `GITHUB_TOKEN`, `SLACK_TOKEN`, database passwords, or API keys directly into plugin env vars
- returning secrets to the agent for “debugging”
- logging raw request headers or auth-bearing URLs

## 15.1 Cross-Platform Secret Adapter Model
Secret handling should be Oberon-native at the contract layer and OS-native only at the adapter layer.

Stable Oberon secret concepts:
- `secret_handle`
- `secret_use_handle`
- `service_handle`
- `secret_class`
- `allowed_operations`
- `security_level`

Secret classes may include:
- API credential
- OAuth credential
- SSH key
- wallet key

Rules:
- plugins and agents should interact only with Oberon secret handles and brokered operations
- platform-specific details such as keychain names, credential-manager entries, socket paths, key files, or hardware-wallet transports must remain hidden behind host adapters
- the same plugin and tool contract should work unchanged across Linux, macOS, and Windows

Possible host adapter backends:
- macOS Keychain or Secure Enclave
- Windows Credential Manager or DPAPI-backed storage
- Linux Secret Service or equivalent secure store
- encrypted Oberon local vault as a portable fallback
- hardware-backed or external signing providers when available

Fallback rule:
- if a native secure store is unavailable, the host may fall back to an encrypted Oberon-managed local vault
- this fallback must preserve the same Oberon secret-handle contract
- the backend security level may differ, but the contract must not

Recommended brokered operations:
- `github.request(service_handle, request)`
- `oauth.request(service_handle, request)`
- `ssh.sign(service_handle, payload)`
- `wallet.sign_tx(service_handle, tx)`
- `wallet.sign_message(service_handle, message)`

Security-level metadata may include:
- `hardware-backed`
- `os-protected`
- `encrypted-local`

That metadata may vary by platform, but plugin behavior and host policy surfaces should remain stable.

## 16. Portability Best Practice
Recommended rule: ordinary plugins should be written against Oberon abstractions only.

Portable plugin guidance:
- do not assume `bash`, PowerShell, or any shell exists
- do not assume POSIX-only or Windows-only path syntax
- do not assume launchd, systemd, or any OS process manager
- do not assume a specific secret-store backend
- do not assume SSH agent socket paths, wallet keystore paths, or OS-native credential APIs
- do not assume host-specific temp or cache directory layout
- use capability handles, artifact handles, and logical workspace paths provided by the host runtime

Host guidance:
- keep platform-specific code inside host adapters
- preserve stable plugin contracts across Linux, macOS, and Windows
- test the same portable plugin packages unchanged across supported hosts

## 17. Initial Delivery Recommendation
Ship the runtime foundation before shipping powerful plugins.

First implementation slice:
- registry
- manifest validation
- search index
- `tool_search`
- `tool_describe`
- `tool_plan`
- `tool_invoke`
- WASM/WASI executor
- private durable storage
- ephemeral scratch storage
- rebuildable cache storage
- input/output validation
- capability handle broker
- structured logs and errors
- provenance metadata plumbing
- plugin conformance harness
- secret custody and redaction broker

Defer:
- workspace write plugin
- network plugins
- secret-aware plugins
- plugin chaining
- user-installed ecosystem without provenance controls
