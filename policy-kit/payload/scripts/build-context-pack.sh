#!/usr/bin/env bash
set -euo pipefail
source scripts/lib/common.sh
load_hooks_env

base="${1:-HEAD~1}"
head="${2:-HEAD}"
out="${CONTEXT_PACK_OUT:-docs/context-pack.md}"
token_budget="${CONTEXT_TOKEN_BUDGET:-8000}"
warn_pct="${CONTEXT_TOKEN_WARN_PCT:-70}"
strict_budget="${CONTEXT_BUDGET_STRICT:-0}"

if ! git rev-parse --verify "$head" >/dev/null 2>&1; then
  echo "Invalid head ref: $head" >&2
  exit 1
fi

if ! git rev-parse --verify "$base" >/dev/null 2>&1; then
  base="$(git rev-list --max-parents=0 "$head" | tail -n1)"
fi

range="${base}..${head}"

changed_files="$(git diff --name-only "$range")"

# Build related-file candidates from import statements in changed files.
related_tmp="$(mktemp)"
while IFS= read -r file; do
  [[ -z "$file" ]] && continue
  [[ ! -f "$file" ]] && continue
  case "$file" in
    *.js|*.jsx|*.ts|*.tsx|*.mjs|*.cjs)
      if has_cmd rg; then
        rg -No "from ['\"][^'\"]+['\"]|require\(['\"][^'\"]+['\"]\)" "$file" \
          | sed -E "s/^.*['\"]([^'\"]+)['\"].*$/\1/" \
          | rg -v '^(node:|[a-zA-Z0-9_-]+$)' \
          >> "$related_tmp" || true
      else
        grep -Eo "from ['\"][^'\"]+['\"]|require\(['\"][^'\"]+['\"]\)" "$file" \
          | sed -E "s/^.*['\"]([^'\"]+)['\"].*$/\1/" \
          | grep -Ev '^(node:|[a-zA-Z0-9_-]+$)' \
          >> "$related_tmp" || true
      fi
      ;;
    *.py)
      if has_cmd rg; then
        rg -No "^\s*(from\s+[a-zA-Z0-9_\.]+\s+import|import\s+[a-zA-Z0-9_\.]+)" "$file" \
          | sed -E 's/^\s*from\s+([^ ]+)\s+import.*$/\1/; s/^\s*import\s+([^ ]+).*$/\1/' \
          >> "$related_tmp" || true
      else
        grep -Eo "^\s*(from\s+[a-zA-Z0-9_\.]+\s+import|import\s+[a-zA-Z0-9_\.]+)" "$file" \
          | sed -E 's/^\s*from\s+([^ ]+)\s+import.*$/\1/; s/^\s*import\s+([^ ]+).*$/\1/' \
          >> "$related_tmp" || true
      fi
      ;;
  esac
done <<<"$changed_files"

sort -u "$related_tmp" -o "$related_tmp"

# Estimate context token load from changed-file content footprint.
changed_char_count=0
changed_file_count=0
while IFS= read -r file; do
  [[ -z "$file" ]] && continue
  [[ ! -f "$file" ]] && continue
  # Count bytes as a practical proxy for chars in ASCII-heavy repos.
  bytes="$(wc -c < "$file" | tr -d ' ')"
  changed_char_count=$((changed_char_count + bytes))
  changed_file_count=$((changed_file_count + 1))
done <<<"$changed_files"

estimated_tokens=$((changed_char_count / 4))
warn_tokens=$((token_budget * warn_pct / 100))
status="ok"
if (( estimated_tokens >= token_budget )); then
  status="over_budget"
elif (( estimated_tokens >= warn_tokens )); then
  status="warning"
fi

mkdir -p "$(dirname "$out")"
{
  echo "# Context Pack"
  echo
  echo "## Goal"
  echo "- Fill this before use."
  echo
  echo "## Constraints"
  echo "- Reference: AGENTS.md"
  echo
  echo "## Canonical Context (Stable Prefix)"
  [[ -f AGENTS.md ]] && echo "- AGENTS.md"
  [[ -f README.md ]] && echo "- README.md"
  echo
  echo "## Changed Files (${range})"
  if [[ -n "${changed_files//[[:space:]]/}" ]]; then
    while IFS= read -r f; do
      [[ -n "$f" ]] && echo "- $f"
    done <<<"$changed_files"
  else
    echo "- (none)"
  fi
  echo
  echo "## Related Imports/Modules (Heuristic)"
  if [[ -s "$related_tmp" ]]; then
    while IFS= read -r r; do
      [[ -n "$r" ]] && echo "- $r"
    done < "$related_tmp"
  else
    echo "- (none detected)"
  fi
  echo
  echo "## Acceptance IDs in Scope"
  echo "- FR-###"
  echo "- AC-###"
  echo
  echo "## Verification Plan"
  echo "- $ <command>"
  echo "- Output: <expected outcome>"
  echo
  echo "## Context Budget"
  echo "- Estimated tokens (changed files): ${estimated_tokens}"
  echo "- Budget: ${token_budget}"
  echo "- Warning threshold: ${warn_tokens} (${warn_pct}%)"
  echo "- Status: ${status}"
} > "$out"

rm -f "$related_tmp"

echo "Wrote context pack: $out"
if [[ "$status" == "warning" ]]; then
  echo "Warning: context estimate ${estimated_tokens} >= warning threshold ${warn_tokens}" >&2
fi
if [[ "$status" == "over_budget" ]]; then
  echo "Warning: context estimate ${estimated_tokens} >= budget ${token_budget}" >&2
  if [[ "$strict_budget" == "1" ]]; then
    echo "Context budget strict mode enabled; failing." >&2
    exit 1
  fi
fi
