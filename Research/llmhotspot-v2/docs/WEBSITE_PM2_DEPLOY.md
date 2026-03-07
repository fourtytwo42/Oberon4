# Website + Broker PM2 Deploy Sequence

## 0) Prepare env files

1. Broker env in repo root (`.env.local` preferred):

```bash
LLMHOTSPOT_TUNNEL_BROKER=1
LLMHOTSPOT_BROKER_DB_BACKEND=postgres
LLMHOTSPOT_BROKER_DATABASE_URL=postgres://llmhotspot_app:***@127.0.0.1:5432/llmhotspot
LLMHOTSPOT_BROKER_SHARED_SECRET=***
CRON_SECRET=***
LLMHOTSPOT_REMINDER_WEBHOOK_URL=https://llmhotspot.com
LLMHOTSPOT_REMINDER_INTERNAL_SECRET=***
LLMHOTSPOT_STRIPE_SANDBOX_MODE=1
STRIPE_SECRET_KEY=sk_test_***
STRIPE_PRICE_ID=price_***
STRIPE_WEBHOOK_SECRET=whsec_***
```

2. Website env at `apps/website/.env.local`:

```bash
cp apps/website/.env.production.example apps/website/.env.local
# then edit values
```

## 1) Build Rust broker binary

```bash
cargo build -p llmhotspot --release
```

## 2) Start/refresh broker process

```bash
bash scripts/pm2-start-broker.sh
# or, if already running
bash scripts/pm2-restart-broker.sh
```

## 3) Start/refresh website process

```bash
bash scripts/pm2-start-website.sh
# or, if already running
bash scripts/pm2-restart-website.sh
```

## 4) Verify health

```bash
pm2 status
curl -fsS http://127.0.0.1:3001/health
curl -fsS http://127.0.0.1:3010/
curl -fsS -X POST http://127.0.0.1:3010/api/pricing
```

## 5) Persist PM2 process list

```bash
pm2 save
```

## 6) Billing reminder dispatch cron (every 15 minutes)

```bash
*/15 * * * * curl -fsS -X POST http://127.0.0.1:3001/api/jobs/billing-reminders-dispatch \
  -H 'x-broker-secret: YOUR_BROKER_SHARED_SECRET' \
  -H 'x-cron-secret: YOUR_CRON_SECRET' >/dev/null 2>&1
```

## 7) Optional startup persistence

```bash
pm2 startup
# run the command PM2 prints, then:
pm2 save
```

## Notes

- Standalone app UI remains unchanged and separate.
- Website calls broker via `WEBSITE_BROKER_BASE_URL`.
- Keep `LLMHOTSPOT_BROKER_SHARED_SECRET` and `WEBSITE_BROKER_SHARED_SECRET` matched.
- Keep `LLMHOTSPOT_REMINDER_INTERNAL_SECRET` and `WEBSITE_REMINDER_INTERNAL_SECRET` matched.
- Sandbox checkout uses Stripe test credentials on broker (`sk_test_...`, test `price_...`, test webhook secret).
- With `LLMHOTSPOT_STRIPE_SANDBOX_MODE=1`, broker rejects live Stripe secret keys for checkout/portal routes.
