#!/usr/bin/env bash
set -euo pipefail

source scripts/lib/common.sh
load_hooks_env

usage() {
  cat <<'USAGE'
Usage: scripts/policy-doctor.sh [--msg <path>] [--commit-only|--push-only] [--no-classifier]

Runs local policy validators in doctor mode and prints actionable failures before hook execution.
Defaults:
- commit check enabled
- push check enabled
- commit message path: .git/COMMIT_EDITMSG (if present)
USAGE
}

msg_file=".git/COMMIT_EDITMSG"
run_commit=1
run_push=1
run_classifier=1

while [[ $# -gt 0 ]]; do
  case "$1" in
    --msg)
      msg_file="${2:-}"
      [[ -n "$msg_file" ]] || { echo "--msg requires value" >&2; exit 2; }
      shift 2
      ;;
    --commit-only)
      run_commit=1
      run_push=0
      shift
      ;;
    --push-only)
      run_commit=0
      run_push=1
      shift
      ;;
    --no-classifier)
      run_classifier=0
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown arg: $1" >&2
      usage
      exit 2
      ;;
  esac
done

status=0

print_header() {
  printf '\n[%s]\n' "$1"
}

if [[ "$run_classifier" == "1" ]]; then
  print_header "risk-classifier"
  if git rev-parse --git-dir >/dev/null 2>&1; then
    if git diff --cached --quiet; then
      echo "No staged changes. Classifier skipped."
    else
      scripts/risk-classifier.sh --cached || true
    fi
  else
    echo "Not in git repo. Classifier skipped."
  fi
fi

if [[ "$run_commit" == "1" ]]; then
  print_header "commit-msg validator"
  if [[ ! -f "$msg_file" ]]; then
    echo "Commit message file not found: $msg_file"
    echo "Use --msg <path> to validate a specific message file."
    status=1
  else
    tmp_out="$(mktemp)"
    if scripts/validate-commit-message.sh "$msg_file" >"$tmp_out" 2>&1; then
      echo "PASS: commit message satisfies policy."
    else
      echo "FAIL: commit message violates policy."
      cat "$tmp_out"
      status=1
    fi
    rm -f "$tmp_out"
  fi
fi

if [[ "$run_push" == "1" ]]; then
  print_header "pre-push validator"
  branch="$(git rev-parse --abbrev-ref HEAD 2>/dev/null || true)"
  if [[ -z "$branch" ]]; then
    echo "Unable to resolve current branch for push validation."
    status=1
  else
    local_ref="refs/heads/$branch"
    local_sha="$(git rev-parse HEAD)"
    upstream="$(git rev-parse --abbrev-ref --symbolic-full-name @{upstream} 2>/dev/null || true)"

    if [[ -n "$upstream" ]]; then
      remote_sha="$(git rev-parse @{upstream})"
      remote_branch="${upstream#*/}"
      remote_ref="refs/heads/${remote_branch}"
    else
      remote_sha="0000000000000000000000000000000000000000"
      remote_ref="refs/heads/$branch"
      echo "No upstream configured; validating as first push for branch '$branch'."
    fi

    tmp_out="$(mktemp)"
    if printf '%s %s %s %s\n' "$local_ref" "$local_sha" "$remote_ref" "$remote_sha" | scripts/validate-push.sh >"$tmp_out" 2>&1; then
      echo "PASS: push range satisfies policy gates."
    else
      echo "FAIL: push range violates policy gates."
      cat "$tmp_out"
      status=1
    fi
    rm -f "$tmp_out"
  fi
fi

if [[ "$status" == "0" ]]; then
  echo
  echo "Policy doctor: all requested checks passed."
else
  echo
  echo "Policy doctor: one or more checks failed."
fi

exit "$status"
