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
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `cargo test -p oberon4-cli --test end_to_end`

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

### Accounts
- [ ] `cargo run -p oberon4-cli -- account connect --provider openai`
  - expect browser-start URL output
- [ ] `cargo run -p oberon4-cli -- account connect --provider anthropic`
  - expect browser-start URL output
- [ ] `cargo run -p oberon4-cli -- account list`
  - expect connected account entries

### TUI
- [ ] start a REPL harness or call TUI library smoke path
  - verify `/help`
  - verify `/status`
  - verify `/config`
  - verify `/model o4-mini`
  - verify `/endpoint https://gateway.example.test`
  - verify `/system be concise`
  - verify plain prompt sends and transcript updates
  - verify `/save smoke`
  - verify `/clear`
  - verify `/load smoke`
  - verify `/quit`

### Tray
- [ ] verify supported-platform adapter or unsupported fallback message
  - expected actions: `open ui`, `show status`, `restart`, `quit`

## Exit Criteria
- [ ] all manual smoke steps pass
- [ ] no panic or unclassified error observed
- [ ] no secret/token values printed
- [ ] any defects discovered are recorded before release
