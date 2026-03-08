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
- [ ] `bash scripts/repair-loop.sh`
  - if this fails, inspect `artifacts/repair-loop/<timestamp>-<pid>/summary.json`
- [ ] `GATE_MODE=strict bash scripts/oberon4-gate.sh`
  - if this fails, inspect `artifacts/failures/<timestamp>-oberon4-gate-<pid>/`

## Optional Live Endpoint Preconditions
- [ ] `bash scripts/live-local-smoke.sh`
  - if this fails, inspect `artifacts/failures/<timestamp>-live-local-smoke-<pid>/`
- [ ] `RUN_LIVE_LOCAL_SMOKE=1 bash scripts/repair-loop.sh`
  - if this fails, inspect `artifacts/repair-loop/<timestamp>-<pid>/summary.json` and the referenced `failure_bundle_path`

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
  - expect provider live smoke, daemon-backed smoke, and `doctor --json` to pass against the local LLM Hotspot

### Accounts
- [ ] `cargo run -p oberon4-cli -- account connect --provider openai`
  - expect browser-start URL output
- [ ] `cargo run -p oberon4-cli -- account connect --provider anthropic`
  - expect browser-start URL output
- [ ] `cargo run -p oberon4-cli -- account list`
  - expect connected account entries

### TUI
- [ ] automated PTY smoke: `bash scripts/tui-pty-smoke.sh`
  - covers `/help`, `/status`, prompt send, `/save`, `/clear`, `/load`, and `/quit`
- [ ] manual TUI spot-check only for anything not covered by PTY smoke
  - verify `/config`
  - verify `/model o4-mini`
  - verify `/endpoint https://gateway.example.test`
  - verify `/system be concise`

### Tray
- [ ] verify supported-platform adapter or unsupported fallback message
  - expected actions: `open ui`, `show status`, `restart`, `quit`

## Exit Criteria
- [ ] all manual smoke steps pass
- [ ] no panic or unclassified error observed
- [ ] no secret/token values printed
- [ ] any defects discovered are recorded before release
- [ ] any failed automated precondition has an associated bundle in `artifacts/failures/` for follow-up diagnosis
- [ ] runtime evidence directories (`.live-manual/`, `artifacts/failures/`, `artifacts/repair-loop/`, `artifacts/live-local-smoke/`) are treated as ephemeral and not staged for release commits
