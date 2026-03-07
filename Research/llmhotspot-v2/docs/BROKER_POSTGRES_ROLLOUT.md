# Broker Postgres Rollout Checklist

## 1) Apply schema

- Create/verify target Postgres database.
- Apply schema SQL:
  - `db/postgres/broker_schema.sql`

## 2) Configure broker runtime

Set environment variables:

- `LLMHOTSPOT_TUNNEL_BROKER=1`
- `LLMHOTSPOT_BROKER_DB_BACKEND=postgres`
- `LLMHOTSPOT_BROKER_DATABASE_URL=postgres://...`
- Existing broker envs (`CF_API_TOKEN`, `CF_ACCOUNT_ID`, `CF_ZONE_ID`, Stripe + admin envs)

Start command:

- `llmhotspot broker --listen 0.0.0.0:3001 --db-backend postgres`

## 3) Validate health and critical flows

- `GET /health`
- `POST /billing/webhook` idempotency check
- `POST /api/licenses/activate`
- `POST /admin/auth/login` and `GET /admin/metrics`

## 4) Monitor

- Broker 5xx/error rates
- DB connection/auth errors
- Webhook replay/reconcile failures
- Entitlement mutation failures

## 5) Emergency rollback

- Switch broker backend to SQLite:
  - `LLMHOTSPOT_BROKER_DB_BACKEND=sqlite`
  - run with `--db-path /tmp/llmhotspot-broker.db`
