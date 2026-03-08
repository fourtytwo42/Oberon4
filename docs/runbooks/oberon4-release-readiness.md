# Oberon4 Release Readiness

Version: 0.1
Status: active

## Required Gates
- [ ] `EG-P0` through `EG-P7` are green
- [ ] `EG-P8` full workspace gate is green
- [ ] `docs/plans/plan-oberon4-v0.1-build.md` has all tasks checked

## Verification Evidence
- [ ] `bash scripts/repair-loop.sh`
- [ ] `GATE_MODE=strict bash scripts/oberon4-gate.sh`
- [ ] PTY TUI smoke: `bash scripts/tui-pty-smoke.sh`
- [ ] local real-endpoint wrapper: `bash scripts/live-local-smoke.sh`
- [ ] optional repair-loop live coverage: `RUN_LIVE_LOCAL_SMOKE=1 bash scripts/repair-loop.sh`
- [ ] optional live endpoint coverage: `OBERON4_LIVE_BASE_URL=<base> RUN_LIVE_ENDPOINT_SMOKE=1 GATE_MODE=smoke bash scripts/oberon4-gate.sh`
- [ ] optional live daemon coverage: `OBERON4_LIVE_BASE_URL=<base> RUN_LIVE_DAEMON_SMOKE=1 GATE_MODE=smoke bash scripts/oberon4-gate.sh`
- [ ] optional PTY TUI coverage through gate: `RUN_TUI_PTY_SMOKE=1 GATE_MODE=smoke bash scripts/oberon4-gate.sh`
- [ ] any failed gate/live verification preserved its troubleshooting bundle under `artifacts/failures/`
- [ ] repair-loop evidence preserved under `artifacts/repair-loop/`

## Requirement Readiness
- [ ] `FR-001` daemon runtime verified
- [ ] `FR-002` TUI command and chat flow verified
- [ ] `FR-003` OpenAI-compatible endpoint path verified
- [ ] `FR-004` modular crate boundaries preserved
- [ ] `FR-005` tray contract and fallback verified
- [ ] `FR-006` tests present for each subsystem
- [ ] `FR-007` OpenAI and Anthropic OAuth flows verified
- [ ] `FR-008` CLI surface verified
- [ ] `FR-009` TUI surface verified

## Diagnostic Readiness
- [ ] `oberon4 doctor` returns success text on healthy configuration
- [ ] `oberon4 doctor --json` returns valid JSON with no secret/token values and includes local runtime metadata (`config_path`, `data_dir`, `socket_path`, `api_key_env`, `config_exists`, `daemon_reachable`) plus control-plane fields (`version_check`, `models_check`, `auth_check`, `diagnostics_check`, `available_models`, `endpoint_version`, `auth_summary`, `diagnostics_summary`, `last_error`)
- [ ] responders know to inspect `artifacts/failures/<timestamp>-<entrypoint>-<pid>/` for `summary.json`, `commands.txt`, `env.txt`, logs, `effective-config.json`, and doctor output before retrying
- [ ] responders know to inspect `artifacts/repair-loop/<timestamp>-<pid>/summary.json` for stage order, pass/fail status, and any downstream failure-bundle pointer
- [ ] live-local smoke confirms the configured model is advertised by `/v1/models` and that `/v1/auth/status` plus `/v1/diagnostics` return valid JSON

## Human Test Entry
- [ ] `docs/runbooks/oberon4-human-test-checklist.md` exists
- [ ] human smoke pass completed
- [ ] no open stop-ship defects remain
