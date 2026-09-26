# Android-Grenzen – Umsetzung

Spec: `spec.md` · Recherche: `recherche.md`. Regeln: AGENTS.md (keine lokalen Builds/Tests, Dateien
< 500 Zeilen, `core/` plattformfrei, Tests mit Präfix `android_task_`, eine Remote-Suite).

## Blöcke

### R Code-Review (fertig)
- 89 Befunde, 84 bestätigt, in sechs Dateigruppen behoben (`80fe206`), Graph `7e169b2`.

### G1–G3 Ersetzen/Hochladen SFTP, WebDAV, FTP (fertig)
- `77d6e0e`: `vfs::promote_staged_with`, `sftp/core/posix_rename.rs`, WebDAV `promote_staged`,
  `ftp/core/staging.rs`. Fertig, wenn: Gerätetests „plain SFTP ersetzt“, FTP Upload/Ersetzen grün.

### G4a Exec-Anbieter Android (nativ)
- Dateien (exklusiv): `native/src/share/os/unix/exec_supervisor.rs` (verschoben aus `linux_os/`,
  Android: `/system/bin/sh`, eigene Prozessgruppe), `native/src/share/os/linux_os/exec.rs` (nur
  `#[path]`), `native/src/share/os/android/exec.rs`, neu `native/src/share/os/android/exec_contain.rs`
  (Subreaper, `/proc`-Nachfahren, Kill, Prüfung), Tests `native/src/share/os/android/exec_contain_tests.rs`
  (Host-Linux wie `daemon::android_platform`), `native/src/share/mod.rs` (cfg/Pfade).
- Schnittstelle: `ContainedExec` wie Lesung §3; `provider_status` meldet `available` nur, wenn Subreaper
  gesetzt und `/system/bin/sh` vorhanden; höchstens ein aktiver Job (`prepare` sonst `WouldBlock`).
- Refs: Lesung exec-host §3/§4; `libc` prctl/kill/waitpid.
- Fertig, wenn: Host-Tests der `/proc`-Auswertung und Eindämmung grün; Desktop-Linux unverändert.

### G4b Exec-Host Fassade und Oberfläche
- Dateien: neu `native/src/mobile/os/shared/domains/share_exec.rs`, `domains/mod.rs` (Dispatch),
  `domains/share_status.rs` (exec-Felder je Gerät/Mitglied, Anbieter), Kotlin `api/ShareApi.kt`,
  `ui/share/*` (Freigabe-Schalter mit Scharfschalten + Warnung + Häkchen, Liste laufender Host-Befehle
  mit Stopp), `api.md` §5 (vom Hauptagent).
- Methoden: `share.setExec {target:{kind:"direct",contactId}|{kind:"room",profileId,deviceId}, enabled}`
  → `{}` (`daemon::mutate_exec_grant`); `share.execJobs {}` → `{incoming:[…], outgoing:[…]}`;
  `share.cancelExecJob {direction, execId, peerDeviceId}` → `{}`; `share.status` erhält
  `execProvider:{available, provider, detail}` und je Gerät/Mitglied `exec:{enabled, allowed}`.
- Fertig, wenn: JVM-/Host-Tests für DTOs grün; Gerätetest: Desktop-CLI führt nach Freigabe einen Befehl
  auf dem Telefon aus, ohne Freigabe wird abgewiesen.

### G5a SMB-Backend (nativ)
- Dateien: neu `native/src/smb/` (`mod.rs`, `core/backend.rs`, `core/connection.rs`, `core/replace.rs`,
  `core/io.rs`, `core/url.rs`, `core/tests.rs`), `native/Cargo.toml` + `Cargo.lock` (`smb2 = "=0.26.0"`),
  `native/src/lib.rs`, `vfs/core/scheme.rs`, `vfs/core/dispatch.rs`, `creds/core/types.rs`
  (`Protocol::Smb`), `connect/core/location.rs`, `connect/core/endpoint.rs`,
  `connect/os/shared/connector.rs` (`connect_smb`), `app/core/dialogs.rs` (Protokollwahl).
- Schnittstelle: `SmbBackend` mit `rename_overwrites = true`, `open_write_new` exklusiv, Ersetzen per
  Compound (Ref smb2.md), Pfade `/freigabe/...`.
- Fertig, wenn: Host-Tests (URL/Pfad-Zerlegung, Rename-Puffer) grün; Gerätetest SMB grün.

### G5b SMB in Fassade und App
- Dateien: `native/src/mobile/core/location.rs` (`LocKind::Smb`), `mobile/os/shared/pool.rs`,
  Kotlin `api/ConnApi.kt` (`PROTOCOLS`), `ui/connections/ConnectionDraft.kt`, `ConnectionForm.kt`
  (Port 445, Label, Hinweis „Startordner beginnt mit der Freigabe“), `api.md`.

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
- `android/test-servers/servers.sh` (Samba-Container `dockurr/samba`), Instrumentierungs-Argumente
  `seSmb*`, Gerätetests `RemoteTaskTest` (SMB: Liste, Upload, Download, Ersetzen, Sync-Aktualisierung),
  `ShareRoomTaskTest`/`share-desktop.sh` (Exec-Host: Freigabe, Befehl vom Desktop, Entzug), Intent-Test
  für G6, G1-Liste in `test-android-task.sh`.

## Agentenplan
- Agent A: G4a → G4b (gleiches Wissen: Exec-Lesung).
- Agent B: G5a → G5b (gleiches Wissen: smb2-Ref, SMB-Lesung).
- Hauptagent: G6, T, Integration, api.md, Doku, Suite, Release.
- A und B parallel (getrennte Dateien); keine Builds.

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
| G4a | offen | |
| G4b | offen | |
| G5a | offen | |
| G5b | offen | |
| G6 | fertig (Code) | Gerätetest in der Suite offen |
| T | offen | |
