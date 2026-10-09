# Smart Explorer – Architektur

Stand: 2026-10-09. Kurzüberblick als erster Einstieg; Details liefert der Code-Graph
(`graphify query "…"`, siehe AGENTS.md) und die Lesungen unter `docs/lesungen/`.

## Zweck
Schneller Datei-Explorer mit tiefem Filtern, Remote-Backends, Sync, Peer-to-Peer-Share und
Speicheranalyse – als Desktop-App (Windows, Linux; Rust + egui) und als Android-App
(Kotlin/Compose über demselben Rust-Kern).

## Einstiege
| Aufgabe / Frage | Anfangen bei |
|---|---|
| Desktop-Start, CLI-Flags | `native/src/main.rs`, `native/src/lib.rs::run_gui`, `native/src/bin/se.rs` (CLI) |
| Desktop-Oberfläche | `native/src/app/` (egui, `App` in `app/core/state.rs`, Frame-Schleife `app/core/frame_update.rs`) |
| Android-App | `android/app/src/main/java/app/smartexplorer/android/` (`SmartExplorerApp`, `MainActivity`, `ui/AppRoot.kt`) |
| Kotlin↔Rust-Vertrag | `docs/superpowers/plans/2026-09-25-android-apk/api.md`; Kotlin `core/Core.kt` + `core/NativeBridge.kt`; Rust `native/android-bridge/src/lib.rs` → `native/src/mobile/` |
| Dateisystem-/Remote-Zugriff | `native/src/vfs/` (`Backend`-Trait), Backends `sftp/`, `ftp/`, `webdav/`, `gdrive/`, `zipfs/`, Share über `daemon::open_share_backend` |
| Frische FTP-Dateisignaturen | `ftp/core/metadata.rs` und `metadata_probe.rs` verwenden bei LIST-Fallback und Einzelstat denselben nichtenumerierenden SIZE-/MDTM-Probe. Nicht unterstützte/550-Probes beweisen keine Abwesenheit; vollständiger Eltern-LIST und geschützte Kinder bleiben maßgeblich. |
| FTP-/FTPS-Uploadabschluss | `ftp/core/data_finish.rs::UploadData` verbindet gespoolten Writer und bekannte-Längen-STOR. Nur für TLS bleibt ein zusätzlicher Socketbesitz bis nach TLS-Drop und tatsächlicher 226/250-Antwort erhalten; plain FTP behält seinen EOF. Flush-/Besitzfehler verhindern Erfolg und gesunden Poolrücklauf, terminale Zielablehnungen bleiben vorrangig. Kein impliziter STOR-Retry. |
| Lokaler Handle-/Privatzugriff | `native/src/local_access/` (autorisierte Root und gepinnte Childgrenzen); Windows `os/windows/directory_handle.rs` und `private_access.rs::open_private_child` für private Inhalte unter gehaltenen Eltern; `directory_rename.rs` für native NT-NoReplace-Hops am gehaltenen Zielroot |
| Bestätigung veröffentlichter Verzeichniseinträge | `vfs::confirm_namespace` → `BackendExtensions::confirm_namespace` → ausgewählter Local-/Agent-OS-Adapter; Linux/Android `vfs/os/linux_os/namespace_flush.rs` hält und prüft den tatsächlichen Parent-FD und Mount. Cache, Share-Guard und IPC geben die tatsächliche Bestätigung weiter. Dateiinhalte und Whole-filesystem-Flush bleiben getrennte Verträge. |
| Eigene native Sync-Backup-/Recoverypfade | `bisync/os/shared/apply_stage.rs::native_namespace` bestimmt den nativen Parent mit `Path::parent` und delegiert dessen Bestätigung direkt an VFS. Native Windows-Pfade gehen hier nicht durch den Slash-only-Parser für Backend-Literalpfade. |
| Orte/Endpunkt-Strings | `native/src/connect/core/location.rs` (`EndpointSpec`), `connect/os/shared/resolution.rs` (`resolve_endpoint`) |
| Filter und Scan | `native/src/filter/`, `native/src/scanner/`, `native/src/rscan/`, Baumzeilen `filter/core/tree.rs` |
| Kopieren/Übertragen | Engine `native/src/transfer/os/shared/engine/` (`run_job` für jede Endpunkt-Kombination: Walker `walk*.rs`, Worker, Ordner-Register, Pakete, Serverkopie, Fehlerprotokoll), Job-Vertrag `transfer/core/job.rs`, Flows `transfer/os/shared/flow.rs` + Regler `transfer/core/flow_control.rs`, Lane `lane.rs`; lokale Kernel-Kopie `native/src/copy/`; Verhalten und Grenzen je Protokoll `docs/TRANSFER_ENGINE.md` |
| Explorer-Übergabe Remote (Windows) | virtuelle Dateien `native/src/virtual_clipboard/os/remote/` (STA-Thread, Liste bei erster Explorer-Anfrage, Vorausladen), Ziehen `native/src/dragout/os/remote.rs`, Auswahlquelle `transfer/os/shared/selection.rs` |
| Medien öffnen mit Weiterschalten | Desktop: `app/os/shared/remote_open.rs::open_file` → `open_local_path`; Windows wählt in `app/core/media_launch_plan.rs` nach der Standard-App (`ms-photos:viewer` für die aktuelle Fotos-App, `Launcher.LaunchFileWithOptionsAsync` mit Nachbarabfrage für ältere Fotos/Store-Apps, sonst `ShellExecute`), Ausführung `app/os/windows/media_launch.rs`; Linux übergibt per `xdg-open` den echten Pfad. Android: Tippen auf Bild/Video/Audio → `ui/viewer/MediaViewer.kt` (Pager über `BrowserTab.shownEntries()`, lokal `fs.open`, sonst `fs.fetch` je Seite). Medienendungen: `types/core/media_kind.rs`. Plan: `docs/plaene/2026-10-09-medien-weiterschalten/plan.md` |
| App-Übertragungen | Einfügen/Ablegen/Dialoge → Job `native/src/app/core/transfer_route.rs`, Fenster „⇅ Übertragungen“ `app/core/transfer_window.rs`/`transfer_center.rs`, Zwischenablage `app/core/transfer_clip.rs` |
| Sync | Jobs `native/src/syncjobs/`, Zwei-Wege `native/src/bisync/`, Einweg-Spiegeln `native/src/sync/` |
| Drive-Syncnamen und dauerhafte Ordneridentität | `gdrive/core/{listing,listing_query}.rs` sammeln vollständige Namenskandidaten; `identity.rs`, `sync_projection.rs`, `sync_projection_names.rs` und `sync_bindings.rs` trennen Literalnamen, logische Schlüssel und IDs. Segmentbezogene Herkunft unterscheidet frisch bewiesene Root-/Pickerauswahl und alte globale Cachehints von vollständiger Parentprojektion; Lesen, Sync-Stat und normaler Writer verwenden dieselbe Grenze. `gdrive/os/shared/binding_store.rs` besitzt private, dateigesperrte atomare Account-/Parent-Records. |
| DAV-Collectionmetadaten und Ordneranlage | `webdav/core/connection.rs` konstruiert die getrennten GET-, Metadata-, Mutation- und Write-Agenten mit demselben verifizierten Transport. `metadata_request.rs` erhält bei einer kanonischen Collection-Slashantwort Methode, Body und Auth ausschließlich an derselben Origin. `transfer_ops.rs` sendet MKCOL direkt an die kodierte Collection-URL; Dateikollisionen brauchen einen frischen Beleg des ursprünglichen Namens. Gespeicherte Rootidentität, GET-Redirects und mutierende No-follow-Verträge bleiben erhalten. |
| Sync-Zustand, Wiederanlauf und Versionen | `bisync/core/run_types.rs` (`StateKey`, `RunSettings`), `bisync/os/shared/{apply_reporting,replacement_journal,versions}.rs`; inkrementelle Planung in `incremental.rs`, frische berührte Checksum-Ziele in `incremental_changes.rs`, vollständige Indexpersistenz und bestätigte Delta-Aktualisierung in `incremental_index_commit.rs`; unsichere/teilweise Generationen behalten den Dirtymarker; beide Seiten und Jobowner teilen dieselbe Engine-Grenze |
| Private Versionsarchive von Shares | `vfs::VersionArchivePolicy` hat den konservativen Default `Provider`. Peer und IPC-Identitätsstub melden `AppPrivate`; Agent und Cache delegieren rein. `bisync/os/shared/version_provider_policy.rs` verlangt dafür einen frischen `sync_stat` des tatsächlichen Roots sowie plain Root und Cancellationprüfung. Save, Listing und Retention verwenden danach die vorhandenen privaten Appdata-Sicherungen; Owner-/Restoregrenzen bleiben in den Versionsmodulen. |
| Sync-Literalpfade | `bisync/core/sync_relative_path.rs` erhält Providerliterale für Apply, Baseline, Checkpoints, SQL, Versionsmanifeste und Recovery. Native Zugriffe prüfen zusätzlich die tatsächlichen Backendlimits; `legacy_backup_path.rs` hält den physischen Pfadvertrag alter Archive. Die separate Agent-Wire-Grammatik bleibt in `agent_proto/core/relative_path.rs`. |
| Alte seitenspezifische Syncnamen | `bisync/core/keys.rs::PathAliases` bildet ausschließlich belegte alte Seitenpfade auf ihren gemeinsamen logischen Schlüssel ab. `os/shared/state_spelling_policy.rs` liest und validiert historische Viermap-Records; `state_spelling_aliases.rs` verbindet diese Zuordnung mit Indexbeweis und berührten Änderungen. `state_spelling_history.rs::DirectoryHistory` trennt historische gefaltete Existenzkeys von Literalankern und rekeyed über den bestehenden Checkpointjournalpfad. Bestätigte Ordneraktionen verwenden ihren logischen Aliaskey; fehlgeschlagene Aktionen behalten ihre Basis. Vollscan, Vorschau, inkrementeller Lauf und gespeicherte Auflösung erhalten tatsächliche I/O-Pfade und schützen belegte physische Zielkollisionen. |
| Desktop-Close/Update bei laufendem Sync | `app/os/shared/sync_exit_gate.rs`, `app/core/sync_run_state.rs`; tatsächliche Worker-Completion gibt Shutdown frei |
| Live-Sync-Protokoll je Job | `bisync/os/shared/run_log.rs` (Datei `<sync data>/job-logs/<id>.log`, Rotation 8 MiB, Leser mit Offset, Aktivität als Fortschritt, Thread-Kontext des Laufs, `LoggingSink` um den Apply-Beobachter), Zeilentexte `run_log_lines.rs`; Ergebniszeile aller Läufer in `syncjobs::record_attempt`; Anzeige Desktop `app/core/sync_job_log_ui.rs`, Android `sync.log` (`mobile/.../sync_log.rs`) → `ui/sync/SyncLogScreen.kt` |
| Wiederaufnahme nach Anmeldefehlern, tote Laufmarken | `daemon/os/shared/job_recheck.rs` (Belege: `creds::credentials_revision`, späterer Erfolg auf demselben Google-Konto; Laufmarke ohne Lebenszeichen → `JobState::interrupted` + Kontrolllauf), Editor-Beleg `syncjobs::job_state_store::recheck_after_edit`; Zulassung `due::recheck_pending`. Keine automatischen Login-Versuche ohne Beleg (Kontosperren). |
| Drive-Änderungsfeed je Sync-Ordner | `gdrive/core/change_scope.rs` (bekannte IDs aus dem Konto-Cache, Elternkette per `files.get`, Shared Drive = Teilabdeckung); `realtime.rs` wertet `ChangeNotice::Ready` als vollständige Abdeckung. Vergleich mit Open-Source-Clients: `docs/refs/gdrive-opensource-sync-2026-10-08.md` |
| Hintergrund-Daemon | `native/src/daemon/` (`run_daemon`, eingebettet `ensure_embedded_daemon`, Nachhol-Lauf `request_catch_up`); `catch_up.rs` besitzt Abschluss/Cancel/Retry, `catch_up_attempt.rs` Zulassung und Fortschritt; `native/src/autostart/` |
| Share/P2P | `native/src/share/` (Iroh/QUIC, Profile, Discovery, Räume), stabile Fassade `share/api_exports.rs`, Exec-Zulassung `share/core/exec_admission.rs`, LAN-Probe-/Dial-Actor `share/os/shared/lan_link_dial.rs`; Share-Server `share-server/` |
| Eigene Share-Stages | `share/core/peer_stages.rs` hält Creatortickets pro `PeerBackend` für die gemeinsame enge Unique-Stage-Grammatik unter `vfs/core/staging_names.rs`; `peer_writer.rs` bestätigt die exklusive Erstellung erst nach Writer-ACK. Finish, Veröffentlichung und Discard konsumieren diese Grenze über `peer_extensions.rs`, `peer_transfer.rs` und `peer_reversible_replace.rs`. Unklare Veröffentlichungen bleiben gesperrt. |
| Schreibfähigkeiten ohne neue Mountbindung | `Backend::probe_staged_write_capabilities` liefert einen falliblen Staged-Write-Snapshot. Der Peer verwendet seine nicht-erwerbende Capability-Abfrage; Cache und Share-Guard leiten sie mit ihren vorhandenen Grenzen weiter. Die drei Bisync-Publishconsumer ändern dadurch während Stage/Backup/Overwrite keine bestehende Lease oder Wurzel. Echte Mounts verwenden weiterhin `mount_path_capabilities`. |
| Explizites Öffnen alter Direct-Peers | `share/core/service.rs::probe_backend_for_target` → `legacy_probe.rs` (vollständig gebundener Pending-Snapshot und echte read-only Peerprobe) → `share/os/shared/legacy_probe_persist.rs` (frische Identitäts-/Entzugsprüfung, Kontakt-CAS, canonical Runtime-Refresh) |
| Share-Energie/Ruhemodus | `share/core/power.rs` (prozessweit: `set_low_power`, `request_probe`, Wachhalte-Hook), Signal-Worker `signal_worker.rs` + `signal_{connected,session,idle,schedule,power,publish,readiness}.rs` (ereignisgesteuert, Wächter-Thread `share-signal-rd`), Leerlauf-Aufräumen `node_idle.rs`; Server `share-server/src/{idle,idle_outbox,signal_session,transport_serve,writer_idle}.rs` (Fähigkeit `idle_keepalive_v1`, Takt K, Bündelung), Relay-Ping-Plan im vendored `iroh-relay` (`AccessControl::ping_schedule`) |
| Eigene Discovery-Angebote (suchbar machen) | Daemon-Buch `share/core/discovery_offer_book.rs` (aus Worker-Ereignissen, in `ShareWorkerSnapshot.discovery_offers`), IPC `daemon/os/shared/ipc_host_commands.rs` (`send_command` → `ShareCommandReply`), CLI `cli/share/discoverable.rs` |
| Speicheranalyse/Duplikate | `native/src/analytics/` (geschützte Bereiche `core/protected.rs` + `apptrash::ProtectedAreas`, Anzeige-Schätzzeilen `core/storage_view.rs`, Android-Duplikatsuche `os/shared/reclaim/finder*.rs`) |
| Fernanalyse, behaltene Ergebnisse und Host-Papierkorb | `share/os/shared/{storage_analysis_host,analysis_tasks,analysis_spool,host_watch}.rs`, `analytics/os/{windows,linux_os,linux_trash}.rs`; handlebasierter Zugriff über `local_access/` |
| Analyse-Bedienung auf dem Desktop | `app/core/analytics_controls_ui.rs` für Auswahl, Navigation, Fortschritt und Cancel; `analytics_ui.rs` für Treemap und nachgelagerte Aktionen |
| Agent-Wire-Framing | `agent_proto/core/types.rs` definiert `Frame`/`PROTO_VERSION`; `codec.rs` reexportiert den Encoder/Decoder aus `frame_io.rs`. Bestehende Vault-Szenarien liegen in `vault_frame_task_tests.rs` mit privaten I/O-Probes in `vault_frame_fixture.rs`. |
| Updates | Desktop `native/src/updater/`, Terminal `se update` `cli/update.rs` → `updater/os/shared/terminal.rs` (Feed-Prüfung, Installationsart, Ersatz an Ort und Stelle), Android `update.*` in `native/src/mobile/os/shared/domains/` |
| Release | `native/publish-release-local.ps1` (einziger Einstieg), `docs/RELEASING.md` |

## Module und Ordner
- `native/src/*/core/` plattformneutrale Logik; `*/os/{windows,linux_os,android,shared}` Adapter,
  per `#[cfg]`/`#[path]` in `mod.rs` gewählt (Android hat eigene Arme; `target_os = "android"` ist nicht `linux`).
- `native/src/app/` nur Desktop (egui); egui-freie Logik liegt in Kernmodulen, `app/` hält Re-Export-Schalen.
- `native/src/mobile/` nur Android (und Linux-Tests): JSON-Fassade, Laufzeit, Tasks/Ereignisse,
  Domänen unter `os/shared/domains/`.
- `native/src/apptrash/` App-Papierkorb je Speichervolume (Android), `native/src/android_fs/` Rename-Kette für FUSE-Speicher.
- `native/android-bridge/` cdylib `libsmart_explorer_android.so` (Workspace-Mitglied; `explorer-command` ist ausgeschlossen).
- `android/` Gradle-Projekt (Modul `:app`): `core/` Brücke/DTOs, `ui/{files,sync,share,more,…}`, `service/`, `work/`, `system/`.

## Datenwege
- Android-Aufruf: Compose-Bildschirm → `Core.call(method, args)` (IO-Thread) → JNI `NativeBridge.call` →
  `mobile::call` → Dispatcher → Kernmodul (z. B. `vfs::Backend::list_dir`) → JSON-Antwort.
  Lange Vorgänge: Task-ID, Fortschritt über `pollEvents` (Ereignispumpe) → `Core.tasks`.
- Hintergrund Android: WorkManager `SyncWorker` → `bg.catchUp` → eingebetteter Daemon-Supervisor →
  `job::run_one` → `bisync::run`; Dauerbetrieb oder „Share im Hintergrund erreichbar“ hält den Prozess mit
  `BackgroundService` (specialUse); `system/KeepAlive*.kt` (Wach-Alarm 10 min → `share.wake`, Netzwechsel),
  `WakeKeeper` (Ereignis `wake` → Partial-Wakelock), `HostMonitor` → `sys.hostState` (Ruhemodus, `deferScheduling`).
- Desktop: GUI ↔ Daemon-Prozess über Loopback-TCP-IPC (`daemon/os/shared/ipc*.rs`); Share-/Agent-Anfragen
  mit Kredit je Anfrage (`agent_proto/core/credit.rs`), damit eine langsame Übertragung kein Blättern blockiert.
- Übertragung: App/Android → `TransferRequest::Job` → Lane → `engine::run_job` → Walker listet parallel unter
  dem Flow der Quelle, Worker holen Erlaubnisse der regelnden Flows, schreiben in private Stufen und
  veröffentlichen ohne Ersetzen; Fortschritt ~150 ms, Fehler als JSON-Zeilen in einer Protokolldatei.
- Persistenz: App-Daten unter `support_dirs::app_data_dir()` (Android: `<filesDir>/smart_explorer`),
  Sync-Jobs `sync/jobs/*.conf`, Zugangsdaten `secrets-v1/` (Datei-Store), Share-Profile/Identität.
  Private Ancestorpfade bleiben auf Android nur durchsuchbar; der private Leaf wird lesbar geöffnet.
  Windows-Journale nutzen RW plus expliziten Seek, damit bestätigte Tail-Kürzung ihre nötigen Handle-Rechte hat.
  Private Windows-Verzeichnisse vererben Owner-Zugriff an gewöhnliche Konfigurations-/Job-/Control-Kinder;
  private Dateien behalten ihre geschützte DACL ohne Vererbungsflags.
- Sync: gespeicherter Endpunkt → gemeinsame VFS-/Literalpfad-Auflösung → `StateKey`/Pairlock →
  optionsbewusster Snapshot → `ApplyScope` mit Checkpoints, Versionen und vorab dauerhaftem
  ReplacementIntent → bestätigte Teilaktionen. Pending-Merge und unklare Veröffentlichung
  schützen ihre Originalpfade bis zum ausdrücklichen Wiederanlauf.
  Ein bestätigter Parent-Namespace ersetzt keinen ausstehenden Fileflush. Unbekannte Adapter
  bestätigen ihn nicht; Androids zusätzlicher FUSE-Pfad verlangt die tatsächliche System-Storage-
  Mountinstanz und erfolgreiches Directory-fsync. Vertrag: `docs/refs/post-publication-namespace.md`.
- Share: vollständig gepinnter Principal → aktuelle persistierte Export-/Kontakt-/Raumrechte →
  OS-Handle-/Pfadgrenze. Statuskanal und mDNS erteilen keine FS-/Exec- oder Uplink-Rechte;
  Rechteentzug invalidiert betroffene Sitzungen, ein Transportabbruch allein keine Analyse-Retention.
  Aktuell zugelassene gepinnte Altpeers nutzen vorhandene ListDir-/Dateisystemabläufe mit konservativen
  Fähigkeiten ohne Mount-Lease; moderne Status-/Entscheidungsgarantien werden dadurch nicht ersetzt.

## Externe Abhängigkeiten
Rust-Crates und ihre Android-Tauglichkeit: `docs/refs/android-rust-deps.md`; JNI: `docs/refs/rust-jni-022.md`;
Gradle/AndroidX-Stände (AAR-Metadaten geprüft): `docs/refs/android-gradle-build.md` §8;
Android-APIs: `docs/refs/android-apis.md`, `docs/refs/android-platform.md`; CI: `docs/refs/android-ci.md`.

## Bauen, Prüfen, Konventionen
- Keine lokalen Builds/Tests (AGENTS.md); Prüfung ausschließlich über die eine Remote-Task-Suite je Batch
  (Android: `.github/workflows/android-task.yml` → `android/test-android-task.sh`).
- RV1: `.github/workflows/review-task.yml` → `native/test-review-task.sh`; eine kandidatengebundene
  Suite mit nativem Linux/Windows und Android-Build/Gerät. Vertrag: `docs/refs/rv1-remote-suite.md`.
- Windows-Startregression 0.5.170: `startup-regression-task.yml` →
  `native/test-startup-regression-task.py`, reale bestehende DACLs und Worker-Handoff
  aus v0.5.169 mit den inkrementellen Development-Ausgaben derselben RV1-Caches.
- Sync-Transparenz 2026-10-08: `sync-transparency-task.yml` → `native/test-sync-transparency-task.py`
  (Präfix `sync_transparency_task_`, Linux/Windows plus Android-Build). Plan: `docs/plaene/2026-10-08-sync-transparenz/`.
- Sync-Verlässlichkeit: `sync-reliability-task.yml` → `native/test-sync-reliability-task.py`
  als einziger Remote-Einstieg für native Provider-/Job-/Wiederanlaufverträge und
  echtes Android-Altappupdate. Plan und Ergebnisorakel: `docs/plaene/2026-10-04-sync-verlaesslichkeit/`.
  Reale Google-Abnahme benötigt autorisierte Test-Secrets; fehlende Anmeldung ist kein Pass.
- Android-Bibliothek: `cargo ndk -t arm64-v8a -t x86_64 --platform 30 -o android/app/src/main/jniLibs build -p smart_explorer_android`
  (im Verzeichnis `native/`), NDK aus `android/ndk-version`; Gradle braucht `-PrustlsVerifierMaven=<Pfad>`.
- Release-APK: `android/build-release-apk.sh` (Job `android-release-apk` in `build.yml`), Signatur aus Repo-Secrets.
- Neue Rust-Dateien < 500 Zeilen; Test-Präfix je Batch (Android: `android_task_`, Übertragungs-Engine:
  `transfer_engine_task_`, Suite `native/test-transfer-engine-task.sh` über `transfer-engine-task.yml`;
  Android-Hintergrund/Analyse: `android_background_task_`, Suite `native/test-android-background-task.sh` über
  `android-background-task.yml`, Gerätestufen in `android/test-android-task.sh`).
