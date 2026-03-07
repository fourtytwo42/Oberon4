# Monetization Compatibility Contract

## Storage backend split

- Desktop app runtime (`llmhotspot serve`) stays self-contained on local SQLite.
- Server-side broker/website runtime (`llmhotspot broker`) uses Postgres by default.
- Broker emergency fallback to SQLite is available via `LLMHOTSPOT_BROKER_DB_BACKEND=sqlite`.

## Admin auth

- `POST /admin/auth/login` exchanges bootstrap secret for an admin session token.
- Session token is returned in JSON (`sessionId`) and as cookie `llmhotspot_admin_session`.
- `POST /admin/auth/logout` revokes the current session.
- `/admin/*` routes accept session auth by default.
- Emergency header-secret fallback is disabled by default and only enabled when `LLMHOTSPOT_ADMIN_ALLOW_SECRET_FALLBACK=1`.

## Gift route compatibility

- Canonical route: `POST /admin/seats/gift`.
- Compatibility alias: `POST /admin/licenses/gift` (same payload/response semantics).
- Alias is deprecated and should be migrated off by clients.

## Audit and entitlement invariants

- Structured billing webhook payload semantics are unchanged.
- License activation and seat-limit enforcement remain unchanged.
- Admin mutations append `audit_logs` records.
- Gift issuance appends `gift_entitlements` records tied to a generated gift id.
