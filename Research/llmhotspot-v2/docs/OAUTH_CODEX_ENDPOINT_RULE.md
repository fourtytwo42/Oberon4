# OAuth Codex Endpoint Rule (Locked)

## Non-negotiable rule

For OpenAI ChatGPT OAuth traffic, this project uses the ChatGPT Codex backend contract, not the standard OpenAI public API contract.

Do not switch OAuth upstream calls to `https://api.openai.com/v1/chat/completions` or `https://api.openai.com/v1/responses` unless this rule is intentionally revised.

## Required OAuth upstream target

- Base URL default: `https://chatgpt.com/backend-api`
- Path default: `/codex/responses`

Runtime env knobs (for controlled overrides only):

- `LLMHOTSPOT_CODEX_BASE_URL`
- `LLMHOTSPOT_CODEX_RESPONSES_PATH`

## Required OAuth request shape and headers

When `LLMHOTSPOT_OAUTH_UPSTREAM_LOCKED=1` (default), OAuth requests must follow Codex semantics:

- Build Codex-style payload (`input`, `instructions`, tool mapping).
- Send OAuth/Codex headers:
  - `chatgpt-account-id` (when available from token claims)
  - `openai-beta: responses=experimental`
  - `originator: pi`
  - `session_id`
  - plus auth/content-type/accept/user-agent

## Lock behavior

- `LLMHOTSPOT_OAUTH_UPSTREAM_LOCKED=1` is the safe default.
- In locked mode, OAuth traffic is pinned to Codex transport behavior.
- Set `LLMHOTSPOT_OAUTH_UPSTREAM_LOCKED=0` only for explicit debugging/migration work.

## Why this exists

The external hotspot reference implementation uses a non-standard OAuth transport contract for ChatGPT Codex. Reverting to normal OpenAI endpoint behavior can silently break OAuth-backed chat flows.

## CI guardrails

This repository includes automated checks that lock default Codex OAuth transport values:

- `crates/llm-gateway/tests/upstream_tests.rs` (`codex_oauth_defaults_are_locked`)
- `crates/llm-control-api/tests/oauth_codex_lock_tests.rs`

## Assistant Text Safe Mode Compatibility Contract

To prevent protocol/tool envelope leakage into user-visible assistant text, `llm-control-api` applies text sanitization only at assistant text projection boundaries.

- Env flag: `LLMHOTSPOT_SAFE_TEXT_MODE`
- Default: enabled
- Explicit disable values: `0`, `false`, `off`, `no` (case-insensitive)

Sanitization scope:

- Filters clearly protocol-like lines from projected assistant text (`message.content` and streaming `delta.content` projection paths).
- Does not mutate structured tool-call objects (`tool_calls`, function names, function arguments, finish reasons).
- Does not mutate inbound tool definitions.

Rollback:

- For emergency rollback, set `LLMHOTSPOT_SAFE_TEXT_MODE=off` to bypass assistant text sanitization while preserving all existing transport/tool-call behavior.
