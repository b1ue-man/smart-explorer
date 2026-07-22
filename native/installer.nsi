; Smart Explorer — per-user NSIS installer. The optional Dokany driver section
; requests UAC separately; installing/updating the app itself remains per-user.
;
; Build (Linux/WSL/macOS cross or Windows):
;   makensis -DVERSION=x.y.z installer.nsi
; Override the exe source when building natively on Windows:
;   makensis -DVERSION=x.y.z "-DEXE_SRC=target\release\smart_explorer.exe" installer.nsi
;   makensis -DVERSION=x.y.z "-DUPDATER_SRC=target\release\smart_explorer_updater.exe" installer.nsi
;   makensis -DVERSION=x.y.z "-DCLI_SRC=target\release\se.exe" installer.nsi
; Silent install:  "Smart Explorer Setup x.y.z.exe" /S
; Silent install with Dokany: ... /S /INSTALLDOKANY=1
;
; What it sets up so the app "just works":
;   * installs Smart Explorer.exe (per-user, %LOCALAPPDATA%\Programs)
;   * makes the bundled se.exe available from new terminals via the user PATH
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
!ifndef CLI_SRC
  ; Default = the gnu cross-compile output (what CI / publish-feed.sh produce).
  !define CLI_SRC "target/x86_64-pc-windows-gnu/release/se.exe"
!endif
!ifndef INSTALLER_OUT
  !define INSTALLER_OUT "../release-native/Smart Explorer Setup ${VERSION}.exe"
!endif
!include "${__FILEDIR__}\dokany-runtime.nsh"
!ifndef DOKANY_MSI_SRC
  !define DOKANY_MSI_SRC "${__FILEDIR__}\target\installer-dependencies\${DOKANY_VERSION}\${DOKANY_MSI_FILENAME}"
!endif

!include "FileFunc.nsh"
!include "Sections.nsh"

!define APP_NAME "Smart Explorer"
!define EXE_NAME "Smart Explorer.exe"
!define UPDATER_EXE_NAME "Smart Explorer Updater.exe"
!define CLI_EXE_NAME "se.exe"
!define VERB "OpenInSmartExplorer"
!define VERB_LABEL "In Smart Explorer öffnen"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\SmartExplorer"

Unicode true
Name "${APP_NAME} ${VERSION}"
OutFile "${INSTALLER_OUT}"
Icon "${__FILEDIR__}\assets\smart-explorer-icon.ico"
UninstallIcon "${__FILEDIR__}\assets\smart-explorer-icon.ico"
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\Smart Explorer"
SetCompressor /SOLID lzma
ShowInstDetails nevershow
ShowUninstDetails nevershow

Var DokanyRuntimeReady
Var DokanyNeedsReboot

; Liability notice the user must accept before installing.
LicenseText "Bitte lesen und akzeptieren Sie die folgenden Hinweise."
LicenseData "${__FILEDIR__}\..\DISCLAIMER.txt"
LicenseForceSelection checkbox "Ich habe die Hinweise gelesen und akzeptiere sie"

Page license
Page components
Page directory
Page instfiles

Section "Smart Explorer (erforderlich)" SEC_MAIN
  SectionIn RO
  SetOutPath "$INSTDIR"

  ; Close ALL running instances before touching the exe. The auto-updater renames
  ; the live binary ("Smart Explorer_old.exe") and can run a worker, so killing
  ; only "Smart Explorer.exe" misses them — and a process still holding a handle
  ; on the (already-deleted) exe makes Windows refuse to recreate it at the same
  ; path, which is the "Error opening file for writing" you can hit even when the
  ; folder looks empty. The IMAGENAME wildcard catches every variant.
  nsExec::Exec 'taskkill /F /T /FI "IMAGENAME eq Smart Explorer*"'
  nsExec::Exec 'taskkill /F /T /IM "smart_explorer.exe"'
  nsExec::Exec 'taskkill /F /T /IM "se.exe"'
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

  Delete "$INSTDIR\${CLI_EXE_NAME}"
  File "/oname=${CLI_EXE_NAME}" "${CLI_SRC}"

  ; Use the Rust helper instead of NSIS string operations: normal NSIS builds
  ; can truncate a long PATH. The helper preserves the raw registry value type,
  ; adds exactly one component, records ownership, and broadcasts the change.
  ExecWait '"$INSTDIR\${CLI_EXE_NAME}" --install-cli-path' $1
  IntCmp $1 0 cli_path_done cli_path_failed cli_path_failed
  cli_path_failed:
    MessageBox MB_OK|MB_ICONEXCLAMATION "se.exe wurde installiert, aber der Benutzer-PATH konnte nicht aktualisiert werden.$\r$\nTerminal-Aufruf: $INSTDIR\${CLI_EXE_NAME}"
  cli_path_done:

  File "${__FILEDIR__}\..\LICENSE"
  SetOutPath "$INSTDIR\licenses\Dokany"
  File "/oname=NOTICE.txt" "${__FILEDIR__}\..\third-party\dokany\NOTICE.txt"
  File "/oname=LICENSE-GPL-3.0.txt" "${__FILEDIR__}\..\third-party\dokany\LICENSE-GPL-3.0.txt"
  File "/oname=LICENSE-LGPL-3.0.txt" "${__FILEDIR__}\..\third-party\dokany\LICENSE-LGPL-3.0.txt"
  File "/oname=LICENSE-MIT.txt" "${__FILEDIR__}\..\third-party\dokany\LICENSE-MIT.txt"
  SetOutPath "$INSTDIR"

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

Section "Dokany ${DOKANY_VERSION} / DLL-API ${DOKANY_API_VERSION} / Treiberprotokoll ${DOKANY_DRIVER_PROTOCOL_VERSION} für Remote-Laufwerke (UAC erforderlich)" SEC_DOKANY
  ; A runtime may have been installed while this installer was open. Recheck
  ; with Smart Explorer's exact library-API/driver-protocol probe before elevating.
  Call CheckDokanyRuntime
  StrCmp $DokanyRuntimeReady "1" dokany_done

  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File "/oname=${DOKANY_MSI_FILENAME}" "${DOKANY_MSI_SRC}"

  ; Use the same Rust path as the GUI and portable CLI. It revalidates the
  ; pinned size/SHA-256 and Authenticode signature, keeps a no-write/no-delete
  ; handle open across elevation, and elevates only System32\msiexec.exe.
  nsExec::ExecToStack '"$INSTDIR\${CLI_EXE_NAME}" drive install-runtime --msi "$PLUGINSDIR\${DOKANY_MSI_FILENAME}"'
  Pop $0
  Pop $1
  StrCmp $0 "0" dokany_verify
  StrCmp $0 "3010" dokany_reboot
  StrCmp $0 "1641" dokany_restart_initiated
  StrCmp $0 "1223" dokany_cancelled
  StrCmp $0 "1602" dokany_cancelled

  IfSilent dokany_abort 0
  MessageBox MB_OK|MB_ICONEXCLAMATION "Dokany konnte nicht installiert werden (MSI-Exitcode $0). Smart Explorer wurde installiert, Remote-Laufwerke bleiben jedoch bis zu einer erfolgreichen Dokany-Installation deaktiviert."
  Goto dokany_done

  dokany_cancelled:
    IfSilent dokany_abort 0
    MessageBox MB_OK|MB_ICONEXCLAMATION "Die Dokany-UAC-Abfrage wurde abgebrochen. Smart Explorer wurde installiert; Remote-Laufwerke benötigen weiterhin Dokany ${DOKANY_VERSION}."
    Goto dokany_done

  dokany_abort:
    SetErrorLevel 1
    Abort

  dokany_reboot:
    StrCpy $DokanyNeedsReboot "1"
    SetRebootFlag true
    Call CheckDokanyRuntime
    IfSilent dokany_done 0
    StrCmp $DokanyRuntimeReady "1" 0 +3
      MessageBox MB_OK|MB_ICONINFORMATION "Dokany ${DOKANY_VERSION} wurde installiert. Windows Installer meldet, dass ein Neustart erforderlich ist."
      Goto dokany_done
    MessageBox MB_OK|MB_ICONINFORMATION "Dokany ${DOKANY_VERSION} wurde installiert, ist aber erst nach einem Windows-Neustart verfügbar."
    Goto dokany_done

  dokany_restart_initiated:
    StrCpy $DokanyNeedsReboot "1"
    SetRebootFlag true
    Goto dokany_done

  dokany_verify:
    Call CheckDokanyRuntime
    StrCmp $DokanyRuntimeReady "1" dokany_done
    IfSilent dokany_abort 0
    MessageBox MB_OK|MB_ICONEXCLAMATION "Dokany meldete eine erfolgreiche Installation, aber Smart Explorer konnte DLL-API ${DOKANY_API_VERSION} / Treiberprotokoll ${DOKANY_DRIVER_PROTOCOL_VERSION} anschließend nicht bestätigen. Remote-Laufwerke bleiben deaktiviert; bitte Windows neu starten oder Dokany erneut installieren."

  dokany_done:
SectionEnd

Function CheckDokanyRuntime
  StrCpy $DokanyRuntimeReady "0"
  IfFileExists "$INSTDIR\${CLI_EXE_NAME}" 0 check_dokany_done
  nsExec::ExecToStack '"$INSTDIR\${CLI_EXE_NAME}" drive runtime'
  Pop $0
  Pop $1
  StrCmp $0 "0" 0 check_dokany_done
  StrCpy $DokanyRuntimeReady "1"
  check_dokany_done:
FunctionEnd

Function .onInit
  StrCpy $DokanyNeedsReboot "0"
  Call CheckDokanyRuntime
  StrCmp $DokanyRuntimeReady "1" dokany_already_ready

  ; Component sections are selected by default for an interactive setup. A
  ; silent app install must never introduce a hidden UAC prompt unless the
  ; caller explicitly opted in with /INSTALLDOKANY=1.
  IfSilent dokany_silent dokany_init_done
  dokany_silent:
    ${GetParameters} $0
    StrCpy $1 ""
    ${GetOptions} $0 "/INSTALLDOKANY=" $1
    StrCmp $1 "1" dokany_init_done
    !insertmacro UnselectSection ${SEC_DOKANY}
    Goto dokany_init_done

  dokany_already_ready:
    !insertmacro UnselectSection ${SEC_DOKANY}
    SectionSetText ${SEC_DOKANY} "Dokany ${DOKANY_VERSION} / DLL-API ${DOKANY_API_VERSION} / Treiberprotokoll ${DOKANY_DRIVER_PROTOCOL_VERSION} (bereits passend installiert)"
  dokany_init_done:
FunctionEnd

; Launch the app after a normal (non-silent) install
Function .onInstSuccess
  StrCmp $DokanyNeedsReboot "1" launch_done
  IfSilent launch_done 0
    Exec '"$INSTDIR\${EXE_NAME}"'
  launch_done:
FunctionEnd

Section "Uninstall"
  ; Kill every variant (see the install section) so the exe isn't left locked.
  nsExec::Exec 'taskkill /F /T /FI "IMAGENAME eq Smart Explorer*"'
  nsExec::Exec 'taskkill /F /T /IM "smart_explorer.exe"'
  nsExec::Exec 'taskkill /F /T /IM "se.exe"'
  Sleep 1000

  ; Undo shell integration via the app's own (reversible) restore BEFORE the
  ; exe is deleted, so folder-opening can't be left pointing at a missing file.
  nsExec::ExecToStack '"$INSTDIR\${EXE_NAME}" --unregister'
  Sleep 600
  ; Remove only the exact PATH component that this installer recorded as its
  ; own, leaving user-added and similarly named entries untouched.
  ExecWait '"$INSTDIR\${CLI_EXE_NAME}" --uninstall-cli-path' $1
  IntCmp $1 0 cli_path_removed cli_path_remove_failed cli_path_remove_failed
  cli_path_remove_failed:
    MessageBox MB_OK|MB_ICONSTOP "Der Smart-Explorer-Eintrag konnte nicht sicher aus dem Benutzer-PATH entfernt werden. Die Deinstallation wurde beendet, bevor se.exe gelöscht wurde.$\r$\nBitte erneut versuchen oder ausführen: $INSTDIR\${CLI_EXE_NAME} --uninstall-cli-path" /SD IDOK
    Abort
  cli_path_removed:
  ; Fallback: remove our uniquely-named context-menu verb keys directly (always
  ; safe — we fully own these).
  DeleteRegKey HKCU "Software\Classes\Directory\shell\${VERB}"
  DeleteRegKey HKCU "Software\Classes\Drive\shell\${VERB}"
  DeleteRegKey HKCU "Software\Classes\Directory\Background\shell\${VERB}"
  nsExec::Exec 'netsh advfirewall firewall delete rule name="Smart Explorer Share Peer Listener"'

  Delete "$INSTDIR\${EXE_NAME}"
  Delete "$INSTDIR\${UPDATER_EXE_NAME}"
  Delete "$INSTDIR\${CLI_EXE_NAME}"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\licenses\Dokany\NOTICE.txt"
  Delete "$INSTDIR\licenses\Dokany\LICENSE-GPL-3.0.txt"
  Delete "$INSTDIR\licenses\Dokany\LICENSE-LGPL-3.0.txt"
  Delete "$INSTDIR\licenses\Dokany\LICENSE-MIT.txt"
  RMDir "$INSTDIR\licenses\Dokany"
  RMDir "$INSTDIR\licenses"
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
