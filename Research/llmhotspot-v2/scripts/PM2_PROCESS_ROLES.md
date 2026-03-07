# PM2 Process Roles

Use these PM2 names consistently to avoid restarting the wrong service:

- `llmhotspot-website`
  - The Next.js website (`/home/hendo420/llmhotspot`)
  - Serves on port `3000`
- `llmhotspot-relay`
  - Remote relay/proxy process
- `llmhotspot-connector`
  - Connector bridge process
- `llmhotspot-v2-control-api`
  - Rust v2 local API/control plane (`/home/hendo420/llmhotspot-v2`)
  - Intended to serve on port `11434`

## Quick checks

```bash
pm2 status
ss -ltnp | rg ':3000|:11434'
```

## Safety rule

Before restart/delete operations, always confirm target process name with:

```bash
pm2 status
```
