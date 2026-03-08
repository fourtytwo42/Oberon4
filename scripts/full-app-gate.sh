#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "[gate] scripts/full-app-gate.sh is deprecated; delegating to scripts/oberon4-gate.sh" >&2
exec "${SCRIPT_DIR}/oberon4-gate.sh" "$@"
