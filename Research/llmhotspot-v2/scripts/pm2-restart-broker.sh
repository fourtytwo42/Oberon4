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
: "${LLMHOTSPOT_CARGO_CLEAN_AFTER_BUILD:=1}"

export APP_NAME BINARY_PATH
export LLMHOTSPOT_CARGO_CLEAN_AFTER_BUILD

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

echo "Building llmhotspot binary (release)..."
cargo build -q -p llmhotspot --release
BUILT_BINARY=1

if ! command -v pm2 >/dev/null 2>&1; then
  echo "pm2 is required but not installed."
  exit 1
fi

if ! pm2 describe "$APP_NAME" >/dev/null 2>&1; then
  echo "pm2 app not found: $APP_NAME"
  echo "Run scripts/pm2-start-broker.sh first."
  exit 1
fi

pm2 restart "$APP_NAME" --update-env
pm2 save
