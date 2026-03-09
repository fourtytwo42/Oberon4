# Task Brief: WASM Plugin Runtime WP6

## Goal
Integrate the host-managed WASM/WASI plugin runtime into provider turns and the daemon/CLI/TUI chat flows so agents can execute the canonical `tool_search`, `tool_describe`, `tool_plan`, `tool_invoke`, and `tool_policy_inspect` loop end-to-end with approval pause/resume support.

## In Scope
- extend the shared provider/transcript contract for tool-aware turns
- upgrade the OpenAI-compatible provider adapter to the canonical tool-aware interface
- add an Anthropic provider adapter crate on the same interface
- replace the daemon one-shot provider call path with an iterative host tool loop
- add approval pause/resume IPC and daemon state
- route CLI and TUI chat UX through the daemon-backed tool loop
- add WP6 tests and governance artifacts

## Out of Scope
- production-mode hardening and release gates from `WP7`
- Gemini or xAI as runtime-selectable providers
- persistent pending-approval state across daemon restarts
- new raw provider-native tool surfaces
- new raw network, raw secret, or raw filesystem interfaces

## Hard Constraints
- inject only the five host-native tools into provider turns
- preserve provider tool call ids exactly
- keep provider-specific schema quirks adapter-scoped
- keep approval-required tool actions paused and resumable, not terminally failed
- ensure no raw secret values or host paths appear in IPC events, CLI output, TUI transcript, or stored session data

## Acceptance
- `EG-WP6` passes with provider compatibility, canonical agent workflow, and approval-required resume flows covered by tests
- `FR-103`: tool discovery and invocation remain host-mediated, not provider-native
- `FR-108`: provider adapters normalize tool execution into the canonical runtime contract
- `FR-110`: planning and approval flows remain explicit and host-authoritative
- `FR-201`: versioned workspace commands are usable through the canonical tool loop
- `FR-203`: GitHub broker flows remain host-custodied when invoked from provider-driven tool turns
- `AC-201.2`: versioned workspace actions invoked through the provider tool loop still preserve revision-safe conflict behavior
- `AC-202.6`: install-and-go broker boundaries remain intact when git/GitHub actions are executed through provider-driven host tool calls
- given an approval-required tool call such as `git.push`, when the assistant requests the call, then the daemon pauses, emits approval metadata, resumes on decision, and continues the same transcript to final assistant output
