#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

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

# Main app defaults (main daemon + console only; no website dependency)
: "${APP_NAME:=oberon-daemon}"
: "${CRATE_NAME:=llmhotspot}"
: "${BUILD_PROFILE:=debug}"         # debug|release
: "${LISTEN_ADDR:=127.0.0.1:11434}"
: "${DB_PATH:=/tmp/oberon-main.db}"
: "${STOP_WEBSITE_PROCESSES:=1}"    # 1 => stop website PM2 apps
: "${STOP_ORCHESTRATION_PROCESSES:=1}" # 1 => stop autopilot/watchdog PM2 apps
: "${CARGO_LOCKED:=0}"              # 1 => use --locked for build

if [ "${BUILD_PROFILE}" = "release" ]; then
  BINARY_PATH_DEFAULT="${ROOT_DIR}/target/release/${CRATE_NAME}"
else
  BINARY_PATH_DEFAULT="${ROOT_DIR}/target/debug/${CRATE_NAME}"
fi

: "${BINARY_PATH:=${BINARY_PATH_DEFAULT}}"

WEBSITE_PM2_NAMES=(
  "oberon-next"
  "llmhotspot-v2-website"
  "llmhotspot-v2-website-admin"
)

ORCH_PM2_NAMES=(
  "oberon-autopilot"
  "oberon-watchdog"
)

usage() {
  cat <<EOF
Usage:
  scripts/main-app.sh <command>

Commands:
  build      Build ${CRATE_NAME} (${BUILD_PROFILE})
  start      Start main app in PM2 (build if missing)
  restart    Rebuild and restart main app in PM2
  up         Build (if needed), stop vestigial services, start main app
  down       Stop main app PM2 process
  status     Show PM2 status + socket check + health probe
  logs       Tail PM2 logs for main app

Environment overrides:
  APP_NAME, CRATE_NAME, BUILD_PROFILE, BINARY_PATH, LISTEN_ADDR, DB_PATH
  STOP_WEBSITE_PROCESSES, STOP_ORCHESTRATION_PROCESSES, CARGO_LOCKED
EOF
}

require_pm2() {
  if ! command -v pm2 >/dev/null 2>&1; then
    echo "pm2 is required but not installed."
    exit 1
  fi
}

build_binary() {
  local -a build_args=(build -q -p "${CRATE_NAME}")
  if [ "${BUILD_PROFILE}" = "release" ]; then
    build_args+=(--release)
  fi
  if [ "${CARGO_LOCKED}" = "1" ]; then
    build_args+=(--locked)
  fi

  echo "Building ${CRATE_NAME} (${BUILD_PROFILE})..."
  (cd "${ROOT_DIR}" && cargo "${build_args[@]}")
}

stop_if_exists() {
  local name="$1"
  if pm2 describe "${name}" >/dev/null 2>&1; then
    pm2 stop "${name}" >/dev/null 2>&1 || true
  fi
}

stop_vestigial_processes() {
  if [ "${STOP_WEBSITE_PROCESSES}" = "1" ]; then
    for name in "${WEBSITE_PM2_NAMES[@]}"; do
      stop_if_exists "${name}"
    done
  fi

  if [ "${STOP_ORCHESTRATION_PROCESSES}" = "1" ]; then
    for name in "${ORCH_PM2_NAMES[@]}"; do
      stop_if_exists "${name}"
    done
  fi
}

start_main_app() {
  require_pm2

  if [ ! -x "${BINARY_PATH}" ]; then
    build_binary
  fi

  if [ ! -x "${BINARY_PATH}" ]; then
    echo "Binary not found or not executable: ${BINARY_PATH}"
    exit 1
  fi

  if pm2 describe "${APP_NAME}" >/dev/null 2>&1; then
    pm2 restart "${APP_NAME}" --update-env >/dev/null
  else
    pm2 start "${BINARY_PATH}" \
      --name "${APP_NAME}" \
      --interpreter none \
      -- --db-path "${DB_PATH}" serve --listen "${LISTEN_ADDR}" --no-tray >/dev/null
  fi
  pm2 save >/dev/null
}

restart_main_app() {
  require_pm2
  build_binary
  if ! pm2 describe "${APP_NAME}" >/dev/null 2>&1; then
    start_main_app
    return
  fi
  pm2 restart "${APP_NAME}" --update-env >/dev/null
  pm2 save >/dev/null
}

status_main_app() {
  require_pm2
  pm2 list
  echo
  echo "Socket check:"
  ss -ltnp | rg ":3000|:11434|:${LISTEN_ADDR##*:}" -S || true
  echo
  echo "Health probe (/v1/endpoints):"
  curl -fsS "http://${LISTEN_ADDR}/v1/endpoints" | { command -v jq >/dev/null 2>&1 && jq . || cat; } || true
  echo
  echo "Console URL: http://${LISTEN_ADDR}/console"
}

logs_main_app() {
  require_pm2
  pm2 logs "${APP_NAME}"
}

CMD="${1:-}"
case "${CMD}" in
  build)
    build_binary
    ;;
  start)
    start_main_app
    status_main_app
    ;;
  restart)
    restart_main_app
    status_main_app
    ;;
  up)
    require_pm2
    stop_vestigial_processes
    start_main_app
    status_main_app
    ;;
  down)
    require_pm2
    if pm2 describe "${APP_NAME}" >/dev/null 2>&1; then
      pm2 stop "${APP_NAME}" >/dev/null
      pm2 save >/dev/null
      echo "Stopped ${APP_NAME}"
    else
      echo "${APP_NAME} not found in PM2"
    fi
    ;;
  status)
    status_main_app
    ;;
  logs)
    logs_main_app
    ;;
  *)
    usage
    exit 2
    ;;
esac
