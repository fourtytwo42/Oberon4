#!/usr/bin/env bash
set -euo pipefail

nsi="installer/windows/llmhotspot.nsi"

require_line() {
  local text="$1"
  if ! rg -Fq "$text" "$nsi"; then
    echo "missing expected NSIS guard: $text" >&2
    exit 1
  fi
}

require_line '!define UPDATE_BACKUP_DIR ".update-backup"'
require_line 'CopyFiles /SILENT "$INSTDIR\llmhotspot.exe" "$INSTDIR\${UPDATE_BACKUP_DIR}\llmhotspot.exe"'
require_line 'IfErrors install_rollback'
require_line 'install_rollback:'
require_line 'CopyFiles /SILENT "$INSTDIR\${UPDATE_BACKUP_DIR}\llmhotspot.exe" "$INSTDIR\llmhotspot.exe"'
require_line 'MessageBox MB_ICONSTOP "Install/update failed. Previous version has been restored when available."'
require_line 'RMDir /r "$INSTDIR\${UPDATE_BACKUP_DIR}"'
require_line 'RMDir /r "$LOCALAPPDATA\LLMHotspot\plugins"'
require_line 'RMDir /r "$LOCALAPPDATA\LLMHotspot\updates"'
require_line 'RMDir /r "$LOCALAPPDATA\LLMHotspot"'

echo "ok: installer rollback + uninstall cleanup guards present"
