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
: "${TEST_PATH:=/v1/models}"
: "${MAX_WAIT_SECS:=90}"

mkdir -p "$STATE_DIR" "$BIN_DIR"

if [ ! -x "$CLOUDFLARED" ]; then
  echo "Missing cloudflared at $CLOUDFLARED (run scripts/cf-tunnel-create-and-test.sh once to bootstrap)." >&2
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
PID_FILE="${STATE_DIR}/${SAFE_NAME}.pid"
META_FILE="${STATE_DIR}/${SAFE_NAME}.meta"

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

echo "Starting cloudflared for ${HOSTNAME} -> ${ORIGIN_URL}"
nohup "$CLOUDFLARED" tunnel --no-autoupdate --config "$CONFIG_FILE" run --token-file "$TOKEN_FILE" "$TUNNEL_ID" \
  >"${STATE_DIR}/${SAFE_NAME}.out.log" \
  2>"${STATE_DIR}/${SAFE_NAME}.err.log" &
echo $! >"$PID_FILE"

cat >"$META_FILE" <<EOF
hostname=${HOSTNAME}
tunnel_id=${TUNNEL_ID}
origin_url=${ORIGIN_URL}
config_file=${CONFIG_FILE}
token_file=${TOKEN_FILE}
pid_file=${PID_FILE}
pid=$(cat "$PID_FILE")
EOF

echo "Waiting for HTTPS: https://${HOSTNAME}${TEST_PATH}"
deadline=$(( $(date +%s) + MAX_WAIT_SECS ))
while [ "$(date +%s)" -lt "$deadline" ]; do
  if curl -fsS --max-time 5 "https://${HOSTNAME}${TEST_PATH}" >/dev/null 2>&1; then
    echo "OK: remote is live"
    echo "Remote endpoint: https://${HOSTNAME}/v1"
    echo "Meta: ${META_FILE}"
    exit 0
  fi
  sleep 2
done

echo "Timed out waiting for https://${HOSTNAME}${TEST_PATH}" >&2
echo "Check logs:" >&2
echo "  ${STATE_DIR}/${SAFE_NAME}.out.log" >&2
echo "  ${STATE_DIR}/${SAFE_NAME}.err.log" >&2
echo "Meta: ${META_FILE}" >&2
exit 1
