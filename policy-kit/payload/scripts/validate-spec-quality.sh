#!/usr/bin/env bash
set -euo pipefail

file="${1:-}"
if [[ -z "$file" || ! -f "$file" ]]; then
  echo "Usage: $0 <spec-or-task-brief-file>" >&2
  exit 2
fi

errors=0
in_ac=0

# Vague terms called out by the book for requirements quality gates.
vague='\b(fast|easy|simple|optimi[sz]e|support|robust|user-friendly|quick)\b'

while IFS= read -r line; do
  if [[ "$line" =~ ^##[[:space:]]+Acceptance[[:space:]]+Criteria ]] || [[ "$line" =~ ^##[[:space:]]+Acceptance[[:space:]]+Tests ]]; then
    in_ac=1
    continue
  fi

  if [[ "$line" =~ ^##[[:space:]]+ && ! "$line" =~ ^##[[:space:]]+Acceptance[[:space:]]+(Criteria|Tests) ]]; then
    in_ac=0
  fi

  [[ "$in_ac" -eq 0 ]] && continue

  # Skip blank or heading-like lines.
  [[ -z "${line//[[:space:]]/}" ]] && continue

  if grep -Eiq "$vague" <<<"$line"; then
    # Allow if line includes measurable qualifier.
    if ! grep -Eq '[0-9]+|%|ms|s|sec|seconds|minutes|MB|GB|rps|qps|p95|p99|Given.+When.+Then' <<<"$line"; then
      echo "Vague acceptance criterion in $file: $line" >&2
      errors=1
    fi
  fi
done < "$file"

if [[ "$errors" -ne 0 ]]; then
  echo "Spec quality gate failed for $file. Quantify acceptance criteria." >&2
  exit 1
fi

exit 0
