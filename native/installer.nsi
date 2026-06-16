; Smart Explorer — per-user NSIS installer (no admin rights required).
;
; Build (Linux/WSL/macOS cross or Windows):
;   makensis -DVERSION=x.y.z installer.nsi
; Override the exe source when building natively on Windows:
;   makensis -DVERSION=x.y.z "-DEXE_SRC=target\release\smart_explorer.exe" installer.nsi
; Silent install:  "Smart Explorer Setup x.y.z.exe" /S
;
; What it sets up so the app "just works":
;   * installs Smart Explorer.exe (per-user, %LOCALAPPDATA%\Programs)
;   * points auto-update at the Git feed (update_source.txt) on first install
;   * registers the "In Smart Explorer öffnen" right-click verb (HKCU, reversible)
;   * Start-menu + desktop shortcuts, Add/Remove Programs entry

!ifndef VERSION
  !define VERSION "0.5.3"
!endif
!ifndef EXE_SRC
  ; Default = the gnu cross-compile output (what CI / publish-feed.sh produce).
  !define EXE_SRC "target/x86_64-pc-windows-gnu/release/smart_explorer.exe"
!endif

!define APP_NAME "Smart Explorer"
!define EXE_NAME "Smart Explorer.exe"
!define VERB "OpenInSmartExplorer"
!define VERB_LABEL "In Smart Explorer öffnen"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\SmartExplorer"

Unicode true
Name "${APP_NAME} ${VERSION}"
OutFile "../release-native/Smart Explorer Setup ${VERSION}.exe"
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\Smart Explorer"
SetCompressor /SOLID lzma
ShowInstDetails nevershow
ShowUninstDetails nevershow

; Liability notice the user must accept before installing.
LicenseText "Bitte lesen und akzeptieren Sie die folgenden Hinweise."
LicenseData "../DISCLAIMER.txt"
LicenseForceSelection checkbox "Ich habe die Hinweise gelesen und akzeptiere sie"

Page license
Page directory
Page instfiles

Section "Install"
  SetOutPath "$INSTDIR"

  ; Close a running instance so the exe can be replaced
  nsExec::Exec 'taskkill /IM "${EXE_NAME}" /F'
  Sleep 400

  File "/oname=${EXE_NAME}" "${EXE_SRC}"
  File "../LICENSE"

  ; Default update feed (Git/HTTPS) — keep an existing (possibly customized) one.
  ; update_source.txt ships the raw.githubusercontent feed URL, so a fresh
  ; install auto-updates from Git with no configuration.
  IfFileExists "$INSTDIR\update_source.txt" +2 0
    File "update_source.txt"

  WriteUninstaller "$INSTDIR\Uninstall.exe"

  ; ── Right-click verb "In Smart Explorer öffnen" (per-user HKCU, reversible) ──
  ; Mirrors shell_register.rs: folders + drives use %1 (clicked item); the folder
  ; background uses %V (the open folder's own path). HKCU\Software\Classes is
  ; merged over the system classes with user priority.
  WriteRegStr HKCU "Software\Classes\Directory\shell\${VERB}" "MUIVerb" "${VERB_LABEL}"
  WriteRegStr HKCU "Software\Classes\Directory\shell\${VERB}" "Icon" '"$INSTDIR\${EXE_NAME}",0'
  WriteRegStr HKCU "Software\Classes\Directory\shell\${VERB}\command" "" '"$INSTDIR\${EXE_NAME}" "%1"'
  WriteRegStr HKCU "Software\Classes\Drive\shell\${VERB}" "MUIVerb" "${VERB_LABEL}"
  WriteRegStr HKCU "Software\Classes\Drive\shell\${VERB}" "Icon" '"$INSTDIR\${EXE_NAME}",0'
  WriteRegStr HKCU "Software\Classes\Drive\shell\${VERB}\command" "" '"$INSTDIR\${EXE_NAME}" "%1"'
  WriteRegStr HKCU "Software\Classes\Directory\Background\shell\${VERB}" "MUIVerb" "${VERB_LABEL}"
  WriteRegStr HKCU "Software\Classes\Directory\Background\shell\${VERB}" "Icon" '"$INSTDIR\${EXE_NAME}",0'
  WriteRegStr HKCU "Software\Classes\Directory\Background\shell\${VERB}\command" "" '"$INSTDIR\${EXE_NAME}" "%V"'

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
  ; safe — we fully own these).
  DeleteRegKey HKCU "Software\Classes\Directory\shell\${VERB}"
  DeleteRegKey HKCU "Software\Classes\Drive\shell\${VERB}"
  DeleteRegKey HKCU "Software\Classes\Directory\Background\shell\${VERB}"

  Delete "$INSTDIR\${EXE_NAME}"
  Delete "$INSTDIR\LICENSE"
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
