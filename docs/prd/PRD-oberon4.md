# PRD: Oberon4 Daemon (Rust-first)

Version: 0.1
Owner: Oberon4
Status: draft

## 1. Problem Statement
Build a local-first, modular agent host with:
- daemon-style runtime
- tray/notification-bar integration path
- terminal TUI chat surface
- OpenAI-compatible custom endpoint support
- strict test discipline for each subsystem

## 2. Goals
- `FR-001`: daemonized runtime with clean startup/shutdown control.
- `FR-002`: TUI chat experience for interactive agent conversations.
- `FR-003`: configurable OpenAI-compatible endpoint and API key handling.
- `FR-004`: modular crate boundaries so subsystems can evolve independently.
- `FR-005`: tray/notification-bar integration contract with platform adapters.
- `FR-006`: deterministic test coverage for every subsystem before merge.

## 3. Non-Goals (v0.1)
- full plugin marketplace
- multi-tenant remote hosting
- mobile clients
- autonomous background task orchestration

## 4. Functional Requirements
- `FR-001` Daemon runtime
  - `AC-001.1`: app supports foreground run mode and daemon lifecycle abstraction.
  - `AC-001.2`: graceful shutdown drains active chat loop without data corruption.
  - `AC-001.3`: runtime exposes health/state endpoint or equivalent status contract.

- `FR-002` TUI chat
  - `AC-002.1`: user can send prompt and receive assistant response in terminal UI.
  - `AC-002.2`: TUI supports minimum commands (`/help`, `/quit`, `/config`).
  - `AC-002.3`: TUI failures show sanitized errors with correlation/request IDs.

- `FR-003` OpenAI-compatible endpoint
  - `AC-003.1`: endpoint base URL is configurable at runtime.
  - `AC-003.2`: model and API key are configurable without code changes.
  - `AC-003.3`: request/response translation supports `/v1/chat/completions` contract.
  - `AC-003.4`: timeout/retry policy is explicit and test-covered.

- `FR-004` Modularity
  - `AC-004.1`: separate crates for core domain, endpoint client, TUI, tray, daemon app.
  - `AC-004.2`: each crate has unit tests for core logic paths.
  - `AC-004.3`: cross-crate contracts are versioned and documented.

- `FR-005` Tray path
  - `AC-005.1`: tray runtime contract exists with platform adapters.
  - `AC-005.2`: unsupported platforms fail cleanly with explicit status.
  - `AC-005.3`: tray action set includes open UI, show status, restart, quit.

- `FR-006` Testing discipline
  - `AC-006.1`: every merged subsystem change includes tests.
  - `AC-006.2`: CI test command and local test command are documented.
  - `AC-006.3`: regressions require an added failing-before/passing-after test.

## 5. Non-Functional Requirements
- `NFR-001` Reliability: no panics on malformed endpoint responses.
- `NFR-002` Security: no plaintext secret logging.
- `NFR-003` Observability: structured logs include request IDs.
- `NFR-004` Maintainability: crate-level boundaries enforced by API contracts.
- `NFR-005` Performance: interactive TUI latency target <250ms local render path.

## 6. Risks and Mitigations
- endpoint variance risk -> adapter tests + strict schema validation
- tray cross-platform risk -> adapter trait + progressive platform rollout
- coupling risk -> crate boundaries + contract tests

## 7. Release Gate (v0.1)
- all `AC-001.*` through `AC-006.*` are `met`
- required test suite green
- docs + implementation + tests synchronized
