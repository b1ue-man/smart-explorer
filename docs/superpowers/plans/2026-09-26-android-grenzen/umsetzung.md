# Android-Grenzen – Umsetzung

Spec: `spec.md` · Recherche: `recherche.md`. Regeln: AGENTS.md (keine lokalen Builds/Tests, Dateien
< 500 Zeilen, `core/` plattformfrei, Tests mit Präfix `android_task_`, eine Remote-Suite).

## Blöcke

### R Code-Review (fertig)
- 89 Befunde, 84 bestätigt, in sechs Dateigruppen behoben (`80fe206`), Graph `7e169b2`.

### G1–G3 Ersetzen/Hochladen SFTP, WebDAV, FTP (fertig)
- `77d6e0e`: `vfs::promote_staged_with`, `sftp/core/posix_rename.rs`, WebDAV `promote_staged`,
  `ftp/core/staging.rs`. Fertig, wenn: Gerätetests „plain SFTP ersetzt“, FTP Upload/Ersetzen grün.

### G4a Exec-Anbieter Android (nativ) – Subreaper je Job (recherche.md E2)
- Dateien: `native/src/share/os/linux_os/exec_supervisor.rs` (bleibt dort; `run(stream, SpawnPolicy)`
  mit `default_shell`, `own_process_group`, optionalem Zwischenprozess; Linux ruft mit den heutigen
  Werten auf), `share/os/linux_os/exec.rs` (Aufruf mit Linux-Policy, sonst unverändert),
  `share/os/android/exec.rs` (Anbieter `android-subreaper`), neu `share/os/android/exec_contain.rs`
  (Zwischenprozess: `pre_exec` → `prctl(PR_SET_CHILD_SUBREAPER)` + rohes `clone(SIGCHLD)`, Status-Pipe,
  `waitpid(-1)` bis ECHILD; `terminate_all`/`confirm_empty`; Job-Wurzel-Datei und Aufräumen beim Start),
  neu `share/os/android/exec_proc.rs` (reines Parsen von `/proc/<pid>/stat` ab der letzten `)`,
  Nachfahren aus einer Tabelle, Startzeit-Abgleich) + `exec_proc_tests.rs` (Host-Linux, nur reine
  Logik, keine echten Kill-/Reap-Tests im gemeinsamen Testprozess), `share/mod.rs` (nur cfg/`#[path]`
  der Exec-Module).
- Keine Ein-Job-Grenze (Registry-Standardgrenzen); `provider_status.available` nur, wenn
  `/system/bin/sh` ausführbar ist; `detail` nennt die Android-Grenzen.
- Fertig, wenn: Host-Tests der `/proc`-Logik grün; Linux-Desktop unverändert (gleiche Policy-Werte);
  Gerätetest `ShareExecTaskTest` Schritte 2–4 ohne übrig gebliebene `sleep`-Prozesse, Hook-Job grün.

### G4b Exec-Host Fassade und Oberfläche
- Nativ: neu `native/src/share/core/exec_targets.rs` (egui-freie `exec_target_views(profiles)` und
  `resolve_exec_target(profiles, key)` aus `app/core/share_exec_ui.rs`, dort genutzt),
  `share/mod.rs` (Modulzeile + Re-Export), `app/core/share_exec_ui.rs` (nutzt die geteilte Funktion,
  Verhalten unverändert), neu `mobile/os/shared/domains/share_exec.rs` (+ `share_exec_tests.rs`),
  `domains/mod.rs` (Dispatch), `domains/share_status.rs`, `domains/share_state.rs` (`StatusInput`),
  `domains/tests.rs` (angepasste Konstruktionen).
- Methoden (Vertrag mit Suite T, `api.md` §5 vom Hauptagent):
  - `share.status` erhält `execProvider:{available, provider, detail}` und
    `execTargets:[{targetKey, relation:"direct"|"room", roomId:String?, roomName:String?, deviceId,
    name, fingerprint, enabled, baseAuthorized, policyRevision:Long}]` (wie `exec_device_views`).
  - `share.setExec {targetKey, enabled}` → `{revision:Long}`; Schlüssel gegen den aktuellen Profilstand
    aufgelöst, sonst `not_found` („Identität geändert“); nicht vollständig
    (`persisted && applied && error == None` verfehlt) → Fehler mit Detail; `enabled:true` bei nicht
    verfügbarem Anbieter → `unsupported` mit Grund.
  - `share.execJobs {}` → `{active:[ExecJob], history:[ExecJob]}`, `ExecJob {direction, execId,
    peerDeviceId, peerName, program, state, startedAt?, finishedAt?, exitCode?, message?}`.
  - `share.cancelExecJob {direction, execId, peerDeviceId}` → `{}` (nicht mehr aktiv → `not_found`).
- Kotlin: `api/ShareApi.kt` (oder neu `api/ShareExecApi.kt`), neu `ui/share/ExecHostSection.kt`
  (Abschnitt „Befehle auf diesem Telefon“: je Ziel Erlauben… → Warnung mit Android-Risiken + Häkchen →
  Aktivieren; Entziehen; laufende/letzte Befehle mit Stopp), `ui/share/ShareScreen.kt`,
  `ShareViewModel.kt`, neu `service/ExecHostNotifier.kt` (laufende Benachrichtigung je eingehendem
  Befehl mit „Stopp“, gespeist aus dem Share-Poller), `system/Notifications.kt` (Kanal),
  `AndroidManifest.xml` (nur ein Empfänger für „Stopp“, falls nötig), JVM-Test der DTOs.
- Fertig, wenn: Host-/JVM-Tests grün; Gerätetest `ShareExecTaskTest` vollständig.

### G5a SMB-Backend (nativ, recherche.md E3)
- Dateien: neu `native/src/smb/` (`mod.rs`, `core/backend.rs`, `core/session.rs`, `core/listing.rs`,
  `core/replace.rs`, `core/io.rs`, `core/url.rs`, `core/errors.rs`, `core/tests.rs`),
  `native/src/lib.rs`, `vfs/core/scheme.rs`, `vfs/core/dispatch.rs`, `creds/core/types.rs`
  (`Protocol::Smb`), `connect/core/location.rs`, `connect/core/endpoint.rs`,
  `connect/os/shared/connector.rs`, `daemon/os/shared/ipc_protocol.rs` (`MountBackendScheme::Smb`, im
  Mount-Pfad abgewiesen), `app/core/error_report.rs`, `cli/connections.rs`, `cli/setup.rs`,
  `app/core/dialogs.rs` (nur erzwungene Match-Arme, Desktop bietet SMB nicht an), Tests mit
  erschöpfenden Matches (`connect/core/sync_paths_task_tests.rs`,
  `connect/os/shared/remote_drive_task_tests.rs`, `app/os/shared/sync_*_task_tests.rs`).
  `Cargo.toml`/`Cargo.lock`: erledigt (Hauptagent).
- Schnittstelle: siehe recherche.md E3 (Primitive, eigenes Listing mit Reparse-Attribut, Löschen mit
  `FILE_OPEN_REPARSE_POINT`, DFS aus, Fehlerzuordnung, je Freigabe ein Tree).
- Fertig, wenn: Host-Tests (URL/Pfad-Zerlegung, Listing-Parser mit Reparse-Attribut, Rename-Puffer,
  Fehlerzuordnung) grün; Gerätetests SMB (RemoteTaskTest, SyncTaskTest) grün.

### G5b SMB in Fassade und App
- Dateien: `native/src/mobile/core/location.rs` (`LocKind::Smb`), `mobile/core/crumbs.rs`,
  `mobile/os/shared/pool.rs`, `mobile/os/shared/domains/connections.rs`, weitere erschöpfende
  `LocKind`-Matches (`mobile/os/shared/fs_list.rs`, `import.rs`, `runtime.rs`, `mobile/core/tests.rs`);
  Kotlin `api/ConnApi.kt`, `api/FilesApi.kt`, `ui/connections/ConnectionDraft.kt`,
  `ConnectionForm.kt` (Felder Freigabe/Domäne nur im Formular, Einfügen von `\\host\freigabe` bzw.
  `smb://`, Hinweis zur Verschlüsselung), JVM-Test `ConnectionDraftTest.kt`.

### G6 „In Smart Explorer öffnen“
- Dateien: `AndroidManifest.xml` (Intent-Filter nur für Ordner-MIME-Typen; Dateien werden bewusst
  nicht beansprucht, sonst stünde die App bei jedem Datei-Öffnen zur Wahl), `MainActivity.kt`
  (Intent → Pfad → `AppNav`, sonst Hinweis), neu `system/OpenWithTarget.kt` (reine Abbildung),
  `system/OpenWithIntent.kt` (Intent/URI), JVM-Test `OpenWithTargetTest.kt`, Gerätetest
  `IntentsTaskTest.openAFolderHandedOverByAnotherApp`.
- Fertig, wenn: JVM-Test der Abbildung grün; Gerätetest: Paketmanager bietet die App für
  `vnd.android.document/directory` an, fremde/private URIs werden abgewiesen, `ACTION_VIEW` mit
  Ordner-URI landet im Ordner.

### T Suite
- `android/test-servers/servers.sh`: Samba-Container `dockurr/samba:4.23.10` auf Port 1445,
  Freigabe `seshare`, Nutzer `sesmb`, serverseitige Download-Datei; Argumente `seSmb*`.
- `RemoteTaskTest.smbUploadDownloadReplaceAndDelete`: `conn.save` mit `protocol:"smb"`,
  `root:"/<freigabe>"`; falsches Passwort → `auth`; falsche Freigabe → `not_found` mit Namen; Listing
  `backend:"smb"`; Upload, Download, zweiter Upload nummeriert, Ersetzen per `fs.uploadEdit`, Kopie,
  `fs.rename`, rekursives Löschen.
- `SyncTaskTest.syncUpdatesReplaceFilesOnPlainSftpFtpAndSmb` (Kritik 7): geänderte lokale Datei ersetzt
  die synchronisierte Kopie auf reinem SFTP, FTP und SMB; keine liegengebliebenen Stufen.
- `ShareExecTaskTest` (Phase A2) + `share-desktop.sh exec` (Marker in
  `/sdcard/SmartExplorerTask/exec-host`), `test-android-task.sh share_exec_host`: erlaubter Befehl,
  Abbruch eines Baums mit `setsid`-Kind und Doppel-Fork-Waise, Zeitlimit, Entzug während des Laufs,
  Abweisung danach; ein Hintergrund-Sync-Job mit schlafendem Vorher-Befehl läuft parallel und bleibt
  grün.
- `IntentsTaskTest.openAFolderHandedOverByAnotherApp` (G6); G1-Liste in `test-android-task.sh`.
- WebDAV-Ersetzen (Kritik 7): Host-Test gegen einen Mock-HTTP-Server, der im Promote-Pfad `MOVE` mit
  `Overwrite: T` prüft (Hauptagent).

## Agentenplan
- Agent A: G4a → G4b (Exec-Lesung, Kritik 1/3/8/12–15).
- Agent B: G5a → G5b (smb2-Ref, SMB-Lesung, Kritik 2/4/5/6/9/10).
- Hauptagent: G6, T, FTP-Vertrag (Kritik 17), Lock, `api.md`, README/TODO, Recherche Android-Grenzen
  (Kritik 12/18), Integration, Graph, Suite, Release.
- A und B parallel mit getrennten Dateien (B ändert nichts unter `share/`, A nichts unter `smb/`,
  `vfs/`, `connect/`, `mobile/core/`); keine Builds.

## Gesamtablauf
| Ablauf | Funktionen | Wie | Erfolg, wenn |
|---|---|---|---|
| G1 Host | Eindämmung, SMB-Hilfen, bisherige Tests | `android-task.yml` Job `host` | alle gelisteten Tests grün |
| G2 Desktop | Bestand | Job `host` (clippy/fmt/check Linux+Windows) | keine Befunde in geänderten Zeilen |
| G3 Build | APK, .so | Job `android-build` | baut, JVM-Tests grün |
| G4 Gerät | R, G1–G6 | Job `device` | alle Tests grün, Abdeckung vollständig |

## Status
| Block | Status | Notiz |
|---|---|---|
| R | fertig | `80fe206` |
| G1–G3 | fertig (Code) | `77d6e0e`; Gerätetests offen |
| G4a | offen | nach Kritik neu entschieden: Subreaper je Job |
| G4b | offen | |
| G5a | offen | |
| G5b | offen | |
| G6 | fertig (Code) | Gerätetest in der Suite offen |
| T | in Arbeit | SMB-Server/-Test, Exec-Host-Ablauf, G6-Test geschrieben; G1-Liste offen |
