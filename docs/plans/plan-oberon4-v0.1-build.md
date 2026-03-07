# Plan: Oberon4 v0.1 Build

Version: 0.1
Status: active
Owner: Oberon4
WorkflowType: feature-development
WorkflowMode: incremental-gates
TaskState: done
Stack: rust
Risk: high-risk

## 1. Goal
Build Oberon4 as a modular Rust-first local daemon with:
- daemon runtime
- local CLI
- terminal TUI chat
- OpenAI-compatible custom endpoint support
- OpenAI and Anthropic OAuth account flows
- strong automated testing before human testing

## 2. Canonical Inputs
- `docs/prd/PRD-oberon4.md`
- `docs/prd/architecture.md`
- `docs/prd/test-strategy.md`
- `AGENTS.md`
- `Research/llmhotspot`
- `Research/openclaw`

## 3. Scope
### In Scope
- Rust workspace bootstrap
- crate boundaries for core, daemon, CLI, TUI, provider, auth, tray
- config, secret storage abstractions, and local state
- OpenAI-compatible chat transport
- OpenAI and Anthropic OAuth account handling
- local IPC between daemon and clients
- CLI and TUI command surfaces from the PRD
- automated test harness and gating

### Out of Scope
- plugin marketplace
- remote multi-user hosting
- mobile clients
- background autonomous orchestration

## 4. Execution Rules
- Complete phases in order. Do not start a dependent phase early.
- A checkbox may be marked complete only after its verification gate passes.
- If a phase reveals a new requirement, update the PRD and this plan before implementation continues.
- Human testing starts only after all automated gates for the relevant phase are green.
- If a task affects multiple crates, add integration coverage before marking it done.

## 5. Progress Tracker

### Phase 0: Foundations
- [x] `P0.1` Create Rust workspace and crate skeletons:
  - `oberon4-core`
  - `oberon4-daemon`
  - `oberon4-cli`
  - `oberon4-tui`
  - `oberon4-provider-openai`
  - `oberon4-auth`
  - `oberon4-tray`
- [x] `P0.2` Establish shared lint, format, and workspace test commands.
- [x] `P0.3` Define shared error model, request ID contract, and result envelope.
- [x] `P0.4` Define shared configuration schema and config file loading rules.
- [x] `P0.5` Define secret storage abstraction and local state storage abstraction.
- [x] `P0.6` Add baseline unit tests for config parsing, error handling, and state abstractions.

Verification gate `EG-P0`:
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

### Phase 1: Core Domain and IPC
- [x] `P1.1` Define domain models for sessions, messages, accounts, providers, daemon status, and saved chats.
- [x] `P1.2` Define service traits:
  - `ChatProvider`
  - `OAuthProvider`
  - `ConfigStore`
  - `SecretStore`
  - `SessionStore`
- [x] `P1.3` Define local IPC command and event contracts shared by daemon, CLI, and TUI.
- [x] `P1.4` Implement mock/fake providers and stores for deterministic tests.
- [x] `P1.5` Add unit and contract tests for domain types and IPC schema.

Dependencies:
- depends on `EG-P0`

Verification gate `EG-P1`:
- `cargo test -p oberon4-core`
- `cargo test -p oberon4-daemon ipc`
- schema/contract tests for IPC request and event envelopes

### Phase 2: Daemon Runtime
- [x] `P2.1` Implement daemon lifecycle abstraction (`start`, `stop`, `status`).
- [x] `P2.2` Implement local IPC server and request dispatch.
- [x] `P2.3` Implement config load/save path in daemon.
- [x] `P2.4` Implement structured logging with request IDs.
- [x] `P2.5` Implement health/status reporting contract.
- [x] `P2.6` Add integration tests for daemon startup, shutdown, and status flow.

Dependencies:
- depends on `EG-P1`

Verification gate `EG-P2`:
- `cargo test -p oberon4-daemon`
- daemon lifecycle integration tests pass
- no panic on malformed local IPC request

### Phase 3: OpenAI-Compatible Provider
- [x] `P3.1` Implement provider client with configurable base URL.
- [x] `P3.2` Implement `/v1/chat/completions` request/response translation.
- [x] `P3.3` Implement timeout and retry policy in provider layer.
- [x] `P3.4` Implement endpoint health/test request path.
- [x] `P3.5` Add contract tests against mocked OpenAI-compatible responses.
- [x] `P3.6` Add failure-path tests for malformed responses, timeout, and auth failure.

Dependencies:
- depends on `EG-P2`

Verification gate `EG-P3`:
- `cargo test -p oberon4-provider-openai`
- provider contract tests pass
- timeout/retry behavior covered by automated tests

### Phase 4: OAuth and Account System
- [x] `P4.1` Extract and adapt llmhotspot OAuth primitives into `oberon4-auth`.
- [x] `P4.2` Implement OpenAI OAuth provider adapter.
- [x] `P4.3` Implement Anthropic OAuth provider adapter.
- [x] `P4.4` Implement local callback handling with state/PKCE validation.
- [x] `P4.5` Implement secure token persistence through secret storage abstraction.
- [x] `P4.6` Implement account lifecycle operations:
  - connect
  - list
  - activate
  - disable
  - disconnect
- [x] `P4.7` Add integration tests for OpenAI OAuth flow.
- [x] `P4.8` Add integration tests for Anthropic OAuth flow.
- [x] `P4.9` Add security tests for callback mismatch, expired state, and secret redaction.

Dependencies:
- depends on `EG-P2`

Verification gate `EG-P4`:
- `cargo test -p oberon4-auth`
- OAuth integration tests for both providers pass
- security-focused callback/state tests pass

### Phase 5: CLI Surface
- [x] `P5.1` Implement CLI parser and command routing.
- [x] `P5.2` Implement:
  - `daemon start`
  - `daemon stop`
  - `daemon status`
- [x] `P5.3` Implement:
  - `config show`
  - `config set <key> <value>`
  - `endpoint test`
- [x] `P5.4` Implement:
  - `chat --message <text>`
  - `logs tail`
  - `doctor`
- [x] `P5.5` Implement account commands:
  - `account connect --provider <openai|anthropic>`
  - `account list`
  - `account activate --provider <openai|anthropic> --account <id>`
  - `account disable --provider <openai|anthropic> --account <id>`
  - `account disconnect --provider <openai|anthropic> --account <id>`
- [x] `P5.6` Add parser, exit-code, and daemon-client integration tests.

Dependencies:
- depends on `EG-P2`
- depends on `EG-P3`
- depends on `EG-P4`

Verification gate `EG-P5`:
- `cargo test -p oberon4-cli`
- command parser tests pass
- CLI integration smoke tests pass

### Phase 6: TUI Surface
- [x] `P6.1` Implement TUI shell and daemon client connection.
- [x] `P6.2` Implement transcript rendering and prompt submission flow.
- [x] `P6.3` Implement TUI commands:
  - `/help`
  - `/quit`
  - `/status`
  - `/config`
- [x] `P6.4` Implement TUI commands:
  - `/model <name>`
  - `/endpoint <url>`
  - `/system <text>`
- [x] `P6.5` Implement TUI commands:
  - `/clear`
  - `/save <name>`
  - `/load <name>`
- [x] `P6.6` Add command parsing, state transition, and chat loop tests.
- [x] `P6.7` Add sanitized error rendering tests.

Dependencies:
- depends on `EG-P2`
- depends on `EG-P3`
- depends on `EG-P4`
- depends on `EG-P5`

Verification gate `EG-P6`:
- `cargo test -p oberon4-tui`
- TUI command/state tests pass
- transcript/chat loop integration tests pass

### Phase 7: Tray Contract
- [x] `P7.1` Implement tray abstraction and unsupported-platform fallback.
- [x] `P7.2` Implement tray actions:
  - open UI
  - show status
  - restart
  - quit
- [x] `P7.3` Add adapter tests and unsupported-platform behavior tests.

Dependencies:
- depends on `EG-P2`

Verification gate `EG-P7`:
- `cargo test -p oberon4-tray`
- unsupported-platform path returns explicit status

### Phase 8: End-to-End and Human-Test Readiness
- [x] `P8.1` Add workspace-wide integration harness for daemon + provider + CLI.
- [x] `P8.2` Add end-to-end test for endpoint configuration and connectivity.
- [x] `P8.3` Add end-to-end test for OAuth account connect and account switching.
- [x] `P8.4` Add end-to-end test for one-shot chat and TUI chat path.
- [x] `P8.5` Add end-to-end test for session save and load.
- [x] `P8.6` Add release-readiness checklist and human test script.
- [x] `P8.7` Run full workspace verification and capture evidence.

Dependencies:
- depends on `EG-P3`
- depends on `EG-P4`
- depends on `EG-P5`
- depends on `EG-P6`
- depends on `EG-P7`

Verification gate `EG-P8`:
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- end-to-end smoke suite passes
- human test checklist is ready

## 6. Human Testing Entry Criteria
Human testing may begin only when:
- [x] all phase gates through `EG-P8` are green
- [x] all PRD acceptance criteria are mapped to implemented work
- [x] no open stop-ship defects remain
- [x] operator test script exists for daemon, CLI, TUI, and OAuth flows

## 7. Requirement Mapping
- `FR-001`: `P1`, `P2`, `P8`
- `FR-002`: `P6`, `P8`
- `FR-003`: `P3`, `P5`, `P6`, `P8`
- `FR-004`: `P0`, `P1`
- `FR-005`: `P7`
- `FR-006`: `P0` through `P8`
- `FR-007`: `P4`, `P5`, `P8`
- `FR-008`: `P5`
- `FR-009`: `P6`

## 8. Progress Update Rules
- When a task is completed, change `[ ]` to `[x]` in this file in the same change that lands the work.
- Do not mark a task complete if implementation exists but verification is still missing.
- If a task is blocked, append `- blocked: <reason>` directly under the task.
- If scope changes, update this plan before continuing implementation.

## 9. Evidence to Capture Per Phase
- commands run
- tests executed
- pass/fail results
- files changed
- remaining risks
- next step
