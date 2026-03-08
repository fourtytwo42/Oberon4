# Oberon4 Plugin Runtime Interface Schema

Version: 0.1
Owner: Oberon4
Status: draft

Purpose:
- define the concrete host/plugin runtime interface shape that the future WIT or component contract should represent

Companion documents:
- `docs/prd/plugin-runtime-contract.md`

## 1. Interface Summary
The runtime interface should express these operations:
- `describe_plugin`
- `list_commands`
- `plan`
- `invoke`
- optional `health_check`

## 2. Core Types

### `PluginDescription`
```json
{
  "id": "workspace-git",
  "version": "0.1.0",
  "displayName": "Versioned Workspace",
  "description": "Workspace editing and versioned git operations."
}
```

### `CommandDescription`
```json
{
  "name": "write_file_atomic",
  "version": "1",
  "description": "Write file contents atomically.",
  "sideEffects": "external-write",
  "requiredCapabilities": ["workspace.write"],
  "supportsPlanning": true
}
```

### `InvocationContext`
```json
{
  "requestId": "req_123",
  "sessionId": "sess_123",
  "pluginId": "workspace-git",
  "pluginVersion": "0.1.0",
  "command": "write_file_atomic",
  "deadline": "2026-03-08T12:00:05Z",
  "runtimeApiVersion": "1.0",
  "capabilityGrants": [],
  "workspaceRevision": "wsr_123",
  "memoryLimit": 134217728,
  "timeLimitMs": 5000,
  "outputLimitBytes": 131072,
  "storageLimitBytes": 268435456,
  "networkPolicy": "denied"
}
```

### `CapabilityGrant`
```json
{
  "kind": "workspace_handle",
  "id": "wrk_123",
  "operations": ["read", "write"]
}
```

## 3. Planning Interface

### `PlanRequest`
```json
{
  "command": "write_file_atomic",
  "input": {},
  "context": {}
}
```

### `PlanResult`
```json
{
  "status": "ok",
  "sideEffects": "external-write",
  "approvalRequired": true,
  "capabilitiesRequired": ["workspace.write"],
  "estimatedCost": "low",
  "estimatedSteps": 1,
  "preconditions": ["workspaceRevision"],
  "blockingIssues": [],
  "resourceLimits": {
    "memory": 134217728,
    "timeMs": 5000
  }
}
```

## 4. Invocation Interface

### `InvokeRequest`
```json
{
  "command": "write_file_atomic",
  "input": {},
  "context": {}
}
```

### `InvokeResult`
```json
{
  "status": "ok",
  "summary": "Wrote docs/notes.md",
  "data": {},
  "artifacts": [],
  "metrics": {
    "durationMs": 43
  },
  "nextSuggestedAction": "git.diff"
}
```

### `InvokeError`
```json
{
  "status": "error",
  "code": "RESOURCE_LIMIT_STORAGE",
  "message": "Workspace quota exceeded.",
  "retryable": false,
  "requestId": "req_123",
  "pluginId": "workspace-git",
  "pluginVersion": "0.1.0",
  "command": "write_file_atomic",
  "nextSuggestedAction": "reduce output or request a higher quota"
}
```

## 5. Health Interface

### `HealthStatus`
```json
{
  "status": "ok",
  "pluginId": "workspace-git",
  "pluginVersion": "0.1.0"
}
```

## 6. Acceptance Rules
- `PIS-101`: runtime interface is concrete enough to map directly into WIT later
- `PIS-102`: planning, invocation, and error envelopes align with runtime PRDs
- `PIS-103`: invocation context carries effective granted limits and handles
