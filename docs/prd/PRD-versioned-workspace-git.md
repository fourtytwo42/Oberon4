# PRD: Oberon4 Versioned Workspace and Git Integration

Version: 0.1
Owner: Oberon4
Status: draft

Companion specifications:
- `docs/prd/PRD-wasm-tool-runtime.md`
- `docs/prd/architecture-wasm-tool-runtime.md`
- `docs/prd/tool-interaction-contract.md`
- `docs/prd/plugin-runtime-contract.md`

## 1. Problem Statement
Define the first concrete tool package on top of the Oberon4 WASM/WASI tool runtime: a versioned workspace surface that lets agents read, edit, diff, commit, and optionally push changes through a safe host-managed workspace plus git and GitHub integration model.

The system must give agents an editable, versioned surface without exposing the outer host filesystem, raw git shell access, or raw GitHub credentials.

## 2. Product Shape
This capability is not one monolithic plugin. It should be modeled as a coordinated tool package:
- `workspace` surface for logical file access and edits
- `git` surface for version control operations on a granted workspace
- `github` surface for authentication, remote operations, and collaboration metadata

Recommended tiering:
- `workspace.*`: Tier 2 brokered privileged service
- `git.*`: Tier 2 brokered privileged service
- `github.*`: Tier 2 brokered privileged service
- optional helper plugins:
  - diff summarizers
  - patch analyzers
  - repo search/index plugins

Packaging rule:
- users should be able to install Oberon, add the package, and use it without compiling local code or installing a separate git toolchain
- the preferred delivery model is prebuilt plugin artifacts plus host runtime support
- git logic may be embedded in the plugin artifact, but trust boundaries remain host-managed

## 3. Goals
- `FR-201`: safe versioned editing surface for agents through logical workspaces, not raw host paths
- `FR-202`: host-brokered git operations for diff, branch, commit, and push
- `FR-203`: host-custodied GitHub authentication and remote integration without raw token exposure
- `FR-204`: side-effect-aware planning and approval model for file edits and git operations
- `FR-205`: auditability, rollback safety, and deterministic versioned outcomes

## 4. Non-Goals (Initial Slice)
- raw shell `git` execution by agents or plugins
- arbitrary host directory access
- direct plugin access to `~/.gitconfig`, SSH keys, or host credential stores
- destructive history rewrite by default
- force push by default
- direct arbitrary network egress from ordinary plugins
- full GitHub repository administration
- generalized multi-repo orchestration in the initial slice

## 5. Functional Requirements
- `FR-201` Versioned workspace surface
  - `AC-201.1`: the host can create or attach a logical workspace sandbox backed by versioned storage.
  - `AC-201.2`: agents and plugins operate on the workspace through a `workspace_handle`, not raw host paths.
  - `AC-201.3`: the workspace surface supports safe read, list, stat, and edit-oriented operations using normalized relative paths only.
  - `AC-201.4`: workspace edits remain contained within the logical workspace scope and cannot escape through traversal, symlink, or platform-native path tricks.
  - `AC-201.5`: the host may grant multiple plugins access to the same logical workspace through separate host-issued handles.
  - `AC-201.6`: shared-workspace edits must use revision identifiers, preconditions, or equivalent conflict controls so plugin actions do not silently overwrite each other.

- `FR-202` Git-backed version control surface
  - `AC-202.1`: the host exposes brokered git operations bound to a granted `workspace_handle`.
  - `AC-202.2`: the initial git command surface should support at least:
    - `git.status`
    - `git.diff`
    - `git.branch.create`
    - `git.commit`
    - `git.push`
  - `AC-202.3`: git operations return structured results, not raw terminal output as the primary contract.
  - `AC-202.4`: git operations must not require the agent to invoke a shell or manage OS-specific git installation details.
  - `AC-202.5`: the git implementation may be embedded inside the shipped plugin artifact so end users do not need a separately installed git binary.
  - `AC-202.6`: if git logic is embedded in the plugin artifact, the host still owns workspace handles, approvals, quotas, audit, credential use, and remote policy enforcement.

- `FR-203` GitHub integration surface
  - `AC-203.1`: users can authenticate GitHub access through Oberon-managed login flows.
  - `AC-203.2`: GitHub credentials remain host-custodied and are never returned to agents or ordinary plugins as raw values.
  - `AC-203.3`: the host can associate a workspace and git remote with an authenticated GitHub identity or service handle.
  - `AC-203.4`: GitHub-backed actions such as clone, remote sync, and push use brokered authenticated operations.

- `FR-204` Planning and approval
  - `AC-204.1`: state-changing workspace edits should support a plan or preview path before mutation.
  - `AC-204.2`: `git.commit` and `git.push` should expose approval expectations and side-effect metadata through describe or plan results.
  - `AC-204.3`: the host can require approval for push and other externally visible write actions.
  - `AC-204.4`: edit and git operations should expose precondition failure clearly when workspace state has changed since planning.

- `FR-205` Auditability and rollback safety
  - `AC-205.1`: every workspace edit, git action, and GitHub action should be associated with structured request and audit identifiers.
  - `AC-205.2`: the host can surface commit ids, branch names, diff summaries, and push outcomes as structured metadata.
  - `AC-205.3`: rollback-relevant metadata such as pre-edit hashes, commit ids, and touched logical paths should be preserved for diagnostics and recovery.
  - `AC-205.4`: user-visible outcomes should clearly distinguish:
    - workspace edited but not committed
    - committed locally
    - pushed remotely

## 6. Recommended Agent-Facing Tool Surface
Recommended commands for the initial slice:
- `workspace.read_file`
- `workspace.list_dir`
- `workspace.stat_path`
- `workspace.write_file_atomic`
- `workspace.apply_patch`
- `git.status`
- `git.diff`
- `git.branch.create`
- `git.commit`
- `git.push`
- `github.auth_status`
- `github.clone_to_workspace`
- `github.repo_info`

Design rule:
- file content changes should happen through `workspace.*`
- versioning should happen through `git.*`
- remote identity and collaboration should happen through `github.*`
- multiple plugins may work on the same workspace, but coordination must happen through host-issued handles and shared-workspace revision controls

## 7. Security and Containment Model
- no raw outer host filesystem access
- no raw git shell access
- no raw GitHub token access
- all workspace operations use normalized logical relative paths
- all git operations are brokered by the host against a granted workspace
- all GitHub operations are brokered by the host against a granted service handle
- workspace writes should be atomic by default
- git push should require policy approval by default
- force push and destructive reset should be out of scope for the initial slice

Trust-boundary rule:
- git object and repository logic may live inside the plugin package
- workspace isolation, remote auth, network policy, and credential use must remain host-brokered
- the plugin may be self-contained for logic, but it must not become the trust anchor

## 8. Storage and Filesystem Model
Workspace rules:
- the workspace sandbox is the editable surface presented to the agent
- the outer host path remains hidden
- the host may back the workspace with a real git repository internally
- the contract stays logical and portable regardless of Linux, macOS, or Windows backing details

Host enforcement rules:
- reject absolute paths
- reject parent-directory traversal
- prevent symlink or reparse-point escape
- enforce maximum file size
- enforce maximum files touched per invocation
- enforce overall workspace or session quotas
- preserve workspace revision or snapshot metadata for shared-workspace conflict detection

## 9. Git and GitHub Auth Model
Recommended auth model:
1. user authenticates GitHub through Oberon
2. Oberon stores the credential in the host secret store
3. Oberon issues a `service_handle` for allowed GitHub operations
4. `git.push` or `github.*` calls resolve credentials inside the host broker
5. the agent receives only sanitized structured results

Avoid:
- exposing `GITHUB_TOKEN`
- injecting raw tokens into plugin environments
- requiring the agent to manage git credentials directly

Install-and-go rule:
- users should not need to compile plugins locally
- users should not need to install a system git binary for the preferred packaged experience
- packaged git functionality should be delivered as prebuilt plugin artifacts loaded by Oberon

Preferred implementation split:
- embedded in plugin:
  - repo status logic
  - diff generation
  - commit construction
  - branch or ref logic where practical
- retained in host:
  - workspace sandbox control
  - service-handle issuance
  - GitHub credential brokerage
  - remote transport policy
  - approvals, quotas, and audit logging

## 10. Result and State Model
Workspace edit results should include:
- `workspaceHandle`
- `workspaceRevision`
- `logicalPath`
- `operation`
- `bytesWritten` or patch outcome
- `preconditionStatus`
- `touched`

Git results should include:
- `workspaceHandle`
- `workspaceRevision`
- `branch`
- `headCommit`
- `dirty`
- `filesChanged`
- `diffArtifact` when applicable

Push results should include:
- `remote`
- `branch`
- `commit`
- `pushed`
- `provider`
- `remoteRef`

## 11. Side-Effect and Approval Defaults
Recommended defaults:
- `workspace.read_*` -> `none`
- `workspace.write_*` -> `external-write`
- `workspace.apply_patch` -> `external-write`
- `git.status` / `git.diff` -> `none`
- `git.branch.create` -> `plugin-private-write` or policy-defined local-write class
- `git.commit` -> `external-write`
- `git.push` -> `external-write` with approval required by default
- GitHub remote mutations -> `external-write` with approval required by default

## 12. Observability and Audit
Required identifiers:
- `requestId`
- `sessionId`
- `workspaceHandle`
- `serviceHandle` when applicable
- `gitCommitId` when applicable
- `provider`

Required audit events:
- workspace attached or created
- file written or patch applied
- git diff requested
- commit created
- push attempted
- push succeeded or failed
- GitHub auth used through a broker

## 13. Verification Requirements
Minimum verification scope for implementation:
- unit tests for path normalization and containment
- contract tests for workspace and git result envelopes
- security tests for traversal and symlink escape rejection
- security tests for secret non-exposure
- integration tests for workspace edit -> diff -> commit flow
- integration tests for commit -> push flow with brokered auth
- cross-platform tests for logical path behavior on Linux, macOS, and Windows
- integration tests for multi-plugin shared-workspace edit conflict detection

## 14. Open Questions
- whether clone should be part of `github.*` only or also exposed through a `workspace.create_from_repo` helper
- whether local branch creation should require explicit approval in stricter policy modes
- whether commit author identity should be fully host-managed or partly user-configurable
- whether pull, fetch, and merge should be part of the initial slice or a follow-on slice
- whether PR creation belongs in the initial slice or a GitHub collaboration follow-up PRD
- whether the first implementation should use a fully embedded git engine in the plugin immediately, or allow a temporary host adapter fallback behind the same contract during bootstrap
