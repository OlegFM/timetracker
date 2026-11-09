; Per-user installer of TimeTracker (NSIS 3, no administrator rights).
;
;   makensis /DVERSION=0.1.0 crates\tt-app\installer\timetracker.nsi
;
; makensis works in the folder of this file, so paths are relative to it: it packs
; ..\..\..\target\release\tt-app.exe and writes
; ..\..\..\target\installer\TimeTracker-<VERSION>-setup.exe. Installs into
; %LOCALAPPDATA%\Programs\TimeTracker, adds a Start menu shortcut and an entry in
; Settings > Apps. The uninstaller removes the program, the shortcut and the autostart
; entry; the data in %APPDATA%\TimeTracker and the logs stay.

Unicode true
SetCompressor /SOLID lzma
RequestExecutionLevel user
ManifestDPIAware true

!ifndef VERSION
  !error "pass /DVERSION=x.y.z"
!endif

!define APP "TimeTracker"
!define EXE "tt-app.exe"
!define ROOT "..\..\.."
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP}"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
!define APPROVED_KEY "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"

Name "${APP}"
OutFile "${ROOT}\target\installer\${APP}-${VERSION}-setup.exe"
InstallDir "$LOCALAPPDATA\Programs\${APP}"
InstallDirRegKey HKCU "${UNINST_KEY}" "InstallLocation"
!system 'mkdir "${ROOT}\target\installer" 2>nul'

VIProductVersion "${VERSION}.0"
VIAddVersionKey /LANG=0 "ProductName" "${APP}"
VIAddVersionKey /LANG=0 "FileDescription" "${APP} setup"
VIAddVersionKey /LANG=0 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=0 "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=0 "LegalCopyright" "All rights reserved"

!include "MUI2.nsh"
!define MUI_ICON "..\assets\app.ico"
!define MUI_UNICON "..\assets\app.ico"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE}"

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

; The first one is the fallback; otherwise the Windows display language is used.
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "Russian"
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "Spanish"

; A running copy locks the executable. Closing it is safe: a running timer is kept in
; the database and continues on the next start.
!macro CLOSE_APP
  nsExec::Exec 'taskkill /IM ${EXE} /F'
  Pop $0
  Sleep 500
!macroend

Section "Install"
  SetShellVarContext current
  !insertmacro CLOSE_APP
  SetOutPath "$INSTDIR"
  File "${ROOT}\target\release\${EXE}"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateShortcut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\${EXE}"

  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${APP}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\${EXE}"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
  SectionGetSize 0 $0
  WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" $0
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  !insertmacro CLOSE_APP
  Delete "$SMPROGRAMS\${APP}.lnk"
  Delete "$INSTDIR\${EXE}"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  ; The autostart entry would point at the removed executable.
  DeleteRegValue HKCU "${RUN_KEY}" "${APP}"
  DeleteRegValue HKCU "${APPROVED_KEY}" "${APP}"
  DeleteRegKey HKCU "${UNINST_KEY}"
SectionEnd

Function .onInit
  SetShellVarContext current
FunctionEnd

Function un.onInit
  SetShellVarContext current
FunctionEnd
