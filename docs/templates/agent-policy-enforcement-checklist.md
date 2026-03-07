# Agent Policy Enforcement Checklist

Use this checklist when AGENTS.md or orchestration policy behavior changes:

- Update at least one enforcement artifact (`scripts/validate-*`, `.githooks/*`, or `docs/templates/*`).
- Confirm push hooks still pass on a clean branch.
- Confirm rollback instructions are present in latest handoff.
