# Oberon4 Approval and Policy Schemas

Version: 0.1
Owner: Oberon4
Status: draft

Purpose:
- define concrete schema shapes for `tool_policy_inspect` and approval payloads

Companion documents:
- `docs/prd/tool-interaction-contract.md`
- `docs/prd/PRD-wasm-tool-runtime.md`

## 1. `tool_policy_inspect` Result Schema

```json
{
  "status": "ok",
  "tool": "git",
  "command": "push",
  "capabilities": {
    "allowed": ["workspace.read", "workspace.write", "github.request"],
    "blocked": []
  },
  "approvalRequired": true,
  "resourceLimits": {
    "memoryBytes": 134217728,
    "timeMs": 5000,
    "storageBytes": 268435456,
    "outputBytes": 131072,
    "networkPolicy": "brokered-only"
  },
  "operatingMode": "production",
  "reasonCodes": []
}
```

Required fields:
- `status`
- `tool`
- `command`
- `capabilities`
- `approvalRequired`
- `resourceLimits`
- `operatingMode`
- `reasonCodes`

## 2. Approval Payload Schema

```json
{
  "approvalId": "appr_123",
  "tool": "git",
  "command": "push",
  "summary": "Push branch feature/docs to origin",
  "sideEffects": "external-write",
  "workspaceImpact": {
    "workspaceHandle": "wrk_123",
    "workspaceRevision": "wsr_123",
    "touchedPaths": ["docs/notes.md"]
  },
  "remoteImpact": {
    "provider": "github",
    "remote": "origin",
    "branch": "feature/docs"
  },
  "artifacts": [],
  "serviceHandlesUsed": ["svc_github_main"],
  "secretClassesInvolved": ["oauth_credential"],
  "resourceLimits": {
    "timeMs": 5000
  },
  "diffSummary": {
    "filesChanged": 1,
    "insertions": 10,
    "deletions": 2
  }
}
```

Rules:
- approval payload must be host-generated
- secret-bearing values must not appear
- approval result must be referenceable by `approvalId`

## 3. Acceptance Rules
- `APS-101`: human approval UI can be driven from a stable schema
- `APS-102`: policy inspection can explain denials and budgets without leaking secrets
