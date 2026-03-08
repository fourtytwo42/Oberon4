# Oberon4 Tool Interaction Contract

Version: 0.1
Owner: Oberon4
Status: draft

## 1. Purpose
Define the canonical agent-facing workflow for discovering, understanding, planning, and invoking tools in Oberon4 without forcing every tool schema or usage guide into the root prompt.

This document is the behavioral companion to:
- `docs/prd/PRD-wasm-tool-runtime.md`
- `docs/prd/architecture-wasm-tool-runtime.md`

Research inputs used for this contract:
- `Research/openclaw/src/agents/pi-tool-definition-adapter.ts`
- `Research/openclaw/src/agents/pi-tools.schema.ts`
- `Research/openclaw/src/agents/pi-embedded-runner/tool-split.ts`
- `Research/openclaw/src/agents/pi-embedded-runner/google.ts`
- `Research/llmhotspot-v2/TOOL_CALL_ALIGNMENT.md`
- `Research/llmhotspot-v2/tests/integration/test_hotspot_matrix.py`

## 2. Canonical Agent Tool Surface
The default agent-facing tool surface is host-native and stable:
- `tool_search`
- `tool_describe`
- `tool_plan`
- `tool_invoke`

Optional host-native inspection tools may also exist:
- `tool_policy_inspect`
- `tool_artifact_describe`

Rules:
- The agent interacts with the host tool surface, not raw plugin exports.
- The host may execute ordinary WASM plugins or brokered privileged services behind the same tool surface.
- Provider-specific tool call formats must be normalized behind this contract.

`tool_policy_inspect` should return:
- effective capability availability
- approval requirements
- effective resource budgets
- operating mode
- reason codes for blocked actions when available

## 3. Canonical Workflow
The default workflow is:
1. identify the needed capability
2. call `tool_search`
3. call `tool_describe` for the selected tool or command
4. call `tool_plan` when the command is stateful, approval-gated, high-cost, or otherwise non-trivial
5. call `tool_invoke`
6. inspect structured results or structured errors
7. retry only with changed inputs, a better command choice, or a clearer plan

The host system prompt and runtime guidance must teach this workflow explicitly.

## 4. Step Requirements

### 4.1 `tool_search`
`tool_search` is required when:
- the agent has not used the tool before in the current task
- the capability is ambiguous
- multiple candidate tools may satisfy the request
- the command name or exact tool boundary is unknown

`tool_search` may be skipped when:
- the tool and command were already described in the current task and remain applicable
- the host explicitly injects an already-scoped command contract for the current step

Expected result fields:
- `matches`
- `tool`
- `command`
- `summary`
- `toolTier`
- `sideEffects`
- `approvalRequired`
- `capabilitiesRequired`
- `confidence` or `score`

### 4.2 `tool_describe`
`tool_describe` is required:
- before first invocation of a tool or command in the current task
- after a tool search result changes the selected command
- after a validation or policy error that indicates command misuse

`tool_describe` should return:
- command summary
- input schema summary
- output schema summary
- side-effect class
- approval expectations
- capability requirements
- pagination or truncation behavior
- artifact/blob usage rules if applicable
- plugin guidance:
  - when to use
  - when not to use
  - examples
  - common mistakes
  - failure recovery hints

### 4.3 `tool_plan`
`tool_plan` is required when any of these are true:
- side-effect class is `external-write` or `destructive`
- approval is required
- the command may touch workspace state
- the command may consume large cost, time, or quota
- the command supports preconditions or conflict detection and the agent has not yet inspected them

`tool_plan` is recommended for:
- `plugin-private-write`
- brokered external integrations
- commands that may emit artifacts, patches, or multi-step execution outputs

`tool_plan` is optional for:
- `none`
- `ephemeral`
- low-cost read-only commands

Best-practice default:
- the host should enforce `tool_plan` for `external-write` and `destructive`
- the host should strongly encourage `tool_plan` for workspace edits and brokered integrations even when not strictly required

Expected result fields:
- `status`
- `tool`
- `command`
- `sideEffects`
- `approvalRequired`
- `capabilitiesRequired`
- `estimatedSteps`
- `estimatedCost`
- `artifactsPlanned`
- `preconditions`
- `blockingIssues`
- `resourceLimits` when relevant
- `approvalPreview` when approval is required

### 4.4 `tool_invoke`
`tool_invoke` must:
- validate input against the canonical command schema before provider dispatch or plugin execution
- include only the structured arguments needed for the selected command
- fail closed on missing required inputs, blocked capabilities, policy denial, or schema mismatch

The agent must not:
- guess hidden fields
- invent command names
- pass raw secrets
- bypass declared artifact or handle workflows by embedding oversized data in text fields

## 5. Prompt Loading Rules
Prompting is staged.

Root prompt content:
- workflow rules
- safety rules
- search-before-use discipline
- plan-before-state-change discipline
- structured retry discipline

Runtime guidance:
- host mediation rules
- side-effect taxonomy
- approval model
- handle-based resource model
- artifact and secret custody model

Plugin guidance:
- loaded lazily through `tool_describe`
- scoped to the selected tool or command
- should not be injected globally unless the command is already active in the current step

Rule:
- the host should prefer progressive disclosure over broad prompt injection
- verbose guidance belongs in describe results, not the root system prompt

## 6. Result Contract
Tool results returned to the agent should separate concise guidance from structured payload.

Recommended success envelope:
- `status`
- `tool`
- `command`
- `summary`
- `data`
- `artifacts`
- `truncated`
- `nextSuggestedAction`

Rules:
- `summary` should be compact and agent-readable
- `data` should remain structured and machine-usable
- large payloads should move to `artifacts`
- tool outputs should not require the agent to parse unstructured logs to find the actual result

## 6.1 Approval Payload Contract
When approval is required, the host should produce a structured approval payload for the human operator rather than relying on free-form agent prose.

Recommended approval fields:
- `approvalId`
- `tool`
- `command`
- `summary`
- `sideEffects`
- `workspaceImpact`
- `remoteImpact`
- `artifacts`
- `serviceHandlesUsed`
- `secretClassesInvolved`
- `resourceLimits`
- `diffSummary` when applicable
- `targetBranch` or `remote` when applicable

Rules:
- approval payloads should be host-generated
- approval payloads must not include raw secret values
- approval outcomes should be auditable and referenceable by id

## 7. Error Contract and Retry Rules
All tool-facing errors should use a normalized envelope.

Required fields:
- `status`
- `code`
- `message`
- `retryable`
- `tool`
- `command`
- `requestId`
- `nextSuggestedAction`

Recommended error codes:
- `UNKNOWN_TOOL`
- `UNKNOWN_COMMAND`
- `INVALID_INPUT`
- `OUTPUT_SCHEMA_INVALID`
- `CAPABILITY_DENIED`
- `POLICY_BLOCKED`
- `APPROVAL_REQUIRED`
- `ARTIFACT_NOT_FOUND`
- `RUNTIME_TIMEOUT`
- `RUNTIME_FAILED`
- `PROVIDER_UNSUPPORTED`
- `RESOURCE_LIMIT_STORAGE`
- `RESOURCE_LIMIT_MEMORY`
- `RESOURCE_LIMIT_TIMEOUT`
- `RESOURCE_LIMIT_OUTPUT`
- `NETWORK_BLOCKED`
- `NETWORK_QUOTA_EXCEEDED`

Retry rules:
- retry only after changing the command, arguments, approval state, or artifact reference
- do not repeat an identical failing tool call more than once
- after `INVALID_INPUT`, use `tool_describe`
- after `POLICY_BLOCKED` or `CAPABILITY_DENIED`, use `tool_policy_inspect` if available
- after `APPROVAL_REQUIRED`, do not invoke again until approval state changes
- after `OUTPUT_SCHEMA_INVALID` or `RUNTIME_FAILED`, prefer a different tool or escalate to host diagnostics

Best-practice default:
- provider-specific retry quirks should stay in provider adapters
- the agent-facing retry policy should remain provider-neutral and side-effect-aware

## 8. Side-Effect Taxonomy
The canonical side-effect classes are:
- `none`
- `ephemeral`
- `plugin-private-write`
- `external-read`
- `external-write`
- `destructive`

Rules:
- `none` and `ephemeral` are default low-risk classes
- `external-write` and `destructive` should require planning and may require approval
- the side-effect class exposed to the agent must be stable across providers

## 9. Delegated and Pending Execution
Some tools may be delegated to a client-side executor or a privileged host subsystem.

Canonical rule:
- the agent still sees the same search/describe/plan/invoke workflow
- the host may internally translate an invoke into a delegated or pending execution state

If the runtime uses pending execution, the result envelope should make that explicit:
- `status: "pending"`
- `summary`
- `nextSuggestedAction`
- correlation identifiers needed for continuation

This is informed by OpenClaw's client-delegated tool pattern, where delegated tools return a structured pending result instead of executing directly.

## 10. Provider Normalization Rules
The agent-facing contract must remain stable even when providers differ.

Normalization rules:
- all providers must map into the same Oberon command identity model
- all providers must preserve a stable tool call correlation id
- all providers must normalize finish reasons or stop reasons into Oberon tool lifecycle states
- provider-specific schema quirks must be handled before the model receives the tool definition
- provider-specific streaming differences must not change the canonical meaning of search, describe, plan, or invoke

## 11. Streaming Rules
If the active provider supports streaming tool calls:
- tool call identifiers and argument deltas should be preserved incrementally when possible
- the final tool call state must still be representable as one canonical invocation request

If the active provider does not support true incremental tool-call streaming:
- the host may buffer provider output
- the host must still emit a correct final canonical tool call state
- the host should record that degraded streaming behavior in provider compatibility docs

## 12. Acceptance Rules
- `TIC-101`: an agent can discover a new tool without root prompt bloat
- `TIC-102`: an agent can retrieve verbose command guidance only when needed
- `TIC-103`: stateful commands require planning according to side-effect rules
- `TIC-104`: tool success and error results are machine-usable and retryable
- `TIC-105`: delegated or pending execution remains visible as a structured state, not an opaque failure
- `TIC-106`: provider differences do not leak into the agent-facing workflow contract
