# RV1 – Umsetzung

Stand: 2026-10-02 (nach Kritiker-Runde, siehe [review.md](review.md)). Spec: [spec.md](spec.md). Befunde:
[Analyse](review-befunde-analyse.md), [Sync](review-befunde-sync.md), [Sicherheit](review-befunde-sicherheit.md).
Entscheidungen: [recherche.md](recherche.md). Syntax: `docs/refs/` (INDEX).

## Regeln für alle Blöcke

- Dateien gehören genau einem Block (Tabelle „Besitz“); Besitzwechsel nur wie dort vermerkt. Wer eine
  Änderung in einer fremden Datei braucht, schreibt sie nach `anfragen/<block>.md` (Datei, Stelle, Änderung,
  Grund) und arbeitet mit einem lokalen Platzhalter weiter; der Orchestrator leitet weiter.
  `native/Cargo.toml`, `native/Cargo.lock`, `share-server/Cargo.toml`, `share-server/Cargo.lock` ändert nur
  der Orchestrator (Anfrage).
- AGENTS.md gilt: Rust-Dateien < 500 Zeilen (neu oder wesentlich geändert; sonst erst ausgliedern), `core/`
  plattformneutral, OS-Verhalten hinter `os/`, kein `unwrap`/`expect` in Produktivpfaden, Links/Junctions
  bleiben geschützte Auslassungen, Endpunkt-/Verbindungsidentität bleibt erhalten, Rückweg (Versionen,
  Konfliktkopien) für jede Überschreibung/Löschung.
- Keine lokalen Builds/Tests/cargo/gradle, keine Server, keine Installationen, keine Commits/Pushes, keine
  weiteren Agenten. Statische Prüfung: `sudo -n /root/.cargo/bin/rustfmt --edition 2021 --check < datei.rs`
  (leere Ausgabe = sauber); jede geänderte Rust-Datei muss so sauber sein.
- Meilenstein-Tests: Präfix `review_task_`, als Rust-Tests in eigenen oder neuen Testdateien (Kotlin-Unit-
  Tests unter `android/app/src/test`, Gerätetests nur als Beschreibung). Liste mit erwartetem Ergebnis in
  `abnahme/<block>.md`. Tests, die eine Umgebung brauchen (Container, zweites Profil, Windows-Dateisystem),
  sind mit `#[ignore]` + Grund markiert und in `abnahme/<block>.md` als „Suite-Stufe“ beschrieben.
- Kotlin↔Rust-Schnittstellen: Delta nach `api-delta/<block>.md` (Methode, Argumente, Antwort, Fehler);
  AND-SHARE-UI überträgt alle Deltas nach `docs/superpowers/plans/2026-09-25-android-apk/api.md`.
- Eigene Status-Zeile in der Tabelle „Status“ unten pflegen (Stand, Verträge, offene Anfragen).
- Registrierungsdateien gehören niemandem allein: `mod.rs` aller Module (z. B. `share/mod.rs`,
  `bisync/mod.rs`, `analytics/mod.rs`, `vfs/mod.rs`, `daemon/mod.rs`), `native/src/lib.rs`,
  `native/src/mobile/os/shared/dispatch.rs`, `native/src/mobile/os/shared/domains/mod.rs`,
  `native/src/cli/mod.rs`, `native/src/app/core/state.rs`. Jeder Block darf dort ausschließlich eigene
  Einträge hinzufügen (Modul-Zeile, `pub use`, Dispatch-Arm, Feld mit `Default`), nichts Fremdes ändern;
  schlägt ein Edit fehl, weil die Datei inzwischen geändert wurde: neu lesen, eigenen Eintrag erneut setzen.
- Kotlin: neue Datenklassen in die eigene `api/*.kt`-Datei des Blocks, nicht nach `core/**`.

## Verträge

Fundament-Blöcke setzen ihren Vertrag als Erstes um (nur Typen, Signaturen, Standard-Implementierungen und
Stubs, die bestehendes Verhalten nicht ändern), tragen die exakten Signaturen hier unter ihrem Vertrag ein,
setzen ihre Status-Zeile auf „Vertrag fertig“ und beenden ihren Lauf mit dem Bericht „Vertrag fertig“. Der
Orchestrator gibt danach per Nachricht den Rest frei.

### V1 VFS (K1)
- `VfsMeta.special: bool` (FIFO, Socket, Gerät; Windows `FILE_ATTRIBUTE_DEVICE` und AF_UNIX-/LX-Tags). Alle
  Konstruktionen werden zuerst ergänzt; danach Status „V1-VfsMeta fertig“ setzen (erst dann ändern andere
  Blöcke die betroffenen Dateien).
- Zwischendatei mit Größe und Änderungszeit (Zeit beim Hochladen, wo nur so möglich) + „Zeit einer fertigen
  Zwischendatei setzen“ (`Ok(false)` = nicht möglich) + `mtime_precision(root)` → {Nanos, Millis, Seconds,
  TwoSeconds, Minutes, Days, Unknown}.
- `io::ErrorKind::StorageFull`/`ReadOnlyFilesystem`/`QuotaExceeded` für „voll“/„nur lesbar“; Helfer
  `vfs::is_target_refusal(&io::Error)`.
- Optionale Haken (Standard „nicht unterstützt“, weitergereicht von Caching-/Agent-/Unavailable-Backend):
  `find_duplicates`, `hash_walk`, `recycle(path, expected)`, `change_signal(root)` (Abo auf Änderungen der
  Gegenseite: Share `watch_v1`, Nextcloud-ETag, Drive-Feed); Fähigkeitsabfragen dazu.
- Lokal: `syncfs`-Helfer (Linux/Android), sicheres Öffnen fremder Dateien (O_NONBLOCK|O_NOFOLLOW + S_ISREG),
  Dateisystem-UUID/Volume-Seriennummer + Pfad relativ zum Einhängepunkt (`vfs::local_volume_identity`).
- Trait-Datei bleibt < 500 Zeilen (Standard-Implementierungen auslagern).

### V2 Share-Draht (K2)
- Fähigkeiten (additiv): `duplicate_search_v1`, `hash_walk_v1`, `list_batches_v1`, `remote_trash_v1`,
  `stage_mtime_v1`, `analysis_deflate_v1`, `analysis_reattach_v1`, `watch_v1`, `export_access_v1`.
- `FsRequest` neu: `DuplicateSearch{path,min_bytes,request_id}`, `HashWalk{path,algo,min_bytes}`,
  `ListDirBatch{path,cursor}`, `Recycle{path,expected_size,expected_sha256}`, `SetStageMtime{staged,mtime_ms}`,
  `WatchExport{path}`; `StorageAnalysis` additiv `request_id`, `node_budget`, `compress`. Die Einordnung
  lesend/schreibend ist ein erschöpfendes `match` ohne Platzhalter.
- `FsMeta.special` (additiv), `FsErrorKind::StorageFull`.
- `AnalysisReport` additiv: `volume`, `platform`, `protected`.
- Freigabe-Typen ziehen in die neue Datei `share/core/export_config.rs` (alte Pfade per `pub use`):
  `SharedRoot{…, access: ExportAccess, allow_system_writes: bool}`, `ExportAccess::default() = ReadOnly`,
  Altdaten über `serde(default = "legacy_read_write")`; `ShareExportConfig` erhält die Verbindungs-Freigabe
  als Liste (`shared_connections: Vec<SharedConnection{name, access}>`, Altfeld `include_connections` wird
  beim Laden migriert). Nach dem Vertrag gehören `fs.rs` H-DISPATCH und `export_config.rs` S-POLICY.
- Host-Einstiege für H-DISPATCH: `serve_duplicate_search`, `serve_hash_walk`, `serve_list_batch`,
  `serve_recycle`, `serve_set_stage_mtime`, `serve_watch` (Signaturen hier nachtragen).

### V3 Sync-Engine (K3)
- Schnappschuss je Seite: Baum + Ordnermenge + Auslassungen mit Art {Link, Unlesbar, Verschwunden,
  NichtDarstellbar, Speziell, Gefiltert, EigeneDatei (Zwischendatei, `.se-sync-replica`, `.se-versions`,
  App-Daten), Systemordner, Einhängung, ZuGroßFürZiel, NameAufZielUnmöglich}; Auslassungen schützen
  Gegenstück und Basis.
- Planungsschlüssel je Paar: Faltung (wenn eine Seite nicht unterscheidet) + NFC (`icu_normalizer`); jede
  Seite behält ihre Schreibweise für I/O.
- Apply meldet erledigte Aktionen laufend: `CompletedAction{rel, kind, src_sig, dst_sig, durable}`;
  Orchestrierung schreibt Basis-Zwischenstände (alle N Aktionen / T Sekunden, nur `durable`) und am Ende.
- Replika: Markierung `.se-sync-replica` (JSON `{replica_id, created_ms, pair_hint}`) je Wurzel; Basis je
  (Paar-ID, Replika-ID A, Replika-ID B); ohne Markierung: Volume-Identität aus V1; „unbekannt“ ≠ „fremd“.
- Löschschutz: `max_delete_pct` 50 + `max_delete_min` 25, Migration per `config_version` in der Job-Datei.
- `SyncJob` additiv: `rt_max_latency_secs`, `rt_poll_secs` (300), `verify_interval_secs` (3600),
  `verify_target_secs` (86400), `cross_mounts` (neue Jobs aus), `versions_location` {Auto, AppData},
  `config_version`. Laufzeitdaten in einer Zustandsdatei je Job (Modul von T-JOBS, V4).
- Versionen: Modul `bisync/os/shared/versions*.rs` (Stub von K3, Besitz danach E-APPLY): Ordner
  `.se-versions/<lauf>/…` + Verzeichnis, Aufbewahrung je Datei, Bereinigung nach jedem Lauf.
- Paar-Sperre `bisync::PairLock::acquire(pair_id)` (Datei-Lock, geräteweit), genutzt von Daemon, Desktop,
  Android und Konfliktlösung.

### V4 Jobs, Überwachung, Wachhalten (T-JOBS)
- `native/src/watch/` (core + `os/{windows,linux_os,android,shared}`): Überwachung je Wurzel mit Filter-
  Callback; Ereignisse (Pfad, Art) oder `Overflow`/`Unavailable(Grund)`; eine inotify-Instanz für alle;
  Windows-Handle-Freigabe beim Entfernen. Wird auch von H-ANALYSIS für `watch_v1` genutzt.
- `native/src/keep_awake/` (os-Adapter): `keep_awake::hold(Reason) -> KeepAwake` (RAII, gezählt; Windows
  Power Request System+Execution und Stromdrosselung aus; Linux logind-Sperre über `zbus`; Android über den
  vorhandenen Wakelock-Haken). Genutzt von T-JOBS (Läufe) und H-DISPATCH (fremde Ströme).
- Job-Zustandsdatei (`last_attempt`, `last_success`, `consecutive_failures`, `last_error`, `blocked`,
  `pending_trigger`) mit Sperre gegen verlorene Schreibvorgänge; Problemzustand für Benachrichtigungen.

### V5 Beziehungen und Rechte (S-REVOKE)
- `types.rs`: Schreibrecht je Direkt-Freigabe (`DirectGrant.write`, neue Grants false, Altdaten true) und je
  Raum; Zustand „neu bestätigen“ (statt „ignoriert“) für Code-Rotation/Identitätsreparatur; Sperr- und
  Entfernungseinträge mit Schlüssel + Knoten; Raum-Merkmal „neue Mitglieder bestätigen“; Wahl
  „Gegenseitig“ beim Code-Hinzufügen/Koppeln (Standard aus).
- Ergebnis der Autorisierung einer Sitzung trägt `may_write`; H-DISPATCH setzt `may_write &&
  root.access == ReadWrite` durch.
- Invalidierung: Ereignis „Recht eingeschränkt für (Schlüssel, Beziehung)“; Präsenz/Laufzeitfelder lösen
  kein `ConfigureProfiles` aus (FA3-Pflichtteil im Daemon-Ereignisweg).

## Blöcke und Besitz

| Block | Inhalt | Besitz (exklusiv) | Start | Fertig, wenn |
|---|---|---|---|---|
| K1 → V-LOCAL | V1; FS6, lokale Teile FS4/FS5/FA7 (tolerante Listen, Dauerhaftigkeit, NOREPLACE-Leiter, mkdir_all-Wurzel, Windows-Namen inkl. reservierte, Nur-lesen-Ersetzen, Rechte, Reparse-Klassen, sicheres Öffnen) | `native/src/{vfs,local_access,copy,android_fs,types}/**`; VfsMeta-Zeilen überall (nur bis „V1-VfsMeta fertig“) | Welle 1 | V1 eingetragen; Y86/Y87/Y94/Y95/Y98/Y100/Y103/Y122/Y99/Y81, A24-lokal, B24/B25 umgesetzt; Tests je Plattform |
| K2 → H-ANALYSIS | V2; FA2/FA4/FA5/FA6/FA7 Host- und Peer-Client-Seite, `watch_v1`-Host | `native/src/share/core/{wire,fs_response,fs_error,server_capabilities,storage_*,peer_*,walk_assembly,framing,backend,export_config}.rs` (export_config nur bis Vertrag), neue `share/core/{duplicate_*,hash_walk*,list_batch*,remote_trash*,analysis_*,watch_*}.rs`, `native/src/share/os/shared/storage_analysis_host.rs`, `native/src/analytics/core/**`, `native/src/analytics/os/shared/{analytics,analytics_budget,analytics_outcome}.rs`, `native/src/analytics/os/shared/reclaim/{finder*,verify,duplicates,local,stage,types,util,retention,budget,cleanup,mod}.rs` | Welle 1 (fremde VfsMeta-Dateien erst nach „V1-VfsMeta fertig“) | V2 eingetragen; Host-Duplikate/Hash-Walk/Listen-Portionen/Papierkorb/Watch/Kompression/Wiederanbindung/Berichtsfelder; FA4-Punkte; Tests |
| K3 → E-PLAN | V3; FS1/FS2-Planung, FS3, FS4-Planung, Y40/Y41/Y44/Y53/Y59/Y63/Y150/Y153, Zwischenstände (B18), Replika/Basis-Schlüssel (B01), Löschschutz-Migration (B09) | `native/src/bisync/core/**`, `native/src/bisync/os/shared/{orchestration,persistence,state_*,incremental*,resolve,preview,sync_flows,sync_overload,duplicate_observation,duplicate_plan}.rs`, neue `bisync/os/shared/{replica*,checkpoint*,pair_lock*}.rs`, `native/src/syncjobs/{core/types,core/validation,os/shared/editor,os/shared/persistence*,os/shared/migration}.rs`; Stub `bisync/os/shared/versions.rs` (danach E-APPLY) | Welle 1 | V3 eingetragen; Basis nie „alles oder nichts“, Zwischenstände, Replika-Logik inkl. Rotation, Löschschutz, Faltung/NFC, Filter je Paar, Speichergrenzen; Tests |
| A-CLIENT | FA1 sofort (scan_remote, resolve_live, Phasen, Hinweise, Ergebnisse freigeben, Wakelock), danach FA2/FA6-Clients, Daemon-Hash-Walk-Korrekturen, A07/B26, Agent-/IPC-Weiterreichung neuer Haken | `native/src/mobile/os/shared/domains/{analyze,analyze_platform}.rs`, `native/src/analytics/os/shared/{remote,analytics_backend}.rs`, `native/src/analytics/os/shared/reclaim/{backend,backend_duplicates}.rs`, `native/src/app/core/{analytics_*,reclaim_*}.rs`, `native/src/daemon/os/shared/{ipc,ipc_protocol,ipc_protocol_bounds,ipc_client,ipc_analysis,backend_server,backend_walk,backend_budget,backend_batch,backend_stream,backend_transfer,backend_tree_send,request_workers}.rs`, `native/src/agent/**`, `native/src/agent_proto/**`, Kotlin `ui/analytics/**`, `api/AnalyzeApi.kt`, `service/{TaskForegroundService,TaskKeeper}.kt` | Welle 1 (FA1 zuerst; Rest nach V2) | Android analysiert Share-Orte auf dem Host; Duplikate host-seitig mit sicherem Rückfall; Tests über `mobile::call` |
| T-JOBS | V4; FS8, FS9, FS10 (Daemon), Y146, Y151 (Rotation, Ergebnisse, Benachrichtigung Desktop), B08/B10/B11/B19/B22/B27/B32 | `native/src/daemon/os/shared/{run_loop,schedule,state,catch_up,job,job_supervisor,host_state,live,boot_marker,handoff,embedded}.rs`, `native/src/daemon/os/{windows,linux_os,android}/platform.rs`, `native/src/daemon/mod.rs`, neue `native/src/{watch,keep_awake,notify_desktop}/**`, `native/src/syncjobs/os/shared/results.rs`, neue `syncjobs/os/shared/job_state*.rs`, `native/src/syncjobs/core/schedule.rs`, `native/src/autostart/**`, `native/src/lib.rs`/`main.rs` (nur Modul-Einträge und Wächter-Einstieg) | Welle 1 (V4 zuerst) | Echtzeit per Ereignissen + Abfrage + Kontroll-Läufe; Wächter; Benachrichtigung; Anschluss-Erkennung Linux/Windows; Tests |
| S-SIGNAL | FC2 + FC3 + FC4 (Client und Server), B20, B21 | `native/src/share/core/{discovery_*,signal_connection,signal_connector,signal_handshake,signal_session,signal_connected,signal_worker*,signal_schedule,signal_publish,signal_subscriptions,signal_readiness,signal_idle,signal_power,endpoint_routes,service}.rs`, `native/src/share/os/shared/{discovery_*,transport_options}.rs`, `share-server/**`, `vendor/iroh-relay-1.0.0/**` (nur falls nötig), `native/src/app/core/{share_discovery_*,menus_settings}.rs`, `native/src/cli/share.rs`, `native/src/cli/share/discoverable*.rs`, `native/src/mobile/os/shared/domains/share_settings.rs`, `docs/SHARE_SERVER.md` | Welle 1 | TLS-Standard Client/Server, Opt-in, Migration, Anmeldung mit Schlüssel, Bindungen, DoS-Grenzen, PIN-Regeln; Tests inkl. gemischter Versionen |
| S-REVOKE | V5; FC5, FA3-Pflichtteil im Daemon-Ereignisweg, Exec-Invarianten (B04), Raum-Bestätigung (B15), signierte Präsenzen/Entscheidungen (B03) | `native/src/share/core/{direct_*,legacy_direct_*,removed_direct_peers,identity,identity_repair,crypto,signal_auth,signal_presence,signal_commands,tracked_signal_*,types,exec*,room_relation}.rs`, `native/src/share/os/shared/{direct_*,legacy_direct_actions,removal,identity_store,lifecycle_view}.rs`, `native/src/share/os/*/exec*.rs`, `native/src/daemon/os/shared/{ipc_host,ipc_host_*,exec_*}.rs`, `native/src/app/core/{share_direct_ui,share_lifecycle_*,share_legacy_lifecycle_ui,share_removal_ui,share_removed_devices_ui,share_identity_rotation,share_exec*}.rs`, `native/src/cli/share/{requests*,grants*,request_selection,lifecycle_output,exec_status}.rs`, `native/src/cli/exec.rs`, `native/src/mobile/os/shared/domains/{share_requests,share_exec}.rs` | Welle 1 (V5 zuerst) | Entziehen per Schlüssel, Bestätigung neuer Geräte, Rotation ohne Aussperren, signierte Präsenzen, Exec-Invarianten; Tests |
| V-REMOTE | FS7, V1 für SFTP/FTP/WebDAV/SMB/Drive inkl. `change_signal` (Nextcloud-ETag, Drive-Feed) | `native/src/{sftp,ftp,webdav,smb,gdrive,connect}/**` | Welle 2 (nach V1) | Y114…Y143 (Fernziel-Teil) umgesetzt; Tests (Container-Stufen als Suite-Stufe) |
| E-APPLY | FS4-Walk, FS5, FS2-Apply, Schnellspiegel, Y145/Y152/B17/Y154/Y155/Y54/Y149-Kern, Versionsordner (B31), Dauerhaftigkeit (B18) | `native/src/bisync/os/shared/{snapshot*,apply*,move_finalize,duplicate_apply,duplicate_backup,versions*}.rs`, `native/src/sync/**` | Welle 2 (nach V3, V1) | Walks tolerant, Versionen je Lauf/Datei auf dem Ziel, Zeiten übertragen, dauerhaft, Ordner; Tests |
| H-DISPATCH | FA3 (Eingrenzen), FC1-Durchsetzung (Nur-lesen, `may_write`, Systemorte, App-Daten, `.se-versions` ausblenden), FC6, Wachhalten fremder Ströme | `native/src/share/core/{server,server_fs,server_admission,server_transfer,server_batch_get,server_batch_put,blocking,authorization_policy,configuration_runtime,node,node_accept,node_sessions,node_idle,node_wake,handshake_limits,fs,fs_access,fs_paths,fs_copy,walk,session,mount_lease*,power*,keepalive,io_deadline}.rs`, `native/src/daemon/os/shared/rooted_backend*.rs` | Welle 2 (nach V2, V5) | jede schreibende Anfrage geprüft, eingegrenzte Invalidierung, faire Grenzen, iteratives Löschen; Tests |
| S-POLICY | FC1-Konfiguration: Standards, Auto-Home-Migration, Räume ohne Freigaben, Verbindungs-Freigabe einzeln, Schreibrecht je Kontakt (UI/CLI), Desktop-Freigaben-UI, CLI | `native/src/share/core/{profiles,profile_persistence,export_config}.rs` (export_config nach V2), `native/src/share/os/shared/profile_*.rs`, `native/src/app/core/{share_exports_ui,share_helpers,share_rooms_ui,share_profile_*,share}.rs`, `native/src/cli/share/exports.rs`, `native/src/mobile/os/shared/domains/share_peers.rs`, `native/src/mobile/core/config.rs` | Welle 2 (nach V2, V5) | neue Profile/Räume ohne Freigaben, Migration, Rechte sichtbar; Tests |
| S-LOCAL | FC7 inkl. B14, IPC-Vorab-Plätze (S60) | `native/src/support_dirs.rs`, `native/src/creds/**`, `native/src/daemon/os/shared/{ipc_listener,locks}.rs`, `native/src/daemon/os/{windows,linux_os}/ipc_storage.rs`, `native/src/share/os/{windows,linux_os}/identity_lock.rs`, `native/src/net/**`, `native/src/share/core/lan_*.rs`, `native/src/share/os/shared/lan_*.rs`, `native/src/app/core/share_lan*_ui.rs`, `native/installer.nsi` | Welle 2 | Rechte ab Erstellung, Windows-ACL-Prüfung, Uplink-Reparatur, LAN-Privatsphäre; Tests |
| D-SYNCUI | Desktop-Bedienung FS3/FS9/FS10/FS12, Y147/Y148/Y156/Y19, Versionen ansehen/wiederherstellen | `native/src/app/core/{job_editor*,menus_sync_jobs,settings_background,bisync_ui,bisync_conflict*,bisync_merge,merge_ui,preview_core,sync_core}.rs`, `native/src/app/os/shared/sync_jobs.rs`, neue `app/core/sync_versions_ui*.rs` | Welle 3 | Desktop zeigt/bedient alles Neue; Tests der Logik |
| AND-SYNC | FS11, Y144, Android-Teile FS8/FS9/FS12 | `native/src/mobile/os/shared/domains/{background,sync_run,sync_jobs,job_json,sync_conflicts,sync_merge}.rs`, `native/src/mobile/os/shared/sys.rs`, Kotlin `work/**`, `service/{BackgroundService,BackgroundController,BackgroundText}.kt`, `system/{BootReceiver,HostMonitor,KeepAlive*,WakeKeeper,Permissions,Notifications}.kt`, `ui/sync/**`, `ui/settings/BackgroundSettings.kt`, `api/SyncApi.kt`, `android/app/src/main/AndroidManifest.xml` | Welle 3 | Wakelock, Alarme, Content-Trigger, Allzugriff-Vorbedingung, Jobstatus/Versionen; Tests |
| AND-SHARE-UI | Android-Bedienung FC1–FC5 | Kotlin `ui/share/**`, `ui/settings/SettingsScreen.kt`, `api/ShareApi.kt`, `docs/superpowers/plans/2026-09-25-android-apk/api.md` | Welle 3 | alle Deltas in api.md, UI vollständig; Tests |
| SUITE | eine Task-Suite | `native/test-review-task.sh`, `.github/workflows/review-task.yml`, neue Suite-Hilfsdateien | nach Welle 3 | jede Abnahme-Zeile unten automatisiert |

Nicht aufgeführte Dateien ändert nur, wer sie per Anfrage zugeteilt bekommt.

## Agentenplan

- Welle 1 (7 gleichzeitig): K1, K2, K3, A-CLIENT, T-JOBS, S-SIGNAL, S-REVOKE.
- Welle 2: V-REMOTE, E-APPLY, H-DISPATCH, S-POLICY, S-LOCAL – sobald Plätze frei und Verträge eingetragen.
- Welle 3: D-SYNCUI, AND-SYNC, AND-SHARE-UI; danach SUITE.
- Nach jeder Welle ein Remote-`check` (`review-task.yml`, Modus `check`); währenddessen werden keine neuen
  Agenten gestartet (AGENTS.md). Fehler gehen per Nachricht an den zuständigen Agenten.
- Commits je fertigem Block (Orchestrator), graphify-Auffrischung nach nativen Änderungen.

## Gesamtablauf (Abnahme in der einen Suite)

| Ablauf | Spec | Wie | Erfolg, wenn |
|---|---|---|---|
| Meilensteine | alle | `cargo test review_task_` auf Linux, Windows (windows-2025-Job) und Android-Host | alle grün, Quell- und Laufliste stimmen überein |
| Fern-Analyse | FA1–FA7 | Rust-Test über `mobile::call("analyze.start"/"reclaim.start")` gegen einen Share-Peer im selben Prozess, Host-Baum mit vielen Ordnern; Zähler am Host | 0 ListDir/Read für Analyse und Duplikatsuche, Ergebnis = lokale Analyse des Hosts, Abbruch sofort |
| Präsenzwechsel | FA3 | laufende Analyse/Übertragung, dann Präsenz eines dritten Kontakts und neues Raum-Mitglied | Strom bleibt offen; Sperre eines Schlüssels schließt alle seine Sitzungen inkl. Exec |
| Sync-Backup | FS1–FS7 | Spiegel- und Zwei-Wege-Jobs lokal→lokal, →SFTP/WebDAV/FTP (Container), Abbruch, Fehlerdatei, leeres Ziel, Rotation zweier Loop-Abbilder (FAT/exFAT), FIFO, langer Name, Windows-Namen | zweiter Lauf kopiert nichts; Abbruch verliert nichts; leeres/fremdes Ziel stoppt; Rotation spiegelt je Laufwerk; FIFO blockiert nicht |
| Echtzeit | FS8 | Daemon-Echtzeit-Job: Umbenennen, Verschieben, gleich große Änderung, Dauer-Schreiber, Überlauf (viele Dateien), Watch-Limit (gesenktes Limit), Neustart mit offener Änderung, Share-Gegenseite (`watch_v1`) | jeder Fall startet einen Lauf innerhalb Entprellung+Höchstwartezeit; Limit → sichtbare Abfrage |
| Sicherheit | FC1–FC7 | Profile ohne Standardfreigabe, Schreiben/Recycle/SetStageMtime auf Nur-lesen, Systemorte, App-Daten, leere PIN, Klartext-Server ohne Opt-in, Lookup-Übernahme am Server, gefälschte Präsenz eines zweiten Kontakts, entfernter Peer mit neuer Geräte-ID, Exec nach „Wieder erlauben“, keine Gegenseitigkeit ohne Wahl | alles abgelehnt; Opt-ins wirken |
| Gemischte Versionen | FC3/FC4/FA2 | `native/test-share-mixed-version-e2e.sh` (veröffentlichte 0.5.126- bzw. 0.5.169-CLI gegen neuen Server/Client) | Grundfunktionen gehen, neue Rechte nur mit neuer Seite |
| Android-Gerät | FA1, FS11, FC-UI | Emulator: Fern-Analyse gegen Desktop-Host, Dauerbetrieb-Lauf mit Bildschirm aus, Allzugriff entzogen, Share-Einstellungen | wie Spec |

## Status

| Block | Status | Notiz |
|---|---|---|
| K1 → V-LOCAL | offen | |
| K2 → H-ANALYSIS | offen | |
| K3 → E-PLAN | offen | |
| A-CLIENT | offen | |
| T-JOBS | offen | |
| S-SIGNAL | offen | |
| S-REVOKE | offen | |
| V-REMOTE | offen | |
| E-APPLY | offen | |
| H-DISPATCH | offen | |
| S-POLICY | offen | |
| S-LOCAL | offen | |
| D-SYNCUI | offen | |
| AND-SYNC | offen | |
| AND-SHARE-UI | offen | |
| SUITE | offen | |
