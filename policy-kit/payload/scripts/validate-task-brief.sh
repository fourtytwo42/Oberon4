#!/usr/bin/env bash
set -euo pipefail

file="${1:-}"
if [[ -z "$file" || ! -f "$file" ]]; then
  echo "Usage: $0 <task-brief-file>" >&2
  exit 2
fi

errors=0

if ! grep -Eq '^#|^##' "$file"; then
  echo "Task brief missing markdown headings: $file" >&2
  errors=1
fi

if ! grep -Eq 'FR-[0-9]+' "$file"; then
  echo "Task brief must include at least one FR-### requirement ID: $file" >&2
  errors=1
fi

if ! grep -Eq 'AC-[0-9]+' "$file"; then
  echo "Task brief must include at least one AC-### acceptance ID: $file" >&2
  errors=1
fi

# Require at least one Given/When/Then criterion for measurable acceptance.
if ! grep -Eiq 'Given.+When.+Then' "$file"; then
  echo "Task brief should include at least one Given/When/Then acceptance criterion: $file" >&2
  errors=1
fi

if grep -Eiq '(^|[[:space:]])(TBD|TODO|TBC)([[:space:]]|$)' "$file"; then
  echo "Task brief contains unresolved placeholders (TBD/TODO/TBC): $file" >&2
  errors=1
fi

if [[ "$errors" -ne 0 ]]; then
  exit 1
fi

exit 0
