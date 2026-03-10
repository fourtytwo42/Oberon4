# Task Brief: Zip Plugin Archive Install

Objective:
- implement a mod-style plugin/package install path using zip archives and generated config files

Requirements:
- `FR-115`: zip-based install and generated per-plugin config model for ordinary plugins and first-party packages
- `AC-115.1`: support `.obplg.zip` and `.obpkg.zip` archives under host-configured archive roots
- `AC-115.2`: generate deterministic per-plugin config files and preserve operator-edited settings across restarts
- `AC-115.3`: removing an archive removes the tool surface while preserving explicit missing-installed diagnostics
- `AC-115.4`: archive integrity uses host-computed `sha256` and fails closed on malformed archive metadata or invalid generated config

Acceptance:
- Given a valid `.obplg.zip` in `plugin_archive_roots`, when the daemon starts, then the plugin appears in `tool_search` and `plugin-config/<id>.toml` is generated with defaults.
- Given an installed archive with `enabled = false` in its generated config, when the daemon starts, then the plugin does not appear in search and policy surfaces report `blocked_by_disable`.
- Given a removed archive whose config still exists, when the daemon starts, then the plugin is unavailable and policy surfaces report `blocked_by_missing_archive`.

Must:
- support `.obplg.zip` and `.obpkg.zip`
- generate `~/.oberon4/plugin-config/<id>.toml`
- preserve operator-edited settings
- keep secrets out of generated config
- fail closed on invalid archive or invalid config

Must not:
- expose raw host paths or secrets to agents
- introduce a remote installer
- break the canonical host tool workflow
