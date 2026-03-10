# Plan: Host-Tool Routing and First-Party Package Modularity

Status: active
Owner: Oberon4

## Objective
- fix canonical host-tool routing so `tool_search`, `tool_describe`, `tool_plan`, `tool_invoke`, and `tool_policy_inspect` are never re-resolved as plugin ids
- convert the coordinated `workspace` / `git` / `github` brokered surface from hardcoded pseudo-builtins into a first-party package discovered from package roots and controlled by runtime config

## In Scope
- runtime config for plugin roots, package roots, and package enable or disable state
- daemon registry and policy changes for first-party package discovery and package-state diagnostics
- host-tool catalog support for search, describe, and policy inspection
- first-party package bootstrap bundle for the versioned workspace package
- daemon regressions for the TUI-discovered `plugin:tool_search` failure and package state behavior

## Out of Scope
- package marketplace or remote download flow
- CLI package-manager UX
- ordinary-plugin trust-boundary redesign
- separate launcher or TUI fullscreen bug fixes

## Acceptance
- `AC-HT1`: host tools can be searched, described, and policy-inspected without any `plugin:<host-tool>` lookup path
- `AC-HT2`: the daemon provider loop no longer produces `validation failed: not found: plugin:tool_search` during the `tool_search -> tool_describe` path
- `AC-PKG1`: the versioned workspace surface loads from first-party package roots rather than hardcoded runtime registration in the daemon host builder path
- `AC-PKG2`: first-party packages can be disabled by config and are hidden from runtime search/describe/plan/invoke surfaces
- `AC-PKG3`: enabled-but-missing first-party packages fail closed with explicit policy or registry diagnostics

## Verification
- `cargo fmt --all`
- `cargo clippy -p oberon4-core -p oberon4-daemon --all-targets -- -D warnings`
- `cargo test -p oberon4-core -p oberon4-daemon`
