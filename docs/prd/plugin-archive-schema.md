# Oberon4 Plugin Archive Schema

Version: 0.1
Owner: Oberon4
Status: draft

Purpose:
- define the canonical archive wrapper for installed ordinary plugins and first-party packages
- define the generated per-plugin config contract used by the runtime loader

Companion documents:
- `docs/prd/plugin-manifest-schema.md`
- `docs/prd/plugin-runtime-contract.md`
- `docs/prd/architecture-wasm-tool-runtime.md`

## 1. Archive Model
Oberon4 supports two installed archive kinds:
- ordinary plugin archive: `.obplg.zip`
- first-party package archive: `.obpkg.zip`

The zip archive is the installed source of truth. Oberon reads manifests and executable artifacts directly from the zip file and generates a separate operator-editable config file for each discovered archive.

## 2. Required Archive Entry
Every archive must contain a top-level `oberon-archive.json`.

Recommended shape:

```json
{
  "id": "workspace-git",
  "version": "0.1.0",
  "kind": "ordinary-plugin",
  "pluginManifestPath": "plugin.json",
  "wasmPath": "plugin.wasm",
  "settings": []
}
```

For first-party packages:

```json
{
  "id": "versioned-workspace",
  "version": "0.1.0",
  "kind": "first-party-package",
  "packageManifestPath": "package.json",
  "settings": []
}
```

## 3. Top-Level Fields

### `id`
- type: `string`
- required: yes
- rules:
  - stable across archive updates
  - must match the inner `plugin.json` or `package.json` id

### `version`
- type: `string`
- required: yes
- rules:
  - must match the inner manifest version

### `kind`
- type: enum
- required: yes
- values:
  - `ordinary-plugin`
  - `first-party-package`

### `pluginManifestPath`
- type: `string`
- required: yes for `ordinary-plugin`

### `wasmPath`
- type: `string`
- required: yes for `ordinary-plugin`

### `packageManifestPath`
- type: `string`
- required: yes for `first-party-package`

### `settings`
- type: `array<ArchiveSetting>`
- required: no
- default: `[]`

### `signature`
- type: `object|null`
- required: no

### `build`
- type: `object|null`
- required: no

## 4. `ArchiveSetting`

```json
{
  "key": "mode",
  "type": "string",
  "default": "safe",
  "description": "Execution mode",
  "sensitivity": "low",
  "operatorEditable": true
}
```

Fields:
- `key`: `string`, required
- `type`: enum, required
  - `string`
  - `integer`
  - `number`
  - `boolean`
  - `array`
  - `object`
- `default`: required
- `description`: `string`, required
- `sensitivity`: `string`, required
- `operatorEditable`: `boolean`, optional

Rules:
- defaults are copied into the generated config file on first discovery
- later archive upgrades may add new settings with defaults
- removed settings should be preserved as deprecated in the generated config until explicitly migrated
- secret-like settings must not be used as a substitute for host-custodied secret handles

## 5. Ordinary Plugin Archive Contents
Required contents:
- `oberon-archive.json`
- `plugin.json`
- `plugin.wasm`

Optional contents:
- docs/assets
- `config.schema.json` or equivalent auxiliary config material

## 6. First-Party Package Archive Contents
Required contents:
- `oberon-archive.json`
- `package.json`
- referenced tool manifests such as `tools/*.plugin.json`

Optional contents:
- docs/assets
- package-level config schema material

Rules:
- first-party package archives register host-brokered tools
- first-party package archives may be manifest-only and need not carry WASM executables
- first-party packages remain host-trusted but still obey enable/disable and manifest-validation rules

## 7. Generated Config Contract
For every discovered archive, Oberon generates:
- `~/.oberon4/plugin-config/<id>.toml`

Recommended shape:

```toml
id = "workspace-git"
kind = "ordinary-plugin"
version = "0.1.0"
archivePath = "/home/user/.oberon4/plugins/workspace-git.obplg.zip"
archiveSha256 = "sha256:..."
installed = true
installedAt = "1760000000"
lastSeenAt = "1760000000"
enabled = true
loadStatus = "ready"

[settings]
mode = "safe"
```

Managed fields:
- `id`
- `kind`
- `version`
- `archivePath`
- `archiveSha256`
- `installed`
- `installedAt`
- `lastSeenAt`
- `enabled`
- `disabledReason`
- `loadStatus`
- `lastError`

Operator-editable fields:
- `[settings]`
- optionally selected runtime override fields if later supported

Rules:
- Oberon preserves operator edits to declared settings across restarts
- if an archive is removed, the config remains and `installed` becomes false
- malformed config values fail only that plugin or package and produce explicit diagnostics

## 8. Verification and Trust Rules
- archive integrity uses host-computed `sha256`
- `md5` may be carried only as informational metadata and must not drive admission or trust
- production admission for ordinary plugins still uses allowlist and conformance policy on top of archive verification
- first-party packages remain host-trusted, but still obey enable/disable, manifest validation, and policy checks

## 9. Acceptance Rules
- `PAS-101`: archive discovery can validate install metadata without executing plugin code
- `PAS-102`: generated config files are deterministic and preserve operator settings
- `PAS-103`: ordinary plugins and first-party packages share one archive-management framework
- `PAS-104`: archive integrity and generated-config diagnostics are explicit and fail closed
