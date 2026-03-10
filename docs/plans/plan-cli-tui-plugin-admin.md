# Plan: CLI/TUI Plugin Admin and Daemon Restart Hardening

## Goal
Add operator-facing plugin load/unload/enable/disable controls through the daemon-backed CLI and TUI, and harden daemon restart handling so launcher-managed restart does not fail closed during normal shutdown/startup races.

## In Scope
- Add daemon IPC commands/events for plugin admin operations.
- Add CLI commands for plugin list/load/unload/enable/disable.
- Add TUI slash commands for plugin list/load/unload/enable/disable.
- Update plugin archive install runbook for the operator flows.
- Fix daemon launcher restart/readiness probing in CLI and TUI.

## Out of Scope
- Remote marketplace or network package install.
- Hot-reload without daemon restart.
- New plugin capability classes or provider/runtime feature changes.

## Acceptance
- `tool` runtime remains unchanged for agent-facing host tool names.
- CLI supports plugin admin commands against the running daemon.
- TUI supports plugin admin slash commands against the running daemon.
- `daemon start` no longer fails spuriously during restart because of a short fixed readiness window.
- Verified by targeted CLI/TUI tests and a real daemon start/status round-trip.

## Verification
- `cargo test -p oberon4-cli -p oberon4-tui`
- `./target/debug/oberon4-cli daemon start && ./target/debug/oberon4-cli daemon status`
