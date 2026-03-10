# Runbook: Plugin Archive Install

## Archive Locations
Default roots:
- ordinary and first-party archive inbox: `~/.oberon4/plugins`
- generated config files: `~/.oberon4/plugin-config`

Configured by:
- `runtime.plugin_archive_roots`
- `runtime.plugin_config_root`

## Install
1. Drop an archive into a configured archive root.
2. Use:
   - `.obplg.zip` for ordinary WASM plugins
   - `.obpkg.zip` for first-party brokered packages
3. Restart the daemon.
4. Oberon will:
   - validate the archive manifest
   - compute archive `sha256`
   - generate `plugin-config/<id>.toml` if missing
   - load the plugin or package if enabled and policy-admitted

CLI path:

```bash
oberon4-cli plugin load /path/to/example.obplg.zip
```

TUI path:
- `/plugin-load /path/to/example.obplg.zip`

## Disable
Edit the generated config:

```toml
enabled = false
```

Restart the daemon.

CLI path:

```bash
oberon4-cli plugin disable <id>
```

TUI path:
- `/plugin-disable <id>`

## Re-enable
Edit the generated config:

```toml
enabled = true
```

Restart the daemon.

CLI path:

```bash
oberon4-cli plugin enable <id>
```

TUI path:
- `/plugin-enable <id>`

## Remove
1. Delete the `.obplg.zip` or `.obpkg.zip`.
2. Restart the daemon.
3. Oberon will keep the generated config but mark the archive as missing-installed and remove the tool surface from discovery and invocation.

CLI path:

```bash
oberon4-cli plugin unload <id>
```

TUI path:
- `/plugin-unload <id>`

## List Installed Plugins
CLI path:

```bash
oberon4-cli plugin list
```

TUI path:
- `/plugins`

## Diagnostics
Policy and registry diagnostics should expose:
- installed: yes/no
- enabled: yes/no
- archive kind
- archive checksum
- blocked reasons such as:
  - `blocked_by_disable`
  - `blocked_by_missing_archive`
  - `blocked_by_contract`
  - `blocked_by_allowlist`
  - `blocked_by_conformance`
