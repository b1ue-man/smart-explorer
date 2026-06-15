; Smart Explorer — per-user NSIS installer (no admin rights required).
; Build:  makensis /DVERSION=x.y.z installer.nsi
; Silent: "Smart Explorer Setup x.y.z.exe" /S

!ifndef VERSION
  !define VERSION "0.2.0"
!endif

!define APP_NAME "Smart Explorer"
!define EXE_NAME "Smart Explorer.exe"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\SmartExplorer"

Unicode true
Name "${APP_NAME}"
OutFile "..\release-native\Smart Explorer Setup ${VERSION}.exe"
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\Smart Explorer"
SetCompressor /SOLID lzma
ShowInstDetails nevershow
ShowUninstDetails nevershow

Page directory
Page instfiles

Section "Install"
  SetOutPath "$INSTDIR"

  ; Close a running instance so the exe can be replaced
  nsExec::Exec 'taskkill /IM "${EXE_NAME}" /F'
  Sleep 400

  File "/oname=${EXE_NAME}" "target\release\smart_explorer.exe"

  ; Default update feed — keep an existing (possibly customized) one
  IfFileExists "$INSTDIR\update_source.txt" +2 0
    File "update_source.txt"

  WriteUninstaller "$INSTDIR\Uninstall.exe"

  ; Shortcuts
  CreateDirectory "$SMPROGRAMS\${APP_NAME}"
  CreateShortcut "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk" "$INSTDIR\${EXE_NAME}"
  CreateShortcut "$DESKTOP\${APP_NAME}.lnk" "$INSTDIR\${EXE_NAME}"

  ; Add/Remove Programs entry (per-user)
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${APP_NAME}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "Silas"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\${EXE_NAME}"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
SectionEnd

; Launch the app after a normal (non-silent) install
Function .onInstSuccess
  IfSilent +2 0
    Exec '"$INSTDIR\${EXE_NAME}"'
FunctionEnd

Section "Uninstall"
  nsExec::Exec 'taskkill /IM "${EXE_NAME}" /F'
  Sleep 400

  ; Undo shell integration via the app's own (reversible) restore BEFORE the
  ; exe is deleted, so folder-opening can't be left pointing at a missing file.
  nsExec::ExecToStack '"$INSTDIR\${EXE_NAME}" --unregister'
  Sleep 600
  ; Fallback: remove our uniquely-named context-menu verb keys directly (always
  ; safe — we fully own these). The default-manager open keys are handled by
  ; --unregister above (which restores a prior handler correctly if present).
  DeleteRegKey HKCU "Software\Classes\Directory\shell\OpenInSmartExplorer"
  DeleteRegKey HKCU "Software\Classes\Drive\shell\OpenInSmartExplorer"
  DeleteRegKey HKCU "Software\Classes\Directory\Background\shell\OpenInSmartExplorer"

  Delete "$INSTDIR\${EXE_NAME}"
  Delete "$INSTDIR\Smart Explorer_old.exe"
  Delete "$INSTDIR\Smart Explorer_update_pending.exe"
  Delete "$INSTDIR\update_source.txt"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk"
  RMDir "$SMPROGRAMS\${APP_NAME}"
  Delete "$DESKTOP\${APP_NAME}.lnk"
  DeleteRegKey HKCU "${UNINST_KEY}"
SectionEnd
