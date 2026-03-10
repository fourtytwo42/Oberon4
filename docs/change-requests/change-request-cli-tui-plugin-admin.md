# Change Request: CLI and TUI Plugin Admin Flow

## Current State
- Archive-backed ordinary plugins and first-party packages can be installed by dropping `.obplg.zip` or `.obpkg.zip` files into `runtime.plugin_archive_roots`.
- Generated per-plugin config is written under `runtime.plugin_config_root`.
- Operators currently manage install state by editing the filesystem and generated config files directly.

## Proposed Delta
- Add daemon-backed plugin admin commands for:
  - list
  - load archive
  - unload by id
  - enable by id
  - disable by id
- Surface those commands through:
  - `oberon4-cli plugin ...`
  - TUI slash commands:
    - `/plugins`
    - `/plugin-load`
    - `/plugin-unload`
    - `/plugin-enable`
    - `/plugin-disable`

## Non-Goals
- No remote marketplace or network installer
- No hot-reload; restart-based activation remains acceptable
- No change to the host-brokered security model

## Compatibility
- Backward-compatible
- Filesystem-driven install/remove/edit remains supported

## Verification Plan
- `cargo test -p oberon4-core -p oberon4-daemon -p oberon4-cli -p oberon4-tui`
- `cargo clippy -p oberon4-core -p oberon4-daemon -p oberon4-cli -p oberon4-tui --all-targets -- -D warnings`
- security/policy/exception gates before push
