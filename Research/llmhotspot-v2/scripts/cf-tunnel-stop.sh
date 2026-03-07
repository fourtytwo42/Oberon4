#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STATE_DIR="${ROOT_DIR}/scripts/_state"

usage() {
  echo "Usage: $0 <tunnel_name_or_meta_file>" >&2
  echo "       $0 --delete <tunnel_name_or_meta_file>  (also delete DNS + tunnel)" >&2
  echo "Example: $0 llmhotspot-abc123" >&2
  echo "Example: $0 ${STATE_DIR}/llmhotspot-abc123.meta" >&2
}

do_delete=0
if [ "${1:-}" = "--delete" ]; then
  do_delete=1
  shift
fi

if [ "${1:-}" = "" ]; then
  usage
  exit 2
fi

arg="$1"
meta=""
if [ -f "$arg" ]; then
  meta="$arg"
else
  meta="${STATE_DIR}/${arg}.meta"
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

echo "Stopped. (DNS/tunnel not deleted)"
echo "Meta: $meta"

if [ "$do_delete" -ne 1 ]; then
  exit 0
fi

if [ -z "${CF_API_TOKEN:-}" ] || [ -z "${CF_ACCOUNT_ID:-}" ] || [ -z "${CF_ZONE_ID:-}" ]; then
  echo "Missing CF_API_TOKEN/CF_ACCOUNT_ID/CF_ZONE_ID in environment; cannot delete." >&2
  exit 1
fi

if [ -n "${dns_record_id:-}" ]; then
  echo "Deleting DNS record id=${dns_record_id}"
  curl -fsS -X DELETE "https://api.cloudflare.com/client/v4/zones/${CF_ZONE_ID}/dns_records/${dns_record_id}" \
    -H "Authorization: Bearer ${CF_API_TOKEN}" >/dev/null || true
fi

if [ -n "${tunnel_id:-}" ]; then
  echo "Deleting tunnel id=${tunnel_id}"
  curl -fsS -X DELETE "https://api.cloudflare.com/client/v4/accounts/${CF_ACCOUNT_ID}/cfd_tunnel/${tunnel_id}" \
    -H "Authorization: Bearer ${CF_API_TOKEN}" >/dev/null || true
fi

echo "Deleted DNS + tunnel."
