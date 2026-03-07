#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STATE_DIR="${ROOT_DIR}/scripts/_state"

usage() {
  echo "Usage: $0 <meta_file>" >&2
  echo "Example: $0 ${STATE_DIR}/cf-remote-young-kernel-beacon_llmhotspot_com.meta" >&2
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

if [ -z "${pm2_name:-}" ]; then
  echo "Meta file missing pm2_name: $meta" >&2
  exit 1
fi

echo "Stopping PM2 process: ${pm2_name}"
pm2 delete "$pm2_name" >/dev/null 2>&1 || true
pm2 save >/dev/null 2>&1 || true

echo "Stopped. Meta: $meta"

