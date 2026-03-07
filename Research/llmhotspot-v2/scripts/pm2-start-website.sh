#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
WEBSITE_DIR="${ROOT_DIR}/apps/website"

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
load_env_file "${WEBSITE_DIR}/.env"
load_env_file "${WEBSITE_DIR}/.env.local"

: "${APP_NAME:=llmhotspot-v2-website}"
: "${APP_NAME_ADMIN:=llmhotspot-v2-website-admin}"
: "${WEBSITE_PORT:=3000}"
: "${WEBSITE_ADMIN_PORT:=3010}"
: "${WEBSITE_HOST:=0.0.0.0}"

export APP_NAME APP_NAME_ADMIN WEBSITE_PORT WEBSITE_ADMIN_PORT WEBSITE_HOST

if ! command -v pm2 >/dev/null 2>&1; then
  echo "pm2 is required but not installed."
  exit 1
fi

if [ ! -f "${WEBSITE_DIR}/package.json" ]; then
  echo "Website app not found: ${WEBSITE_DIR}/package.json"
  exit 1
fi

if [ ! -d "${WEBSITE_DIR}/node_modules" ]; then
  echo "Installing website dependencies..."
  (cd "$WEBSITE_DIR" && npm install)
fi

echo "Building website..."
(cd "$WEBSITE_DIR" && npm run build)

start_or_restart() {
  local process_name="$1"
  local port="$2"

  export PORT="$port"
  export HOSTNAME="${WEBSITE_HOST}"

  if pm2 describe "$process_name" >/dev/null 2>&1; then
    echo "App already exists in pm2; applying env and restarting: $process_name"
    if ! pm2 restart "$process_name" --update-env; then
      pm2 delete "$process_name" >/dev/null 2>&1 || true
    fi
  fi

  if ! pm2 describe "$process_name" >/dev/null 2>&1; then
    echo "Starting pm2 app: $process_name on port $port"
    pm2 start npm \
      --name "$process_name" \
      --cwd "$WEBSITE_DIR" \
      -- run start
  fi
}

start_or_restart "$APP_NAME" "$WEBSITE_PORT"
start_or_restart "$APP_NAME_ADMIN" "$WEBSITE_ADMIN_PORT"

pm2 save

echo "Health check: http://127.0.0.1:${WEBSITE_PORT}/"
curl -fsS "http://127.0.0.1:${WEBSITE_PORT}/" >/dev/null || true
echo "Health check: http://127.0.0.1:${WEBSITE_ADMIN_PORT}/admin"
curl -fsS "http://127.0.0.1:${WEBSITE_ADMIN_PORT}/admin" >/dev/null || true

echo "Website listening on: http://127.0.0.1:${WEBSITE_PORT}/"
echo "Admin UI listening on: http://127.0.0.1:${WEBSITE_ADMIN_PORT}/admin"
