#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="${ROOT_DIR}/scripts/_bin"
STATE_DIR="${ROOT_DIR}/scripts/_state"
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

: "${CF_API_TOKEN:?missing CF_API_TOKEN in .env}"
: "${CF_ACCOUNT_ID:?missing CF_ACCOUNT_ID in .env}"

# Required for DNS record creation.
: "${CF_ZONE_ID:?missing CF_ZONE_ID in .env (zone id for llmhotspot.com)}"

# Defaults.
: "${CF_BASE_DOMAIN:=llmhotspot.com}"
: "${ORIGIN_URL:=http://127.0.0.1:11434}"
: "${SUBDOMAIN_PREFIX:=}"
: "${TEST_PATH:=/v1/models}"
: "${MAX_WAIT_SECS:=90}"
: "${CLEANUP_ON_FAIL:=1}"

mkdir -p "${BIN_DIR}" "${STATE_DIR}"

download_cloudflared() {
  local os arch url tmp
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  arch="$(uname -m)"

  if [ "$os" != "linux" ]; then
    echo "Unsupported OS for auto-download: $os" >&2
    return 1
  fi

  case "$arch" in
    x86_64|amd64) url="https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64" ;;
    aarch64|arm64) url="https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-arm64" ;;
    *)
      echo "Unsupported arch for auto-download: $arch" >&2
      return 1
      ;;
  esac

  tmp="${CLOUDFLARED}.tmp"
  echo "Downloading cloudflared -> ${CLOUDFLARED}"
  curl -fsSL -o "$tmp" "$url"
  chmod +x "$tmp"
  mv "$tmp" "$CLOUDFLARED"
}

if [ ! -x "$CLOUDFLARED" ]; then
  download_cloudflared
fi

rand_hex() {
  # 10 hex chars; enough to avoid collisions.
  python3 - <<'PY'
import secrets
print(secrets.token_hex(5))
PY
}

SUB="$(rand_hex)"
if [ -n "$SUBDOMAIN_PREFIX" ]; then
  SUB="${SUBDOMAIN_PREFIX}-${SUB}"
fi

HOSTNAME="${SUB}.${CF_BASE_DOMAIN}"
TUNNEL_NAME="llmhotspot-${SUB}"
CONFIG_FILE="${STATE_DIR}/${TUNNEL_NAME}.yml"
PID_FILE="${STATE_DIR}/${TUNNEL_NAME}.pid"
META_FILE="${STATE_DIR}/${TUNNEL_NAME}.meta"
TOKEN_FILE="${STATE_DIR}/${TUNNEL_NAME}.token"

echo "Creating tunnel via Cloudflare API: ${TUNNEL_NAME}"
TUNNEL_CREATE_JSON="$(
  curl -fsS \
    -X POST "https://api.cloudflare.com/client/v4/accounts/${CF_ACCOUNT_ID}/cfd_tunnel" \
    -H "Authorization: Bearer ${CF_API_TOKEN}" \
    -H "Content-Type: application/json" \
    --data "{\"name\":\"${TUNNEL_NAME}\"}"
)"
TUNNEL_ID="$(echo "$TUNNEL_CREATE_JSON" | jq -r '.result.id // empty')"
TUNNEL_TOKEN="$(echo "$TUNNEL_CREATE_JSON" | jq -r '.result.token // empty')"
if [ -z "$TUNNEL_ID" ] || [ -z "$TUNNEL_TOKEN" ]; then
  echo "Failed to create tunnel (missing id/token). Raw response:" >&2
  echo "$TUNNEL_CREATE_JSON" | jq . >&2 || true
  exit 1
fi

umask 077
printf '%s' "$TUNNEL_TOKEN" >"$TOKEN_FILE"
chmod 600 "$TOKEN_FILE"

cleanup_failed() {
  local code="$1"
  if [ "$code" -eq 0 ] || [ "$CLEANUP_ON_FAIL" -eq 0 ]; then
    return 0
  fi
  # Best-effort cleanup. Don't fail the original error path.
  if [ -n "${DNS_RECORD_ID:-}" ]; then
    curl -fsS -X DELETE "https://api.cloudflare.com/client/v4/zones/${CF_ZONE_ID}/dns_records/${DNS_RECORD_ID}" \
      -H "Authorization: Bearer ${CF_API_TOKEN}" >/dev/null 2>&1 || true
  fi
  if [ -n "${TUNNEL_ID:-}" ]; then
    curl -fsS -X DELETE "https://api.cloudflare.com/client/v4/accounts/${CF_ACCOUNT_ID}/cfd_tunnel/${TUNNEL_ID}" \
      -H "Authorization: Bearer ${CF_API_TOKEN}" >/dev/null 2>&1 || true
  fi
}
trap 'cleanup_failed $?' EXIT

echo "Assigning DNS: ${HOSTNAME} -> ${TUNNEL_ID}.cfargotunnel.com"
export _CF_HOSTNAME="$HOSTNAME"
export _CF_TUNNEL_ID="$TUNNEL_ID"
DNS_RECORD_ID="$(python3 - <<'PY'
import json, os, sys
import urllib.request

token = os.environ["CF_API_TOKEN"]
zone = os.environ["CF_ZONE_ID"]
hostname = os.environ["_CF_HOSTNAME"]
tunnel_id = os.environ["_CF_TUNNEL_ID"]

url = f"https://api.cloudflare.com/client/v4/zones/{zone}/dns_records"
payload = {
  "type": "CNAME",
  "name": hostname,
  "content": f"{tunnel_id}.cfargotunnel.com",
  "ttl": 1,
  "proxied": True
}
req = urllib.request.Request(
  url,
  data=json.dumps(payload).encode("utf-8"),
  headers={
    "Authorization": f"Bearer {token}",
    "Content-Type": "application/json",
  },
  method="POST",
)
with urllib.request.urlopen(req, timeout=15) as resp:
  body = resp.read().decode("utf-8", "replace")
data = json.loads(body)
if not data.get("success"):
  sys.stderr.write("Cloudflare DNS create failed: " + body + "\n")
  sys.exit(1)
print(data["result"]["id"])
PY
)"
if [ -z "$DNS_RECORD_ID" ]; then
  echo "Failed to create DNS record id." >&2
  exit 1
fi

cat >"$CONFIG_FILE" <<EOF
tunnel: ${TUNNEL_ID}

ingress:
  - hostname: ${HOSTNAME}
    service: ${ORIGIN_URL}
  - service: http_status:404
EOF

echo "Starting cloudflared (background) for ${HOSTNAME} -> ${ORIGIN_URL}"
nohup "$CLOUDFLARED" tunnel --no-autoupdate --config "$CONFIG_FILE" run --token-file "$TOKEN_FILE" "$TUNNEL_ID" \
  >"${STATE_DIR}/${TUNNEL_NAME}.out.log" \
  2>"${STATE_DIR}/${TUNNEL_NAME}.err.log" &
echo $! >"$PID_FILE"

cat >"$META_FILE" <<EOF
tunnel_name=${TUNNEL_NAME}
tunnel_id=${TUNNEL_ID}
hostname=${HOSTNAME}
origin_url=${ORIGIN_URL}
config_file=${CONFIG_FILE}
token_file=${TOKEN_FILE}
dns_record_id=${DNS_RECORD_ID}
pid_file=${PID_FILE}
pid=$(cat "$PID_FILE")
EOF

echo "Waiting for HTTPS to come up: https://${HOSTNAME}${TEST_PATH}"
deadline=$(( $(date +%s) + MAX_WAIT_SECS ))
while [ "$(date +%s)" -lt "$deadline" ]; do
  if curl -fsS --max-time 5 "https://${HOSTNAME}${TEST_PATH}" >/dev/null 2>&1; then
    echo "OK: tunnel is serving https://${HOSTNAME}${TEST_PATH}"
    echo "Meta: ${META_FILE}"
    exit 0
  fi
  sleep 2
done

echo "Timed out waiting for https://${HOSTNAME}${TEST_PATH}" >&2
echo "Check logs:" >&2
echo "  ${STATE_DIR}/${TUNNEL_NAME}.out.log" >&2
echo "  ${STATE_DIR}/${TUNNEL_NAME}.err.log" >&2
echo "Meta: ${META_FILE}" >&2
exit 1
