# Security Checklist: WASM Plugin Runtime WP7

ThreatModel: OWASP + STRIDE
UntrustedInputBoundary: plugin manifests, plugin output, provider content, broker request input
PIIHandling: unchanged
RetentionImpact: unchanged
LogRedaction: host redaction pipeline remains mandatory

## Abuse Cases Reviewed
- non-allowlisted ordinary plugin attempts execution in production
- conformance-failed ordinary plugin attempts execution in production
- plugin output attempts to bypass production-mode admission by using different host tools
- plugin or broker failure path attempts to leak secret or path material through diagnostics
- confused-deputy service use attempts to bypass admission and policy gates

## Controls Verified
- mode-aware admission enforced before `describe`, `plan`, and `invoke`
- `tool_search` filters production-blocked ordinary plugins from agent discovery
- production allowlist is checksum-based and explicit
- conformance failures are recorded and block ordinary plugins in production
- production keeps stricter budgets and approvals than dev
- built-in brokered packages remain trusted without expanding ordinary-plugin authority
- existing redaction and secret-broker protections remain active in both modes
- workspace/path containment protections remain covered by regression tests

## Verification Evidence
- daemon policy/admission tests in [plugin_runtime.rs](/home/hendo420/Oberon4/crates/oberon4-daemon/src/plugin_runtime.rs)
- execution regression tests in [plugin_runtime_execution.rs](/home/hendo420/Oberon4/crates/oberon4-daemon/tests/plugin_runtime_execution.rs)
- threat baseline in [threat-model-wasm-plugin-runtime.md](/home/hendo420/Oberon4/docs/security/threat-model-wasm-plugin-runtime.md)

## Residual Risk
- `dev` mode remains intentionally permissive for local development
- production trust still depends on operator-maintained checksum allowlists
- conformance is host-run and bounded to the runtime checks implemented in this repo; stronger external signing/certification can still be added later
