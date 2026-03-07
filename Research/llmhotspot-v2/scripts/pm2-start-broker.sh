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

: "${APP_NAME:=llmhotspot-v2-broker}"
: "${BINARY_PATH:=${ROOT_DIR}/target/release/llmhotspot}"
: "${DB_PATH:=/tmp/llmhotspot-broker.db}"
: "${BROKER_LISTEN_ADDR:=0.0.0.0:3001}"
: "${LLMHOTSPOT_BROKER_DB_BACKEND:=postgres}"

# Enable broker API (subdomain + CF tunnel issuance).
: "${LLMHOTSPOT_TUNNEL_BROKER:=1}"

# Domain where per-device endpoints will be created (CNAME records in CF zone).
: "${ENDPOINTS_BASE_DOMAIN:=${CF_BASE_DOMAIN:-llmhotspot.com}}"

export APP_NAME BINARY_PATH DB_PATH BROKER_LISTEN_ADDR LLMHOTSPOT_BROKER_DB_BACKEND
export LLMHOTSPOT_TUNNEL_BROKER ENDPOINTS_BASE_DOMAIN

BROKER_ARGS=(broker --listen "$BROKER_LISTEN_ADDR")
if [ "${LLMHOTSPOT_BROKER_DB_BACKEND}" = "postgres" ]; then
  if [ -n "${LLMHOTSPOT_BROKER_DATABASE_URL:-}" ] || [ -n "${DATABASE_URL:-}" ]; then
    BROKER_ARGS=(broker --listen "$BROKER_LISTEN_ADDR" --db-backend postgres)
  else
    echo "Warning: missing LLMHOTSPOT_BROKER_DATABASE_URL for postgres backend; falling back to sqlite."
    export LLMHOTSPOT_BROKER_DB_BACKEND=sqlite
    BROKER_ARGS=(--db-path "$DB_PATH" broker --listen "$BROKER_LISTEN_ADDR" --db-backend sqlite)
  fi
else
  BROKER_ARGS=(--db-path "$DB_PATH" broker --listen "$BROKER_LISTEN_ADDR" --db-backend sqlite)
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
    pm2 delete "$APP_NAME" >/dev/null 2>&1 || true
  fi
fi

if ! pm2 describe "$APP_NAME" >/dev/null 2>&1; then
  echo "Starting pm2 app: $APP_NAME"
  pm2 start "$BINARY_PATH" \
    --name "$APP_NAME" \
    --interpreter none \
    -- "${BROKER_ARGS[@]}"
fi

pm2 save

echo "Health check: http://127.0.0.1:${BROKER_LISTEN_ADDR##*:}/health"
curl -fsS "http://127.0.0.1:${BROKER_LISTEN_ADDR##*:}/health" >/dev/null || true
echo "Broker listening on: http://${BROKER_LISTEN_ADDR}"
