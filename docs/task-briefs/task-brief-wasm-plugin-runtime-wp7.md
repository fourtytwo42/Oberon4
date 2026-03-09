# Task Brief: WASM Plugin Runtime WP7

## Goal
Make plugin-runtime `dev` and `production` modes real, enforce production admission and stricter policy, and prove the runtime ship-safety story with conformance and threat-model-driven regressions.

## In Scope
- shared runtime config additions for operating mode and allowlist
- policy-contract extensions for admission and conformance diagnostics
- daemon registry conformance evaluation for ordinary plugins
- production gating for ordinary plugins
- stricter production budgets and approval defaults
- `tool_policy_inspect` diagnostics for mode/admission/conformance
- WP7 tests and governance/security artifacts

## Out of Scope
- new providers
- new plugin capabilities or tool families
- raw secret, raw network, or raw filesystem interfaces
- ecosystem-wide plugin certification beyond the host-run conformance signal

## Hard Constraints
- `dev` remains the default mode
- built-in host/brokered packages are host-trusted and exempt from ordinary-plugin allowlist blocking
- production allowlisting uses manifest provenance checksum, not plugin id alone
- ordinary plugins blocked by mode policy must fail closed across `tool_search`, `tool_describe`, `tool_plan`, and `tool_invoke`
- no diagnostics may leak raw host paths, secret material, or unsafe provenance internals

## Acceptance
- `EG-WP7` passes
- `FR-110`: planning/policy remain host-authoritative with explicit production gating
- `FR-113`: secret custody and redaction protections remain intact under production hardening
- `FR-114`: platform-neutral contracts stay unchanged while only host policy/adapters harden
- `AC-107.5`: resource governance remains enforced and is stricter in production mode
- `AC-109.5`: policy inspection exposes approval and operating-mode outcomes clearly
- `AC-113.5` and `AC-113.6`: cross-platform secret handling remains Oberon-native while production hardening does not leak backend details
- given a valid ordinary plugin in `dev`, when it is searched or described, then it remains available without allowlist enforcement
- given an ordinary plugin in `production` with missing allowlist or failing conformance, when the agent searches, describes, plans, or invokes it, then the host blocks it deterministically with machine-readable diagnostics
