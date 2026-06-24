; Smart Explorer — per-user NSIS installer (no admin rights required).
;
; Build (Linux/WSL/macOS cross or Windows):
;   makensis -DVERSION=x.y.z installer.nsi
; Override the exe source when building natively on Windows:
;   makensis -DVERSION=x.y.z "-DEXE_SRC=target\release\smart_explorer.exe" installer.nsi
;   makensis -DVERSION=x.y.z "-DUPDATER_SRC=target\release\smart_explorer_updater.exe" installer.nsi
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
!ifndef UPDATER_SRC
  ; Default = the gnu cross-compile output (what CI / publish-feed.sh produce).
  !define UPDATER_SRC "target/x86_64-pc-windows-gnu/release/smart_explorer_updater.exe"
!endif

!define APP_NAME "Smart Explorer"
!define EXE_NAME "Smart Explorer.exe"
!define UPDATER_EXE_NAME "Smart Explorer Updater.exe"
!define VERB "OpenInSmartExplorer"
!define VERB_LABEL "In Smart Explorer öffnen"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\SmartExplorer"

Unicode true
Name "${APP_NAME} ${VERSION}"
OutFile "../release-native/Smart Explorer Setup ${VERSION}.exe"
Icon "${__FILEDIR__}\assets\smart-explorer-icon.ico"
UninstallIcon "${__FILEDIR__}\assets\smart-explorer-icon.ico"
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\Smart Explorer"
SetCompressor /SOLID lzma
ShowInstDetails nevershow
ShowUninstDetails nevershow

; Liability notice the user must accept before installing.
LicenseText "Bitte lesen und akzeptieren Sie die folgenden Hinweise."
LicenseData "${__FILEDIR__}\..\DISCLAIMER.txt"
LicenseForceSelection checkbox "Ich habe die Hinweise gelesen und akzeptiere sie"

Page license
Page directory
Page instfiles

Section "Install"
  SetOutPath "$INSTDIR"

  ; Close ALL running instances before touching the exe. The auto-updater renames
  ; the live binary ("Smart Explorer_old.exe") and can run a worker, so killing
  ; only "Smart Explorer.exe" misses them — and a process still holding a handle
  ; on the (already-deleted) exe makes Windows refuse to recreate it at the same
  ; path, which is the "Error opening file for writing" you can hit even when the
  ; folder looks empty. The IMAGENAME wildcard catches every variant.
  nsExec::Exec 'taskkill /F /T /FI "IMAGENAME eq Smart Explorer*"'
  nsExec::Exec 'taskkill /F /T /IM "smart_explorer.exe"'
  Sleep 1200

  ; Clear leftovers from a previous/interrupted auto-update so the fresh exe lands.
  Delete "$INSTDIR\Smart Explorer_old.exe"
  Delete "$INSTDIR\Smart Explorer_update_pending.exe"

  ; Write the exe with retries: if a handle is still releasing, wait + re-kill
  ; rather than dropping the user into Abort/Retry/Ignore. SetOverwrite try makes
  ; a failed File set the error flag instead of prompting.
  SetOverwrite try
  StrCpy $0 0
  write_exe:
    Delete "$INSTDIR\${EXE_NAME}"
    ClearErrors
    File "/oname=${EXE_NAME}" "${EXE_SRC}"
    IfErrors 0 write_done
    IntOp $0 $0 + 1
    IntCmp $0 6 write_failed
    Sleep 1000
    nsExec::Exec 'taskkill /F /T /FI "IMAGENAME eq Smart Explorer*"'
    nsExec::Exec 'taskkill /F /T /IM "smart_explorer.exe"'
    Goto write_exe
  write_failed:
    MessageBox MB_OK|MB_ICONSTOP "Konnte $INSTDIR\${EXE_NAME} nicht schreiben.$\r$\nBitte alle Smart-Explorer-Fenster schließen (ggf. im Task-Manager 'Smart Explorer' beenden) und die Installation erneut starten."
    Abort
  write_done:
  SetOverwrite on

  Delete "$INSTDIR\${UPDATER_EXE_NAME}"
  File "/oname=${UPDATER_EXE_NAME}" "${UPDATER_SRC}"

  File "${__FILEDIR__}\..\LICENSE"

  ; Best-effort Windows Defender Firewall rule for direct Share peer listeners.
  ; The app binds a dynamic local TCP port, so the rule is program-based.
  ; Managed machines may require admin/policy approval; the app also retries at
  ; Share startup and reports failure in diagnostics.
  nsExec::Exec 'netsh advfirewall firewall delete rule name="Smart Explorer Share Peer Listener"'
  nsExec::Exec 'netsh advfirewall firewall add rule name="Smart Explorer Share Peer Listener" dir=in action=allow program="$INSTDIR\${EXE_NAME}" enable=yes profile=any'

  ; Default update feed (Git/HTTPS) — keep an existing (possibly customized) one.
  ; update_source.txt ships the raw.githubusercontent feed URL, so a fresh
  ; install auto-updates from Git with no configuration.
  IfFileExists "$INSTDIR\update_source.txt" +2 0
    File "${__FILEDIR__}\update_source.txt"

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
  ; Kill every variant (see the install section) so the exe isn't left locked.
  nsExec::Exec 'taskkill /F /T /FI "IMAGENAME eq Smart Explorer*"'
  nsExec::Exec 'taskkill /F /T /IM "smart_explorer.exe"'
  Sleep 1000

  ; Undo shell integration via the app's own (reversible) restore BEFORE the
  ; exe is deleted, so folder-opening can't be left pointing at a missing file.
  nsExec::ExecToStack '"$INSTDIR\${EXE_NAME}" --unregister'
  Sleep 600
  ; Fallback: remove our uniquely-named context-menu verb keys directly (always
  ; safe — we fully own these).
  DeleteRegKey HKCU "Software\Classes\Directory\shell\${VERB}"
  DeleteRegKey HKCU "Software\Classes\Drive\shell\${VERB}"
  DeleteRegKey HKCU "Software\Classes\Directory\Background\shell\${VERB}"
  nsExec::Exec 'netsh advfirewall firewall delete rule name="Smart Explorer Share Peer Listener"'

  Delete "$INSTDIR\${EXE_NAME}"
  Delete "$INSTDIR\${UPDATER_EXE_NAME}"
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
