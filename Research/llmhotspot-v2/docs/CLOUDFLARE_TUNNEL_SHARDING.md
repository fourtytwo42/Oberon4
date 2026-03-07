# Cloudflare Tunnel Sharding Plan

This note captures the agreed approach for scaling tunnel count while keeping a single brand domain.

## Goal

- Keep user-facing endpoints under `llmhotspot.com`.
- Scale beyond one Cloudflare account's tunnel/route limits.
- Preserve managed UX ("it just works"), without requiring users to self-host.

## Key Constraints

- A single DNS zone can only be active in one Cloudflare account at a time.
- Cloudflare account tunnel limits (default) apply per account.
- Splitting across accounts requires DNS delegation for sub-zones.

## Working Pattern

Use delegated sub-zones:

- `alpha.llmhotspot.com` -> Cloudflare account A
- `beta.llmhotspot.com` -> Cloudflare account B
- `gamma.llmhotspot.com` -> Cloudflare account C

Issue user endpoints inside each shard:

- `user1.alpha.llmhotspot.com`
- `user2.alpha.llmhotspot.com`
- `user1.beta.llmhotspot.com`

This keeps one root brand domain while distributing tunnel management across accounts.

## DNS Setup (Namecheap + Cloudflare)

Important distinction:

- Namecheap "Nameservers" fields are registrar-level and affect the entire root domain.
- Delegation of `alpha`/`beta` is done with NS records inside the root DNS zone.

### Steps

1. Put root zone `llmhotspot.com` in one primary DNS provider/account (Cloudflare or otherwise).
2. In Cloudflare account A, add zone `alpha.llmhotspot.com`.
3. In Cloudflare account B, add zone `beta.llmhotspot.com`.
4. In root zone `llmhotspot.com`, add NS records:
   - `alpha` -> nameserver 1 assigned to `alpha.llmhotspot.com`
   - `alpha` -> nameserver 2 assigned to `alpha.llmhotspot.com`
   - `beta` -> nameserver 1 assigned to `beta.llmhotspot.com`
   - `beta` -> nameserver 2 assigned to `beta.llmhotspot.com`
5. After propagation, manage records/tunnels for `*.alpha.llmhotspot.com` in account A, and `*.beta.llmhotspot.com` in account B.

Verify delegation:

```bash
dig NS alpha.llmhotspot.com
dig NS beta.llmhotspot.com
```

## Automation Outline

For each new tenant:

1. Choose shard (`alpha`, `beta`, etc.) based on capacity.
2. Create/assign tunnel in the shard account.
3. Create DNS record for `tenant.<shard>.llmhotspot.com`.
4. Save mapping in control plane:
   - tenant -> shard account -> tunnel id -> endpoint hostname

## Security Notes

- Do not reuse one tunnel credential across many unrelated tenants.
- Prefer per-tenant (or tightly scoped pooled) credentials.
- Rotate/revoke tunnel credentials when deprovisioning users.

## Product Direction Captured

- Free tier can remain local-only (no managed remote endpoint).
- Paid tier can include managed remote endpoint(s) using shard DNS + tunnel assignment.
