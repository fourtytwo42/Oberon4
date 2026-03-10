# Plan: GitHub OAuth Login

## Summary
Add GitHub as a first-class OAuth provider in Oberon4 by extending the existing daemon-backed account connect flow. Expose the operator entrypoints through the CLI and TUI while keeping tokens host-custodied and brokered into the existing `github.*` and `git.push` runtime paths.

## Scope
- In scope:
  - `OAuthProviderKind::Github`
  - auth adapter and provider metadata in `oberon4-auth`
  - daemon account-connect and active-account resolution for GitHub
  - CLI `account connect --provider github`
  - TUI `/github-login`
  - runtime GitHub broker wiring from the active GitHub account
- Out of scope:
  - device-code or headless auth flows
  - new GitHub-only IPC commands
  - raw token exposure

## Acceptance
- `FR-101`: GitHub is available as a stable OAuth provider kind across shared models and storage
- `AC-201`: CLI accepts `account connect --provider github` and completes the daemon-backed OAuth flow
- `AC-202`: TUI accepts `/github-login` and starts the existing asynchronous OAuth connect flow
- `AC-203`: active GitHub account state feeds the runtime GitHub service broker so `github.auth_status` reflects real connectivity
- `AC-204`: no raw GitHub token becomes agent-visible or plugin-readable

## Files
- `crates/oberon4-core/src/domain.rs`
- `crates/oberon4-core/src/mock.rs`
- `crates/oberon4-core/src/state.rs`
- `crates/oberon4-auth/src/lib.rs`
- `crates/oberon4-daemon/src/lib.rs`
- `crates/oberon4-daemon/src/plugin_runtime_broker.rs`
- `crates/oberon4-cli/src/lib.rs`
- `crates/oberon4-cli/tests/end_to_end.rs`
- `crates/oberon4-tui/src/lib.rs`

## Verification
```bash
cargo fmt --check
env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-auth -p oberon4-daemon -p oberon4-cli -p oberon4-tui
```
