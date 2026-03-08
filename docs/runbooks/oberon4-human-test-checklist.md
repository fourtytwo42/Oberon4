# Oberon4 Human Test Checklist

Version: 0.1
Status: active

## Scope
Use this checklist only after automated gates through `EG-P8` are green.

## Environment
- Rust toolchain installed
- `TMPDIR=/home/hendo420/Oberon4/.tmp`
- `CARGO_INCREMENTAL=0`
- `CARGO_PROFILE_DEV_DEBUG=0`
- `CARGO_PROFILE_TEST_DEBUG=0`
- `RUSTFLAGS='-C debuginfo=0'`

## Automated Preconditions
- [ ] `bash scripts/install-git-hooks.sh`
- [ ] `bash scripts/check-policy-stack.sh`
- [ ] `bash scripts/policy-drift-audit.sh`
- [ ] `bash scripts/repair-loop.sh`
  - if this fails, inspect `artifacts/repair-loop/<timestamp>-<pid>/summary.json`
  - inspect any referenced `failure_bundle_path` before reproducing manually
- [ ] `GATE_MODE=strict bash scripts/oberon4-gate.sh`
  - if this fails, inspect `artifacts/failures/<timestamp>-oberon4-gate-<pid>/`
- [ ] `bash scripts/oberon4-security-gate.sh`
  - expect `gitleaks`, pinned `semgrep`, `cargo-audit`, and `cargo-deny` stages to pass
- [ ] `cargo run -p oberon4-cli -- doctor --json`
  - expect `version_check=pass`, `models_check=pass`, `auth_check=pass`, and `diagnostics_check=pass` in the JSON output before manual troubleshooting continues
  - if `api_key_env` is configured, this should work from either the exported shell env or the local Oberon4 secret store

## Optional Live Endpoint Preconditions
- [ ] `bash scripts/live-local-smoke.sh`
  - if this fails, inspect `artifacts/failures/<timestamp>-live-local-smoke-<pid>/`
- [ ] `RUN_LIVE_LOCAL_SMOKE=1 bash scripts/repair-loop.sh`
  - if this fails, inspect `artifacts/repair-loop/<timestamp>-<pid>/summary.json` and the referenced `failure_bundle_path`
- [ ] optional local remediation: `RUN_REPAIR_REMEDIATE=1 bash scripts/repair-loop.sh`
  - use this only after diagnosis-only repair fails and local restart/rebuild is acceptable

## Manual Smoke Flow
### CLI
- [ ] `cargo run -p oberon4-cli -- daemon start`
  - expect `running=true`
- [ ] `cargo run -p oberon4-cli -- config show`
  - expect redacted config output
- [ ] `cargo run -p oberon4-cli -- config set endpoint.base_url https://gateway.example.test`
  - expect deterministic success output
- [ ] `cargo run -p oberon4-cli -- endpoint test`
  - expect `endpoint test passed`
- [ ] `cargo run -p oberon4-cli -- chat --message "hello"`
  - expect assistant reply text
- [ ] `cargo run -p oberon4-cli -- logs tail`
  - expect JSON log line with `request_id`
- [ ] `cargo run -p oberon4-cli -- doctor`
  - expect success once daemon is running
- [ ] optional live smoke: `bash scripts/live-local-smoke.sh`
  - expect control-plane validation, provider live smoke, daemon-backed smoke, and `doctor --json` to pass against the local LLM Hotspot

### Accounts
- [ ] `cargo run -p oberon4-cli -- account connect --provider openai`
  - expect browser-start URL output
- [ ] `cargo run -p oberon4-cli -- account connect --provider anthropic`
  - expect browser-start URL output
- [ ] `cargo run -p oberon4-cli -- account list`
  - expect connected account entries

### TUI
- [ ] automated PTY smoke: `bash scripts/tui-pty-smoke.sh`
  - covers the default structured fullscreen chat render, permanent left session rail, `/sessions`, `/settings`, `/status`, `/help`, prompt send, `/save`, `/fork`, `/load`, `/quit`, and explicit `oberon4 tui --no-alt-screen`
- [ ] manual TUI spot-check only for anything not covered by PTY smoke
  - verify `cargo run -p oberon4-cli -- tui` uses normal terminal scrollback with no sticky overview/header block
  - verify `/config`
  - verify `/sessions`
  - verify `/settings`
  - verify `/accounts`
  - verify `/diagnostics`
  - verify `/more`
  - verify `/model o4-mini`
  - verify `/endpoint https://gateway.example.test`
  - verify `/system be concise`
  - verify multiline drafting with `Ctrl+J`
  - verify slash autocomplete with `Tab`
  - verify automatic session titling after the first completed exchange
  - verify selected session titles scroll when the sidebar selection is longer than the visible width
  - verify `Esc` backs out from settings/accounts/help/session browser before returning to chat

### Tray
- [ ] verify supported-platform adapter or unsupported fallback message
  - expected actions: `open ui`, `show status`, `restart`, `quit`

## Exit Criteria
- [ ] all manual smoke steps pass
- [ ] no panic or unclassified error observed
- [ ] no secret/token values printed
- [ ] any defects discovered are recorded before release
- [ ] any failed automated precondition has an associated bundle in `artifacts/failures/` for follow-up diagnosis
- [ ] failed bundles include `daemon-runtime.txt` and any available `daemon-out.log` / `daemon-err.log` before manual daemon investigation starts
- [ ] failed bundles include `logs-tail.txt`, `request-summary.txt`, and `evidence-summary.json` before manual log hunting starts
- [ ] runtime evidence directories (`.live-manual/`, `artifacts/failures/`, `artifacts/repair-loop/`, `artifacts/live-local-smoke/`) are treated as ephemeral and not staged for release commits
