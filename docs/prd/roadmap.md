# Oberon4 Roadmap (Initial)

## Phase 1: Foundations
- workspace split into modular crates
- core domain model + error model
- endpoint client with custom base URL support
- TUI command loop (basic)
- unit tests for all crates

Exit gate:
- `AC-001.1`, `AC-002.1`, `AC-003.1`, `AC-004.1`, `AC-006.1`

## Phase 2: Daemon + Tray Contracts
- daemon lifecycle manager
- tray action contract and no-op/desktop adapters
- status signaling between daemon and tray
- integration tests for startup/shutdown/action dispatch

Exit gate:
- `AC-001.2`, `AC-005.1`, `AC-005.2`, `AC-005.3`

## Phase 3: Hardening
- retries/timeouts/correlation IDs
- structured logs + redaction
- integration and contract test expansion
- smoke test scripts for local runs

Exit gate:
- all ACs in `PRD-oberon4.md` marked `met`
