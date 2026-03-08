#!/usr/bin/env bash
set -euo pipefail

source scripts/lib/common.sh
load_hooks_env

max_files="${MAX_PUSH_CHANGED_FILES:-15}"
max_lines="${MAX_PUSH_CHANGED_LINES:-500}"
churn_escalation_files="${CHURN_ESCALATION_FILES:-10}"
churn_escalation_lines="${CHURN_ESCALATION_LINES:-300}"
repo="$(resolve_repo_slug || true)"
adr_required_path_regex="${ADR_REQUIRED_PATH_REGEX:-^apps/|^packages/|^src/|^infrastructure/|^api/|^docs/architecture/}"
change_request_required_path_regex="${CHANGE_REQUEST_REQUIRED_PATH_REGEX:-^apps/|^packages/|^src/|^api/|^infrastructure/migrations/|openapi|schema}"
run_policy_drift_audit="${HOOKS_RUN_POLICY_DRIFT_AUDIT:-1}"
run_exception_debt_report="${HOOKS_RUN_EXCEPTION_DEBT_REPORT:-1}"
exception_debt_strict="${HOOKS_EXCEPTION_DEBT_STRICT:-0}"
exception_debt_horizon_days="${HOOKS_EXCEPTION_DEBT_HORIZON_DAYS:-14}"
auto_sync_policy_kit_on_push="${HOOKS_AUTO_SYNC_POLICY_KIT_ON_PUSH:-0}"
policy_kit_sync_path_regex="${POLICY_KIT_SYNC_PATH_REGEX:-^AGENTS\.md$|^\.githooks/|^scripts/(validate-|check-policy-stack|bootstrap-policy-stack|bootstrap-security-tools|policy-doctor|risk-classifier|run-agent|oberon4-security-gate|security-gate|hooks\.env|lib/common\.sh|install-git-hooks|install-drift-cron|run-drift-watch|policy-drift-audit|exception-debt-report|local-ci-gate|sync-policy-kit)\.?(sh|ps1)?$|^docs/policy/|^docs/security/|^docs/templates/|^docs/examples/commit-messages/|^tests/policy/|^policy-kit/|^security/}"
did_run_security_gate=0
did_run_policy_drift_audit=0
did_run_exception_debt_report=0
did_sync_policy_kit=0
run_dedicated_security_gate_on_push="${HOOKS_RUN_DEDICATED_SECURITY_GATE_ON_PUSH:-0}"

validate_issue_open() {
  local issue_id="$1"
  local strict_issue_validation="${HOOKS_REQUIRE_GH_ISSUE_VALIDATION:-0}"

  if ! has_cmd gh; then
    if [[ "$strict_issue_validation" == "1" ]]; then
      echo "Push blocked: GitHub CLI (gh) is required for issue-state validation." >&2
      return 1
    fi
    echo "Warning: gh not found; skipping issue-state validation." >&2
    return 0
  fi

  if [[ -z "${repo//[[:space:]]/}" ]]; then
    if [[ "$strict_issue_validation" == "1" ]]; then
      echo "Push blocked: repository slug could not be resolved for issue-state validation." >&2
      return 1
    fi
    echo "Warning: repository slug not resolved; skipping issue-state validation." >&2
    return 0
  fi

  local state
  if ! state="$(gh api "repos/${repo}/issues/${issue_id}" --jq '.state' 2>/dev/null)"; then
    echo "Push blocked: unable to resolve issue #${issue_id} in ${repo}." >&2
    return 1
  fi

  if [[ "$state" != "open" ]]; then
    echo "Push blocked: issue #${issue_id} is '${state}', expected 'open'." >&2
    return 1
  fi
}

validate_task_briefs_in_range() {
  local range="$1"
  local err=0
  local file

  while IFS= read -r file; do
    [[ -z "$file" ]] && continue
    [[ "$file" == "docs/templates/task-brief.md" ]] && continue
    if ! scripts/validate-task-brief.sh "$file"; then
      err=1
    fi
  done < <(git diff --name-only "$range" | grep -E 'task-brief.*\.md$|^docs/task-briefs/.*\.md$' || true)

  return "$err"
}

validate_specs_in_range() {
  local range="$1"
  local err=0
  local file

  while IFS= read -r file; do
    [[ -z "$file" ]] && continue
    [[ ! -f "$file" ]] && continue
    if ! scripts/validate-spec-quality.sh "$file"; then
      err=1
    fi
  done < <(git diff --name-only "$range" | grep -E '(^docs/task-briefs/.*\.md$|task-brief.*\.md$|(^|/)spec\.md$|(^|/)specification.*\.md$)' || true)

  return "$err"
}

validate_adr_gate_in_range() {
  local range="$1"
  local changed
  changed="$(git diff --name-only "$range")"

  if grep -Eq "$adr_required_path_regex" <<<"$changed"; then
    if ! grep -Eq '^docs/adr/[0-9]{4}-.*\.md$' <<<"$changed"; then
      echo "Push blocked: architecture-impact changes detected but no ADR update found." >&2
      echo "Add/update docs/adr/NNNN-<title>.md in the same push." >&2
      exit 1
    fi
  fi
}

require_plan_artifact_for_medium_high_risk() {
  local range="$1"
  local logs
  logs="$(git log --format=%B "$range")"

  if grep -Eiq '^Risk:[[:space:]]*(medium|high)[[:space:]]*$' <<<"$logs"; then
    local changed
    changed="$(git diff --name-only "$range")"

    if grep -Eq '^docs/plans/plan-.*\.md$' <<<"$changed"; then
      return 0
    fi

    # Allow explicit reference to an existing plan artifact.
    local plan_path
    plan_path="$(grep -Eio '^Plan:[[:space:]]*docs/plans/[^[:space:]]+\.md[[:space:]]*$' <<<"$logs" | sed -E 's/^Plan:[[:space:]]*//I' | tail -n1)"
    if [[ -n "${plan_path//[[:space:]]/}" && -f "$plan_path" ]]; then
      return 0
    fi

    echo "Push blocked: Risk medium/high requires a plan artifact." >&2
    echo "Add docs/plans/plan-<topic>.md in this push or reference existing plan with 'Plan: docs/plans/<file>.md'." >&2
    exit 1
  fi
}

require_change_request_for_behavior_or_contract_changes() {
  local range="$1"
  local changed
  changed="$(git diff --name-only "$range")"

  if grep -Eq "$change_request_required_path_regex" <<<"$changed"; then
    if ! grep -Eq '^docs/change-requests/.*\.md$' <<<"$changed"; then
      echo "Push blocked: behavior/contract-impact changes require a change request document." >&2
      echo "Add docs/change-requests/<name>.md in the same push." >&2
      exit 1
    fi
  fi
}

require_enforcement_artifact_when_rules_change() {
  local range="$1"
  local changed
  changed="$(git diff --name-only "$range")"

  if grep -Eq '^AGENTS\.md$' <<<"$changed"; then
    if ! grep -Eq '^scripts/(validate-.*|security-gate\.sh|install-git-hooks\.sh)$|^\.githooks/|^docs/templates/|^\.github/pull_request_template\.md$' <<<"$changed"; then
      echo "Push blocked: AGENTS.md changed without enforcement artifact updates." >&2
      echo "Update at least one validator/hook/template in the same push." >&2
      exit 1
    fi
  fi
}

require_enforcement_artifact_when_debug_brief_changes() {
  local range="$1"
  local changed
  changed="$(git diff --name-only "$range")"

  if grep -Eq '^docs/debug-brief\.md$' <<<"$changed"; then
    if ! grep -Eq '^scripts/(validate-.*|security-gate\.sh|install-git-hooks\.sh)$|^\.githooks/|^docs/templates/|^\.github/pull_request_template\.md$' <<<"$changed"; then
      echo "Push blocked: docs/debug-brief.md changed without template/validator enforcement updates." >&2
      echo "Update at least one validator/hook/template in the same push." >&2
      exit 1
    fi
  fi
}

enforce_churn_escalation_rules() {
  local range="$1"
  local changed_files="$2"
  local changed_lines="$3"
  local logs
  logs="$(git log --format=%B "$range")"

  if (( changed_files >= churn_escalation_files || changed_lines >= churn_escalation_lines )); then
    if ! grep -Eiq '^Risk:[[:space:]]*high[[:space:]]*$' <<<"$logs"; then
      echo "Push blocked: churn escalation triggered (${changed_files} files, ${changed_lines} lines)." >&2
      echo "Require at least one commit with 'Risk: high' for high-churn pushes." >&2
      exit 1
    fi

    cmd_count="$(grep -Ec '^[[:space:]]*\$[[:space:]]+.+$' <<<"$logs" || true)"
    if (( cmd_count < 2 )); then
      echo "Push blocked: churn escalation requires at least 2 verification commands in commit messages." >&2
      exit 1
    fi

    if ! grep -Eq '^Rollback:[[:space:]]*.+$' <<<"$logs"; then
      echo "Push blocked: churn escalation requires explicit rollback notes in commit message." >&2
      exit 1
    fi
  fi
}

enforce_no_hidden_magic_docs() {
  local range="$1"
  local changed
  changed="$(git diff --name-only "$range")"

  if grep -Eq '^\.githooks/|^scripts/(validate-|install-git-hooks|hooks\.env|lib/common\.sh)' <<<"$changed"; then
    if ! grep -Eq '^README\.md$|^docs/policy/local-enforcement\.md$|^policy-kit/README\.md$' <<<"$changed"; then
      echo "Push blocked: automation behavior changed without README update (No Hidden Magic gate)." >&2
      echo "Update README.md, docs/policy/local-enforcement.md, or policy-kit/README.md with trigger/check/bypass notes." >&2
      exit 1
    fi
  fi
}

enforce_phase_checkpoint_artifact() {
  local range="$1"
  local logs
  logs="$(git log --format=%B "$range")"
  local changed
  changed="$(git diff --name-only "$range")"

  if grep -Eiq '^Risk:[[:space:]]*high[[:space:]]*$|^Override:[[:space:]]*large-change[[:space:]]*$' <<<"$logs"; then
    if ! grep -Eq '^docs/handoffs/handoff-.*\.md$' <<<"$changed"; then
      echo "Push blocked: high-risk/large-change work requires checkpoint artifact update." >&2
      echo "Add/update docs/handoffs/handoff-<phase>.md in the same push." >&2
      exit 1
    fi
  fi
}

maybe_sync_policy_kit() {
  local range="$1"
  local changed
  changed="$(git diff --name-only "$range")"

  if [[ "$auto_sync_policy_kit_on_push" != "1" ]]; then
    return 0
  fi

  if [[ "$did_sync_policy_kit" -eq 1 ]]; then
    return 0
  fi

  if ! grep -Eq "$policy_kit_sync_path_regex" <<<"$changed"; then
    return 0
  fi

  echo "Policy kit auto-sync: syncing BOS policy stack to standalone policy kit repo..."
  bash scripts/sync-policy-kit.sh --push
  did_sync_policy_kit=1
}

while read -r local_ref local_sha remote_ref remote_sha; do
  [[ -z "${local_ref:-}" ]] && continue

  range=""
  if [[ "$remote_sha" =~ ^0+$ ]]; then
    range="$local_sha"
  else
    range="$remote_sha..$local_sha"
  fi

  changed_files="$(git diff --name-only "$range" | wc -l | tr -d ' ')"
  changed_lines="$(git diff --numstat "$range" | awk '{a+=$1; d+=$2} END {print a+d+0}')"

  if (( changed_files > max_files || changed_lines > max_lines )); then
    # Allow override with repo-scoped label in any commit message in range.
    if ! git log --format=%B "$range" | grep -Eiq '^Override:[[:space:]]*large-change[[:space:]]*$'; then
      echo "Push blocked: change set too large (${changed_files} files, ${changed_lines} lines)." >&2
      echo "Limits: ${max_files} files, ${max_lines} lines." >&2
      echo "Split into smaller changes or add 'Override: large-change' in a commit message." >&2
      exit 1
    fi
  fi

  # Validate linked issues from commit messages are open in this repo.
  issue_url_regex="$(issue_url_regex_for_repo "$repo")"
  issue_ids="$(
    git log --format=%B "$range" \
      | grep -Eio "^Issue:[[:space:]]*(#([0-9]+)|${issue_url_regex})" \
      | grep -Eo '[0-9]+$' \
      | sort -u || true
  )"
  if [[ -z "${issue_ids//[[:space:]]/}" ]]; then
    echo "Push blocked: no Issue references found in commit messages for range ${range}." >&2
    exit 1
  fi
  while IFS= read -r issue_id; do
    [[ -z "$issue_id" ]] && continue
    validate_issue_open "$issue_id" || exit 1
  done <<<"$issue_ids"

  # Validate task brief files touched in this range.
  validate_task_briefs_in_range "$range" || exit 1
  validate_specs_in_range "$range" || exit 1
  scripts/validate-agent-lineage.sh "$range"
  validate_adr_gate_in_range "$range"
  require_plan_artifact_for_medium_high_risk "$range"
  require_change_request_for_behavior_or_contract_changes "$range"
  require_enforcement_artifact_when_rules_change "$range"
  require_enforcement_artifact_when_debug_brief_changes "$range"
  enforce_churn_escalation_rules "$range" "$changed_files" "$changed_lines"
  enforce_no_hidden_magic_docs "$range"
  enforce_phase_checkpoint_artifact "$range"
  maybe_sync_policy_kit "$range"

  if [[ "$did_run_policy_drift_audit" -eq 0 && "$run_policy_drift_audit" == "1" ]]; then
    bash scripts/policy-drift-audit.sh
    did_run_policy_drift_audit=1
  fi

  if [[ "$did_run_exception_debt_report" -eq 0 && "$run_exception_debt_report" == "1" ]]; then
    bash scripts/exception-debt-report.sh "$exception_debt_horizon_days" "$exception_debt_strict"
    did_run_exception_debt_report=1
  fi

  # Local CI safety net: smoke + tests + build.
  bash scripts/local-ci-gate.sh

  if [[ "$run_dedicated_security_gate_on_push" == "1" && "$did_run_security_gate" -eq 0 ]]; then
    scripts/security-gate.sh .
    did_run_security_gate=1
  fi
done

exit 0
