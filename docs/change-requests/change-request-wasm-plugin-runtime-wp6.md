# Change Request: WASM Plugin Runtime WP6

Version: 0.1
Status: approved-for-implementation
Owner: Oberon4

## Current State
- `WP5` shipped the runtime, brokers, and first built-in `workspace`, `git`, and `github` command package, but provider turns still used a one-shot assistant call path.
- The daemon, CLI, and TUI did not yet run the canonical host tool loop:
  - `tool_search`
  - `tool_describe`
  - `tool_plan`
  - `tool_invoke`
  - `tool_policy_inspect`
- Approval-required tool actions could not pause and resume the same streamed assistant turn.
- Anthropic support was not available as a runtime-selectable provider in the host chat path.

## Proposed Delta
- extend the shared provider/transcript contract for tool-aware turns:
  - `ChatRole::Tool`
  - `tool_call_id`
  - canonical provider finish states
  - provider capability metadata
  - tool-aware provider trait
- upgrade the OpenAI-compatible adapter to the canonical tool-aware contract
- add a new Anthropic provider adapter crate using the same canonical contract
- replace the daemon one-shot provider flow with a canonical iterative tool loop that:
  - injects only the five host-native tools
  - executes tool calls through `PluginRuntimeHost`
  - appends host-generated tool result messages
  - preserves provider tool call ids
- add in-memory approval pause/resume support through daemon IPC:
  - `ResolveApproval`
  - `ApprovalRequired`
  - `ToolResult`
  - `ToolError`
  - `ChatPaused`
- route CLI and TUI chat UX through the daemon-backed tool loop with approval resume support

## Non-Goals
- no new raw provider-native tool surface exposed to agents
- no Gemini or xAI runtime-selectable providers in this phase
- no persistent pending-approval storage across daemon restarts
- no new workspace/git/github commands beyond the already-shipped built-ins
- no raw network, raw filesystem, or raw secret interfaces

## Compatibility Impact
- backward-compatible at the top-level tool contract:
  - the model still sees only `tool_search`, `tool_describe`, `tool_plan`, `tool_invoke`, and `tool_policy_inspect`
- daemon/chat transcript persistence now supports tool messages and approval-resume state
- runtime-selectable provider support expands to:
  - `open_ai_compatible`
  - `anthropic`
- Gemini and xAI remain compatibility metadata/testing only

## Migration
- existing saved sessions continue to load because missing `tool_call_id` fields are tolerated
- existing daemon/CLI/TUI chat paths are migrated onto the tool-aware provider trait
- provider selection now uses `endpoint.provider` in shared config for runtime routing
- no schema migration is required for plugin manifests or runtime tool definitions

## Rollback
1. revert the `WP6` implementation commit
2. revert the `WP6` governance/checkpoint commit if split
3. rerun:
   - `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-daemon`
   - `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-cli`
   - `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-tui`
4. confirm:
   - daemon chat reverts to non-tool-loop behavior
   - approval pause/resume IPC events are absent
   - Anthropic is no longer runtime-selectable

## Verification Plan
- `cargo fmt --check`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-core`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-provider-openai`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-provider-anthropic`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-daemon`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-cli`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-tui`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo clippy --workspace --all-targets -- -D warnings`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test --workspace`
- `PATH="$HOME/.cargo/bin:$PATH" bash scripts/oberon4-security-gate.sh`
- `bash scripts/policy-drift-audit.sh`
- `bash scripts/exception-debt-report.sh`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' bash scripts/local-ci-gate.sh`
