---
name: llmhotspot-control
description: Operate and configure LLM Hotspot over local or remote API endpoints.
metadata: {"openclaw":{"emoji":"fire","requires":{"anyBins":["python","python3","py"]}}}
---

# LLM Hotspot Control

Use this skill to inspect and administer an LLM Hotspot instance.

## When to use

Use this skill whenever the user asks to:
- inspect app state, endpoints, models, diagnostics, OAuth accounts, or keys
- change app configuration (tunnel mode, OAuth account routing/priority, key state)
- run admin operations (diagnostics clear/export, factory reset)

## Target selection

1. Prefer a remote console URL when the agent is not on the same LAN.
2. If user provides `https://.../remote?admin_key=...`, pass it as `--target`.
3. If user provides a direct API base (`.../v1`), pass it as `--target`.
4. Otherwise default to local API (`http://127.0.0.1:11434/v1`).

The helper script auto-parses remote console URLs and extracts `admin_key`.

## Command helper

Use:

```bash
python {baseDir}/llmhotspot_admin.py discover --target "<target>"
```

Generic request:

```bash
python {baseDir}/llmhotspot_admin.py request --target "<target>" --method GET --path /v1/endpoints
python {baseDir}/llmhotspot_admin.py request --target "<target>" --method POST --path /v1/tunnel-mode --body '{"mode":"quick"}'
```

Convenience commands:

```bash
python {baseDir}/llmhotspot_admin.py auth-status --target "<target>"
python {baseDir}/llmhotspot_admin.py endpoints --target "<target>"
python {baseDir}/llmhotspot_admin.py oauth-accounts --target "<target>"
python {baseDir}/llmhotspot_admin.py diagnostics --target "<target>"
python {baseDir}/llmhotspot_admin.py set-tunnel-mode --target "<target>" --mode quick
```

## Safety

- For destructive actions (`factory-reset`, key delete/revoke, oauth disconnect), ask for confirmation first.
- Never log or echo raw admin keys unless explicitly requested.
- Prefer read-only calls first when debugging connectivity or auth issues.
