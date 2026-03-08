# Oberon4 Release Readiness

Version: 0.1
Status: active

## Required Gates
- [ ] `EG-P0` through `EG-P7` are green
- [ ] `EG-P8` full workspace gate is green
- [ ] `docs/plans/plan-oberon4-v0.1-build.md` has all tasks checked

## Verification Evidence
- [ ] `GATE_MODE=strict bash scripts/oberon4-gate.sh`
- [ ] PTY TUI smoke: `bash scripts/tui-pty-smoke.sh`
- [ ] local real-endpoint wrapper: `bash scripts/live-local-smoke.sh`
- [ ] optional live endpoint coverage: `OBERON4_LIVE_BASE_URL=<base> RUN_LIVE_ENDPOINT_SMOKE=1 GATE_MODE=smoke bash scripts/oberon4-gate.sh`
- [ ] optional live daemon coverage: `OBERON4_LIVE_BASE_URL=<base> RUN_LIVE_DAEMON_SMOKE=1 GATE_MODE=smoke bash scripts/oberon4-gate.sh`
- [ ] optional PTY TUI coverage through gate: `RUN_TUI_PTY_SMOKE=1 GATE_MODE=smoke bash scripts/oberon4-gate.sh`
- [ ] any failed gate/live verification preserved its troubleshooting bundle under `artifacts/failures/`

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
- [ ] `oberon4 doctor --json` returns valid JSON with no secret/token values
- [ ] responders know to inspect `artifacts/failures/<timestamp>-<entrypoint>-<pid>/` for `summary.json`, `commands.txt`, `env.txt`, logs, and doctor output before retrying

## Human Test Entry
- [ ] `docs/runbooks/oberon4-human-test-checklist.md` exists
- [ ] human smoke pass completed
- [ ] no open stop-ship defects remain
