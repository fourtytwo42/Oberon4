# PM2 Processes (llmhotspot server)

Last updated: 2026-02-15

## Active

### `llmhotspot-v2-broker`

- Binary: `/home/hendo420/llmhotspot-v2/target/release/llmhotspot`
- Typical args: `broker --listen 0.0.0.0:3001 --db-backend postgres`
- Required env: `LLMHOTSPOT_BROKER_DATABASE_URL=postgres://...`
- Purpose: Central tunnel broker behind `keys.llmhotspot.com`.
  - Issues per-device Cloudflare tunnel + DNS (`/api/tunnels/provision`)
  - Lease heartbeat (`/api/tunnels/heartbeat`)
  - Reap stale leases / delete tunnels (`/api/jobs/tunnels-reap`)
  - Dispatch billing reminder emails (`/api/jobs/billing-reminders-dispatch`)
- Storage: Postgres for broker/website server state (billing, entitlements, broker leases, audits).
- Emergency fallback: set `LLMHOTSPOT_BROKER_DB_BACKEND=sqlite` and provide `--db-path`.

### `llmhotspot-v2-control-api`

- Binary: `/home/hendo420/llmhotspot-v2/target/release/llmhotspot`
- Typical args: `--db-path /tmp/llmhotspot-test-client.db serve --listen 0.0.0.0:11434 --no-tray`
- Purpose: Main Hotspot runtime (API + console) for the machine.
  - Serves local API/console on `:11434`
  - Runs TunnelManager when `LLMHOTSPOT_TUNNEL_RUNTIME=1`:
    - Calls `keys.llmhotspot.com` broker for per-device tunnel credentials
    - Spawns/supervises bundled `cloudflared`
    - Maintains remote endpoint + remote console
- Storage: SQLite at `/tmp/llmhotspot-test-client.db` (app state, keys, etc.)

## Backend Split Contract

- `llmhotspot serve` remains self-contained and uses local SQLite only.
- `llmhotspot broker` should use Postgres in production.

### `llmhotspot-website` (legacy)

- CWD: `/home/hendo420/llmhotspot`
- Typical args: `npm start`
- Purpose: Legacy Next.js website.
  - Not used for tunnel provisioning anymore (broker is authoritative).

## Removed

### `llmhotspot-relay` (legacy)

- Previously: `npm run relay:start` (stopped/removed from PM2)

### `llmhotspot-connector` (legacy)

- Previously: `node scripts/relay-connector-bridge.mjs` (stopped/removed from PM2)


### `llmhotspot-v2-website`

- CWD: `/home/hendo420/llmhotspot-v2/apps/website`
- Typical args: `pnpm start` (or `npm run start`)
- Purpose: new public website + customer/admin web product layer (landing, checkout, seat management, admin UI).
- Storage: Postgres + Redis.
- Start script: `bash scripts/pm2-start-website.sh`
- Restart script: `bash scripts/pm2-restart-website.sh`

See deploy sequence:
- `docs/WEBSITE_PM2_DEPLOY.md`
