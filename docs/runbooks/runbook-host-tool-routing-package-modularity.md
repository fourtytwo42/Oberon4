# Runbook: Host-Tool Routing and First-Party Package Modularity

## Scope
- canonical host-tool routing in the daemon provider loop
- first-party package discovery for the coordinated `workspace` / `git` / `github` surface
- runtime config for plugin roots, package roots, and package enable or disable state

## Normal Operation
1. Start the daemon with the configured runtime package roots and plugin roots.
2. Verify host tools appear through `tool_search` and `tool_describe`.
3. Verify installed and enabled first-party packages appear through runtime search and policy inspection.
4. Verify disabled or missing packages fail closed with explicit policy or registry diagnostics.

## Verification
- `cargo fmt --all`
- `cargo clippy -p oberon4-core -p oberon4-daemon --all-targets -- -D warnings`
- `cargo test -p oberon4-core -p oberon4-daemon`

## Rollback
1. Revert the host-tool routing and first-party package modularity commit.
2. Restart the daemon.
3. Re-run the verification commands above.
4. Confirm host tools and the versioned workspace surface return to the prior built-in registration path.

## Operational Notes
- host tools remain built-in and are not removable
- package install or remove is filesystem-driven under configured package roots
- package enable or disable is config-driven
- first-party package execution remains host-brokered rather than ordinary WASM execution
