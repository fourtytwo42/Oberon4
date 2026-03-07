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

: "${PLAN_FILE:=${ROOT_DIR}/docs/plans/plan-agent-micro-jobs.md}"
: "${STATE_FILE:=${ROOT_DIR}/.agent-orchestration/state.json}"
: "${APP_PREFIX:=oberon-build}"
: "${AUTOPILOT_NAME:=${APP_PREFIX}-autopilot}"
: "${WATCHDOG_NAME:=${APP_PREFIX}-watchdog}"
: "${CONCURRENCY:=5}"
: "${MAX_PER_DEPT:=5}"
: "${SLEEP_SEC:=3}"
: "${WATCHDOG_CHECK_SEC:=15}"
: "${AUTO_GIT_SYNC:=1}"
: "${GIT_SYNC_ISSUE_REF:=#21}"
: "${GIT_SYNC_MIN_DONE_DELTA:=1}"
: "${GIT_SYNC_MIN_SECONDS:=180}"
: "${GIT_SYNC_ON_PHASE_CHANGE:=1}"
: "${GIT_SYNC_PUSH_ENABLED:=1}"

usage() {
  cat <<EOF
Usage:
  scripts/multi-agent-build.sh <command>

Commands:
  init       Initialize orchestration state from plan
  start      Start PM2-managed autopilot + watchdog
  up         init (if missing) + start
  gate       Run full-app completion gate checks
  screens    Capture console UI screenshots for review
  finalize   Run gate + screenshots
  pause      Request drain pause (no new dispatch)
  pause-now  Immediate pause
  resume     Resume run
  drain      Pause with drain and wait until paused
  recover    Reset stale in_progress jobs to pending
  status     Dashboard + PM2 status + open concerns
  monitor    Live monitor (agent-control monitor)
  logs       PM2 logs for autopilot/watchdog
  stop       Stop PM2 autopilot/watchdog

Environment overrides:
  PLAN_FILE, STATE_FILE, APP_PREFIX, AUTOPILOT_NAME, WATCHDOG_NAME
  CONCURRENCY, MAX_PER_DEPT, SLEEP_SEC, WATCHDOG_CHECK_SEC
  AUTO_GIT_SYNC, GIT_SYNC_ISSUE_REF, GIT_SYNC_MIN_DONE_DELTA, GIT_SYNC_MIN_SECONDS
  GIT_SYNC_ON_PHASE_CHANGE, GIT_SYNC_PUSH_ENABLED
EOF
}

require_pm2() {
  if ! command -v pm2 >/dev/null 2>&1; then
    echo "pm2 is required but not installed."
    exit 1
  fi
}

init_state() {
  mkdir -p "$(dirname "${STATE_FILE}")"
  node "${ROOT_DIR}/scripts/agent-control.js" init --plan "${PLAN_FILE}" --state-file "${STATE_FILE}"
}

start_pm2() {
  require_pm2

  if [ ! -f "${STATE_FILE}" ]; then
    echo "State file not found: ${STATE_FILE}"
    echo "Run: scripts/multi-agent-build.sh init"
    exit 1
  fi

  pm2 delete "${AUTOPILOT_NAME}" >/dev/null 2>&1 || true
  GIT_SYNC_ISSUE_REF="${GIT_SYNC_ISSUE_REF}" \
  GIT_SYNC_MIN_DONE_DELTA="${GIT_SYNC_MIN_DONE_DELTA}" \
  GIT_SYNC_MIN_SECONDS="${GIT_SYNC_MIN_SECONDS}" \
  GIT_SYNC_ON_PHASE_CHANGE="${GIT_SYNC_ON_PHASE_CHANGE}" \
  GIT_SYNC_PUSH_ENABLED="${GIT_SYNC_PUSH_ENABLED}" \
  pm2 start node --name "${AUTOPILOT_NAME}" --cwd "${ROOT_DIR}" -- \
    scripts/agent-autopilot.js \
    --loop \
    --concurrency "${CONCURRENCY}" \
    --max-per-dept "${MAX_PER_DEPT}" \
    --sleep-sec "${SLEEP_SEC}" \
    --auto-git-sync "${AUTO_GIT_SYNC}" \
    --state-file "${STATE_FILE}" >/dev/null

  pm2 delete "${WATCHDOG_NAME}" >/dev/null 2>&1 || true
  CHECK_EVERY_SEC="${WATCHDOG_CHECK_SEC}" \
  CONCURRENCY="${CONCURRENCY}" \
  MAX_PER_DEPT="${MAX_PER_DEPT}" \
  SLEEP_SEC="${SLEEP_SEC}" \
  pm2 start bash --name "${WATCHDOG_NAME}" --cwd "${ROOT_DIR}" -- \
    scripts/autopilot-watchdog.sh "${ROOT_DIR}" "${STATE_FILE}" >/dev/null

  pm2 save >/dev/null
}

pause_run() {
  local mode="$1"
  node "${ROOT_DIR}/scripts/agent-control.js" pause-run --mode "${mode}" --reason "operator requested pause via multi-agent-build.sh" --state-file "${STATE_FILE}"
}

resume_run() {
  node "${ROOT_DIR}/scripts/agent-control.js" resume-run --state-file "${STATE_FILE}"
}

recover_stale() {
  node - <<'NODE' "${STATE_FILE}"
const fs = require("fs");
const p = process.argv[1];
const s = JSON.parse(fs.readFileSync(p, "utf8"));
const changed = [];
for (const j of Object.values(s.jobs || {})) {
  if (j.status === "in_progress") {
    j.status = "pending";
    if (!Array.isArray(j.notes)) j.notes = [];
    j.notes.push({ ts: new Date().toISOString(), text: "[manual-recover] stale in_progress -> pending" });
    changed.push(j.id);
  }
}
if (changed.length > 0) {
  if (!Array.isArray(s.events)) s.events = [];
  s.events.push({ ts: new Date().toISOString(), type: "manual.recover.stale", jobs: changed });
  s.updatedAt = new Date().toISOString();
  fs.writeFileSync(p, JSON.stringify(s, null, 2));
}
console.log(changed.length ? `Recovered: ${changed.join(", ")}` : "No stale in_progress jobs found.");
NODE
}

drain_wait() {
  pause_run "drain"
  echo "Waiting for drain pause to finalize..."
  for _ in $(seq 1 240); do
    mode="$(node -e 'const s=require(process.argv[1]); console.log(s?.pipeline?.runControl?.mode||"running")' "${STATE_FILE}")"
    inprog="$(node -e 'const s=require(process.argv[1]); const n=Object.values(s.jobs||{}).filter(j=>j.status==="in_progress").length; console.log(n)' "${STATE_FILE}")"
    echo "mode=${mode} in_progress=${inprog}"
    if [ "${mode}" = "paused" ]; then
      echo "Drain complete."
      return 0
    fi
    sleep 2
  done
  echo "Timed out waiting for drain pause."
  return 1
}

show_status() {
  node "${ROOT_DIR}/scripts/agent-control.js" summary --state-file "${STATE_FILE}"
  echo
  require_pm2
  pm2 list | rg "${AUTOPILOT_NAME}|${WATCHDOG_NAME}|name|status|online|stopped|errored" -n -S || true
  echo
  node "${ROOT_DIR}/scripts/agent-control.js" concerns --status open --limit 20 --state-file "${STATE_FILE}"
}

run_gate() {
  "${ROOT_DIR}/scripts/full-app-gate.sh"
}

run_screens() {
  "${ROOT_DIR}/scripts/capture-console-ui.sh"
}

stop_pm2() {
  require_pm2
  pm2 stop "${AUTOPILOT_NAME}" >/dev/null 2>&1 || true
  pm2 stop "${WATCHDOG_NAME}" >/dev/null 2>&1 || true
  pm2 save >/dev/null
}

CMD="${1:-}"
case "${CMD}" in
  init)
    init_state
    ;;
  start)
    start_pm2
    show_status
    ;;
  up)
    [ -f "${STATE_FILE}" ] || init_state
    start_pm2
    show_status
    ;;
  gate)
    run_gate
    ;;
  screens)
    run_screens
    ;;
  finalize)
    run_gate
    run_screens
    ;;
  pause)
    pause_run "drain"
    show_status
    ;;
  pause-now)
    pause_run "immediate"
    show_status
    ;;
  resume)
    resume_run
    show_status
    ;;
  drain)
    drain_wait
    show_status
    ;;
  recover)
    recover_stale
    show_status
    ;;
  status)
    show_status
    ;;
  monitor)
    node "${ROOT_DIR}/scripts/agent-control.js" monitor --interval 3 --state-file "${STATE_FILE}"
    ;;
  logs)
    require_pm2
    pm2 logs "${AUTOPILOT_NAME}" "${WATCHDOG_NAME}"
    ;;
  stop)
    stop_pm2
    show_status
    ;;
  *)
    usage
    exit 2
    ;;
esac
