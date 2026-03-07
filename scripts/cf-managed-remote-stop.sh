#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STATE_DIR="${ROOT_DIR}/scripts/_state"

usage() {
  echo "Usage: $0 <meta_file>" >&2
  echo "Example: $0 ${STATE_DIR}/cf-remote_young-kernel-beacon_llmhotspot_com.meta" >&2
}

meta="${1:-}"
if [ -z "$meta" ]; then
  usage
  exit 2
fi
if [ ! -f "$meta" ]; then
  echo "Meta file not found: $meta" >&2
  exit 1
fi

# shellcheck disable=SC1090
source "$meta"

if [ -n "${pid:-}" ]; then
  echo "Stopping cloudflared pid=${pid}"
  kill "$pid" >/dev/null 2>&1 || true
fi

echo "Stopped. Meta: $meta"

