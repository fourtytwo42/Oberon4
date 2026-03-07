# PM2 Process Roles

Use these PM2 names consistently:

- `oberon-main-app`
  - Primary Rust daemon (`llmhotspot serve`) + console
  - Serves on `127.0.0.1:11434` (console at `/console`)
- `oberon-autopilot`
  - Micro-job orchestration autopilot (optional; non-user-facing)
- `oberon-watchdog`
  - Autopilot health watchdog (optional; non-user-facing)
- `oberon-build-autopilot`
  - PM2-managed autopilot used by `scripts/multi-agent-build.sh`
- `oberon-build-watchdog`
  - PM2-managed watchdog used by `scripts/multi-agent-build.sh`
- `oberon-next`
  - Legacy website process on `3000` (vestigial for current local-app-only flow)

## Main-App-Only Control

Use the canonical control script:

```bash
scripts/main-app.sh up
scripts/main-app.sh status
scripts/main-app.sh logs
```

Notes:
- `up` defaults to stopping vestigial website + orchestration PM2 processes.
- The only required runtime for local usage is `oberon-main-app`.

## Quick checks

```bash
pm2 list
ss -ltnp | rg ':11434|:3000'
curl -I http://127.0.0.1:11434/console
```

## Safety rule

Before restart/delete operations, always verify target name:

```bash
pm2 list
```

## Multi-Agent Build Process

Use:

```bash
scripts/multi-agent-build.sh up
scripts/multi-agent-build.sh status
scripts/multi-agent-build.sh monitor
```

Runbook:
- `docs/runbooks/multi-agent-build.md`
