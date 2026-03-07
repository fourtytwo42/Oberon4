#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

load_env_file() {
  local env_file="$1"
  if [ -f "$env_file" ]; then
    echo "Loading env: $env_file"
    set -a
    # shellcheck disable=SC1090
    source "$env_file"
    set +a
  fi
}

load_env_file "${ROOT_DIR}/.env"
load_env_file "${ROOT_DIR}/.env.local"
# If the website repo is checked out next door, reuse its base URL config.
load_env_file "${ROOT_DIR}/../llmhotspot/.env"
load_env_file "${ROOT_DIR}/../llmhotspot/.env.local"

# Defaults can be overridden by .env/.env.local or already-exported env vars.
: "${APP_NAME:=llmhotspot-v2-control-api}"
: "${BINARY_PATH:=${ROOT_DIR}/target/debug/llmhotspot}"
: "${DB_PATH:=/tmp/llmhotspot-live.db}"
: "${LISTEN_ADDR:=0.0.0.0:11434}"

: "${LLMHOTSPOT_WEBSITE_BASE_URL:=${APP_BASE_URL:-https://keys.llmhotspot.com}}"
: "${LLMHOTSPOT_TUNNEL_BROKER_URL:=https://keys.llmhotspot.com}"
: "${LLMHOTSPOT_LICENSE_KEY:=LLMH-DEMO-0001-TEST-0001}"
: "${LLMHOTSPOT_ENDPOINT_SLUG:=young-kernel-beacon}"
: "${LLMHOTSPOT_TUNNEL_RUNTIME:=1}"

export APP_NAME BINARY_PATH DB_PATH LISTEN_ADDR
export LLMHOTSPOT_WEBSITE_BASE_URL LLMHOTSPOT_TUNNEL_BROKER_URL LLMHOTSPOT_LICENSE_KEY LLMHOTSPOT_ENDPOINT_SLUG LLMHOTSPOT_TUNNEL_RUNTIME
if [ -n "${LLMHOTSPOT_DEVICE_ID:-}" ]; then
  export LLMHOTSPOT_DEVICE_ID
fi

if ! command -v pm2 >/dev/null 2>&1; then
  echo "pm2 is required but not installed."
  exit 1
fi

if [ ! -x "$BINARY_PATH" ]; then
  echo "Binary not found or not executable: $BINARY_PATH"
  exit 1
fi

if pm2 describe "$APP_NAME" >/dev/null 2>&1; then
  echo "App already exists in pm2; applying env and restarting: $APP_NAME"
  if ! pm2 restart "$APP_NAME" --update-env; then
    echo "Restart failed; removing stale pm2 entry and starting fresh."
    pm2 delete "$APP_NAME" >/dev/null 2>&1 || true
    pm2 start "$BINARY_PATH" \
      --name "$APP_NAME" \
      --interpreter none \
      -- --db-path "$DB_PATH" serve --listen "$LISTEN_ADDR" --no-tray
  fi
else
  echo "Starting pm2 app: $APP_NAME"
  pm2 start "$BINARY_PATH" \
    --name "$APP_NAME" \
    --interpreter none \
    -- --db-path "$DB_PATH" serve --listen "$LISTEN_ADDR" --no-tray
fi

pm2 save

if command -v jq >/dev/null 2>&1; then
  pm2 jlist | jq -r ".[] | select(.name==\"$APP_NAME\") | .pid" > "${ROOT_DIR}/.llmhotspot.pid"
fi

sleep 1
echo "Health check: http://localhost:11434/v1/endpoints"
attempt=0
max_attempts=10
while [ "$attempt" -lt "$max_attempts" ]; do
  attempt=$((attempt + 1))
  endpoints_json="$(curl -fsS http://localhost:11434/v1/endpoints || true)"
  if [ -n "$endpoints_json" ] && echo "$endpoints_json" | grep -q '"remote_endpoint_url":"https\?://'; then
    echo "$endpoints_json" | { command -v jq >/dev/null 2>&1 && jq . || cat; }
    exit 0
  fi
  sleep 1
done

echo "$endpoints_json" | { command -v jq >/dev/null 2>&1 && jq . || cat; }
echo "Warning: remote endpoint was not populated within ${max_attempts}s."
