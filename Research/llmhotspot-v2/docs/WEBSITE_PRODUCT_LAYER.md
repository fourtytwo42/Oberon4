# Website Product Layer (Next.js)

## Scope

This website layer is separate from the standalone app console/management UI.

- Standalone app (`llmhotspot serve`) and `llm-control-api` UI remain unchanged.
- Website (`apps/website`) provides:
  - public landing + seat slider checkout
  - customer OTP-driven seat management
  - admin operations UI

## Stack

- Next.js App Router + Tailwind v4
- Postgres for website sessions/idempotency/contact state
- Redis for cache and rate-limiting
- Broker APIs remain entitlement and billing source of truth

## Required website env vars

- `WEBSITE_BROKER_BASE_URL`
- `WEBSITE_BROKER_SHARED_SECRET` (recommended)
- `WEBSITE_POSTGRES_URL`
- `WEBSITE_REDIS_URL` (optional)
- `WEBSITE_CHECKOUT_SUCCESS_URL`
- `WEBSITE_CHECKOUT_CANCEL_URL`
- `WEBSITE_CONTACT_SALES_URL`

## API boundaries

Website BFF routes call broker server-to-server. Browser clients should not call privileged broker admin routes directly.

## Pricing lock

- max self-serve seats: 100
- tiers:
  - 1: $20
  - 2-5: $18
  - 6-10: $17
  - 11-20: $16
  - 21-30: $14
  - 31-50: $12
  - 51-100: $10
- >100 seats: contact sales
