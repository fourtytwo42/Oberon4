# Handoff: CLI/TUI Plugin Admin and Daemon Restart Hardening

## Phase
verify

## Completed Scope
- Added daemon-backed CLI plugin admin commands for list/load/unload/enable/disable.
- Added TUI slash commands for plugin list/load/unload/enable/disable.
- Added IPC command/event support for plugin admin operations.
- Updated archive install runbook for operator-facing CLI/TUI flows.
- Hardened CLI/TUI daemon launcher restart probes to wait for shutdown and then wait longer for readiness.

## Remaining Scope
- None for this change set.

## Risks
- The plugin admin change set is broad across CLI, TUI, daemon, core IPC, and runtime registration, so follow-up manual validation in the TUI is still useful after merge.
- The restart hardening changes timing behavior in launcher paths, so future daemon lifecycle changes should keep the same state-based probing model.

## Verification
- `cargo test -p oberon4-cli -p oberon4-tui`
- `./target/debug/oberon4-cli daemon start && ./target/debug/oberon4-cli daemon status`

## Next Step
- Manual TUI smoke validation of `/plugins`, `/plugin-load`, `/plugin-enable`, `/plugin-disable`, and `/plugin-unload` against a running daemon.
