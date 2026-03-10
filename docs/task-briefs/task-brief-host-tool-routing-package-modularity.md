# Task Brief: Host-Tool Routing and First-Party Package Modularity

## Goal
Fix the daemon/runtime bug where canonical host tools can be mistaken for plugin ids, and make the versioned workspace package installable and removable through first-party package roots plus runtime config.

## Constraints
- keep host tools fixed and always built-in
- keep `workspace`, `git`, and `github` host-brokered
- no raw host filesystem, raw shell git, or raw credential exposure
- package add/remove is filesystem-driven; package enable/disable is config-driven

## Acceptance
- `AC-HT1`
- `AC-HT2`
- `AC-PKG1`
- `AC-PKG2`
- `AC-PKG3`

## Verification
- `cargo fmt --all`
- `cargo clippy -p oberon4-core -p oberon4-daemon --all-targets -- -D warnings`
- `cargo test -p oberon4-core -p oberon4-daemon`
