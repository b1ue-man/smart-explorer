# Bestand: Datenwege der Baumerfassung und des Vergleichs (Stand 2026-10-31)

Stand: Code-Stand `HEAD 29310ace` (`git log -1 --oneline`). Diese Datei beschreibt den **heutigen Code**; sie enthält keine Planung. Alle Zeilenangaben sind Datei:Zeile im Arbeitsbaum. Laufzeiten sind **nicht gemessen**: Aussagen über Unterschiede sind Ableitungen aus dem Code und als Ursachen-Kandidaten mit Beleg gekennzeichnet.

## 0. Auftrag, Umfang, Annahmen

- Auftrag: Bestand der Wege, die Verzeichnisbäume erfassen oder zwei Bäume vergleichen, mit Aufrufkette, Systemaufrufen bzw. Netzanfragen je Ordner und je Datei, Parallelität, Barrieren, Wartezeiten, festen Werten, Prüfschlüsseln, Folge-Läufen, Gegenseiten-Implementierung und Ursachenliste.
- Kurskorrektur (Auftraggeber, 2026-10-31): Umfang sind **alle** Dateioperationen dieser Art, lokal und entfernt. Die Beispiele (Sync zwischen zwei Laufwerken, Speicheranalyse, rekursive Ansicht) sind keine Abgrenzung.
- Die vier Hauptwege und die weiteren Baum-Operationen stehen in den Abschnitten 4 bis 7 und 10.
- Annahmen (ausdrücklich):
  - A1: „rekursive Ansicht“ = Ctrl+R (`native/src/app/core/frame_keyboard.rs:310-311`, `KbdAct::ToggleRecursive`). Sie gilt lokal und entfernt.
  - A2: „Vergleichen“ = Knopf „Vergleichen“ eines Sync-Jobs (`native/src/app/core/menus_sync_jobs.rs:134-137`).
  - A3: „Speicheranalyse“ = Analyse-Ansicht: `analytics::scan` (lokal) und `analytics::scan_remote` (entfernt).
  - A4: „Sync-Job-Lauf“ umfasst den Dienst-Lauf (`native/src/daemon/os/shared/job.rs:217`), den GUI-Lauf (`native/src/app/core/sync_core.rs:228`), den GUI-Manuallauf (`native/src/app/os/shared/sync_manual_run.rs:307`) und den mobilen Lauf (`native/src/mobile/os/shared/domains/sync_run.rs:281`).
  - A5: „Standard-Job“ = Werte aus `SyncJob::new`: Richtung `Both` (`native/src/syncjobs/core/types.rs:195`), `include_hidden = true` (Zeile 199), `verify_target_secs = 86400` (Zeile 211), `delete_policy = Propagate` (Zeile 216), `compare = MtimeSize` (Zeile 218), `max_delete_pct = 50` (Zeile 224), `cross_mounts = false` (Zeile 227).
  - A6: Die vier Gegenseiten sind: lokaler Ordner oder lokales Laufwerk (`LocalBackend`), Direct Share (Share-Peer, `PeerBackend`), SSH mit se-agent (`AgentBackend`), SSH/SFTP ohne Agent (SFTP-Backend). Die SFTP-Interna sind nicht gelesen (Abschnitt 14, O3).
- Pfade ohne Präfix sind relativ zu `native/src/`; Pfade mit Präfix `native/`, `docs/` oder `se-agent/` sind vollständig angegeben.
- Nicht gelesene Teile sind in Abschnitt 13 je Datei genannt; offene Punkte sind in Abschnitt 14 exakt benannt.

## 1. Befunde im Überblick

1. **Der Standard-Job liest bei jedem Lauf und bei jeder Vorschau beide Seiten vollständig.** Inkrementell laufen nur einseitige Spiegel mit vorhandenem Index (Abschnitt 8). `orchestration.rs:344-349`, `incremental.rs:56-77`.
2. **Entfernt: der Standard-Job wählt den Durchlauf auf der Gegenseite nie.** `cross_mounts = false` (`syncjobs/core/types.rs:227`) bricht `walk_snapshot_via_agent` ab (`snapshot_agent.rs:25-31`, Bedingung `!opts.cross_mounts`, Zeile 26). Dann läuft der Ordner-für-Ordner-Pfad: je Ordner mindestens `sync_stat` und `list_dir_tolerant` (`snapshot_walk.rs:74-92`).
3. **Entfernt, Share-Gegenseite: je Eintrag eine Netzanfrage.** `scan_listing` ruft `sync_child_path` für jeden Eintrag auf, auch für gefilterte (`snapshot_dir.rs:120`, Ausführung vor dem Filter in Zeile 141-171). Der Share-Client sendet bei Fähigkeit `literal_children_v1` je Eintrag `SyncChildPath` als eigene Rundreise (`share/core/peer_literal_paths.rs:17-25`). Es gibt keine Stapelung.
4. **Entfernt, Speicheranalyse: eine Anfrage für den ganzen Baum.** Share: `FsStorageAnalysis` (`share/core/peer_storage_analysis.rs:37-48`). Agent: `WalkTree` mit einem `Tree`-Frame (`agent/core/walk.rs:15-70`, `agent_proto/core/server.rs:81-115`). Keine Stat-Anfrage je Ordner, keine Flow-Permits (`analytics/os/shared/remote.rs:10-53`).
5. **Prüfschlüssel im Standardmodus `MtimeSize`: keine.** Für Share-, Agent- und SFTP-Seiten liefert `hash_mode` `None` (`bisync/os/shared/snapshot_hash.rs:78-88`), weil `provides_content_hash` dort `false` ist (`vfs/core/core.rs:368-370`, keine Überschreibung in Share/Agent/SFTP-Clientcode; Suche über `native/src`).
6. **Prüfschlüssel im Modus `Checksum` ohne Host-Walk: vollständiger Download je Datei.** `signature_hash` führt `content_hash` aus (`snapshot_dir.rs:238-243`); dieser liest den Inhalt über das Netz (`snapshot_dir.rs:250-280`, `hash_streamed` `:282-316`) und macht dazu vier Metadatenanfragen (Abschnitt 3.4).
7. **Rekursive Ansicht entfernt: Barriere je Block von Ordnern.** `rscan/os/shared/parallel.rs:24-60` wartet je Block der Länge `parallelism` (begrenzt auf 2 bis 16, Zeile 9) auf die langsamste Auflistung; danach eine Barriere je Ebene (Zeile 62).
8. **Lokal: Mount-Fakten je Verzeichnis im Sync-Walk, ohne Cache.** `snapshot_policy::protected` ruft für jedes lokale Verzeichnis `local_mount_boundary` und `snapshot_mounts::missing` auf (`snapshot_policy.rs:98-110`); letzteres liest je Aufruf die Metadaten von `nested-mounts.json` (`snapshot_mounts.rs:55`, `stamp()` Zeile 19-25) und sperrt einen globalen Mutex (`snapshot_mounts.rs:51-54`). Der Cache aus `apply_path_context` wirkt nur im Anwendungsschritt (`apply_reporting.rs:125-139`), nicht beim Einlesen.
9. **Lokal: je Eintrag Schutz-Prüfung mit Mutex und vier Umgebungsvariablen.** `own_path` (`snapshot_policy.rs:6-14`) ruft `own_prefixes()` auf, das die Umgebung neu liest (`prefix_key()`, Zeile 28-36) und den Cache sperrt (Zeile 40-64). Dies gilt für jeden lokalen Eintrag, der nicht schon als Motor- oder Staging-Name abgewiesen wurde (`snapshot_policy.rs:73-75`).
10. **Lokal: die Analyse und der Scanner der rekursiven Ansicht prüfen keine Mount-Grenze.** In `native/src/analytics/**`, `native/src/scanner/**` und `native/src/folder_index/**` gibt es keine Prüfung auf Gerätewechsel (`dev()`/Mount); die einzigen `dev`-Vergleiche liegen in der Papierkorb-Zuordnung (`analytics/os/linux_trash.rs:19-31`) und in der Watch-Baumerfassung (`watch/os/linux_os/inotify_tree.rs`, nur bei `cross_mounts = false`).
11. **Lokale rekursive Ansicht: jeder Eintrag wird an die GUI geschickt.** `scanner/os/walk.rs:210-235` baut je Eintrag ein `FileEntry` (`Arc<str>` für Pfad, Name und Erweiterung, Elternpfad als geteilter Verweis), `walk.rs:347-367` sendet Stapel zu 1024 Einträgen oder alle 60 ms. Die GUI hängt alle Einträge an (`app/os/shared/platform_helpers.rs:28-45`) und baut die Ansicht am Ende neu (`app/core/drains_connect.rs:23-37`).
12. **Lokale Analyse: je Eintrag nur Größe und Zählung.** `analytics_walk.rs:55-110` überspringt Links und Sonderdateien und hält nur `MAX_RETAINED_FILES_PER_DIRECTORY` Dateien je Ordner (`analytics_backend.rs:252-284` für den Remote-Pfad, `analytics_walk.rs:105-110` für den lokalen).
13. **Parallelität ist nicht der Unterschied zugunsten der Analyse.** Desktop-Analyse: 2 Threads (`analytics/os/mod.rs:27-29`, `analytics.rs:225-231`); Scanner: höchstens 4 (`scanner/os/shared.rs:433-436`). Die Analyse ist pro Eintrag leichter (Befund 12), nicht breiter.
14. **Konfliktvorschau beim Einfügen (Zusammenführen): serieller Quellbaum mit einer Zielanfrage je Datei.** `transfer/os/shared/engine/selection.rs:114-224` läuft den Quellbaum im Stapel ab (Zeile 127-131), ruft je Ordner `list_dir_tolerant` (Zeile 135-137) und je Datei `target.stat` auf (Zeile 201).

## 2. Feste Werte

Jede Zeile nennt den Ort des Werts. Werte ohne Angabe sind nicht gelesen.

| Bereich | Wert | Ort |
|---|---|---|
| Snapshot: Abfrage des Lese-Threads | 50 ms | `bisync/os/shared/snapshot_pair.rs:22` |
| Snapshot: Koordinator-Takt | 50 ms | `bisync/os/shared/snapshot_walk.rs:23` |
| Snapshot: Leerlauf eines Listing-Threads | 250 ms | `snapshot_walk.rs:27`, `:240-244` |
| Snapshot: maximale Tiefe | 512 | `snapshot_walk.rs:20` |
| Prüfung je Datei: Lesepuffer (Stream) | 65 536 Byte | `snapshot_dir.rs:21`, `snapshot_hash.rs:42` |
| Budget: Einträge je Seite (Minimum) | 1 000 000 | `bisync/core/limits.rs:37-42` (`FALLBACK`), Ableitung `:47-61` |
| Budget: Pfadtext je Seite (Minimum) | 128 MiB | `limits.rs:39` |
| Budget: je Eintrag im Speicher | 512 Byte | `limits.rs:9` |
| Überlast: Geduld | 5 × 60 s = 300 s | `bisync/os/shared/sync_overload.rs:22`, `transfer/core/engine_policy.rs:18` (`RETRY_AFTER_MAX` = 60 s) |
| Flow: Startgrenze | 2 | `docs/TRANSFER_ENGINE.md:71` (`START_LIMIT`) |
| Flow: Mindeststart Share | 8 | `transfer/os/shared/flow_registry.rs:17` (`PEER_START_LIMIT`), `:113-125` |
| Flow: Mindeststart SFTP/SMB/WebDAV | 8 | `flow_registry.rs:128` (`REMOTE_START`), `:119-121` |
| Flow: Mindeststart FTP/Drive | 4 | `flow_registry.rs:131` (`SESSION_START`), `:122` |
| Flow: Mindeststart drehende Platte | 8 | `flow_registry.rs:33` (`ROTATING_START`) |
| Flow: Startwert lokal | 2 × Kerne (Unix), 4 × Kerne (Windows) | `transfer/os/unix.rs:25`, `transfer/os/windows.rs:20` |
| Flow: Merkfrist | 15 min | `docs/TRANSFER_ENGINE.md:78` |
| Backend-Parallelität, Standard | `rayon::current_num_threads()` (Kerne) | `vfs/core/core.rs:261-263` |
| Backend-Parallelität, Überschreibungen | FTP 1 (`ftp/core/ftp.rs:441-443`), WebDAV 2 (`webdav/core/webdav.rs:440-442`), Zip 1 (`zipfs/os/shared/zipfs.rs:248-250`), Gerätemount 1 (`daemon/os/shared/devices_mount.rs:102-104`), Tresor-Aufgabenbaum 8 (`daemon/os/shared/vault_task_tree.rs:82`) | jeweils genannte Stellen; SFTP, SMB, Share-Peer, Agent und lokal überschreiben nicht (Suche über `native/src`) |
| Scanner (rekursive Ansicht, lokal): Threads | `min(available_parallelism, 4)`, Stack 8 MiB | `scanner/os/shared.rs:433-439` |
| Scanner: Fortschritt | 150 ms | `scanner/os/shared.rs:408` |
| Scanner: Stapel an die GUI | 1024 Einträge oder 60 ms | `scanner/os/walk.rs:15-16` |
| Scanner: Teilbäume vor Auflistung | ab 128 Unterordnern | `scanner/os/walk.rs:290-302` |
| Analyse lokal: Threads | Standard Desktop 2, Android `min(Kerne, 4)`; Umgebungsvariable `SMART_EXPLORER_ANALYTICS_THREADS`; Begrenzung 1 bis 4 | `analytics/os/mod.rs:22-29`, `analytics.rs:225-231` |
| Analyse lokal: Stack je Thread | 64 MiB | `analytics.rs:50` |
| Analyse entfernt: Listen je Analyse | `parallelism`, begrenzt auf 1 bis 16 | `analytics/os/shared/analytics_backend.rs:17`, `:34` |
| Analyse entfernt: Stack | 64 MiB | `analytics_backend.rs:39` |
| rekursive Ansicht entfernt: Blockgröße | `parallelism`, begrenzt auf 2 bis 16 | `rscan/os/shared/parallel.rs:9` |
| rekursive Ansicht entfernt: Stapel | 256 Einträge; Fortschritt 150 ms | `rscan/os/shared/rscan.rs:27-28` |
| rscan-Budget | 1 000 000 Einträge, 128 MiB Text, Tiefe 512 | `rscan/os/shared/budget.rs:3-5` |
| Einfügen: Fortschritt | 150 ms | `transfer/os/shared/engine/mod.rs:74` |
| Einfügen: Abfrage der Vorschau-Schleife | 100 ms | `engine/selection.rs:359` |
| Share: Listen-Stapel | 2048 Einträge, geschätzt 512 KiB | `share/core/list_batch_wire.rs:10-12` |
| Share: je Eintrag im JSON | 192 Byte Gemeinkosten | `list_batch_wire.rs:13` |
| Share: Frame-Grenze | 4 MiB | `list_batch_wire.rs:15` |
| Share: Hash-Walk-Stapel | `LIST_BATCH_MAX_BYTES` mit Faktor 6 je Eintrag | `share/os/shared/host_hash_walk.rs:44-57` |
| Share: Hash-Puffer | 1 MiB | `host_hash_walk.rs:116` |
| Share: Tiefe Hash-Walk | 2048 | `host_hash_walk.rs:159`, `:285` |
| Share: Wiederholbare Lesevorgänge (`Stat`, `SyncChildPath`, `ListDir`, …) | Budget 40 s | `share/core/peer_request.rs:21`, Liste `peer_request_policy.rs:3-17` |
| Share: alle übrigen Anfragen (alle `FsRequest`-Varianten, die `is_retryable_read` nicht nennt, `share/core/peer_request_policy.rs:3-17`) | 60 s | `share/core/io_deadline.rs:7` |
| Share: Hash-Walk-Warteschlange | Wiederholung nach 10 ms | `share/core/peer_hash_walk.rs:148` |
| Agent: Protokollversion | 12 | `agent_proto/core/types.rs:3` |
| Agent: Anforderungen je Verbindung | 64 (Budget 64 MiB ÷ Startwert 1 MiB) | `agent_proto/core/credit.rs:23`, `:27`, `:33` |
| Agent: Metadatenanfrage | 20 s | `agent/core/backend.rs:18` |
| Agent: Abfrage Baum-Walk | 250 ms | `agent/core/walk.rs:28` |
| Agent: Abfrage Hash-Walk | 200 ms | `agent/core/search.rs:115` |
| Agent-Server: Fortschritt Baum-Walk | 200 ms | `agent_proto/core/server.rs:96` |
| Agent: Teil einer toleranten Listung | 1 MiB | `agent_proto/os/shared/ext_ops.rs:20` |
| Agent-Server: Baum-Walk-Budget | 1 000 000 Knoten, 128 MiB Text, Tiefe 512 | `agent_proto/os/shared/fs.rs:8-10` |
| Ordnerindex: Fortschritt | 200 ms | `folder_index/os/walk.rs:10` |
| Kopiersammlung (rekursiv): Budget | 1 000 000 Einträge, 128 MiB, Tiefe 512, 100 Meldungen | `scanner/os/collect.rs:8-11` |

## 3. Gemeinsamer Abgleichskern (Sync-Snapshot)

Dieser Abschnitt gilt für den Sync-Lauf (4) und die Vergleichsvorschau (5). Beide rufen dieselbe Einleseroutine auf.

### 3.1 Aufrufkette bis zu den Netzanfragen

1. `read_pair` (`bisync/os/shared/snapshot_pair.rs:61-69`) → `read_pair_with` (`:73-173`). Zwei Threads, `sync-read-a` und `sync-read-b` (`:151-160`), rufen je `walk_snapshot_observed` auf (`:119-149`, Aufruf `:129-140`). Der Hauptthread fragt alle 50 ms den Zustand ab (`:166-171`) und wartet auf beide Seiten (`:172`).
2. `walk_snapshot_observed` (`bisync/os/shared/snapshot.rs:178-254`):
   - `apply_boundary::guard` für die Wurzel (`snapshot.rs:196`, `apply_boundary.rs:24-33`: ein `sync_stat` der Wurzel).
   - Agent-Durchlauf, falls zulässig (`snapshot.rs:197-208`; Bedingungen in Abschnitt 3.6).
   - Sonst `walk_tree` (`snapshot.rs:236`, Parallelität `be.parallelism().max(1)`).
   - Der Flow wird nur für entfernte Seiten gebildet (`snapshot.rs:213`, `be.is_local()`).
3. `walk_tree` (`snapshot_walk.rs:30-65`): ein Koordinator (`:150-176`) startet bis zur Grenze `may_grow` Listing-Threads (`:178-189`). Remote: Grenze `flow.snapshot().limit + 1` (`:179-180`). Lokal: Grenze `local_threads` = `parallelism()` (`:181`). Arbeit kommt aus einer Warteschlange (`:226-262`). Ein Thread wartet bei leerer Schlange und laufender Arbeit höchstens 250 ms (`:240-244`).
4. Je besuchtem Ordner (`visit`, `snapshot_walk.rs:274-359`):
   - Remote: ein Listing-Permit des Flows (`flow.acquire_meta`, `:298`) unter der Überlast-Logik (`under_permits`, `sync_overload.rs:120-148`).
   - `list_plain_directory` (`snapshot_walk.rs:74-92`): `sync_stat(dir)` (`:76`), danach bei normalen Ordnern `snapshot_policy::protected` (`:85-89`), danach `list_dir_tolerant(dir)` (`:91`).
   - Auslassungen aus dem Listing werden registriert (`:326-339`).
   - `scan_listing` (`:340`) läuft **nach** dem Permit-Block, also ohne Listing-Permit (Abschnitt 3.3).
5. `scan_listing` (`bisync/os/shared/snapshot_dir.rs:62-205`) prüft jeden Eintrag in dieser Reihenfolge:
   - Namensprüfung `validate_component` (`:78-90`).
   - Doppelnamen-Prüfung (`:92-109`).
   - Literalpfad `sync_child_path` (`:120`, Netzanfrage bei Share, Abschnitt 3.5).
   - Budget (`:130-140`).
   - Ausschluss `excluded` aus versteckt und Filter (`:141-142`).
   - `snapshot_policy::protected` (`:143`).
   - Filter nach Größe und Alter (`:157-171`).
   - Für Ordner: Eintrag in `dirs` und in die Warteschlange (`:172-178`).
   - Für Dateien: `signature_hash` (`:180`, Abschnitt 3.4), danach `Sig` (`:193-201`).
6. `merge` (`snapshot_walk.rs:361-388`) sortiert je Ordner die Dateien nach Name, dann Änderungszeit absteigend und Kennung (`:364-369`) und fügt sie in den gemeinsamen Baum ein.
7. Nach beiden Seiten: `omissions.exclude_tree`, Dublettenplan, Auslassungen (`snapshot_pair.rs:181-195`).

### 3.2 Netzanfragen je Ordner und je Datei im Standardmodus

Bezeichnung: „Stat“ = `sync_stat` (Standard `stat`, `vfs/core/extensions.rs:102-104`), „Listing“ = `list_dir_tolerant`.

| Seite | je Ordner | je Eintrag (Datei und Ordner, auch gefiltert) | je Datei im Modus `MtimeSize` |
|---|---|---|---|
| Share-Peer | 1 Stat (`share/core/backend.rs:182-189`, `FsRequest::Stat`) + 1 Listing (`peer_list_batch.rs:22-29`, `ListDirBatch`, mehrere Teile; ohne `list_batches_v1`: `ListDir`, `peer_list_batch.rs:12-16`) | 1 `SyncChildPath`, wenn der Host `literal_children_v1` meldet (`peer_literal_paths.rs:17-25`); sonst keine Anfrage (`:17-21`) | 0 |
| SSH mit se-agent | 1 Stat (`agent/core/backend.rs:166-172`, `Frame::Stat`) + 1 Listing (`agent/core/extensions.rs:175-197`, `Frame::ListTolerant`, Teile je 1 MiB, Abschluss `End`; ohne Fähigkeit `ext-v1`: `Frame::ListDir`, `:175-179`) | 0 Netzanfragen im Client-Code; `sync_child_path` delegiert an das innere Backend (`agent/core/extensions.rs:162-164`), dessen Anfragen sind offen (O4) | 0 |
| SSH/SFTP ohne Agent | 1 Stat (Standard `stat`) + 1 Listing (`sftp/core/extensions.rs:64`, Überschreibung von `list_dir_tolerant`) | Standard `sync_child_path` ohne Netzanfrage (`vfs/core/extensions.rs:95-97`), sofern SFTP es nicht überschreibt (ungeprüft, O3) | 0 |
| Lokal | 1 Stat (Standard `stat`; Annahme: ein Systemaufruf, `LocalBackend::stat` nicht gelesen, O1) + Listing (`vfs/os/shared/local_extensions.rs:57`, nicht gelesen, O1) + Mount-Fakten je Verzeichnis (Abschnitt 3.7) | 0 Systemaufrufe im Walk-Code selbst; je Eintrag: Schutz-Prüfung mit Mutex (Befund 9) | 0 |

Hinweis zur Wurzel: Die Wurzel wird zweimal per `sync_stat` angefragt (`apply_boundary.rs:33` und `snapshot_walk.rs:76`).

Gefilterte Einträge kosten je Share-Eintrag ebenfalls die Anfrage `SyncChildPath`, weil `sync_child_path` vor dem Filter steht (`snapshot_dir.rs:120` vor `:141-171`).

### 3.3 Parallelität, Barrieren, Wartezeiten

- Beide Seiten laufen gleichzeitig (`snapshot_pair.rs:150-160`). Der Lauf endet erst, wenn beide Seiten fertig sind (`:166-172`). Barriere.
- Je Seite: Koordinator und Listing-Threads (Abschnitt 3.1, Punkt 3). Remote-Grenze `flow.limit + 1`; die Startgrenze der Flow-Art Share ist 8 (Abschnitt 2), SFTP ebenfalls 8, FTP und Drive 4. Die Grenze lernt der Flow (`docs/TRANSFER_ENGINE.md:71-76`); Überlast halbiert sie (`:76`).
- Keine Ebenenbarriere: eine Warteschlange, Threads holen Arbeit (`snapshot_walk.rs:226-262`). Ein Thread wartet höchstens 250 ms auf neue Ordner (`:240-244`).
- Je Ordner ist die Verarbeitung der Einträge eine sequentielle Schleife im Worker (`snapshot_dir.rs:74-203`); bei Share folgt daraus eine Rundreise je Eintrag (Abschnitt 3.2).
- Überlast: Wartezeit ohne Permit, dann Wiederholung; Geduld 300 s (`sync_overload.rs:77-88`, `:22`, `:120-148`).
- Frühes Kopieren (nur Sync-Lauf, nicht Vorschau): Kopierthreads `sync-early` in der Anzahl `max(parallelism)`, begrenzt durch `max_transfers` (`bisync/os/shared/early_copy.rs:167-177`); gestartet in `early_copy_record.rs:17-43`. Die Spur ist aus bei `dry_run`, `move_files`, Aliasen, Planregeln und Doppelnamen (`early_copy.rs:140-146`).
- Wartezeit Remote-Metadaten: Standard 20 s (Agent, `agent/core/backend.rs:18`), 40 s (Share wiederholbar, `peer_request.rs:21`), 60 s (Share sonst, `io_deadline.rs:7`).

### 3.4 Prüfschlüssel (Hash): Auswahl, Ort, Wiederverwendung

Modusauswahl (`snapshot_hash.rs:78-88`, aufgerufen in `snapshot_pair.rs:88` für jede Seite, Gegenseite als `other`):

| `compare` | Seite `this` | Ergebnis |
|---|---|---|
| `SizeOnly` | egal | `None` |
| `Checksum` | egal | `FullFresh` |
| `MtimeSize` | `this` liefert Inhalts-Hash | `NativeOnly` |
| `MtimeSize` | `this` lokal und `other` liefert Inhalts-Hash | `Full` |
| `MtimeSize` | sonst | `None` |

Liefert Inhalts-Hash: Google Drive (`gdrive/core/backend.rs:360-363`, Kommentar: `md5Checksum` als kostenloser Hash), WebDAV mit `oc:checksums` (`webdav/core/webdav.rs:458-461`, bedingt), der Share-Host-Wrapper (`share/core/fs_guard_backend.rs:304-306`, delegiert an das Innere), Gerätebaum nein (`mount/core/device_tree_backend.rs:289-291`). Standard `false` (`vfs/core/core.rs:368-370`). Der Rückgabewert von Drive ist im Auszug nicht vollständig gelesen (O14).

Je Datei (`snapshot_dir.rs:214-245`, `signature_hash`):

1. `NativeOnly`: nur der Hash aus dem Listing (`content_md5`), ohne Lesen (`:222`).
2. `Full`: Hash aus dem Listing, falls vorhanden (`:224-226`); sonst Wiederverwendung des Hashes der letzten Basis, wenn Größe und Änderungszeit gleich sind und der Hash von 0 verschieden ist (`:227-233`); sonst `content_hash` (`:234`).
3. `FullFresh`: Hash aus dem Listing, nur wenn vorhanden und von 0 verschieden (`:238-242`); sonst immer `content_hash`.
4. `None`: 0, keine Anfrage (`:221`).

`content_hash` (`snapshot_dir.rs:250-280`) bei entfernter Seite:

- `apply_guard::capture` (`apply_guard.rs:75-100`): eine `sync_stat`-Anfrage (`current_metadata`, `:139-150`).
- Lesen unter Flow-Permit (`flow.acquire_for`, `:267-273`), `hash_streamed` (`:282-316`): erneut `capture` (eine Anfrage), dann `open_read_regular` (eine Öffnung), dann Lesen aller Bytes (Puffer 64 KiB, `:21`), dann `revalidate` (eine Anfrage).
- `apply_guard::revalidate` nach dem Lesen (`:278`, eine Anfrage).
- Ergebnis je Datei: vier Metadatenanfragen, eine Öffnung und die vollständige Dateiübertragung.

Lokal: `hash_file` liest die Datei in 64-KiB-Blöcken (`snapshot_hash.rs:30-65`), mit zwei Metadatenprüfungen (`apply_guard`).

Wo Hashes berechnet werden, wenn der Host es kann (Durchlauf auf der Gegenseite, Abschnitt 3.6): Agent `WalkHashed2` (`agent_proto/os/shared/ext_ops.rs:241-251`, `md5_file` auf dem Agent-Host), Share-Host (`share/os/shared/host_hash_walk.rs:110-149`, auf dem Host) — dann nur Hash-Ziffern über das Netz.

Zwischenspeicher: Im Abgleich gibt es keinen dauerhaften Hash-Speicher auf der Netzseite. `sync_backend` entfernt die Caching-Schicht (`vfs/os/shared/sync_roots.rs:6-8`, Aufruf in `app/core/preview_core.rs:51`). Die Wiederverwendung betrifft nur `Full` und den letzten Basisstand (`snapshot_dir.rs:227-233`). `content_digest` (SHA-256) wird im Abgleich nicht aufgerufen (Suche über `native/src/bisync`: kein Treffer).

### 3.5 Felder je Eintrag über die Verbindung (Drahtformat)

- Share, Listing (`FsMeta`, JSON): `name`, `is_dir`, `is_symlink`, `size`, `mtime_ms`, `btime_ms`, `hidden`, `system`, `id`, `special` (`share/core/wire.rs:245-259`).
- Share, `SyncChildPath`: Anfrage `parent` und `literal_name`; Antwort `path` (`peer_literal_paths.rs:22-29`). Host: zwei Zugriffsprüfungen je Anfrage (`peer_literal_paths.rs:34-54`, `access.resolve` `:41` und `:45`, `check_read` `:40` und `:52`).
- Share, Hash-Walk: `FsHashEntry` mit `rel`, `is_dir`, `size`, `mtime_ms`, `digest` (`share/core/hash_walk_wire.rs:34`; Hostseite `host_hash_walk.rs:221-226`, `:258-264`). Ein Eintrag ist eine JSON-Zeile im Stapel; Abschluss `Done` mit Summen (`peer_hash_walk.rs:106-113`).
- Agent, tolerantes Listing: `WireMeta` mit `name`, `is_dir`, `is_symlink`, `special`, `size`, `mtime_ms`, `content_md5` (`agent_proto/os/shared/ext_ops.rs:109-120`). Kein `hidden`-Feld (Befund V1 in Abschnitt 11.3).
- Agent, Hash-Walk v2: `HashEntry` mit `rel`, `is_dir`, `size`, `mtime_ms`, `md5` (`ext_ops.rs:263-271`), je Eintrag ein eigenes Frame (`:253-257`, `:230`).
- Agent, Baum-Walk: `WireNode` mit `name`, `size`, `is_dir`, `children` (`agent_proto/os/shared/fs.rs:282-288`, `:306-319`); keine Änderungszeit.

Beim Client wird `hidden` für Agent-Einträge immer `false` gesetzt (`native/src/agent/core/metadata.rs:13`). Für SFTP, WebDAV und FTP ist `hidden` der Punktname (`sftp/core/metadata.rs:28`, `webdav/core/metadata.rs:62`, `ftp/core/metadata.rs:85`), für SMB das Attribut oder der Punktname (`smb/core/listing.rs:109`), für Google Drive `false` (`gdrive/core/metadata.rs:85`).

### 3.6 Wahl oder Abwahl des Durchlaufs auf der Gegenseite (Sync-Snapshot)

Bedingungen in `walk_snapshot_via_agent` (`snapshot_agent.rs:25-31`), jede führt zu `Ok(None)`:

1. `backend.is_local()` — lokale Seiten laufen immer im Ordner-für-Ordner-Pfad.
2. `!opts.cross_mounts` — Begründung im Code (`:20-22`): das alte Protokoll hat kein Feld für die Mount-Grenze; nur die Metadatenwalk kann sie einhalten.
3. `!filter.include_hidden` — Begründung (`:23-24`): Hash-Einträge enthalten keine Versteckt-Attribute; nur die Metadatenwalk kann versteckte Dateien ohne Punkt ausschließen. Im Standard ist `include_hidden = true` (`syncjobs/core/types.rs:199`), diese Bedingung greift also nur bei abgewähltem Job.
4. `backend.has_duplicate_file_names()` — Mehrfachnamen (ID-adressierte Anbieter).

Weitere Abbrüche:

- Hash-Walk v2 (`crate::vfs::hash_walk`, `vfs/core/extension_calls.rs:252-263`): `Ok(false)` ohne Erweiterung; Agent: ohne `features.extensions` (Etikett `ext-v1`), `agent/core/extensions.rs:322-330`; Share: ohne `hash_walk_v1` (`share/core/peer_hash_walk.rs:26-28`); SFTP und lokal: keine Implementierung (`vfs/core/extensions.rs:246-259`, Standard `Ok(false)`).
- Hash-Walk legacy (`walk_hashed`, `snapshot_agent.rs:157-158`): nur Agent mit Etikett `sync-links-v1` (`agent/core/search.rs:98-100`, `agent_proto/core/features.rs:123`); Share-Client ohne Überschreibung (`vfs/core/core.rs:478-492`, Standard `Ok(false)`).
- `HASH_WALK_LINK_BOUNDARY` (legacy Server bei jedem Link): `Ok(None)` (`snapshot_agent.rs:146-147`, `:176-178`); Quelle `agent_proto/os/shared/hash.rs:26-41`.
- Ohne Einträge und ohne Unterstützung des Legacy-Walks: `Ok(None)` (`snapshot_agent.rs:171`); sind bereits Einträge gesendet, ist es ein Fehler (`:153-155`).
- Budget (`snapshot_agent.rs:62-65`): Abbruch mit Fehler, nicht `None`.

Konsequenz: Mit dem Standardjob (`cross_mounts = false`) wird der Durchlauf auf der Gegenseite nie gewählt. Die Begründung ist die Mount-Grenze, die für entfernte Seiten **auf der Clientseite ohnehin nicht geprüft wird** (`snapshot_policy.rs:98`: nur `backend.is_local()`). `cross_mounts` wirkt auf entfernten Seiten damit nur über diese Auswahl.

### 3.7 Lokale Mount-Fakten je Verzeichnis (Sync-Snapshot)

- `snapshot_policy::protected` (`snapshot_policy.rs:66-112`): Bei lokalen Verzeichnissen (`is_dir && is_local`, Zeile 98) wird `mount_omission` aufgerufen (`:99-109`).
- `mount_omission` (`apply_path_context.rs:99-129`): Ohne aktiven Kontext (nur `apply_reporting.rs:125-139` aktiviert ihn) rechnet es jedes Mal neu (`:106-108`).
- Die Berechnung (`snapshot_policy.rs:104-107`): `local_mount_boundary(path)` (Quelle `vfs/os/shared/local_extensions.rs`, nicht gelesen, O2), dann `snapshot_mounts::missing` (`snapshot_mounts.rs:36-88`).
- `snapshot_mounts::missing` (`:36-88`): bildet einen Schlüssel aus Kennung des Backends und Wurzel per Zeichenkettenformat und Hash (`:50`), liest je Aufruf die Metadaten von `nested-mounts.json` (`stamp()`, `:19-25`, Aufruf `:55`), sperrt den globalen Mutex `KNOWN` (`:51-54`) und sucht im Verzeichnisbaum (`:61-73`). Wenn ein Verzeichnis eingehängt ist und noch nicht gespeichert, schreibt die Funktion die Datei unter einer Sperre (`:74-86`).
- Der Wurzelpfad wird nicht geprüft (`snapshot_mounts.rs:47-49`).
- Pro lokales Verzeichnis entstehen damit: eine Metadatenabfrage (`stamp`), ein `local_mount_boundary`-Aufruf, eine Zeichenkettenbildung mit Hash und eine Mutex-Sperre.
- Volumenwurzeln: `volume_root` ruft für Wurzeln `local_mount_boundary` auf (`snapshot_policy.rs:114-123`).

### 3.8 Gemischte Paare (lokal gegen entfernt)

- Beide Seiten werden von `read_pair` gleichzeitig gelesen (`snapshot_pair.rs:150-160`); jede Seite folgt ihren eigenen Regeln.
- Lokale Seite: kein Flow (`snapshot.rs:213`), kein Agent-Durchlauf (`snapshot_agent.rs:25`, `backend.is_local()`), Mount-Prüfung je Verzeichnis (`snapshot_policy.rs:98-110`, Abschnitt 3.7), Parallelität bis zur Zahl der Kerne (`snapshot.rs:236`).
- Entfernte Seite: Flow-Permits je Ordner (`snapshot_walk.rs:298`), Agent- oder Share-Durchlauf nach Abschnitt 3.6, keine Mount-Prüfung auf der Clientseite (`snapshot_policy.rs:98`).
- Prüfschlüssel je Seite mit `hash_mode(this, other)` (`snapshot_pair.rs:88`): Im Modus `MtimeSize` erhält die lokale Seite `Full`, wenn die entfernte Seite einen Inhalts-Hash liefert (Google Drive, WebDAV mit Checksummen; Abschnitt 3.4), sonst `None`. Die entfernte Seite erhält `NativeOnly` nur bei eigenem Inhalts-Hash. Im Modus `Checksum` erhalten beide Seiten `FullFresh` (`snapshot_hash.rs:81`); die lokale Seite liest dann lokal (`hash_file`), die entfernte per Download (Abschnitt 3.4).
- Barriere: Die Einlesedauer ist das Maximum beider Seiten, weil auf beide gewartet wird (`snapshot_pair.rs:166-172`).
- Frühes Kopieren (nur Lauf): Die Anzahl der Kopierthreads folgt dem breiteren Endpunkt (`early_copy.rs:169-176`). Kopien zur entfernten Seite schreiben dort (Schreibkosten, Abschnitt 4.3).

## 4. Weg (a): Sync-Job-Lauf

### 4.1 Aufrufkette

- Auslöser:
  - Dienst: `job.rs:96` (`attempt`) → `run_settings` (`job.rs:201`, Standard `ScanDepth::Incremental` über `RunSettings::for_job`, `bisync/core/run_types.rs:259-264`; für Ursachen `Verify`, `Startup`, `CatchUp` auf `VerifySources`, `job.rs:256-261`) → `run_with` (`job.rs:217`).
  - GUI: `sync_core.rs:145` (`launch_bisync`) → Thread → `run_with` (`sync_core.rs:228`).
  - GUI manuell: `sync_manual_run.rs:180` (`execute`) → `run_with` (`:307`).
  - Mobil: `mobile/os/shared/domains/sync_run.rs:281`.
- `run_with` → `run_at` (`bisync/os/shared/orchestration.rs:147-171`, Protokoll je Lauf) → `run_at_inner` (`:173-381`):
  - Wurzelprüfung (`:190`), Paarsperre mit Wartezeit (`:193-206`), Identitätsmigration (`:207`), Replikaerkennung (`:218`), Wiederanlauf einer Ersetzung (nur bei `!dry_run`, `:230-243`), Basislinie und Journal laden (`:244-305`), Zwischenstand verdichten (`:306-314`), Versionen (`:316-321`).
  - Entscheidung: `settings.depth != Full && !replicas.changed && !due` → `try_incremental_run` (`:344-345`), sonst oder bei `None` → `run_full_locked` (`:346-348`).
  - Ende: Versionsabschluss und Aufräumen (`:352-379`).
- `run_full_locked` (`bisync/os/shared/orchestration_full.rs:16-110`): Schreibweisen laden (`:20`), Index-Wiederanlauf (`:39`), Quellcursor (`:42-43`, bei Zwei-Richtungs-Jobs `None`), frühes Kopieren (`:49-58`), `read_pair_copying` (`:59-66`), Leer-Seiten-Sperre (`:94-108`), Planung und Anwendung (ab `:110`), Checkpoint, Index-Bootstrap (`:473-477`, bei Zwei-Richtungs-Jobs ohne Wirkung, `incremental_index_commit.rs:31-44`).

### 4.2 Netzanfragen für Sitzungsaufwand (je Lauf, nicht je Ordner)

- `replica::identify` (`bisync/os/shared/replica.rs:40-…`) fragt je Seite die Markerdatei ab (`stat`, `:130`), liest sie gegebenenfalls (`open_read_regular`, `:139`) und fragt die Wurzel ab (`stat`, `:187`). Ein Schreibaufruf (`open_write_copy_stage_sized`, `:206`) steht im Code; seine Bedingung ist nicht gelesen (O10). Je Seite sind damit mindestens zwei Anfragen (Markerdatei, Wurzel) belegt (Teile von `replica.rs` nicht gelesen, O10).
- `backend_identity_migration::migrate` (`orchestration.rs:207`, `backend_identity_migration.rs:12-…`, Inhalt nicht gelesen, O10).
- `validate_sync_roots` (`orchestration.rs:190`, Quelle `vfs/os/shared/sync_roots.rs:10-20`, Remote-Zweig nicht gelesen).

### 4.3 Gegenseiten im Lauf

Die Einlese-Kosten sind die des Abschnitts 3.2 bis 3.4. Zusätzlich im Lauf:

- Frühes Kopieren (`early_copy.rs`) übertragt neue Dateien während des Einlesens. Diese Kopien kosten je Datei Netzzugriffe (Lesen und Schreiben); sie gehören zur Laufzeit des Laufs, nicht zur Vorschau.
- Planung über Basis, Seiten und Ausschlüsse (Abschnitt 3.1, Punkt 7; Quelle `bisync/core/plan*.rs`, nicht gelesen, O15).
- Anwendung, Checkpoints und Journal (`checkpoint_run.rs`, `checkpoint_journal.rs`; nur Kopfkommentare gelesen, O15).

## 5. Weg (b): „Vergleichen“

- Auslöser: `menus_sync_jobs.rs:134-137` (Knopf, nur wenn keine Vorschau läuft) → `compare_id` (`:250`) → `CompareKind::Preview` für Nicht-Tresor-Jobs (`app/core/vault_models_sync.rs:249-252`) → `launch_preview` (`app/core/preview_core.rs:8`).
- `launch_preview`: Thread `preview` (`:22`); Job frisch laden (`:24-29`, `current_preview_job` `:236`), `checked_opts(true)` (`:34`, Probelauf), Endpunkte auflösen (`:36`, `:38`), `sync_backend` (`:51`, Caching-Schicht entfernt), `bisync::preview_with` (`:52`).
- `preview_with` (`bisync/os/shared/preview.rs:56-200`):
  - Wurzeln prüfen (`:68`), Paarsperre (`:70`), Identitätsmigration (`:72`), Replikaerkennung ohne Schreiben (`:73`), Basislinie und Journal (`:82-96`), Einlesen (`:97`).
  - **Immer** `read_pair` (`:97`); kein Folge-Lauf, kein frühes Kopieren (`early_copy.rs:140`: `dry_run` schaltet aus).
  - Danach Schutz für ausstehende Abgleiche (`:100`), Planung (`:118`), Dublettenplan nur bei Spiegel (`:143-190`), Ergebnis (`:191-…`).
- Netzanfragen: wie Abschnitt 3.2 bis 3.4 je Seite, plus die Replika- und Identitätszugriffe aus Abschnitt 4.2. Der Aufruf von `identify` übergibt `false` (`preview.rs:73`); die Bedeutung dieses Parameters ist nicht gelesen.
- Unterschied zu (a): kein frühes Kopieren (`early_copy.rs:140`) und keine Anwendung. In den gelesenen Zeilen 1-200 von `preview.rs` gibt es keinen Schreibaufruf auf Basis oder Journal; die Zeilen 201-292 sind nicht gelesen.

## 6. Weg (c): rekursive Ansicht (Ctrl+R)

### 6.1 Entfernte Seite

- Auslöser: `frame_keyboard.rs:309-311` (`KbdAct::ToggleRecursive`: `self.recursive = !self.recursive`, dann `self.rescan()`).
- `rescan` (`app/core/scanning.rs:158-168`): bei entfernter Verbindung `invalidate_cache()` (`:163-165`), dann `start_scan_with(p, false, true)` (`:167`).
- `start_scan_with` (`scanning.rs:18-119`): der Umfang kommt aus `scan_scope` (`app/core/filter_scope.rs:27-46`): ohne einschränkenden Filter `max_depth = None` (unbegrenzt), mit einschränkendem Filter (`filter_prunes`) eine Aufbewahrung mit `retention.max_depth()` (`:35-46`).
- Entfernt: `rscan::start_scan_backend` (`scanning.rs:96-103`) → Thread `rscan-driver` (`rscan/os/shared/rscan.rs:52-81`) → `run` (`:83-161`):
  - Wurzel: `backend.stat(root)` (`:95`), eine Anfrage.
  - Parallel, wenn `max_depth.is_none() && backend.parallelism() > 1` (`:135-138`), sonst seriell mit `list_dir` je Ordner (`:140-159`).
- `parallel::run` (`rscan/os/shared/parallel.rs:8-64`): je Breitenebene (`frontier`, `:21-23`) Blöcke der Länge `parallelism` (`:24`); je Block ein rayon-Pool mit `parallelism` Threads, jeder Ordner des Blocks: `backend.list_dir(path)` (`:36-41`); Ergebnisse werden danach sequentiell im Treiber verarbeitet (`:45-60`); die Sammlung des Blocks ist die Barriere (`:29-44`); die Ebene ist nach allen Blöcken fertig (`:62`).
- Netzanfragen: je Ordner **ein** `list_dir`; je Eintrag keine Anfrage (Metadaten sind im Listing); eine Stat-Anfrage für die Wurzel. Keine Stat je Ordner, keine `sync_child_path`, keine Flow-Permits (Suche in `rscan/**`: keine Treffer für `flow`, `permit`, `acquire`).
- Gegenseite:
  - Share: `list_dir` ruft `peer_list_batch::list` (`share/core/backend.rs:172-180`) und liefert einen Fehler, wenn der Host Auslassungen meldet (`:174-177`). Die rekursive Ansicht bricht damit bei einem nicht listbaren Eintrag im Ordner ab (beobachtetes Verhalten, nicht ausgeführt).
  - Agent: `Frame::ListDir` (`agent/core/backend.rs:158-164`), Server `list_local` (`agent_proto/os/shared/fs.rs:78-105`, je Eintrag `symlink_metadata`).
  - SFTP: Backend-`list_dir` (nicht gelesen, O3).
- Ausgabe: `ScanMessage` in Stapeln zu 256 (`rscan.rs:27`), Fortschritt alle 150 ms (`:28`); GUI: `drain_scan_channel_with_denied` nimmt bis zu 64 Nachrichten je Aufruf (`app/os/shared/platform_helpers.rs:28-45`), hängt Einträge an (`:40`); während des Laufs `view_dirty` (`app/core/drains_connect.rs:33-35`), am Ende einmal `recompute_view()` (`:30`).

### 6.2 Lokale Seite

- Gleiche Auslösung und gleiches `scan_scope`; ohne Entfernung ruft `start_scan_with` den lokalen Scanner: `start_scan(root, ScanOpts{ follow_symlinks: false, max_depth, retention }, tx)` (`app/core/scanning.rs:104-112`).
- `scanner::start_scan` (`native/src/scanner/os/shared.rs:61-63`) → Thread `scan-driver` (`:75-107`) → `run_scan` (`:250-487`):
  - Wurzel: eine Metadatenabfrage (`:315-325`); das Wurzel-Element wird einmal gesendet (`:345`).
  - Fortschrittsthread: alle 150 ms (`:400-422`).
  - Pool mit `min(available_parallelism, 4)` Threads und 8 MiB Stack (`:433-441`); `walk_parallel` (`:449`).
- `walk_parallel` (`scanner/os/walk.rs:28-75`): Ordner der Ebene per `par_iter` (`:40`); rekursiv je Unterordner (`:61-65`).
- `list_directory` (`walk.rs:83-310`): je Ordner `source_media::GuardedIterator` (`:89-96`), dann `local_access::read_directory` (`:93`); bei einem vorgegebenen Verzeichnishandle (`scanner.confined`) `retry::read_confined` (`:91-92`).
- `local_access::read_directory` (Linux, `native/src/local_access/os/linux_os.rs:23-60`): liest alle Einträge, sortiert nach Inode (`:28-30`), ruft je Eintrag `lstat` für Größe und Zeiten auf (`:36-60`). Versteckt wird über den Punkt am Anfang bestimmt (Auszug `linux_os.rs:1-60`).
- `local_access::read_directory` (Windows, `local_access/os/windows/directory.rs:102-130`): ein Verzeichnishandle und `GetFileInformationByHandleEx` mit Puffern zu 8192 Wörtern (`:28-29`, `:177-181`); Größe und Zeiten aus dem Puffer. In einem Rückfallzweig ruft `next_entry` je Eintrag `entry.metadata()` (`:262`). Wann dieser Rückfall greift, ist nicht gelesen (O13).
- Je Eintrag in `walk.rs`: Apptrash- und Name-Prüfung (`:143-149`), Pfadbildung (`:150`), Größe und Zeiten (`:189-197`), `FileEntry` (`:221-235`), Zähler (`:236-246`), Budget je Eintrag (`shared.rs:140-157` über `claim_entry`), Stapelung (`walk.rs:347-367`), Unterordner gesammelt und ab 128 vorab verarbeitet (`:290-302`).
- Barrieren: keine Ebenenbarriere (verschachtelte `par_iter`-Aufrufe); der Stapel-Teilbaum ab 128 Unterordnern ist ein Synchronisationspunkt (`:290-302`).

## 7. Weg (d): Speicheranalyse

### 7.1 Entfernte Seite

- Auslöser (GUI): `app/core/analytics_core.rs:236-252` (`start_analytics_scan_remote`) → Worker mit `invalidate_cache()` (`:486`) → `analytics::scan_remote` (`:488`). Dienst-Variante: `daemon/os/shared/ipc_analysis.rs:183`.
- `scan_remote` (`native/src/analytics/os/shared/remote.rs:10-53`) in dieser Reihenfolge:
  1. `backend.scan_storage` (`:16`). Ergebnis `Some` → fertig (`:18`).
  2. Ohne `supports_walk_tree` → `scan_backend` (`:23-25`).
  3. Sonst `walk_tree` (`:36`); bei `Some` → Ergebnis (`:37-47`), bei `None` → `scan_backend` (`:49`).
- Share-Peer:
  - `scan_storage` (`share/core/backend.rs:195-201` → `share/core/peer_storage_analysis.rs:10-49`): `legacy_capabilities` (`:16-19`); sonst eine Fähigkeitsanfrage **ohne** Zwischenspeicher (`:20-24`), dann bei `storage_analysis_v2` eine Analyseanfrage mit `node_budget`, `compress = analysis_deflate_v1` und `request_id` bei `analysis_reattach_v1` (`:37-48`).
  - Host: `analysis_tasks.rs:207-237`: `storage_analysis_host::scan_bounded` (`:214`), Ergebnis mit Deflate bei Fähigkeit (`:223-236`, `:226-229`).
  - Ohne `storage_analysis_v2`: Legacy-Pfad über `walk_tree` (`peer_storage_analysis.rs:31-34`, `share/core/backend.rs:207-213` → `peer_storage_snapshot::walk_peer`, nicht gelesen, O8).
- Agent: `scan_storage_budgeted` (`agent/core/analysis.rs:12-55`):
  - Innere `scan_storage` zuerst (`:17`); bei Dienst-Funktion (`features().service`) das Ergebnis des Dienstes (`:18-20`).
  - Sonst `walk_tree_with_budget` mit Empfängerbudget (`:26`, `:36`) → Frame `WalkTree` (`walk.rs:25`), Serverfortschritt alle 200 ms (`server.rs:96`), Client-Abfrage alle 250 ms (`walk.rs:28`), `Tree` als Abschluss (`walk.rs:39`).
  - Speicherüberlauf (`OutOfMemory`): Rückfall auf `scan_backend` mit Hinweis (`analysis.rs:40-47`).
- Agent-Server `WalkTree`: `handle_walk_tree` (`agent_proto/core/server.rs:81-115`): Fortschrittsthread (`:82-96`), `walk_dir_counted` (`agent_proto/os/shared/fs.rs:223-231`). `walk_dir_counted_inner` (`:233-320`): je Ordner `require_plain_directory` (`:246`) und `read_dir` (`:250`); je Eintrag `symlink_metadata` (`:258`); Unterordner parallel über den globalen rayon-Pool, wenn mehr als einer (`:290-297`); Ergebnis: `WireNode` ohne Änderungszeit (`:282-288`).
- SFTP ohne Agent: `supports_walk_tree` ist im Standard `false` (`vfs/core/core.rs:406-408`) und wird von SFTP nicht überschrieben (Suche über `native/src`: Überschreibungen nur in Share, Agent, Cache und Test); also `scan_backend` je Ordner (Abschnitt 7.2).

### 7.2 Remote-Pfad `scan_backend`

- `analytics_backend.rs:20-59`: Threadzahl `parallelism().clamp(1, 16)` (`:34`), eigener Pool mit 64 MiB Stack (`:35-43`), `Walker::dir` ab Wurzel (`:53`).
- Je Ordner `list` (`:176-248`): **ein** `list_dir_tolerant` (`:181`); Symlinks, Sonderdateien und Papierkorbnamen werden übersprungen (`:190-195`); Pseudoordner (`/proc`-artig) übersprungen (`:228-230`); Dateien nur als Größe (`:196-210`); Ordner mit Budgetprüfung (`:232-240`).
- Rekursion: parallel über `into_par_iter`, wenn mehr als ein Unterordner (`:131-143`, Kommentar Zeile 1-7: „no barrier per level“, „each folder assembled as soon as its subfolders are done“).
- Dateien je Ordner: nur die größten `MAX_RETAINED_FILES_PER_DIRECTORY` (Wert nicht gelesen, O18), der Rest als Aggregat (`:252-284`).
- Netzanfragen: je Ordner ein Listing; je Eintrag keine. **Keine** Stat-Anfrage je Ordner, keine Flow-Permits.

### 7.3 Lokale Seite

- `analytics::scan` (`analytics.rs:63-65`) → `scan_with_guard` (`:70`) → `scan_in` (`:104-111`) → `scan_in_with` (`:142-220`).
- Threads: `local_scan_threads()` (`:157`, `:225-231`) = 2 auf dem Desktop (`analytics/os/mod.rs:27-29`), Android `min(Kerne, 4)` (`:22-24`). Pool nur, wenn `parallel_scan_allowed()` (`analytics.rs:169`): Linux immer (`local_access/os/linux_os.rs:11-13`), Windows nur ohne Thread-Token (`local_access/os/windows/privilege.rs:29-37`).
- `scan_dir_with` (`analytics.rs:260-333`): ruft `read_directory` (`:312`, Linux und Windows wie in 6.2) und `scan_entries_with` (`analytics_walk.rs:16-110`).
- Je Eintrag: Überspringen von Links und Sonderdateien (`analytics_walk.rs:55-57`), Namens-Ausschluss (`:60-66`), Pseudoordner (`:82-84`), für Ordner `budget.claim` (`:85-86`) und Stapel zu 256 Unterordnern (`:88-104`), für Dateien eine Heap-Auswahl (`:105-110`). Keine Schutzbereichsprüfung je Eintrag (`analytics_walk.rs` enthält kein `protected`; die Schutzbereiche setzt `analytics.rs:158-159` je Wurzel).
- Netzanfragen: nicht zutreffend; Systemaufrufe je Ordner: ein `read_dir` (Linux mit Inode-Sortierung), je Eintrag ein `lstat` (`linux_os.rs:36-60`).
- Keine Mount-Prüfung (Befund 10).

## 8. Folge-Läufe: Basis, Index, Änderungsquelle

### 8.1 Entscheidung

- `run_at_inner` (`orchestration.rs:344-349`): inkrementell nur bei `settings.depth != Full`, unveränderter Replikaidentität (`replicas.changed` false) und ohne fällige Vollprüfung (`due`, `:337-343`: `verify_target_secs > 0` und letzte Vollprüfung älter als dieser Wert; fehlende Historie gilt als fällig).
- `try_incremental_run` (`incremental.rs:79-…`) verlangt zusätzlich:
  - kein Planregelwerk (`:83-85`), keine ausstehende Zusammenführung (`:86-90`), kein Probelauf, Historie vorhanden, kein Dirty-Marker des Index (`:93-98`);
  - einseitiger Spiegel: `mirror_source` liefert nur bei `DeletePolicy::Mirror` ohne `move_files` und bei `AtoB` oder `BtoA` eine Quelle (`:56-77`).
- Zwei-Richtungs-Standardjob: `mirror_source` = `None` (Richtung `Both`, `incremental.rs:75`) → `try_incremental_run` gibt `None` zurück (`:102`) → Vollprüfung. **Standardjobs laufen immer vollständig.**
- Probelauf (`dry_run`): `try_incremental_run` gibt `None` zurück (`incremental.rs:93-97`), also `run_full_locked`; die Vorschau ruft `try_incremental_run` nie auf (Abschnitt 5).

### 8.2 Basis und Index (einseitiger Spiegel)

- Basis: gespeicherte Signaturen je Pfad (`Baseline`), geladen aus dem Journal (`orchestration.rs:299-305`, `checkpoint_journal.rs`).
- Index: `SyncStateStore` (Paarsatz, Einträge, Cursor) über `open_store` (`incremental.rs:136-137`, `incremental_index_commit.rs:315-…`); Prüfung auf Übereinstimmung von Basis und Index (`incremental.rs:145-150`), Paarsatz und Wurzelkennung (`:138-142`).
- Bootstrap: `bootstrap_run` schreibt den Index nur bei einseitigem Spiegel (`incremental_index_commit.rs:42-43`); sonst kein Index.

### 8.3 Änderungsquelle

- `source.supports_changes()` (`incremental.rs:157`) ist wahr für Google Drive (`gdrive/core/backend.rs:369-371`), für den Share-Host-Wrapper nur über das Innere (`share/core/fs_guard_backend.rs:307-309`) und für den Cache-Wrapper (`vfs/core/cache.rs:376-378`). Standard `false` (`vfs/core/core.rs:373-375`). Keine Überschreibung für lokal, SFTP, Share-Client oder Agent (Suche über `native/src`).
- Mit Änderungsquelle und `ScanDepth::Incremental`: `changes_from_backend` ruft `changes_since(root, cursor)` (`incremental_collect.rs:132`); ohne Cursor oder bei Zurücksetzen → `Rebuild` (`:126-128`, `:140-142`).
- Ohne Änderungsquelle (Standard für lokal, Share-Peer, Agent, SFTP, SMB, FTP und WebDAV; keiner dieser Clients überschreibt `supports_changes`, Suche über `native/src`): `changes_from_source_walk_scoped` (`incremental.rs:169-178`) liest die **gesamte Quelle** erneut über `walk_snapshot_with_options` (`incremental_collect.rs:271-281`, mit derselben Auswahl wie in 3.6). Nur das Ziel wird gezielt geprüft (`target_touched_drifted_spelled`, `incremental.rs:250-261`).
- Unvollständige Quelle → `Rebuild` (`incremental_collect.rs:289-291`).

### 8.4 Echtzeit-Auslöser

- Watch-Ereignisse (`native/src/watch/**`, Konsum in `daemon/os/shared/realtime.rs`, `daemon/os/shared/job_triggers.rs`): der Kopf von `realtime.rs` beschreibt je Echtzeit-Job einen Empfänger mit dauerhaften Weckmeldungen; die Importe nennen `PendingTrigger` und `RunCause` (`realtime.rs:1-10`). Eine Änderungsliste an den Abgleich ist dort nicht erkennbar; der Rest der Datei ist nicht gelesen (O17). Der Lauf bleibt ein normaler Lauf (Abschnitt 8.1).
- Die lokale Watch-Baumerfassung (`watch/os/linux_os/inotify_tree.rs:246-293`) durchläuft den Baum seriell: je Ordner ein `inotify_add_watch` (`add_dir`, `:295-303`) und ein `read_dir` (`:265`); je Eintrag `metadata` (`:271-281`, mit Gerätevergleich bei `cross_mounts = false`).

### 8.5 Zusammenfassung Folge-Läufe

| Fall | Basis | Index | Änderungsquelle | Lesen der Gegenseite |
|---|---|---|---|---|
| Zwei-Richtungs-Standardjob | genutzt für Planung | keiner | keine | beide Seiten vollständig, jeder Lauf und jede Vorschau |
| einseitiger Spiegel, Quelle mit Änderungsquelle (Drive), `ScanDepth::Incremental` | genutzt | genutzt | `changes_since` | nur Änderungen; Ziel gezielt |
| einseitiger Spiegel, Quelle ohne Änderungsquelle | genutzt | genutzt | keine | Quelle vollständig, Ziel gezielt |
| Vollprüfung (`due`, `Verify`, `Startup`, `CatchUp`) | genutzt | Neuaufbau | je nach Fall | voll |

## 9. Gegenseiten-Implementierung

### 9.1 Share-Host (Direct Share)

- Anfragen werden über `server_fs.rs` verteilt; `StorageAnalysis` → `storage_analysis_server::serve` (`share/core/server_fs.rs:172-173`).
- Hash-Walk (`host_hash_walk.rs:370-408`):
  - Einstieg `run` (`:370`); Einzeldatei-Zweig für `single_file` (`:377-379`); sonst `visit` (`:396`), dann Abschluss `Done` (`:398-407`).
  - `visit` (`:409-461`): Dynamische Container nur mit Liste (`:411-433`); sonst Auflösung (`:435`); lokale Wurzel: Handle-begrenztes Durchlaufen (`local`, `:439-457`); Wurzel auf einem anderen Backend: `remote` (`:459`).
  - `local` (`:150-276`): serielle Tiefensuche (Rekursion `:228-234`, Tiefe 2048 `:159`); je Ordner `read_directory` (`:166`); je Eintrag: versteckt-Ausschluss nach `host_list::hidden` (`:179-183`, nur Papierkorb, Motor- und Staging-Namen, `host_list.rs:14-18`), Unrepräsentierbares (`:189-199`), Links (`:200-203`), Sonder (`:204-211`), Ordner (`:212-235`), Datei (`:238-273`).
  - Datei: `open_regular_child` (`:240`) **auch ohne Hash** (`algo = None`), dann `metadata` (`:241`), `digest` (`:248`; ohne Algorithmus `None`, `:111-113`), erneut `metadata` (`:249`).
  - `remote` (`:277-367`): je Ordner `list_dir_tolerant` des Backends (`:292`), `sync_child_path` je Eintrag (`:332`), Datei per `open_read_regular` und Hash über den Puffer (`:349-350`).
- Stapelung: `add` sammelt Einträge bis `LIST_BATCH_MAX_BYTES` mit Faktor 6 (`:44-72`), `flush` sendet `Batch` (`:93-109`).
- Algorithmus: MD5 oder SHA-256 nach Anfrage (`:131-148`), nur Ziffern auf der Leitung.
- Fähigkeiten: `hash_walk_v1`, `hash_file_v1`, `list_batches_v1`, `literal_children_v1`, `analysis_deflate_v1`, `analysis_reattach_v1`, `storage_analysis_v2` (`share/core/wire_capabilities.rs:13-59`, `:86`, `:112`; `peer_storage_analysis.rs:27`).
- Speicheranalyse-Host (`share/os/shared/storage_analysis_host.rs`):
  - Budget mit Knotengrenze (`:39`).
  - Wurzeln je Ast nacheinander (`:76-82`).
  - Lokale Wurzel: `analytics::scan_confined_with` (`:139-140`) — also derselbe lokale Analysepfad wie in 7.3, mit 2 Threads auf dem Desktop.
  - Entfernte Wurzel: `analytics::scan_remote` (`:177`).
  - Datenträgerangabe: `host_volume_usage` (`:154`).

### 9.2 se-agent und `agent_proto`

- Der Agent ist eine eigenständige Binärdatei (`se-agent/src/main.rs:14-60`); die Protokollschicht ist per Pfad eingebunden (`se-agent/src/main.rs:5-6`), `serve` in `agent_proto/core/server.rs:119` (Signatur). Die Startoptionen `--serve` und `--serve-root` (`main.rs:25-43`).
- Zuordnung der Anfragen (`agent_proto/core/server.rs`): `ListDir` `:224`, `Stat` `:225`, `TryExists` `:226`, `WalkTree` `:227`, `WalkHashed` `:311`, `ListTolerant` `:369`, `WalkHashed2` `:370-374`.
- `ListTolerant` (`agent_proto/os/shared/ext_ops.rs:45-91`): je Ordner `read_dir` (`:54`), je Eintrag `symlink_metadata` (`:78`), Namen ohne gültiges UTF-8 als Auslassung (`:59-74`); Teile zu 1 MiB (`:20`, `:82-85`, `:93-107`), Abschluss `End` (`:90`).
- `WalkHashed2` (`ext_ops.rs:125-261`): **serielle** Stapel-Tiefensuche (`:150-151`, Schleife `:151-259`); je Ordner `symlink_metadata` (`:157`) und `read_dir` (`:169`); je Eintrag `symlink_metadata` (`:213`), Link als `HashOmitted` (`:220-224`), Ordner als `HashEntry` mit Änderungszeit (`:225-232`), Sonderdatei als Auslassung (`:234-237`), Datei unter `min_bytes` übersprungen (`:238-240`), sonst MD5 über `md5_file` (`:241-252`, Lesen der ganzen Datei, `:321-327`, `read_regular_exact` `:331-…`) und `HashEntry` (`:253-257`). Eine Nachricht je Eintrag.
- `WalkHashed` (legacy, `agent_proto/os/shared/hash.rs:10-96`): serielle Tiefensuche; ein Link bricht den **ganzen** Durchlauf mit `HASH_WALK_LINK_BOUNDARY` ab (`:26-41`); Sonderdateien werden **still übersprungen** (kein Zweig für sie, `:74-92`); Dateien mit MD5, wenn angefordert (`:75-80`).
- `WalkTree` (`fs.rs:223-320` und `server.rs:81-115`): siehe 7.1; Größen und Namen, keine Änderungszeit, keine Hashes.
- Fähigkeiten (Etiketts im Hello-Text, `agent_proto/core/features.rs:6-26`, `:90-150`): `sync-links-v1` (`link_aware_hash`, `:6`, `:123`), `ext-v1` (`extensions`, `:14`, `:125`), `credit-v1`, `stage-v1`, `batch-v1`, `slots-N`; `lz4-v1`, `range-v1`, `folder-reservation-v1`, `file-identity-v1`, `capacity-v1` (`features.rs:6-22`). Protokollversion 12 (`types.rs:3`).
- Client-Gates: `list_dir_tolerant` nur mit `extensions`, sonst `list_dir` (`agent/core/extensions.rs:175-179`); `hash_walk` nur mit `extensions` (`:322-330`); `walk_hashed` nur mit `link_aware_hash` (`agent/core/search.rs:98-100`).
- Drahtformat: binäre Frames (`agent_proto/core/frame_encode.rs:60-79` für `Dir`, `Stat`, `WalkTree`, `Tree`; `:161` für `HashEntry`; `agent_proto/core/frame_ops.rs:33-71` für `DirPart`). Der LZ4-Zweig ist nur der letzte Fallback für Frames ohne eigene Kodierung (`frame_encode.rs:213-222`). Kompression der Listen-, Baum- und Hash-Frames: nein (aus den Kodierern abgeleitet).
- Gegenstellen älterer Version: ohne `ext-v1` keine `ListTolerant` und kein `WalkHashed2` (Client-Fallback auf `ListDir` und Legacy); ohne `sync-links-v1` kein `WalkHashed` (Client-Fallback auf Ordnerlauf).

### 9.3 Dienst-Vermittlung (`daemon`)

- `daemon/os/shared/backend_hash.rs:1-8` (Kommentar): Für ein vom Dienst bedientes Backend streamt ein Share-Host mit `hash_walk_v1` seine Einträge; jedes Backend ohne diese Fähigkeit wird im Dienst selbst mit tolerantem Listing durchlaufen (Links, Sonder- und unlesbare Einträge als Auslassungen).
- Je Eintrag ein Frame (`item_frame`, `:50-61`).
- `ipc_backend_extensions.rs` (Kopf): Operationen der Share-Identität hinter einem Agent-Backend; Einzelheiten nicht gelesen (O12).

## 10. Weitere Operationen mit Baumerfassung oder Baumvergleich

### 10.1 Einfügen: Vorschau der Ordnerkonflikte („Zusammenführen“)

- Start: `app/core/folder_conflict_ui.rs:20-33` (`Admission::new` → `FolderPreviewScan::start_with_wake`), mobil `mobile/os/shared/fs_edit.rs:133` (synchron) und `:162` (Start).
- Thread `folder-conflict-preview` (`transfer/os/shared/engine/selection.rs:283-361`):
  1. `folder_conflicts` (`selection.rs:14-99`): je ausgewählter Wurzel `root_kind` (lokal `lstat` `:104-110`; entfernt `stat` `:111`), dann `target.stat(destination)` (`:57`), bei belegter Zielwurzel Namen „(2)“ bis `REMOTE_UNIQUE_ATTEMPTS` mit je einer `stat`-Anfrage bis zum freien Namen (`:67-88`). Die Wurzeln werden nicht rekursiv verglichen.
  2. Melden (`:336`), dann je Ordner `count_folder_conflict` (`:337-353`).
  3. `count_folder_conflict` (`:114-224`): **serieller** Stapel über den Quellbaum (`:127-131`, `stack.pop()`); je Ordner `list_dir_tolerant(source)` (`:135-137`); je Eintrag Namensprüfung (`:156-163`), Budget je Knoten (`:166-172`), Ausschlüsse für Links, Sonderdateien und Papierkorb (`:173-177`), Filter (`:178-194`); je **Datei** `target.stat(destination)` (`:199-201`) → Zählung „kollidiert“ oder „neu“ (`:202-208`). Bei Fehler: Auslassung (`:209-212`).
  4. Fortschritt je Ordner und alle 150 ms (`:214-217`, Konstante `engine/mod.rs:74`).
  5. Danach Abfrageschleife alle 100 ms, bis der Vorgang endet (`:355-360`).
- Netzkosten: je Quellordner ein Listing, je Quelldatei eine Zielanfrage (lokal ein Systemaufruf, entfernt eine Rundreise). Serielle Verarbeitung, keine Parallelität.

### 10.2 Einfügen: Discovery des Quellbaums

- `transfer/os/shared/engine/discovery.rs:16-24` (`discover`), `walk_roots` (`:26-57`): Der Walker `walk` (`transfer/os/shared/walk*.rs`, nicht gelesen, O5) listet den **Quellbaum** über `LocalLister` oder `BackendLister` (`:35-43`); Ausschlüsse nur für Kopien im gleichen Namensraum (`:63-74`).
- Ereignisse je Datei und Ordner: `on_event` (`:104-141`) fügt `Work::Dir` und `Work::File` in die Warteschlange (`:109-125`); Gegenstück Namens-Export (`:146-165`).
- Explizite Paare (`feed_pairs`, `:167-195`): Größe und Zeit nur bei Bedarf über `inspect` (`:198-216`, ein Permit `acquire_meta` `:199-201`).
- Der Ziel­baum wird in der Discovery nicht verglichen; der Vergleich je Datei liegt in den Workern (nicht gelesen, O15).

### 10.3 Duplikatsuche (Reclaim)

- Einstieg `analytics/os/shared/reclaim/finder.rs:162-` (`find_duplicates`, Schutzbereiche `ProtectedAreas::for_walk`, `:172`), Kopf (`:1-8`): Dateien ab Mindestgröße sind Kandidaten; eine Größe braucht zwei Dateien.
- Walker `finder_walk.rs` (`Walk::run` `:91`, `visit` `:105`, Kandidaten `offer` `:259`, Stapel 128 Dateien `:22`), mit Pool (`finder.rs:269-270`).
- Vergleich (`finder_compare.rs:41-71`): Sortierung nach Größe (`:46-51`); nur Größen mit zwei oder mehr Dateien (`:52`); Fingerabdruck mit den ersten und letzten 64 KiB (`:58-68`, `SAMPLE_BYTES` in `finder.rs:36`), parallel über `compare.each`; danach vollständiger Vergleich (nicht gelesen).
- Netzkosten: nur in der Lokalversion; die Walker-Netzvariante liegt außerhalb.

### 10.4 Ordnergrößen

- Eigener Ordnergrößen-Durchlauf im Lesebereich: keiner gefunden (Suche `folder_size|dir_size|Ordnergröße|directory_size|recursive_size` in `native/src`: kein Treffer für einen Durchlauf). Ordnergrößen entstehen aus dem Analysebaum (`app/core/analytics_core.rs:294`, Hinweis „Ordnergrößen“).

### 10.5 Ordnerindex (Suche nach Ordnernamen)

- `folder_index/mod.rs:1-10` (Kommentar): speicherinterner Index aller Ordner unter den gewählten Wurzeln für unscharfe Ordnersuche; Speicher als Textdatei im App-Datenordner.
- `folder_index/os/walk.rs:18-154` (`build_index_excluding`): Stapel-Tiefensuche (`:29`, `:49`); je Ordner Medienwächter mit Hintergrundpriorität (`:68-79`) und `read_dir` (`:77`); je Eintrag Medienwächter (`:98`) und `symlink_metadata` (`:99`); nur Ordner werden aufgenommen; Fortschritt alle 200 ms (`:10`, `:66`).
- Verbraucher: Optik-Index (`app/os/shared/optical_index.rs`), mobiler Index (`mobile/os/shared/index.rs`), Laufwerksinventar (`app/os/shared/drive_inventory.rs`), Pfadprüfung (`app/os/shared/watchers.rs:69`).
- Mount-Grenze: keine Prüfung (Befund 10).

### 10.6 Kopiersammlung für rekursive Auswahl

- `scanner/os/collect.rs:40-60` (`collect_recursive`, `collect_recursive_with_access`) mit Grenzen `:8-11`. Aufrufer: `vfs/os/windows/verbatim.rs:227` und `app/os/shared/clipboard_lifecycle.rs:283`. Die Schleife (`collect`, `:60-320`) ist nicht vollständig gelesen (O16); gelesen sind Kopf, Grenzen und die Aufrufe.

### 10.7 Watch-Baumerfassung

- Siehe 8.4 (`watch/os/linux_os/inotify_tree.rs:246-303`).
- Die Windows-Variante (`watch/os/windows/backend.rs`, `rdcw_io.rs`) ist nicht gelesen (O17).

## 11. Ursachenliste der Laufzeitdifferenz

Alle Ursachen sind Code-Ableitungen. Es gibt keine Messungen. Reihenfolge nach Wirkung auf den entfernten Fall, dann lokal.

### 11.1 Entfernt (Sync-Vergleich und Lauf gegenüber Speicheranalyse)

- **R1: Rundreise je Eintrag bei Share-Gegenseiten.** Der Sync-Snapshot ruft `sync_child_path` je Eintrag auf (`snapshot_dir.rs:120`) und der Share-Client sendet je Eintrag `SyncChildPath` (`peer_literal_paths.rs:17-25`), sequentiell im Worker, ohne Stapelung und ohne Listing-Permit (`snapshot_walk.rs:340`). Die Analyse und die rekursive Ansicht haben diesen Schritt nicht (`analytics_backend.rs:176-240`, `rscan/**`).
- **R2: Standard-Job wählt den Durchlauf auf der Gegenseite nie.** `cross_mounts = false` (`syncjobs/core/types.rs:227`) bricht `walk_snapshot_via_agent` ab (`snapshot_agent.rs:26`). Die Analyse nutzt auf Share- und Agent-Seiten eine Anfrage für den ganzen Baum (`remote.rs:16-36`), unabhängig von Mount-Optionen; auf SFTP-Seiten ist es je Ordner ein Listing (Abschnitt 7.2).
- **R3: Je Ordner im Sync eine Stat-Anfrage mehr als in der Analyse.** `sync_stat` je Ordner (`snapshot_walk.rs:76`) zusätzlich zum Listing (`:91`); die Analyse fragt auf SFTP-Seiten je Ordner nur das Listing (`analytics_backend.rs:181`) und auf Agent- und Share-Seiten gar keine Ordneranfrage (`agent_proto/os/shared/fs.rs:223-320`, `remote.rs:16-36`).
- **R4: Doppelter Durchlauf.** Der Sync liest beide Seiten vollständig (`snapshot_pair.rs:150-173`), die Analyse liest eine Seite. Vorschau und Lauf teilen sich dieselbe Einleseroutine (Abschnitt 5).
- **R5: Flow-Grenze begrenzt die Parallelität des Syncs.** Remote-Grenze `flow.limit + 1` (`snapshot_walk.rs:178-186`), Startgrenze 8 bei Share, SFTP, SMB, WebDAV; 4 bei FTP und Drive (`flow_registry.rs:113-131`), Überlast halbiert (`docs/TRANSFER_ENGINE.md:76`). Die Analyse nutzt `parallelism()` bis 16 ohne Flow (`analytics_backend.rs:34`). Die tatsächliche Grenze hängt vom gelernten Stand ab (nicht gemessen).
- **R6: Barriere je Block in der rekursiven Ansicht.** `parallel.rs:24-44` wartet je Block der Länge `parallelism` auf die langsamste Auflistung; der Analysepfad hat keine Blockgrenze (`analytics_backend.rs:1-7`, `:131-143`).
- **R7: Sync-spezifische Arbeit über das Einlesen hinaus.** Replikaerkennung mit Dateizugriffen je Seite (`replica.rs:130`, `:139`, `:187`), Basislinie und Journal laden (`orchestration.rs:299-305`), Dublettenplan (`snapshot_pair.rs:186-195`), Planung (`bisync/core/plan*.rs`, nicht gelesen). Die Analyse hat keine dieser Schritte.
- **R8: Prüfschlüssel in `Checksum` ohne Durchlauf auf der Gegenseite.** Jede Datei: vier Metadatenanfragen, eine Öffnung und die vollständige Übertragung (`snapshot_dir.rs:250-280`; `apply_guard.rs:139-150`). Die Analyse liest nie Inhalte. Der Standardmodus `MtimeSize` vermeidet das (`snapshot_hash.rs:78-88`).
- **R9: Gegenseiten-Hash-Walk ist seriell (Agent, Share-Host).** `WalkHashed2` (`agent_proto/os/shared/ext_ops.rs:150-259`) und `host_hash_walk.rs:150-276` laufen seriell; sie werden im Standard nicht gewählt (R2).
- **R10: Viele Ordner mit wenigen Einträgen.** Der Sync stellt je Ordner Stat und Listing (R3); die Analyse stellt je Ordner nur das Listing (`analytics_backend.rs:181`) bzw. gar keine Ordneranfrage (Agent-Baum, `agent_proto/os/shared/fs.rs:223-320`). Je kleiner die Ordner, desto größer der Anteil der Ordneranfragen an der Laufzeit (Richtung, nicht gemessen).
- **R11: Die Analyse gibt nur das Ergebnis zurück.** Der Share-Host und der Agent berechnen den Baum auf dem Gerät, das die Daten hat; der Ergebnisstrom ist ein einzelner Baum (`analysis_tasks.rs:214-236`, `agent/core/walk.rs:25-39`).
- **R12: Die Analyse auf dem Share-Host läuft nur mit 2 Threads (Desktop).** `analytics/os/mod.rs:27-29`. Das ist kein Vorteil der Analyse gegenüber dem Sync, sondern ein Hinweis, dass Parallelität nicht die Ursache der Differenz ist.

### 11.2 Lokal

- **L1: Mount-Fakten je Ordner im Sync.** `snapshot_policy.rs:98-110`, `snapshot_mounts.rs:19-25,50-55`: je Ordner eine Metadatenabfrage, eine Zeichenketten-Hash-Bildung und eine Sperre des globalen Mutex. Die Analyse und der Scanner führen keine Prüfung aus (Befund 10).
- **L2: Schutzprüfung je Eintrag im Sync.** `own_path` liest vier Umgebungsvariablen und ein Hostmerkmal und sperrt den Cache je Eintrag (`snapshot_policy.rs:28-64,73-75`). Die Analyse prüft je Eintrag nur Name, Link und Sonderdatei (`analytics_walk.rs:55-66`).
- **L3: Zwei Vollwalks.** Der Sync liest beide Seiten vollständig (`snapshot_pair.rs:150-173`), die Analyse eine Seite (Befund R4).
- **L4: Verarbeitung je Eintrag im Sync.** Pfadtext und Literal-Name je Eintrag (`snapshot_dir.rs:119-120`, `literal_child` `:336-342`), Glob-Prüfung (`:141-142`), `Sig`, Einfügen in den Baum und Sortierung je Ordner (`snapshot_walk.rs:361-388`). Die Analyse zählt Größen (`analytics_walk.rs:199-205`).
- **L5: Lokale rekursive Ansicht sendet jeden Eintrag an die GUI.** `FileEntry` mit drei `Arc<str>` (`scanner/os/walk.rs:221-225`), Stapel an den Kanal (`:347-367`), Anhängen in der GUI (`platform_helpers.rs:40`), Neuaufbau am Ende (`drains_connect.rs:30`) und `view_dirty` während des Laufs (`:33-35`). Die Analyse hält nur Größen und begrenzte Dateilisten (`analytics_backend.rs:252-284`, `analytics_walk.rs:105-110`).
- **L6: Pro Ordner ein Medienwächter im Scanner und im Ordnerindex.** `scanner/os/walk.rs:89-96`, `folder_index/os/walk.rs:68-79`; die Analyse hat keinen (`analytics/**` ohne `source_media`). Kosten nicht gelesen (O6).
- **L7: Lokaler Listing-Pfad des Syncs.** Der Sync nutzt `LocalBackend::list_dir_tolerant` (`vfs/os/shared/local_extensions.rs:57`, nicht gelesen, O1); ob er Metadaten je Eintrag separat abfragt, ist offen. Die Analyse und der Scanner nutzen `local_access::read_directory` (Linux: eine Sortierung und ein `lstat` je Eintrag).
- **L8: Die Parallelität des Syncs ist höher, die Arbeit je Eintrag größer.** Sync-Walk: bis zu `parallelism` je Seite (Kerne, `core.rs:261-263`); Scanner: bis 4; Analyse: 2 (Desktop). Die Analyse gewinnt durch den geringeren Aufwand je Eintrag, nicht durch Parallelität (Befund 13).

### 11.3 Beobachtete Verhaltensunterschiede (kein Laufzeitbezug, für den Erhalt zu beachten)

- **V1: Versteckte Dateien beim Agenten.** `hidden` ist für Agent-Einträge immer `false` (`agent/core/metadata.rs:13`); `snapshot_dir.rs:141` prüft nur dieses Feld. Mit `include_hidden = false` werden Punktdateien auf Agent-Seiten im Ordner-für-Ordner-Pfad nicht ausgeschlossen, auf SFTP-Seiten schon (`sftp/core/metadata.rs:28`). Die Auswirkung ist nicht ausgeführt.
- **V2: Mount-Grenze entfernt.** Entfernte Seiten prüfen keine Mount-Grenze auf der Clientseite (`snapshot_policy.rs:98`), die Gegenseiten-Durchläufe (Agent, Share) ebenfalls nicht (`fs.rs:233-320`, `ext_ops.rs:125-261`, `host_hash_walk.rs:150-276`).
- **V3: Legacy-Hash-Walk bricht bei Links ab, Sonderdateien werden still übergangen.** `agent_proto/os/shared/hash.rs:26-41`, `:74-92`.
- **V4: Rekursive Ansicht entfernt bricht bei nicht listbaren Einträgen ab.** `share/core/backend.rs:174-177`.
- **V5: Mount-Grenze lokal nur im Sync.** Der Scanner und die Analyse zeigen Verzeichnisse hinter Mounts (Befund 10); der Sync trennt sie (`snapshot_policy.rs:98-110`).

## 12. Prüfung von `plan.md`, Abschnitt B

- **B1 (Speicheranalyse über Share und Agent): bestätigt**, mit Ergänzungen. Der Agent nutzt `WalkTree` in einer Anfrage (`agent/core/analysis.rs:36`); nur bei Speichermangel fällt er auf `scan_backend` zurück (`analysis.rs:40-47`). SFTP ohne Agent nutzt `scan_backend` (Abschnitt 7.1). Der Share-Host analysiert lokale Wurzeln mit dem lokalen Analysescanner (`storage_analysis_host.rs:139-140`).
- **B2 (Sync-Snapshot entfernt, Ausweichen auf Ordner-für-Ordner): bestätigt** (`snapshot.rs:197`, `snapshot_agent.rs:25-31`). Korrektur: `!filter.include_hidden` ist nur bei abgewähltem Job wirksam; der Standard ist `include_hidden = true` (`syncjobs/core/types.rs:199`). Der Standard-Job wird nur durch `cross_mounts = false` ausgeschlossen. Der Ausweichpfad ist kein reiner Rückfall: `walk_snapshot_observed` läuft danach sofort mit `walk_tree` weiter (`snapshot.rs:209-254`).
- **B3 (rekursive Ansicht entfernt, Blöcke je Ebene): bestätigt** (`parallel.rs:24-60`). Korrektur: Die Blockgröße ist `parallelism` begrenzt auf 2 bis 16 (`:9`); die Auflistung ist `list_dir` (nicht tolerant), je Ordner eine Stat-Anfrage nur für die Wurzel; keine Flow-Permits.
- **B4 (offene Punkte): beantwortet** in Abschnitt 3.4 (Hash-Ermittlung), Abschnitt 9.1 bis 9.2 (Durchlauf auf der Gegenseite), Abschnitt 5 (Vergleichen), Abschnitt 8 (Folge-Läufe) und Abschnitt 10.1 (Zusammenführen-Vorschau). Das Einfügen mit Zusammenführen selbst (Dateiregeln) ist nicht beschrieben (O15).

## 13. Gelesene Dateien (Lesetiefe)

Zugehörigkeit: „L“ = im ursprünglichen Lesebereich des Auftrags; „E“ = im erweiterten Lesebereich der Kurskorrektur; „K“ = außerhalb beider Listen, gelesen, weil eine Aufrufkette dorthin führt (Begründung in der Klammer). Tiefe: „voll“ = alle Zeilen; „Teil“ = genannte Zeilenbereiche; „Suche“ = nur Trefferzeilen per `rg`.

| Datei | Zugehörigkeit | Tiefe |
|---|---|---|
| `docs/plaene/2026-10-31-sync-abgleich-schnell/plan.md` | L | voll |
| `docs/ARCHITEKTUR.md` | L | Teil (82–150) |
| `docs/TRANSFER_ENGINE.md` | L | Teil (50, 60–78, 172–175) |
| `docs/GOTCHAS.md` | L | Suche (ohne Befund) |
| `native/src/app/core/menus_sync_jobs.rs` | L | Teil (134–137, 240–272) |
| `native/src/syncjobs/core/types.rs` | L | Teil (195–240, 300–346) |
| `native/src/syncjobs/os/shared/persistence_codec.rs`, `editor.rs`, `vault_job.rs` | L | Suche (cross_mounts/include_hidden) |
| `native/src/bisync/mod.rs` | L | Suche (Zeile 266) |
| `native/src/bisync/core/types.rs` | L | Teil (195–215, 326–400) |
| `native/src/bisync/core/run_types.rs` | L | Teil (203–213, 259–264) |
| `native/src/bisync/core/limits.rs` | L | voll |
| `native/src/bisync/core/compare.rs`, `plan.rs`, `plan_decide.rs`, `plan_dirs.rs`, `plan_filter.rs`, `plan_index.rs`, `plan_pair.rs`, `plan_rules.rs`, `plan_types.rs`, `keys.rs`, `omissions.rs`, `paths.rs`, `guards.rs`, `completion.rs`, `baseline_records.rs`, `duplicate_types.rs`, `snapshot_types.rs`, `sync_relative_path.rs` | L | nicht gelesen (O15) |
| `native/src/bisync/os/shared/snapshot.rs` | L | voll |
| `native/src/bisync/os/shared/snapshot_agent.rs` | L | voll |
| `native/src/bisync/os/shared/snapshot_walk.rs` | L | voll |
| `native/src/bisync/os/shared/snapshot_dir.rs` | L | voll |
| `native/src/bisync/os/shared/snapshot_pair.rs` | L | voll |
| `native/src/bisync/os/shared/snapshot_hash.rs` | L | voll |
| `native/src/bisync/os/shared/snapshot_policy.rs` | L | voll |
| `native/src/bisync/os/shared/snapshot_mounts.rs` | L | voll |
| `native/src/bisync/os/shared/snapshot_duplicates.rs` | L | Teil (12–42) |
| `native/src/bisync/os/shared/snapshot_walk_tests.rs`, `snapshot_pair_tests.rs` | L | nicht gelesen (Tests) |
| `native/src/bisync/os/shared/preview.rs` | L | Teil (1–200 von 292) |
| `native/src/bisync/os/shared/orchestration.rs` | L | Teil (85–135, 147–392) |
| `native/src/bisync/os/shared/orchestration_full.rs` | L | Teil (16–110, 455–479) |
| `native/src/bisync/os/shared/orchestration_plan.rs` | L | nicht gelesen (O15) |
| `native/src/bisync/os/shared/incremental.rs` | L | Teil (1–300) |
| `native/src/bisync/os/shared/incremental_collect.rs` | L | Teil (91–160, 229–300) |
| `native/src/bisync/os/shared/incremental_changes.rs`, `incremental_index_commit.rs` (außer 31–47) | L | nicht gelesen (O15) |
| `native/src/bisync/os/shared/engine_change_feed.rs` | L | Kopf |
| `native/src/bisync/os/shared/early_copy.rs` | L | Teil (126–200, Suche) |
| `native/src/bisync/os/shared/early_copy_record.rs` | L | Teil (17–43) |
| `native/src/bisync/os/shared/sync_overload.rs` | L | voll |
| `native/src/bisync/os/shared/apply_boundary.rs` | L | Teil (1–80) |
| `native/src/bisync/os/shared/transfer_stream.rs` | L | voll |
| `native/src/bisync/os/shared/checkpoint_run.rs`, `checkpoint_journal.rs` | L | Kopf |
| `native/src/bisync/os/shared/apply_path_context.rs` | L | Teil (99–129), Suche (60–66) |
| `native/src/bisync/os/shared/apply_guard.rs` | L | Teil (75–115, 139–150) |
| `native/src/bisync/os/shared/apply_reporting.rs` | L | Suche (125, 139, 278, 285) |
| `native/src/bisync/os/shared/replica.rs` | L | Suche (40, 130, 139, 187, 206) |
| `native/src/bisync/os/shared/backend_identity_migration.rs` | L | Suche (12–15) |
| `native/src/bisync/os/shared/tests/*`, `*_tests.rs` | L | nicht gelesen (Tests) |
| `native/src/vfs/mod.rs` | L | Teil (130–170) |
| `native/src/vfs/core/extension_types.rs` | L | Teil (180–318) |
| `native/src/vfs/core/extensions.rs` | L | Teil (95–137, 236–262) |
| `native/src/vfs/core/extension_calls.rs` | L | Teil (1–140, 240–337) |
| `native/src/vfs/core/cache_extensions.rs` | L | Teil (54–84), Suche (139–146) |
| `native/src/share/core/peer_hash_walk.rs` | L | voll |
| `native/src/share/core/hash_walk_wire.rs` | L | Kopf |
| `native/src/share/core/peer_content_digest.rs` | L | nicht gelesen |
| `native/src/share/core/peer_extensions.rs` | L | Teil (60–72, 138–162), Suche |
| `native/src/share/core/wire_capabilities.rs` | L | Teil (13–59, 86, 112) |
| `native/src/share/core/fs_request.rs` | L | Teil (53–101) |
| `native/src/share/core/fs_response.rs` | L | Suche |
| `native/src/share/core/host_requests.rs` | L | nicht gelesen |
| `native/src/share/core/list_batch_wire.rs` | L | Teil (1–30) |
| `native/src/share/os/shared/host_hash_walk.rs` | L | voll |
| `native/src/share/os/shared/host_hash_file.rs` | L | nicht gelesen |
| `native/src/share/os/shared/host_list.rs` | L | Teil (14–18), Suche |
| `native/src/share/os/shared/host_stream.rs` | L | nicht gelesen |
| `native/src/share/os/shared/analysis_tasks.rs` | L | Teil (196–240) |
| `native/src/share/core/peer_storage_analysis.rs` | K (Aufruf aus `share/core/backend.rs:195-201`) | Teil (1–60, 60–173 nicht) |
| `native/src/share/core/peer_list_batch.rs` | K (Aufruf aus `peer_extensions.rs:68-69`) | Teil (10–32) |
| `native/src/share/core/peer_literal_paths.rs` | K (Aufruf aus `peer_extensions.rs:65-66`) | Teil (1–60) |
| `native/src/share/core/peer_stream.rs` | K (Aufruf aus `peer_hash_walk.rs:26`) | Teil (14–30) |
| `native/src/share/core/peer_request.rs`, `peer_request_policy.rs`, `io_deadline.rs`, `backend_capabilities.rs` | K | Teil (wie in 9.1 und 3.3 genannt) |
| `native/src/share/core/backend.rs` | K (Aufruf aus `vfs`-Schnittstelle) | Teil (172–213) |
| `native/src/share/core/fs_guard_backend.rs` | K | Suche (304–310, 427–445) |
| `native/src/share/os/shared/storage_analysis_host.rs` | K (Aufruf aus `analysis_tasks.rs:214`) | Teil (11–82, 139–177, Suche 154) |
| `native/src/agent/core/backend.rs` | L | Teil (18–30, 84–112, 119–133, 143–200, 256–268) |
| `native/src/agent/core/extensions.rs` | L | Teil (160–200, 310–360) |
| `native/src/agent/core/analysis.rs` | L | voll |
| `native/src/agent/core/walk.rs` | L | voll |
| `native/src/agent/core/search.rs` | L | Teil (80–130) |
| `native/src/agent/core/metadata.rs` | L | voll |
| `native/src/agent_proto/core/server.rs` | L | Teil (60–120, 204–232), Suche (311, 369–374) |
| `native/src/agent_proto/os/shared/fs.rs` | L | Teil (1–40, 78–105, 223–330) |
| `native/src/agent_proto/os/shared/ext_ops.rs` | L | Teil (1–285, 321–331) |
| `native/src/agent_proto/os/shared/hash.rs` | L | Teil (1–100) |
| `native/src/agent_proto/core/features.rs` | L | Teil (1–150) |
| `native/src/agent_proto/core/types.rs` | L | Teil (1–20) |
| `native/src/agent_proto/core/frame_encode.rs` | L | Teil (60–79, 161, 205–226), Suche |
| `native/src/agent_proto/core/frame_ops.rs`, `credit.rs`, `codec.rs` | L | Suche |
| `se-agent/src/main.rs`, `vfs.rs` | L | voll |
| `se-agent/src/transfer.rs` | L | Teil (1–30) |
| `native/src/daemon/os/shared/backend_hash.rs` | L | Teil (1–62) |
| `native/src/daemon/os/shared/ipc_backend_extensions.rs` | L | Kopf |
| `native/src/daemon/os/shared/job.rs` | L | Teil (96, 196–268) |
| `native/src/daemon/os/shared/ipc_analysis.rs` | L | Kopf, Suche (183) |
| `native/src/daemon/os/shared/realtime.rs` | L | Kopf |
| `native/src/daemon/os/shared/job_triggers.rs` | L | Suche |
| `native/src/analytics/os/shared/analytics.rs` | E | Teil (41–70, 94–111, 142–240, 260–340) |
| `native/src/analytics/os/shared/analytics_backend.rs` | E | voll |
| `native/src/analytics/os/shared/remote.rs` | E | voll |
| `native/src/analytics/os/shared/analytics_walk.rs` | E | Teil (1–110), Suche |
| `native/src/analytics/os/mod.rs` | E | Teil (1–36) |
| `native/src/analytics/os/shared/reclaim/finder.rs` | E | Teil (1–12, 162–175), Suche |
| `native/src/analytics/os/shared/reclaim/finder_compare.rs` | E | Teil (41–71) |
| `native/src/analytics/os/shared/reclaim/finder_walk.rs`, `backend_agent.rs`, `budget.rs` | E | Suche / Kopf |
| `native/src/analytics/os/linux_trash.rs` | E | Suche |
| `native/src/scanner/mod.rs` | E | voll |
| `native/src/scanner/os/shared.rs` | E | voll |
| `native/src/scanner/os/walk.rs` | E | Teil (1–100, 140–312, 326–387) |
| `native/src/scanner/os/collect.rs` | E | Teil (8–11, 40–60), Suche |
| `native/src/scanner/os/budget.rs` | E | Suche |
| `native/src/local_access/os/linux_os.rs` | E | Teil (1–60) |
| `native/src/local_access/os/windows/directory.rs` | E | Teil (1–10, 28–29, 77–82, 100–130, 177–181, 251–270), Suche |
| `native/src/local_access/os/windows/privilege.rs` | E | Teil (25–40) |
| `native/src/folder_index/mod.rs` | E | Kopf |
| `native/src/folder_index/os/walk.rs` | E | Teil (1–100) |
| `native/src/watch/os/linux_os/inotify_tree.rs` | E | Teil (24–45, 246–303) |
| `native/src/watch/**` (übrige Dateien) | E | Suche (Verbraucher) |
| `native/src/transfer/os/shared/engine/selection.rs` | E | voll |
| `native/src/transfer/os/shared/engine/discovery.rs` | E | voll |
| `native/src/transfer/os/shared/engine/mod.rs` | E | Suche (74) |
| `native/src/transfer/core/folder_conflict.rs` | E | Suche (Typen 20–59) |
| `native/src/app/core/folder_conflict_ui.rs` | E | Teil (15–34), Suche |
| `native/src/app/core/recursive*.rs` | E | Suche (ohne Treffer für Baumerfassung) |
| `native/src/app/core/scanning.rs` | E | Teil (5–10, 18–120, 158–176) |
| `native/src/app/core/scan*.rs` | E | Suche (ohne weitere Treffer) |
| `native/src/app/core/drains_connect.rs` | K (Auswertung der Scan-Nachrichten) | Teil (13–48) |
| `native/src/app/os/shared/platform_helpers.rs` | K (Auswertung der Scan-Nachrichten) | Teil (11–70) |
| `native/src/app/core/analytics_core.rs` | K (Auslösung der Analyse) | Teil (236–252, 478–492) |
| `native/src/app/core/frame_keyboard.rs` | K (Ctrl+R) | Teil (300–325) |
| `native/src/app/core/filter_scope.rs` | K (Umfang der Ansicht) | Teil (27–46) |
| `native/src/app/core/preview_core.rs` | K (Auslösung der Vorschau) | Teil (1–75), Suche (236) |
| `native/src/app/core/vault_models_sync.rs` | K (Vergleichsart) | Teil (236–252) |
| `native/src/vfs/os/shared/sync_roots.rs` | K (Sync-Wurzeln) | Teil (6–20) |
| `native/src/vfs/core/core.rs` | K (Standardwerte des Backends) | Teil (261–263, 368–375, 397–416, 475–492) |
| `native/src/vfs/core/backend_defaults.rs` | K | Teil (195–260) |
| `native/src/vfs/core/cache.rs` | K | Suche (Caching-Schicht) |
| `native/src/transfer/os/shared/flow_registry.rs` | K (Startgrenzen) | Teil (1–135), Suche |
| `native/src/transfer/os/shared/flow.rs` | K | Suche (200, 254) |
| `native/src/connect/os/shared/resolution.rs` | K (Endpunkt-Auflösung) | Teil (7–29) |
| `native/src/mobile/os/shared/fs_edit.rs`, `domains/analyze.rs` | K (mobile Aufrufer) | Suche |
| `native/src/sftp/core/extensions.rs`, `gdrive/core/backend.rs`, `gdrive/core/metadata.rs`, `webdav/core/webdav.rs`, `webdav/core/metadata.rs`, `ftp/core/ftp.rs`, `ftp/core/metadata.rs`, `smb/core/listing.rs`, `mount/core/device_tree_backend.rs`, `zipfs/os/shared/zipfs.rs`, `daemon/os/shared/ipc_client.rs`, `vault/os/shared/backend.rs`, `daemon/os/shared/devices_mount.rs` | K | Suche (Überschreibungen von Parallelität, Listung, Hash, Verzeichnisbaum und `hidden`) |
| `native/src/vfs/os/windows/verbatim.rs` | K (Aufrufer von `collect_recursive`) | Suche (227) |
| git: `git show --stat 6e5bd9c9`, `git show --stat 956d498e` | K | Commit-Kopf: `6e5bd9c9` ändert nur `native/src/local_access/os/linux_os.rs`; `956d498e` ändert Kopierpfade und den Flow-Startwert (`transfer/os/shared/flow_registry.rs`) |

## 14. Offene Fragen (exakt)

- **O1**: `native/src/vfs/os/shared/local.rs` und `native/src/vfs/os/shared/local_extensions.rs` (insbesondere `list_dir_tolerant` in Zeile 57, `stat`, Listen-Systemaufrufe je Ordner und je Eintrag). Außerhalb der Leseliste. Ohne sie sind die Systemaufrufe des lokalen Sync-Listings unbekannt.
- **O2**: `local_mount_boundary` (`vfs/os/shared/local_extensions.rs`), Kosten je Aufruf. Außerhalb der Leseliste.
- **O3**: SFTP-Backend (`native/src/sftp/core/**`): Paketzahl je `stat`, je `list_dir_tolerant` (Überschreibung in `sftp/core/extensions.rs:64`) und je `sync_child_path`; Flow-Art. Außerhalb der Leseliste.
- **O4**: Innere Backends des SSH-Agenten (`AgentBackend::from_streams*`, Aufrufer außerhalb): Art des inneren Backends, dessen `sync_child_path`- und `stat`-Anfragen und die Flow-Art (`scheme()` delegiert, `agent/core/backend.rs:143-145`). Annahme: die Flow-Art entspricht SFTP; nicht belegt.
- **O5**: Walker und Lister des Einfügens (`native/src/transfer/os/shared/walk*.rs`, `walk_listers.rs`): Listen je Ordner, parallele Stufen, Ausnahmen. Außerhalb der Leseliste.
- **O6**: `source_media::acquire` und `GuardedIterator` (Kosten je Ordner, Sperren). Außerhalb der Leseliste.
- **O7**: `apptrash::hidden_app_folders_in` (je Eintrag im Sync-Snapshot, `snapshot_policy.rs:77`): Kosten. Außerhalb der Leseliste.
- **O8**: Share-Host: `share/core/peer_storage_snapshot.rs` (Legacy-`walk_peer`), `share/core/storage_snapshot.rs`, `share/core/walk_assembly.rs`. Außerhalb der Leseliste.
- **O9**: Share-Host-Listing (`share/os/shared/host_list.rs`, außer 14–18): Listen je Ordner, Stapelung, Versteckt-Regeln für `ListDirBatch`.
- **O10**: `bisync/os/shared/replica.rs` vollständig (Zeilen 40–206 gelesen nur in Trefferzeilen), `backend_identity_migration.rs` vollständig: Dateizugriffe je Lauf im Detail.
- **O11**: `transfer/os/shared/flow.rs` `acquire_meta` (reservierter Listing-Platz) und das Lernen der Grenze (nicht gelesen).
- **O12**: `connect::resolve_endpoint` und `open_saved_at`: ob die GUI-Vorschau und der GUI-Lauf ein direktes Backend oder ein Dienst-Backend (IPC) verwenden. Teil gelesen (`connect/os/shared/resolution.rs:7-29`).
- **O13**: Windows-Enumerator `local_access/os/windows/directory.rs`: Bedingung für den Rückfallzweig mit `entry.metadata()` je Eintrag (`:251-270`).
- **O14**: `gdrive/core/backend.rs` `provides_content_hash` (Rückgabewert im Auszug nicht vollständig gelesen) und `CachingBackend` (`vfs/core/cache.rs`): ob die rekursive Ansicht das zwischengespeicherte oder das direkte Backend nutzt (`scanning.rs:163-165` leert den Cache).
- **O15**: Planung und Anwendung (`bisync/core/plan*.rs`, `bisync/os/shared/apply*.rs`, Checkpoint-Inhalt), Konfliktregeln beim Zusammenführen (`transfer/core/folder_conflict.rs` nur als Typliste, Worker-Entscheidung nicht gelesen).
- **O16**: `scanner/os/collect.rs` Schleife `collect` (`:60-320`) und Aufrufer `clipboard_lifecycle.rs:283`.
- **O17**: Windows-Watch (`watch/os/windows/*`) und Watch-Verbraucher in `daemon/os/shared/realtime.rs` ab Zeile 150.
- **O18**: `analytics/os/shared/analytics_budget.rs`: Werte von `MAX_RETAINED_FILES_PER_DIRECTORY` und Budget.
- **O19**: `rscan` gegen Share: `list_dir` schlägt bei Auslassungen fehl (`share/core/backend.rs:174-177`), Auswirkung auf die Anzeige der rekursiven Ansicht nicht gelesen (`app/core/**` Auswertung).
- **O20**: Es gibt keine Laufzeitmessung. Alle Ursachen (Abschnitt 11) sind Code-Ableitungen und müssen mit einer Messung auf den Stellen bestätigt werden, an denen die Aufrufe liegen (Anzahl Rundreisen je Ordner, Dauer je `SyncChildPath`, Anteil der Mount-Prüfung, Anteil der GUI-Verarbeitung).
- **O21**: Die Parallelitätsgrenze des Flows zum Messzeitpunkt (gelernter Wert) ist nicht bekannt; die Grenze ist im Abschnitt 2 als Startwert genannt.
