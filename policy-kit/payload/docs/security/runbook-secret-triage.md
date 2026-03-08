# Secret Triage Runbook

Version: 0.1
Status: active

## Goal
Operate the Oberon4 secret scan as a blocking control while keeping existing reviewed debt quarantined in `security/gitleaks-baseline.json`.

## Rules
- Existing findings may be suppressed only through `security/gitleaks-baseline.json`.
- New findings outside the baseline are stop-ship.
- Do not add inline ignore comments in source files for this workflow.
- Baseline refreshes must be human-reviewed before commit.

## Triage Flow
1. Run `bash scripts/bootstrap-security-tools.sh`.
2. Run `bash scripts/oberon4-security-gate.sh`.
3. If `secret_scan` fails, inspect:
   - `artifacts/security-gate/<timestamp>-<pid>/gitleaks-report.json`
   - `artifacts/security-gate/<timestamp>-<pid>/secret_scan.log`
4. For each finding, classify it:
   - `real-secret-active`: rotate/revoke the credential and remove it from the repo state if possible.
   - `historical-secret-stale`: record why it is stale and keep it quarantined in the baseline until a history-rewrite decision is made.
   - `test-or-research-fixture`: keep it quarantined in the baseline with a rationale until the underlying content is cleaned or excluded by approved product scope changes.
5. Refresh the baseline only after the report is reviewed:
   - regenerate `security/gitleaks-baseline.json`
   - review the diff for only the intended fingerprints
6. Escalate to history rewrite planning when:
   - a credential is still active and repo history contains it
   - the same finding reappears after supposed cleanup
   - the baseline grows unexpectedly without a clear source

## Operator Notes
- `gitleaks` reports are machine-readable and should be treated as the canonical evidence for secret triage.
- `Research/` findings are still findings; this runbook does not exempt research content from review.
- Runtime artifact directories should stay ignored, but historical committed runtime artifacts must be triaged through the baseline until history cleanup is addressed.
