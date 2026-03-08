# Oberon4 Release Readiness

Version: 0.1
Status: active

## Required Gates
- [ ] `EG-P0` through `EG-P7` are green
- [ ] `EG-P8` full workspace gate is green
- [ ] `docs/plans/plan-oberon4-v0.1-build.md` has all tasks checked

## Verification Evidence
- [ ] `bash scripts/install-git-hooks.sh`
- [ ] `bash scripts/check-policy-stack.sh`
- [ ] `bash scripts/policy-drift-audit.sh`
- [ ] `bash scripts/repair-loop.sh`
- [ ] `GATE_MODE=strict bash scripts/oberon4-gate.sh`
- [ ] PTY TUI smoke: `bash scripts/tui-pty-smoke.sh`
- [ ] security bootstrap: `bash scripts/bootstrap-security-tools.sh`
- [ ] security gate: `bash scripts/oberon4-security-gate.sh`
- [ ] local real-endpoint wrapper: `bash scripts/live-local-smoke.sh`
- [ ] optional repair-loop live coverage: `RUN_LIVE_LOCAL_SMOKE=1 bash scripts/repair-loop.sh`
- [ ] optional remediation path exercised when needed: `RUN_REPAIR_REMEDIATE=1 bash scripts/repair-loop.sh`
- [ ] optional live endpoint coverage: `OBERON4_LIVE_BASE_URL=<base> RUN_LIVE_ENDPOINT_SMOKE=1 GATE_MODE=smoke bash scripts/oberon4-gate.sh`
- [ ] optional live daemon coverage: `OBERON4_LIVE_BASE_URL=<base> RUN_LIVE_DAEMON_SMOKE=1 GATE_MODE=smoke bash scripts/oberon4-gate.sh`
- [ ] optional PTY TUI coverage through gate: `RUN_TUI_PTY_SMOKE=1 GATE_MODE=smoke bash scripts/oberon4-gate.sh`
- [ ] optional security coverage through gate: `RUN_SECURITY_GATE=1 GATE_MODE=smoke bash scripts/oberon4-gate.sh`
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
  - includes default fullscreen `oberon4 tui`, explicit inline compatibility `oberon4 tui --no-alt-screen`, slash-command overlay navigation, permanent session rail UX, `/sessions`, `/settings`, `/accounts`, `/diagnostics`, `/more`, `/fork`, multiline drafting, and automatic session titling coverage
- [ ] account metadata compatibility coverage present for legacy/additive registry shapes

## Diagnostic Readiness
- [ ] `oberon4 doctor` returns success text on healthy configuration
- [ ] `oberon4 doctor --json` returns valid JSON with no secret/token values and includes local runtime metadata (`config_path`, `data_dir`, `socket_path`, `api_key_env`, `config_exists`, `daemon_reachable`) plus control-plane fields (`version_check`, `models_check`, `auth_check`, `diagnostics_check`, `available_models`, `endpoint_version`, `auth_summary`, `diagnostics_summary`, `last_error`)
- [ ] diagnostics and live verification succeed when the configured API-key env is available either from the exported shell env or the local Oberon4 secret store
- [ ] `security/gitleaks-baseline.json` is present, reviewed, and only suppresses approved existing findings
- [ ] `security/semgrep.yml` is present, reviewed, and used by the security gate
- [ ] `deny.toml` is present, reviewed, and used by the security gate
- [ ] `docs/security/runbook-secret-triage.md` and `docs/security/security-checklist-secret-triage.md` are current
- [ ] responders know to inspect `artifacts/failures/<timestamp>-<entrypoint>-<pid>/` for `summary.json`, `commands.txt`, `env.txt`, logs, `effective-config.json`, and doctor output before retrying
- [ ] responders know failure bundles now include `daemon-runtime.txt` plus best-effort `daemon-out.log` and `daemon-err.log`
- [ ] responders know failure bundles now include `logs-tail.txt`, `request-summary.txt`, and `evidence-summary.json` before manual reproduction starts
- [ ] responders know to inspect `artifacts/repair-loop/<timestamp>-<pid>/summary.json` for stage order, pass/fail status, and any downstream failure-bundle pointer
- [ ] responders understand remediation mode is optional local recovery evidence, not a routine release gate requirement
- [ ] live-local smoke confirms the configured model is advertised by `/v1/models` and that `/v1/auth/status` plus `/v1/diagnostics` return valid JSON
- [ ] optional maintenance command `bash scripts/prune-artifacts.sh --dry-run` has been reviewed before pruning old runtime evidence
- [ ] security run artifacts under `artifacts/security-gate/` are available for the latest security gate run
- [ ] operators understand strict local enforcement now includes the security gate by default once the security toolchain is bootstrapped

## Human Test Entry
- [ ] `docs/runbooks/oberon4-human-test-checklist.md` exists
- [ ] human smoke pass completed
- [ ] no open stop-ship defects remain
