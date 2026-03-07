#!/usr/bin/env bash
set -euo pipefail

source scripts/lib/common.sh
load_hooks_env

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SOURCE_DIR="$ROOT_DIR/policy-kit"
TARGET_DIR="${POLICY_KIT_REPO_DIR:-$HOME/bos-policy-kit}"
PUSH_CHANGES=0
ALLOW_DIRTY_TARGET=0
SKIP_BUILD=1
BUILD_SOURCE=0
COMMIT_MSG=""
TARGET_BRANCH="${POLICY_KIT_TARGET_BRANCH:-main}"

usage() {
  cat <<'USAGE'
Usage: scripts/sync-policy-kit.sh [options]

Synchronize BOS policy-kit/ into a standalone bos-policy-kit repository.

Options:
  --target <dir>          Target bos-policy-kit repo path
  --push                  Push target repo after commit
  --allow-dirty-target    Allow syncing when target repo has uncommitted changes
  --build-source          Run source policy-kit/build-payload.sh before sync (mutates BOS working tree)
  --skip-build            Skip payload refresh build step
  --message <text>        Custom commit message for target repo
  -h, --help              Show help
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target)
      TARGET_DIR="${2:-}"
      [[ -n "$TARGET_DIR" ]] || { echo "--target requires value" >&2; exit 2; }
      shift 2
      ;;
    --push)
      PUSH_CHANGES=1
      shift
      ;;
    --allow-dirty-target)
      ALLOW_DIRTY_TARGET=1
      shift
      ;;
    --skip-build)
      SKIP_BUILD=1
      shift
      ;;
    --build-source)
      BUILD_SOURCE=1
      SKIP_BUILD=0
      shift
      ;;
    --message)
      COMMIT_MSG="${2:-}"
      [[ -n "$COMMIT_MSG" ]] || { echo "--message requires value" >&2; exit 2; }
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown arg: $1" >&2
      usage
      exit 2
      ;;
  esac
done

if [[ ! -d "$SOURCE_DIR" ]]; then
  echo "Source policy-kit dir not found: $SOURCE_DIR" >&2
  exit 1
fi

if [[ ! -d "$TARGET_DIR/.git" ]]; then
  echo "Target is not a git repo: $TARGET_DIR" >&2
  echo "Clone bos-policy-kit first, or pass --target <repo-path>." >&2
  exit 1
fi

if [[ "$ALLOW_DIRTY_TARGET" != "1" ]]; then
  if [[ -n "$(git -C "$TARGET_DIR" status --porcelain)" ]]; then
    echo "Target repo has uncommitted changes: $TARGET_DIR" >&2
    echo "Commit/stash there first, or rerun with --allow-dirty-target." >&2
    exit 1
  fi
fi

rsync -a --delete --exclude '.git' "$SOURCE_DIR/" "$TARGET_DIR/"

if [[ "$SKIP_BUILD" != "1" && "$BUILD_SOURCE" == "1" ]]; then
  bash "$SOURCE_DIR/build-payload.sh"
  rsync -a --delete --exclude '.git' "$SOURCE_DIR/" "$TARGET_DIR/"
fi

git -C "$TARGET_DIR" add -A
if git -C "$TARGET_DIR" diff --cached --quiet; then
  echo "Policy kit sync: no changes to commit."
  exit 0
fi

if [[ -z "$COMMIT_MSG" ]]; then
  bos_sha="$(git -C "$ROOT_DIR" rev-parse --short HEAD)"
  COMMIT_MSG="Sync policy kit from BOS ${bos_sha}"
fi

git -C "$TARGET_DIR" commit -m "$COMMIT_MSG"

if [[ "$PUSH_CHANGES" == "1" ]]; then
  git -C "$TARGET_DIR" push origin "$TARGET_BRANCH"
fi

echo "Policy kit sync complete."
echo "Target: $TARGET_DIR"
echo "Push: $PUSH_CHANGES"
