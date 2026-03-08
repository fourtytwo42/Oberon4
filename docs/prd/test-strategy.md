# Oberon4 Test Strategy

## 1. Test Scope Baseline
Stack: `rust`
Required scopes:
- `unit` for each crate
- `integration` for daemon + endpoint + TUI flow
- `contract` for OpenAI-compatible request/response mapping
- `smoke` for startup/chat/quit path

## 2. Required Commands
- canonical gate: `GATE_MODE=strict bash scripts/oberon4-gate.sh`
- smoke gate: `GATE_MODE=smoke bash scripts/oberon4-gate.sh`
- canonical repair loop: `bash scripts/repair-loop.sh`
- artifact pruning: `bash scripts/prune-artifacts.sh --dry-run`
- local baseline: `cargo test`
- focused crate: `cargo test -p <crate-name>`
- lint gate (when enabled): `cargo clippy --all-targets -- -D warnings`
- PTY TUI smoke: `bash scripts/tui-pty-smoke.sh`

Gate behavior:
- `strict` runs `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, and `cargo test -p oberon4-cli --test end_to_end`
- `smoke` defaults to no heavyweight checks unless individual `RUN_*` flags are enabled
- optional live verification remains opt-in through `RUN_LIVE_ENDPOINT_SMOKE=1` and `RUN_LIVE_DAEMON_SMOKE=1`
- optional PTY TUI verification remains opt-in through `RUN_TUI_PTY_SMOKE=1`
- optional live repair-loop verification remains opt-in through `RUN_LIVE_LOCAL_SMOKE=1 bash scripts/repair-loop.sh`
- optional remediation remains opt-in through `RUN_REPAIR_REMEDIATE=1 bash scripts/repair-loop.sh`
- machine-readable diagnostics are available through `oberon4 doctor --json` and should be used by automation when diagnosis output needs stable parsing; the JSON now includes runtime metadata (`config_path`, `data_dir`, `socket_path`, `api_key_env`, `config_exists`, `daemon_reachable`) plus control-plane checks (`version_check`, `models_check`, `auth_check`, `diagnostics_check`, `available_models`, `endpoint_version`, `auth_summary`, `diagnostics_summary`, `last_error`)
- when `endpoint.api_key_env` is configured and the shell env is unset, `doctor` and `doctor --json` should best-effort materialize `env.<api_key_env>` from the local Oberon4 secret store for the current process only
- failed gate or local-live runs write a troubleshooting bundle under `artifacts/failures/<timestamp>-<entrypoint>-<pid>/`
- repair-loop runs write machine-readable stage logs and `summary.json` under `artifacts/repair-loop/<timestamp>-<pid>/`
- failure bundles now include `logs-tail.txt`, `request-summary.txt`, and `evidence-summary.json` in addition to the existing doctor/config/runtime artifacts

## 2.1 Optional Live Hotspot Smoke
Use this when a real LLM Hotspot universal endpoint is available and you want live-provider evidence in addition to deterministic tests.

Required env:
- `OBERON4_LIVE_BASE_URL`

Optional env:
- `OBERON4_LIVE_OPENAI_MODEL` default `gpt-5.1-codex-mini`
- `OBERON4_LIVE_ANTHROPIC_MODEL` default `claude-sonnet-4-6`
- `OBERON4_LIVE_TIMEOUT_SECS` default `90`
- `OBERON4_LIVE_API_KEY_ENV` name of the env var holding the bearer token when the live endpoint requires auth

Commands:
- preferred local real-endpoint smoke: `bash scripts/live-local-smoke.sh`
- shell smoke: `scripts/live-hotspot-smoke.sh`
- daemon-backed CLI smoke: `scripts/live-daemon-smoke.sh`
- Rust live provider test only:
  - `OBERON4_LIVE_HOTSPOT=1 OBERON4_LIVE_BASE_URL=<base> cargo test -p oberon4-provider-openai --test live_hotspot -- --ignored --nocapture`
- Rust live daemon/CLI test only:
  - `OBERON4_LIVE_HOTSPOT=1 OBERON4_LIVE_BASE_URL=<base> cargo test -p oberon4-cli --test live_daemon -- --ignored --nocapture`

Rules:
- keep live tests non-destructive
- do not use account/key revoke/delete operations in automated live smoke
- confirm success by control-plane route validation (`/v1/version`, `/v1/models`, `/v1/auth/status`, `/v1/diagnostics`), provider-level chat responses, and daemon-backed CLI live integration test
- prefer `scripts/live-local-smoke.sh` when validating the known local LLM Hotspot environment because it also checks `doctor --json`, requires control-plane checks to pass, writes a machine-readable run summary, and can source the configured bearer env from the local Oberon4 secret store
- if the gate or local live wrapper fails, inspect the emitted bundle in `artifacts/failures/` before re-running so diagnosis uses the original failure context
- prefer `scripts/repair-loop.sh` as the first local recovery command because it runs `doctor --json`, the strict gate, PTY smoke, and optionally the live-local wrapper in one fixed order
- if the initial `doctor --json` stage fails and local recovery is appropriate, use `RUN_REPAIR_REMEDIATE=1 bash scripts/repair-loop.sh` to attempt a local rebuild plus daemon restart before re-running diagnosis
- use `scripts/prune-artifacts.sh --dry-run` to review stale runtime evidence before removing old bundles

## 3. Coverage Expectations by Requirement
- `FR-001`: lifecycle tests (start, health, graceful stop)
- `FR-002`: command parsing + render output tests plus PTY smoke for the real `/help`, `/status`, prompt, `/save`, `/load`, `/quit` path
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
- treat `.live-manual/`, `artifacts/failures/`, `artifacts/repair-loop/`, and `artifacts/live-local-smoke/` as ephemeral runtime evidence, not review artifacts to commit
- config, session, and secret-store changes require fixture-backed compatibility coverage for additive legacy shapes
- account metadata persistence changes require fixture-backed compatibility coverage for additive legacy registry shapes
