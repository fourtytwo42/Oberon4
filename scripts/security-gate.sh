#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "scripts/security-gate.sh is deprecated; use scripts/oberon4-security-gate.sh" >&2
if [[ $# -gt 0 ]]; then
  export SECURITY_GATE_ROOT="$1"
  shift
fi
exec bash "${SCRIPT_DIR}/oberon4-security-gate.sh" "$@"
