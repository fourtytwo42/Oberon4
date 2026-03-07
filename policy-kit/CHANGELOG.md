# Policy Kit Changelog

All notable changes to this policy kit are documented here.

## v1.4.0 - 2026-03-05

- Added locked scripted orchestration governance to payload `AGENTS.md`:
  - multi-tier role contracts (CEO/department/worker/verifier/reconciliator)
  - mandatory micro-job dispatch + end-gate enforcement
  - concern triage requirement before phase promotion
  - scripted control-plane lifecycle (`build -> reconcile -> verify -> fix -> advance`)
  - progress semantics for final completion ticks
- Added token-budget and parallelism defaults:
  - `maxParallelWorkers=5` baseline with adaptive reduction on failures/concern spikes
  - context-minimized worker prompting (`rg/sed/tail` reference-first)
- Added checkpoint and git hygiene rules:
  - batch/phase checkpoint cadence (`scripts/checkpoint.sh`)
  - runtime orchestration paths treated as non-product artifacts
  - push-only-at-safe-milestone guidance

## v1.3.0 - 2026-02-23

- Added agent incident model:
  - `docs/templates/agent-incident.md`
  - `docs/incidents/README.md`
- Added periodic drift tooling:
  - `scripts/install-drift-cron.sh`
  - `scripts/run-drift-watch.sh`
- Added Windows runner parity:
  - `scripts/run-agent.ps1`
- Added golden commit message examples:
  - `docs/examples/commit-messages/*`
- Added policy validator regression tests:
  - `tests/policy/*`
- Added policy stack health check:
  - `scripts/check-policy-stack.sh`
- Added strict subagent invocation enforcement:
  - subagents must use `run-agent.sh`
  - direct `codex exec` blocked for subagent commit evidence

## v1.2.0 - 2026-02-23

- Added anti-recursive multi-agent controls.
- Added guarded launcher (`scripts/run-agent.sh`).
- Added lineage validator (`scripts/validate-agent-lineage.sh`).
- Added run manifests (`.agent-runtime/<task-id>/runs/*.json`).
- Added lock timeout and force-unlock orchestration controls.

## v1.1.0 - 2026-02-23

- Added docs/policy extraction from root README.
- Added debug-brief enforcement gate.
- Expanded policy sync and payload portability.

## v1.0.0 - 2026-02-22

- Initial reusable policy-kit packaging.
- Bootstrap/update/payload workflow.
