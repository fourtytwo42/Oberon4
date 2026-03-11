# Handoff: GitHub OAuth Login

## Done
- Added GitHub as a first-class OAuth provider in shared models, auth, daemon, CLI, and TUI.
- Added CLI support for `account connect --provider github`.
- Added TUI support for `/github-login`.
- Wired the active GitHub account into the runtime GitHub service broker.

## Verification
- `cargo fmt --check`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-auth -p oberon4-daemon -p oberon4-cli -p oberon4-tui`

## Risks
- Real GitHub login now depends on both GitHub app credentials being available through env vars or the host secret store.

## Next Step
- Validate the operator flow against a real GitHub OAuth client configuration with credentials stored in `~/.oberon4/secrets.json`.
