# Security Exception: Wasmtime WP2 Advisory Hold

Status: active
Owner: Oberon4
Linked Issue: `#1`
ExceptionExpires: 2026-04-30

## Scope
- `RUSTSEC-2025-0118`
- `RUSTSEC-2026-0006`
- `RUSTSEC-2026-0020`
- `RUSTSEC-2026-0021`
- dependency: `wasmtime 34.0.2`
- dependency: `wasmtime-wasi 34.0.2`

## Reason
- `WP2` requires Wasmtime Preview 2/component-model support that still fits the repo's Rust `1.85.x` toolchain.
- The first patched Wasmtime line (`36.0.6`) requires Rust `1.86.0` and triggered a full dependency rebuild that exhausted the current local disk budget during verification.
- `WP2` ordinary plugins are restricted to private/session/cache storage and deny raw network, brokered services, workspace handles, and artifacts, which reduces the immediately reachable surface relative to the broader upstream advisories.

## Follow-Up
- upgrade the runtime to a patched Wasmtime line once the repo toolchain and CI/storage budgets can support that move
- remove the `audit.toml` and `deny.toml` ignore entries in the same change that upgrades Wasmtime
- rerun:
  - `bash scripts/oberon4-security-gate.sh`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`

## Residual Risk
- upstream advisories remain present in the pinned Wasmtime line
- this exception is time-bound and should not roll forward silently beyond the expiry date
