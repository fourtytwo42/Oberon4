# LLM Hotspot Monetization and Entitlements Plan

## Goal

Build a production-ready monetization and entitlement system for LLM Hotspot with:

- Stripe subscription billing
- Email-first identity (no username/password accounts)
- One license key per subscription
- Seat-based device activation
- Self-service device retirement and seat management
- Clear cancel and downgrade paths

This document captures agreed decisions and the implementation plan so work can proceed in the website and broker services.

## Product and Deployment Scope

## VM-hosted services

This VM should run:

- `llmhotspot-v2-broker` (Cloudflared provisioning service)
- New website (billing, management, support pages)
- Subscription and entitlement backend endpoints

This VM should not run:

- `llmhotspot serve` client runtime (`llmhotspot-v2-control-api`)

The client app runs on user machines.

## Pricing Model

## Personal Pro

- Billing basis: Stripe subscription item quantity with tiered volume pricing
- Self-serve seat cap: `100` seats
- Above 100 seats: `Contact Sales`
- Tier schedule (monthly, per seat):
  - `1` seat: `$20`
  - `2-5` seats: `$18`
  - `6-10` seats: `$17`
  - `11-20` seats: `$16`
  - `21-30` seats: `$14`
  - `31-50` seats: `$12`
  - `51-100` seats: `$10`
- Same license key for all seats under that subscription

## Enterprise

- If needs exceed self-serve boundaries, route to Contact Sales
- Public messaging: "Contact us for enterprise pricing"

## Identity and Access Model (No Traditional Accounts)

No password accounts are required.

Identity and control are email-based:

- User enters email at Stripe checkout
- Email becomes customer identity for management flows
- Sensitive management actions require email OTP verification

## Core Entitlement Rules

- Active paid subscription enables Pro entitlements
- Seat limit equals paid quantity
- One active device binding consumes one seat
- Billing grace period: `30 days` after payment failure or cancellation-at-period-end
- During grace, existing assigned seats remain valid
- After grace expires, Pro entitlements are suspended and Pro tunnel access is deprovisioned

Free behavior remains available according to product policy.

## License and Seat Model

## License key

- One license key per subscription
- Key remains stable across renewals and quantity changes
- Key is delivered in post-purchase email
- Key can be used by app for manual activation fallback

## Device bindings

Each bound seat stores:

- `device_id`
- `device_name`
- `os`
- `app_version`
- `label` (user-editable)
- `seat_index` (stable seat slot within the subscription)
- `tunnel_id` (dedicated Cloudflared tunnel for this seat)
- `tunnel_hostname`
- status (`active` or `retired`)
- `first_seen_unix`
- `last_seen_unix`

Seat usage is `count(active bindings)`.

## Per-seat tunnel lifecycle

- Each paid seat gets one dedicated Cloudflared tunnel
- Tunnel is provisioned when a seat is assigned to a device
- The same tunnel is reused for that seat on reconnect and normal operation
- If a seat is retired and later reassigned, either:
  - reuse the seat's existing tunnel, or
  - reprovision only if tunnel health is invalid
- While subscription is active or in 30-day grace, keep seat tunnels available
- After grace expiry, deprovision tunnels for suspended subscriptions

## Checkout and Activation Flows

## Path A - Auto return from checkout (preferred)

1. User clicks `Get Pro` in app.
2. App opens Stripe Checkout.
3. User pays with email + card.
4. Stripe redirects to app callback.
5. App calls backend activation endpoint.
6. Backend verifies Stripe session and subscription status.
7. Backend binds current device (if seat available) and returns Pro entitlement.

## Path B - Email key fallback

1. User receives purchase email containing license key.
2. User enters key inside app.
3. App calls activation endpoint with key + device metadata.
4. Backend validates key + active subscription + seat capacity.
5. If valid, device is bound and Pro is enabled.

## Seat Full Behavior

When seat capacity is full and another device attempts activation:

- Show message: `All seats are in use`
- Offer action 1: `Retire an existing device`
- Offer action 2: `Add seats` (tiered pricing shown in-app)
- Offer action 3: `Contact Sales` for larger needs

## Seat Reassignment (Email OTP)

Because no password accounts exist, use OTP:

1. User opens management link and enters purchase email.
2. Backend sends one-time code or magic link.
3. User verifies OTP.
4. Management page shows active devices and seat usage.
5. User retires a device to free a seat.
6. User activates new device via app.

Security controls:

- OTP expiration (for example 10 minutes)
- Attempt limits and rate limits
- Audit log for request, verify, retire, bind

## Management Page Requirements

Management page must display:

- Subscription status
- Current seat quantity and monthly price
- Seats used / total
- License key
- Device list with:
  - `device_name`
  - `os`
  - `app_version`
  - `label`
  - first seen and last seen timestamps
  - status

Management actions:

- Increase seats
- Decrease seats
- Retire device
- Rename/edit device label
- Cancel subscription
- Open billing management

## Admin Control Plane (Private)

Add a separate admin interface and API for operator actions.

Security and hosting model:

- Hosted on a separate port from public website and customer APIs
- Bind address should support LAN-only or localhost-only operation
- Protected by one admin secret (single-operator model in v1)
- No multi-user admin account system in initial implementation
- Recommended hardening:
  - allowlist source IPs
  - TLS termination in front of admin port
  - rate limiting and lockout on failed logins
  - session timeout and CSRF protections

Admin capabilities:

- View platform-wide metrics and health:
  - active subscriptions
  - seats sold, seats assigned, seats available
  - active/grace/suspended breakdown
  - tunnel health and assignment counts
  - OTP delivery health and failures
- Manage any customer subscription:
  - cancel, resume, pause where supported
  - update seat quantity
  - override grace when needed
- Manage any key:
  - disable, enable, rotate, delete
  - view key metadata and usage context
- Manage any seat and device:
  - list all seats globally
  - unassign/reassign seats
  - retire device bindings
  - edit labels and metadata
  - force tunnel reprovision/deprovision
- Run maintenance actions:
  - trigger entitlement re-sync from Stripe
  - trigger seat/tunnel reconciler
  - clear stuck provisioning states
  - export diagnostics and audit reports

## Gift Seats and Time-Bound Entitlements

Admin UI must support gifting seats without requiring customer self-serve checkout.

Gift workflow:

1. Admin enters recipient email.
2. Admin selects seat quantity.
3. Admin sets gift duration (fixed end date or duration window).
4. System creates gift entitlement and sends recipient email.
5. Recipient uses OTP + key activation flow to claim and assign seats.

Gift rules:

- Gifts create explicit entitlement records separate from Stripe billing source
- Gifts must include start and end timestamps
- On gift expiry, gifted seats deactivate unless replaced by paid seats
- Gifts should be visible in both customer management view and admin audit logs

## Cancellation and Compliance Requirements

Cancellation must be easy and always available:

- App has a visible `Downgrade` path to unsubscribe
- Email receipts include `Manage subscription` and `Cancel subscription` links
- Management page includes cancellation action
- Confirmation screen and confirmation email are sent after cancel

Compliance intent:

- No dark patterns
- Clear renewal and cancellation disclosures
- Clear effective cancellation timing
- Maintain auditable records of billing and cancellation events

## Stripe Integration Plan

## Stripe objects

- Product: Pro
- Price: recurring monthly, unit amount = $20 per seat
- Subscription quantity controls seat count

## Stripe endpoints and events

Required backend routes:

- `POST /billing/create-checkout`
- `POST /billing/webhook`
- `POST /billing/create-portal-session`

Admin routes (private control plane):

- `POST /admin/auth/login`
- `POST /admin/auth/logout`
- `GET /admin/metrics`
- `POST /admin/subscriptions/{id}/update`
- `POST /admin/subscriptions/{id}/cancel`
- `POST /admin/licenses/{id}/disable`
- `POST /admin/licenses/{id}/enable`
- `POST /admin/licenses/{id}/rotate`
- `POST /admin/seats/{seat_id}/unassign`
- `POST /admin/seats/{seat_id}/reassign`
- `POST /admin/seats/gift`
- `POST /admin/maintenance/reconcile`

Compatibility notes (implemented backend parity):

- `POST /admin/auth/login` issues an admin session token (also returned as `llmhotspot_admin_session` cookie).
- `POST /admin/auth/logout` revokes the current admin session.
- Admin routes prefer session auth; emergency `x-admin-secret` fallback is only available when `LLMHOTSPOT_ADMIN_ALLOW_SECRET_FALLBACK=1`.
- Canonical gift route is `POST /admin/seats/gift`; `POST /admin/licenses/gift` remains as a compatibility alias and is deprecated.
- Admin actions are written to `audit_logs`; gift issuance is persisted in `gift_entitlements`.
- Runtime storage split:
  - `llmhotspot serve` remains local SQLite (self-contained app runtime).
  - `llmhotspot broker` uses Postgres by default (`LLMHOTSPOT_BROKER_DB_BACKEND=postgres` + `LLMHOTSPOT_BROKER_DATABASE_URL`).

Required webhook handling:

- `checkout.session.completed`
- `invoice.paid`
- `invoice.payment_failed`
- `customer.subscription.updated`
- `customer.subscription.deleted`

Webhook processing updates:

- subscription status
- quantity (seat cap)
- billing period timestamps
- entitlement active/inactive state

## App and Broker Authorization Checks

## Broker checks

For provisioning endpoints, verify:

- license exists
- subscription is active
- device binding is active and within seat cap

Provisioning behavior:

- On first seat assignment, provision and persist tunnel for that seat
- On subsequent requests for same seat, reuse persisted tunnel
- Avoid churn by not recreating tunnels when unnecessary

If not eligible:

- deny Pro provisioning
- return reason (`seat_full`, `subscription_inactive`, `key_invalid`, etc.)

## App checks

On activation and periodic refresh:

- validate entitlement state
- read seat limits
- enforce behavior in client UI

## Data Model (Logical)

Suggested tables:

- `customers`
- `subscriptions`
- `licenses`
- `device_bindings`
- `email_otp_challenges`
- `billing_events_audit`
- `admin_sessions`
- `gift_entitlements`
- `audit_logs` (admin actions + security events)
- `otp_delivery_events`

Email/OTP infrastructure requirements:

- OTP email sender integration (for example Resend, Postmark, SES, or SMTP relay)
- Branded transactional templates:
  - OTP verification
  - purchase confirmation with key
  - cancellation confirmation
  - gift entitlement invitation
- Delivery status tracking with retries and failure logging

## Messaging and Pricing Copy

Website messaging:

- Personal Pro: "Tiered pricing from $20 down to $10 per seat"
- "One key, as many seats as you buy"
- "Manage seats and devices with email verification"
- Enterprise: "Contact Sales for more than 100 seats"

## Implementation Phases

## Phase 1 - Billing foundation

- Stripe checkout route
- Stripe webhook ingestion
- subscription + license persistence
- post-purchase email with key and management link

## Phase 2 - Entitlement enforcement

- broker gating by entitlement
- seat cap enforcement
- app activation endpoints

## Phase 3 - Self-service management

- email OTP flow
- management page
- device retirement and labeling
- seat quantity updates
- cancel and downgrade wiring

## Phase 4 - Hardening and launch checks

- abuse/rate limiting
- audit logs and observability
- legal copy and policy review
- smoke tests for checkout, activation, cancel, downgrade, transfer
- admin port isolation and firewall validation
- admin auth brute-force protection tests
- OTP sender integration and deliverability checks

## Open Decisions To Finalize During Build

- Maximum automated seat decrease behavior when currently over-assigned
- Exact OTP delivery method (code vs magic link or both)
- Support response SLA and contact workflow for enterprise
