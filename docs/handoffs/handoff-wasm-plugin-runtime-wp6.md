# Handoff: WASM Plugin Runtime WP6

Phase: `WP6`
Status: complete
Owner: Oberon4

## Completed Scope
- extended the shared provider/transcript contract so daemon-managed chat turns can represent provider-driven tool calls without leaking provider-native wire formats
- upgraded the OpenAI-compatible provider path to the canonical tool-aware contract
- added Anthropic as a runtime-selectable provider crate using the same host tool contract
- replaced the daemon one-shot provider completion path with a canonical iterative tool loop that:
  - injects only `tool_search`, `tool_describe`, `tool_plan`, `tool_invoke`, and `tool_policy_inspect`
  - executes tool calls through `PluginRuntimeHost`
  - appends structured tool result messages back into the transcript
  - preserves tool call ids and normalized finish states
- added in-memory approval pause/resume handling through daemon IPC and resumed provider turns after approve/deny decisions
- routed CLI and TUI chat flows through the daemon-backed tool loop and rendered:
  - tool call deltas
  - tool result and tool error summaries
  - approval-required prompts and resumed completions

## Verification Evidence
- `cargo fmt --check`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-core`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-provider-openai`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-provider-anthropic`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-daemon`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-cli`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-tui`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo clippy --workspace --all-targets -- -D warnings`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test --workspace`

## Remaining Phase
- `WP7` Mode and Release Hardening

## Risks / Notes
- approval pause state is intentionally in-memory only in `WP6`; daemon restart invalidates pending approvals
- Gemini and xAI remain compatibility metadata/testing only and are not runtime-selectable providers in this phase
- the daemon preserves sequential tool-call execution in stable provider order; parallel tool execution remains out of scope for `WP6`
- on hosts with limited disk, the constrained Rust profile plus periodic `cargo clean` is still important for full-workspace verification

## Next Action
- begin `WP7` by hardening operating modes, plugin allowlisting/validation, stricter production budgets and approvals, and the conformance/threat-model-driven regression suite
