; LLM Hotspot NSIS installer
; Required defines passed by release-windows.ps1:
;   APP_DIR  - directory containing llmhotspot.exe + cloudflared.exe
;   VERSION  - app version (e.g. 0.1.0)
;   OUT_FILE - output setup exe absolute path

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"
!include "WordFunc.nsh"

!ifndef APP_DIR
  !error "APP_DIR define is required"
!endif
!ifndef VERSION
  !error "VERSION define is required"
!endif
!ifndef OUT_FILE
  !error "OUT_FILE define is required"
!endif

!define APP_NAME "LLM Hotspot"
!define PUBLISHER "LLM Hotspot"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\LLMHotspot"
!define APP_REG_KEY "Software\LLMHotspot"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"

!ifdef BRANDING_HEADER_BITMAP
  !define MUI_HEADERIMAGE
  !define MUI_HEADERIMAGE_RIGHT
  !define MUI_HEADERIMAGE_BITMAP "${BRANDING_HEADER_BITMAP}"
!endif

!ifdef BRANDING_WELCOME_BITMAP
  !define MUI_WELCOMEFINISHPAGE_BITMAP "${BRANDING_WELCOME_BITMAP}"
  !define MUI_UNWELCOMEFINISHPAGE_BITMAP "${BRANDING_WELCOME_BITMAP}"
!endif

Name "${APP_NAME}"
OutFile "${OUT_FILE}"
InstallDir "$PROGRAMFILES64\LLM Hotspot"
InstallDirRegKey HKLM "${APP_REG_KEY}" "InstallDir"
RequestExecutionLevel admin

Var PreviousVersion

!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_TEXT "Open LLM Hotspot now"
!define MUI_FINISHPAGE_RUN_FUNCTION LaunchLlmHotspot

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "terms.txt"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_UNPAGE_FINISH

!insertmacro MUI_LANGUAGE "English"

Function .onInit
  SetRegView 64
  nsExec::ExecToLog 'taskkill /IM "llmhotspot-shell.exe" /F /T'
  nsExec::ExecToLog 'taskkill /IM "llmhotspot.exe" /F /T'
  DeleteRegValue HKCU "${RUN_KEY}" "LLMHotspot"
  Delete "$LOCALAPPDATA\LLMHotspot\launch_llmhotspot.vbs"
  ReadRegStr $PreviousVersion HKLM "${UNINSTALL_KEY}" "DisplayVersion"
  ${If} $PreviousVersion != ""
    ${VersionCompare} "${VERSION}" "$PreviousVersion" $0
    ; VersionCompare: 0 equal, 1 left newer, 2 left older.
    ${If} $0 == 2
      MessageBox MB_ICONSTOP "A newer version ($PreviousVersion) is already installed.$\r$\nSetup ${VERSION} will exit."
      Abort
    ${EndIf}
  ${EndIf}
FunctionEnd

Function LaunchLlmHotspot
  Exec '"$INSTDIR\llmhotspot-shell.exe"'
FunctionEnd

Section "Install"
  SetRegView 64
  SetOutPath "$INSTDIR"

  File "${APP_DIR}\llmhotspot.exe"
  File "${APP_DIR}\llmhotspot-shell.exe"
  File "${APP_DIR}\cloudflared.exe"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  WriteRegStr HKLM "${APP_REG_KEY}" "InstallDir" "$INSTDIR"

  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayName" "${APP_NAME}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "UninstallString" "$\"$INSTDIR\Uninstall.exe$\""
  WriteRegStr HKLM "${UNINSTALL_KEY}" "QuietUninstallString" "$\"$INSTDIR\Uninstall.exe$\" /S"
  WriteRegDWORD HKLM "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINSTALL_KEY}" "NoRepair" 1

  ; Register custom protocol handler for deep-link callbacks.
  WriteRegStr HKCR "llmhotspot" "" "URL:LLM Hotspot Protocol"
  WriteRegStr HKCR "llmhotspot" "URL Protocol" ""
  WriteRegStr HKCR "llmhotspot\DefaultIcon" "" "$INSTDIR\llmhotspot-shell.exe,0"
  WriteRegStr HKCR "llmhotspot\shell" "" "open"
  WriteRegStr HKCR "llmhotspot\shell\open\command" "" "$\"$INSTDIR\llmhotspot-shell.exe$\" $\"%1$\""

  ; Start at user logon for all users on this machine.
  WriteRegStr HKLM "${RUN_KEY}" "LLMHotspot" "$\"$INSTDIR\llmhotspot-shell.exe$\""
SectionEnd

Section "Uninstall"
  SetRegView 64
  nsExec::ExecToLog 'taskkill /IM "llmhotspot-shell.exe" /F /T'
  nsExec::ExecToLog 'taskkill /IM "llmhotspot.exe" /F /T'
  Delete "$INSTDIR\llmhotspot.exe"
  Delete "$INSTDIR\llmhotspot-shell.exe"
  Delete "$INSTDIR\cloudflared.exe"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"

  DeleteRegKey HKLM "${UNINSTALL_KEY}"
  DeleteRegKey HKLM "${APP_REG_KEY}"
  DeleteRegKey HKCR "llmhotspot"
  DeleteRegValue HKCU "${RUN_KEY}" "LLMHotspot"
  DeleteRegValue HKLM "${RUN_KEY}" "LLMHotspot"
SectionEnd
