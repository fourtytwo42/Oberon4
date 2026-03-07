#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/checkpoint.sh --phase <phase-id> --summary "<short summary>" [--commit]

Examples:
  scripts/checkpoint.sh --phase batch3-build --summary "Batch 3 build checkpoint"
  scripts/checkpoint.sh --phase batch3-verify --summary "Batch 3 verify gate" --commit

Behavior:
  - creates/updates docs/handoffs/handoff-<phase-id>.md
  - stages all changes except runtime orchestration artifacts
  - writes commit message template to .tmp/checkpoint-commit-<phase-id>.txt
  - with --commit, creates commit using that message file
EOF
}

phase=""
summary=""
do_commit=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --phase) phase="${2:-}"; shift 2 ;;
    --summary) summary="${2:-}"; shift 2 ;;
    --commit) do_commit=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *)
      echo "Unknown arg: $1" >&2
      usage
      exit 2
      ;;
  esac
done

if [[ -z "$phase" || -z "$summary" ]]; then
  echo "--phase and --summary are required." >&2
  usage
  exit 2
fi

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$repo_root" ]]; then
  echo "Not inside a git repository." >&2
  exit 1
fi
cd "$repo_root"

handoff_dir="docs/handoffs"
handoff_path="$handoff_dir/handoff-${phase}.md"
mkdir -p "$handoff_dir" ".tmp" ".agent-orchestration"

if [[ ! -f "$handoff_path" ]]; then
  cat > "$handoff_path" <<EOF
# Handoff: ${phase}

## Summary
${summary}

## Scope Completed
- 

## Validation Evidence
- command:
- result:

## Open Risks / Follow-ups
- 
EOF
fi

commit_msg_file=".tmp/checkpoint-commit-${phase}.txt"
cat > "$commit_msg_file" <<EOF
Summary: ${summary}
Type: maintenance
Risk: medium
Stack: rust
TestScope: unit,integration,smoke
Verification:
\$ <fill in verification command(s)>
Output: <fill in key result>
WorkflowType: collaboration
WorkflowMode: incremental-gates
TaskState: done
NoNewFacts: yes
AgentRole: orchestrator
StackProfile: polyglot
AgentDepth: 0
Delegation: planner,implementer,verifier,reviewer
SpawnedAgents: <fill>
Invocation: run-agent.sh
PhaseCheckpoint: ${handoff_path}
EOF

git add -A

# Never include runtime/control plane churn in checkpoint commits.
for pathspec in \
  ".agent-runtime" \
  ".agent-orchestration" \
  ".tmp" \
  "tmp" \
  "/tmp"; do
  git restore --staged -- "$pathspec" 2>/dev/null || true
done

echo "Prepared checkpoint:"
echo "- phase: $phase"
echo "- handoff: $handoff_path"
echo "- commit template: $commit_msg_file"
echo
git status --short

if [[ "$do_commit" == "1" ]]; then
  git commit -F "$commit_msg_file"
  echo
  echo "Committed with checkpoint template."
fi
