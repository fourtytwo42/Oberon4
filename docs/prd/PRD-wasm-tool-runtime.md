# PRD: Oberon4 WASM/WASI Tool Runtime

Version: 0.1
Owner: Oberon4
Status: draft

Companion specifications:
- `docs/prd/tool-interaction-contract.md`
- `docs/prd/provider-tool-compatibility.md`
- `docs/prd/plugin-runtime-contract.md`

## 1. Problem Statement
Build a secure, host-managed tool plugin system for Oberon4 where agent-usable tools execute as isolated WASM/WASI modules instead of arbitrary host code. The system must let agents discover tools on demand through a compact search surface, load verbose tool guidance only when needed, and execute plugins with explicit capability boundaries and isolated storage.

## 2. Goals
- `FR-101`: host-managed WASM/WASI runtime for tool plugin execution.
- `FR-102`: agent-facing tool discovery flow that avoids prompt and context bloat.
- `FR-103`: verbose, staged prompting model for the host runtime and each plugin.
- `FR-104`: plugin packaging and manifest contract with stable command metadata and schemas.
- `FR-105`: secure per-plugin storage model with isolated durable and ephemeral spaces.
- `FR-106`: capability-based security model with deny-by-default execution boundaries.
- `FR-107`: observable invocation lifecycle with structured errors, auditability, and deterministic results.
- `FR-108`: typed host/plugin contract with stable internal transport and agent-facing schemas.
- `FR-109`: command planning, side-effect classification, and approval-aware execution flow.
- `FR-110`: plugin provenance, lifecycle management, and conformance validation.
- `FR-111`: tiered tool model separating core host tools, brokered privileged services, and ordinary WASM plugins.
- `FR-112`: workspace, artifact, and large-payload handling model for file and media oriented tools.
- `FR-113`: secret custody model where agents and plugins use secrets indirectly through host mediation rather than reading raw secret values.
- `FR-114`: cross-platform, host-agnostic runtime contract that behaves consistently across supported Oberon platforms.

## 3. Non-Goals (Initial Runtime Slice)
- host shell execution from plugins
- workspace file write support from plugins
- arbitrary network egress from plugins
- cross-plugin direct RPC
- secret-provider integration inside plugins
- user-installed untrusted plugins without validation, policy, and provenance controls
- cross-plugin shared state without explicit host mediation
- direct plugin-defined top-level prompt injection into the root system prompt
- raw secret value reads by the agent or plugin
- arbitrary raw external credential injection into plugin runtime environments
- plugin contracts that depend on shell availability, host-native process managers, or platform-specific path semantics

## 4. Functional Requirements
- `FR-101` Host-managed WASM/WASI runtime
  - `AC-101.1`: the host loads a plugin from a validated manifest plus a WASM module artifact.
  - `AC-101.2`: the host instantiates plugin execution with explicit CPU/time/memory limits.
  - `AC-101.3`: the host, not the plugin, owns tool registration, policy enforcement, and invocation routing.
  - `AC-101.4`: plugin execution fails closed when manifest validation, capability checks, or runtime initialization fails.

- `FR-102` Agent-facing discovery flow
  - `AC-102.1`: the agent can search installed tools and commands through a compact discovery tool.
  - `AC-102.2`: the agent can request verbose details for a specific tool or command before invocation.
  - `AC-102.3`: the agent can invoke a command through a stable host-mediated invocation tool without needing every plugin schema in the root prompt.
  - `AC-102.4`: discovery results include enough metadata for the agent to decide whether a tool is read-only, stateful, or otherwise constrained.

- `FR-103` Prompting model
  - `AC-103.1`: the core system prompt teaches the agent to search before use, inspect before first invocation, and retry based on structured tool errors.
  - `AC-103.2`: the runtime exposes verbose host-level operational guidance separate from plugin-specific guidance.
  - `AC-103.3`: each plugin can contribute verbose usage guidance, examples, safety notes, and command-specific help through runtime-managed prompt payloads.
  - `AC-103.4`: plugin guidance is loaded progressively so unused tool instructions do not inflate session context.

- `FR-104` Plugin manifest and command contract
  - `AC-104.1`: each plugin declares a stable `plugin id`, `version`, `description`, `commands`, `capabilities`, and prompting metadata.
  - `AC-104.2`: each command declares a stable command name, description, input schema, output schema, and side-effect classification.
  - `AC-104.3`: manifest parsing rejects malformed, ambiguous, or unsupported capability declarations.
  - `AC-104.4`: the runtime indexes manifest metadata for search without executing plugin code.

- `FR-105` Secure storage model
  - `AC-105.1`: each plugin receives a private durable storage area not shared with other plugins by default.
  - `AC-105.2`: each plugin invocation receives an isolated ephemeral workspace or scratch space.
  - `AC-105.3`: plugin storage paths are host-controlled and are not arbitrary host filesystem paths.
  - `AC-105.4`: the runtime can remove ephemeral storage after invocation or session end without affecting durable plugin state.

- `FR-106` Capability-based security
  - `AC-106.1`: plugins run with no ambient host filesystem, network, environment, or process execution privileges unless explicitly granted.
  - `AC-106.2`: capabilities are declared in the manifest and enforced by the host runtime, not trusted from plugin self-reporting alone.
  - `AC-106.3`: unsupported or disallowed capabilities fail before command execution begins.
  - `AC-106.4`: initial runtime slice supports only a minimal capability set needed for private storage and deterministic execution.

- `FR-107` Invocation lifecycle and observability
  - `AC-107.1`: tool invocation validates input against the declared schema before entering the plugin runtime.
  - `AC-107.2`: plugin output validates against the declared output schema before it is returned to the agent.
  - `AC-107.3`: runtime errors return structured machine-usable error objects with retry guidance when appropriate.
  - `AC-107.4`: the host records structured logs or traces for plugin load, search, describe, invoke, success, and failure events.
  - `AC-107.5`: the host enforces explicit resource budgets for memory, CPU or wall-clock time, output size, storage growth, and artifact growth.
  - `AC-107.6`: resource limit breaches fail with structured error codes rather than unbounded execution or opaque crashes.

- `FR-108` Typed host/plugin contract
  - `AC-108.1`: the runtime defines a stable typed host/plugin interface for plugin metadata and command execution that is independent from the agent-facing JSON contract.
  - `AC-108.2`: the agent-facing invocation contract continues to use explicit input and output schemas suitable for model-guided tool calls.
  - `AC-108.3`: every command declares a stable command id or name, versioned input schema, versioned output schema, side-effect class, and required capabilities.
  - `AC-108.4`: contract mismatches between manifest declarations and runtime-exposed commands fail activation or invocation with explicit errors.

- `FR-109` Planning and side-effect-aware execution flow
  - `AC-109.1`: the host supports a pre-execution planning or dry-run path for commands that may have side effects, approval requirements, or meaningful runtime cost.
  - `AC-109.2`: commands declare a normalized side-effect class from a host-defined taxonomy rather than free-form text.
  - `AC-109.3`: search and describe results expose side-effect class, capability requirements, and approval expectations so the agent can choose safely before invocation.
  - `AC-109.4`: the runtime can block or require approval for commands based on side-effect class and capability policy before plugin execution begins.
  - `AC-109.5`: when approval is required, the host must generate a structured approval payload that describes the requested action, affected surfaces, and policy context without exposing secrets.

- `FR-110` Provenance, lifecycle, and conformance
  - `AC-110.1`: each plugin carries provenance metadata sufficient to identify publisher, source, version, and artifact integrity.
  - `AC-110.2`: the runtime supports explicit plugin lifecycle stages such as install, activate, upgrade, deactivate, and uninstall.
  - `AC-110.3`: plugins are not considered production-eligible until they pass a host-run conformance suite covering manifest, capability, storage, timeout, and error-contract behavior.
  - `AC-110.4`: runtime logs and diagnostics preserve plugin version and provenance context for every command execution event.

- `FR-111` Tiered tool model
  - `AC-111.1`: the runtime distinguishes between core host tools, brokered privileged service tools, and ordinary WASM plugins.
  - `AC-111.2`: core host tools remain host-native and stable even as plugin inventory changes.
  - `AC-111.3`: brokered privileged service tools use narrow host APIs for external systems or risky capabilities instead of broad raw capability grants to plugins.
  - `AC-111.4`: the runtime exposes enough metadata for agents to understand which tier a tool belongs to and what trust or policy expectations apply.

- `FR-112` Workspace and artifact handling
  - `AC-112.1`: the capability model can express `workspace.read` and `workspace.write` separately, with host-controlled scope and path policy.
  - `AC-112.2`: file and media oriented commands can exchange large inputs and outputs through host-managed artifact or blob handles rather than requiring large inline payloads.
  - `AC-112.3`: read and write style tools support pagination, truncation, or chunking rules that are visible to the agent through describe or result metadata.
  - `AC-112.4`: state-changing file edit commands support explicit preconditions or conflict detection so stale or unsafe edits fail clearly.
  - `AC-112.5`: workspace-oriented plugins must use host-issued logical workspace handles and normalized relative paths rather than raw host filesystem paths.
  - `AC-112.6`: the host must reject absolute paths, parent-directory traversal, and other escape-oriented path forms before any workspace read or write occurs.
  - `AC-112.7`: the host must prevent symlink or reparse-point escape from the granted logical workspace scope before read or write completion.
  - `AC-112.8`: workspace-oriented reads and writes must be subject to host-controlled quotas, maximum file size limits, and file-count limits.
  - `AC-112.9`: the host may grant multiple plugins access to the same logical workspace through separate host-issued handles without exposing shared raw filesystem mounts.
  - `AC-112.10`: shared-workspace writes must support revision identifiers, preconditions, or equivalent conflict detection so one plugin cannot silently overwrite another plugin's changes.

- `FR-113` Secret custody and redaction
  - `AC-113.1`: the agent cannot read raw secret values from the secret store through discovery, describe, planning, invocation, or logs.
  - `AC-113.2`: ordinary plugins cannot directly read raw secret values; they may only request brokered secret use through host-approved operations or opaque secret handles.
  - `AC-113.3`: runtime logs, tool results, traces, and plugin stderr/stdout surfaces must redact secret material before it can be shown to agents or operators unless an explicitly authorized secure diagnostic path exists.
  - `AC-113.4`: brokered integrations such as GitHub or messaging providers should prefer host-owned authenticated requests over injecting raw access tokens into plugin environments.
  - `AC-113.5`: secret handling for API tokens, OAuth credentials, SSH keys, wallet keys, and similar materials must use one Oberon-native handle and broker model regardless of the underlying operating system.
  - `AC-113.6`: operating-system-specific secret stores, agents, or signing backends must remain behind host adapters and must not change the agent-facing or plugin-facing secret contract.

- `FR-114` Cross-platform Oberon-native contract
  - `AC-114.1`: the host/plugin runtime contract must avoid requiring platform-specific shell, filesystem, process-manager, or path conventions.
  - `AC-114.2`: plugins must interact with host-managed logical resources and capability handles rather than raw platform-native resource identifiers whenever practical.
  - `AC-114.3`: platform-specific differences such as secret storage, filesystem mapping, browser adapters, and external integration adapters must be confined to host implementations behind stable runtime contracts.
  - `AC-114.4`: the runtime foundation must be testable on Linux, macOS, and Windows without changing plugin manifests or command contracts for ordinary portable plugins.

## 5. Non-Functional Requirements
- `NFR-101` Security: deny-by-default capability model with host-owned policy enforcement.
- `NFR-102` Isolation: plugin durable storage must be isolated per plugin and invocation scratch storage must be isolated per run or session.
- `NFR-103` Discoverability: agent discovery flow must minimize root prompt footprint while still enabling precise tool selection.
- `NFR-104` Explainability: search, describe, and invoke flows must return sufficiently explicit metadata and error details for agent self-correction.
- `NFR-105` Compatibility: the host-plugin contract must remain stable across plugin upgrades within the same major runtime version.
- `NFR-106` Observability: plugin lifecycle and command execution must be traceable with structured events and stable identifiers.
- `NFR-107` Extensibility: additional capability classes such as workspace read, workspace write, network, and secrets must be addable without redesigning the entire host contract.
- `NFR-108` Performance: tool discovery must not require loading or instantiating every plugin module on each agent turn.
- `NFR-109` Policy precision: capability grants should be issued as narrow host-mediated handles or brokered access paths rather than broad ambient privileges.
- `NFR-110` Operability: plugin lifecycle actions, health, and compatibility state must be inspectable without manually loading plugin internals into agent context.
- `NFR-111` Secret minimization: raw secret values should have the smallest possible in-memory and runtime exposure window and should not be copied into general agent- or plugin-visible payloads.
- `NFR-112` Data containment: large binary or document payloads should be exchanged through host-managed handles to avoid prompt bloat, accidental leakage, and oversized tool results.
- `NFR-113` Portability: the plugin contract must be Oberon-native and platform-neutral so portable plugins can run unchanged across all supported Oberon host systems.
- `NFR-114` Platform isolation: operating-system-specific behavior must live in host adapter layers and must not leak into ordinary plugin contracts or prompting guidance.
- `NFR-119` Secret portability: secret custody and secret-backed operations must expose a stable Oberon-native contract across Linux, macOS, and Windows, regardless of the underlying secure storage or signing backend.
- `NFR-115` Filesystem containment: workspace-oriented plugins must remain contained within host-issued logical workspace scopes regardless of host path syntax, symlink behavior, or operating system.
- `NFR-116` Shared workspace safety: multiple plugins may operate on the same logical workspace only through host mediation with explicit revision, precondition, or conflict controls.
- `NFR-117` Resource governance: plugin runtime resource usage must be bounded by host-enforced defaults for storage, artifacts, memory, CPU or time, output size, and network policy.
- `NFR-118` Resource tunability: default resource budgets must be configurable by host policy so deployments can relax or tighten limits without changing the plugin contract.
- `NFR-120` Mode clarity: development and production operating modes must be distinguishable in host policy and must not silently change plugin contracts.

## 6. Product Rules and Operating Model
- the host runtime owns discovery, policy, prompting, schema validation, and execution routing
- plugins are implementation units, not first-class prompt-time tools
- the default agent-facing surface is a compact host-mediated tool API:
  - `tool_search`
  - `tool_describe`
  - `tool_plan`
  - `tool_invoke`
- a policy or diagnostic surface may expose host-native inspection tools such as `tool_policy_inspect` without making secrets or privileged internals readable
- plugin prompt content is retrieved lazily through the runtime rather than being permanently injected into the root system prompt
- plugin storage is secure-space based, not arbitrary host path based
- host-native discovery and policy tools remain separate from plugin-implemented business tools
- the host/plugin transport contract and the agent-facing command schema contract are separate layers and must not be conflated
- secrets are host-custodied assets, not ordinary plugin-readable configuration values
- portable plugins should depend on Oberon runtime abstractions, not directly on Linux, macOS, or Windows host behavior

Recommended operating modes:
- `dev`
- `production`

Mode differences should be expressed through host policy, not through incompatible plugin contracts.

## 6.1 Recommended Implementation Defaults
The following defaults should guide the first implementation unless a later approved change artifact explicitly overrides them:

- define one canonical Oberon-native internal model for tool definitions, tool calls, plan results, invoke results, errors, artifacts, and capability grants
- keep providers at the edge through adapter layers rather than making the runtime provider-native internally
- use typed WIT or component-model contracts at the host/plugin boundary
- keep JSON Schema at the agent-facing input/output boundary
- keep `tool_search`, `tool_describe`, `tool_plan`, `tool_invoke`, approval, policy inspection, secret custody, and audit logging host-native
- default to one ordinary plugin instance per invocation for the first shipping slice
- default capability grants to opaque, invocation-scoped, revocable handles
- keep secrets host-custodied and prefer brokered authenticated operations over raw credential injection
- move large payloads into host-managed immutable artifacts rather than inline tool results
- use normalized logical relative paths for workspace-oriented tools and reject raw absolute or platform-native paths in ordinary plugin contracts
- broker workspace file operations through host-controlled APIs rather than exposing raw writable host mounts
- enforce workspace quotas, maximum file size limits, and file-count limits at the host layer
- enforce conservative default resource budgets for storage, artifacts, memory, CPU or time, and output size at the host layer
- keep raw network access denied for ordinary plugins unless an explicit later policy enables a narrower network capability
- evict rebuildable cache and expired session scratch before durable plugin storage when reclaiming space
- tie approval defaults to side-effect class:
  - `none` and `ephemeral` -> auto-allow
  - `plugin-private-write` and `external-read` -> allow-by-policy
  - `external-write` -> approval-required
  - `destructive` -> strong approval or deny-by-default
- keep shell, raw network, raw environment, and raw secret access out of scope for ordinary plugins in the first runtime slice

## 7. Initial Capability Envelope
- allowed in initial runtime slice:
  - private durable storage
  - ephemeral scratch storage
  - rebuildable cache storage
  - clock/time read
  - random bytes
- reserved for a later but first-class supported extension path:
  - workspace read
  - workspace write
  - artifact/blob handle exchange
  - brokered service calls
- not allowed in initial runtime slice:
  - host shell/process spawn
  - host environment access
  - workspace write access
  - network access
  - cross-plugin calls
  - direct secret access

## 7.1 Default Resource Budgets
Recommended host-enforced defaults for the first runtime slice:
- private durable storage per plugin: `64 MB`
- session scratch storage per invocation or session: `256 MB`
- rebuildable cache storage per plugin: `512 MB`
- retained artifacts per plugin: `256 MB`
- single artifact size: `32 MB`
- memory per invocation: `128 MB`
- default wall-clock timeout per invocation: `5s`
- default inline output size before artifact fallback: `128 KB`
- default stdout or stderr capture per invocation: `64 KB`
- ordinary plugin network access: denied by default

These are recommended starting defaults, not hardcoded universal limits.

Configuration rules:
- deployments must be able to override these defaults through host policy
- host policy may define:
  - global defaults
  - per-plugin overrides
  - per-command overrides
  - maximum ceilings and minimum floors where appropriate
- effective budgets should be derived by host policy resolution rather than fixed in plugin code
- no plugin should run with unbounded defaults, even when limits are customized

## 8. Risks and Mitigations
- WASI isolation overconfidence risk -> keep host capability broker narrow and explicit; do not rely on WASI defaults alone
- prompt sprawl risk -> make discovery compact and detail retrieval explicit through `tool_describe`
- manifest drift risk -> validate manifest and runtime command contract together before plugin activation
- plugin ecosystem fragmentation risk -> require stable ids, versioning, and contract compatibility rules
- performance risk from plugin enumeration -> search against a host-built index, not live module instantiation
- capability overgrant risk -> use brokered per-invocation capability handles instead of broad filesystem or runtime grants
- operational drift risk -> require lifecycle metadata and conformance tests before activation
- secret exfiltration risk -> keep secrets in a host-owned custody path, prefer brokered authenticated operations, and redact all agent-visible outputs by default
- payload explosion risk -> use artifact/blob handles for large file/media results instead of inline transport

## 9. Release Gate (Runtime Foundation)
- all `AC-101.*` through `AC-107.*` are `met`
- all `AC-108.*` through `AC-110.*` are `met`
- host discovery and invocation contracts documented and testable
- capability and storage boundaries verified by automated tests
- runtime logging and structured error path verified
- secret-custody and redaction paths verified by automated tests

## 10. Open Questions
- whether the host/plugin contract should use WIT-native typed interfaces only, or WIT plus JSON payloads at the command boundary
- whether plugin provenance will require signatures, checksums, allowlists, or all three in the first shipping slice
- how plugin version compatibility will be negotiated across runtime upgrades
- which brokered service families should ship first after the runtime foundation (for example GitHub, web, or database access)
- whether file edit preconditions should be hash-based only or also support structured patch context
- whether the first runtime target should standardize immediately on WASI preview 2/component model only, or keep a transitional compatibility layer during bootstrap
