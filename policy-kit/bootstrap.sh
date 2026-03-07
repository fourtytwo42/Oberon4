#!/usr/bin/env bash
set -euo pipefail

KIT_DIR="$(cd "$(dirname "$0")" && pwd)"
PAYLOAD_DIR="$KIT_DIR/payload"
TARGET_DIR="$(pwd)"
FORCE=0
PRESERVE_CONFIG=1
REPO_SLUG=""
RUN_INSTALL=1

usage() {
  cat <<USAGE
Usage: $0 [--target DIR] [--repo-slug owner/repo] [--force] [--no-install] [--no-preserve-config]
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target)
      TARGET_DIR="$2"; shift 2 ;;
    --repo-slug)
      REPO_SLUG="$2"; shift 2 ;;
    --force)
      FORCE=1; shift ;;
    --no-install)
      RUN_INSTALL=0; shift ;;
    --no-preserve-config)
      PRESERVE_CONFIG=0; shift ;;
    -h|--help)
      usage; exit 0 ;;
    *)
      echo "Unknown option: $1" >&2
      usage
      exit 2 ;;
  esac
done

if [[ ! -d "$PAYLOAD_DIR" ]]; then
  echo "Missing payload dir: $PAYLOAD_DIR" >&2
  exit 1
fi

mkdir -p "$TARGET_DIR"

copy_file() {
  local src="$1"
  local rel="$2"
  local dst="$TARGET_DIR/$rel"

  mkdir -p "$(dirname "$dst")"

  if [[ -e "$dst" ]]; then
    if [[ "$PRESERVE_CONFIG" == "1" && "$rel" == "scripts/hooks.env" ]]; then
      echo "preserve $rel"
      return 0
    fi
    if [[ "$FORCE" != "1" ]]; then
      echo "skip $rel (exists)"
      return 0
    fi
  fi

  cp "$src" "$dst"
  echo "write $rel"
}

while IFS= read -r src; do
  rel="${src#$PAYLOAD_DIR/}"
  copy_file "$src" "$rel"
done < <(find "$PAYLOAD_DIR" -type f | sort)

if [[ -n "${REPO_SLUG}" ]]; then
  env_file="$TARGET_DIR/scripts/hooks.env"
  if [[ -f "$env_file" ]]; then
    if grep -q '^HOOKS_REPO_SLUG=' "$env_file"; then
      sed -i "s|^HOOKS_REPO_SLUG=.*$|HOOKS_REPO_SLUG=${REPO_SLUG}|" "$env_file"
    else
      printf '\nHOOKS_REPO_SLUG=%s\n' "$REPO_SLUG" >> "$env_file"
    fi
    echo "set HOOKS_REPO_SLUG=$REPO_SLUG"
  fi
fi

if [[ "$RUN_INSTALL" == "1" ]]; then
  if [[ -f "$TARGET_DIR/scripts/install-git-hooks.sh" ]]; then
    (cd "$TARGET_DIR" && bash scripts/install-git-hooks.sh)
  else
    echo "warning: install script not found in target" >&2
  fi
fi

echo "Bootstrap complete for $TARGET_DIR"
