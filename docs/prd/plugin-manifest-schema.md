# Oberon4 Plugin Manifest Schema

Version: 0.1
Owner: Oberon4
Status: draft

Purpose:
- define the canonical `plugin.json` schema for Oberon4 WASM/WASI plugins

Companion documents:
- `docs/prd/plugin-runtime-contract.md`
- `docs/prd/PRD-wasm-tool-runtime.md`

## 1. Schema Shape
Recommended manifest shape:

```json
{
  "id": "workspace-git",
  "version": "0.1.0",
  "displayName": "Versioned Workspace",
  "description": "Workspace editing and versioned git operations.",
  "runtimeApiVersion": "1.0",
  "commands": [],
  "capabilities": {},
  "prompting": {},
  "provenance": {}
}
```

## 2. Top-Level Fields

### `id`
- type: `string`
- required: yes
- rules:
  - lowercase kebab-case
  - stable across upgrades

### `version`
- type: `string`
- required: yes
- rules:
  - semantic version string

### `displayName`
- type: `string`
- required: yes

### `description`
- type: `string`
- required: yes

### `runtimeApiVersion`
- type: `string`
- required: yes
- rules:
  - compared against host-supported runtime API versions

### `commands`
- type: `array<CommandManifest>`
- required: yes
- rules:
  - at least one command
  - command names unique within plugin

### `capabilities`
- type: `CapabilityManifest`
- required: yes

### `prompting`
- type: `PromptingManifest`
- required: yes

### `provenance`
- type: `ProvenanceManifest`
- required: yes

## 3. `CommandManifest`

```json
{
  "name": "write_file_atomic",
  "version": "1",
  "description": "Write file contents atomically.",
  "inputSchema": {},
  "outputSchema": {},
  "tags": ["workspace", "write"],
  "sideEffects": "external-write",
  "requiredCapabilities": ["workspace.write"],
  "approvalExpectation": "required",
  "supportsPlanning": true,
  "supportsPagination": false,
  "supportsArtifacts": false,
  "preconditionModel": "workspace-revision"
}
```

Fields:
- `name`: `string`, required
- `version`: `string`, required
- `description`: `string`, required
- `inputSchema`: `object`, required
- `outputSchema`: `object`, required
- `tags`: `array<string>`, optional
- `sideEffects`: enum, required
  - `none`
  - `ephemeral`
  - `plugin-private-write`
  - `external-read`
  - `external-write`
  - `destructive`
- `requiredCapabilities`: `array<string>`, required
- `approvalExpectation`: enum, optional
  - `none`
  - `recommended`
  - `required`
- `supportsPlanning`: `boolean`, optional
- `supportsPagination`: `boolean`, optional
- `supportsArtifacts`: `boolean`, optional
- `preconditionModel`: enum, optional
  - `none`
  - `workspace-revision`
  - `file-hash`
  - `patch-context`

## 4. `CapabilityManifest`

Recommended shape:

```json
{
  "storage": {
    "private": true,
    "session": true,
    "cache": false
  },
  "workspace": {
    "read": true,
    "write": false
  },
  "artifacts": {
    "read": true,
    "create": false
  },
  "services": ["github.request"]
}
```

Rules:
- capabilities are declarative requests, not grants
- the host resolves effective granted capabilities

## 5. `PromptingManifest`

Recommended shape:

```json
{
  "promptVersion": "1",
  "summary": "Use for safe versioned workspace operations.",
  "whenToUse": "Use when reading or editing the granted workspace.",
  "whenNotToUse": "Do not use for remote-only metadata operations.",
  "examples": [
    {
      "intent": "Update a file and commit the change",
      "command": "write_file_atomic"
    }
  ],
  "failureRecovery": [
    "If a revision conflict occurs, re-read the target and re-plan."
  ],
  "safetyNotes": [
    "Writes require a valid workspace handle and may require approval."
  ]
}
```

## 6. `ProvenanceManifest`

Recommended shape:

```json
{
  "publisher": "oberon4",
  "source": "local-bundle",
  "checksum": "sha256:...",
  "signature": null,
  "build": {
    "target": "wasm32-wasip2",
    "createdAt": "2026-03-08T00:00:00Z"
  }
}
```

Fields:
- `publisher`: `string`, required
- `source`: `string`, required
- `checksum`: `string`, required
- `signature`: `object|null`, optional
- `build`: `object`, optional

## 7. Acceptance Rules
- `PMS-101`: host can validate manifest shape without executing plugin code
- `PMS-102`: command contracts are explicit and searchable
- `PMS-103`: capability requests remain declarative and host-mediated
- `PMS-104`: prompting and provenance metadata are first-class manifest fields
