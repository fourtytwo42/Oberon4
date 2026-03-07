#!/usr/bin/env bash
set -uo pipefail

ROOT="${1:-$(pwd)}"
STATE_FILE="${2:-$ROOT/.agent-orchestration/state.json}"
CHECK_EVERY_SEC="${CHECK_EVERY_SEC:-15}"
CONCURRENCY="${CONCURRENCY:-5}"
MAX_PER_DEPT="${MAX_PER_DEPT:-5}"
SLEEP_SEC="${SLEEP_SEC:-3}"

AUTOPILOT_CMD="node scripts/agent-autopilot.js --loop --concurrency ${CONCURRENCY} --max-per-dept ${MAX_PER_DEPT} --sleep-sec ${SLEEP_SEC}"
AUTOPILOT_LOG="${AUTOPILOT_LOG:-/tmp/oberon-autopilot.log}"
WATCHDOG_LOG="${WATCHDOG_LOG:-/tmp/oberon-watchdog.log}"
PID_FILE="${PID_FILE:-$ROOT/.agent-runtime/autopilot/watchdog.pid}"

mkdir -p "$ROOT/.agent-runtime/autopilot"
echo "$$" > "$PID_FILE"

log() {
  printf '%s [watchdog] %s\n' "$(date -u +%FT%TZ)" "$*" | tee -a "$WATCHDOG_LOG"
}

trap 'log "exiting (code=$?)"' EXIT

has_autopilot() {
  pgrep -f "node scripts/agent-autopilot.js --loop" >/dev/null 2>&1
}

state_guard_ok() {
  if [[ ! -f "$STATE_FILE" ]]; then
    return 1
  fi
  node -e '
const fs=require("fs");
const p=process.argv[1];
const s=JSON.parse(fs.readFileSync(p,"utf8"));
const completed=Boolean(s?.pipeline?.completed);
const mode=s?.pipeline?.runControl?.mode || "running";
if (completed || mode==="paused" || mode==="pause_requested") process.exit(1);
process.exit(0);
' "$STATE_FILE"
}

reset_stale_in_progress() {
  node -e '
const fs=require("fs");
const p=process.argv[1];
const s=JSON.parse(fs.readFileSync(p,"utf8"));
let changed=[];
for (const j of Object.values(s.jobs||{})) {
  if (j.status==="in_progress") {
    j.status="pending";
    j.notes=j.notes||[];
    j.notes.push({ts:new Date().toISOString(), text:"[watchdog] stale in_progress reset to pending before restart"});
    changed.push(j.id);
  }
}
if (changed.length) {
  s.events=s.events||[];
  s.events.push({ts:new Date().toISOString(), type:"watchdog.reset_stale_in_progress", jobs:changed});
  s.updatedAt=new Date().toISOString();
  fs.writeFileSync(p, JSON.stringify(s,null,2));
}
console.log(changed.join(","));
' "$STATE_FILE"
}

start_autopilot() {
  (
    cd "$ROOT"
    nohup bash -lc "$AUTOPILOT_CMD" >>"$AUTOPILOT_LOG" 2>&1 &
    echo $!
  )
}

log "started (interval=${CHECK_EVERY_SEC}s, state=${STATE_FILE})"

while true; do
  sleep "$CHECK_EVERY_SEC"

  if ! state_guard_ok; then
    log "guard skip (paused/pause_requested/completed/missing state)"
    continue
  fi

  if has_autopilot; then
    log "healthy (autopilot running)"
    continue
  fi

  stale="$(reset_stale_in_progress 2>/dev/null || true)"
  if [[ -n "${stale:-}" ]]; then
    log "recovered stale jobs: ${stale}"
  fi

  new_pid="$(start_autopilot 2>/dev/null || true)"
  if [[ -n "${new_pid:-}" ]]; then
    log "autopilot restarted pid=${new_pid}"
  else
    log "restart attempt failed"
  fi
done
