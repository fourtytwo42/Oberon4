# Oberon4 Test Strategy

## 1. Test Scope Baseline
Stack: `rust`
Required scopes:
- `unit` for each crate
- `integration` for daemon + endpoint + TUI flow
- `contract` for OpenAI-compatible request/response mapping
- `smoke` for startup/chat/quit path

## 2. Required Commands
- local baseline: `cargo test`
- focused crate: `cargo test -p <crate-name>`
- lint gate (when enabled): `cargo clippy --all-targets -- -D warnings`

## 3. Coverage Expectations by Requirement
- `FR-001`: lifecycle tests (start, health, graceful stop)
- `FR-002`: command parsing + render output tests
- `FR-003`: endpoint config validation + mapping tests + timeout tests
- `FR-004`: crate API contract tests
- `FR-005`: tray adapter tests for action mapping and unsupported platform fallback
- `FR-006`: regression tests mandatory for each bugfix

## 4. Failure Policy
- no merge on failing required tests
- flaky tests must be quarantined with owner + expiry date
- failing regression must include MRE and before/after evidence

## 5. Test Artifacts
- each PR/change set must include requirement checklist with `met/not met`
- include command outputs for required test scopes
