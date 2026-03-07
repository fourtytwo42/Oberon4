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

if ! pm2 describe "$APP_NAME" >/dev/null 2>&1 || ! pm2 describe "$APP_NAME_ADMIN" >/dev/null 2>&1; then
  echo "pm2 app(s) not found: $APP_NAME / $APP_NAME_ADMIN"
  echo "Run scripts/pm2-start-website.sh first."
  exit 1
fi

echo "Building website..."
(cd "$WEBSITE_DIR" && npm run build)

export PORT="${WEBSITE_PORT}"
export HOSTNAME="${WEBSITE_HOST}"
pm2 restart "$APP_NAME" --update-env

export PORT="${WEBSITE_ADMIN_PORT}"
export HOSTNAME="${WEBSITE_HOST}"
pm2 restart "$APP_NAME_ADMIN" --update-env

pm2 save
