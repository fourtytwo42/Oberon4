# Handoff: Host-Tool Routing and First-Party Package Modularity

## Done
- fixed canonical host-tool routing so `tool_search`, `tool_describe`, and `tool_policy_inspect` no longer flow through plugin-id lookup
- added runtime config for `plugin_roots`, `package_roots`, `enabled_packages`, and `disabled_packages`
- added first-party package discovery and install-state diagnostics
- converted the versioned workspace registration path to a first-party package bundle loaded from package roots in the daemon host-builder path
- added regressions for the `plugin:tool_search` failure and package disable or missing behavior

## Remaining
- optional operator UX for package management beyond filesystem-plus-config control
- launcher bugfixes outside this host-tool and package-modularity scope

## Risks
- first-party package bootstrap currently seeds the default package bundle once and then expects later add or remove operations to be filesystem-driven
- direct `FilePluginRegistry::load_from_roots(...)` retains the legacy built-in fallback for compatibility, while the daemon host path uses the new package-root flow

## Verification
- `cargo fmt --all`
- `cargo clippy -p oberon4-core -p oberon4-daemon --all-targets -- -D warnings`
- `cargo test -p oberon4-core -p oberon4-daemon`

## Next Step
- if needed, add operator-facing diagnostics or CLI commands for installed and enabled first-party package state without changing the host-brokered trust boundary
