# Plan: Zip Plugin Archive Install

Status: implemented
Owner: Oberon4

Goal:
- add zip-based install and generated per-plugin config for ordinary plugins and first-party packages

Scope:
- archive discovery from `plugin_archive_roots`
- generated config files under `plugin_config_root`
- ordinary `.obplg.zip` and first-party `.obpkg.zip` support
- archive-aware policy and diagnostics

Out of scope:
- remote marketplace or download flow
- live hot reload
- moving first-party package execution out of host brokers

Acceptance summary:
- archive-backed plugins and packages are discoverable
- generated config files are created and preserved
- disable and missing-archive states are explicit
- host tool routing remains direct and never re-resolves `tool_search` as a plugin id
