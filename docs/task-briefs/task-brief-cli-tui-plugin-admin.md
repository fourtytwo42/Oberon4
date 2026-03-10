# Task Brief: CLI and TUI Plugin Admin Flow

## Goal
Provide an operator-facing way to load, unload, enable, disable, and list plugins from the CLI and TUI.

## Requirements
- `FR-701` Provide daemon-backed CLI commands to list, load, unload, enable, and disable plugins.
- `FR-702` Provide daemon-backed TUI slash commands to list, load, unload, enable, and disable plugins.
- `FR-703` Keep canonical host tools directly routed so they are never resolved as plugin ids during provider-loop execution.

## In Scope
- daemon IPC contract for plugin admin
- CLI plugin commands
- TUI slash commands
- archive/config-backed state changes
- operator runbook updates

## Out of Scope
- plugin marketplace
- remote download/install
- live hot-reload

## Acceptance
- `AC-701` `plugin list` returns discovered archive/plugin state via the daemon.
- `AC-702` `plugin load <archive>` installs the archive into the configured archive root through the daemon path.
- `AC-703` `plugin unload <id>` removes the installed archive and leaves generated config in a missing-installed state.
- `AC-704` `plugin enable <id>` and `plugin disable <id>` mutate generated config and effective runtime state.
- `AC-705` TUI exposes equivalent slash commands for list/load/unload/enable/disable.
- `AC-706` Host tools remain directly routed and are never treated as plugin ids.

## Given / When / Then
- Given a running daemon and an installed archive-backed plugin, when the operator runs `plugin list`, then the daemon returns installed and enabled state without requiring direct filesystem edits.
- Given a running daemon and a valid plugin archive path, when the operator runs `plugin load <archive>` or `/plugin-load <archive>`, then the archive is registered and becomes visible through the plugin admin surfaces.
- Given a provider turn that uses canonical host tools, when the assistant calls `tool_search`, `tool_describe`, `tool_plan`, and `tool_invoke`, then host-tool routing stays canonical and no `plugin:tool_search` lookup path is attempted.
