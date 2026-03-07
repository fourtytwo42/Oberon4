#!/usr/bin/env bash
set -euo pipefail

range="${1:-}"
if [[ -z "$range" ]]; then
  echo "Usage: $0 <git-range>" >&2
  exit 2
fi

parse_last_field() {
  local pattern="$1"
  local msg="$2"
  grep -Eio "$pattern" <<<"$msg" | sed -E "s/^[^:]+:[[:space:]]*//I" | tr '[:upper:]' '[:lower:]' | tail -n1
}

while IFS= read -r sha; do
  [[ -z "$sha" ]] && continue
  msg="$(git show -s --format=%B "$sha")"

  # Skip merge/revert/fixup/squash commits.
  subject="$(head -n1 <<<"$msg")"
  if [[ "$subject" =~ ^Merge\  || "$subject" =~ ^Revert\  || "$subject" =~ ^fixup! || "$subject" =~ ^squash! ]]; then
    continue
  fi

  risk="$(parse_last_field '^Risk:[[:space:]]*(low|medium|high)[[:space:]]*$' "$msg")"
  if [[ "$risk" != "medium" && "$risk" != "high" ]]; then
    continue
  fi

  role="$(parse_last_field '^AgentRole:[[:space:]]*(orchestrator|implementer|verifier|reviewer|solo)[[:space:]]*$' "$msg")"
  depth="$(parse_last_field '^AgentDepth:[[:space:]]*(0|1)[[:space:]]*$' "$msg")"
  delegation="$(parse_last_field '^Delegation:[[:space:]]*(none|planner,implementer,verifier,reviewer|implementer,verifier,reviewer|planner,implementer,verifier|planner,verifier|planner,implementer|implementer,verifier|verifier,reviewer|implementer,reviewer|planner,reviewer|planner|implementer|verifier|reviewer)[[:space:]]*$' "$msg")"

  if [[ -z "$role" || -z "$depth" || -z "$delegation" ]]; then
    echo "Push blocked: commit ${sha} missing required lineage fields (AgentRole, AgentDepth, Delegation)." >&2
    exit 1
  fi

  if [[ "$role" == "orchestrator" && "$depth" != "0" ]]; then
    echo "Push blocked: commit ${sha} orchestrator must use AgentDepth: 0." >&2
    exit 1
  fi

  if [[ "$role" == "solo" && "$depth" != "0" ]]; then
    echo "Push blocked: commit ${sha} solo role must use AgentDepth: 0." >&2
    exit 1
  fi

  if [[ "$role" == "implementer" || "$role" == "verifier" || "$role" == "reviewer" ]]; then
    if [[ "$depth" != "1" ]]; then
      echo "Push blocked: commit ${sha} subagent role '${role}' must use AgentDepth: 1." >&2
      exit 1
    fi
    if [[ "$delegation" != "none" ]]; then
      echo "Push blocked: commit ${sha} subagent role '${role}' must declare Delegation: none." >&2
      echo "Incident template: docs/templates/agent-incident.md" >&2
      exit 1
    fi
    if ! grep -Eiq '^SpawnedAgents:[[:space:]]*0[[:space:]]*$' <<<"$msg"; then
      echo "Push blocked: commit ${sha} subagent role '${role}' must declare SpawnedAgents: 0." >&2
      exit 1
    fi
  fi

  if [[ "$role" == "orchestrator" && "$delegation" != "none" ]]; then
    if ! grep -Eiq '^SpawnBudget:[[:space:]]*[0-9]+[[:space:]]*$' <<<"$msg"; then
      echo "Push blocked: commit ${sha} delegating orchestrator must declare SpawnBudget." >&2
      exit 1
    fi
    if ! grep -Eiq '^MaxSpawnDepth:[[:space:]]*1[[:space:]]*$' <<<"$msg"; then
      echo "Push blocked: commit ${sha} delegating orchestrator must enforce MaxSpawnDepth: 1." >&2
      exit 1
    fi
  fi
done < <(git rev-list --reverse "$range")

exit 0
