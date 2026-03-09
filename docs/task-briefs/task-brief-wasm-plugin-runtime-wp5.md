# Task Brief: WASM Plugin Runtime WP5

## Goal
Implement the first concrete versioned workspace package for the Oberon WASM/WASI plugin runtime so agents can use brokered `workspace.*`, `git.*`, and `github.*` commands without raw host filesystem access, raw secret access, or a system `git` dependency.

## In Scope
- Register first-party `workspace`, `git`, and `github` built-ins in the daemon runtime.
- Implement brokered workspace read/write/patch command orchestration.
- Implement embedded host-managed git status/diff/branch/commit/push behavior.
- Implement brokered GitHub auth status, repo info, and clone-to-workspace behavior.
- Integrate these commands into search/describe/plan/invoke/policy-inspect.
- Add WP5 tests and update phase governance artifacts.

## Out of Scope
- Provider-specific prompt/tool adapter work from `WP6`.
- Production-mode hardening and release-gate tightening from `WP7`.
- Destructive git history operations.
- Generic raw network or raw secret-read interfaces.

## Hard Constraints
- Keep all workspace paths logical, relative, and brokered.
- Keep all GitHub auth host-custodied and redacted.
- Do not require a system `git` binary.
- Preserve the existing top-level host tool workflow.
- Fail closed on stale workspace revisions, blocked capabilities, and broker misuse.

## Acceptance
- `EG-WP5` passes with workspace, git, GitHub, and end-to-end versioned-workspace flows covered by tests.
- `FR-201`: versioned workspace editing remains revision-safe and conflict-aware.
- `FR-202`: git-style versioning is available without raw shell git dependency.
- `FR-203`: GitHub auth and repo operations stay brokered and host-custodied.
- `AC-201.2`: stale workspace writes fail with structured conflict behavior.
- `AC-202.5`: git package use does not require local user compilation or a system `git` install.
- `AC-202.6`: embedded git behavior remains behind host-managed broker boundaries.
- Given a user clones a GitHub repo to a logical workspace, when the agent edits a file, diffs it, commits it, and pushes it, then the runtime returns structured brokered results without exposing credentials.
