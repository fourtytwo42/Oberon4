#!/usr/bin/env bash
set -euo pipefail

git config core.hooksPath .githooks
chmod +x \
  .githooks/commit-msg \
  .githooks/pre-push \
  scripts/lib/common.sh \
  scripts/validate-commit-message.sh \
  scripts/validate-push.sh \
  scripts/validate-task-brief.sh \
  scripts/validate-spec-quality.sh \
  scripts/validate-agent-lineage.sh \
  scripts/policy-drift-audit.sh \
  scripts/exception-debt-report.sh \
  scripts/local-ci-gate.sh \
  scripts/oberon4-gate.sh \
  scripts/oberon4-security-gate.sh \
  scripts/check-policy-stack.sh \
  scripts/bootstrap-policy-stack.sh \
  scripts/policy-doctor.sh \
  scripts/risk-classifier.sh \
  scripts/sync-policy-kit.sh \
  scripts/build-context-pack.sh \
  scripts/snapshot-debug-brief.sh \
  scripts/security-gate.sh \
  scripts/run-agent.sh \
  scripts/install-drift-cron.sh \
  scripts/run-drift-watch.sh \
  scripts/reset-state.sh

echo "Installed local git hooks at .githooks"
echo "Current hooksPath: $(git config --get core.hooksPath)"
