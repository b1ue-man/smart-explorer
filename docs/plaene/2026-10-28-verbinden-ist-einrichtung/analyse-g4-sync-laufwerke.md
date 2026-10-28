# Analyse G4: Sync-Aufträge, se-Laufwerke, Tresore, ZIP

Stand: 2026-10-28. Quellstand: eingefrorene Kopie von `a67d7296` (siehe `analyse-plan.md`).
Methode: Teil 1 des Analyseplans (statisch, Code mit `Datei:Zeile`). Nichts wurde gebaut,
getestet oder ausgeführt. Jede Zeit-, Durchsatz- und Systemaufrufzahl unten ist eine Schätzung
aus dem Code und den Annahmen A1–A10; Teil 2 (Messung) liegt bei G5. Alle Pfade sind relativ zu
`native/src/`, sofern nicht anders angegeben.

Aus `analyse-kopieren.md` übernommen und hier im Code nachgeprüft: Flow-Startwert 2
(`transfer/core/flow_control.rs:33`), `acquire_pair` (`transfer/os/shared/flow.rs:444-474`),
`FolderRegister::ensure` (`transfer/os/shared/engine/folders.rs:193-233`), Kosten von
`ensure_private_dir` (`creds/os/private_storage_unix.rs:15-107`,
`creds/os/private_storage_windows.rs:6-72`).

---

## 0 Annahmen

| Nr | Annahme (alle unbelegt bis zur G5-Messung) |
|---|---|
| A1 | Datenträger NVMe: 2 GB/s lesen, 2 GB/s schreiben; Lesen und Schreiben auf derselben SSD zusammen 2 GB/s. `fsync`/`fdatasync` einer kleinen Datei 1 ms. `syncfs` mit wenig schmutzigen Daten 3 ms, mit 64 MiB schmutzigen Daten 35 ms. |
| A2 | CPU 4 Kerne. MD5 0,6 GB/s je Kern. AES-256-GCM (RustCrypto mit AES-NI und PCLMULQDQ) 1,5 GB/s je Kern. HMAC-SHA256 ohne SHA-Befehlserweiterung 0,5 GB/s. Inflate mit miniz_oxide 0,3 GB/s Ausgabe je Kern, mit zlib-rs 0,6 GB/s. |
| A3 | Linux, warmer Cache: `lstat`/`statx` 1 µs, `open`+`close` 3 µs, Lesen und Parsen von `/proc/self/mountinfo` 30 µs bei 40 Einträgen und 200 µs bei 300 Einträgen (Android, Snap-Systeme), `realpath` über sysfs plus Lesen von `device/type` 15 µs. |
| A4 | Windows: `CreateFile`+`CloseHandle` 40 µs mit Defender-Filter, `GetVolumePathNameW` 15 µs, `FlushFileBuffers` 2 ms, `MoveFileExW` mit `MOVEFILE_WRITE_THROUGH` 1,5 ms, ein `ensure_private_dir` (Handle-Kette plus DACL-Prüfung) 150 µs. |
| A5 | LAN: Roundtrip (RT) einer SFTP- oder Share-Anfrage einschließlich Serverbearbeitung 0,5 ms; Nutzlast 115 MB/s (1 GbE). |
| A6 | WAN bzw. Relay: RT 30 ms, Nutzlast 12 MB/s (100 Mbit/s). |
| A7 | Loopback-IPC zwischen Laufwerk-Host und Daemon (Agent-Frame hin und zurück, Daemon-Thread-Start) 60 µs. |
| A8 | Datensätze: D1 = 5000 × 4 KiB in 50 Ordnern direkt unter der Wurzel (Relativtiefe k = 2), D2 = 1000 × 1 MiB in 10 Ordnern, D3 = 1 × 10 GiB. Ziel leer (Erstlauf). Richtung „beide“ (Standard aus `SyncJob::new`, `syncjobs/core/types.rs:195`). Entfernte Wurzel liegt 3 Ebenen unter `/` (r = 3). |
| A9 | Vergleichswerte von rsync, robocopy, rclone, cp, 7-Zip, Cryptomator Desktop, gocryptfs und cryfs sind Erfahrungswerte bzw. dokumentiertes Verhalten, nicht gemessen. |
| A10 | Linux-Desktop mit ext4; die Optik-Probe findet ein Blockgerät unter `/sys/dev/block` (NVMe), das kein optisches Laufwerk ist. |

---

## 1 Kurzfazit, nach erwartetem Gewinn geordnet

| Nr | Befund | Beleg | Wirkung (Schätzung) |
|---|---|---|---|
| F1 | Sync-Apply prüft pro Datei ~31-mal per lokalem `stat` (jedes mit Optik-Probe über sysfs), liest ~9-mal `/proc/self/mountinfo` für `target_limits` plus ~7-mal für Mount-Grenzen, ruft ~21-mal `app_data_dir()` auf (jeder Aufruf baut den privaten App-Ordner per `openat`-Kette neu auf). | §2.4 | ~1100 Systemaufrufe und ~1,8 ms Kernzeit je 4-KiB-Datei unter Linux, ~6–8 ms unter Windows |
| F2 | Sync zu entfernten Zielen: ~43 aufeinanderfolgende RTs je neuer Datei (SFTP), davon 7 nur für `target_limits` ohne Cache und 8 für ein je Datei wiederholtes `mkdir_all` ab `/`. | §2.4 | LAN 22 ms, WAN 1,3 s je Datei und Worker |
| F3 | Sync-Checkpoint: alle 64 Aktionen oder 2 s `syncfs` des Ziels, `fsync` des Journals und `syncfs` des App-Datenträgers, während die Sperre gehalten wird, die jeder Worker in `gate()` und `completed()` braucht. | §2.5 | alle Worker stehen; D1: ~78 Stillstände à ~12 ms, D2: 16 à ~40 ms |
| F4 | Sync-Datenpfad: immer read/write mit 256-KiB-Puffer und MD5 über jedes Byte, auch wenn kein Verbraucher den Digest braucht; kein `copy_file_range`, kein Reflink, kein `CopyFile2`. | §2.4 Schritt 9, §2.6 | D3 lokal: ~18 s (MD5-gebunden) statt 7–10 s Platte bzw. <0,1 s Reflink |
| F5 | Sync-Versionen: jede ersetzte oder gelöschte lokale Datei wird ganz gelesen (Hash), 2 Manifeste mit je 2 `fsync` plus Verzeichnis-`fsync`, `fsync` der alten Datei, Umbenennen plus Verzeichnis-`fsync`. | §2.7 | ~9 Synchronisierungen je gelöschter Datei; 10-GiB-Ersetzung +18 s Lesen |
| F6 | Laufwerk-Schreiben: `close(2)` (FUSE `flush`) bzw. Dokany-`Cleanup` lädt die ganze Datei synchron hoch; OS-Dateimanager kopieren Datei für Datei. | §3.3 | D1 auf SFTP-Laufwerk ~85–160 s statt ~2–3 s (rclone) |
| F7 | Laufwerk-Lesen: erster Lesezugriff lädt die ganze Datei und ruft `fdatasync` auf den Cache auf, bevor das erste Byte zurückkommt (Schreib-Lese-Laufwerk immer, Nur-Lese-Laufwerk bis 500 MiB). Bereichslesen kostet 3 RTs je Leseaufruf ohne Vorauslesen. | §3.4 | D3: erstes Byte nach ~93 s (LAN) |
| F8 | Laufwerk-Pfadprüfung: jeder Zugriff prüft Wurzel und alle Vorfahren per IPC-`stat` (`verify_backend_ancestors = true`); im Daemon geht jedes `stat` ungecacht zum Backend; SFTP-Laufwerke haben genau eine gleichzeitige Anfrage (`parallelism() = 1`). | §3.2 | (1+d) zusätzliche RTs je Rückruf; SFTP-Laufwerk vollständig seriell |
| F9 | Tresor-Laufwerk: jedes Byte wird dreimal mit AES-GCM bearbeitet (Spool versiegeln, Spool entschlüsseln, für den Tresor verschlüsseln), in zwei aufeinanderfolgenden Phasen. | §4.3 | D3 ~20 s statt ~10 s |
| F10 | Mirror-Jobs: nach jedem fehlerfreien Volllauf liest `bootstrap_run` beide Seiten erneut vollständig und fragt je Eintrag `stat` und `item_id` nacheinander ab. | §2.3 Ende | D1 auf SFTP LAN +5 s, WAN +5 min |
| F11 | ZIP: einfädiges Entpacken, 8-KiB-Kopierpuffer (Desktop, nur Windows), `fsync` je Datei (Android), miniz_oxide; im Produkt gibt es kein ZIP-Erstellen. | §5 | Android D1 10–50 s statt ~2 s |

---

## 2 Sync-Aufträge des Hintergrunddienstes

### 2.1 Einstiege, Prozesse, Threads, Kanäle

Einstiege in dieselbe Engine `crate::bisync::run_with` (`bisync/os/shared/orchestration.rs:91`):
Daemon `daemon/os/shared/job.rs:217`, Desktop `app/os/shared/sync_manual_run.rs:307` und
`app/core/sync_core.rs:228`, Android `mobile/os/shared/domains/sync_run.rs:281`, Tresor-Jobs
`vault/os/shared/sync_job.rs:259`. Der Desktop-Schnellspiegel (`crate::sync`, `sync/os/shared/sync.rs`)
ist kein Worker-Weg; er nutzt dieselbe Kopierfunktion `apply_transaction::copy` über
`apply_mirror.rs` `quick_copy` mit `StageDurability::Now` (Dateisynchronisierung je Datei) und
überlappt Listen und Kopieren (`sync/os/shared/sync_pass.rs:1-11`).

Prozess: der Daemon (Hintergrunddienst). Threads je Lauf, alle kurzlebig unter `std::thread::scope`:

| Thread | Anzahl | Quelle |
|---|---|---|
| Job-Thread des Daemons | 1 | `daemon/os/shared/job.rs:12-94` |
| `sync-read-a`, `sync-read-b` | 2; der Job-Thread pollt alle 50 ms | `bisync/os/shared/snapshot_pair.rs:19, 122-145` |
| `sync-walk` je Seite | lokal ≤ `rayon::current_num_threads()` (`vfs/core/core.rs:261`), entfernt ≤ Flow-Limit + 1 (`snapshot_walk.rs:178-186`); Leerlauf 250 ms, Koordinator 50 ms | `snapshot_walk.rs:23-27, 191` |
| `sync-checkpoint` (Zeitgeber, 2 s) | 1 | `checkpoint_run.rs:19, 156-185` |
| `sync-apply` | ≤ Flow-Limit; Obergrenze `max_transfers` oder `shared_connection_cap` | `apply_pool.rs:329-343`, `apply_reporting.rs:293-299` |
| `sync-md5` | 1 je kopierter Datei > 1 MiB | `stream_hash.rs:12-14, 144-148` |

Kanäle: `stream_hash.rs` `sync_channel(4)` mit 256-KiB-Blöcken, jeder Block per `to_vec()`
kopiert (`stream_hash.rs:117`). Sonst keine Kanäle; die Phasen sind über Rückgabewerte gekoppelt.
Nebenläufigkeit des Apply: Flow-Start 2, Verdopplung je Messfenster (≥ 100 ms) bei +10 % Durchsatz
(`transfer/core/flow_control.rs:33-57`); ein neuer Worker startet nur, wenn kein Worker auf
Erlaubnisse wartet (`apply_pool.rs:329-335`). Der Flow sieht die Bytes einer Aktion erst nach deren
Ende (`apply_pool.rs:439`).

### 2.2 Phasen und Aufrufkette (Erstlauf, Richtung „beide“)

Die Phasen laufen streng nacheinander; in keiner Phase überlappen Lesen und Übertragen.

1. **Start**: `run_for` (`job.rs:22`) schreibt den Laufzustand (`update_job_state` :31), `run_job_hook(Before)` :144,
   `connect::resolve_endpoint` je Seite :152, :168 (lokal `LocalBackend::new`,
   `connect/os/shared/resolution.rs:29`). `run_at` (`orchestration.rs:147`) öffnet das Job-Log,
   `run_at_inner` :173: `validate_sync_roots` :190, `PairLock::acquire_wait` :194,
   `backend_identity_migration::migrate` :207, `replica::identify` :218,
   `replacement_recovery::recover_locked` :231, `Journal::load` :299, `journal.compact` :307
   (nur bei vorhandenem Zustand), `RunVersions::begin` :316.
2. **Volllauf oder inkrementell** :344-349: inkrementell nur für Einweg-Spiegel
   (`incremental.rs:56-77`) mit Historie ohne Dirty-Marker; „beide“ läuft immer vollständig
   (`orchestration_full.rs:17`).
3. **Scan**: `read_pair` (`orchestration_full.rs:45` → `snapshot_pair.rs:58`), je Seite
   `walk_snapshot_with_options` (`snapshot.rs:356`) → `walk_tree` (`snapshot_walk.rs:30`). Der Plan
   beginnt erst nach dem Ende beider Seiten (`snapshot_pair.rs:138-145`).
4. **Plan**: `prepare_spelled` (`orchestration_full.rs:99`), nur CPU; Dedupe-Planung :134-190 nur bei Spiegeln.
5. **Blockprüfung** `blocked` :191: Seite leer, Löschschwellen; vor jeder Änderung (FS3).
6. **Checkpoint-Start**: `CheckpointSink::new` :242 lädt das Journal erneut und schreibt einen leeren
   Frame (`checkpoint_run.rs:65-72` → `fsync` + `syncfs`); `sink.planned` :286 schreibt einen zweiten Frame.
7. **Apply** in `sink.during` :381 → `apply_planned_reporting` (`apply.rs:127`) →
   `apply_reporting::run` (`apply_reporting.rs:98`): Ordneranlage-Durchlauf seriell :124-146
   (nur leere Ordner; befüllte entstehen über `FolderRegister`), Vorprüfung `sync_filesystem` je lokaler
   Seite :161-185 (ein `syncfs` je Seite), Pool `run_actions_stoppable` :308.
8. **Ordnerentfernung** seriell :331.
9. **Abschluss**: `sink.finish` :394 (letzter Flush, `journal.compact` = Baseline und Ordnerliste
   privat atomar schreiben, Journal löschen, `syncfs`), `state_spellings::save` :415, `save_history`
   :427, `bootstrap_run` :450 (nur Spiegel, §2.3 Ende), `versions.finish` und `prune_after_run`
   (`orchestration.rs:353-378`, listet `.se-versions` beider Seiten), `persist` (`job.rs:73`),
   `run_job_hook(After)`.

### 2.3 Scan je Ordner und je Eintrag

Je Ordner (`snapshot_walk.rs:274` `visit` → :74 `list_plain_directory`):
- `vfs::sync_stat(dir)` :76. Lokal `LocalBackend::stat` (`vfs/os/shared/local.rs:194`):
  `optical::metadata_guard` (`vfs/os/shared/local_optical.rs:17`) = `lstat` + `source_media::acquire_current`
  → Linux `probe` (`source_media/os/linux.rs:5`: `lstat`) → `probe_device` :30 (`realpath` von
  `/sys/dev/block/M:m` plus bis zu 3 Leseversuche `device/type`), danach `symlink_metadata` und
  `reported_name`. Entfernt: 1 RT am Browse-Cache vorbei (`vfs/core/cache_extensions.rs:56-58`).
- Für Unterordner `snapshot_policy::protected` :86.
- `list_dir_tolerant` :91 → lokal `list_tolerant` (`local.rs:120`: noch eine Optik-Probe,
  `read_dir`, je Eintrag ein `statx`, `local_access/os/linux_os.rs:23-53`).

Je Eintrag (`snapshot_dir.rs:60` `scan_listing`):
- `snapshot_policy::protected` :141 → `own_path` (`snapshot_policy.rs:6`) → `support_dirs::app_data_dir()`
  (`support_dirs.rs:196-200`) ruft bei **jedem** Aufruf `ensure_private_dir` auf: `symlink_metadata`,
  `exists`, `canonicalize`, `open("/")`, `openat` je Pfadkomponente, `fstat`/`fchmod`
  (`creds/os/private_storage_unix.rs:15-103`) ≈ 15–20 Systemaufrufe je gescanntem Eintrag.
- Für lokale Unterordner zusätzlich `local_mount_boundary` (`vfs/os/linux_os/local_platform.rs:100-116`:
  `canonicalize(parent)` + `mountinfo::read`) und `snapshot_mounts::missing` (`snapshot_mounts.rs:117-141`:
  globale Mutex, `fs::metadata(nested-mounts.json)`; der Pfad entsteht über `sync_data_dir()`, also zwei
  weitere `ensure_private_dir`).
- Hash `signature_hash` :178, :212. Modus aus `hash_mode` (`snapshot_hash.rs:418-427`): Größe+Zeit
  ohne Anbieter-Hash → keiner; lokale Seite gegenüber Drive/Nextcloud → `Full` (jede neue oder geänderte
  lokale Datei wird schon beim Scan gelesen, und beim Kopieren ein zweites Mal); Prüfsummenmodus →
  `FullFresh` (jede Datei beider Seiten, entfernt per Download).
- `content_hash` :248: `capture` (stat), `hash_file` (`snapshot_hash.rs:370`: stat, `open_read_regular`,
  64-KiB-Puffer, MD5, stat), `revalidate` (stat) → 4 stat + 1 open je gehashter Datei, im Listing-Thread,
  innerhalb eines Ordners nacheinander.

Mirror-Index (nur Einweg-Spiegel): `bootstrap_run` (`incremental_index_commit.rs:31`) →
`engine_change_feed::bootstrap` (`engine_change_feed.rs:189-260`) läuft nach dem Apply erneut
`walk_snapshot_with_options` über Seite A und danach Seite B und ruft je Datei und Ordner
`backend.stat` und `backend.item_id` nacheinander auf (2 × (Dateien + Ordner) × 2 Anfragen).

### 2.4 Eine Kopieraktion `CopyAtoB` (neue Datei, Tiefe k = 2), Schritt für Schritt

Abkürzungen: **S** = ein lokales `stat` über `LocalBackend::stat` (≈ 15 Systemaufrufe, 20 µs unter Linux
nach A3/A10; Windows: `GetVolumePathNameW` mit 64-KiB-Puffer `source_media/os/windows.rs:23-50` plus
`symlink_metadata`, ≈ 60 µs nach A4). **M** = ein `target_limits` (Linux `filesystem_profile`
= `resolve_existing`/`realpath` + Lesen und Parsen von `/proc/self/mountinfo`,
`vfs/os/linux_os/local_platform.rs:118-139`, ≈ 8 Aufrufe, 40 µs; Windows `volume_facts` = `CreateFile`
+ `GetVolumeInformationByHandleW`, `vfs/os/windows/local_platform.rs:176-193`, ≈ 50 µs).
**P** = ein `own_path` mit `ensure_private_dir` (≈ 18 Aufrufe, 20 µs Linux; ≈ 150 µs Windows nach A4).
**B** = eine Mount-Grenzprüfung je Ordnerkomponente (`canonicalize` + `mountinfo` + `fs::metadata`
unter globaler Mutex, ≈ 45 µs). `guard` = `apply_boundary::guard` (`apply_boundary.rs:244-297`):
`sync_stat(root)` :253, `own_path(root)` :262, `target()` → M :271/:305, je Komponente `sync_stat` und
`snapshot_policy::protected` (P, bei Ordnern zusätzlich B) :274-295; bei k = 2 also 3 S + 1 M + 3 P + 1 B.

| Nr | Schritt (Funktion, Datei:Zeile) | lokal → lokal (Linux) | lokal → SFTP: RTs |
|---|---|---|---|
| 1 | `admit` → `guard(Quelle)` `apply_reporting.rs:231` | 3 S, 1 M, 3 P, 1 B | 0 |
| 2 | `guard(Ziel)` :233 | 3 S, 1 M, 3 P, 1 B | 3 stat + 1 `statvfs@openssh` (`sftp/core/extensions.rs:173-185`) |
| 3 | `apply_boundary::target(Ziel)` :234 | 1 M | 1 `statvfs` |
| 4 | `sync_path(parent)` + `FolderRegister::ensure` :236-245 | erster Zugriff je Ordner: Anlage | erster Zugriff: Anlage |
| 5 | `flows.transfer` :254 → `acquire_pair` (`transfer/os/shared/flow.rs:444`) | Sperre | 0 |
| 6 | `Actions::run` (`apply_actions.rs:68`): `gate` (Sink-Sperre) :69, `sync_path` ×2 :86-89, `guard(Quelle)` :90, `guard(Ziel)` :99 | 6 S, 2 M, 6 P, 2 B | 3 stat + 1 `statvfs` |
| 7 | `capture(Quelle)` :188, `target(Ziel)` :191 | 1 S, 1 M | 1 `statvfs` |
| 8 | `apply_transaction::copy` (`apply_transaction.rs:28`): `gate` :45, `capture(Quelle)` :46, `guard(Ziel)` :51, `capture(Ziel)` :58 | 5 S, 1 M, 3 P, 1 B | 4 stat + 1 `statvfs` |
| 9 | `apply_stage::stage` (`apply_stage.rs:69`): `mkdir_all(parent)` :85, `unique_staging_path` :87 (`vfs/core/promotion.rs:34-56`, `try_exists`), `unix_mode(Quelle)` :110, `open_read_regular` :161 (Optik-Probe, `lstat`/`open`/`fstat`/`fcntl`×2), `open_write_copy_stage_timed` :162 (`create_new_private`, O_EXCL 0600, `local_extensions.rs:78-87`), `stream` :168 (`transfer_stream.rs:423-458`: read/write 256 KiB, MD5 inline bis 1 MiB, danach Hilfsthread), `revalidate(Quelle)` :189, `capture(Stage)` :190, `finish_stage` :207 (`vfs/os/shared/local_stage.rs:33-76`: `lstat`/`open`/`fstat`, `futimens`, `fchmod`; `fsync` nur bei `Now`) | 3 S + ≈ 25 Aufrufe Datenpfad | `mkdir_all` = `mkdir` + `stat` je Komponente ab `/` (`sftp/core/backend.rs:361-379`, `transfer_ops.rs:54-73`) = 8 bei r + k − 1 = 4; `try_exists` 1; `open` 1; `write` ≥ 1; `close` 1; `capture` 1; `finish_stage`: `setstat` 1 + `open`/`fsync`/`close` 3 (`sftp/core/extensions.rs:215-234`) |
| 10 | `gate` :79, `revalidate(Ziel)` :80, `guard(Ziel)` :87 | 4 S, 1 M, 3 P, 1 B | 4 stat + 1 `statvfs` |
| 11 | Ziel fehlte → keine Version :94-127; `gate`, `revalidate(Quelle)` :132, `guard(Ziel)` :133 | 4 S, 1 M, 3 P, 1 B | 3 stat + 1 `statvfs` |
| 12 | `Staged::publish` → `replacement_publish.rs:10`: `revalidate(Ziel)` :18, `promote_staged_create` :83 → `validate_staged_file` (`promotion.rs:147`, stat) + `rename_no_replace` (`renameat2(NOREPLACE)`, `local_platform.rs:186`), `current_like` :86, bei `Deferred` kein `confirm_namespace` :118-122 | 3 S + 1 `renameat2` | 3 stat + 1–2 Umbenennung |
| 13 | `report` → `CheckpointSink::completed` (`checkpoint_run.rs:424-436`, Sperre, ab 64 offenen Aktionen Flush), Log-Zeile (`run_log.rs:46-68`, ein `write` unter Mutex), Beobachter (`daemon/os/shared/own_writes.rs:74-93`) | 1 `write` | 0 |

Summe je neuer 4-KiB-Datei lokal → lokal unter Linux: 31 S, 9 M, 21 P, 7 B ≈ 465 + 72 + 378 + 84 + 40
≈ **1040 Systemaufrufe**, davon ~16 vollständige Lesungen von `/proc/self/mountinfo`;
≈ 620 + 360 + 420 + 315 + 40 µs ≈ **1,8 ms Kernzeit** je Datei und Worker (Schätzung). Unter Windows
≈ 31 × 60 + 9 × 50 + 21 × 150 + Datenpfad (`FlushFileBuffers` 2 ms + `MoveFileExW` 1,5 ms) ≈ **9 ms**.

Summe je neuer Datei lokal → SFTP: **≈ 43 RTs** (15 `stat` aus 5 `guard`, 7 `statvfs`, 8 `mkdir_all`,
13 Stage-, Prüf- und Veröffentlichungsanfragen). Lokal → Share (Direct LAN): `stat` und
`target_limits` sind je eine RPC (`share/core/peer_extensions.rs:115-128` ruft `Capabilities` je Aufruf),
`mkdir_all` eine RPC (`share/core/backend.rs:424-431`) → **≈ 34 RTs**.

Durability je neuer Datei (`StageDurability`, `apply_actions.rs:221-226`): Linux auf Batch-Dateisystemen
(ext4, XFS, Btrfs, F2FS, tmpfs, overlay laut `vfs/core/fs_profile.rs:53-57`) mit `cross_mounts = false`
(Job-Standard `syncjobs/core/types.rs:227`) → `Deferred`: kein `fsync` je Datei, Bestätigung über `syncfs`
im Checkpoint. Windows: `sync_filesystem` meldet für NTFS `true` (`vfs/os/shared/local_extensions.rs:97-106`),
aber `batched_device` ist `None` (`local.rs:106-117`) → `FlushFileBuffers` je Stage
(`local_stage.rs:66-73`) und `MoveFileExW` mit Write-Through je Veröffentlichung
(`vfs/os/windows/local_platform.rs:220`). Android-Speicher (FUSE) → `PerFileOnly`
(`fs_profile.rs:57`) → `Now`: `fsync` der Stage plus `confirm_namespace` (Verzeichnis-`fsync`,
`vfs/os/linux_os/namespace_flush.rs:12-40`) je Datei; dort hat `/proc/self/mountinfo` typischerweise
Hunderte Zeilen (A3), also ~16 × 200 µs ≈ 3,2 ms je Datei nur für Mount-Tabellen.
Ersetzt eine Kopie eine vorhandene Datei, gilt immer `Now` und zusätzlich die Version (§2.7).

### 2.5 Checkpoint und Journal

`CheckpointSink::completed` (`checkpoint_run.rs:424-436`) hält die Zustandssperre und ruft bei 64 offenen
Aktionen (`CHECKPOINT_ACTIONS`, :18) oder nach 2 s (:19) `flush` (:251-354) auf:
`flush_side` → `vfs::sync_filesystem` → Linux `syncfs` des Ziels (`local_platform.rs:62-75`), danach
`Journal::append` (`checkpoint_journal.rs:121-174`): `ensure_directory` (private Ordnerkette),
`symlink_metadata`, `open`, `set_len`, `seek`, 3 × `write`, `sync_all` und `sync_filesystem` auf den
Journalpfad = ein zweites `syncfs`, jetzt auf dem Datenträger der App-Daten (meist derselbe wie das Ziel).
`should_stop` (:438-443) nimmt dieselbe Sperre und wird von `gate()` in jeder Aktion 3–4-mal aufgerufen
(`apply_actions.rs:69`, `apply_transaction.rs:45, 79, 129`). Während eines Flushes stehen deshalb alle
Worker. `syncfs` schreibt alle schmutzigen Daten des ganzen Dateisystems, auch die anderer Programme.

### 2.6 Datenpfad je Byte

`transfer_stream::stream` (`transfer_stream.rs:423-458`): ein Thread liest 256 KiB, schreibt sie
(`write_all`) und gibt sie an `StreamHasher::update`; Lesen und Schreiben überlappen nicht.
MD5 läuft für das erste MiB im Kopierthread, danach im Hilfsthread mit Kanal-Tiefe 4 (`stream_hash.rs:12-14`),
also bremst MD5 (0,6 GB/s nach A2) jede große Kopie. Der Digest wird für Größe+Zeit-Vergleich ohne
Anbieter-Hash anschließend verworfen (`apply_actions.rs:40-50` setzt `hash = 0` bei `HashMode::None`).
Verbraucher des Digests sind nur: Prüfsummenmodus und Anbieter-MD5 (`apply_stage.rs:184-206`),
`opts.verify` und reversible Ersetzung (`replacement_publish.rs:99-115`), Ersetzungs-Journal
(`replacement_journal::prepare`, :49-57) und Versionsmanifest (`version_save.rs:194-283`).
Spool über eine Temp-Datei nur bei nicht autoritativer Größe oder gleicher Gegenstelle ohne gleichzeitiges
Lesen/Schreiben (`apply_stage.rs:91-93`; Standard `concurrent_read_write = true`, `vfs/core/core.rs:285-287`).

### 2.7 Ersetzen, Löschen, Konfliktkopien

- **Ersetzen** einer vorhandenen Datei (`apply_transaction.rs:94-114`): `version_save::save`
  (`version_save.rs:17`) → bei `VersionsLocation::Auto` und Anbieterarchiv `archive` :65-191:
  `replica()` (stat und Lesen der Markierungsdatei), `ensure_dirs` (stat Wurzel + 4 Komponenten +
  `mkdir_all`, `version_manifest.rs:165-188`), **vollständiges Lesen der alten Datei mit MD5** bei lokalem
  Backend :90-110, `intent.json` über `record::write` (`version_manifest.rs:100-129`: `DurableFile` = `fsync`
  beim `flush`, danach `finish_stage(Now)` = zweites `fsync`, Umbenennen, Verzeichnis-`fsync`),
  `finish_stage(Now)` auf die alte Datei (`fsync`) :126-133, Umbenennen nach `.se-versions` :134,
  `namespace` (Verzeichnis-`fsync`) :147, `capture`, `entry.json` (wieder 2 `fsync` + Verzeichnis-`fsync`).
  Danach Stage mit `Now` (`fsync`) und `confirm_namespace`. Summe ≈ 9 Synchronisierungen plus ein
  vollständiges Lesen der alten Datei.
- **Löschen** (`apply_transaction.rs:187-309`): dieselbe Version wie beim Ersetzen (Jobs sind immer
  reversibel, `syncjobs/core/types.rs:328`), dazu `normalized_missing` auf der Gegenseite
  (`apply_boundary.rs:321-367`, listet jeden Pfadteil) und `namespace` nach dem Löschen. Papierkorb-Option:
  private Kopie über `copy_private` (`version_save.rs:194-283`: komplette Kopie in die App-Daten mit `fsync`).
- **Konfliktkopie** (`KeepBoth`, `apply_actions.rs:200-219` → `apply_transfer.rs:142-200`): die vorhandene
  Zieldatei wird über `stage()` vom Ziel-Backend gelesen und auf dasselbe Backend geschrieben; entfernt heißt
  das vollständiger Download und Upload über diesen Rechner (bei gleicher Gegenstelle ohne gleichzeitiges
  Lesen/Schreiben zusätzlich über eine Temp-Datei), lokal eine volle Kopie statt Reflink.

### 2.8 Zeitlinien (Schätzung nach A1–A8)

**Lokal → lokal, Linux ext4, gleiche NVMe**

| Datensatz | Start | Scan | Plan | Apply | Checkpoint-Stillstände | Ende | Gesamt | Leerlauf |
|---|---|---|---|---|---|---|---|---|
| D1 | 30 ms | 50 ms (≈ 100 000 Systemaufrufe, vor allem `own_path`) | 20 ms | 5000 × 1,8 ms / 4 Kerne ≈ 2,3 s (Flow erreicht 4 nach ≈ 0,2 s) | 78 × ≈ 12 ms ≈ 0,9 s, alle Worker stehen | 50 ms | **≈ 3,3 s** | Platte ≈ 85 % untätig; CPU steht während der Stillstände |
| D2 | 30 ms | 10 ms | 5 ms | 1000 × (1,8 ms Prüfung + 0,5 ms Lesen + 1,7 ms MD5) / 4 ≈ 1,0 s | 16 × ≈ 40 ms ≈ 0,65 s | 40 ms | **≈ 1,7 s** | Platte ≈ 40 % untätig |
| D3 | 30 ms | <1 ms | <1 ms | 10 GiB / 0,6 GB/s (MD5-gebunden) ≈ 18 s; 1 Worker | Ende: `syncfs` ≈ 0,5–1,5 s | 30 ms | **≈ 19 s** | Platte ≈ 45 % untätig; 3 Kerne untätig |

Bei vorhandenem Ziel (D3 ersetzt eine 10-GiB-Datei): + ≈ 18 s Lesen der alten Datei für die Version,
+ `fsync` der Stage (≈ 5 s, teilweise überlappt) → ≈ 40 s.

**Lokal → lokal, Windows NTFS:** D1 ≈ 5000 × 9 ms / ≈ 5 parallele Flushes ≈ 9 s plus 78 × 2 ms Journal;
D2 ≈ 1000 × (9 ms + 2 ms Daten + 1,7 ms MD5) / 4 ≈ 3,2 s; D3 ≈ 18 s (MD5-gebunden) + `FlushFileBuffers`
der Stage (≈ 5 s, seriell nach dem Schreiben) ≈ 23 s.

**Lokal → SFTP, LAN (A5):** D1 ≈ 5000 × (43 × 0,5 ms + 1 ms Server-`fsync`) / 8 Worker ≈ 14 s
(Flow-Hochlauf von 2 auf 8 ≈ 0,4 s); D2 ≈ max(1000 × 22 ms / 8, 1 GiB / 115 MB/s) ≈ 9,3–12 s, ab ≈ 4
parallelen Workern linkgebunden; D3 = ein Strom (SFTP-Durchsatz eines Stroms gehört zu G3), MD5-Hilfsthread
mit 0,6 GB/s begrenzt nicht, Server-`fsync` von 10 GiB am Ende ≈ 5 s seriell → ≈ 95–175 s.
**Lokal → SFTP, WAN (A6):** D1 ≈ 5000 × 43 × 30 ms / 8 ≈ **13 min**; D2 ≈ 1000 × 1,4 s / 8 ≈ 175 s statt 87 s
Linkgrenze.

### 2.9 Vergleich mit dem Maßstab

| Weg | Referenz (A9) | se (Schätzung) | Verhältnis | Leerlauf des knappsten Mittels | Ursachen | Maßnahmen | Gewinn (Schätzung) |
|---|---|---|---|---|---|---|---|
| lokal→lokal Linux D1 | `rsync -a` ≈ 0,4 s; `cp -r` ≈ 0,15 s | 3,3 s | 8–20× | Platte ≈ 85 % | F1, F3 | MS1–MS3, MS5 | → ≈ 0,4 s |
| lokal→lokal Linux D2 | `cp -r` 0,6 s; `rsync` 1,2 s | 1,7 s | 1,4–3× | Platte ≈ 40 % | F1, F3, F4 | MS1–MS5 | → ≈ 0,6 s |
| lokal→lokal Linux D3 | `cp` (copy_file_range) 7–10 s, Reflink < 0,1 s | 19 s | 2× bzw. > 100× | Platte ≈ 45 % | F4 | MS4 | → 7–10 s bzw. < 0,1 s |
| lokal→lokal Windows D1 | `robocopy /MT:16` ≈ 1,5 s | 9 s | ≈ 6× | Platte ≈ 80 % | F1 (Windows-Kosten), Flush je Datei | MS1–MS3, MS17 | → ≈ 2 s |
| lokal→SFTP LAN D1 | `rsync` über SSH ≈ 2 s; `rclone copy --transfers 8` ≈ 3 s | 14 s | 5–7× | Netz (RT-gebunden) ≈ 85 % | F2 | MS1, MS6–MS9 | → ≈ 2–3 s |
| lokal→SFTP WAN D1 | `rsync` ≈ 10 s | 13 min | ≈ 80× | Netz ≈ 99 % | F2 | MS6–MS9, MS14 | → 10–30 s |
| lokal→SFTP LAN D2 | Linkgrenze 9,3 s | 9,3–12 s | 1–1,3× | Netz ≤ 25 % | Hochlauf, F2 | MS6–MS8, MS15 | → 9,5 s |
| Spiegel-Ende D1 SFTP | rsync: keiner | +5 s LAN, +5 min WAN | — | Netz/Platte 100 % in dieser Phase | F10 | MS12 | → 0 |

### 2.10 Zielbild Sync und Maßnahmen

Zielbild: ein Lauf als Pipeline mit einem Pfadkontext je Lauf. Scan beider Seiten (parallel wie heute),
Plan, danach Kopien über die Transfer-Engine-Primitive (Kernel-Kopie lokal, Pakete für kleine Dateien zu
Share/SFTP, ein Committer-Thread für Gruppen-Commits), Prüfungen nur am Blatt mit gehaltenem
Elternverzeichnis-Handle, keine prozessweiten Dateisystem-Abfragen je Datei.

| Nr | Änderung (Ort) | Gewinn (Schätzung) | Berührte Invariante und wie sie gehalten wird |
|---|---|---|---|
| MS1 | **Pfadkontext je Lauf.** Neue Datei `bisync/os/shared/apply_path_context.rs`: je Seite Wurzel-Metadaten, `TargetLimits` (einmal), Mount-Tabelle (einmal), Menge geprüfter Elternordner mit gehaltenem Verzeichnis-Handle (`local_access::ConfinedDirectory`, wie `local_extensions.rs:143-168`). `apply_boundary::guard` (`apply_boundary.rs:244`) und `target` (:298) nehmen den Kontext; das Blatt wird mit einem `fstatat(dirfd, name, AT_SYMLINK_NOFOLLOW)` geprüft. Aufrufer: `apply_reporting.rs:231-234`, `apply_actions.rs:90-105, 191`, `apply_transaction.rs:51, 87, 133, 199, 265`, `version_save.rs:29`, `apply_dirs.rs:23, 67`. | lokal 31 S + 9 M + 21 P + 7 B → ≈ 6 Blatt-`stat` je Datei; ≈ 1040 → ≈ 40 Systemaufrufe; 1,8 ms → ≈ 0,1 ms (Linux), 9 ms → ≈ 4 ms (Windows, Rest = Flush) | Links, Junctions und Mounts in Elternpfaden bleiben geschützte Auslassungen: jedes Elternverzeichnis wird einmal mit `O_NOFOLLOW|O_DIRECTORY` (Windows `FILE_FLAG_OPEN_REPARSE_POINT`) geöffnet und gehalten; ein später ersetzter Elternordner kann den Zugriff nicht umleiten (stärker als heute, wo zwischen `stat` und `open` ein Zeitfenster liegt). Mount-Grenze: `st_dev`/`STATX_MNT_ID` des neu geöffneten Ordners gegen den Elternordner, `AT_NO_AUTOMOUNT`. Drift am Blatt: `capture`/`revalidate` bleiben unverändert. |
| MS2 | `own_path` (`snapshot_policy.rs:6-27`) mit einmal je Prozess berechneten Präfixen (`OnceLock`), neue Funktion in `support_dirs.rs` neben `app_data_dir` (:196) ohne `ensure_private_dir` je Aufruf. | Scan: −15–20 Systemaufrufe je Eintrag; Apply: in MS1 enthalten | Eigene Dateien werden nie synchronisiert: der Präfixvergleich bleibt gleich; das Anlegen des privaten Ordners einmal je Prozess genügt dem Vergleich. |
| MS3 | Optik-Probe je Gerät zwischenspeichern: `source_media::probe` (`source_media/mod.rs:34`), `probe_device` (`os/linux.rs:30`), Windows `probe` (`os/windows.rs:23`) mit Cache `st_dev`/Volume → Ergebnis; neu prüfen nur für optische Geräte (`diskseq`) und bei unbekanntem Gerät. | −14 Systemaufrufe je S (Linux); −`GetVolumePathNameW` und −64 KiB Nullung je S (Windows) | Optik-Arbitrierung: die Geräteklasse eines eingehängten `st_dev` ändert sich nicht; optische Medien behalten Permit und Medienwechselprüfung. |
| MS4 | **Kernel-Kopie ohne unnötigen Digest.** In `apply_stage.rs:160-179`: sind beide Seiten lokal, `expected.hash() == 0`, kein Anbieter-MD5, kein `verify`, Ziel fehlt → `destination.server_copy_to_stage` (`vfs/os/shared/local.rs:329-343` → `copy::copy_to_new_file`: `copy_file_range`, Reflink, `CopyFile2`, SMB-Offload). Braucht ein Verbraucher (§2.6) den Digest: Lesen, Schreiben und MD5 in drei Threads mit Ringpuffer statt Inline-Hash. | D3 lokal 19 s → 7–10 s (ext4/NTFS) bzw. < 0,1 s (Btrfs, XFS mit Reflink, ReFS/Dev Drive); D2 −1,7 ms je Datei | „Digest und Größe gehören zu den gelesenen Bytes“ (`transfer_stream.rs:398`) gilt nur, wo ein Verbraucher existiert; Größe kommt aus der Kopierfunktion; `revalidate(Quelle)` nach der Kopie bleibt; Stage bleibt exklusiv neu angelegt und unveröffentlicht bis `publish`. |
| MS5 | **Checkpoint aus dem Worker-Pfad.** `CheckpointSink::completed` (`checkpoint_run.rs:424`) hängt nur an; `should_stop` (:438) liest Atomics; der Thread `sync-checkpoint` (:156) übernimmt `flush` als Gruppen-Commit (alle 250 ms oder 1024 Aktionen). `Journal::append` (`checkpoint_journal.rs:121-174`): `fdatasync` der Journaldatei statt `sync_all` + `syncfs` des App-Datenträgers; Verzeichnis-`fsync` nur beim Anlegen. `can_delete` (:222) wartet weiter synchron auf einen Flush. | Worker stehen nie still: D1 −0,9 s, D2 −0,65 s; je Append ≈ 1 statt ≈ 6 ms | Reihenfolge bleibt: `syncfs` des Ziels vor dem Journal-Eintrag einer `Deferred`-Aktion (im Committer). Absturz vor dem Eintrag: beide Seiten haben gleiche Größe und Zeit (Zeit setzt `finish_stage`), der nächste Lauf erkennt Konvergenz; ohne Baseline-Eintrag entsteht nie eine Löschung. |
| MS6 | `target_limits` je Lauf: SFTP `statvfs@openssh` (`sftp/core/extensions.rs:173-185`) und Share `Capabilities` (`share/core/peer_extensions.rs:115-128`) im Pfadkontext (MS1) einmal je Wurzel. | −7 RTs je Datei | Namensgrenzen sind je Wurzel fest; ein Wechsel des Servers mitten im Lauf wird über den Verbindungsabbruch erkannt. |
| MS7 | Doppeltes `mkdir_all(parent)` in `apply_stage.rs:84-86` (und `stage_bytes` :233) entfällt, wenn `FolderRegister` den Ordner als `Ready` meldet; SFTP-`mkdir_all` (`sftp/core/backend.rs:361-379`) beginnt beim tiefsten vorhandenen Ordner statt bei `/`. | −8 RTs je Datei (SFTP) | Ordner bleiben echte Ordner: `FolderRegister` legt über `create_dir` an, lokal `ensure_plain_component` (`local.rs:318`, `local_dirs.rs:101-120`) mit Link-Ablehnung. |
| MS8 | SFTP-Stage: `CREATE|EXCL` statt `try_exists` (`promotion.rs:49`), `fsetstat` und `fsync@openssh` auf dem offenen Handle statt erneutem Öffnen (`sftp/core/extensions.rs:215-234`), `validate_staged_file` (`promotion.rs:147`) entfällt für die eben erfasste eigene Stage (`apply_stage.rs:190`). Abstimmung mit G3. | −5 RTs je Datei | Exklusive Stage und Veröffentlichung ohne Ersetzen bleiben; die Stage-Prüfung ist durch `capture(Stage)` erbracht. |
| MS9 | Kleine neue Dateien zu Share und SFTP über die Paket-Primitive der Transfer-Engine (`share/core/peer_batch_put.rs`, `sftp/core/batch_put.rs`) mit Belegen je Mitglied (Vorschlag B in `analyse-kopieren.md`). | D1 SFTP LAN 14 s → ≈ 2–3 s; WAN 13 min → ≈ 30 s | Erfassen und Prüfen je Datei: der Host liefert Größe, Zeit und Identität je Mitglied unter derselben Rechteprüfung. |
| MS10 | Ordnerläufe (`apply_reporting.rs:124-146, 331`) ebenenweise parallel über den Pool; Verzeichnis-`fsync` je angelegtem Ordner (`apply_dirs.rs:93`) auf `Deferred`-Dateisystemen über den Checkpoint-`syncfs`. | 50 leere Ordner lokal −50–250 ms seriell; entfernt −50 × RT | Eltern vor Kindern (Ebenenreihenfolge), Entfernen tiefste Ebene zuerst. |
| MS11 | Versionen (`version_save.rs:65-191`): lokal Identitätsbeleg (Gerät, Inode, Größe, Zeit, ctime) statt vollständigem Lesen (:90-110), Digest bei Bedarf später; `record::write` (`version_manifest.rs:100-129`) nur ein `fsync` (`open_write_copy_stage_unsynced` + `finish_stage(Now)`); Absichten mehrerer Dateien gruppiert mit einem `fsync` vor den Umbenennungen. | Löschen D1: ≈ 9 → ≈ 1 Synchronisierung je Datei; 10-GiB-Ersetzung −18 s | Reversibilität: Absicht ist dauerhaft vor dem Umbenennen (gruppiertes `fsync` davor); Wiederherstellung prüft Größe, Zeit und Identität, den Digest dort, wo er vorhanden ist. |
| MS12 | Spiegel-Index (`engine_change_feed.rs:189-260`) aus Snapshot und abgeschlossenen Aktionen des Laufs aufbauen, nur berührte Einträge prüfen; Rest im Hintergrund nach Meldung des Erfolgs. | −2 vollständige Scans und −4 × Einträge Anfragen am Laufende | Dirty-Marker bleibt bis zum geprüften Aufbau (`incremental_index_commit.rs:61`). |
| MS13 | Konfliktkopie serverseitig: `apply_transfer.rs:142-200` über `server_copy_to_stage` (lokal Reflink/`copy_file_range`, SFTP `copy-data` in `sftp/core/copy_data.rs`, Share-Serverkopie). | entfernt: −Download −Upload der Konfliktdatei | Original bleibt bis zur Veröffentlichung des Geschwisters ohne Ersetzen an seinem Platz. |
| MS14 | Scan und Apply überlappen (Produktentscheidung zu FS3 nötig): additive Kopien (Ziel fehlt) starten ordnerweise, sobald beide Wurzeln gelistet sind und keine Seite leer ist; Löschungen und Löschschwellen erst nach dem vollständigen Scan. | entfernte Bäume mit vielen Ordnern: Scanzeit verschwindet aus dem kritischen Pfad | FS3 („vor der ersten Änderung“) gilt dann nur noch für Löschungen und Seite-leer — ohne Entscheidung bleibt die Barriere und MS14 entfällt. |
| MS15 | Scan-Hashing (`snapshot_dir.rs:178-200`) in einem begrenzten Pool über Dateien eines Ordners; im Modus `Full` neue Dateien ohne Gegenstück nicht vorab hashen (der Kopierstrom liefert den Digest). | D3 zu Drive: −18 s Vorab-Hash | Baseline-`Sig.hash` neuer Dateien kommt aus `outcome.source` des Kopierstroms. |
| MS16 | Flow-Start lokaler Paare bei Kernanzahl statt 2 (`transfer/core/flow_control.rs:33`, `flow_window.rs:19-21`). | Hochlauf −0,2 s | keine |
| MS17 | Windows: Stages mehrerer Dateien parallel mit `FlushFileBuffers` abschließen (NTFS fasst Log-Commits zusammen), Veröffentlichungen parallel. | D1 Windows Flush-Anteil ÷ Parallelität | Stage ist vor ihrer Veröffentlichung dauerhaft (unverändert). |
| MS18 | Job-Log (`run_log.rs:46-68`) gepuffert, Flush alle 150 ms und am Laufende. | −1 `write` je Aktion | Log-Inhalt unverändert; Fortschrittssignal `mark_active` bleibt je Zeile. |

---

## 3 se-Laufwerke (FUSE unter Linux, Dokany unter Windows, Dokumentanbieter unter Android)

Hinweis zum Auftrag: unter Windows nutzt se **Dokany**, nicht CfAPI oder WinFsp
(`mount/os/windows/`, `docs/ARCHITEKTUR.md` Zeile „Einhängen je Betriebssystem“).

### 3.1 Prozesse, Threads, IPC

| Ebene | Linux | Windows | Android |
|---|---|---|---|
| Prozess je Laufwerk | Laufwerk-Host `--mount-host` (`mount/os/linux_os/fuse_host.rs:1-60`) | Laufwerk-Host-Prozess (`mount/os/windows/host.rs`) | im App-Prozess (`mobile/os/shared/domains/docs_engines.rs`) |
| Kernel-Lesethreads | `fuser` mit 2 Leseschleifen, `clone_fd` (`fuse_host.rs:47, 184-199`) | Dokany-Bibliothek, `single_thread = 0` (`dokany_abi.rs:69`) | AppFuse über `openProxyFileDescriptor` (`android/app/src/main/java/app/smartexplorer/android/system/documents/SeDocumentsProvider.kt:140-146`) |
| Arbeitsthreads | elastischer Pool `fuse-worker`, 2 bleiben, ohne Obergrenze, Rest endet nach 2 s (`fuse_workers.rs:25-27, 70-97`) | Dokany-Rückrufthreads; Zeitlimit-Überwachung `mount-timeout-supervisor` (`callback_timeout.rs:71`), Rückruf-Zeitlimit 300 s (`callback_status.rs:29`, `host.rs:346`) | 1 `HandlerThread` je geöffnetem Deskriptor (`ProxyFileCallback.kt`), 1 Thread je `release` (`docs_io.rs:202-216`) |
| Metadaten-Hintergrund | `fuse-metadata-refresh` alle 20 s (`fuse_workers.rs:28, 189-270`) | `metadata_refresh.rs` | — |
| Backend-Weg | Host → **eine** Loopback-TCP-Verbindung mit Agent-Protokoll (`daemon/os/shared/mount_client_host.rs:255-286`) → Daemon `serve_backend` (`backend_server.rs:124`, ein Thread je Anfrage) → `RootedBackend` (`rooted_backend.rs`) → `CachingBackend` → Backend | gleich | direkt im Prozess |
| Gleichzeitige Backend-Anfragen | `MountRequestGate` mit `parallelism().clamp(1, 8)` (`ipc_protocol_mount_capabilities.rs:55`, `mount_proxy.rs:22`): SFTP 1 (`sftp/core/backend.rs:433-437`), Share 8 (`share/core/backend.rs:441-443`); Übertragungen warten höchstens 15 s, Metadaten 10 s (`mount_request_gate.rs:7, 16`) | gleich | — |
| Rahmen und Kredit | Agent-`CHUNK` 256 KiB (`agent_proto/core/types.rs:25`), Kredit 1 MiB je Anfrage, Fenster bis 8 MiB, 64 MiB je Verbindung (`agent_proto/core/credit.rs:23-33`); ein Schreibaufruf = ein Frame ohne Puffer (`agent/core/stream.rs:165-191`) | gleich | — |

### 3.2 Pfadprüfung je Rückruf

- Host: `MountEngine::open` → `open_at_root(…, true, true)` (`mount/core/engine_recovery.rs:43-58`), also
  `verify_backend_ancestors = true`. `project_checked` (`mount/core/engine.rs:130-153`) ruft `stat(Wurzel)`
  und `stat` jedes Vorfahren über die IPC auf, bei Pfadtiefe d also (1 + d − 1) IPC-Anfragen; aufgerufen aus
  `materialize_at` (`materialization.rs:98`), `read_range` (`range_read.rs:73`) und `stat_overlay`
  (`metadata.rs:42`, auch bei Cache-Treffer, denn die Prüfung kommt vor dem Cache).
- Daemon: `RootedBackend::stat` (`rooted_backend.rs:252-263`) → `PathResolver::stat`
  (`rooted_backend_case.rs:30-33`) → `walk` :35-120: bei `revalidate_root` (vertrauter Modus oder Wurzel
  nicht vom Backend begrenzt) `validate_root` je Anfrage (`stat` jedes Wurzelvorfahren ungecacht, :157-182),
  je Komponente gecachte Auflistung oder `stat`, für die letzte Komponente immer `raw.stat` ungecacht (:93).
  Schreibende Auflösung (`checked_destination`, `resolver(true)`) läuft vollständig ungecacht (:176-190).

### 3.3 Schreiben auf ein Laufwerk (Datei aus dem OS-Dateimanager auf das Laufwerk kopieren)

Ablauf je Datei, Linux (`cp`), Windows (Explorer: `CreateFile`, `WriteFile`, `SetFileTime`,
`CloseHandle` → `Cleanup`), beide kopieren Datei für Datei nacheinander:

| Nr | Rückruf | Funktionen (Datei:Zeile) | Kosten |
|---|---|---|---|
| 1 | `lookup` / `GetFileInformation` | `stat_cached` → `project_checked` | (1 + d − 1) IPC-`stat`, je ≥ 1 RT |
| 2 | `create` (FUSE `fuse_ops_io.rs:85-135`) bzw. `CreateFile` | `MountEngine::open_file` (`file_io.rs:14`) → `materialize_at` (`materialization.rs:82`): Pfad-Gate, `project_checked`, `materialize_fetch` :153 (`backend.stat` = NotFound, `clean_cache.trim`, `reserve_growth`, `spool.allocate` = neue Datei, `sync_data` der leeren Spool :278), `materialize_install` :288 (`persist_entry` → `journal.rs:94-129`: Siegeln, `write`, `sync_data` unter der Journal-Sperre) | ≈ d + 1 RTs, 2 `fdatasync` |
| 3 | `write` je 128 KiB–1 MiB (FUSE `max_write` 1 MiB, `fuse_fs.rs:34, 250`, ohne Writeback-Cache) | `MountEngine::write` (`file_io.rs:136-158`): Handle-Sperre und Klon, `spool.open_file` (`spool.rs:167-182`: `lstat` + `open`), `fstat`, `reserve_growth`, `mark_dirty`, `seek`, `write_all`, `close`; 2 Thread-Wechsel (Leseschleife → Pool) | ≈ 6 Systemaufrufe + Datenmenge je Rückruf |
| 4 | `setattr` (Zeiten, Modus) | `pending_attributes` (`mount/core/pending_attributes.rs`), angewendet bei der Veröffentlichung (`file_commit.rs:101`) | 0 RT bis zur Veröffentlichung |
| 5 | `flush` bei `close(2)` (`fuse_ops_io.rs:168-205`) bzw. `Cleanup` (`callbacks_mutation.rs:106-160, 189-205`) | `flush_entry` (`file_commit.rs:9-205`) **synchron**: `detect_conflict` :30 (`stat`), `collect_orphan_stages` :36, `unique_staging_path` :37 (`try_exists`, schreibende Auflösung), `spool.sync_data` :42, `stages().begin` :46 (`stages.json` komplett neu schreiben + `sync_data` + `rename`, `stage_ledger.rs:75-122`), `open_write_new` :50 (schreibende Auflösung + `open`), `io::copy` :60 mit **8-KiB-Puffer** (Standard von `std::io::copy` für diese Typen) → je 8 KiB ein Agent-Frame, `flush`/`close`, `detect_conflict` :74 (`stat`), `finish_pending_stage` :101, `promote_staged_no_replace` :106 (zwei schreibende Auflösungen + `stat` + Umbenennen), `stages().resolve` :113 (Ledger neu schreiben + `sync_data`), `backend.stat` :164, `forget_entry` :186 (Journal + `sync_data`) | ≈ 15 RTs bei d = 2 ohne Wurzelprüfung, ≈ 35–50 RTs mit `revalidate_root` (r = 3); 4 `fdatasync`; Upload der ganzen Datei; der Aufrufer wartet |
| 6 | `release` / `CloseFile` | `close` (`file_io.rs:217-234`): Schreibsperre des Namensraums, `cleanup_committed_entry` | lokal |

Windows: `write_file` (`callbacks_io.rs:42-91`) fragt bei Paging-I/O zusätzlich `len` ab. Android: `doc_release`
(`docs_io.rs:202-216`) veröffentlicht in einem eigenen Thread, die schreibende App wartet nicht; `fsync` des
Deskriptors veröffentlicht synchron (`ProxyFileCallback.kt` `onFsync`).

### 3.4 Lesen von einem Laufwerk (Datei vom Laufwerk in einen lokalen Ordner kopieren)

| Nr | Rückruf | Funktionen | Kosten |
|---|---|---|---|
| 1 | `open` (`fuse_ops_io.rs:29-64`) | `stat` + `open_metadata_file` (`file_io.rs:52-81`): ohne Inhalt | (1 + d) IPC-`stat` |
| 2 | erstes `read` (`fuse_ops_io.rs:137-147`, Puffer `vec![0; size]` je Aufruf) bzw. `read_file` (`callbacks_io.rs:15-40`) | `read` (`file_io.rs:108`) → `read_range` nur bei Nur-Lese-Laufwerk (Android: Nur-Lese-Handles) und Größe > max(Cache 500 MiB, 1 MiB) (`range_read.rs:47-54`, `cache_policy.rs:6`); sonst `materialize_opened` → `materialize_fetch`: `stat`, `open_read_id`, `io::copy` der **ganzen Datei** mit 8 KiB in die Spool :256-260, `sync_data` :268, zweites `stat` :269-276 | ganze Datei über das Netz, dann `fdatasync`; erstes Byte erst danach |
| 3 | weitere `read` | `spool.open_file` + `seek` + `read` + `close` je Rückruf (`file_io.rs:119-133`) | lokal ≈ 5 Systemaufrufe |
| 3′ | `read` bei Bereichslesen | `read_range` (`range_read.rs:56-106`): `project_checked`, `stat`, `open_read_at`, `read_exact`, `stat` | 3 RTs + Bereich je Rückruf, ohne Vorauslesen |

Spool eines Tresor-Laufwerks: jede Spool-Datei liegt im Format `SIV_GCM` unter dem Spool-Schlüssel
(`vault/os/shared/sealed_spool.rs:1-80`); jedes `open_file` liest und entschlüsselt den 68-Byte-Kopf.

### 3.5 Zeitlinien (Linux FUSE, SFTP-Laufwerk im LAN, `cp`; Schätzung)

| Datensatz | Schreiben auf das Laufwerk | Leerlauf | Lesen vom Laufwerk | Leerlauf |
|---|---|---|---|---|
| D1 | je Datei ≈ 20 RTs (10 ms) + 6 `fdatasync` (6 ms) + 25 IPC (1,5 ms) ≈ 17 ms → **≈ 85 s** (mit `revalidate_root` ≈ 160 s) | Netz ≈ 97 % | je Datei ≈ 10 RTs + 1 `fdatasync` ≈ 6,5 ms → **≈ 33 s** | Netz ≈ 95 % |
| D2 | je Datei ≈ 17 ms + 1 MiB Upload 9 ms ≈ 26 ms → **≈ 26 s** | Netz ≈ 65 % | je Datei 9 ms Download + 7 ms → ≈ 16 s | Netz ≈ 45 % |
| D3 | Spool schreiben ≈ 5 s (10 GiB Cache-Platz nötig, sonst Fehler aus `reserve_growth`), `close` → `sync_data` ≈ 2 s, Upload ≈ 93 s (8-KiB-Frames) → **≈ 100 s**, `close` blockiert ≈ 95 s | Netz während der Spool-Phase 100 %, Platte während des Uploads ≈ 95 % | Download der ganzen Datei ≈ 93 s, `fdatasync` ≈ 3 s, danach lokales Kopieren ≈ 7 s → **≈ 103 s**; erstes Byte nach ≈ 96 s | Zielplatte 96 s untätig; Netz 7 s untätig |

Nur-Lese-Laufwerk, D3 (> 500 MiB → Bereichslesen): je 128 KiB 3 RTs (1,5 ms) + 1,1 ms → ≈ 50 MB/s,
also ≈ 215 s. Windows Explorer liest 1 MiB je Aufruf → ≈ 100 MB/s im LAN, im WAN ≈ 10 MB/s.
SFTP-Laufwerk: Gate 1 → während eines Uploads warten andere Dateien und Metadaten; nach 15 s bzw. 10 s
Wartezeit liefern sie Zeitüberschreitungen (`mount_request_gate.rs:7-16`).

### 3.6 Vergleich mit dem Maßstab

| Weg | Referenz (A9) | se | Verhältnis | Leerlauf | Ursachen | Maßnahmen | Gewinn |
|---|---|---|---|---|---|---|---|
| Schreiben D1 | `rclone mount --vfs-cache-mode writes`: `close` nach lokalem Cache-Schreiben, Upload asynchron (`--vfs-write-back 5s`, `--transfers 4`) ≈ 2–3 s bis alles oben ist | 85–160 s | 30–60× | Netz ≈ 97 % | F6, F8, Journal-`fsync` | ML1, ML2, ML5, ML7, ML8 | → ≈ 3–5 s |
| Schreiben D2 | rclone ≈ Linkgrenze 9,3 s | 26 s | 2,8× | Netz ≈ 65 % | F6 | ML1, ML6 | → ≈ 10 s |
| Schreiben D3 | rclone: `cp` fertig nach ≈ 5 s, Upload 93 s, gesamt ≈ 98 s | 100 s, `close` blockiert 95 s | 1,0× gesamt | Netz in Spool-Phase, Platte in Upload-Phase | F6 | ML1 (Upload beginnt sofort, überlappt) | → ≈ 93 s, `close` sofort |
| Lesen D3 | rclone `--vfs-cache-mode full`/CfAPI-Hydrierung: Daten fließen ab dem ersten RT, ≈ 93 s | 103 s, erstes Byte nach 96 s | 1,1× gesamt, Startlatenz ≈ 1000× | Zielplatte 96 s | F7 | ML4 | → ≈ 93 s, erstes Byte nach ≈ 3 RTs |
| Lesen D1 | rclone-Lesen ≈ 1–2 RTs je Datei ≈ 7 s | 33 s | ≈ 5× | Netz ≈ 95 % | F7, F8 | ML2, ML4, ML5 | → ≈ 8 s |

### 3.7 Zielbild Laufwerke und Maßnahmen

Zielbild: Rückrufe arbeiten nur gegen die lokale Spool und das Journal; Upload und Download laufen in einem
Hintergrund-Pool je Laufwerk mit Flow-Regelung; Lesen bedient Bereiche, sobald sie vorliegen; der Daemon
prüft Pfade einmal und hält Metadaten kurz zwischen.

| Nr | Änderung (Ort) | Gewinn (Schätzung) | Berührte Invariante und wie sie gehalten wird |
|---|---|---|---|
| ML1 | **Asynchrones Zurückschreiben.** FUSE `flush` (`fuse_ops_io.rs:168-205`) und Dokany `Cleanup` (`callbacks_mutation.rs:159, 189-205`) rufen nur noch `spool.sync_data` und bestätigen den bereits dauerhaften `Dirty`-Eintrag; `flush_entry` (`file_commit.rs:9`) läuft in einem Upload-Pool des Hosts (neue Datei `mount/core/upload_queue.rs`), Größe = Gate-Limit (ML8). `fsync` (FUSE) und `FlushFileBuffers` (`callbacks_io.rs:93-140`) veröffentlichen weiterhin synchron. Android verhält sich schon so (`docs_io.rs:202-216`). | D1 85 s → ≈ 3–5 s; D2 26 s → ≈ 10 s; `close` bei D3 sofort | Absturzsicherheit: Spool und Journal-Eintrag `Dirty` sind vor der Rückkehr von `close` dauerhaft; Retry und Wiederanlauf veröffentlichen nach einem Absturz (vorhandene Wege `stage_ledger.rs`, Journal-Wiederherstellung). Konflikte: `detect_conflict` vor und nach dem Upload unverändert. Lesen nach `close` sieht die lokale Fassung (`metadata.rs:40-50` bevorzugt den Eintrag). OS-Vertrag: „`close` erfolgreich“ heißt nicht mehr „auf der Gegenstelle“; Produktentscheidung inklusive Anzeige ausstehender Uploads und Verhalten beim Aushängen (warten oder Retry). |
| ML2 | Host-Vorfahrenprüfung entfällt: `MountEngine::open` setzt `verify_backend_ancestors = false` (`engine_recovery.rs:43-58`), `project_checked` (`engine.rs:130-153`) prüft nur Projektion und Schreibweise. | −(1 + d − 1) IPC-`stat` je Rückruf | Wurzelsicherheit liegt im Daemon: `RootedBackend` prüft jede Komponente (`rooted_backend_case.rs:35-120`); der Host gilt als nicht vertrauenswürdig (`rooted_backend.rs:17-20`), seine Prüfung schützt nichts zusätzlich. |
| ML3 | Daemon: `validate_root` (`rooted_backend_case.rs:41-44, 157-182`) je Anfrage durch eine an ein Handle gebundene oder kurz (1 s) gültige Prüfung ersetzen; letzte Komponente bei Lesezugriffen aus dem Cache, sofern der Aufrufer danach ohnehin `stat` vergleicht. | −r RTs je Anfrage mit `revalidate_root` | Wurzel darf nicht durch einen Link ersetzt werden: bei Zeitfenster-Variante Sicherheitsentscheidung nötig; ohne Entscheidung nur die Handle-Variante. |
| ML4 | **Streamende Hydrierung.** `materialize_fetch` (`materialization.rs:153-286`): dünn belegte Spool mit Bereichskarte, Hintergrund-Download in Reihenfolge mit Vorauslesen (8–32 MiB), `read` wartet nur auf seinen Bereich; kein `sync_data` für saubere Daten (:268). Schreib-Lese-Handles warten bei Schreibzugriffen auf noch fehlende Teile des betroffenen Bereichs. | erstes Byte nach ≈ 3 RTs statt nach dem ganzen Download; D3 Lesen 103 s → ≈ 93 s mit gleichzeitigem Schreiben der Zielplatte | Konsistenz: heute werden Bytes erst nach dem `stat`-Vergleich am Ende ausgegeben; beim Streamen könnte eine App Bytes einer Fassung lesen, die sich während des Downloads ändert. Haltbar über Identitätsprüfung beim Öffnen und alle N MiB, Fehler `WouldBlock` wie heute; die verbleibende Abschwächung ist zu entscheiden. |
| ML5 | Spool-Handle je Eintrag offen halten (`file_io.rs:119, 147, 171, 186, 259`; `spool.rs:167-182`) statt `lstat`/`open`/`close` je Rückruf; Tresor-Spool: Kopf einmal entschlüsseln. | −5 Systemaufrufe und unter Windows ein Defender-Scan je Rückruf | „Jeder Spool-Zugriff läuft über `SpoolStorage`“: das Handle stammt aus `SpoolStorage`. |
| ML6 | Kopierpuffer 1 MiB statt `io::copy` mit 8 KiB (`materialization.rs:260`, `file_commit.rs:60`); FUSE-`read` mit wiederverwendeten Puffern (`fuse_ops_io.rs:142`). | 256-KiB-Frames statt 8-KiB-Frames; −97 % Frames | keine |
| ML7 | Journal und Stage-Ledger zusammenführen und gruppiert synchronisieren (`journal.rs:94-129`, `stage_ledger.rs:75-122`). | je Datei 4 → ≈ 1 `fdatasync` | Stage ist vor ihrem exklusiven Anlegen verzeichnet (Reihenfolge im gemeinsamen Journal). |
| ML8 | Gate je Backend-Pool: Übertragungen nach `transfer_ceiling`/Pool-Kapazität (`sftp/core/backend.rs:428-431`), Metadaten in eigener Spur (`mount_request_gate.rs`, `ipc_protocol_mount_capabilities.rs:55`). | SFTP-Laufwerk parallel statt seriell | Verbindungsgrenzen der Gegenstelle: Pool-Kapazität bleibt die Obergrenze. |
| ML9 | FUSE-Konfiguration (`fuse_fs.rs:246-252`): `blksize` 1 MiB in den Attributen (größere `cp`-Puffer), `max_readahead` ≥ 1 MiB, `FUSE_ASYNC_READ`, `FUSE_PARALLEL_DIROPS`, `max_background` ≥ 64; Writeback-Cache bleibt aus. | weniger Rückrufe je MiB | Größe und Zeit bleiben in der Hoheit des Hosts (Writeback-Cache bleibt aus). |
| ML10 | Bereichslesen (`range_read.rs:56-106`): ein fortlaufender Leser je Handle, `stat` beim Öffnen und alle N MiB statt zweimal je Rückruf, Vorauslesen. | Nur-Lese D3 ≈ 50 MB/s → Linkgrenze | Änderungserkennung: Identität bei Öffnen und je N MiB. |

---

## 4 Tresore (Cryptomator Format 8)

### 4.1 Wege

| Weg | Ablauf |
|---|---|
| Kopieren in einen bzw. aus einem Tresor (Transfer-Engine, G1) | `VaultBackend` (`vault/os/shared/backend.rs`) als Ziel bzw. Quelle; Schreiben `stage_writer` (`backend_write.rs`) → `VaultWriter` (`backend_io.rs:256-398`) → `EncryptingWriter` (`cryptomator/core/content_stream.rs:185-301`) → Stage im Speicher (`open_write_copy_stage(_sized)` des inneren Backends) → `promote` (`backend_write.rs` `Promote::Create/Replace/ToId`) |
| Tresor-Laufwerk | Mount-Engine (§3) über `VaultBackend` im Daemon, Spool versiegelt mit eigenem Spool-Schlüssel (`sealed_spool.rs`), Journal- und Ledger-Pfade versiegelt (`journal_sealer.rs`) |
| Tresor-Sync-Job | allgemeiner Sync (§2) über den **Chiffretext** beider Speicherorte, ohne Schlüssel, immer Volllauf, Regeln R1–R12 mit zwei Apply-Phasen (`vault/os/shared/sync_job.rs:1-16`, `vault/core/sync_rules.rs:1-25`) |

### 4.2 Kosten je Chunk und je Datei

- Chunk: 32 KiB Klartext (`cipher_combo.rs` `CHUNK_PAYLOAD`), `SIV_GCM` mit 12 Byte Nonce + 16 Byte Tag
  (Standard für neue Tresore, `manage_create.rs:121`), `SIV_CTRMAC` mit 16 + 32 Byte.
- Verschlüsseln je Chunk (`content_chunks.rs:119-160`): `getrandom` für die Nonce (ein Systemaufruf),
  neuer `Vec`, Kopie des Klartexts, `Aes256Gcm::new_from_slice` (Schlüsselplan je Chunk, :30-31),
  `RwLock`-Lesesperre des Masterkeys (`keys.rs` `MasterkeyCell::with`), dann `write_all` des Chunks auf den
  inneren Schreiber (`content_stream.rs:226-234`), also ein Schreibaufruf je 32 KiB.
  `EncryptingWriter::write` nimmt höchstens einen Chunk-Rest an (:270-284).
- Entschlüsseln je Chunk (`content_stream.rs:130-144`): `read_full` von 32 KiB + 28 Byte vom inneren Leser,
  neuer `Zeroizing<Vec>`, Kopie in den Aufruferpuffer.
- Je Datei: Kopf (40 Byte Inhaltsschlüssel, AES-GCM), Name per AES-SIV mit der Eltern-Ordner-ID
  (`names.rs` `encrypt_name`, NFC-Normalisierung), Ordnerauflösung `dir_for` (`backend_nodes.rs:137-151`) aus
  dem 20-s-Cache (`dir_cache.rs:20`), bei Fehltreffer je Ebene ein Öffnen und Lesen von `dir.c9r`
  (`read_dir_id` :170-177) plus `content_dir` (AES-SIV + SHA-1); `examine` mit `stat`, bei Ordnern
  zusätzlich Auflistung (:243-259).
- `reader_at` für Bereichslesen (`backend_io.rs:175-229`) öffnet den Speicher zweimal (Kopf und Bereich).

### 4.3 Zeitlinien (Schätzung)

| Weg | D1 | D2 | D3 |
|---|---|---|---|
| Engine: lokal → Tresor auf lokaler NVMe | Krypto vernachlässigbar; Kosten = Engine (G1) + innerer Stage-Weg (`DurableFile` mit `fsync`, `vfs/os/shared/local.rs:221-228`) | ≈ 1 GiB / 1,2 GB/s ≈ 0,9 s Krypto, seriell mit dem Schreiben im selben Thread | ≈ 9 s Krypto + Lese- und Schreibwartezeit ≈ 12–15 s gegenüber ≈ 10 s Plattengrenze; `SIV_CTRMAC`-Tresore ≈ 20 s (HMAC 0,5 GB/s) |
| Tresor-Laufwerk, Schreiben (Speicher lokal) | wie §3.3 plus Kopf-Entschlüsselung je Spool-Öffnung | Spool versiegeln, beim Hochladen entschlüsseln und für den Tresor verschlüsseln | Phase 1: Spool versiegeln ≈ 9 s; Phase 2 (beim `close`): entschlüsseln (Host) + 8-KiB-Frames + verschlüsseln (Daemon) ≈ 10 s → **≈ 20 s**; Platte in Phase 2 teils untätig, ein Kern je Phase |
| Tresor-Sync-Job | wie §2 (Chiffretext) plus Phasenbarriere: Phase 2 beginnt erst nach der letzten Aktion von Phase 1 (`apply_reporting.rs:359-428`) | wie §2 | wie §2 |

### 4.4 Vergleich mit dem Maßstab

| Weg | Referenz (A9) | se | Verhältnis | Ursachen | Maßnahmen |
|---|---|---|---|---|---|
| Engine D3 in lokalen Tresor | gocryptfs ≈ Plattengrenze dank paralleler Krypto; Cryptomator Desktop (Java, FUSE/WinFsp) deutlich darunter | 12–15 s gegen ≈ 10 s Grenze | 1,2–1,5× | Krypto seriell im Kopierthread, Schlüsselplan, `getrandom`, Allokation je Chunk | MT1, MT2 |
| Tresor-Laufwerk D3 | Cryptomator Desktop: eine Verschlüsselung je Byte | ≈ 20 s gegen ≈ 10 s | ≈ 2× | F9, F6 | MT3, ML1 |
| Tresor-Sync-Job | rclone/rsync über Chiffretext ohne Phasen | wie §2 plus Barriere | — | Phasenbarriere | MT5 plus §2 |

### 4.5 Maßnahmen

| Nr | Änderung (Ort) | Gewinn | Berührte Invariante und wie sie gehalten wird |
|---|---|---|---|
| MT1 | Je Datenstrom eine `Aes256Gcm`-Instanz (`content_chunks.rs:30-31, 55-57`), Nonces aus einem je Strom mit `getrandom` gesäten CSPRNG oder gesammelt per `getrandom` für viele Chunks (`:127`), Verschlüsseln in einen wiederverwendeten Puffer, mehrere Chunks je innerem Schreibaufruf (≈ 1 MiB) in `EncryptingWriter` (`content_stream.rs:185-301`) und `DecryptingReader` (:95-180). | Krypto-Nebenkosten je Chunk ≈ −5 µs; −31 von 32 Schreibaufrufen | Format 8 unverändert (Nonce zufällig je Chunk, AAD = Chunk-Nummer + Kopf-Nonce); Prüfung des Tags vor Freigabe des Klartexts unverändert. |
| MT2 | Parallele Chunk-Krypto für Dateien ab 4 MiB: Leser → N Krypto-Worker → Schreiber in Reihenfolge (neue Datei `cryptomator/core/content_parallel.rs`). | D3 Engine → Plattengrenze ≈ 10 s | Byteformat und Chunk-Reihenfolge unverändert; Sperre greift weiter je Chunk (`FormatError::Locked`). |
| MT3 | Tresor-Laufwerk mit einer Verschlüsselung: die Spool enthält bereits den Tresor-Chiffretext (Kopf mit dem Inhaltsschlüssel des Tresors); der Upload kopiert Chiffretext ohne Entschlüsseln (`sealed_spool.rs`, `vault/os/shared/spool_key.rs`, `backend_write.rs`). | D3 ≈ 20 s → ≈ 10 s; −2 von 3 AES-Durchläufen | „Kein Klartext auf der Platte“ bleibt. Schlüsseltrennung (eigener Spool-Schlüssel) entfällt: nach dem Sperren ist die Spool weiterhin unlesbar; ausstehende Uploads brauchen erneutes Entsperren (heute ebenso, da der Tresor-Schreiber gesperrt fehlschlägt). Sicherheitsentscheidung nötig. |
| MT4 | Entschlüsselten Kopf je geöffneter Datei zwischenspeichern (`backend_io.rs:175-229`). | −1 Speicherzugriff je Bereichslesen | Identitätsbindung: Kopf gilt für dieselbe Speicher-Identität. |
| MT5 | Tresor-Sync-Phasen ordnerweise freigeben statt globaler Barriere (`apply_reporting.rs:359-428`, `sync_rules.rs:368-440`, `held_back` ab :385). | Ende von Phase 1 überlappt Phase 2 | R4 je Inhaltsordner: eine Knoten-Aktion startet erst nach allen Phase-1-Aktionen ihres Ordners. |

---

## 5 ZIP

### 5.1 Wege

| Weg | Ablauf | Threads |
|---|---|---|
| ZIP öffnen und durchsuchen | `ZipBackend::open` (`zipfs/os/shared/zipfs.rs:126-160`) liest das Zentralverzeichnis einmal; `parallelism() = 1` (:248), `transfer_ceiling = Kernanzahl` (:254-259) | Aufrufer |
| Datei aus dem ZIP kopieren (Engine) | `open_read` (:213-227) → `entry::open` (`entry.rs:204-228`): bis 256 KiB in den Speicher, sonst Thread `zip-read` je Eintrag mit `bounded(1)`-Kanal und neuem 256-KiB-`Vec` je Stück (:166-200); jeder Leser öffnet die Archivdatei neu (`ArchiveFile::clone`, :37-45) | 1 je großem Eintrag |
| Entpacken Desktop | nur Windows (`app/os/shared/remote_zip.rs` `start_zip_extract`, `#[cfg(windows)]`) → `extract_all` (`zipfs.rs:275`) → `extract_replacing` (`extract_replace.rs:41-78`): `ZipArchive` auf ungepufferter `File`, je Eintrag `by_index`, `linked_component` (`lstat` je Komponente), `create_dir_all`, `File::create`, `io::copy` mit 8 KiB; kein `fsync` | 1 (`zip-extract`) |
| Entpacken Android | `mobile/os/shared/import.rs:263` → `extract_all_controlled` (`extract.rs:53-147`): Vorlauf über alle Einträge, je Datei `create_dir_all`, `create_new`, 256-KiB-Puffer, Fortschritt je Block, **`sync_all` je Datei** (:184) | 1 |
| ZIP erstellen | im Produkt nicht vorhanden; `ZipWriter` nur in Tests (`zipfs.rs:289, 301`, `extract_replace.rs:104`) | — |

Inflate: `zip` 9 mit `deflate-flate2`, `flate2` mit miniz_oxide (`native/Cargo.toml:173, 194`).

### 5.2 Zeitlinien und Vergleich (Schätzung, deflate-komprimiert)

| Datensatz | Desktop (Windows) | Android | Referenz (A9) | Verhältnis |
|---|---|---|---|---|
| D1 | je Eintrag ≈ 0,3 ms (`CreateFile`, kleine Lesezugriffe, `lstat`-Kette) → ≈ 1,5 s | je Datei `fsync` 3–10 ms → 15–50 s | 7-Zip ≈ 1 s | Desktop 1,5×; Android 15–50× |
| D2 | Inflate 1 GiB / 0,3 GB/s ≈ 3,4 s + 131 072 Schreibaufrufe à 8 KiB ≈ 0,4 s → ≈ 4 s | ≈ 4 s + 1000 `fsync` ≈ 5–14 s | 7-Zip ≈ 2–3 s; parallele Rohgrenze 4 × 0,3 GB/s ≈ 0,9 s | 1,5× bzw. 4× |
| D3 | ein Deflate-Strom, nicht teilbar: ≈ 34 s (miniz_oxide) | ≈ 34 s + `fsync` | 7-Zip ≈ 15–20 s; zlib-rs ≈ 17 s | ≈ 2× |

### 5.3 Maßnahmen

| Nr | Änderung (Ort) | Gewinn | Berührte Invariante und wie sie gehalten wird |
|---|---|---|---|
| MZ1 | Paralleles Entpacken über Einträge (`extract_replace.rs:41-78`, `extract.rs:53-147`): Pool mit Kernanzahl Workern, je Worker eigener `ZipArchive`-Klon (`entry.rs:37-45`), Ordner vorab aus dem Zentralverzeichnis, 1-MiB-Puffer statt `io::copy` (:73), `BufReader` auf der Archivdatei. | D2 4 s → ≈ 1 s | Zip-Slip-Schutz (`enclosed_name`), Link-Prüfung je Eintrag (geprüfte Ordner je Lauf zwischengespeichert), Ersetzen (Desktop) bzw. nie Ersetzen (Android `create_new`) bleiben. |
| MZ2 | Android: kein `sync_all` je Datei (`extract.rs:184`), stattdessen keiner oder ein `syncfs` am Ende. | D1 15–50 s → ≈ 2 s | Dauerhaftigkeit je Datei entfällt; ein Absturz hinterlässt heute schon Teildateien (Entfernen nur bei Fehler) — Produktentscheidung. |
| MZ3 | `flate2` mit zlib-rs (reines Rust, keine C-Abhängigkeit) statt miniz_oxide (`native/Cargo.toml:194`). | Inflate ≈ 2× (A2) | Windows-gnu-Build ohne C-Abhängigkeit bleibt. |
| MZ4 | Engine-Leser: Puffer wiederverwenden, Kanaltiefe 2–4 (`entry.rs:166-200`). | weniger Nullung und Wartezeit | keine |
| MZ5 | ZIP erstellen: kein Weg vorhanden, nichts zu beschleunigen. Falls es kommt: Deflate je Eintrag parallel, Schreiber in Reihenfolge. | — | — |

---

## 6 Gesamtreihenfolge der Maßnahmen

| Rang | Maßnahmen | betroffene Wege | erwarteter Gewinn (Schätzung) | Abhängigkeit |
|---|---|---|---|---|
| 1 | MS1, MS2, MS3 | Sync lokal und entfernt, Linux, Windows, Android | lokal D1 8–20× schneller | keine |
| 2 | ML1, ML2, ML7 | Laufwerk-Schreiben alle Plattformen | D1 20–30× | Produktentscheidung zu `close`-Semantik |
| 3 | MS5 | Sync alle Ziele | Worker-Stillstände entfallen | keine |
| 4 | MS6, MS7, MS8, MS9 | Sync zu SFTP/Share | D1 LAN 5–7×, WAN bis 25× | MS9 nach Vorschlag B aus `analyse-kopieren.md` |
| 5 | MS4 | Sync lokal große Dateien | D3 2× bzw. > 100× mit Reflink | keine |
| 6 | ML4, ML5, ML6, ML10 | Laufwerk-Lesen | Startlatenz ≈ 1000×, D1 ≈ 4× | Entscheidung zur Konsistenz beim Streamen |
| 7 | MS11, MS13 | Sync Ersetzen, Löschen, Konflikte | Löschen ≈ 9× weniger Synchronisierungen | keine |
| 8 | MT1, MT2, MT3 | Tresor-Engine, Tresor-Laufwerk | 1,2–2× | MT3: Sicherheitsentscheidung |
| 9 | MZ1, MZ2, MZ3 | ZIP | 2–25× | MZ2: Produktentscheidung |
| 10 | MS10, MS12, MS14–MS18, ML3, ML8, ML9, MT4, MT5, MZ4 | Rest | kleiner bzw. fallabhängig | MS14 und ML3: Entscheidungen |

---

## 7 Messpunkte für G5

| Weg | Messung | Erwartung vorher (diese Analyse) |
|---|---|---|
| Sync lokal D1/D2/D3 (Linux) | `strace -f -c` des Daemon-Laufs; Zahl der `openat` auf `/proc/self/mountinfo` und `/sys/dev/block` je Datei; `fsync`/`fdatasync`/`syncfs`-Zahl; Zeitreihe der Bytes je 100 ms | ≈ 1040 Systemaufrufe, ≈ 16 Mountinfo-Lesungen, 2 `syncfs` + 1 `fsync` je 64 Dateien |
| Sync lokal (Windows) | ETW/Procmon-Zählung `CreateFile`, `FlushFileBuffers`; mit und ohne Defender-Ausnahme | ≈ 9 ms je Datei |
| Sync zu entfernt | verzögernder Test-Backend-Wrapper (`bisync/os/shared/test_remote.rs` als Vorlage) zählt `stat`, `statvfs`/`Capabilities`, `mkdir`, `open`, `rename` je Datei; `tc netem` 1 ms und 30 ms | ≈ 43 (SFTP) bzw. ≈ 34 (Share) Anfragen je Datei |
| Spiegel-Ende | Dauer von `bootstrap_run` getrennt | 2 Scans + 4 × Einträge Anfragen |
| Laufwerk Linux | `cp -r` von D1/D2/D3 auf und von einem FUSE-Laufwerk mit SFTP-Container; Dauer von `close(2)`; Zeit bis zum ersten Byte | `close` ≈ Upload-Dauer; erstes Byte nach ganzem Download |
| Laufwerk Windows | Explorer-gleiches `CopyFile2` auf ein Dokany-Laufwerk mit Share-Paar | wie Linux, Gate 8 |
| Tresor | Mikro-Benchmark `EncryptingWriter`/`DecryptingReader` mit 1 GiB; Engine-Kopie D3 in Tresor; Tresor-Laufwerk D3 | ≈ 1,2 GB/s je Strom; Laufwerk ≈ 2× Engine |
| ZIP | `extract_all` und `extract_all_controlled` mit D1/D2/D3; 7-Zip auf denselben Daten | §5.2 |

---

## 8 Gelesene Dateien (Baseline-Kopie)

`docs/ARCHITEKTUR.md`. daemon/os/shared: `job.rs`, `own_writes.rs` (73-114), `mount_client_host.rs`
(1-80, 180-330), `mount_proxy.rs` (1-60), `mount_request_gate.rs` (1-200), `ipc.rs` (285-350),
`backend_server.rs` (1-40, 124-200), `backend_ranges.rs` (30-60), `rooted_backend.rs` (1-270),
`rooted_backend_case.rs` (1-182), Zeilen mit `parallelism` in `ipc_protocol_mount_capabilities.rs`.
bisync: `mod.rs`, os/shared/{`orchestration`, `orchestration_full`, `snapshot_pair`, `snapshot`,
`snapshot_walk`, `snapshot_dir`, `snapshot_hash`, `snapshot_policy`, `snapshot_mounts`,
`snapshot_duplicates` (1-80), `apply`, `apply_pool`, `apply_reporting`, `apply_actions`, `apply_groups`,
`apply_transaction`, `apply_transfer`, `apply_stage`, `apply_guard`, `apply_boundary`, `apply_dirs`,
`apply_delete` (1-60), `apply_mirror` (1-91), `transfer_stream`, `stream_hash`, `replacement_publish`,
`checkpoint_run`, `checkpoint_journal`, `version_save`, `version_manifest` (100-230),
`version_provider_policy` (1-25), `sync_flows`, `sync_overload` (1-60), `run_log` (1-140, 282-310),
`incremental` (1-140), `incremental_index_commit` (31-61), `engine_change_feed` (189-260),
`state_store` (1-30)}; core/{`types` (330-410), `plan_dirs`}. syncjobs/core/types.rs (190-370).
sync/{`mod.rs`, os/shared/`sync.rs` (1-80), `sync_pass.rs` (1-60)}. connect/os/shared/resolution.rs;
connector.rs (404-450). vfs: core/{`extensions` (85-200), `extension_calls` (50-215), `cache_extensions`
(40-115), `promotion` (1-167), `core` (Zeilen zu `parallelism`, `try_exists`, `concurrent_read_write`,
`namespace_identity`), `fs_profile` (Zeilen 11-113)}, os/shared/{`local` (85-400), `local_extensions`,
`local_stage`, `local_dirs`, `local_optical`}, os/linux_os/{`local_platform` (55-188), `mountinfo`,
`namespace_flush`, `stage`}, os/windows/{`local_platform` (100-240), `local_writes` (Zeilen 12-140),
`volume_info` (56-90)}. source_media/{`mod.rs` (Zeilen 1-96), `reader.rs` (10-40), `permit.rs`
(195-220), os/`linux.rs`, os/`windows.rs` (23-51)}. local_access/os/linux_os.rs (23-170).
creds/os/{`private_storage_unix.rs` (1-110), `private_storage_windows.rs` (1-75)}. support_dirs.rs (1-20,
196-210). apptrash/mod.rs (32-63). transfer: core/{`flow_control` (20-60), `flow_window` (Kopf)},
os/shared/{`flow.rs` (444-474), `flow_registry.rs` (65-90), engine/`folders.rs` (193-233)}.
copy/os/shared/server_copy.rs (23-53). sftp/core/{`backend.rs` (355-380, 428-460), `extensions.rs`
(165-240), `transfer_ops.rs` (54-87), `io_adapters.rs` (161-195)}. share/core/{`backend.rs` (420-447),
`peer_extensions.rs` (110-130)}. agent/core/{`stream.rs` (Zeilen zu `lz4` und 165-191), `backend.rs`
(100-120)}; agent_proto/core/{`types.rs` (Konstanten), `credit.rs` (Konstanten), `features.rs` (Zeilen
zu `LABEL_LZ4`)}. mount: `mod.rs` (1-80), core/{`engine.rs` (1-160), `engine_recovery.rs` (40-80),
`file_io.rs`, `materialization.rs`, `range_read.rs`, `file_commit.rs`, `file_commit_stage.rs`,
`commit.rs`, `metadata.rs` (16-60), `spool.rs` (142-250), `journal.rs` (61-160), `stage_ledger.rs`
(60-140), `path_sealer.rs`, `cache_policy.rs` (1-60), `types.rs` (240-300, Zeilen zu
`RangeReadPolicy`)}, os/linux_os/{`fuse_ops_io.rs`, `fuse_workers.rs`, `fuse_runtime.rs` (1-120),
`fuse_host.rs` (1-60, Zeilen 184-199), `fuse_fs.rs` (230-300, Zeile 34), Zeile 29 von `fuse_ops_meta.rs`},
os/windows/{`callbacks_io.rs` (1-140), `callbacks_mutation.rs` (106-205), `callback_status.rs` (29-61),
`host.rs` (330-360), Zeilen 53-74 von `dokany_abi.rs`}. mobile/os/shared/domains/docs_io.rs (1-60,
Zeilen 190-260). android/app/src/main/java/app/smartexplorer/android/system/documents/{`ProxyFileCallback.kt`,
Zeilen 140-146 von `SeDocumentsProvider.kt`}. cryptomator/core/{`content_stream.rs`, `content_chunks.rs`
(1-160), `keys.rs` (1-130), `cipher_combo.rs`, `names.rs`, `dir_ids.rs`}. vault/os/shared/{`backend_io.rs`,
`backend.rs` (77-270), `backend_nodes.rs` (130-272), `backend_write.rs` (1-100 und Funktionsliste),
`backend_extensions.rs` (114-152), `backend_identity.rs`, `dir_cache.rs`, `sealed_spool.rs` (1-80),
`sync_job.rs` (1-60, 230-300)}; vault/core/sync_rules.rs (1-50, Zeilen zu Phasen). zipfs: alle Dateien
außer Tests. app/os/shared/remote_zip.rs. analytics/core/tree_deflate.rs (1-20). native/Cargo.toml
(160-200), native/Cargo.lock (Einträge `flate2`, `zip`).

## 9 Unsicher oder nicht verifiziert

1. Alle Zeit-, Durchsatz- und Systemaufrufzahlen sind Schätzungen nach A1–A10; die Zählung der
   Systemaufrufe je `S`, `M`, `P`, `B` ist aus dem Code abgeleitet, nicht mit `strace` gezählt.
2. Ob der Daemon auf der Loopback-Verbindung des Laufwerk-Hosts `lz4-v1` ankündigt und damit jeden
   Datenblock komprimiert: `backend_ranges.rs:47-58` kündigt es an, wenn das Backend Bereichslesen kann;
   ob `serve_backend` für Laufwerke diese Kennungen sendet, wurde nicht bis zum Ende verfolgt. Wenn ja,
   kostet es CPU ohne Nutzen (Maßnahme: für Loopback abschalten).
3. Wann `revalidate_root` für welche Backends gilt (`root_confinement` je Backend), wurde nicht je Backend
   geprüft; die RT-Zahlen in §3.3 sind deshalb als Spanne (ohne/mit Wurzelprüfung) angegeben.
4. Effektive FUSE-Schreibgröße: `set_max_write(1 MiB)` wirkt nur, wenn der Kernel `max_pages` gewährt
   (ab Linux 4.20) und `fuser` es anfordert; nicht geprüft. Ohne das gelten 128 KiB.
5. Schreibgröße von AppFuse (Android `ProxyFileDescriptorCallback`) und Zahl der Rückrufe je MiB: nicht
   geprüft.
6. Ob `FolderRegister` im Sync-Apply bei lokalen Seiten `create_dir` des `LocalBackend` mit Link-Ablehnung
   nutzt: aus `apply_reporting.rs:148-157` (`Side::Remote(endpoints.a)`) und `local.rs:318` geschlossen,
   `FolderRegister::create` selbst nicht gelesen.
7. SFTP-Durchsatz eines einzelnen Stroms (D3) und das Pipelining von `russh-sftp` gehören zu G3 und sind
   hier nur als Spanne eingesetzt.
8. Kosten der Windows-Dokany-Rückrufe durch Cache-Manager-Paging-I/O und Defender-Scans der Spool-Dateien
   sind nicht aus dem Code bestimmbar.
9. Verhalten von `replica::identify` beim Erstlauf (ob Markierungsdateien auf beiden Seiten geschrieben
   werden) wurde nicht gelesen; die Startphase ist pauschal mit 30 ms angesetzt.
10. Die Referenzwerte (rsync, robocopy, rclone, cp, 7-Zip, Cryptomator Desktop, gocryptfs, cryfs) sind
    Erfahrungswerte (A9) und werden erst in G5 auf denselben Daten gemessen.
