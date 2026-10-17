# Laufwerke unter Windows: Bestand Dokany (Lesung)

Stand 2026-10-17, Lesestand da2646d2

Pfadangaben sind relativ zum Repository-Wurzelverzeichnis. `Datei:Zeile` nennt die Zeile der gelesenen Datei bei Commit da2646d2. Zitate sind wörtlich; englische Quelltexte bleiben unübersetzt. Annahmen sind mit „Annahme“ gekennzeichnet. Fehlende Tatsachen sind mit „Lücke“ und ihrer Kennung aus Abschnitt 5 benannt.

## 1. Ziel und Fragen

Ziel: belegter Bestand des Windows-Laufwerks-Backends Dokany. Die Lesung dient Strang S2 „Laufwerke und Einhängen je Betriebssystem“ und Ergebnis E10 in `docs/plaene/2026-10-17-rechte-desktop-tresore/plan.md`. E10 fragt, ob ein Cryptomator-Einhängeweg Dokany ersetzen und Zuverlässigkeit bringen kann. Diese Lesung bewertet nichts und empfiehlt nichts.

Nutzerwortlaut zu E10 (`plan.md:24`): „…see if any of these could replace donkey and add reliability or not…“

Annahme aus Plan A (`plan.md:28`): „donkey“ bezeichnet das Windows-Laufwerks-Backend, also Dokany. Der Bestand bestätigt das: `native/src/mount/os/windows/dokany_abi.rs:1` beschreibt die Datei als „Minimal Dokany 2.3.1 (library API 231, driver protocol `0x190`) ABI used by the dynamic runtime loader“.

Planfassung: `plan.md` hat sich während der Lektüre auf dem Datenträger geändert. Die E10-Zeile (Zeile 24) und die Annahme zu „donkey“ (Zeile 28) sind im geänderten Stand unverändert. Die übrigen Abschnitte des geänderten Plans sind nicht gelesen (Lücke L13).

Fragen:
- F1: Dokany-Version und -Variante, Installation, Aktualisierung, Rechte.
- F2: Callbacks, Zeitlimits und Timeout-Behandlung, Benachrichtigungen, Netzwerk- oder lokales Laufwerk, Ordner-Mounts.
- F3: bekannte Fehlerbilder aus Code-Kommentaren, GOTCHAS und TODO.
- F4: Stellen, die bei einem Wechsel auf ein Windows-Dateisystem-Frontend, das Dokany ersetzt (zum Beispiel WinFsp), geändert werden müssen.
- F5: Ergebnis und Entscheidung der früheren Cloud-Files-API-Untersuchung.

## 2. Befunde je Frage

### 2.1 F1: Version, Variante, Installation, Aktualisierung, Rechte

Version und Quelle:
- Library-API 231 und Treiberprotokoll `0x190` (dezimal 400): `dokany_abi.rs:1`, `dokany_abi.rs:11-12`. Die Zahlenwerte stehen in `native/src/mount` (`dokany_abi.rs:25`, `dokany_abi.rs:28`), diese Datei ist nicht gelesen (Lücke L1).
- ABI-Quellen laut Kommentar: Tag `v2.3.1.1000` von `dokan-dev/dokany` (`dokan/dokan.h`) und Commit `ce2ec8565591d21ed5b58f8233e9d81a730823ad` von `dokan-dev/dokan-rust` (MIT, Stand API 230), `dokany_abi.rs:6-9`.
- Dynamisches Laden statt Verlinkung. Begründung: „linking `dokan2.lib` would prevent every Smart Explorer executable from starting when Dokany is not installed“ (`dokany_abi.rs:4-5`). DLL-Name `dokan2.dll` (`dokany_abi.rs:29`).
- Eigene Typnachbildung der ABI (`DokanOptions`, `DokanFileInfo`, `DokanOperations`, Callback-Signaturen): `dokany_abi.rs:50-237`.
- Version im Manifest: `DOKANY_VERSION "2.3.1.1000"`, `DOKANY_API_VERSION "231"`, `DOKANY_DRIVER_PROTOCOL_VERSION "400"` (`native/dokany-runtime.nsh:4-8`).

Varianten (pro Prozess wird genau eine Laufzeit ausgewählt; eine davon ist privat):
- Offizielle System32-Variante: `LoadedModule::system32()` lädt mit `LoadLibraryExW(..., LOAD_LIBRARY_SEARCH_SYSTEM32)` (`runtime_loader.rs:26-35`). Einstieg: `DokanyRuntime::preflight()` (`runtime.rs:153-155`).
- Private Variante: `RuntimeSelection::select` nutzt sie nur bei `MountRuntimePreference::Auto` und nicht leerem `BUNDLED_DOKANY_BYTES` (`runtime_selection.rs:22`). Die Bytes landen unter `<cache_root>/private-runtime/<BUNDLED_DOKANY_SHA256>-<identity>/smart-explorer-dokan2.dll` (`private_payload.rs:50-59`), werden mit `create_new` geschrieben (`private_payload.rs:60-77`) und per SHA-256 geprüft (`private_payload.rs:87-98`). Nach dem Laden wird der tatsächliche Modulpfad gegen das gesperrte Objekt verglichen (`private_payload.rs:117-132`, `runtime_loader.rs:54-57`).
- Eingebettete Konstanten: `include!(concat!(env!("OUT_DIR"), "/private_dokany.rs"))` (`private_payload.rs:18`). Deklariert werden `BUNDLED_DOKANY_BYTES`, `BUNDLED_DOKANY_SHA256`, `BUNDLED_DOKANY_SOURCE`, `BUNDLED_DOKANY_SOURCE_ARCHIVE`, `BUNDLED_DOKANY_SOURCE_SHA256`. Alle fünf sind für reine Entwicklungsbuilds ohne private Variante leer (`native/dokany-private/README.md:142-146`). Die erzeugende Build-Stufe ist nicht gelesen (Lücke L2).
- Zusätzliches Symbol nur in der privaten Variante: `SmartExplorerBatchContinuationCountV1` (`runtime.rs:170-178`).
- Batching-Option `OPTION_ALLOW_IPC_BATCHING` (`1 << 12`, `dokany_abi.rs:36`): wird vor jeder Erzeugung entfernt (`dokany_abi.rs:84-91`) und nur in der privaten Variante bei `single_thread == 0` gesetzt (`runtime.rs:240-242`). Begründung im Kommentar: „Dokany 2.3.1 sends batching to the driver but truncates 0x1000 to BOOLEAN (u8) when choosing its consumer, dropping trailing events.“ (`dokany_abi.rs:85-87`). Nachweis und Entfernungsbedingung verweisen auf `docs/MOUNT_BATCHING.md` (`dokany_abi.rs:89`), die Datei ist nicht gelesen (Lücke L3).
- Rückfall auf die offizielle Variante: „private rejected ({error}); using official non-batched fallback“ (`runtime_selection.rs:33-34`) und „private attempt unavailable or unfinished ({error}); using official non-batched fallback“ (`runtime_selection.rs:38-39`). Scheitert die private Stufe, bevor ein Dateisystem existiert, erfolgt der Rückfall sofort (`runtime_selection.rs:31`).
- Die offizielle Variante erhält die Korrekturen der privaten Variante nicht: „The official-runtime fallback remains unchanged and does not inherit these private-library corrections.“ (`native/dokany-private/README.md:59-61`).

Private Variante: Herkunft und Rezept (`native/dokany-private/`):
- Selbstbeschreibung: „This is Smart Explorer's modified **user-mode DLL**, not an official Dokany release. It does not replace, rebuild, install or re-sign the shared System32 DLL, kernel driver or pinned official MSI.“ (`README.md:3-6`).
- Quelle: unveränderlicher Dokany-Commit `f1d5de68ff459af94e309cfdd171e4b8ca2af4dd`, den das README als „v2.3.1.1000“ bezeichnet (`README.md:10-11`). Die Gleichsetzung mit dem Tag ist nicht gegen Git geprüft (Lücke L14). Quelle und Prüfsummen: `recipe.json:3-7` (`source_commit`, `source_url`, `source_archive_sha256`, `patch_sha256`).
- Zielwerte in `recipe.json`: `dll_name` `dokan2.dll` (`:8`), `library_api` 231 (`:9`), `driver_protocol` 400 (`:10`), `machine` AMD64, `configuration` Release, `platform` x64 (`:11-13`), `platform_toolset` v143, `visual_studio_major` 17 (`:14-15`), `minimum_msvc_version` 14.30.0 und `minimum_sdk_version` 10.0.19041.0 (`:16-17`), `runtime_library` MultiThreaded (`:18`).
- Erlaubte Importe (vollständig, `recipe.json:19-24`): `advapi32.dll`, `kernel32.dll`, `shell32.dll`, `user32.dll`.
- Pflichtexporte (vollständig, `recipe.json:25-58`, 33 Namen): `DokanCloseHandle`, `DokanCreateFileSystem`, `DokanDebugMode`, `DokanDriverVersion`, `DokanGetMountPointList`, `DokanInit`, `DokanIsFileSystemRunning`, `DokanIsNameInExpression`, `DokanMain`, `DokanMapKernelToUserCreateFileFlags`, `DokanNetworkProviderInstall`, `DokanNetworkProviderUninstall`, `DokanNotifyCreate`, `DokanNotifyDelete`, `DokanNotifyRename`, `DokanNotifyUpdate`, `DokanNotifyXAttrUpdate`, `DokanNtStatusFromWin32`, `DokanOpenRequestorToken`, `DokanRegisterWaitForFileSystemClosed`, `DokanReleaseMountPointList`, `DokanRemoveMountPoint`, `DokanResetTimeout`, `DokanServiceDelete`, `DokanServiceInstall`, `DokanSetDebugMode`, `DokanShutdown`, `DokanUnmount`, `DokanUnregisterWaitForFileSystemClosed`, `DokanUseStdErr`, `DokanVersion`, `DokanWaitForFileSystemClosed`, `SmartExplorerBatchContinuationCountV1`.
- Die Laufzeit löst 14 Dokan-Funktionen auf (Abschnitt 3). Von den 32 Dokan-Namen der Pflichtexportliste werden 18 im Laufzeitcode nicht aufgelöst: `DokanDebugMode`, `DokanGetMountPointList`, `DokanMain`, `DokanMapKernelToUserCreateFileFlags`, `DokanNetworkProviderInstall`, `DokanNetworkProviderUninstall`, `DokanNotifyRename`, `DokanNotifyXAttrUpdate`, `DokanOpenRequestorToken`, `DokanRegisterWaitForFileSystemClosed`, `DokanReleaseMountPointList`, `DokanRemoveMountPoint`, `DokanServiceDelete`, `DokanServiceInstall`, `DokanSetDebugMode`, `DokanUnmount`, `DokanUnregisterWaitForFileSystemClosed`, `DokanUseStdErr`.
- Änderungen gegenüber dem Quellstand (`README.md:15-36`): Batching-/SingleThread-Optionen normalisieren; Batching-Maske vor der Umwandlung in den einbyte-großen `BOOLEAN`-Typ als Vergleich bilden; privater Zähler `SmartExplorerBatchContinuationCountV1`, Beschreibung: „this counter proves continuation dispatch, not successful remote writes.“ (`README.md:21-23`); Verzeichnisaufzählung ab dem Vektorindex fortsetzen; Metadaten-Fehlerstatus nicht mehr durch `STATUS_INVALID_PARAMETER` ersetzen (`README.md:28-30`); Gerätesschließung nur vom Worker signalisieren, der die Unmount-Benachrichtigung eingereiht hat (`README.md:31-33`); Threadpool-Arbeit und Schließungszulassung serialisieren (`README.md:34-36`).
- Auslöser der Korrekturen vom 2026-09-28: „after a native crash dump proved that a volume-broadcast callback returned into the already unloaded DLL“ (`README.md:38-39`). Ausgelieferte Bytes: „These exact bytes shipped in Smart Explorer 0.5.165“ (`README.md:45-46`).
- Lizenz: „Dokany's library is LGPL-3.0-or-later“ (`README.md:168`). Die Pflichten nach LGPL 4(d)(1) sind laut README nicht allein durch feste Byte-Bindung erfüllt: „Strict runtime byte pinning does not itself satisfy the replaceable-shared-library alternative in LGPL section 4(d)(1).“ (`README.md:178-180`).

Build- und Freigabeweg der privaten DLL (`native/prepare-dokany-private.ps1`, nur gelesen, nicht ausgeführt):
- Erzeugung nur auf dem Remote-Windows-Runner (Umgebungsvariablen `GITHUB_ACTIONS=true` oder `SMART_EXPLORER_REMOTE_RUNNER=1`): `prepare-dokany-private.ps1:303-305`. Die Prüfung `-VerifyOnly` läuft ohne Build (`:301-302`).
- Feste Konstanten gegen `recipe.json` (`:290-299`): Commit, Archiv-SHA-256, Archivgröße 728462, Patch-SHA, Quell-URL `codeload.github.com`, Toolset v143, x64, Release, MultiThreaded, Bibliothek API 231, Treiberprotokoll 400, AMD64, `dokan2.dll`, Visual Studio 17.
- PE-Prüfung ohne Laden der DLL (`:103-197`): AMD64 und PE32+ (`:114-122`), keine weitergeleiteten Exporte (`:167-169`), keine verzögerten Importe (`:187-189`), Importe nur aus `allowed_imports` (`:190-192`), Exportmenge gleich `required_exports` (`:193-195`).
- Manifest-Prüfung (`:221-240`): Provenienz, Toolchain, Payload-Hash, PE-Identität, optional erwarteter SHA-256 über `-ExpectedDllSha256`.
- Freigabe: `-RequireApproved` verlangt die Dateien unter `native/assets/dokany-private/` als committet und unverändert (`:204-215`, `git ls-files` und `git status`).
- Build: MSBuild mit `Release`, `x64`, `v143`, `RuntimeLibrary=MultiThreaded`, `ImportDirectoryBuildProps=false` und `ImportDirectoryBuildTargets=false` (`:386-392`).
- Quellarchiv: `HttpClient` mit `AllowAutoRedirect = false` (`:352-354`). Annahme aus dem Quelltext: der Download setzt eine direkte Erfolgsantwort voraus. Nicht ausgeführt.
- Ausgabe genau drei Dateien: `dokan2.dll`, `manifest.json`, `corresponding-source.zip` (`native/dokany-private/README.md:108`).

Circuit Breaker der privaten Variante:
- Marker `private-<BUNDLED_DOKANY_SHA256>.attempt` unter `<cache_root>/<Mount-ID>/` (`runtime_attempt.rs:31-32`), exklusiv mit `create_new` und `share_mode(0)` (`runtime_attempt.rs:33-42`).
- Entfernt wird er nur nach kontrolliertem Ende über `FileDispositionInfo` (`runtime_attempt.rs:49-60`), aufgerufen in `runtime_selection.rs:48-59` und `host.rs:175-177`.
- Selbstbeschreibung: „Conservative circuit breaker: an unfinished private attempt selects System32 on the next retry. It neither detects every hang nor replays pending writes.“ (`runtime_attempt.rs:1-2`).

Installation (`install_runtime`, `runtime_install.rs:13-48`):
1. `DokanyRuntime::preflight()`. Bei Erfolg: `AlreadyReady` (`runtime_install.rs:16-17`).
2. Installiert wird nur bei `RuntimeNotInstalled` oder `DriverUnavailable` (`runtime_install.rs:18-20`). Jeder Preflight-Fehler, der weder `RuntimeNotInstalled` noch `DriverUnavailable` ist, bricht ab mit „vorhandene Dokany-Laufzeit wird nicht automatisch ersetzt: {error}“ (`runtime_install.rs:21-25`).
3. Manifestwerte (`pinned_msi`), dann Download oder lokale Datei (`acquire_msi`) (`runtime_install.rs:28-29`).
4. `LockedMsi::open` (Größe und SHA-256), dann `verify_authenticode` (`runtime_install.rs:30-31`).
5. `run_elevated` (`runtime_install.rs:32`). Exitcode 0 führt zu einer erneuten Preflight-Prüfung. Scheitert sie, lautet die Meldung „Dokany-Installer war erfolgreich, aber DLL-API … ist danach nicht einsatzbereit“ (`runtime_install.rs:40-46`). Das Ergebnis liefert `DriveRuntimeInstallOutcome::from_msi_exit_code` (`runtime_install.rs:47`), dessen Definition ist nicht gelesen (Lücke L1).

Manifest und Download (`runtime_install_download.rs`):
- Manifest `native/dokany-runtime.nsh`, eingebettet mit `include_str!` (`:10`). Erforderliche Schlüssel (`:41-49`): `DOKANY_VERSION`, `DOKANY_API_VERSION`, `DOKANY_DRIVER_PROTOCOL_VERSION`, `DOKANY_MSI_FILENAME`, `DOKANY_MSI_URL`, `DOKANY_MSI_SIZE`, `DOKANY_MSI_SHA256`. Jeder Schlüssel muss genau einmal definiert sein (`:158-180`).
- Plausibilitätsprüfung (`:51-66`): API-Wert gleich `DOKANY_API_VERSION`, Treiberprotokoll gleich `DOKANY_DRIVER_PROTOCOL_VERSION`, Version mit genau vier numerischen Teilen, Dateiname gleich `Dokan_x64.msi`, URL gleich `https://github.com/dokan-dev/dokany/releases/download/v{version}/{file_name}`, Größe größer als 0, SHA-256 mit 64 Zeichen aus `0-9a-f`.
- Download (`:75-142`): nur HTTPS (`:95`), höchstens 2 Weiterleitungen (`:96`), Timeout 180 s (`:12`, `:97`), User-Agent `SmartExplorer-Dokany-Installer/1` (`:100`). Die Endadresse muss gleich der Quell-URL sein oder mit `https://release-assets.githubusercontent.com/` beginnen (`:11`, `:144-146`). Content-Length muss der Manifestgröße entsprechen (`:109-119`). Gelesen werden höchstens Größe plus 1 Byte (`:121-129`). Zieldatei: `<Temp>/smart-explorer/runtime-install/dokany-*.msi` (`:83-92`).
- Lokale Datei (Parameter `local`): wird ohne Download übernommen (`:75-81`). Die Prüfung in `LockedMsi::open` gilt auch für sie.

Prüfung der MSI (`runtime_install_process.rs`):
- `LockedMsi::open` (`:50-95`): Pfad muss absolut sein und darf keine `.`- und `..`-Komponenten enthalten (`:51-60`). Alle Elternordner bleiben mit `FILE_SHARE_READ` ohne Share-Delete offen (`:61-64`, `:202-241`). Begründung: „Keep every pathname ancestor open without share-delete so a same-user process cannot replace the verified path before elevation.“ (`:61-63`). Die Datei selbst wird mit `FILE_SHARE_READ` geöffnet (`:65-70`). Reparse-Punkte und Nicht-Dateien werden abgelehnt (`:74-76`). Größe (`:77-83`) und SHA-256 über den gesperrten Handle (`:84-89`, `:292-309`) müssen passen.
- `verify_authenticode` (`:97-138`): `WinVerifyTrust` mit `WTD_REVOKE_WHOLECHAIN`, `WTD_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT`, `WTD_DISABLE_MD2_MD4`, `WTD_UICONTEXT_INSTALL`, `WTD_UI_NONE` (`:107-112`). Akzeptiert wird nur der Status 0 (`:130-137`). Diese Datei prüft keinen Signaturinhaber (Publisher, Subject oder Thumbprint). Befund aus dem Quelltext dieser Datei.

Ausführung (`run_elevated`, `runtime_install_process.rs:140-199`):
- `ShellExecuteExW` mit Verb `runas`, Programm `<System32>\msiexec.exe` (`:142-146`), Parameter `/i "<MSI-Pfad>" /passive /norestart ADDLOCAL=DokanDriverFeature INSTALLDEVFILES=0` (`:329-341`).
- Warten: `WaitForSingleObject(process, INFINITE)` (`:177`). Eine Zeitgrenze fehlt in diesem Pfad.
- Abbruch der Rechteabfrage (Win32 1223) liefert `ElevatedLaunchError::Cancelled` (`:161-165`). `runtime_install.rs:34-35` macht daraus `Cancelled { code: 1223 }`.
- Exitcode über `GetExitCodeProcess` (`:191-198`).

Installer (`native/installer.nsi`):
- Kopf: „Smart Explorer — per-user NSIS installer. The optional Dokany driver section requests UAC separately; installing/updating the app itself remains per-user.“ (`:1-2`). Stille Installation mit Dokany: `/S /INSTALLDOKANY=1` (`:11`).
- Lizenzdateien unter `$INSTDIR\licenses\Dokany\`: `NOTICE.txt`, `LICENSE-GPL-3.0.txt`, `LICENSE-LGPL-3.0.txt`, `LICENSE-MIT.txt` (`:135-139`). Die Laufzeit-Lizenzdateien der privaten DLL (`runtime_notices.rs:9-11`) enthalten keine MIT-Datei. Das ist ein Unterschied zwischen Installer und Laufzeit.
- MSI-Einbettung: Standardquelle `target\installer-dependencies\<Version>\<Dateiname>` (`:39-40`), beim Setup in `$PLUGINSDIR` abgelegt (`:196`).
- Abschnitt „Dokany … für Remote-Laufwerke (UAC erforderlich)“ (`:188`). Vorab-Prüfung `CheckDokanyRuntime` (`:191`). Aufruf des CLI-Befehls `"$INSTDIR\${CLI_EXE_NAME}" drive install-runtime --msi "$PLUGINSDIR\${DOKANY_MSI_FILENAME}"` (`:201`). Kommentar: „It revalidates the pinned size/SHA-256 and Authenticode signature, keeps a no-write/no-delete handle open across elevation, and elevates only System32\msiexec.exe.“ (`:199-200`).
- Ergebnis-Codes des Aufrufs (`:204-208`): 0 prüft erneut, 3010 Neustart nötig, 1641 Neustart eingeleitet, 1223 und 1602 gelten als abgebrochen.
- Fehlertexte bei Fehlschlag (`:210-211`): „Dokany konnte nicht installiert werden (MSI-Exitcode $0). Smart Explorer wurde installiert, Remote-Laufwerke bleiben jedoch bis zu einer erfolgreichen Dokany-Installation deaktiviert.“ Im stillen Modus führen Fehlschlag, Abbruch und Verifikationsfehler zum Abbruch des Installers (`IfSilent dokany_abort`, `installer.nsi:210`, `:215`, `:242`; `SetErrorLevel 1` und `Abort` in `:219-221`). Neustart: `SetRebootFlag true` bei den Exitcodes 3010 und 1641 (`installer.nsi:225`, `:236`). Abbruch der UAC-Abfrage (`:216`): „Remote-Laufwerke benötigen weiterhin Dokany ${DOKANY_VERSION}.“
- Vorab-Prüfung (`:248-256`): `"$INSTDIR\${CLI_EXE_NAME}" drive runtime`, Exitcode 0 bedeutet bereit.
- Stille Installation ohne Opt-in (`:262-274`): der Abschnitt wird abgewählt, außer `/INSTALLDOKANY=1` ist gesetzt. Kommentar: „A silent app install must never introduce a hidden UAC prompt unless the caller explicitly opted in with /INSTALLDOKANY=1.“
- Deinstallation (`:314-341`): entfernt Lizenzdateien (`:326-329`) und Kontextmenüschlüssel (`:318-320`). Eine Deinstallation der Dokany-Laufzeit ist im sichtbaren Umfang nicht vorhanden.
- Kontextmenü „In Smart Explorer öffnen“ (HKCU, `:156-166`) für Verzeichnisse, Laufwerke (`Software\Classes\Drive\shell\${VERB}`, `:164-166`) und Verzeichnishintergründe. Die Laufwerksregel gehört zur Shell-Integration, nicht zu Dokany.

Fetch-Skripte (Build-Zeit, `native/fetch-dokany-runtime.ps1`, `native/fetch-dokany-runtime.sh`):
- Beide lesen das Manifest, prüfen Format, Größe und SHA-256 und legen die MSI unter `target/installer-dependencies/<Version>/<Dateiname>` ab (`fetch-dokany-runtime.ps1:64-66`, `fetch-dokany-runtime.sh:81-83`).
- PowerShell: `Invoke-WebRequest` mit bis zu vier Versuchen (`fetch-dokany-runtime.ps1:80-91`). Der Quelltext prüft die Endadresse der Weiterleitung nicht.
- Shell: `curl --proto '=https' --tlsv1.2 --fail --location --retry 4 --retry-all-errors` (`fetch-dokany-runtime.sh:108-109`). Der Quelltext prüft die Endadresse der Weiterleitung nicht.
- Beide Skripte prüfen nur Größe und SHA-256. Die Authenticode-Prüfung findet erst zur Laufzeit statt (`runtime_install_process.rs:97-138`). Befund aus dem Quelltext.

Rechte:
- Installation: UAC-Erhöhung über `runas` (`runtime_install_process.rs:145`). Abbruch der Abfrage: 1223 (`:163-164`). Annahme: die Erhöhung verlangt Administratorrechte. Der Quelltext nennt das nicht ausdrücklich.
- Setup: die Dokany-Sektion verlangt UAC (`installer.nsi:188`). Stille Installation ohne `/INSTALLDOKANY=1` startet keine UAC-Abfrage (`installer.nsi:262-274`).
- Laufzeit: „the Dokany driver is not installed, not running, or not reachable“ (`runtime.rs:81-84`). Die Rechte des Mount-Host-Prozesses sind in diesem Ordner nicht belegt (Lücke L4).
- Private DLL: die Datei bleibt mit `FILE_SHARE_READ` ohne Schreibfreigabe geöffnet (`private_payload.rs:60-82`). Nach dem Laden gilt: „Payload ownership denies file writes/deletion until this returns.“ (`runtime_loader.rs:101`).

Befunde aus GOTCHAS zur Installation (`docs/GOTCHAS.md:239-287`):
- Der NSIS-Installer bettet das geprüfte Dokany-MSI als standardmäßig gewählte, optionale Offline-Komponente ein. Portable und automatisch aktualisierte Nutzer erhalten dieselbe Version über die GUI oder über `se drive install-runtime` (`GOTCHAS.md:250-253`).
- „Keep the base install per-user and invoke UAC only for the system-wide MSI. Plain `/S` must skip it; only `/S /INSTALLDOKANY=1` opts a silent setup in. Never uninstall the shared Dokany runtime with Smart Explorer.“ (`GOTCHAS.md:253-256`).
- Pin laut GOTCHAS (`GOTCHAS.md:256-260`): Version `2.3.1.1000`, DLL-API 231, Treiberprotokoll 0x190/400, URL des Dokan-MSI, 9.269.248 Byte, SHA-256 `69ff8cb37bfec3a75921c85ffd1c6370b50a9ec4ecef2cf3a009d488dcbf5465`. Dieselben Werte stehen in `native/dokany-runtime.nsh:8-12`.
- Die GOTCHAS-Quellen sind laut Dokument am 2026-07-22 geprüft (`GOTCHAS.md:263`). Das Datum stammt aus dem Dokument und ist nicht erneut geprüft.

### 2.2 F2: Callbacks, Zeitlimits, Benachrichtigungen, Netzwerk oder lokal, Ordner-Mounts

Callback-Umfang: 25 Felder in `DokanOperations` (`dokany_abi.rs:211-237`). 23 sind belegt, `lock_file` und `unlock_file` sind `None` (`callbacks.rs:26-27`). Begründung im Kommentar: „Without DOKAN_OPTION_FILELOCK_USER_MODE, Dokany correctly owns file lock enforcement in the driver and never calls these fields.“ (`callbacks.rs:24-25`). Vollständige Liste: Abschnitt 3.

Zeitlimits und Timeout-Behandlung:
- `CALLBACK_TIMEOUT_MS = 300_000` (5 Minuten, `callback_status.rs:23`). Gesetzt als `DokanOptions.timeout` (`host.rs:201`).
- Verlängerung: Callbacks der Schutzklasse „long“ (Abschnitt 3) registrieren sich beim `CallbackTimeoutSupervisor` (`callback_status.rs:96-101`, `:165`). Ein Thread ruft im Abstand `RESET_INTERVAL` = 30 s `DokanResetTimeout(300_000, file_info)` auf (`callback_timeout.rs:16`, `:312-317`).
- Schlägt der Reset fehl, wird der Request als fehlgeschlagen markiert (`callback_timeout.rs:223-226`). Folgen: „Dokany could not extend a running remote filesystem request“ (`callback_status.rs:125-130`) bzw. „Dokany could not supervise a running finalization request“ (`callback_status.rs:165-173`), jeweils mit Abbruchflag (`callback_context.rs:140-142`).
- Annahme aus dem Code: Die Software begrenzt die Dauer eines „long“-Callbacks nicht. Solange der Reset Erfolg meldet, wird die Frist alle 30 s neu gesetzt. Ein Abbruch eines hängenden Callbacks ist in diesen Dateien nicht vorhanden. Ob Dokany selbst eine harte Obergrenze hat, ist nicht belegt (Lücke L5).
- Ausdrücklich im Code: „Timeout supervision is diagnostic after the operation has completed. It must never turn an acknowledged remote commit into a Windows failure that invites the application to replay the save or rename.“ (`callback_status.rs:135-137`).
- Panik-Behandlung: `catch_unwind` um Callback-Körper und Kontextauflösung (`callback_status.rs:57-77`, `:110-123`, `:195-201`). Bei Panik: Abbruchflag und `STATUS_UNHANDLED_EXCEPTION`, außer ein Ergebnis wurde bereits bestätigt (`callback_status.rs:74`).
- Langsame Callbacks: Schwelle `SLOW_CALLBACK` = 500 ms (`callback_status.rs:24`, `:132-134`, `:190-192`). Ausgabe `mount callback slow: class={class}, elapsed_ms={ms}` (`callback_context.rs:164-168`), höchstens 32 Ausgaben (`callback_context.rs:156-163`). Klassen: `long`, `short`, `finalize` (`callback_status.rs:133`, `:191`).
- Reset-Hinweis: `mount callback still running after {RESET_INTERVAL} ms` (`callback_timeout.rs:306-310`), einmal je Request und höchstens 32 Ausgaben je Supervisor (`callback_timeout.rs:17`, `:293-301`).
- Fehlerabbildung: `io_status` ordnet `io::ErrorKind` Win32-Codes zu (`callback_status.rs:215-248`). Nicht zugeordnete Arten werden zu `ERROR_INVALID_DATA` (`callback_status.rs:245`). Win32-Codes gehen über `DokanNtStatusFromWin32` (`callback_status.rs:114`, `runtime.rs:275-277`).
- Host-Kontrollschleife: 250 ms (`host.rs:29`, `host_lifecycle.rs:57`). Jeder Durchlauf fragt `filesystem.wait(0)` ab (`host_lifecycle.rs:68-82`).
- Schließen: `DokanCloseHandle` über `DokanyFileSystem::close` (`runtime.rs:334-349`). Beim Ablegen der Runtime ruft `RuntimeInner::drop` `DokanShutdown` auf (`runtime.rs:378-382`).
- Watchdog beim Schließen: Phasenschwelle 500 ms (`shutdown_watchdog.rs:6`). Ausgabe `mount shutdown still blocked: phase={phase}, elapsed_ms={ms}` (`shutdown_watchdog.rs:105-110`). Der Watchdog bricht nichts ab (`shutdown_watchdog.rs:82-122`).
- Metadaten-Auffrischung: alle 20 s (`metadata_refresh.rs:7`, `:54-78`).
- Endstatus beim Schließen: der Host setzt ihn, nicht der Callback `unmounted` (`callbacks_metadata.rs:188-190`).

Benachrichtigungen (`notifications.rs`):
- Quelle: `engine.drain_metadata_changes(128)` (`notifications.rs:7`, `:22`). Bei gelöschten Ordnern und geänderten Dateien wird der Inhalt ungültig gemacht (`notifications.rs:23-26`).
- Zustellung an Dokany: `DokanNotifyCreate` (`runtime.rs:291-293`), `DokanNotifyDelete` (`runtime.rs:295-297`), `DokanNotifyUpdate` (`runtime.rs:305-309`). Symbole: `runtime.rs:430-432`.
- Nicht angenommene Zustellung: die Liste bleibt erhalten, die Fehlermeldung lautet „Dokany remote-change notification was not accepted“ (`notifications.rs:42-46`). Nur die Host-Schleife stößt die Zustellung an (`notifications.rs:1`, `host_lifecycle.rs:83-91`). Die Meldung „mount change delivery deferred: {error}“ erscheint einmal auf stderr (`host_lifecycle.rs:85-89`).
- Pfadbildung: `mounted_path` prüft Wurzel und Kanonik (`notifications.rs:53-62`) und baut `X:\…` mit einer Grenze von 32.767 UTF-16-Einheiten (`notifications.rs:63-66`).

Netzwerk oder lokales Laufwerk:
- Das eingehängte Laufwerk ist ein lokales Laufwerk mit Laufwerksbuchstaben. `options.unc_name = null()` (`host.rs:204`). Eine Optionskonstante für Netzlaufwerke ist nicht definiert. Definiert sind nur `OPTION_WRITE_PROTECT` (`1 << 3`), `OPTION_MOUNT_MANAGER` (`1 << 6`), `OPTION_CURRENT_SESSION` (`1 << 7`), `OPTION_CASE_SENSITIVE` (`1 << 9`), `OPTION_ALLOW_IPC_BATCHING` (`1 << 12`) (`dokany_abi.rs:32-36`).
- Gesetzte Optionen (`host.rs:193-200`): immer `OPTION_CURRENT_SESSION | OPTION_MOUNT_MANAGER`, zusätzlich `OPTION_CASE_SENSITIVE` bei case-sensitiven Pfaden und `OPTION_WRITE_PROTECT` bei Nur-Lesen.
- Die Quelle des Laufwerks kann ein Netzwerk-Backend sein. Laut GOTCHAS wird „the selected backend root“ über Dokany-Callbacks als Laufwerksbuchstabe sichtbar (`docs/GOTCHAS.md:234-236`). Die Quellarten sind in `docs/GOTCHAS.md:335-336` genannt (plain SFTP, Local/UNC, WebDAV, FTP, Google Drive). Dieser Ordner enthält ihre Implementierung nicht. Der Mount-Host greift über `MountEngine` darauf zu (`host.rs:65-72`).
- Laufwerksbuchstabe (`host.rs:294-309`): ein konfigurierter Buchstabe (`DriveSelection::Letter`) muss frei sein (`GetLogicalDrives`). Automatisch: Kandidaten `D` bis `Z`, in der Reihenfolge `Z` bis `D`, nur freie Buchstaben (`host.rs:304-308`). Belegte Buchstaben werden beim Start übersprungen (`host.rs:252-274`).
- Mount-Punkt: `X:\` (`wide.rs:21-26`, `host.rs:246-247`). Der Callback `mounted` akzeptiert nur `X:` oder `X:\` (`callbacks_metadata.rs:163-186`, Prüfung `:172-179`). Fehlertext: „Dokany returned a non-drive mount point“ (`callbacks_metadata.rs:194-199`).

Ordner-Mounts:
- In diesem Ordner nicht vorhanden. Der Mount-Punkt ist ausschließlich ein Laufwerksbuchstabe (`callbacks_metadata.rs:172-179`, `host.rs:294-309`). Die Suche nach `folder` in den Dateien dieses Ordners liefert nur Testfälle und Kommentare (keine Ordner-Mount-Logik).

Volume-Eigenschaften (`callbacks_metadata.rs:113-161`, `host.rs:193-203`):
- Label: Standard „Smart Explorer“ (`callbacks_metadata.rs:125-127`), höchstens 32 Zeichen (`:130`).
- Dateisystemname: `SmartEx` (`:132`). Maximale Komponentenlänge: 255 (`:137-142`).
- Flags: `FILE_CASE_PRESERVED_NAMES`, `FILE_UNICODE_ON_DISK`, optional `FILE_CASE_SENSITIVE_SEARCH`, optional `FILE_READ_ONLY_VOLUME` (`:143-154`).
- Allocation-Unit und Sektorgröße: 4096 (`host.rs:202-203`).
- Seriennummer: FNV-1a-Hash der Mount-ID (`metadata.rs:90-92`, `:131-135`; Aufruf `host.rs:140`).
- Freier Platz: `GetDiskFreeSpaceExW` auf dem lokalen Cache-Pfad (`callbacks_metadata.rs:97-107`). Kommentar: „The local whole-file spool is a hard lower capacity boundary. Remote quotas are backend-specific and still surface on commit.“ (`callbacks_metadata.rs:97-98`).

### 2.3 F3: bekannte Fehlerbilder aus Code-Kommentaren, GOTCHAS und TODO

Suche nach `TODO`, `FIXME`, `XXX`, `HACK` in allen Dateien dieses Ordners: kein Treffer. Die Befunde stammen aus Kommentaren der Quelltexte, aus `native/dokany-private/README.md` und aus `docs/GOTCHAS.md`. Zitate sind wörtlich.

Code-Kommentare (Produktion):
1. Batching verwirft Ereignisse: `dokany_abi.rs:85-87`: „Dokany 2.3.1 sends batching to the driver but truncates 0x1000 to BOOLEAN (u8) when choosing its consumer, dropping trailing events.“
2. Schließen vor dem Ende der asynchronen Volume-Benachrichtigung: `runtime_loader.rs:73-75`: „Official 2.3.1 can return from close/shutdown before its asynchronous volume broadcast returns.“
3. Working-Directory-Öffnung: `handle_state.rs:108-115`: „Enforcing open-time compatibility here made every CreateProcess working-directory open fail (“Der Verzeichnisname ist ungültig”) as soon as any shell handle held a delete-capable grant on the folder.“
4. Reparse-Punkte: `callbacks_metadata.rs:228-230`: „Dokany would treat FILE_ATTRIBUTE_REPARSE_POINT as usable only when GetReparsePoint is implemented.“ Folge im Code: Links werden in `find_entries` ausgeblendet (`callbacks_metadata.rs:231-233`).
5. Sicherheitsabfragen: `callbacks_unsupported.rs:37-39`: „STATUS_NOT_IMPLEMENTED specifically asks Dokany to synthesize the current-user descriptor. STATUS_NOT_SUPPORTED would instead make common Explorer/application security probes fail outright.“
6. Zurückgenommene Löschanforderung: `callbacks_mutation.rs:52-53`: „Dokany sends a second callback with DeletePending cleared when Windows cancels a previously requested disposition.“
7. Öffnen statt Anlegen: `callbacks_open.rs:313-316`: „Dokany uses STATUS_OBJECT_NAME_COLLISION as the successful "opened rather than created" signal for the kernel's SUPERSEDE/OPEN_IF/OVERWRITE_IF dispositions“.
8. Kein Wiederholen nach Commit: `callbacks_io.rs:123-125`: „The backend already committed the atomic promotion. Returning an error would invite the application to retry a save whose remote namespace has already changed.“
9. Umbenennung nach Commit: `callbacks_mutation.rs:89-90`: „The remote namespace mutation already succeeded. Returning failure would invite a replay; stop and remount instead.“
10. Konflikt beim Schreiben: `callbacks_io.rs:128-139`: Konflikt ergibt `MountStatus::Conflict` und `WouldBlock` mit „remote file changed while the mounted copy was open“.
11. Circuit Breaker: `runtime_attempt.rs:1-2`: „It neither detects every hang nor replays pending writes.“ Ergänzend `runtime_selection.rs:46-47`: „Any other exit leaves the marker for compatibility mode on the next Retry.“
12. Wiederholung nach Verbindungsverlust: `host.rs:105-110`: „cached mounted-drive changes could not be retried safely: {error}; keep the cache and use Retry after connectivity is restored“.
13. Recovery-Zustand vor dem ersten Callback: `host.rs:152-154`: „Arm recovery ownership before Dokany may dispatch its first callback.“
14. Privates Batching-Fehlerbild in den Dokany-Korrekturen: `native/dokany-private/README.md:38-39`: „after a native crash dump proved that a volume-broadcast callback returned into the already unloaded DLL“.
15. Upstream-Metadatenfehler: `native/dokany-private/README.md:28-30`: „Preserve failed metadata callback statuses instead of replacing every failure with `STATUS_INVALID_PARAMETER`.“ Die offizielle Variante behält diesen Fehler (`README.md:59-61`).

Weitere Warnhinweise im Code ohne Fehlerbild: `runtime_install_process.rs:61-63` (Elternordner offen halten), `private_payload.rs:106-108` („Newly created empty directories can acquire a reparse point through attribute access despite sharing restrictions.“), `runtime.rs:157-158` („The task suite can require this path without silently succeeding against the official, non-batched runtime instead.“).

GOTCHAS (Abschnitt „Dokany remote-drive boundary“, `docs/GOTCHAS.md:218-442`):
- `GOTCHAS.md:220-227`: „Disable IPC batching before every filesystem creation. The official 2.3.1.1000 DLL truncates the `0x1000` option to a byte when selecting its consumer, while the driver receives batching enabled. Trailing requests in a batch are then lost.“
- `GOTCHAS.md:228-233`: „A no-follow open is not evidence of a link.“ Blanket flag rejection breaks `GetFileAttributesExW`; .NET Framework's fallback can hide the original error behind "file not found".
- `GOTCHAS.md:234-238`: „This is a real user-mode filesystem, not CfAPI.“ Der Text nennt Cryptomator als UX-Vorbild und stellt fest: „Smart Explorer owns the backend proxy and whole-file cache rather than a vault.“
- `GOTCHAS.md:429-432`: „Remote latency is application latency. Reads first materialize an entire file, and each changed flush uploads it entirely.“ und „supervised metadata and whole-file I/O callbacks can extend Dokany's timeout while pending. Ambiguous mutations are never replayed.“
- `GOTCHAS.md:433-435`: „The reported free space is the local spool's lower bound, not the remote quota.“
- `GOTCHAS.md:436-442`: „The surface is intentionally narrower than NTFS.“ Nicht unterstützt: Dateiattribute und Zeitstempel setzen, ACL-/Sicherheitsschreibvorgänge, alternative Datenströme, Öffnen über ID und echter Reparse-Punkt-Zugriff.
- `GOTCHAS.md:415-420`: „A host exit code is not an actionable diagnosis.“ Der Text verlangt, DLL-API- und Treiberprotokoll-Abweichungen getrennt zu melden.
- `GOTCHAS.md:381-388` und `:389-397`: Schreiben als Ganzdatei-Transaktion und Konfliktprüfung, die „reduce risk“ und keine universelle CAS ist.

TODO: `docs/TODO.md` ist nicht gelesen (Lücke L7).

### 2.4 F4: Stellen bei einem Wechsel des Frontends

Die vollständige Liste der gelesenen Stellen steht in Abschnitt 4. Zusammenfassung ohne Auslassung: Frontend-Grenze (Typen, Konstanten, Callbacks, Laufzeitladen), Installer und Lieferkette (Manifest, Fetch-Skripte, privates Rezept, Installer), Laufzeitprüfung, gespeicherte Laufwerke (Konfiguration außerhalb dieses Ordners, Lücke L9).

### 2.5 F5: frühere Cloud-Files-API-Untersuchung (CfAPI)

Gelesen: `docs/CFAPI_REVIEW.md` (vollständig), `docs/cfapi_review/` (Gliederung aller acht Dateien und die Schlussfolgerungsabschnitte, siehe Abschnitt 6).

Status und Geltungsbereich (wörtlich, `CFAPI_REVIEW.md:3-8`): „**Historical review:** this document audits a CfAPI provider implementation that existed during the 2026-06-16 investigation. Current `native/src` no longer contains `cfprovider.rs` or `cfsync.rs`; the active remote-open path is temp-copy + mtime watch + `Backend::open_write` save-back. Treat the issue register below as the evidence for why reviving a native Cloud Files provider must be a new, safety-fixed feature, not as current shipped behavior.“

Befund des Dokuments (`CFAPI_REVIEW.md:27-31`): „**The open path is now *mostly* correct for plain binary files, but the feature has three classes of defect — two that crash/corrupt (must-fix safety) and one that is architectural (cannot be fully fixed with this crate's defaults).**“

Speichern (`CFAPI_REVIEW.md:36-40`): „**SAVE-BACK is fundamentally broken for most editors:** the common "write temp → rename over original" atomic-save (Word, Excel, VS Code, Win11 Notepad) raises `CF_CALLBACK_TYPE_NOTIFY_RENAME`, which the crate defaults to `Err(NotSupported)` … This alone makes CfAPI editing unreliable.“

Lebenszyklus (`CFAPI_REVIEW.md:42-43` und `:137-138`): Der Provider existiert nur, solange die GUI läuft. „Consequence: hydration only works while Smart Explorer runs.“ Ein Panic in einem Callback „unwinds across `extern "system"` (no `catch_unwind` in `proxy.rs`) = UB.“

Entscheidung des Dokuments (`CFAPI_REVIEW.md:45-49`): „This validates the recommendation to make the **persistent-mirror/Temp mode the default** and treat CfAPI as experimental. The mirror approach sidesteps *every* issue below (no rename callback, no size/alignment contract, no 4 KiB blob, no NTFS-name panic, works with the app closed, handles Google-Docs export naturally).“

Historische Empfehlung (`CFAPI_REVIEW.md:185-190`): „make mirror/Temp the default; keep any revived CfAPI path explicitly experimental and only after the safety fixes (I1–I4, I7–I9) are applied so it can never crash the app. The current tree follows the mirror/temp direction and has no active CfAPI toggle. Whole-tree on-demand placeholders (CfAPI's one genuine advantage) only matter for browsing huge remote trees of large files — not the edit-a-file workflow driving this.“

Gegenstand der Entscheidung: CfAPI-Einhängung ist ein Sync-Root-Ordner unter dem Benutzerprofil, kein Laufwerksbuchstabe (`CFAPI_REVIEW.md:65-66`: `local_root` = `%USERPROFILE%\Smart Explorer\<san(label)>`). Diese Lage ist für die Frage E10 relevant und ist im Dokument selbst so beschrieben.

Schlussfolgerungen der Einzeldokumente (wörtlich):
- `w2_lifecycle.md` (Abschnitt „Cross-cutting severity summary“, `:541-end`): „The two must-fix items: **(Q1) implement `rename`/`delete` + re-mark in-sync so real editors can save**, and **(Q4) unregister CfAPI sync roots on uninstall** (plus reconnect at startup, Q3). Everything else is hardening on top of a sound watch-and-upload architecture.“
- `w2_redteam.md` (Abschnitt „Ranked 'most dangerous' — top 5“, `:530-end`): Platz 1 „Google Docs hydrate empty / cannot hydrate (declared size 0 vs N-byte export)“, Platz 2 „atomic-save editors are BLOCKED by our defaulted `rename`/`delete` callbacks (`NotSupported`)“, Platz 3 „special-char / NUL remote names break a whole folder“, Platz 4 „label sanitizer inconsistency“, Platz 5 „non-4KB-aligned mid-file required range → `STATUS_CLOUD_FILE_INVALID_REQUEST`, then `.unwrap()` panic across FFI = UB.“
- `w2_verify.md` („Summary of corrections / overstatements found“, `:391-end`): „No claim was found to be outright WRONG (no REFUTED verdicts). One claim (C) is overstated in its mechanism; two (A, B) have minor imprecisions that do not change the conclusions or recommended fixes.“
- `w1_placeholders.md` (Abschnitt 5, `:331-411`): ISSUE-1 bis ISSUE-6 mit Schweregraden RISK, RISK (BUG under adversarial input), RISK, RISK, FIDELITY, OK/minor.
- `w1_fetchdata.md` (Abschnitt (d), `:393-479`): BUG (fehlende 4096-Byte-Ausrichtung), BUG (kurzer Backend-Read), RISK (gesamter Bereich im RAM), FIDELITY (fehlender Fortschritt, Timeout 60 s).
- `w1_registration.md` (Abschnitt 6, `:328-end`): Sync-Root-Id-Kollisionen, 174-Zeichen-Grenze, Panic bei langem Label, Unregister bei bereits bestehender Registrierung.
- `w2_web_trace.md` (Abschnitt 4, `:421-499`): NEW-1 bis NEW-7.
- `w1_crate_map.md` (`:275-282`): Prioritäten #9 (Größenabweichung bei Google Docs), #6/#7 (Panic über FFI), #8 (Umstellung auf Full-Hydration).

Inhaltsverzeichnis `docs/cfapi_review/` (acht Dateien, ohne eigene Gliederungsdatei): `w1_crate_map.md`, `w1_fetchdata.md`, `w1_placeholders.md`, `w1_registration.md`, `w2_lifecycle.md`, `w2_redteam.md`, `w2_verify.md`, `w2_web_trace.md`. Die Überschriftenliste aller acht Dateien wurde gelesen (Suche nach Überschriften); sie ist in diesem Dokument nicht vollständig wiedergegeben. Der Quellenindex steht in `CFAPI_REVIEW.md:194-201`.

Für E10 festgehalten: Der Untersuchungsumfang lautet „every Cloud Files API (CfAPI) call that provider code made“ (`CFAPI_REVIEW.md:10-11`). Ein Ersatz eines Laufwerksbuchstaben-Backends ist darin nicht untersucht (Befund aus dem Umfang des Dokuments). Die Entscheidung war „mirror/Temp“ als Standard. CfAPI blieb „experimental“ (`CFAPI_REVIEW.md:185-190`). Ob der Stand heute so gilt, ist nicht geprüft (Lücke L17).

## 3. Liste der Callbacks

Quelle: `callbacks.rs:6-36`. Schutzklassen: „long“ heißt `guard_long_with_context` (Zeitüberwachung) oder `void_guard_long` (Callbacks ohne Rückgabe). „short“ heißt `guard_with_context` (`callback_status.rs:40-55`, `:141-152`).

| Nr | Dokany-Feld (`dokany_abi.rs`) | Registrierung (`callbacks.rs`) | Implementierung (Datei:Zeile) | Schutzklasse | Verhalten laut Code |
|---|---|---|---|---|---|
| 1 | `create_file` | `:8` | `callbacks_open.rs:44` | long | Optionen prüfen (`FILE_OPEN_BY_FILE_ID` ergibt `ERROR_NOT_SUPPORTED`, `callbacks_open.rs:359-361`); Metadaten lesen; Ordner anlegen oder öffnen; Datei verzögert öffnen; Dispositionen 0 bis 5 |
| 2 | `cleanup` | `:9` | `callbacks_mutation.rs:106` | long (void) | Löschung festschreiben oder Inhalt flushen |
| 3 | `close_file` | `:10` | `callbacks_mutation.rs:131` | long (void) | Löschung oder Flush, Handle entfernen, Engine schließen |
| 4 | `read_file` | `:11` | `callbacks_io.rs:15` | long | Lesen über die Engine; Ordner ergibt `ERROR_DIRECTORY` |
| 5 | `write_file` | `:12` | `callbacks_io.rs:42` | long | Anhängen oder Schreiben; Paging-Schreiben wird begrenzt |
| 6 | `flush_file_buffers` | `:13` | `callbacks_io.rs:93` | long | Engine-Flush; Konflikt ergibt `WouldBlock` |
| 7 | `get_file_information` | `:14` | `callbacks_metadata.rs:24` | long | `BY_HANDLE_FILE_INFORMATION` aus Handle oder Cache |
| 8 | `find_files` | `:15` | `callbacks_metadata.rs:54` | long | Eintragsliste aus dem Cache; `.` und `..` außer im Wurzelordner; Links ausgeblendet |
| 9 | `find_files_with_pattern` | `:16` | `callbacks_metadata.rs:67` | long | wie `find_files` mit Dokany-Mustervergleich (`is_name_in_expression`) |
| 10 | `set_file_attributes` | `:17` | `callbacks_unsupported.rs:11` | short | `ERROR_NOT_SUPPORTED` |
| 11 | `set_file_time` | `:18` | `callbacks_unsupported.rs:19` | short | `ERROR_NOT_SUPPORTED` |
| 12 | `delete_file` | `:19` | `callbacks_mutation.rs:14` | long | Löschanforderung oder deren Rücknahme (`delete_pending`) |
| 13 | `delete_directory` | `:20` | `callbacks_mutation.rs:21` | long | wie `delete_file`, für Ordner |
| 14 | `move_file` | `:21` | `callbacks_mutation.rs:61` | long | Umbenennung über Reservierung und `rename_with_shared_destination` |
| 15 | `set_end_of_file` | `:22` | `callbacks_io.rs:146` | long | Größe über `engine.truncate` setzen |
| 16 | `set_allocation_size` | `:23` | `callbacks_io.rs:165` | long | Kürzen nur, wenn die Zielgröße kleiner als die aktuelle Größe ist |
| 17 | `lock_file` | `:26` | keine (`None`) | keine | Sperren erzwingt der Treiber (`callbacks.rs:24-25`) |
| 18 | `unlock_file` | `:27` | keine (`None`) | keine | wie `lock_file` |
| 19 | `get_disk_free_space` | `:28` | `callbacks_metadata.rs:82` | short | `GetDiskFreeSpaceExW` auf dem Cache-Pfad |
| 20 | `get_volume_information` | `:29` | `callbacks_metadata.rs:113` | short | Label, Dateisystemname `SmartEx`, Flags |
| 21 | `mounted` | `:30` | `callbacks_metadata.rs:163` | short | setzt den Laufwerksbuchstaben und meldet `Mounted` |
| 22 | `unmounted` | `:31` | `callbacks_metadata.rs:188` | short | keine Aktion; der Host setzt den Endstatus (`:189-190`) |
| 23 | `get_file_security` | `:32` | `callbacks_unsupported.rs:29` | short | `STATUS_NOT_IMPLEMENTED`, damit Dokany den Sicherheitsdeskriptor des aktuellen Benutzers erzeugt |
| 24 | `set_file_security` | `:33` | `callbacks_unsupported.rs:47` | short | `ERROR_NOT_SUPPORTED` |
| 25 | `find_streams` | `:34` | `callbacks_unsupported.rs:57` | short | `ERROR_NOT_SUPPORTED` |

Summen: 25 Felder. 23 sind belegt, davon 14 mit Schutzklasse „long“ (Nr. 1, 2, 3, 4, 5, 6, 7, 8, 9, 12, 13, 14, 15, 16) und 9 mit „short“ (Nr. 10, 11, 19, 20, 21, 22, 23, 24, 25). 2 Felder sind `None` (Nr. 17, 18).

Die Operationstabelle wird in `callbacks.rs:6` gebaut. Der Kontext erreicht die Callbacks über `global_context` (`host.rs:205`, Auflösung in `callback_context.rs:198-218`).

Dokany-Symbole, die die Laufzeit auflöst (vollständig, `runtime.rs:415-432`, 14 Namen): `DokanInit`, `DokanShutdown`, `DokanVersion`, `DokanDriverVersion`, `DokanCreateFileSystem`, `DokanIsFileSystemRunning`, `DokanWaitForFileSystemClosed`, `DokanCloseHandle`, `DokanResetTimeout`, `DokanIsNameInExpression`, `DokanNtStatusFromWin32`, `DokanNotifyCreate`, `DokanNotifyDelete`, `DokanNotifyUpdate`. In der privaten Variante kommt `SmartExplorerBatchContinuationCountV1` hinzu (`runtime.rs:170-172`).

Verwendete Optionskonstanten (vollständig): `OPTION_WRITE_PROTECT`, `OPTION_MOUNT_MANAGER`, `OPTION_CURRENT_SESSION`, `OPTION_CASE_SENSITIVE`, `OPTION_ALLOW_IPC_BATCHING` (`dokany_abi.rs:32-36`).

## 4. Liste der wechselabhängigen Stellen

Bei einem Wechsel auf ein Windows-Dateisystem-Frontend, das Dokany ersetzt (zum Beispiel WinFsp), betrifft das folgende Stellen. Die Liste ist vollständig für die gelesenen Dateien. Ungelesene Aufrufer sind in Abschnitt 5 benannt.

A. Frontend-Grenze (Dokany-spezifische Typen, Konstanten, Callback-Signaturen, Laufzeit)
- `native/src/mount/os/windows/dokany_abi.rs`: gesamte Datei. Insbesondere Optionskonstanten (`:32-36`), `DokanOptions` (`:50-92`), `DokanFileInfo` (`:94-107`), Callback-Typen (`:109-207`), `DokanOperations` (`:209-237`), Rückgabecodes `SUCCESS` bis `VERSION_ERROR` (`:38-45`), `prepare_for_create` (`:84-91`).
- `native/src/mount/os/windows/runtime.rs`: `DokanyRuntime::preflight` (`:153-155`), `preflight_private` (`:159-166`), `initialize` (`:168-209`), `create_file_system_raw` (`:226-254`), `reset_timeout` (`:256-262`), `is_name_in_expression` (`:264-273`), `nt_status_from_win32` (`:275-277`), `DokanyFileSystem::{batch_continuation_count, notify_create, notify_delete, notify_update, is_running, wait, close}` (`:285-349`), `DokanyCreateError::from_code` (`:356-369`), `RuntimeInner::drop` (`:378-382`), `Api::resolve` (`:406-435`).
- `native/src/mount/os/windows/runtime_loader.rs`: `LoadedModule::system32` (`:26-35`), `private` (`:37-59`), `retain_official_callbacks_until_exit` (`:73-86`), `symbol` (`:88-96`), `load_error` (`:106-113`).
- `native/src/mount/os/windows/callback_status.rs`: `guard_*` und Win32-/NTSTATUS-Abbildung (`:40-55`, `:203-248`).
- `native/src/mount/os/windows/callback_timeout.rs`: Reset-Schleife mit `DokanResetTimeout` (`:312-317`).
- `native/src/mount/os/windows/callback_context.rs`: Feld `runtime: DokanyRuntime` (`:23`), Kontextauflösung über `options.global_context` (`:198-218`), `context_key` und `set_context_key` über `DokanFileInfo.context` und `is_directory` (`:220-242`).
- `native/src/mount/os/windows/callbacks.rs`: gesamte Datei, Operationstabelle (`:6-36`).
- `native/src/mount/os/windows/callbacks_open.rs`: `create_file` (`:44-112`), Dispositions- und Attributkonstanten (`:17-42`), `opened_existing` mit der Konvention `STATUS_OBJECT_NAME_COLLISION` (`:317-328`), `disposition_for_file` (`:330-353`), `validate_create_options` (`:355-370`).
- `native/src/mount/os/windows/callbacks_io.rs`, `callbacks_mutation.rs`, `callbacks_metadata.rs`, `callbacks_unsupported.rs`: alle Callback-Körper mit Dokany-Signaturen und `DokanFileInfo`-Feldern (`context`, `is_directory`, `delete_pending`, `paging_io`, `write_to_end_of_file`).
- `native/src/mount/os/windows/host.rs`: `run_mount_host` (`:47-180`), `CallbackStorage::new` mit den Optionen (`:190-213`), `select_drive` und `encode_mount_point` (`:244-249`), `start_on_available_drive` (`:252-274`), `drive_candidates` (`:294-309`).
- `native/src/mount/os/windows/host_lifecycle.rs`: `filesystem.wait`, `is_running`, `close` (`:68-82`, `:105`).
- `native/src/mount/os/windows/notifications.rs`: `notify_create`, `notify_delete`, `notify_update` (`:33-41`).
- `native/src/mount/os/windows/wide.rs`: `encode_mount_point` (`:21-26`), Format `X:\`.
- `native/src/mount/os/windows/metadata.rs`: Ausgabeformen `BY_HANDLE_FILE_INFORMATION` und `WIN32_FIND_DATAW` (`:20-88`). Die Typen sind Windows-generisch; die Funktionen sind Helfer der Dokany-Callbacks.
- `native/src/mount/os/windows/mod.rs`: Re-Exporte `dokany_abi::{...}` und `runtime::{...}` (`:39-51`).
- `native/src/mount/os/windows/runtime_notices.rs`: Lizenzdateien `NOTICE.txt`, `LICENSE-GPL-3.0.txt`, `LICENSE-LGPL-3.0.txt` über `include_bytes!` (`:9-11`) und deren Bereitstellung (`:22-34`).

B. Installation und Lieferkette
- `native/src/mount/os/windows/runtime_install.rs` (`:13-48`), `runtime_install_download.rs` (`:10-180`), `runtime_install_process.rs` (`:43-350`): Paketname `Dokan_x64.msi`, Feature `DokanDriverFeature`, Quell-URL-Muster `dokan-dev/dokany`, Manifestschlüssel.
- `native/src/mount/os/windows/private_payload.rs` (Eingebettete Konstanten über `OUT_DIR`, `:18`; Staging `:28-103`), `runtime_selection.rs` (`:17-44`, Schalter `:22`), `runtime_attempt.rs` (Marker `:31-32`).
- `native/dokany-runtime.nsh` (Manifest, `:4-12`): Quelle der Werte für `runtime_install_download.rs` (`:10`).
- `native/installer.nsi`: Abschnitte Dokany (`:38-40`, `:65-66`, `:135-139`, `:156-166`, `:188-290`, `:314-329`). Der Installer ruft den CLI-Befehl `drive install-runtime --msi` (`:201`) und `drive runtime` (`:251`).
- `native/fetch-dokany-runtime.ps1` (`:1-103`) und `native/fetch-dokany-runtime.sh` (`:1-122`): Build-Zeit-Download.
- `native/prepare-dokany-private.ps1` (`:1-419`): Erzeugung und Prüfung der privaten DLL.
- `native/dokany-private/recipe.json` (`:1-60`), `native/dokany-private/README.md` (`:1-193`).

C. Laufzeitprüfung
- `runtime.rs::DokanyRuntime::preflight` (`:153-155`) und die Versionsprüfung `validate_dokany_version_domains` (`:181-193`). Die Funktion liegt in `native/src/mount` (Lücke L1).
- `runtime_selection.rs::RuntimeSelection::select` (`:17-44`).
- `host.rs::preflight_runtime` (`:43-45`, `pub(crate)`) und der Status `MountStatus::RuntimeUnavailable` (`host.rs:94-96`).

D. Gespeicherte Laufwerke und Mount-Konfiguration
- `host.rs` liest `config.drive` (`:124`), `config.runtime_preference` (`:69`, `:89`), `config.label` (`:139`), `config.mode` (`:138`, `:151`), `config.metadata` und `config.cache` (`:67-68`) aus `MountRuntimeConfig` in `native/src/mount` (Lücke L1).
- `callbacks_metadata.rs::mounted` (`:163-186`) übernimmt den Laufwerksbuchstaben aus Dokany in den Kontext.
- Speicherung und Wiederherstellung gespeicherter Laufwerksbuchstaben: außerhalb dieses Ordners (Lücke L9).

E. Mount-Ebene außerhalb des Ordners (nicht gelesen, Aufrufstellen belegt)
- `MountEngine`, `MountStatus`, `DriveLetter`, `DriveSelection`, `MountRecovery`, `EntryCondition`, `OpenFileOptions`, `OpenDisposition`, `NamespaceOutcome`, `FlushOutcome`, `RenameOutcome`, `HandleId`, `DeleteToken`, `MetadataChange`, `CacheSpaceProbe`, `MountRuntimePreference`, `MountRuntimeConfig`, `DriveRuntimeInstallOutcome`, `DOKANY_LIBRARY_API_VERSION`, `DOKANY_DRIVER_PROTOCOL_VERSION`, `validate_dokany_version_domains`, `requests_maximum_allowed`, `maximum_allowed_full_grant`, `maximum_allowed_read_grant`, `windows_ordinal_key`, `spool::audit_recovery` (Lücke L1).

F. Tests und Harnesses (nur Testbezug, für ein Frontend-Wechsel-Inventar mit zu ändern)
- Die Harness-Dateien enthalten Dokany-Signaturen und Callback-Überschreibungen (`mounted_volume_task_trace.rs:33-36`, `optimization_volume_io.rs:173-178`, `optimization_script_callbacks.rs:24-42`, `vault_volume_errors.rs:17-25`). Sie müssen bei einem Frontend-Wechsel mit geändert werden (Lücke L10).

## 5. Lücken und offene Punkte

Jede Lücke bezeichnet die fehlende Tatsache genau.

- L1: Mount-Ebene außerhalb von `native/src/mount/os/windows/` ist nicht gelesen. Fehlende Tatsachen: die Zahlenwerte von `DOKANY_LIBRARY_API_VERSION` und `DOKANY_DRIVER_PROTOCOL_VERSION`; die Funktion `validate_dokany_version_domains`; `DriveRuntimeInstallOutcome::from_msi_exit_code`; die Typen `DriveSelection`, `DriveLetter`, `MountRuntimePreference`, `MountRuntimeConfig`, `MountEngine`.
- L2: Die Build-Stufe, die `OUT_DIR/private_dokany.rs` erzeugt (`native/dokany-private/README.md:128-131` nennt `build.rs` und `build_support/private_dokany.rs`), ist nicht gelesen. Fehlende Tatsache: die Prüflogik der Build-Stufe für die eingebetteten Bytes.
- L3: `docs/MOUNT_BATCHING.md` ist nicht gelesen. Fehlende Tatsache: der Nachweis für den Batching-Verlust und die Bedingung für das Entfernen des Schutzes.
- L4: Die Rechte und der Startweg des Mount-Host-Prozesses sind nicht belegt. Gelesen ist nur, dass der Host über `--mount-host <MountId>` gestartet wird (`docs/GOTCHAS.md:291-293`). `connect_mount_host` in `native/src/daemon` ist nicht gelesen.
- L5: Das Verhalten eines dauerhaft hängenden Callbacks ist nicht belegt. Der Code verlängert die Frist alle 30 s, bricht aber nicht ab. Ob Dokany selbst eine harte Obergrenze hat, ist nicht belegt.
- L6: `docs/GOTCHAS.md` ist nur im Abschnitt „Dokany remote-drive boundary“ (`:218-442`) gelesen. Die übrigen Abschnitte sind nicht gelesen. Die Mustersuche `dokany|winfsp|mount|laufwerk|drive letter` ergab in der Datei nur Treffer in `:218-440`.
- L7: `docs/TODO.md` ist nicht gelesen. Die Leseliste enthält sie nicht, Frage F3 nennt sie jedoch.
- L8: Nicht gelesen: `native/dokany-private/batching.patch` (Patch-Inhalt) und `native/dokany-private/LICENSE.LGPL-3.0.txt` sowie `LICENSE.GPL-3.0.txt` (Lizenztexte). Das Vorhandensein der Dateien unter `native/third-party/dokany/`, auf die `runtime_notices.rs:9-11` und `installer.nsi:135-139` verweisen, ist nicht geprüft.
- L9: Speicherung gespeicherter Laufwerke und deren Wiederherstellung nach einem Neustart liegen außerhalb von `native/src/mount/os/windows/`. Die Aufrufstellen sind in `host.rs` belegt, die Speicherung nicht.
- L10: Die Harness-Dateien (`mounted_volume_*`, `optimization_*`, `vault_*`) gehören zu `#[cfg(test)]`-Modulen (`host.rs:35-41`, `mounted_volume_task_tests.rs:14-19`, `optimization_volume_tests.rs:12-27`, `optimization_volume_io.rs:18-19`). Sie sind hier als Testharness eingeordnet. Ihr Verhalten im Produktivbetrieb ist daraus nicht belegt. Einige Harness-Stellen rufen `std::process::abort()` auf (`vault_volume_enumeration.rs:63-66`, `optimization_script_probe.rs:47-50`); das gilt nur für Tests.
- L11: Ob die Werte im Manifest `native/dokany-runtime.nsh` mit den Konstanten in `native/src/mount` übereinstimmen, ist nicht im Quelltext geprüft. Die Laufzeitprüfung verlangt die Übereinstimmung (`runtime_install_download.rs:53-54`). Die Konstanten selbst sind nicht gelesen (Lücke L1).
- L12: Die Dokany-Funktionen und -Strukturen sind nicht gegen die Dokany-Dokumentation oder die Header des Tags `v2.3.1.1000` geprüft. Die Lesung hat keine externen Quellen abgefragt. Alle Quellenaussagen stammen aus Kommentaren und Dateien im Repository.
- L13: `docs/plaene/2026-10-17-rechte-desktop-tresore/plan.md` hat sich während der Lektüre geändert. Die E10-Zeile (`:24`) und die Annahme zu „donkey“ (`:28`) sind im geänderten Stand unverändert. Die übrigen Abschnitte des geänderten Plans sind nicht gelesen.
- L14: Die Gleichsetzung des Commits `f1d5de68ff459af94e309cfdd171e4b8ca2af4dd` mit dem Tag `v2.3.1.1000` ist nur durch die Angabe in `native/dokany-private/README.md:10-11` belegt. Git-Tags sind nicht geprüft.
- L15: Die Build-Zeit-Download-Skripte (`fetch-dokany-runtime.ps1`, `fetch-dokany-runtime.sh`) prüfen die Endadresse nicht, die Laufzeit-Installation dagegen schon (`runtime_install_download.rs:144-146`). Ob diese Lücke im Build-Pfad praktische Folgen hat, ist nicht belegt; die Skripte prüfen den SHA-256 aus dem Manifest.
- L16: Ob die Abfrage `prepare-dokany-private.ps1:352-354` (`AllowAutoRedirect = false`) bei der tatsächlichen Codeload-Antwort funktioniert, ist nicht ausgeführt und nicht belegt (Annahme aus dem Quelltext).
- L17: Die Aussage in `CFAPI_REVIEW.md:187-188`, dass der aktuelle Baum „has no active CfAPI toggle“, ist eine Angabe des Dokuments vom 2026-06-16. Ob der heutige Stand dies bestätigt, ist nicht geprüft. Der Abschnitt „Current `native/src`“ (`CFAPI_REVIEW.md:4-5`) ist eine Angabe des Dokuments.
- L18: Die Gliederungen und Schlussfolgerungen der Einzeldokumente `docs/cfapi_review/` sind nur in Teilen gelesen (Überschriftenliste aller acht Dateien; Schlussfolgerungsabschnitte gemäß Abschnitt 6). Die übrigen Abschnitte sind nicht gelesen.
- L19: Die CLI-Befehle `se drive install-runtime` und `se drive runtime` (`installer.nsi:201`, `:251`) und die GUI-Aufrufer von `install_runtime` sind nicht gelesen. Ihre Ein- und Ausgaben sind für die Oberflächen-Inventur (Ausgaben als JSON, Vervollständigung, MCP) nicht belegt.
- L20: Die Wartezeit ohne Grenze in `runtime_install_process.rs:177` (`WaitForSingleObject(process, INFINITE)`) hat keine Gegenstelle in den gelesenen Dateien. Ob der Aufrufer ein Zeitlimit setzt, ist nicht belegt.

## 6. Grenzen der Lektüre

Gelesen (vollständig, sofern nicht anders angegeben):
- `docs/plaene/2026-10-17-rechte-desktop-tresore/plan.md`: Titel und Abschnitt A (Zeilen 1 bis 61, Lesung vor der Änderung). Nach der Änderung wurden die Zeilen 24, 28 und 29 bis 30 geprüft.
- Alle 47 Nicht-Test-Dateien unter `native/src/mount/os/windows/` (44 im Ordner selbst, 3 in Unterordnern): die Dateien des Ordners selbst sowie `callback_timeout/schedule.rs`, `handle_state/share_index.rs` und `handle_state/validation.rs`. Dazu die zehn Harness-Dateien, die nur über `cfg(test)`-Module eingebunden sind (`mounted_volume_fixture_backend.rs`, `mounted_volume_task_trace.rs`, `mounted_volume_checker_process.rs`, `optimization_volume_io.rs`, `optimization_script_probe.rs`, `optimization_script_callbacks.rs`, `vault_volume_enumeration.rs`, `vault_volume_byname.rs`, `vault_volume_errors.rs`, `vault_volume_metadata.rs`).
- Nicht gelesen wegen Testdatei-Namensmuster (12 Dateien): `bulk_timeout_task_tests.rs`, `schedule_task_tests.rs`, `bulk_handle_task_tests.rs`, `share_index_task_tests.rs`, `mount_options_task_tests.rs`, `mounted_volume_task_tests.rs`, `optimization_runtime_tests.rs`, `optimization_volume_tests.rs`, `optimization_watch_tests.rs`, `recovery_cache_task_tests.rs`, `vault_open_task_tests.rs`, `windows_remote_volume_task_tests.rs`. Die Einbindung über `#[path]` zeigt: `vault_open_task_tests.rs` wird als Harness unter `optimization_volume_tests.rs:24-25` eingebunden.
- `native/dokany-private/`: `README.md` und `recipe.json` vollständig. Die Lizenz- und Patchdateien sind nicht gelesen (Lücke L8). Die Binärdateien sind nicht gelesen.
- `native/dokany-runtime.nsh`, `native/fetch-dokany-runtime.ps1`, `native/fetch-dokany-runtime.sh`, `native/prepare-dokany-private.ps1`: vollständig.
- `native/installer.nsi`: Abschnitte zu Dokany und Laufwerken (`:1-12`, `:38-40`, `:65-66`, `:135-139`, `:156-166`, `:188-290`, `:314-329`). Die übrigen Abschnitte sind nicht gelesen. Die Suche nach `dokan`, `drive`, `laufwerk`, `INSTALLDOKANY`, `msi`, `runtime` (case-insensitiv) in der gesamten Datei hat keine weiteren Treffer außerhalb der gelesenen Abschnitte ergeben.
- `docs/CFAPI_REVIEW.md`: vollständig.
- `docs/cfapi_review/`: Überschriftenliste aller acht Dateien; Schlussfolgerungsabschnitte von `w1_placeholders.md`, `w1_fetchdata.md`, `w1_registration.md`, `w2_lifecycle.md`, `w2_redteam.md`, `w2_verify.md`, `w2_web_trace.md` und `w1_crate_map.md` gemäß den Zeilenangaben in Abschnitt 2.5 (Lücke L18).
- `docs/refs/windows-virtual-files.md`: vollständig. Der Inhalt betrifft Clipboard-Datenobjekte (Lazy virtual files für Zwischenablage und Drag-out), kein Laufwerks-Backend.
- `docs/GOTCHAS.md`: nur `:216-442` (Abschnitt „Windows remote-drive integration“ und „Dokany remote-drive boundary“).
- Suche nach Kommentaren mit `TODO`, `FIXME`, `XXX`, `HACK`, `folder`, `unc_name`, `OPTION_` und `DOKAN_` in den Dateien unter `native/src/mount/os/windows/`.

Nicht gelesen (außerhalb der Leseliste): `docs/TODO.md`, `docs/MOUNT_BATCHING.md`, `docs/ARCHITEKTUR.md`, `native/src/mount` außerhalb des Windows-Ordners, `native/src/daemon`, die Build-Stufe `build.rs` und `build_support/`, GUI- und CLI-Quellen.

Entscheidungen zur Lesemethode:
- Keine Builds, Tests, Compiler, cargo- oder Paketaufrufe und keine Server (AGENTS.md).
- Keine graphify-Abfrage. Die Leseliste dieses Auftrags beschränkt die Lektüre auf die genannten Dateien und Ordner.
- Keine Netzabfragen zu Dokany, WinFsp oder Microsoft-Dokumentation. Quellenangaben stammen aus dem Repository.
- Git: nur `git log -1 --oneline` und `git status --short` zur Prüfung des Lesestands (Dokumentations-Kontrollgang aus AGENTS.md).
- Mustersuchen wurden auf den Ordner `native/src/mount/os/windows/` und auf die Dateien der Leseliste beschränkt. Ausnahme: die Grep-Treffer in `docs/GOTCHAS.md` wurden nur für die Auswahl der Zeilen genutzt.
- Zitate sind wörtlich aus den gelesenen Zeilen übernommen; Auslassungen sind mit „…“ gekennzeichnet.
- Dieses Dokument enthält keine Geräte-, Konto- oder Pfaddaten des Nutzers.
