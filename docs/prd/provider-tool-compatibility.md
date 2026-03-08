# Oberon4 Provider Tool Compatibility

Version: 0.1
Owner: Oberon4
Status: draft

## 1. Purpose
Define how Oberon4 normalizes tool use across model providers while preserving one canonical agent-facing tool contract.

This document is informed by:
- OpenClaw's tool definition adapter and schema normalization paths
- llmhotspot-v2's tool-call alignment notes and provider integration tests

Primary research inputs:
- `Research/openclaw/src/agents/pi-tool-definition-adapter.ts`
- `Research/openclaw/src/agents/pi-tools.schema.ts`
- `Research/openclaw/src/agents/pi-embedded-runner/tool-split.ts`
- `Research/openclaw/src/agents/pi-embedded-runner/google.ts`
- `Research/llmhotspot-v2/TOOL_CALL_ALIGNMENT.md`
- `Research/llmhotspot-v2/tests/integration/test_hotspot_matrix.py`

## 2. Canonical Oberon Model
Oberon should normalize all providers into one internal model:
- `tool definition`
- `tool call`
- `tool call delta`
- `tool result`
- `finish state`
- `provider diagnostics`

Rules:
- provider adapters translate provider-native request and response formats into the canonical Oberon model
- the rest of the tool runtime should not branch on provider-specific wire formats
- provider quirks must be resolved before tool contracts reach the agent or plugin runtime

Best-practice default:
- Oberon should be canonical internally and adapter-driven at the provider edge rather than provider-native internally

## 3. Canonical Tool Definition Shape
The Oberon canonical tool definition should include:
- `toolName`
- `description`
- `parametersSchema`
- `toolTier`
- `sideEffects`
- `approvalRequired`
- `capabilitiesRequired`

Provider adapters may reshape this when talking to a provider, but the canonical fields above remain authoritative.

## 4. Provider Matrix

### 4.1 OpenAI Responses-style Providers
Expected behavior:
- tool definitions can be represented as function-style tool objects
- tool calls return explicit function-call items with call identifiers
- delegated tools can remain in a pending state and continue on follow-up turns

Compatibility rules:
- preserve tool call ids exactly
- preserve partial argument deltas when available
- normalize final state into the Oberon invoke lifecycle

Notes from OpenClaw:
- Responses-native systems can emit explicit function call items and continue after a client or host provides function call output
- delegated tools may return a structured `pending` result instead of executing immediately

### 4.2 OpenAI Chat Completions-style Providers
Expected behavior:
- tool definitions use `type: "function"` with nested `function.{name,description,parameters}`
- non-streaming tool calls should terminate with `finish_reason: "tool_calls"`
- tool calls should be present in `message.tool_calls`
- streaming tool calls should arrive in `delta.tool_calls` chunks when supported

Compatibility rules:
- root tool schema must be `type: "object"`
- top-level union schemas should be flattened before dispatch
- final tool-call state must be representable even if provider streaming is buffered

Evidence from llmhotspot tests:
- OpenAI-compatible tests expect `finish_reason == "tool_calls"` for tool use
- streaming tests expect `delta.tool_calls`

### 4.3 Anthropic Messages-style Providers
Expected behavior:
- tool definitions use `name`, `description`, and `input_schema`
- tool-use responses terminate with `stop_reason: "tool_use"`
- tool invocations appear as `tool_use` content blocks
- tool results continue as `tool_result` style follow-up content

Compatibility rules:
- preserve tool-use block ids and map them to canonical call ids
- normalize `tool_use` stop reasons into the canonical invoke lifecycle
- preserve mixed text-plus-tool responses when both appear in the same assistant turn

Evidence from llmhotspot tests:
- Anthropic-compatible tests expect `stop_reason == "tool_use"`
- streaming tests expect `content_block_start` events with `tool_use`

### 4.4 Google / Gemini Routed Providers
Expected behavior:
- schemas may pass through an OpenAPI-like parameter surface rather than full JSON Schema support
- unsupported JSON Schema keywords may cause tool registration or execution failures

Compatibility rules:
- clean or scrub unsupported schema keywords before dispatch
- avoid relying on top-level union schemas
- keep order and transcript sanitation rules in the provider adapter, not in plugins

Evidence from OpenClaw:
- Gemini-facing schema paths strip unsupported keywords
- some provider routes require additional transcript or turn-order sanitation

### 4.5 xAI-style Providers
Expected behavior:
- certain validation keywords may be rejected even when valid in fuller JSON Schema dialects

Compatibility rules:
- strip provider-unsupported constraint keywords before dispatch
- preserve as much semantic shape as possible after stripping

Evidence from OpenClaw:
- xAI-specific schema cleaning removes unsupported validation keywords

## 5. Schema Normalization Rules
Schema normalization should occur in the provider adapter layer.

Required normalization rules:
- ensure top-level tool parameter schemas are object-shaped when required by the provider
- flatten `anyOf` or `oneOf` root unions into a single object schema when provider support is weak
- preserve enums and descriptive fields where possible during flattening
- remove provider-unsupported keywords rather than passing through known-invalid schemas

This follows the OpenClaw pattern of normalizing tool schemas once so the rest of the runtime can reuse the same tool definition set.

## 6. Tool Registration Strategy
Oberon should use one normalized custom tool path across providers rather than mixing provider-native built-ins and custom wrappers opportunistically.

Rationale:
- keeps policy enforcement consistent
- keeps sandbox and capability routing consistent
- keeps search/describe/plan/invoke semantics stable

This is consistent with OpenClaw's approach of routing tools through its custom tool adapter layer rather than splitting behavior across provider-native tool systems.

## 7. Finish and Stop Reason Normalization
Provider finish or stop states must map into the canonical Oberon states:
- `assistant_text_complete`
- `tool_call_requested`
- `tool_call_pending`
- `tool_result_required`
- `response_complete`
- `response_incomplete`
- `provider_error`

Mapping examples:
- OpenAI Chat Completions `finish_reason: "tool_calls"` -> `tool_call_requested`
- Anthropic `stop_reason: "tool_use"` -> `tool_call_requested`
- Responses-style delegated `pending` state -> `tool_call_pending`

## 8. Streaming Compatibility Rules
Preferred behavior:
- preserve true incremental tool-call argument deltas
- preserve call ids throughout the stream
- emit a distinct final finish event or state transition

Fallback behavior:
- if a provider adapter must buffer before projection, it must still emit a correct final canonical tool call state
- degraded streaming must be documented as a provider limitation, not silently treated as full parity

Evidence from llmhotspot:
- buffering an entire response into one tool-call chunk is functionally workable but not true streaming parity

## 9. Mixed Content and Tool Use
Provider adapters must preserve assistant text content when a response includes both text and tool calls.

Rules:
- text content must not be dropped simply because a tool call exists
- the canonical model should allow both text content and tool-call content in one response
- provider adapters must preserve ordering where the provider exposes it

## 10. Parallel Tool Calls
If a provider supports parallel tool calls:
- preserve provider call ids
- preserve deterministic ordering in the canonical model
- do not rely on map iteration order

Rules:
- canonical ordering should be stable by provider stream order or explicit sequence index
- provider adapters must not leak nondeterministic ordering into tool invocation planning

## 11. Degraded and Unsupported Modes
If a provider cannot support a canonical feature cleanly:
- mark the feature as degraded or unsupported in provider capability metadata
- fall back to the closest safe behavior
- do not expose unsupported behavior as if it were fully supported

Examples:
- true incremental tool-call streaming unavailable -> support final buffered tool call state only
- unsupported schema keywords -> scrub or reject before registration
- no stable tool call id -> synthesize one and mark provenance as adapter-generated

## 12. Provider Capability Matrix
Each provider adapter should publish capability metadata at least for:
- tool definitions
- non-streaming tool calls
- streaming tool calls
- partial argument deltas
- mixed text plus tool responses
- parallel tool calls
- schema cleaning required
- delegated or pending continuation support

## 13. Acceptance Rules
- `PTC-101`: the same Oberon tool contract can be projected to OpenAI, Anthropic, and other supported providers
- `PTC-102`: provider-specific schema quirks are handled before tool dispatch
- `PTC-103`: finish and stop reasons are normalized into canonical Oberon states
- `PTC-104`: degraded streaming is explicit and testable
- `PTC-105`: provider differences do not change the agent-facing search/describe/plan/invoke workflow
