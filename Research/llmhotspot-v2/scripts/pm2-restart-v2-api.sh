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
: "${LLMHOTSPOT_WEBSITE_BASE_URL:=${APP_BASE_URL:-https://keys.llmhotspot.com}}"
: "${LLMHOTSPOT_TUNNEL_BROKER_URL:=https://keys.llmhotspot.com}"
: "${LLMHOTSPOT_LICENSE_KEY:=LLMH-DEMO-0001-TEST-0001}"
: "${LLMHOTSPOT_ENDPOINT_SLUG:=young-kernel-beacon}"
: "${LLMHOTSPOT_TUNNEL_RUNTIME:=1}"
: "${LLMHOTSPOT_CARGO_CLEAN_AFTER_BUILD:=1}"

export APP_NAME
export LLMHOTSPOT_WEBSITE_BASE_URL LLMHOTSPOT_TUNNEL_BROKER_URL LLMHOTSPOT_LICENSE_KEY LLMHOTSPOT_ENDPOINT_SLUG LLMHOTSPOT_TUNNEL_RUNTIME
export LLMHOTSPOT_CARGO_CLEAN_AFTER_BUILD
if [ -n "${LLMHOTSPOT_DEVICE_ID:-}" ]; then
  export LLMHOTSPOT_DEVICE_ID
fi

BUILT_BINARY=0
cleanup_cargo_artifacts() {
  if [ "${BUILT_BINARY}" != "1" ]; then
    return
  fi
  case "${LLMHOTSPOT_CARGO_CLEAN_AFTER_BUILD}" in
    0|false|FALSE|off|OFF|no|NO)
      return
      ;;
  esac
  echo "Cleaning cargo build artifacts..."
  cargo clean -p llmhotspot || true
}
trap cleanup_cargo_artifacts EXIT

echo "Building llmhotspot binary..."
cargo build -q -p llmhotspot
BUILT_BINARY=1

if ! command -v pm2 >/dev/null 2>&1; then
  echo "pm2 is required but not installed."
  exit 1
fi

if ! pm2 describe "$APP_NAME" >/dev/null 2>&1; then
  echo "pm2 app not found: $APP_NAME"
  echo "Run scripts/pm2-start-v2-api.sh first."
  exit 1
fi

if ! pm2 restart "$APP_NAME" --update-env; then
  echo "Restart failed; attempting clean recreate via start script."
  bash "${ROOT_DIR}/scripts/pm2-start-v2-api.sh"
  exit 0
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
