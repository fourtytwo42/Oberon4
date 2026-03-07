#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/run-agent.sh --role <orchestrator|implementer|verifier|reviewer> --task-id <id> [--cd <dir>] [--prompt <text> | --prompt-file <path>] [--force-unlock] [--lock-timeout-secs <n>]

Environment controls:
  AGENT_DEPTH        Current depth (default: 0)
  AGENT_CAN_SPAWN    1 if current role may spawn (default: 0)
  AGENT_SPAWN_BUDGET Max allowed orchestrator subagent launches per task (default: 3)
  AGENT_MODEL        Optional model override for codex

Notes:
  - Only orchestrator may run with AGENT_CAN_SPAWN=1.
  - Non-orchestrator roles are always forced to no-delegation.
  - Max depth is 1.
EOF
}

role=""
task_id=""
workdir="."
prompt=""
prompt_file=""
force_unlock=0
lock_timeout_secs="${AGENT_LOCK_TIMEOUT_SECS:-1800}"

json_escape() {
  local s="${1:-}"
  s="${s//\\/\\\\}"
  s="${s//\"/\\\"}"
  s="${s//$'\n'/\\n}"
  s="${s//$'\r'/\\r}"
  s="${s//$'\t'/\\t}"
  printf '%s' "$s"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --role) role="${2:-}"; shift 2 ;;
    --task-id) task_id="${2:-}"; shift 2 ;;
    --cd) workdir="${2:-}"; shift 2 ;;
    --prompt) prompt="${2:-}"; shift 2 ;;
    --prompt-file) prompt_file="${2:-}"; shift 2 ;;
    --force-unlock) force_unlock=1; shift ;;
    --lock-timeout-secs) lock_timeout_secs="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *)
      echo "Unknown arg: $1" >&2
      usage
      exit 2
      ;;
  esac
done

if [[ -z "$role" || -z "$task_id" ]]; then
  echo "Missing required args: --role and --task-id" >&2
  usage
  exit 2
fi

if [[ "$role" != "orchestrator" && "$role" != "implementer" && "$role" != "verifier" && "$role" != "reviewer" ]]; then
  echo "Invalid role: $role" >&2
  exit 2
fi

if [[ -z "$prompt" && -z "$prompt_file" ]]; then
  echo "Provide --prompt or --prompt-file" >&2
  exit 2
fi

if [[ -n "$prompt" && -n "$prompt_file" ]]; then
  echo "Provide only one of --prompt or --prompt-file" >&2
  exit 2
fi

if [[ ! -d "$workdir" ]]; then
  echo "Working directory not found: $workdir" >&2
  exit 2
fi

incident_template="$workdir/docs/templates/agent-incident.md"
incident_dir="$workdir/docs/incidents"

record_agent_incident() {
  local category="$1"
  local summary="$2"
  local details="${3:-}"
  local ts
  local slug
  local incident_path
  ts="$(date -u +%FT%TZ)"
  slug="$(date -u +%Y%m%dT%H%M%SZ)"
  mkdir -p "$incident_dir"
  incident_path="$incident_dir/agent-incident-${task_id}-${slug}.md"

  cat > "$incident_path" <<EOF
# Agent Incident: ${category}

## Summary
${summary}

## Incident Metadata
- Timestamp: ${ts}
- Task ID: ${task_id}
- Role: ${role}
- AgentDepth: ${agent_depth:-unknown}
- ParentRole: ${parent_role:-none}
- ParentSession: ${parent_session:-none}

## Trigger Details
${details}

## Immediate Mitigation
- Stop subagent execution.
- Return control to orchestrator.
- Keep delegation disabled for subagents.

## Follow-Up
- Root cause:
- Preventive rule update:
- Validator/hook update:
- Verification evidence:
EOF

  echo "AgentIncident: ${incident_path}" >&2
  echo "Template: ${incident_template}" >&2
}

if [[ ! "$lock_timeout_secs" =~ ^[0-9]+$ ]]; then
  echo "--lock-timeout-secs must be numeric." >&2
  exit 2
fi

agent_depth="${AGENT_DEPTH:-0}"
agent_can_spawn="${AGENT_CAN_SPAWN:-0}"
spawn_budget="${AGENT_SPAWN_BUDGET:-3}"
parent_role="${AGENT_PARENT_ROLE:-none}"
parent_session="${AGENT_PARENT_SESSION:-none}"

if [[ ! "$agent_depth" =~ ^[0-9]+$ ]]; then
  echo "AGENT_DEPTH must be numeric." >&2
  exit 2
fi

if (( agent_depth > 1 )); then
  echo "Blocked: max agent depth is 1 (received AGENT_DEPTH=${agent_depth})." >&2
  record_agent_incident "loop-violation" "Agent depth exceeded hard limit." "AGENT_DEPTH=${agent_depth}"
  exit 1
fi

if [[ "$role" != "orchestrator" && "$agent_can_spawn" == "1" ]]; then
  echo "Blocked: only orchestrator may spawn subagents." >&2
  record_agent_incident "delegation-violation" "Subagent attempted to enable spawning." "role=${role}, AGENT_CAN_SPAWN=${agent_can_spawn}"
  exit 1
fi

if [[ "$role" == "orchestrator" && "$agent_depth" -ne 0 ]]; then
  echo "Blocked: orchestrator must run at AGENT_DEPTH=0." >&2
  record_agent_incident "delegation-violation" "Orchestrator started with invalid depth." "role=${role}, AGENT_DEPTH=${agent_depth}"
  exit 1
fi

if [[ "$role" != "orchestrator" ]]; then
  agent_can_spawn="0"
fi

state_dir="$workdir/.agent-runtime/$task_id"
mkdir -p "$state_dir"
runs_dir="$state_dir/runs"
mkdir -p "$runs_dir"

orchestrator_lock="$state_dir/orchestrator.lock"
spawn_count_file="$state_dir/spawn.count"
lineage_file="$state_dir/lineage.log"
now_epoch="$(date -u +%s)"
run_id="$(date -u +%Y%m%dT%H%M%SZ)-${role}-$$"
manifest_file="$runs_dir/${run_id}.json"

touch "$spawn_count_file"
if [[ ! -s "$spawn_count_file" ]]; then
  echo "0" > "$spawn_count_file"
fi

current_spawns="$(cat "$spawn_count_file")"
if [[ ! "$current_spawns" =~ ^[0-9]+$ ]]; then
  echo "Corrupt spawn counter in $spawn_count_file" >&2
  exit 1
fi

if [[ "$role" == "orchestrator" ]]; then
  if [[ -f "$orchestrator_lock" ]]; then
    lock_pid="$(awk -F= '/^pid=/{print $2}' "$orchestrator_lock" 2>/dev/null || true)"
    lock_ts_epoch="$(awk -F= '/^ts_epoch=/{print $2}' "$orchestrator_lock" 2>/dev/null || true)"
    lock_age=0
    if [[ -n "$lock_ts_epoch" && "$lock_ts_epoch" =~ ^[0-9]+$ ]]; then
      lock_age=$(( now_epoch - lock_ts_epoch ))
    fi

    if [[ "$force_unlock" == "1" ]]; then
      rm -f "$orchestrator_lock"
      echo "Warning: force-unlocked orchestrator lock for task '${task_id}'." >&2
    elif [[ -n "$lock_pid" ]] && kill -0 "$lock_pid" 2>/dev/null; then
      echo "Blocked: orchestrator lock already active for task '${task_id}' (pid ${lock_pid}, age ${lock_age}s)." >&2
      echo "Use --force-unlock only after confirming the active session is stale." >&2
      record_agent_incident "loop-violation" "Concurrent orchestrator lock detected." "task=${task_id}, pid=${lock_pid}, age_secs=${lock_age}"
      exit 1
    elif (( lock_age >= lock_timeout_secs )); then
      rm -f "$orchestrator_lock"
      echo "Warning: removed stale orchestrator lock for task '${task_id}' (age ${lock_age}s)." >&2
    else
      rm -f "$orchestrator_lock"
    fi
  fi
  printf "pid=%s\nts=%s\nts_epoch=%s\n" "$$" "$(date -u +%FT%TZ)" "$now_epoch" > "$orchestrator_lock"
  trap 'rm -f "$orchestrator_lock"' EXIT

  if [[ "$agent_can_spawn" == "1" ]]; then
    if (( current_spawns >= spawn_budget )); then
      echo "Blocked: spawn budget exceeded for task '${task_id}' (${current_spawns}/${spawn_budget})." >&2
      record_agent_incident "loop-violation" "Spawn budget exceeded." "task=${task_id}, spawns=${current_spawns}, budget=${spawn_budget}"
      exit 1
    fi
    echo $(( current_spawns + 1 )) > "$spawn_count_file"
  fi
fi

model_args=()
if [[ -n "${AGENT_MODEL:-}" ]]; then
  model_args=(--model "$AGENT_MODEL")
fi

role_header="Role=${role}; Task=${task_id}; Depth=${agent_depth}; CanSpawn=${agent_can_spawn}"

if [[ -n "$prompt_file" ]]; then
  if [[ ! -f "$prompt_file" ]]; then
    echo "Prompt file not found: $prompt_file" >&2
    exit 2
  fi
  full_prompt="$(printf "%s\n\n%s" "$role_header" "$(cat "$prompt_file")")"
else
  full_prompt="$(printf "%s\n\n%s" "$role_header" "$prompt")"
fi

printf "%s role=%s depth=%s can_spawn=%s\n" "$(date -u +%FT%TZ)" "$role" "$agent_depth" "$agent_can_spawn" >> "$lineage_file"

status="success"
failure_reason=""

if ! codex -a never exec \
  --ephemeral \
  --sandbox workspace-write \
  --cd "$workdir" \
  --skip-git-repo-check \
  "${model_args[@]}" \
  "$full_prompt"; then
  status="failed"
  failure_reason="codex_exec_nonzero"
fi

manifest_ts="$(date -u +%FT%TZ)"
cat > "$manifest_file" <<EOF
{
  "run_id": "$(json_escape "$run_id")",
  "timestamp": "$(json_escape "$manifest_ts")",
  "task_id": "$(json_escape "$task_id")",
  "role": "$(json_escape "$role")",
  "depth": $agent_depth,
  "can_spawn": $agent_can_spawn,
  "spawn_budget": $spawn_budget,
  "parent_role": "$(json_escape "$parent_role")",
  "parent_session": "$(json_escape "$parent_session")",
  "workdir": "$(json_escape "$workdir")",
  "prompt_source": "$(json_escape "${prompt_file:-inline}")",
  "status": "$(json_escape "$status")",
  "failure_reason": "$(json_escape "$failure_reason")"
}
EOF

if [[ "$status" != "success" ]]; then
  echo "Agent run failed. Manifest: $manifest_file" >&2
  exit 1
fi
