# Task Brief: Host-Tool Routing and First-Party Package Modularity

## Goal
Fix the daemon/runtime bug where canonical host tools can be mistaken for plugin ids, and make the versioned workspace package installable and removable through first-party package roots plus runtime config.

## Constraints
- keep host tools fixed and always built-in
- keep `workspace`, `git`, and `github` host-brokered
- no raw host filesystem, raw shell git, or raw credential exposure
- package add/remove is filesystem-driven; package enable/disable is config-driven

## Requirement Mapping
- `FR-102`
- `FR-104`
- `FR-110`
- `FR-111`
- `FR-201`

## Acceptance
- `AC-102.1`
- `AC-102.2`
- `AC-104.4`
- `AC-111.2`
- `AC-201.2`
- Given the daemon provider loop handles canonical host tools, when the model calls `tool_search` followed by `tool_describe`, then the runtime must not attempt any `plugin:tool_search` lookup and must preserve host-tool result messages as canonical tool results.
- Given the versioned workspace package is installed and enabled from a configured package root, when the runtime host is built, then `workspace`, `git`, and `github` must appear through search, describe, plan, and invoke without hardcoded built-in registration in the daemon host-builder path.
- Given a first-party package is disabled or enabled-but-missing, when the runtime receives describe, plan, invoke, or policy-inspect requests for that package, then the host must fail closed with explicit package-state diagnostics rather than treating the package as an ordinary plugin lookup failure.

## Verification
- `cargo fmt --all`
- `cargo clippy -p oberon4-core -p oberon4-daemon --all-targets -- -D warnings`
- `cargo test -p oberon4-core -p oberon4-daemon`
