# Change Request: GitHub OAuth Callback and TUI Hardening

## Current State
- GitHub OAuth app credentials were partially environment-bound.
- The fullscreen TUI OAuth path could look stalled after browser authorization.
- Localhost callback handling in the TUI listener could miss IPv6 localhost redirects.

## Evidence
- `docs/debug-brief-github-oauth-secret-store.md`
- `docs/handoffs/handoff-github-oauth-login.md`
- Focused verification:
  - `cargo test -p oberon4-auth github`
  - `cargo test -p oberon4-cli github_account_connect_uses_env_backed_app_credentials`
  - `cargo test -p oberon4-tui oauth_callback`
  - `cargo test -p oberon4-tui oauth_status_cleanup_removes_github_login_url_lines`

## Proposed Delta
- Read GitHub OAuth app credentials from env or host secret store.
- Include the GitHub client secret in token exchange.
- Accept `/github-connect` as an alias for `/github-login`.
- Surface the GitHub login URL while OAuth is pending, then clear it after completion/failure/cancel.
- Bind localhost callback listeners for both IPv4 and IPv6 loopback.

## Non-Goals
- No device-code flow addition.
- No new daemon IPC contract.
- No raw token exposure to CLI, TUI, or plugins.

## Contract and Compatibility Impact
- Backward compatible for existing CLI/TUI operators.
- Existing callback URL `http://localhost:1455/auth/callback` remains valid.
- Existing env-var path continues to work alongside host secret store fallback.

## Migration Plan
- None required for existing operators.
- Operators may keep using env vars or store GitHub app credentials in `~/.oberon4/secrets.json`.

## Rollback
- Revert commit `5b30905` to restore the previous GitHub OAuth and TUI behavior.

## Verification Plan
- `cargo fmt --all -- --check`
- `cargo test -p oberon4-auth github`
- `cargo test -p oberon4-cli github_account_connect_uses_env_backed_app_credentials`
- `cargo test -p oberon4-tui oauth_callback`
- `cargo test -p oberon4-tui oauth_status_cleanup_removes_github_login_url_lines`
