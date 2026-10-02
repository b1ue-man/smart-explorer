# RV1 – Umsetzung

Stand: 2026-10-02. Spec: [spec.md](spec.md). Befunde: [Analyse](review-befunde-analyse.md),
[Sync](review-befunde-sync.md), [Sicherheit](review-befunde-sicherheit.md). Recherche: [recherche.md](recherche.md).

Regeln für alle Blöcke (zusätzlich zu AGENTS.md): Dateien gehören genau einem Block (Tabelle „Besitz“).
Wer eine Änderung in einer fremden Datei braucht, schreibt sie als Anfrage nach
`anfragen/<block>.md` (Datei, Stelle, gewünschte Änderung, Grund) und arbeitet mit einem lokalen
Platzhalter weiter; der Orchestrator leitet weiter. Neue oder wesentlich geänderte Rust-Dateien < 500
Zeilen (sonst erst ausgliedern). Keine lokalen Builds/Tests; statische Prüfung nur per
`sudo -n /root/.cargo/bin/rustfmt --edition 2021 --check < datei.rs` (Ausgabe leer = sauber). Jeder Block
schreibt seine Meilenstein-Tests (Präfix `review_task_`, Rust-Tests im eigenen Besitz oder neuen
Testdateien; Android-Gerätetests nur als Beschreibung in `abnahme/<block>.md`) und listet sie mit
erwartetem Ergebnis in `abnahme/<block>.md`. Kotlin↔Rust-Schnittstellen werden als Delta in
`api-delta/<block>.md` beschrieben (Methode, Argumente, Antwort, Fehler); Block AND-SHARE-UI bzw.
AND-SYNC überträgt sie nach `docs/superpowers/plans/2026-09-25-android-apk/api.md`.

## Verträge (verbindlich; Fundament-Blöcke K1–K3 setzen sie als Erstes um und tragen die exakten Signaturen hier nach)

### V1 VFS (Block K1)
- `VfsMeta.special: bool` – Eintrag ist weder Datei noch Ordner noch Link (FIFO, Socket, Gerät; Windows
  `FILE_ATTRIBUTE_DEVICE`, AF_UNIX-/LX-Reparse-Tags). Alle Konstruktionen werden ergänzt.
- Zwischendatei mit Zeitstempel: Backend-Methode, die eine Zwischendatei mit bekannter Größe und
  optionaler Änderungszeit öffnet (Backends, die die Zeit nur beim Hochladen setzen können – WebDAV
  `X-OC-Mtime`, Drive `modifiedTime` –, nutzen sie dort), plus eine Methode „Zeit einer fertigen
  Zwischendatei setzen“ (lokal `File::set_times`, SFTP setstat, FTP MFMT, Share neue Anfrage), die
  `Ok(false)` liefert, wenn das Backend keine Zeiten übernehmen kann. Dazu `mtime_precision(root)` →
  {Millis, Seconds, TwoSeconds, Minutes, Days, Unknown} für Vergleiche.
- Fehlerarten: `io::ErrorKind::StorageFull`/`ReadOnlyFilesystem`/`QuotaExceeded` werden von allen
  Backends für „voll“/„schreibgeschützt“ geliefert (Hilfsfunktion `vfs::is_target_refusal(&io::Error)`).
- Lokale Wurzel-Identität: `vfs::local_root_identity(path) -> Option<RootIdentity>` (Windows:
  Volume-Seriennummer + FileId der Wurzel + Dateisystemname; Linux/Android: st_dev + st_ino der Wurzel
  + statfs-Typ (+ FS-UUID, wenn billig)); Fernwurzeln: None.
- Optionale Backend-Haken (Standard „nicht unterstützt“), weitergereicht von Caching-/Agent-/
  Unavailable-Backend: `find_duplicates(root, opts, progress)`, `hash_walk(root, algo, sink)`,
  `recycle(path, expected)`; Fähigkeitsabfragen dazu.
- Backend-Trait-Datei bleibt < 500 Zeilen (Standard-Implementierungen in eigene Datei auslagern).

### V2 Share-Draht (Block K2)
- Neue Fähigkeiten in `server_capabilities::describe` (additiv): `duplicate_search_v1`, `hash_walk_v1`,
  `list_batches_v1`, `remote_trash_v1`, `stage_mtime_v1`, `analysis_deflate_v1`, `analysis_reattach_v1`,
  `export_access_v1` (Rechte je Wurzel in den Capabilities).
- Neue `FsRequest`-Varianten (je mit Fähigkeit; alte Hosts antworten „unsupported“):
  `DuplicateSearch{path,min_bytes,request_id}`, `HashWalk{path,algo,min_bytes}`,
  `ListDirBatch{path,cursor}`, `Recycle{path,expected_size,expected_sha256}`,
  `SetStageMtime{staged,mtime_ms}` (bzw. `mtime_ms` an `PromoteNoReplace`/`WriteNew`),
  `StorageAnalysis` erweitert um `request_id`, `node_budget`, `compress`.
- `FsErrorKind::StorageFull` (ältere Peers lesen `Unknown`).
- `AnalysisReport` additiv: `volume: Option<{used,total}>`, `platform: Option<PlatformFigures>`,
  `protected: Vec<ProtectedOmission>`.
- Freigabe-Rechte: `ShareExportConfig`/`SharedRoot` erhalten `access: ExportAccess {ReadOnly, ReadWrite}`
  (`serde(default)` = ReadWrite für bestehende Einträge; neue Einträge ReadOnly – siehe S-POLICY).
- Host-Einstiegsfunktionen (Signaturen trägt K2 hier nach), die H-DISPATCH in die Anfrage-Verteilung
  einhängt: `serve_duplicate_search`, `serve_hash_walk`, `serve_list_batch`, `serve_recycle`,
  `serve_set_stage_mtime`.

### V3 Sync-Engine und Jobs (Block K3)
- Schnappschuss-Ergebnis je Seite: Baum + Ordnermenge + Auslassungen mit Art
  {Link, Unlesbar, Verschwunden, NichtDarstellbar, Speziell, Gefiltert, Eigene Zwischendatei,
  Systemordner}; Auslassungen schützen Gegenstück und Basis (wie heute Links).
- Planungsschlüssel: je Paar Faltung (Groß/klein, wenn eine Seite nicht unterscheidet) + NFC; jede Seite
  behält ihre Schreibweise für I/O.
- Ergebnis einer Aktion: `CompletedAction{rel, kind, src_sig, dst_sig}` (vom Apply beobachtete
  Signaturen); Basis-Aktualisierung nur daraus + „konvergiert“-Liste (gleiche Signaturen aus der
  Planung) – kein Voll-Scan nach dem Lauf.
- Basis v2 (additiv): `root_identity` je Seite, `mtime_precision` je Seite, Ordnermenge.
- `SyncJob` additiv: `rt_max_latency_secs`, `rt_poll_secs` (Standard 300), `verify_interval_secs`
  (Standard 3600), Löschschutz-Standards (`max_delete_pct` 50, für bestehende Jobs mit 0 migriert),
  `versions_location` {Auto, AppData}. Laufzeitdaten (`last_attempt`, `last_success`,
  `consecutive_failures`, `last_error`, `blocked`, `pending_trigger`) liegen in einer eigenen
  Zustandsdatei je Job (Modul gehört T-JOBS); `last_run` bleibt lesbar (Migration).
- Paar-Sperre: `bisync::PairLock::acquire(pair_id)` geräteweit (Datei-Lock), von allen Läufern genutzt.

## Blöcke und Besitz

Reihenfolge nach Wichtigkeit; „nach“ = startet, wenn der genannte Block seine Verträge eingetragen hat.

| Block | Inhalt (Spec) | Besitz (exklusiv) | Abhängig |
|---|---|---|---|
| K1 → V-LOCAL | V1, dann FS6 + lokale Teile FS4/FA7 (Listen tolerant), Linux-NOREPLACE-Rückfall, mkdir_all-Wurzel, Windows-Namen, Rechte, Reparse-Klassen | `native/src/vfs/**`, `native/src/local_access/**`, `native/src/copy/**`, `native/src/android_fs/**`, `native/src/types/**`; VfsMeta-Konstruktorzeilen überall | – |
| K2 → H-ANALYSIS | V2, dann FA2 Host-Teil, FA4, FA5, FA7, FA6 Papierkorb-Host | `native/src/share/core/{wire,fs_response,fs_error,server_capabilities,storage_*,peer_storage_*,peer_walk,walk_assembly,peer_request,framing,peer_batch_*}.rs`, neue `share/core/{duplicate_*,hash_walk*,list_batch*,remote_trash*,analysis_*}.rs`, `native/src/share/os/shared/storage_analysis_host.rs`, `native/src/analytics/core/**` | – |
| K3 → E-PLAN | V3, dann FS1, FS2 (Planung), FS3, FS4 (Planung: Filter/Faltung/Grenzen), Y40, Y41, Y44, Y53, Y59, Y63, FS12-Anteile Y150 (Basis an Job gebunden, Aufräumen beim Löschen), Y153 (Filter ohne Groß/klein) | `native/src/bisync/core/**`, `native/src/bisync/os/shared/{orchestration,persistence,state_*,incremental*,resolve,preview,sync_flows,sync_overload,duplicate_observation,duplicate_plan}.rs`, `native/src/syncjobs/{core/types,core/validation,os/shared/editor,os/shared/persistence*,os/shared/migration}.rs` | – |
| E-APPLY | FS4 (Walk), FS5, FS2 (Apply: Zeit setzen), Schnellspiegel `sync/`, FS12-Anteile Y145 (App-Daten als Auslassung), Y152 (Einhängepunkte), Y154 (Ziel-FS-Grenze), Y155 (Inhalt prüfen), Y54, Y149 (Versions-Verzeichnis lesbar + Wiederherstellen-Funktion im Kern) | `native/src/bisync/os/shared/{snapshot*,apply*,move_finalize,duplicate_apply,duplicate_backup}.rs`, neue `bisync/os/shared/versions*.rs`, `native/src/sync/**` | nach K3, K1 |
| V-REMOTE | FS7, V1 für SFTP/FTP/WebDAV/SMB/Drive | `native/src/{sftp,ftp,webdav,smb,gdrive,connect}/**`, `native/src/agent/core/backend.rs` (nur Weiterreichung neuer Haken) | nach K1 |
| H-DISPATCH | FA3, FC1-Durchsetzung (Nur-lesen, App-Daten nie erreichbar), FC6, Haltesignale FA5 | `native/src/share/core/{server,server_fs,server_admission,server_transfer,server_batch_*,blocking,authorization_policy,configuration_runtime,node,node_accept,node_sessions,node_idle,handshake_limits,fs,fs_access,fs_paths,fs_copy,walk,session,mount_lease*,power*,keepalive}.rs` | nach K2 |
| A-CLIENT | FA1, FA2/FA6 Clients, Daemon-Hash-Walk-Korrekturen | `native/src/mobile/os/shared/domains/{analyze,analyze_platform}.rs`, `native/src/analytics/os/**`, `native/src/app/core/{analytics_*,reclaim_*}.rs`, `native/src/daemon/os/shared/{ipc_analysis,backend_walk,backend_budget}.rs`, `native/src/agent/core/search.rs`, Kotlin `ui/analytics/**`, `api/AnalyzeApi.kt`, `service/TaskForegroundService.kt`, `service/TaskKeeper.kt` | nach K2 |
| T-JOBS | FS8, FS9, FS10 (Daemon-Seite), FS12-Anteile Y146 (Befehle davor/danach), Y151 (Protokoll rotieren, Ergebnisse je Job ohne verlorene Schreibvorgänge, Problemzustand für Benachrichtigungen), Y145/Y152 in der Überwachung | `native/src/daemon/os/shared/{run_loop,schedule,state,catch_up,job,job_supervisor,host_state,live,boot_marker,handoff}.rs`, `native/src/daemon/os/{windows,linux_os,android}/platform.rs`, neue `native/src/watch/**` (OS-Adapter unter `watch/os/{windows,linux_os,android,shared}`), `native/src/syncjobs/os/shared/results.rs`, neue `syncjobs/os/shared/job_state*.rs`, `native/src/syncjobs/core/schedule.rs`, `native/src/autostart/**` | nach K3 |
| D-SYNCUI | Desktop-Bedienung FS3/FS9/FS10/FS12 (Job-Status, blockierte Läufe bestätigen, Überwachungsart, Versionen ansehen/wiederherstellen, Job löschen mit Aufräumen, Problem-Hinweis), Y147 (Zusammenführung), Y148 (Einzeldatei aus Vorschau), Y156 (Schließen während Lauf), Y19 | `native/src/app/core/{job_editor*,menus_sync_jobs,settings_background,bisync_ui,bisync_conflict*,bisync_merge,merge_ui,preview_core,sync_core}.rs`, `native/src/app/os/shared/sync_jobs.rs`, neue `app/core/sync_versions_ui*.rs` | nach T-JOBS, E-APPLY |
| AND-SYNC | FS11 inkl. Y144 (Allzugriff als Vorbedingung), Android-Teile FS8/FS9/FS12 (MediaStore-Beobachter, Jobstatus, Versionen, Benachrichtigung „Sync-Probleme“, Job löschen mit Aufräumen) | `native/src/mobile/os/shared/domains/{background,sync_run,sync_jobs,job_json,sync_conflicts,sync_merge}.rs`, `native/src/mobile/os/shared/sys.rs`, Kotlin `work/**`, `service/{BackgroundService,BackgroundController,BackgroundText}.kt`, `system/{BootReceiver,HostMonitor,KeepAlive*,WakeKeeper,Permissions,Notifications}.kt`, `ui/sync/**`, `ui/settings/BackgroundSettings.kt`, `api/SyncApi.kt` | nach T-JOBS |
| S-POLICY | FC1 (Konfiguration, Standards, Räume, Verbindungen, Migration, Desktop-UI, CLI) | `native/src/share/core/{profiles,profile_persistence,room_relation,types}.rs`, `native/src/share/os/shared/{profile_*}.rs`, `native/src/app/core/{share_exports_ui,share_helpers,share_rooms_ui,share_profile_*}.rs`, `native/src/cli/share/exports.rs`, `native/src/mobile/os/shared/domains/share_peers.rs`, `native/src/mobile/core/config.rs` | nach K2 |
| S-PAIR | FC2 | `native/src/share/core/discovery_*.rs`, `native/src/share/os/shared/discovery_*.rs`, `share-server/src/discovery*.rs`, `native/src/app/core/share_discovery_*.rs`, `native/src/cli/share/discoverable*.rs` | – |
| S-TRUST | FC3, FC4 (Client und Server) | `native/src/share/core/{signal_connection,signal_connector,signal_handshake,signal_session,signal_connected,signal_worker*,signal_schedule,signal_publish,signal_subscriptions,signal_readiness,signal_idle,signal_power,endpoint_routes,service}.rs`, `native/src/share/os/shared/transport_options.rs`, `share-server/**` außer `discovery*.rs`, `vendor/iroh-relay-1.0.0/**` (nur falls nötig), `native/src/app/core/menus_settings.rs`, `native/src/cli/share.rs`, `native/src/mobile/os/shared/domains/share_settings.rs`, `docs/SHARE_SERVER.md` | – |
| S-REVOKE | FC5 | `native/src/share/core/{direct_*,legacy_direct_*,removed_direct_peers,identity,identity_repair,crypto,signal_auth,signal_presence,signal_commands,tracked_signal_*}.rs`, `native/src/share/os/shared/{direct_*,legacy_direct_actions,removal,identity_store,lifecycle_view}.rs`, `native/src/daemon/os/shared/ipc_host_*.rs`, `native/src/app/core/{share_direct_ui,share_lifecycle_*,share_legacy_lifecycle_ui,share_removal_ui,share_removed_devices_ui,share_identity_rotation}.rs`, `native/src/cli/share/{requests*,grants*,request_selection,lifecycle_output}.rs`, `native/src/mobile/os/shared/domains/share_requests.rs` | – |
| S-LOCAL | FC7, IPC-Vorab-Plätze (S60) | `native/src/support_dirs.rs`, `native/src/creds/**`, `native/src/daemon/os/shared/{ipc,ipc_listener,ipc_protocol*,locks}.rs`, `native/src/daemon/os/{windows,linux_os}/ipc_storage.rs`, `native/src/share/os/{windows,linux_os}/identity_lock.rs`, `native/src/net/**`, `native/src/share/core/{lan_*}.rs`, `native/src/share/os/shared/lan_*.rs`, `native/src/app/core/share_lan*_ui.rs` | – |
| AND-SHARE-UI | Android-Bedienung FC1–FC5, FA1-Anzeige liegt bei A-CLIENT | Kotlin `ui/share/**`, `ui/settings/SettingsScreen.kt`, `api/ShareApi.kt`, `docs/superpowers/plans/2026-09-25-android-apk/api.md` | nach S-POLICY, S-PAIR, S-TRUST, S-REVOKE |

Nicht zugeordnete Dateien ändert nur, wer sie per Anfrage zugeteilt bekommt.

## Agentenplan

- Welle 1 (sofort, parallel): K1, K2, K3, S-PAIR, S-TRUST, S-REVOKE, S-LOCAL (7).
- Welle 2 (sobald Plätze frei und Verträge eingetragen): V-REMOTE (nach K1), E-APPLY (nach K3), H-DISPATCH
  und A-CLIENT (nach K2), T-JOBS (nach K3), S-POLICY (nach K2).
- Welle 3: AND-SYNC und D-SYNCUI (nach T-JOBS/E-APPLY), AND-SHARE-UI (nach den S-Blöcken), danach Suite-Block SUITE
  (eine Task-Suite `native/test-review-task.sh` + `.github/workflows/review-task.yml`, Modi `check`/`suite`).
- Fundament-Agenten (K1–K3) laufen nach ihren Verträgen als V-LOCAL/H-ANALYSIS/E-PLAN weiter.
- Nach jeder Welle: ein Remote-`check` (Übersetzen Host, Windows-Ziel, Android-Ziel, share-server,
  Kotlin), Fehler gehen per Nachricht an den zuständigen Agenten.

## Gesamtablauf (Abnahme, eine Suite)

| Ablauf | Funktionen | Wie | Erfolg, wenn |
|---|---|---|---|
| Meilenstein-Tests | alle | `cargo test review_task_` (Linux, Windows, Android-Host) | alle grün |
| Fern-Analyse E2E | FA1–FA5 | zwei lokale `se`-Profile über Share-Server (Loopback), Host-Baum mit 50k Einträgen: Analyse und Duplikatsuche vom Client; Zählung der ListDir-Anfragen am Host | Ergebnis = lokale Analyse des Hosts, keine ListDir-Flut, kein Datei-Download bei Duplikaten |
| Sync-Backup E2E | FS1–FS7 | Spiegel- und Zwei-Wege-Jobs lokal→lokal, lokal→SFTP/WebDAV/FTP (Container), Abbruch mitten im Lauf, Fehlerdatei, leeres Ziel, rotierendes Ziel, FIFO, langer Name | zweiter Lauf kopiert nichts; Abbruch verliert nichts; leeres Ziel stoppt; FIFO blockiert nicht |
| Echtzeit | FS8 | Daemon mit Echtzeit-Job: Umbenennen, Ordner verschieben, Datei unverändert groß ändern, Dauer-Schreiber, Neustart mit offener Änderung | jeder Fall startet einen Lauf innerhalb Entprellung+Höchstwartezeit |
| Sicherheit | FC1–FC7 | Profile ohne Standardfreigabe, Schreibversuch auf Nur-lesen, App-Daten-Zugriff, leere PIN, Klartext-Server ohne Opt-in, Server-Übernahme eines Lookups, entfernter Peer mit neuer Geräte-ID | alles abgelehnt; Opt-ins funktionieren |
| Android-Gerät | FA1, FS11, FC-UI | Emulator: Fern-Analyse gegen Desktop-Host, Dauerbetrieb-Lauf mit Bildschirm aus, Share-Einstellungen | wie Spec |

## Status

| Block | Agent | Status | Notiz |
|---|---|---|---|
| K1 → V-LOCAL | – | offen | |
| K2 → H-ANALYSIS | – | offen | |
| K3 → E-PLAN | – | offen | |
| E-APPLY | – | offen | |
| V-REMOTE | – | offen | |
| H-DISPATCH | – | offen | |
| A-CLIENT | – | offen | |
| T-JOBS | – | offen | |
| D-SYNCUI | – | offen | |
| AND-SYNC | – | offen | |
| S-POLICY | – | offen | |
| S-PAIR | – | offen | |
| S-TRUST | – | offen | |
| S-REVOKE | – | offen | |
| S-LOCAL | – | offen | |
| AND-SHARE-UI | – | offen | |
| SUITE | – | offen | |
