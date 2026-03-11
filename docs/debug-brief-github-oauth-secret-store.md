# Debug Brief

## Symptom
- GitHub OAuth login could not complete for local OAuth apps because Oberon4 only supported the GitHub client ID from environment and did not supply a client secret during token exchange.

## Repro Steps
1. Register a localhost GitHub OAuth app with callback `http://localhost:1455/auth/callback`.
2. Start the daemon and TUI with only `OBERON4_GITHUB_CLIENT_ID` set.
3. Run `/github-login` or `account connect --provider github`.
4. Complete the browser authorization and observe token exchange failure.

## Current State Snapshot
- Branch: current workspace
- Commit: uncommitted
- Dirty files: `crates/oberon4-auth/src/lib.rs`, `crates/oberon4-tui/src/lib.rs`, `artifacts/`

## Key Evidence
- Failing behavior:
  - GitHub OAuth app flow requires both app identity and a secret-backed token exchange path for this localhost callback model.
- Failing test:
  - `cargo test -p oberon4-auth`
- Error output:
  - Before test fix: fallback coverage failed because the test wrote the secret file outside the `OBERON4_DATA_DIR/secrets.json` lookup path.

## Hypotheses Tried
- H1: Environment-only client ID support was insufficient for GitHub OAuth app exchange.
- H2: Reusing Oberon4's existing host secret store for `client_id` and `client_secret` would preserve current custody boundaries without adding a new config surface.

## Outcomes
- GitHub OAuth config now reads `client_id` and `client_secret` from environment or host secret store.
- GitHub token exchange now sends `client_secret` when configured.
- Added regression coverage for host secret store fallback using the real default secret-store path semantics.

## Next Smallest Step
- Rerun `cargo test -p oberon4-auth`, then store the GitHub app credentials in `~/.oberon4/secrets.json` and verify the daemon/TUI login path.
