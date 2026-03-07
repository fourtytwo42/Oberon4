#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STATE_DIR="${ROOT_DIR}/scripts/_state"
BIN_DIR="${ROOT_DIR}/scripts/_bin"
CLOUDFLARED="${BIN_DIR}/cloudflared"

load_env_file() {
  local env_file="$1"
  if [ -f "$env_file" ]; then
    set -a
    # shellcheck disable=SC1090
    source "$env_file"
    set +a
  fi
}

load_env_file "${ROOT_DIR}/.env"
load_env_file "${ROOT_DIR}/.env.local"

: "${LLMHOTSPOT_WEBSITE_BASE_URL:=http://localhost:3000}"
: "${LLMHOTSPOT_LICENSE_KEY:?missing LLMHOTSPOT_LICENSE_KEY}"
: "${LLMHOTSPOT_DEVICE_ID:?missing LLMHOTSPOT_DEVICE_ID}"
: "${ORIGIN_URL:=http://127.0.0.1:11434}"
: "${PM2_NAME_PREFIX:=llmhotspot-v2-tunnel-}"

mkdir -p "$STATE_DIR" "$BIN_DIR"

if [ ! -x "$CLOUDFLARED" ]; then
  echo "Missing cloudflared at $CLOUDFLARED (run scripts/cf-tunnel-create-and-test.sh once to bootstrap)." >&2
  exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "Missing jq (required). Install jq and retry." >&2
  exit 1
fi

echo "Requesting tunnel provisioning from website: ${LLMHOTSPOT_WEBSITE_BASE_URL}"
PROV_JSON="$(
  curl -fsS -X POST "${LLMHOTSPOT_WEBSITE_BASE_URL%/}/api/tunnels/provision" \
    -H 'Content-Type: application/json' \
    --data "{\"licenseKey\":\"${LLMHOTSPOT_LICENSE_KEY}\",\"deviceId\":\"${LLMHOTSPOT_DEVICE_ID}\"}"
)"

OK="$(echo "$PROV_JSON" | jq -r '.ok // false')"
if [ "$OK" != "true" ]; then
  echo "Provision failed:" >&2
  echo "$PROV_JSON" | jq . >&2 || echo "$PROV_JSON" >&2
  exit 1
fi

HOSTNAME="$(echo "$PROV_JSON" | jq -r '.tunnel.hostname // empty')"
TUNNEL_ID="$(echo "$PROV_JSON" | jq -r '.tunnel.tunnelId // empty')"
TUNNEL_TOKEN="$(echo "$PROV_JSON" | jq -r '.tunnel.tunnelToken // empty')"

if [ -z "$HOSTNAME" ] || [ -z "$TUNNEL_ID" ] || [ -z "$TUNNEL_TOKEN" ]; then
  echo "Provision response missing hostname/tunnelId/tunnelToken." >&2
  echo "$PROV_JSON" | jq . >&2 || true
  exit 1
fi

SAFE_NAME="cf-remote-${HOSTNAME//./_}"
CONFIG_FILE="${STATE_DIR}/${SAFE_NAME}.yml"
TOKEN_FILE="${STATE_DIR}/${SAFE_NAME}.token"
META_FILE="${STATE_DIR}/${SAFE_NAME}.meta"
PM2_NAME="${PM2_NAME_PREFIX}${HOSTNAME//./-}"

umask 077
printf '%s' "$TUNNEL_TOKEN" >"$TOKEN_FILE"
chmod 600 "$TOKEN_FILE"

cat >"$CONFIG_FILE" <<EOF
tunnel: ${TUNNEL_ID}

ingress:
  - hostname: ${HOSTNAME}
    service: ${ORIGIN_URL}
  - service: http_status:404
EOF

cat >"$META_FILE" <<EOF
hostname=${HOSTNAME}
tunnel_id=${TUNNEL_ID}
origin_url=${ORIGIN_URL}
config_file=${CONFIG_FILE}
token_file=${TOKEN_FILE}
pm2_name=${PM2_NAME}
EOF

echo "Starting cloudflared under PM2: ${PM2_NAME}"
pm2 start "$CLOUDFLARED" --name "$PM2_NAME" -- \
  tunnel --no-autoupdate --config "$CONFIG_FILE" run --token-file "$TOKEN_FILE" "$TUNNEL_ID" >/dev/null

pm2 save >/dev/null 2>&1 || true

echo "OK: PM2 is running the tunnel"
echo "Remote endpoint: https://${HOSTNAME}/v1"
echo "Remote console: https://${HOSTNAME}/console"
echo "Meta: ${META_FILE}"

