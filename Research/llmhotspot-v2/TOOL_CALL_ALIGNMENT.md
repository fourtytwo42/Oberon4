# Tool-Call Alignment: llmhotspot-v2 vs OpenClaw

Comparison of how each system handles tool calls through the Codex endpoint
(`chatgpt.com/backend-api/codex/responses`), and a roadmap to make tool calls
work correctly on the hotspot.

---

## 1. Architecture Overview

### OpenClaw (TypeScript, Responses API native)

OpenClaw speaks the **OpenResponses API** natively. Clients POST to `/v1/responses`
with tools in Responses format. OpenClaw's agent core executes tools internally,
but **client-delegated tools** use a "pending" pattern:

```
Client ──POST /v1/responses──▶ OpenClaw Gateway
                                    │
                              [Agent core runs]
                                    │
                              Model calls tool
                                    │
                              execute() returns
                              { status: "pending" }
                                    │
                              ◀── SSE: response.output_item.added (function_call)
                              ◀── SSE: response.output_item.done  (function_call)
                              ◀── SSE: response.completed (status: "incomplete")
                                    │
Client executes tool locally        │
                                    │
Client ──POST /v1/responses──▶ (input includes function_call_output)
```

Key files:
- `pi-tool-definition-adapter.ts:190` — converts client tools; execute() returns `{status:"pending"}`
- `openresponses-http.ts:580` — non-streaming: emits function_call output item
- `openresponses-http.ts:835` — streaming: emits SSE function_call events
- `open-responses.schema.ts:221` — parses `function_call_output` in follow-up input

### llmhotspot-v2 (Rust, Chat Completions API bridge)

The hotspot accepts **Chat Completions API** requests from clients (e.g. Cursor,
Continue.dev, ChatGPT-compatible apps). In OAuth/locked mode it converts these to
Codex Responses API format, sends them upstream, then projects the Codex SSE
response back to Chat Completions format.

```
Client ──POST /v1/chat/completions──▶ Hotspot
                                         │
                                   [build_codex_payload]
                                   Chat msgs + tools → Responses format
                                         │
                                   ──POST codex/responses──▶ Codex Backend
                                         │
                                   ◀── SSE (Responses API events)
                                         │
                                   [project_chat_stream_from_responses_sse]
                                   Responses SSE → Chat Completions format
                                         │
                                   ◀── Chat Completions JSON/SSE to client
```

Key files:
- `lib.rs:2799` (`build_codex_payload`) — converts Chat messages/tools → Codex input
- `lib.rs:2731` (`map_chat_tools_to_responses`) — maps tool definitions
- `lib.rs:2981` (`build_codex_request_headers`) — sets Codex headers
- `lib.rs:2140` (`project_chat_stream_from_responses_sse`) — projects Codex SSE → Chat format
- `lib.rs:1926-1981` — dispatch: non-streaming collects+projects, streaming also collects+projects

---

## 2. Side-by-Side Comparison

### 2.1 Tool Definition Format

| Aspect | OpenClaw | llmhotspot-v2 |
|--------|----------|---------------|
| Client sends | `{ type:"function", function:{ name, description, parameters } }` (Responses format) | `{ type:"function", function:{ name, description, parameters } }` (Chat format) |
| Sent to Codex as | Same as client (already Responses format) | `{ type:"function", name, description, parameters }` (mapped via `map_chat_tools_to_responses`) |
| `tool_choice` | Passed through | Mapped via `map_chat_tool_choice_to_responses` |
| `parallel_tool_calls` | Not explicitly set | Hardcoded `true` |

**Status:** Tool definitions map correctly. No issues here.

### 2.2 Conversation History with Tool Calls

| Aspect | OpenClaw | llmhotspot-v2 |
|--------|----------|---------------|
| Previous tool calls | Client sends `{ type:"function_call", call_id, name, arguments }` in `input[]` | Converts assistant `tool_calls[]` → `{ type:"function_call", call_id, name, arguments }` in `input[]` |
| Previous tool results | Client sends `{ type:"function_call_output", call_id, output }` in `input[]` | Converts `role:"tool"` msgs → `{ type:"function_call_output", call_id, output }` in `input[]` |

**Status:** Conversion looks correct. Both produce the same Codex input format.

### 2.3 Codex Response Projection (THE CRITICAL DIFFERENCE)

This is where the systems diverge significantly.

#### OpenClaw: Native Responses API output

OpenClaw emits Responses API events directly to the client. When a tool call
happens, it emits:

```
SSE: response.output_item.added   → { type:"function_call", id, call_id, name, arguments }
SSE: response.output_item.done    → same item with status:"completed"
SSE: response.completed           → { status:"incomplete" }   ← response is "incomplete"
```

The client (which speaks Responses API) knows to:
1. Execute the tool
2. Send the result back as `function_call_output` in the next request

#### llmhotspot-v2: Responses → Chat Completions projection

The hotspot must convert Codex Responses SSE into Chat Completions format.
This happens in `project_chat_stream_from_responses_sse()` (lib.rs:2140).

**What it handles:**
- `response.output_item.added/done` with `type:"function_call"` → collects call_id, name, arguments
- `response.function_call_arguments.delta` → appends to arguments
- `response.function_call_arguments.done` → sets final arguments
- Builds Chat Completions `tool_calls[]` array with `{ index, id, type:"function", function:{ name, arguments } }`
- Sets `finish_reason: "tool_calls"` when tool_calls present

**What it outputs (non-streaming, lib.rs:1926):**
```json
{
  "id": "chatcmpl-llmhotspot-codex",
  "object": "chat.completion",
  "choices": [{
    "index": 0,
    "finish_reason": "tool_calls",
    "message": {
      "role": "assistant",
      "content": null,
      "tool_calls": [{
        "index": 0,
        "id": "call_xxx",
        "type": "function",
        "function": { "name": "exec_command", "arguments": "{...}" }
      }]
    }
  }]
}
```

**What it outputs (streaming, lib.rs:2083):**
```
data: {"id":"chatcmpl-llmhotspot-codex","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[...]},"finish_reason":"tool_calls"}]}

data: [DONE]
```

---

## 3. Identified Problems

### Problem 1: Streaming emits a SINGLE chunk (not incremental)

**Location:** `chat_completion_sse_from_responses()` at lib.rs:2083

The streaming path collects the **entire** Codex SSE response into memory,
projects it, then emits a single `data:` chunk followed by `[DONE]`. This means:

- The client gets **no incremental streaming** for tool call arguments
- For large tool arguments, the client blocks until the full response is buffered
- Some clients (Cursor, Continue.dev) expect incremental `tool_calls` deltas
  in the standard Chat Completions streaming format

**Expected Chat Completions streaming format for tool calls:**
```
data: {"choices":[{"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"exec_command","arguments":""}}]},"finish_reason":null}]}

data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"cm"}}]},"finish_reason":null}]}

data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"d\":\"ls"}}]},"finish_reason":null}]}

data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"}"}}]},"finish_reason":null}]}

data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}

data: [DONE]
```

Each chunk only contains the **delta** (new characters of the arguments string),
not the full tool call. The hotspot currently sends everything at once.

### Problem 2: Streaming path for `codex_mode && stream` bypasses projection entirely

**Location:** lib.rs:1973-1981

When `stream=true` in codex mode, the code does:
```rust
forward_openai_sse(&cfg, &session.access_token, &payload, Some(&request_headers))
    .await
    .map(|v| RoutedResponse::Sse(v.response))
```

This returns raw Codex SSE which IS then intercepted at lib.rs:940-951:
```rust
Ok(RoutedResponse::Sse(resp)) => {
    chat_completion_sse_from_responses(resp).await
}
```

So projection does happen, but it's the single-chunk buffer-all approach described
in Problem 1. The client receives Codex Responses events reformatted as one big
Chat Completions chunk, not true incremental streaming.

### Problem 3: No `response.incomplete` / stop reason mapping

When Codex returns a response with `status: "incomplete"` (meaning the model
wants the client to execute a tool and continue), the hotspot correctly sets
`finish_reason: "tool_calls"`. However, it doesn't preserve any signal about
whether the response is truly done or expecting continuation.

In Chat Completions this is handled purely by `finish_reason: "tool_calls"` vs
`"stop"`, and the hotspot does get this right. **This is not a bug** — the
mapping is correct.

### Problem 4: Assistant message content alongside tool calls may be lost

When Codex returns both text content AND tool calls in the same response, the
non-streaming path (lib.rs:1945-1955) handles this correctly:

```rust
let mut message = json!({
    "role": "assistant",
    "content": if projected.text.is_empty() { Value::Null } else { Value::String(projected.text) }
});
if !projected.tool_calls.is_empty() {
    message["tool_calls"] = Value::Array(projected.tool_calls);
}
```

However, the streaming path (lib.rs:2088-2094) also handles both:
```rust
if !projected.text.is_empty() {
    delta.insert("content".to_string(), Value::String(projected.text));
}
if !projected.tool_calls.is_empty() {
    delta.insert("tool_calls".to_string(), Value::Array(projected.tool_calls));
}
```

**This is fine for the buffered approach**, but when we move to true incremental
streaming, text deltas and tool call deltas need to be interleaved correctly.

### Problem 5: Multiple parallel tool calls — ordering not guaranteed

The projection function uses a HashMap keyed by item_id. When Codex returns
multiple parallel tool calls, they are collected by item_id but the final
iteration order depends on HashMap ordering (not insertion order).

At lib.rs:2315-2326, tool calls are filtered and re-indexed:
```rust
for tc in tool_calls.into_iter().filter(|tc| tc.name.is_some()) {
    projected_tool_calls.push(json!({
        "index": tc.index,
        ...
    }));
}
```

The `tool_calls` Vec preserves insertion order, so this is actually fine.
The HashMap is only used for lookup; the Vec maintains order. **Not a bug.**

### Problem 6: `text` content duplication risk

Both `response.output_text.delta` and `response.output_item.done` (type=message)
can contribute text to the `content` string. If Codex sends both the streaming
deltas AND a final done event with full text, the content gets doubled.

The mitigation at lib.rs:2309-2313 only uses `done_text` if `content` is empty:
```rust
if content.is_empty() {
    if let Some(done) = done_text {
        content = done;
    }
}
```

But `response.output_item.done` with `type:"message"` at line 2200-2215
directly appends to `content` without checking — this path could double-add
text that was already accumulated from deltas. **Potential bug.**

---

## 4. What OpenClaw Does That We Don't Need

Because OpenClaw is a native Responses API server with its own agent runtime,
several of its patterns don't apply to the hotspot:

| OpenClaw Feature | Why we don't need it |
|------------------|---------------------|
| Client tool delegation (`status:"pending"`) | We're a proxy, not an agent runtime. Codex handles tool decisions. |
| `before_tool_call` hooks | We pass tools through to Codex; we don't execute them. |
| Agent session management | Codex manages its own sessions. |
| `onClientToolCall` callbacks | No internal tool execution loop. |
| Tool blocking/policy | Not our responsibility as a proxy. |

---

## 5. Roadmap: Making Tool Calls Work

### Phase 1: Fix the streaming projection (HIGH PRIORITY)

**Goal:** Emit incremental Chat Completions SSE chunks as Codex SSE events arrive,
instead of buffering the entire response.

**What to change in `lib.rs`:**

Replace the current `chat_completion_sse_from_responses()` (which buffers everything)
with a true streaming transformer that reads Codex SSE events one at a time and
immediately emits the corresponding Chat Completions chunk.

**Mapping table (Codex SSE → Chat Completions SSE):**

| Codex Event | Chat Completions Chunk |
|-------------|----------------------|
| `response.output_item.added` (type=function_call) | `delta: { tool_calls: [{ index, id: call_id, type:"function", function: { name, arguments:"" } }] }`, `finish_reason: null` |
| `response.function_call_arguments.delta` | `delta: { tool_calls: [{ index, function: { arguments: "<delta>" } }] }`, `finish_reason: null` |
| `response.function_call_arguments.done` | (optional, can be skipped if deltas were emitted) |
| `response.output_text.delta` | `delta: { content: "<delta>" }`, `finish_reason: null` |
| `response.output_item.done` (type=message) | (skip if deltas were already emitted) |
| `response.completed` | `delta: {}`, `finish_reason: "tool_calls"` or `"stop"` |
| `[DONE]` | `data: [DONE]` |

**Implementation sketch:**

```rust
async fn stream_codex_as_chat_chunks(upstream: reqwest::Response) -> axum::response::Response {
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(64);
    let body = Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx));

    tokio::spawn(async move {
        let mut stream = upstream.bytes_stream();
        let mut buffer = String::new();
        let mut model = String::from("gpt-5.1");
        let mut tool_index_map: HashMap<String, usize> = HashMap::new();
        let mut next_tool_index: usize = 0;
        let mut has_tool_calls = false;

        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(c) => String::from_utf8_lossy(&c).to_string(),
                Err(_) => break,
            };
            buffer.push_str(&chunk);

            while let Some(pos) = buffer.find("\n") {
                let line = buffer[..pos].to_string();
                buffer = buffer[pos + 1..].to_string();

                let Some(data) = line.strip_prefix("data: ") else { continue };
                if data.trim() == "[DONE]" {
                    // Emit final chunk with finish_reason
                    let finish = if has_tool_calls { "tool_calls" } else { "stop" };
                    let chunk = format_chat_chunk(&model, json!({}), Some(finish));
                    let _ = tx.send(format!("data: {chunk}\n\ndata: [DONE]\n\n")).await;
                    return;
                }

                let Ok(event) = serde_json::from_str::<Value>(data) else { continue };
                match event.get("type").and_then(Value::as_str) {
                    Some("response.created") => { /* extract model */ }
                    Some("response.output_text.delta") => {
                        if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                            let chunk = format_chat_chunk(&model, json!({"content": delta}), None);
                            let _ = tx.send(format!("data: {chunk}\n\n")).await;
                        }
                    }
                    Some("response.output_item.added") => {
                        if let Some(item) = event.get("item") {
                            if item.get("type").and_then(Value::as_str) == Some("function_call") {
                                let call_id = item.get("call_id").and_then(Value::as_str).unwrap_or("");
                                let name = item.get("name").and_then(Value::as_str).unwrap_or("");
                                let item_id = item.get("id").and_then(Value::as_str).unwrap_or("");
                                let idx = next_tool_index;
                                tool_index_map.insert(item_id.to_string(), idx);
                                next_tool_index += 1;
                                has_tool_calls = true;
                                let chunk = format_chat_chunk(&model, json!({
                                    "tool_calls": [{ "index": idx, "id": call_id, "type": "function",
                                        "function": { "name": name, "arguments": "" } }]
                                }), None);
                                let _ = tx.send(format!("data: {chunk}\n\n")).await;
                            }
                        }
                    }
                    Some("response.function_call_arguments.delta") => {
                        let item_id = event.get("item_id").and_then(Value::as_str).unwrap_or("");
                        if let Some(&idx) = tool_index_map.get(item_id) {
                            if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                                let chunk = format_chat_chunk(&model, json!({
                                    "tool_calls": [{ "index": idx,
                                        "function": { "arguments": delta } }]
                                }), None);
                                let _ = tx.send(format!("data: {chunk}\n\n")).await;
                            }
                        }
                    }
                    _ => {} // skip other events
                }
            }
        }
    });

    // Build SSE response
    let mut resp = (StatusCode::OK, body).into_response();
    resp.headers_mut().insert(CONTENT_TYPE, "text/event-stream".parse().unwrap());
    resp.headers_mut().insert(CACHE_CONTROL, "no-cache".parse().unwrap());
    resp
}
```

### Phase 2: Fix text content duplication (MEDIUM PRIORITY)

**Goal:** Prevent double-adding text when both streaming deltas and done events
contain the same content.

**What to change:**

In `project_chat_stream_from_responses_sse()` at lib.rs:2200, when processing
`response.output_item.done` with `type:"message"`, only extract text if no
streaming deltas were received for this item:

```rust
// Track whether we've seen any output_text.delta events
let mut saw_text_deltas = false;

// In the response.output_text.delta handler:
saw_text_deltas = true;
content.push_str(delta);

// In the response.output_item.done handler for type:"message":
if !saw_text_deltas {
    // Only use done text if we didn't get streaming deltas
    for part in parts { ... }
}
```

This also needs to be carried into the Phase 1 streaming implementation.

### Phase 3: Verify round-trip tool call flow end-to-end (HIGH PRIORITY)

**Goal:** Confirm the full cycle works:
1. Client sends `tools[]` in Chat Completions format
2. Hotspot converts to Codex format, sends upstream
3. Codex returns `function_call` in SSE
4. Hotspot projects to Chat Completions `tool_calls`
5. Client sends tool result as `role:"tool"` message
6. Hotspot converts to `function_call_output` and sends to Codex
7. Codex returns continuation

**Test plan:**

Create an integration test that simulates the full round-trip with mock Codex
SSE responses. The existing test at
`tests/codex_tool_call_flow_tests.rs` covers payload building and projection
separately but doesn't test the full HTTP round-trip.

Test cases to add:
- Single tool call → result → continuation
- Multiple parallel tool calls → results → continuation
- Tool call with streaming text before/after
- Tool call with empty arguments
- Tool call with large arguments (verify streaming doesn't truncate)

### Phase 4: Add proper streaming for non-tool content (LOW PRIORITY)

Currently, even text-only Codex responses are buffered before projection.
The Phase 1 streaming fix will also improve text streaming latency.

### Phase 5: Handle edge cases (LOW PRIORITY)

- `response.failed` events during tool call streaming (currently only checked
  in `collect_codex_sse`, needs to work in streaming mode too)
- Timeout handling — current 20s timeout may be too short for complex tool calls
- Model name propagation — use Codex's reported model name, not the requested one

---

## 6. Quick Reference: Event Flow Diagrams

### Current (broken for many clients):

```
Codex SSE events arrive over time:
  t=0   response.created
  t=1   response.output_item.added (function_call)
  t=2   response.function_call_arguments.delta  "{"
  t=3   response.function_call_arguments.delta  "\"cmd\":"
  t=4   response.function_call_arguments.delta  "\"ls\"}"
  t=5   response.function_call_arguments.done
  t=6   response.completed
  t=7   [DONE]

What client receives (ALL AT ONCE at t=7):
  data: {"choices":[{"delta":{"role":"assistant","tool_calls":[{full tool call}]},"finish_reason":"tool_calls"}]}
  data: [DONE]
```

### Target (incremental streaming):

```
Codex SSE events arrive:     Client receives Chat chunks:
  t=0 response.created    →  (nothing yet)
  t=1 output_item.added   →  data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"exec_command","arguments":""}}]},"finish_reason":null}]}
  t=2 args.delta "{"      →  data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{"}}]},"finish_reason":null}]}
  t=3 args.delta "\"cmd\":" → data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"cmd\":"}}]},"finish_reason":null}]}
  t=4 args.delta "\"ls\"}" → data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"ls\"}"}}]},"finish_reason":null}]}
  t=5 args.done           →  (skip, deltas already emitted)
  t=6 response.completed  →  data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}
  t=7 [DONE]              →  data: [DONE]
```

---

## 7. Files to Modify

| File | Change | Phase |
|------|--------|-------|
| `crates/llm-control-api/src/lib.rs` | Replace `chat_completion_sse_from_responses()` with true streaming transformer | Phase 1 |
| `crates/llm-control-api/src/lib.rs` | Fix text duplication in `project_chat_stream_from_responses_sse()` | Phase 2 |
| `crates/llm-control-api/src/lib.rs` | Add `response.failed` handling in streaming path | Phase 5 |
| `crates/llm-control-api/tests/codex_tool_call_flow_tests.rs` | Add full round-trip integration tests | Phase 3 |
| `crates/llm-gateway/src/upstream.rs` | Make timeout configurable | Phase 5 |

---

## 8. Summary

The hotspot's tool-call **data conversion** (Chat → Codex and back) is
fundamentally correct. The tool definitions, conversation history with
tool_calls/tool results, and the projection of Codex function_call events to
Chat Completions tool_calls all map the right fields.

**The primary issue is streaming delivery.** The hotspot buffers the entire
Codex response before emitting a single Chat Completions chunk. Many clients
(Cursor, Continue.dev, etc.) expect incremental streaming of tool call
arguments via multiple SSE chunks. The single-chunk approach may cause:

1. **Timeouts** — client may time out waiting for the first chunk
2. **Parsing failures** — some clients expect tool_calls to arrive incrementally
3. **UX degradation** — no progressive display of tool call activity

Phase 1 (true incremental streaming) is the critical fix. Phases 2-5 are
correctness and robustness improvements.

---

## 9. Compatibility Trace Notes (2026-02-14)

To diagnose IDE-specific tool-call failures, llmhotspot-v2 now has a dedicated
compatibility trace channel in `crates/llm-control-api/src/lib.rs`.

Enable with env vars:
- `LLMHOTSPOT_TRACE_IO=1`
- `LLMHOTSPOT_TRACE_COMPAT=1`

Key trace events:
- `compat_request_profile`
  - request format classification (`chat_completions` / `responses` / `hybrid`)
  - inbound vs forwarded tool schema profile (wrapped function / flat function / custom)
  - inbound vs forwarded `tool_choice`
  - upstream path and attempt metadata
- `compat_stream_summary`
  - Responses SSE event counts and head of event sequence
  - parse error count for SSE JSON frames
  - tool announcement counts and suppressed duplicates
  - argument delta emitted/suppressed counts
  - arguments done emitted/suppressed counts
  - termination reason (`[DONE]`, `response.failed`, `stream_eof_no_done`)

Quick usage:
1. restart API with env loaded (`bash scripts/pm2-restart-v2-api.sh`)
2. reproduce the failing IDE request
3. inspect logs:
   - `pm2 logs llmhotspot-v2-api --lines 400 --nostream 2>&1 | rg "compat_request_profile|compat_stream_summary"`

This is intended as a stable "first look" signal before digging into full raw
`relay_inbound`/`relay_outbound` payloads.
