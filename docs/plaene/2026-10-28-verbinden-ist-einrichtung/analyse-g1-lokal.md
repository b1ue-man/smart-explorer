# Analyse G1: jeder lokale Übertragungsweg, Ablauf, Engpässe, Zielentwurf

Stand: 2026-10-28. Quellstand: eingefrorene Kopie von `a67d7296`. Teil G1 des
Analyseplans `analyse-plan.md`. Baut auf `analyse-kopieren.md` (erster Durchgang)
auf, prüft ihn und vertieft ihn; Abweichungen stehen in Abschnitt 0.2.

Methode: nur statisch (Code gelesen, `file:line` relativ zu `native/src/`). Nichts
gebaut, gestartet oder gemessen. Jede Zahl zu Zeit, Systemaufrufen oder Dateien/s
ist eine **Schätzung** aus dem Code und den Grundannahmen in 0.3; die einzige
Messung ist die vorhandene Basismessung (`se cp -r` lokal→lokal 5000 × 4 KiB in
0,85 s, `cp -r` 0,27 s, Linux, 4 Kerne). Annahmen sind als **(Annahme)**
markiert. Die Messung selbst gehört zu G5 (Abschnitt 11 nennt die G1-Fragen).

Kürzel:
- **DW** = ein dauerhafter privater Schreibvorgang `support_dirs::write_private_atomic`
  (support_dirs.rs:67-97).
- **PR** = ein privater Lesevorgang `record_io::read_record` (record_io.rs:83-138)
  über `support_dirs::open_private_file` (support_dirs.rs:15-20).
- **Journal** = Absturzprotokoll je Auftrag (`JobRun`, job_store.rs:23-30; Belege
  file_recovery_receipts.rs).
- **GUI-Weg** = Auftrag mit `recoverable: true` (Desktop-Einfügen, Android).
  **CLI-Weg** = Auftrag mit `recoverable: false, id: None` (cli/engine_copy.rs:306-309).

---

## 0. Kurzfassung

### 0.1 Wichtigste Befunde (nach erwarteter Wirkung geordnet)

1. **Journal unter einer Sperre je Auftrag (GUI, Android).** Je neuer Datei hält ein
   Worker die Sperre `JobRun.data` für 1 PR (Wiederaufnahmeprüfung,
   missing_retry.rs:99-108 → file_recovery_receipts.rs:278-281), 2 PR + 3 DW
   (`start_file`, file_recovery_receipts.rs:315-390) und 1 PR (`after_checked`,
   file_recovery.rs:114-116). Der 4. DW (file_recovery.rs:177) läuft ohne Sperre. Jeder
   DW = 2 fsync unter Unix (Datei, Ordner) und auf Windows FlushFileBuffers plus
   `MoveFileExW(REPLACE|WRITE_THROUGH)` (vfs/os/windows/local_writes.rs:105-119).
   Schätzung: Linux-NVMe ≈ 6,5 ms gesperrt je Datei → ≈ 150 Dateien/s je Auftrag
   (Spanne 50–500), Windows ≈ 18 ms → ≈ 55 Dateien/s (Spanne 25–110), Android ≈ 75
   Dateien/s. `cp -r` schafft im selben Fall ≈ 18 500 Dateien/s (gemessen 0,27 s).
2. **Belege kosten außerhalb der fsyncs rund 1000 Systemaufrufe je Datei.** Jeder DW
   läuft 6 Mal den privaten Ordner von `/` aus ab (private_storage_unix.rs:15-102:
   `canonicalize`, `openat`-Kette, 2× `fstat`), jede Belegstempelung macht
   `LocalBackend::stat` (4× lstat plus Optikprobe über sysfs, vfs/os/shared/local.rs:194-204,
   local_optical.rs:17-25) und eine stabile Dateiidentität (Elternordner öffnen,
   2× Eintrag öffnen, 2× `fstat`/`fstatfs`/Handle-Export, linux_file_proof.rs:13-55).
   Schätzung GUI-Weg Linux: ≈ 1050 Systemaufrufe + 8 fsync je 4-KiB-Datei; `cp`: ≈ 10.
3. **Auch der CLI-Weg zahlt Verwaltungsaufwand ohne Nutzen.** `LocalObserver::new`
   (engine/local.rs:63 → file_recovery_observer.rs:17-23) macht je Datei einen
   vollständigen Quell-Schnappschuss (stat + stabile Identität), auch wenn kein
   Journal existiert und der Schnappschuss nie gelesen wird. Dazu je Datei eine
   Optikprobe (`source_media::acquire`, copy/os/shared/staging.rs:100-108; Linux
   `realpath` in `/sys/dev/block` plus bis zu 3 Dateilesungen, source_media/os/linux.rs:30-74).
   Schätzung: ≈ 65 von ≈ 100 Systemaufrufen je kleiner Datei sind dieser Aufwand.
   Das erklärt einen Teil des Abstands 0,85 s zu 0,27 s.
4. **Vor dem ersten Byte steht im GUI-Weg eine zusätzliche Phase.** Jedes Einfügen mit
   Baumlayout geht durch die Ordnerentscheidung (`requires_preview`,
   app/core/folder_conflict_ui.rs:135-139): ein Vorschau-Thread statet jede
   ausgewählte Wurzel mit `LocalBackend::stat` (engine/selection.rs:12-98), und die
   GUI erkennt „fertig“ erst im nächsten Bild (`request_repaint_after(100 ms)`,
   folder_conflict_ui.rs:100). Danach reserviert die Engine jeden ausgewählten Ordner
   mit mindestens 3 DW (engine/folder_reservation.rs:131-300) und schreibt den Auftrag
   mit 3 DW (job_store.rs:77-205, 269-294). Schätzung Zeit bis zum ersten Byte:
   Linux 70–150 ms, Windows 150–400 ms; bei 100 ausgewählten Ordnern + 0,4–2 s.
5. **Der alte CLI-/MCP-Baumweg verschlüsselt jede Datei in einen Temp-Spool.** `se mv`
   über Datenträgergrenzen, `se cp` in ein vorhandenes Ziel oder mit `--force`, der
   MCP-`move` und der MCP-`copy` mit `folder_conflict: keep-both` laufen nicht über
   die Engine, sondern über `copy_entry_with` (cli/tree_ops.rs:107-147): erst jede
   Quelldatei AES-verschlüsselt in ein Temp-Verzeichnis mit `fsync` je Datei
   (cli/tree_spool.rs:92-143), dann einzeln ins Ziel (cli/tree_apply.rs:22-60). Ein
   Thread, dreifache Datenträger-E/A. Schätzung: ≤ ⅓ der Gerätegeschwindigkeit für
   große, 20–300 Dateien/s für kleine Dateien.
6. **Große Einzeldatei: weniger Parallelität als vorgesehen.** Die Zusatzthreads einer
   großen Kopie (`take_extras`, engine/local.rs:200-244) nimmt sie nur von freien
   Erlaubnissen des Flusses; ein neuer lokaler Fluss startet bei 2
   (transfer/core/flow_control.rs:33). Der direkte Linux-Weg (O_DIRECT) läuft dann mit 2
   statt der vorgesehenen 4 Threads (copy/os/linux_direct_plan.rs:20). Schätzung: 0–40 %
   unter Gerätegrenze, abhängig vom Laufwerk.
7. **Windows: gefilterte Kopie für andere Programme läuft über den GUI-Thread.** Das
   virtuelle Datenobjekt und jeder `IStream` entstehen auf dem GUI-STA
   (virtual_clipboard/os/windows.rs:258-283, read_stream.rs:33-41). Explorer liest
   jeden Block per COM-Aufruf, den die GUI-Nachrichtenschleife zustellt. Lange
   relative Pfade (≥ 260 Zeichen) werden stattdessen einzeln, einthreadig und mit
   `fsync` je Datei in einen Temp-Ordner kopiert (snapshot_download.rs:30-47,
   download_file.rs:84).
8. **Verschieben einzelner Dateien auf demselben Datenträger ist ein Kopierpfad mit
   Quarantäne.** Je Datei: Quellumbenennung in ein `.move`-Geschwister
   (copy/os/shared/move_guard.rs:73-133), Hardlink-Alias (direct_move.rs:31-36), Umbenennen
   ins Ziel, 2 Ordner-fsync (durability.rs:5-38) und 4–5 DW Journal. Ganze ungefilterte
   Ordner gehen dagegen in einem Umbenennen (engine/local.rs:274-340).

### 0.2 Korrekturen und Ergänzungen zu `analyse-kopieren.md`

| Aussage im ersten Durchgang | Befund jetzt | Beleg |
|---|---|---|
| (b4) „Für andere Programme bereitstellen“ läuft mit Absturzprotokoll | Falsch: `submit_job_for` setzt für `TransferPurpose::Provide` `recoverable = false` | app/core/transfer_jobs.rs:20-22 |
| CLI-Weg überspringt die Belege bis auf den Schnappschuss | Der Schnappschuss (stat + stabile Identität) läuft je Datei ohne Nutzen; dazu kommt die Optikprobe je Datei | engine/local.rs:63, file_recovery_observer.rs:17-23, staging.rs:100-108 |
| ≈ 5 Ordnerprüfungen je DW | 6 Läufe von `directory()` je DW: `ensure_private_dir` ×3, `directory()` in `open`/`create_file` ×2, `sync_directory` ×1 | support_dirs.rs:67-97, private_storage_unix.rs:15-102, 176-178 |
| 3 DW unter der Sperre | Zusätzlich 3 PR unter derselben Sperre (Wiederaufnahmeprüfung, Index, Beleg) und 1 PR in `after_checked` | missing_retry.rs:99-108, file_recovery_receipts.rs:278-281, 326-328 |
| `se cp` hat keinen Spool | Nur der geeignete Fall (Ziel fehlt, keine Links, kein `--force`) geht über die Engine; alle anderen CLI-/MCP-Kopien und `se mv` über Datenträger nutzen den verschlüsselten Spool | cli/ops_copy.rs:157-183, cli/ops_move.rs:175-190, cli/tree_spool.rs:66-151 |
| Ordneranlage lokal ohne Kosten | Lokale Ordner: `lstat` + `mkdir`; aber jeder ausgewählte Ordner mit „Beide behalten“ kostet ≥ 3 DW Reservierung vor dem ersten Byte | folders_local.rs `ensure_plain_dir`, folder_reservation.rs:131-300 |
| — | Jedes GUI-Einfügen wartet zuerst auf die Ordner-Vorschau | folder_conflict_ui.rs:135-139, selection.rs:244-330 |

### 0.3 Grundannahmen der Schätzungen (alle **Annahme**)

| Größe | Linux (NVMe, ext4) | Windows (NVMe, NTFS, Defender an) | Android (UFS, f2fs + FUSE) |
|---|---|---|---|
| fsync kleine Datei bzw. Ordner | 1 ms (0,3–3 ms) | FlushFileBuffers 2 ms (0,5–5 ms) | 2 ms (0,5–5 ms) |
| Umbenennen mit `MOVEFILE_WRITE_THROUGH` | — | 1 ms (0,1–3 ms; Annahme: erzwingt NTFS-Logflush) | — |
| Pfad-Systemaufruf / CreateFile+Close | 1–3 µs | 20–50 µs (Filtertreiber) | 1–3 µs, FUSE-Aufruf 50–300 µs |
| Kopie 4 KiB im Kernel | 10–20 µs | `CopyFile2` 300–800 µs | 50–300 µs (FUSE) |
| Gerät Lesen/Schreiben/Kopie auf demselben Gerät | 3 / 2 / ≈ 1,1 GB/s | 3 / 2 / ≈ 1,1 GB/s | 1,5 / 0,8 / ≈ 0,5 GB/s |

---

## 1. Einstiege, Prozesse, Threads

### 1.1 Einstiege (vollständig für lokale Quellen und lokale Ziele)

| # | Nutzeraktion | Plattform | Kette bis zur Engine bzw. zum Kopierer | Auftrag |
|---|---|---|---|---|
| E1 | Einfügen im se-Fenster aus eigener Zwischenablage (Strg+V, Menü) | Windows, Linux | `clipboard_paste_with_text` (app/os/shared/clipboard.rs:116-138) → `submit_paste` (app/core/transfer_jobs.rs:72-86) → `paste_job` (app/core/transfer_route.rs:124-138) → `build_job` (249-284: `recoverable: true`, `RecoveryMode::New`, Ordner `KeepBoth`, Dateien `Rename`) → `submit_job_for` (transfer_jobs.rs:16-35) → Ordnerentscheidung (folder_conflict_ui.rs:147-175) → `submit_approved_job_for` → `launch_transfer` (transfer/os/shared/lane.rs:303-328) → `engine::run_job` (lane.rs:147, engine/mod.rs:107-145) | GUI-Weg |
| E2 | Einfügen aus der OS-Zwischenablage (Dateien aus einem anderen Programm) | Windows; Linux nur mit installiertem Desktop-Surface-Anbieter (app/os/linux_os.rs:141-167) | `read_clipboard_files` → `os_paths_selection` → `submit_paste` (clipboard.rs:149-177) → wie E1 | GUI-Weg |
| E3 | Ziehen von außen in das se-Fenster | Windows, Linux | `handle_os_drop` (app/os/shared/drag_drop.rs:22-51) → `submit_paste` → wie E1 | GUI-Weg |
| E4 | Ziehen zwischen Tabs/Bereichen | Windows, Linux | `drop_files_into_tab` (drag_drop.rs:69-124) → `submit_paste` | GUI-Weg |
| E5 | „Kopieren/Verschieben nach…“ | Windows, Linux | `dialog_job` (transfer_route.rs:142-161) → `build_job` (Konflikt aus dem Dialog, Layout Tree/Flatten) | GUI-Weg; Flatten ohne Ordnervorschau |
| E6 | Ausschneiden + Einfügen (Verschieben) | Windows, Linux | wie E1 mit `CopyMode::Move`; nur lokal→lokal erlaubt (transfer_route.rs:260-262) | GUI-Weg, Modus Move |
| E7 | Kopieren in andere Programme, ungefilterte lokale Auswahl | Windows | `write_clipboard_files` CF_HDROP (clipboard.rs:84-105); der fremde Dateimanager kopiert, se nicht beteiligt | — |
| E8 | Kopieren in andere Programme, gefilterte Ansicht | Windows | `start_filtered_preparation` (clipboard_lifecycle.rs:72-118) sammelt rekursiv (`collect_recursive_with_access`, clipboard_lifecycle.rs:283-288) → `set_virtual_clipboard` → Explorer liest `IStream` aus dem se-GUI-Prozess (virtual_clipboard/os/windows.rs:116-136, read_stream.rs) | kein Engine-Auftrag |
| E9 | Wie E8 mit relativem Pfad ≥ 260 UTF-16-Einheiten | Windows | `materialize_long_clipboard` (clipboard_materialize.rs:10-47) → `download_clipboard_snapshot` (transfer/os/shared/snapshot_download.rs:8-61) → `download_file_progress` je Datei (download_file.rs:9-113) | kein Engine-Auftrag |
| E10 | Ziehen aus se in ein anderes Programm, lokal | Windows; Linux nicht unterstützt (app/os/linux_os.rs:198-200) | `drag_out_files` (drag_drop.rs:197-208); der fremde Dateimanager kopiert | — |
| E11 | „Für andere Programme bereitstellen“ | Windows | nur mit entfernter Quelle (clipboard_provide.rs:21-28) → Engine entfernt→lokal-Temp, `recoverable = false` (transfer_jobs.rs:20-22); entfernter Teil gehört zu G2/G3 | Engine ohne Journal |
| E12 | Android: Kopieren/Verschieben | Android | `fs.transfer` (mobile/os/shared/transfer.rs:29-48) → Task-Thread → `run` (162-176) → `plan_job` (211-240, `recoverable: true`) → `drive::run_job` → `launch_transfer` (mobile/os/shared/drive.rs:91-100) → Engine; Fortschritt über `drain_transfer` (drive.rs:173-213) | GUI-Weg (Journal) |
| E13 | `se cp` (geeigneter Fall: Ziel fehlt, kein Link, kein `--force`) | alle | `copy_observed_in_job` (cli/ops_copy.rs:83-192) → `engine_copy::try_copy` (cli/engine_copy.rs:119-212) → `run` (299-352: `recoverable: false`, `Conflict::Skip`, Ordner `Merge`) → `run_view` | CLI-Weg |
| E14 | `se cp` sonst (vorhandenes Ziel, `--force`, Link als Wurzel) | alle | `copy_entry_with` (cli/tree_ops.rs:107-147): Plan → `SourceSpool::collect` (verschlüsselt, fsync je Datei) → `apply_transfer` | alter Baumweg |
| E15 | `se mv` | alle | `try_rename_fast` (cli/transfer.rs:98-166, ein Umbenennen im selben Namensraum) sonst `copy_entry_with` + `remove_copied_source_verified` (cli/ops_move.rs:175-200) | Umbenennen oder alter Baumweg |
| E16 | MCP `copy` | alle | `transfer_tools::start` (cli/mcp/transfer_tools.rs:22-63) → `copy_observed_with_folders`; Standard `merge` → E13/E14; `keep-both` → Reservierung legt den Zielordner an (folder_reservation.rs:393-432), danach ist E13 nicht mehr geeignet (Ziel existiert) → E14 | wie E13/E14 |
| E17 | MCP `move` | alle | `move_observed_with_folders` → wie E15 | wie E15 |

Varianten je Einstieg: gleicher Datenträger, anderer Datenträger, Wechseldatenträger
FAT/exFAT, Netzlaufwerk/UNC als lokaler Pfad, eingehängtes Tresorlaufwerk als lokaler
Pfad. Abschnitt 6 behandelt jede Variante.

### 1.2 Prozesse

| Weg | Prozess, in dem die Bytes bewegt werden |
|---|---|
| E1–E6 | GUI-Prozess selbst: `launch_transfer` startet den Thread `transfer-job` im GUI-Prozess (lane.rs:317-320). Der Worker-Daemon ist nicht beteiligt. |
| E8 | GUI-Prozess (COM-Server auf dem GUI-STA) und Explorer-Prozess (Kopiermaschine des Explorers). |
| E9 | GUI-Prozess, Thread `clip-long-paths` (clipboard_materialize.rs:23-30). |
| E7, E10 | fremder Dateimanager. |
| E12 | Android-App-Prozess (JNI-Bibliothek), Task-Thread der Runtime. |
| E13–E17 | `se`-Prozess (CLI) bzw. `se mcp`-Prozess (Job-Thread `spawn_worker`, cli/mcp/jobs.rs:295). |

### 1.3 Threads je Engine-Auftrag (E1–E6, E11–E13, E16-Engine)

| Thread | Anzahl | Lebensdauer | Quelle |
|---|---|---|---|
| `folder-conflict-preview` (nur GUI, nur Layout Tree mit Wurzeln) | 1 | bis Vorschau fertig bzw. abgebrochen | selection.rs:251-330 |
| `transfer-job` (Vorbereitung, Verteiler, Fortschritt) | 1 | ganzer Auftrag | lane.rs:317-320, dispatch.rs:101-158 |
| `transfer-discovery` | 1 | bis der Lauf durch die Auswahl endet | dispatch.rs:103-105 |
| Lister-Threads des Walkers | bis `flow.snapshot().limit` der Quelle (Start 2) | solange Ordner anstehen | walk.rs:168-172 |
| `transfer-worker` | 1 … Grenze des Flusses (Start 2, Decke 256) | entsteht bei Bedarf, endet nach 250 ms ohne Arbeit | dispatch.rs:122-136, mod.rs:100 |
| `copy-range-N` (Linux, große Datei) | 1 + Zusatzerlaubnisse je großer Datei (maximal 4 direkt) | eine Datei | copy/os/linux_range.rs:112-131 |
| `copy-read` (Windows, dauerhafte Kopie: Verschieben über Datenträger, Überschreiben) | 1 je Datei | eine Datei | copy/os/shared/pipe_copy.rs:52-70 |
| Sensor-Thread | 1 je Prozess | Prozess | engine/mod.rs:281-282 |
| GUI-Hauptthread | 1 | zeichnet bei laufender Übertragung alle 50 ms (frame_update.rs:197-229) | |
| Windows: `notify`-Watcher-Thread über alle Laufwerke rekursiv | 1 | GUI-Laufzeit | app/os/windows/watchers.rs:58-114 |

Prioritäten: Worker, Range- und Lesethreads senken sich auf nice 5 (Linux), nice 10
(Android), `THREAD_PRIORITY_BELOW_NORMAL` (Windows) (transfer/os/unix.rs:10-15, 67-81;
transfer/os/windows.rs:69-83; worker.rs:25). Die E/A-Priorität bleibt normal.

### 1.4 Warteschlangen, Kanäle, Puffer

| Objekt | Kapazität | Weckverhalten | Quelle |
|---|---|---|---|
| `WorkQueue` (Dateien vor Ordnern) | 16 MiB Pfadtext + 96 B je Eintrag | ein `Condvar`, `notify_all` bei jedem `push` und `pop` (weckt Verteiler und alle wartenden Worker) | engine/queue.rs:19-21, 162-218 |
| `TransferMsg`-Kanal | unbegrenzt (`crossbeam_channel::unbounded`) | Fortschritt alle 150 ms, ein `Done` | lane.rs:304, mod.rs:96 |
| Walker-Warteschlange | unbegrenzt (`VecDeque<Task>`) | `Condvar`, Koordinator 50 ms | walk.rs:110-115, 25 |
| Fluss (`Flow`) | Grenze 2…256 + 1 Metadatenplatz | `notify_all` bei jeder Freigabe | flow.rs:175-245, 365-390 |
| Speicherbudget | ¼ des verfügbaren Speichers, 256 KiB je lokaler Operation | Sperre + Condvar | engine/ops.rs:294-299, transfer/os/shared/memory.rs |
| Worker-Puffer | 256 KiB je Worker (`vec![0; COPY_BUFFER]`), lokal→lokal ungenutzt | einmal je Worker | worker.rs:26, ops.rs:17 |
| Kernel-Kopierstück Linux | 8 MiB je `copy_file_range`-Aufruf | — | copy/os/linux_os.rs:298 |
| Direkte Linux-Kopie | 4 MiB ausgerichtet je Thread | — | linux_direct_plan.rs:13, 23 |
| Windows-Handlekopie | 4 × 1 MiB Ring, 1 Lesethread | — | copy/os/windows.rs:220-225 |
| Windows-Optikprobe | 64 KiB (`vec![0u16; 32768]`) + 2 KiB je Aufruf | — | source_media/os/windows.rs:31, 37 |

---

## 2. Aufrufkette lokal→lokal Kopieren, Schritt für Schritt

Spalten: GUI = mit Journal, CLI = ohne Journal. Systemaufrufe für Linux geschätzt,
Windows als Win32-Aufrufe.

### 2.1 Vorphase (einmal je Auftrag)

| Schritt | Ort | Linux | Windows | GUI | CLI |
|---|---|---|---|---|---|
| V1 Einfügen löst Auftrag aus, `new_job_id` | transfer_route.rs:249-284 | — | — | ja | — |
| V2 Ordnervorschau: Thread, je ausgewählter Wurzel `LocalBackend::stat` (4× lstat + Optikprobe), je Ordner zusätzlich `target.stat` und Namenssuche „Name (n)“ | selection.rs:12-98, 244-330 | ≈ 25 Aufrufe je Wurzel | ≈ 3 CreateFile/GetFileAttributes + `GetVolumePathNameW` + 64-KiB-Puffer je Wurzel | ja | nein |
| V3 GUI erkennt „fertig“ im nächsten Bild (bis 100 ms) | folder_conflict_ui.rs:26-36, 100 | — | — | ja | nein |
| V4 `launch_transfer`, Thread `transfer-job` | lane.rs:303-328 | clone + 1 Thread | 1 Thread | ja | (CLI ruft `run_view` direkt) |
| V5 `run_view`: erster Fortschritt, `validate`, `prepare` | engine/mod.rs:122-145, prepare.rs:10-57 | | | ja | ja |
| V6 `job_store::preflight`: Tresorkontext + PR (nicht vorhanden) | job_store.rs:207-260 | ≈ 50 | ≈ 30 CreateFile | ja | übersprungen |
| V7 Zielwurzel: `prepare_local_root` = `ensure_plain_dir` je Pfadkomponente | folders_local.rs `prepare_local_root` | 1 lstat je Komponente | 1 GetFileAttributes je Komponente | ja | ja |
| V8 `job_store::begin`: privater Ordner, Sperrdatei anlegen + öffnen + `try_lock`, PR (fehlt), Zielordner-Identität, Schnappschuss jeder Quellwurzel, **DW Auftrag**, `initialize` = **DW Index** | job_store.rs:77-205, file_recovery_receipts.rs:88-114 | ≈ 450 + 4 fsync | ≈ 250 CreateFile + 2 Flush | ja | übersprungen |
| V9 Wurzelplan: je ausgewähltem Ordner mit „Beide behalten“ `Reservations::open` + `reserve` = ≥ 3 DW + Stage-`mkdir` + Identität + Umbenennen; je Wurzel `selected_directory` (lstat); `check_local_targets` zweimal | roots.rs:118-327, folder_reservation.rs:121-300 | ≥ 3 DW je Ordner | ≥ 3 DW je Ordner | ja | nein (Merge) |
| V10 `prepare_apply`: `mutation_started` = **DW Auftrag** | job_store.rs:269-294 | 1 DW | 1 DW | ja | nein |
| V11 `Engine::new`: Flüsse holen (Registry-Sperre), Sensoren | engine/mod.rs:219-284 | gering | gering | ja | ja |
| V12 `move_whole_roots` (nur Verschieben) | engine/local.rs:274-340 | | | Move | — |
| V13 `run`: Discovery-Thread, erster Worker sofort (`workers == 0`) | dispatch.rs:101-136 | 2 Threads | 2 Threads | ja | ja |

Der Auftragsdatensatz enthält alle ausgewählten Pfade (`items: view.items.clone()`,
job_store.rs:60) und wird bei V8, V10 und Abschluss vollständig neu geschrieben.

### 2.2 Discovery (eigener Thread, überlappt mit den Workern)

`discover` (discovery.rs:16-24) → `walk` (walk.rs:133-176): Lister-Threads bis zur
Grenze des Quellflusses, jede Auflistung unter einer Metadatenerlaubnis desselben
Flusses wie die Kopien. `LocalLister::list` (walk_listers.rs:78-125): je Ordner eine
Optik-Führung (`acquire_current`, Probe), `read_dir` + `statx` je Eintrag. Je Datei
`queue.push` (Sperre + `notify_all`). Ordner gehen in eine eigene Spur und werden nur
genommen, wenn keine Datei wartet (queue.rs:6-9, 204). Discovery begrenzt lokal nicht:
5000 Einträge in einem Ordner ≈ 10–20 ms (Schätzung).

### 2.3 Je Datei (Worker), Kopieren, neue Datei, Konflikt „Rename“ (GUI) bzw. „Skip“ (CLI)

| # | Schritt | Ort | Linux-Systemaufrufe (Schätzung) | Windows | Sperre | GUI | CLI |
|---|---|---|---|---|---|---|---|
| W1 | `queue.pop` | queue.rs:196-218 | Futex | — | Warteschlange | ja | ja |
| W2 | Elternordner: Register-Treffer; erster Treffer legt an (`lstat` + `mkdir`) unter Metadatenerlaubnis | worker.rs:66-77, folders.rs:191-247, folders_local.rs | 0 bzw. 2 | 0 bzw. 2 | Register | ja | ja |
| W3 | Speicher reservieren (256 KiB Budget) | worker.rs:90-94 | — | — | Budget | ja | ja |
| W4 | Erlaubnis(se): ein Fluss bei gleichem Datenträger, zwei bei verschiedenen | worker.rs:96, flow.rs:444-473 | Futex | — | Fluss | ja | ja |
| W5 | `stats.begin` (String-Kopie), `flows_have_spare` + `notify` | worker.rs:102-108, stats.rs:253-260 | Futex | — | Stats, Fluss | ja | ja |
| W6 | Wiederaufnahmeprüfung: **PR** des Belegs (fehlt) | ops.rs:270, missing_retry.rs:99-108, file_recovery_receipts.rs:278-305 | ≈ 50 | ≈ 25 CreateFile | **Journal** | ja | übersprungen |
| W7 | Pfade bauen (`native` ×2, `path_of`) | engine/local.rs:30-31 | — | — | Alias | ja | ja |
| W8 | `take_extras`: kleine Datei → nichts | engine/local.rs:211-216 | 0 | 0 | — | ja | ja |
| W9 | `LocalObserver::new` → `snapshot(source)` = `LocalBackend::stat` + stabile Identität | file_recovery_observer.rs:17-23, job_store_validation.rs:30-58 | ≈ 45 | ≈ 15 CreateFile + 2× `GetVolumePathNameW`-Probe-Pfad + Volumeabfragen | — | ja | **ja, ungenutzt** |
| W10 | Quelle `lstat`, Typprüfung | safe_file.rs:80-89 | 1 | 1 | — | ja | ja |
| W11 | Zielwahl: Rename = kein Aufruf; Skip/Overwrite = `lstat` Ziel | safe_file.rs:294-345 | 0/1 | 0/1 | — | 0 | 1 |
| W12 | Optikprobe + Führung (`source_media::acquire`) | staging.rs:100-108, source_media/permit.rs:100-108, os/linux.rs:5-74 | ≈ 20 (sysfs) | `GetVolumePathNameW` + `GetDriveTypeW` + 64 KiB | — | ja | ja |
| W13 | Quelle öffnen ohne Link, Schnappschuss vom Handle | staging.rs:268-273, local_access/os/linux_os.rs:93-120 | 3–4 | 2 | — | ja | ja |
| W14 | Windows: `stage_by_path` → `copy_by_path`: **zweite** Probe, `copy_flags` (`std::fs::metadata` der Quelle, `GlobalMemoryStatusEx`), `CopyFile2(FAIL_IF_EXISTS\|COPY_SYMLINK[\|NO_BUFFERING])`, `open_staged` (öffnen, Metadaten, Klassifizierung), Längen- und Unverändert-Prüfung (Quelle erneut öffnen) | staging.rs:277-322, copy/os/windows.rs:228-272, 375-432, 450-466 | — | ≈ 6 CreateFile + CopyFile2 | — | ja | ja |
| W15 | Linux: Stage `.…smart-explorer-<hex>.part` exklusiv anlegen, `io::copy(Take(8 MiB))` = 2× `statx` + 2× `copy_file_range` (4096, dann 0) | staging.rs:139-145, 443-456, linux_os.rs:324-365 | 5 | — | — | ja | ja |
| W16 | Fortschritt je Stück: Flusssperre + Atomics | ops.rs:210-217, flow.rs:339-345, 412-417 | Futex | — | Fluss | ja | ja |
| W17 | Unverändert-Prüfung (`fstat` + `lstat`), Rechte übernehmen (`fchmod`); kein fsync bei Kopie | staging.rs:154-176, linux_os.rs:226-259 | 3 | 1 SetFileInformation | — | ja | ja |
| W18 | Stage-Identität vom Handle + `lstat` des Stage-Pfads | safe_file.rs:161-175 | 2 | 1 + 1 CreateFile | — | ja | ja |
| W19 | `before_publish`: `stamp(staged)` (stat + Identität), `previous_presence(target)` (lstat, fehlt) | file_recovery_observer.rs:54-77 | ≈ 47 | ≈ 18 CreateFile | — | ja | übersprungen |
| W20 | `start_file`: **PR Index**, `repair_new_entry` (ohne `pending` sofort zurück), **PR Beleg** (fehlt), **DW Index mit `pending`**, **DW Beleg**, **DW Index `count+1`** | file_recovery_receipts.rs:315-390 | ≈ 100 + 3 × 175 + 6 fsync | ≈ 50 + 3 × 80 CreateFile + 3 Flush + 3 WT-Umbenennen | **Journal** | ja | übersprungen |
| W21 | Veröffentlichen ohne Ersetzen: `preflight_no_replace` (lstat) + `renameat2(RENAME_NOREPLACE)`; Windows `MoveFileExW(MOVEFILE_WRITE_THROUGH)`; bei Namenskonflikt „Name (n)“ ab (2) linear | safe_file.rs:186-252, linux_os.rs:271-277, android_fs/os/rename.rs:60-89, copy/os/windows.rs:173-194 | 2 | 1 GetFileAttributes + MoveFileEx (WT) | — | ja | ja |
| W22 | `after_publish_handle` → `after_checked`: **PR Beleg**, `verify(proof)` (stat + Identität), `stamp(target)` (stat + Identität), **DW Beleg** | file_recovery.rs:105-178 | ≈ 50 + 92 + 175 + 2 fsync | ≈ 25 + 36 + 80 CreateFile + Flush + WT | PR unter **Journal**, DW ohne | ja | übersprungen |
| W23 | Handles schließen; Erlaubnis zurück (`finish_with`, Fluss + `notify_all`), Speicher zurück, `file_done` | worker.rs:116-128, 172-178 | 2 + Futex | 2 CloseHandle | Fluss | ja | ja |

Summen je neuer 4-KiB-Datei (Schätzung):

| | Linux CLI-Weg | Linux GUI-Weg | Windows CLI-Weg | Windows GUI-Weg | `cp`/robocopy |
|---|---|---|---|---|---|
| Systemaufrufe bzw. CreateFile | ≈ 100 (davon ≈ 65 W9 + W12) | ≈ 1050 | ≈ 30 CreateFile + CopyFile2 | ≈ 450 CreateFile + CopyFile2 | ≈ 10 bzw. ≈ 5 + CopyFile |
| fsync / Flush | 0 | 8 | 0 (+ 1 WT-Umbenennen, Annahme) | 4 Flush + 5 WT-Umbenennen | 0 |
| Zeit unter Journal-Sperre | 0 | ≈ 6,5 ms | 0 | ≈ 18 ms | — |
| Größere Allokationen | Pfade, `stats`-Name, `FileWork` | zusätzlich 4 × `serde_json` (≈ 1–2 KiB), 4 × Belegdeserialisierung, Identitäts-Strings | zusätzlich 2 × 64 KiB Probe-Puffer, Wide-Strings je API | wie links + Belege | — |

### 2.4 Abschlussphase

`Engine::finish` (engine/mod.rs:381-410): `prune_moved_dirs` (nur Move), `record.finish`
= **DW Auftrag** (job_store.rs:333-346), Fortschritt, `remember_limits` → Hinweisdatei
(**DW** wenn geändert, flow.rs:362-378, hints.rs:218), `Done`. GUI: nächstes Bild
(≤ 50 ms) → `drain_transfers` → bei sichtbarem Zielordner `rescan()` der ganzen Ansicht
(transfer_lifecycle.rs:16-33, scanning.rs:158-167). Belegordner `<id>.files/` und
Auftragsdatei bleiben liegen; im gelesenen Code fand sich keine Löschung
abgeschlossener Aufträge (Suche nach `transfer-jobs`, `remove_dir_all`; unbestätigt).

---

## 3. Kopierprimitive je Plattform

### 3.1 Linux

| Fall | Primitive | Quelle |
|---|---|---|
| Datei < 16 MiB oder Warteschlange nicht leer | `std::io::copy(Take<&File, 8 MiB>, &mut File)` → `copy_file_range` (Kernel; Reflink auf btrfs/XFS mit reflink, serverseitig auf NFS 4.2/CIFS); anderer Datenträger mit Kernel ≥ 5.19 → `EXDEV` → std fällt auf `sendfile` zurück (Annahme zum std-Verhalten) | linux_os.rs:324-365 |
| Datei ≥ 16 MiB, Warteschlange leer, Zusatzerlaubnisse frei, Ziel Flash (`queue/rotational` = 0) | `parallel` = 1 + freie Erlaubnisse. Datei ≥ ¼ verfügbarer Speicher: O_DIRECT über `/proc/self/fd`-Neuöffnung, 4 MiB je Anfrage, ≤ 4 Threads; sonst `copy_file_range` je Bereich parallel | engine/local.rs:200-244, local_plan.rs:89-133, linux_direct.rs, linux_range.rs |
| wie oben, Ziel nicht als Flash bekannt, ≥ 128 MiB, ungepuffert | Messprobe mit 1 und 2 Threads, Urteil je Gerätepaar für den Prozess | linux_probe.rs, local_plan.rs:142-208 |
| Reflink-Dateisystem | `clones()` klont das erste MiB per `FICLONERANGE` und verzichtet dann auf O_DIRECT; `copy_file_range` reflinkt den Rest | linux_direct.rs:256-277 |
| Verschieben, Überschreiben | ein Thread, `sync_all` der Stage, Veröffentlichung, Ordner-fsync | safe_file.rs:128-139, staging.rs:167-172, durability.rs |
| Veröffentlichung | `preflight_no_replace` + `renameat2(RENAME_NOREPLACE)`; bei `EINVAL/ENOSYS/EOPNOTSUPP` Hardlink + Quelle löschen, sonst geprüftes `rename` | linux_os.rs:271-277, android_fs/os/rename.rs:60-89 |

Kein `fsync` je kopierter Datei (wie `cp`, staging.rs:1-5). Jeder fsync im GUI-Weg
stammt aus dem Journal, nicht aus der Kopie.

### 3.2 Windows

| Fall | Primitive | Quelle |
|---|---|---|
| neue Kopie | `CopyFile2(FAIL_IF_EXISTS \| COPY_SYMLINK)` auf den Stagenamen, `NO_BUFFERING` ab ¼ verfügbarem Speicher; SMB-Offload und ReFS-Blockklonen macht `CopyFile2` selbst | copy/os/windows.rs:250-272, 375-432, copy_flags.rs:50-64 |
| geschützte Quelle (nur über `local_access` lesbar), `CopyFile2`-Fehler außer Platz/Existenz | Handlekopie `pipe_copy`: Lesethread + 4 × 1 MiB `ReadFile`/`WriteFile` | staging.rs:306-318, windows.rs:292-309 |
| Verschieben über Datenträger, Überschreiben | Handlekopie (1 Thread je Datei zusätzlich), FlushFileBuffers | safe_file.rs:128-139, staging.rs:167-172 |
| Veröffentlichung | `preflight_no_replace` + `MoveFileExW(MOVEFILE_WRITE_THROUGH)`; Überschreiben `MoveFileExW(REPLACE_EXISTING \| WRITE_THROUGH)` | windows.rs:173-213, vfs/os/windows/local_writes.rs:105-163 |
| Bereiche, Messprobe | gibt es nicht; `copy_probed` = `copy_handles` | windows.rs:274-286, engine/local.rs:126-162 |

### 3.3 Android

Wie Linux, mit diesen Unterschieden:
- `source_media::probe` ist leer (Android nutzt `os/unsupported.rs`), also keine Optikkosten.
- Ziel auf freigegebenem Speicher `/storage/...` ist FUSE (MediaProvider): kein Block-
  gerät (Hauptnummer 0) → keine Bereiche, kein O_DIRECT, Plan `Single`
  (local_plan.rs:107-133); stabile Identität nicht verfügbar (linux_file_proof.rs:31-35).
- Veröffentlichung: `renameat2(RENAME_NOREPLACE)` scheitert auf FUSE ohne `rename2`
  (Annahme für MediaProvider) → Hardlink scheitert → `lstat` + `rename`: 4 FUSE-Aufrufe
  statt 1 je Datei. Das Ergebnis wird nur protokolliert, nicht gemerkt
  (android_fs/os/rename.rs:129-151): jede Datei wiederholt die Leiter.
- Journal liegt in `<filesDir>/smart_explorer` auf `/data` (f2fs oder ext4), fsync auf UFS.

---

## 4. Nebenläufigkeit und Fluss im lokalen Fall

- Ein Fluss je Datenträger: Linux nach `st_dev` (vfs/os/linux_os/local_platform.rs:37-49),
  Windows nach Laufwerksbuchstabe bzw. `unc://server/freigabe`
  (vfs/os/windows/local_platform.rs:61-86). Gleicher Datenträger: eine Erlaubnis je
  Operation (flow.rs:450-455); verschiedene: je eine auf beiden Flüssen.
- Start bei 2 oder der Hälfte des zuletzt gelernten Werts, mindestens 2
  (flow_control.rs:33, flow_window.rs:19-21). Fenster ≥ 100 ms und ≥ 8 Abschlüsse
  (flow_control.rs:35-40). Verdopplung nur bei ≥ 10 % mehr Gutput; Gutput = Bytes +
  64 KiB je Abschluss (flow_control.rs:28, 225-240).
- Verteiler startet einen Worker nur, wenn alle laufen und beide Flüsse eine freie
  Erlaubnis haben (dispatch.rs:122-126).
- **GUI-Weg:** zusätzliche Worker warten an der Journal-Sperre; der Gutput steigt beim
  Verdoppeln nicht → Langsamstart endet nach zwei Fenstern bei 4; die
  Warteschlangenbremse (mittlere Dauer > 2 × Minimum, flow_control.rs:44) verhindert
  weitere Aufwärtsproben. Ergebnis (Schätzung): 2–4 Worker, Durchsatz = 1 /
  gesperrte Zeit je Datei.
- **CLI-Weg:** keine gemeinsame Sperre; Gutput wächst mit Workern, bis CPU oder
  Journal des Dateisystems begrenzen. Unter Linux serialisiert der Elternordner
  (`i_rwsem`) Anlegen und Umbenennen im selben Zielordner (Annahme Kernelverhalten);
  bei ≈ 6000 Dateien/s noch keine Grenze.
- **Große Datei:** Zusatzthreads nur aus *jetzt* freien Erlaubnissen (`try_acquire`,
  engine/local.rs:250-270). Frischer Fluss mit Grenze 2, ein Worker hält 1 → 1 Extra →
  2 Threads statt 4. Das Wachstum des Flusses während der Kopie kommt dieser Datei
  nicht mehr zugute.
- **Metadatenplatz:** Auflistungen und Ordneranlage nutzen den einen reservierten
  Metadatenplatz, wenn die Worker alle Erlaubnisse halten (flow.rs:200-217). Bei tiefen
  Bäumen mit wenigen Dateien je Ordner wird die Ordneranlage dadurch einspurig.
- **Weckaufrufe:** jeder `push`/`pop` und jede Erlaubnisfreigabe ruft `notify_all`
  (queue.rs:174, 216; flow.rs:389). Bei N Workern N + 1 Weckvorgänge je Datei
  (Schätzung: unter 5 µs je Datei bei 8 Workern, kein Engpass).

---

## 5. Varianten des lokalen Pfads

| Variante | Was sich ändert | Engpass zusätzlich zu Abschnitt 2 |
|---|---|---|
| Gleicher Datenträger | ein Fluss; Linux `copy_file_range` innerhalb eines Dateisystems (Reflink auf btrfs/XFS) | Kopierlast und Journal-fsync auf demselben Gerät, wenn das Ziel auf dem Systemlaufwerk liegt |
| Anderer Datenträger | zwei Flüsse (beide Erlaubnisse nötig, flow.rs:456-473); Linux `EXDEV` → `sendfile` | Journal liegt immer auf dem Datenträger der App-Daten, unabhängig von Quelle und Ziel |
| Wechseldatenträger FAT/exFAT Linux | `renameat2(NOREPLACE)` unterstützt (Annahme: vfat/exfat lehnen nur andere Flags ab); stabile Identität nicht verfügbar → Belegzweig ohne Objektbeweis (file_recovery.rs:144-167); vfat mit Mount-Option `flush` (udisks-Standard, Annahme) schläft beim Schließen jeder beschriebenen Datei bis 100 ms im Kernel | Je Worker ≤ 10 Dateien/s bei vfat+`flush`; nur mehr gleichzeitige Worker helfen (Schätzung) |
| Wechseldatenträger FAT/exFAT Windows | `MoveFileExW(WRITE_THROUGH)`; „Schnelles Entfernen“ (Standard) schaltet Schreibcache ab (Annahme) | jede Metadatenänderung synchron; `CopyFile2` je Datei langsam |
| Netzlaufwerk/UNC Windows | eigener Fluss je Freigabe, Start 2; Optikprobe endet früh für UNC (source_media/os/windows.rs:26-28); stabile Identität scheitert erst nach `open_root` über alle Vorfahren (windows_identity.rs:7-27, local_access/os/windows/directory_handle.rs:76-115) | jeder CreateFile/Query ist ein SMB-Roundtrip: CLI-Weg ≈ 15–20 Roundtrips je Datei, GUI-Weg ≈ 40–60 (Schätzung), Explorer ≈ 5 |
| Netzmount Linux (NFS/CIFS) | kein Blockgerät → keine Bereiche; `copy_file_range` serverseitig; NFS schließt mit COMMIT je Datei (Annahme Close-to-open) | Roundtrips je Metadatenaufruf wie oben |
| Eingehängtes Tresorlaufwerk als lokaler Pfad | Zielpfad liegt im se-eigenen FUSE/CfAPI-Host (G4); Belege werden versiegelt, sobald ein Ort im Tresor liegt (record_io.rs:24-71) | Versiegelung je DW (AES, Mikrosekunden) gegenüber fsync vernachlässigbar; Durchsatz des Laufwerkshosts ist G4 |

---

## 6. Verschieben (Ausschneiden/Einfügen, `se mv`, MCP `move`)

| Fall | Ablauf | Kosten je Datei (Schätzung) | Quelle |
|---|---|---|---|
| GUI, ganze ungefilterte Ordner, gleicher Datenträger | eine Umbenennung je Ordner (Reservierung bzw. `move_folder`), danach nicht mehr im Lauf | ≈ 0 | engine/local.rs:274-340 |
| GUI, einzelne Dateien, gleicher Datenträger (Auswahl von Dateien, Zusammenführen, Filter) | Quarantäne: Elternordner halten, Quelle öffnen, Identität, `before_source_quarantine` = `start_file` (2 PR + 3 DW gesperrt), Umbenennen in `.…smart-explorer-<hex>.move`, Prüfungen; direktes Verschieben: Hardlink-Alias, Handle, `before_publish` (2 PR + 1 DW gesperrt), Umbenennen ins Ziel, Prüfung; `finish_direct_move`: fsync Zielordner + fsync Quellordner; `after` (PR + DW) | ≈ 5 DW + 2 Ordner-fsync + 3 Umbenennungen + 1 Link; Linux ≈ 12 ms, Windows ≈ 30 ms | move_guard.rs:73-133, direct_move.rs:18-120, durability.rs:5-38 |
| GUI, anderer Datenträger | Quarantäne wie oben, dann dauerhafte Kopie mit 1 Thread (`parallel` erzwungen 1), `sync_all`, Veröffentlichung, `confirm_handle_publication`, `finish_staged_commit` (Ordner-fsync, Quarantäne löschen, Ordner-fsync), Journal | ≈ 5 DW + 1 Datei-fsync + 2–3 Ordner-fsync; Windows zusätzlich 1 Lesethread je Datei | safe_file.rs:100-271 |
| `se mv`, gleicher Namensraum | eine Umbenennung (`rename_no_replace`) | ≈ 0 | cli/transfer.rs:98-166 |
| `se mv`, anderer Datenträger; MCP `move` ebenso | alter Baumweg: Plan, Spool (verschlüsselt, fsync je Datei), Anwenden, Quelle verifiziert entfernen | 3-fache E/A + AES + 2 fsync je Datei | cli/ops_move.rs:175-200, tree_spool.rs, tree_apply.rs |

---

## 7. GUI-Seite: Kosten und ob sie bremst

| Quelle | Kosten | Bremst die Kopie? | Ort |
|---|---|---|---|
| Ordnervorschau vor dem Start | 1 Thread, `LocalBackend::stat` je ausgewählter Wurzel (inkl. Optikprobe), bei Ordnerkonflikt rekursive Zählung | verzögert das erste Byte: 16–100 ms Bildlatenz + Statzeit (5000 ausgewählte Dateien: Linux ≈ 0,1–0,3 s, Windows ≈ 0,5–2 s; Schätzung) | folder_conflict_ui.rs, selection.rs |
| Neuzeichnen | alle 50 ms, solange eine Übertragung läuft | nur bei CPU-Sättigung: die Kopierthreads haben niedrigere Priorität als der GUI-Thread | frame_update.rs:197-229 |
| Fortschrittsnachrichten | 1 je 150 ms, `poll` liest ≤ 16 je Bild | nein | lane.rs:219-259 |
| Windows-Dateiwächter | rekursiv über alle Laufwerke; je kopierter Datei ≈ 4 Ereignisse (Stage anlegen, schreiben, umbenennen), je DW ≈ 3 Ereignisse im App-Datenordner (Annahme: nicht ausgefiltert); `drain_watcher` holt ≤ 8000 je Bild und ruft für `Create(Any)` `std::fs::metadata` **auf dem GUI-Thread** | GUI-Weg 5000 Dateien ≈ 80 000 Ereignisse, ≈ 20 000 `metadata`-Aufrufe auf dem GUI-Thread (Schätzung 0,5–1 s GUI-Zeit); bremst die Kopie nur über CPU | app/os/windows/watchers.rs:58-114, 158-290 |
| Neuscan nach Abschluss | `rescan()` der ganzen Ansicht, wenn sie das Ziel zeigt | konkurriert mit dem nächsten Einfügen um Datenträger und CPU | transfer_lifecycle.rs:16-33, 154-175 |
| Miniaturbilder, Metadaten neuer Dateien | im Desktop-Code nicht vorhanden (Suche nach „thumbnail“ ohne Treffer in `app/`) | nein | — |
| Gefilterte Kopie für andere Programme (E8) | jeder `IStream::Read` des Explorers ist ein COM-Aufruf in den GUI-STA; zugestellt nur zwischen zwei Bildern | **ja**: Explorer kopiert so schnell, wie die GUI-Nachrichtenschleife Aufrufe annimmt | virtual_clipboard/os/windows.rs:258-283, read_stream.rs:45-66 |

Antwort auf „Kann die GUI oder der Worker die Kopie drosseln?“: Die GUI drosselt
direkt nur E8 (COM über den GUI-Thread) und indirekt den Start (Vorschau + Bildlatenz)
sowie bei CPU-Sättigung (niedrigere Thread-Priorität der Kopie). Der Worker-Daemon ist
an lokalen Kopien nicht beteiligt. Den Durchsatz des GUI-Wegs begrenzt die
Journal-Sperre im eigenen Prozess.

---

## 8. Zeitleisten (Schätzung, Grundannahmen 0.3)

Legende: `#` Mittel ausgelastet, `.` Mittel wartet, `L` Journal-Sperre, `f` fsync.

### 8.1 5000 × 4 KiB in einem ausgewählten Ordner

**Linux GUI-Weg (heute)**
```
t=0      Strg+V
0–100ms  Vorschau-Thread + Bildlatenz             Platte .  CPU .
~100ms   prepare: 2 PR, 6–7 DW (Auftrag, Index,   Platte f  CPU .
         Reservierung ≥3, mutation_started)
~120ms   Discovery listet (≈15 ms), Worker 1 startet → erstes Byte ≈ 120 ms
Hochlauf Grenze 2 → 4 nach ≈ 2 Fenstern; Gutput steigt nicht
stetig   je Datei ≈ 6,5 ms L (3 PR + 3 DW = 6 fsync), 2,4 ms ohne L (1 DW, Kopie)
         → ≈ 150 Dateien/s; Datenrate 0,6 MB/s; Platte wartet auf Flush-Latenz,
         Bandbreite > 99 % frei; CPU ≈ 5–10 % eines Kerns
Ende     ≈ 33 s (Spanne 10–100 s): record.finish (DW), Hinweis (DW), Done,
         GUI ≤ 50 ms, Neuscan der Ansicht
```
**Linux CLI-Weg (heute, Messung 0,85 s):** Start ≈ 20–40 ms (Prozess, Ziele, 2–3 DW
Befehlsjournal), dann ≈ 100 Aufrufe ≈ 170 µs je Datei je Worker, Grenze wächst
2 → 8 in ≈ 0,5 s; ≈ 6000 Dateien/s. `cp -r`: 0,27 s, ≈ 10 Aufrufe je Datei.

**Windows GUI-Weg (heute):** erstes Byte ≈ 150–400 ms (Vorschau, ≈ 7 DW à ≈ 5 ms);
stetig ≈ 18 ms gesperrt je Datei (3 PR ≈ 2,3 ms, 3 DW ≈ 16 ms) → ≈ 55 Dateien/s →
≈ 90 s (Spanne 40–200 s). Watcher-Ereignisse belasten den GUI-Thread. robocopy
`/MT:16` gleicher Fall (Annahme): 2–4 s.

**Android GUI-Weg nach `/storage/emulated/0`:** gesperrt ≈ 13 ms (3 DW à 4 ms f2fs-fsync
+ PR) → ≈ 75 Dateien/s → ≈ 65 s; je Datei zusätzlich ≈ 10 FUSE-Aufrufe ohne Sperre.

### 8.2 1000 × 1 MiB

| Weg | Erstes Byte | Stetig | Gesamt | Referenz |
|---|---|---|---|---|
| Linux GUI | ≈ 120 ms | je Datei 0,2–0,5 ms Kopie (Seitencache) + 6,5 ms L → ≈ 150 Dateien/s ≈ 150 MB/s; Platte schreibt im Hintergrund zurück, Bandbreite ≈ 90 % frei | ≈ 7 s | `cp -r` ≈ 0,5–1 s + Rückschreiben |
| Linux CLI | ≈ 30 ms | Seitencache-Geschwindigkeit, Grenze 2 → 16 | ≈ 0,5–1 s | wie `cp` |
| Windows GUI | ≈ 300 ms | ≈ 55 Dateien/s | ≈ 18 s | robocopy `/MT` ≈ 1–2 s (Annahme) |

### 8.3 1 × 10 GiB, gleicher NVMe-Datenträger

```
t=0      Strg+V; Vorschau 1 stat; Bildlatenz ≤ 100 ms
~100ms   prepare (≈ 6 DW)
~120ms   Worker: take_extras: Probe der Quelle, Gerätepaar, rotational;
         Datei ≥ ¼ freier Speicher → ungepuffert, Plan Ranges(3);
         frischer Fluss Grenze 2 → nur 1 Zusatzerlaubnis → 2 Threads O_DIRECT,
         je 4 MiB lesen, dann schreiben
stetig   2 × 4 MiB in Arbeit (vorgesehen 4 × 4 MiB); Schätzung 70–100 % der
         Gerätekopierrate (≈ 0,8–1,1 GB/s); Fluss wächst, die laufende Kopie
         nimmt keine neuen Threads auf
Ende     ≈ 10–13 s; Veröffentlichen + 4 DW ≈ 10 ms; Neuscan
```
Mit viel freiem Speicher (Datei < ¼): gepufferte Bereichskopie mit 2 Threads
`copy_file_range`, Rückschreiben durch den Kernel. Windows: `CopyFile2` mit
`NO_BUFFERING` erst ab ¼ freiem Speicher; darunter gepuffert über den Cache-Manager
(Annahme: bei ≥ 1 GiB ungünstiger als ungepuffert). Reflink-Dateisystem: Sekundenbruchteile.

---

## 9. Ursachen je Weg, gegen das Maß gestellt

| Weg | Maß (Werkzeug bzw. Grenze) | se heute (Schätzung) | Verhältnis | Hauptursachen mit Beleg |
|---|---|---|---|---|
| Linux GUI 5000 × 4 KiB | `cp -r` 0,27 s | ≈ 33 s | ≈ 1 % | Journal-Sperre + 8 fsync (W6, W20, W22); Vorphase; Grenze 2–4 |
| Linux GUI 1000 × 1 MiB | `cp -r` ≈ 0,7 s | ≈ 7 s | ≈ 10 % | wie oben |
| Linux GUI 10 GiB | Geräterate | 70–100 % | 70–100 % | 2 statt 4 Threads (Abschnitt 4) |
| Linux CLI 5000 × 4 KiB | 0,27 s | 0,85 s (gemessen) | 32 % | W9 + W12 (≈ 65 Aufrufe), Start bei 2 |
| Windows GUI 5000 × 4 KiB | robocopy `/MT:16` 2–4 s (Annahme) | ≈ 90 s | 2–4 % | Journal; ≈ 80 CreateFile je DW (`open_root` über alle Vorfahren); Flush + WT-Umbenennen |
| Windows CLI 5000 × 4 KiB | wie oben | ≈ 3–6 s | 50–100 % | 2 Proben + `metadata` + `open_staged` + Identitäten je Datei; WT-Umbenennen |
| Android GUI 5000 × 4 KiB | `cp` im Gerät (Annahme ≈ 2–5 s auf FUSE) | ≈ 65 s | 3–8 % | Journal; Umbenennungsleiter 4 FUSE-Aufrufe |
| `se mv` / MCP `move` über Datenträger | `mv` (Kopie + Löschen) | ≤ ⅓ Geräterate, 20–300 Dateien/s | ≤ 33 % | verschlüsselter Spool, fsync je Datei, 1 Thread |
| E8 gefilterte Kopie (Windows) | Explorer-Kopie derselben Dateien | durch GUI-Bildtakt begrenzt | unbekannt, Messung nötig | COM über GUI-STA |
| E9 lange Pfade | robocopy | 1 Thread, fsync je Datei, doppelte Kopie (Temp + Explorer) | ≤ 10 % (Schätzung) | snapshot_download.rs, download_file.rs:84 |
| GUI-Verschieben einzelner Dateien, gleicher Datenträger | Explorer/`mv`: 1 Umbenennen je Datei | ≈ 80 Dateien/s Linux, ≈ 30 Windows | ≈ 1 % | Quarantäne + Alias + 5 DW + 2 Ordner-fsync |

---

## 10. Zielentwurf

Ziel je Weg: Gesamtdurchsatz ≥ 90 % des schnellsten Werkzeugs bzw. der Gerätegrenze,
vom ersten bis zum letzten Byte. Kennzeichnung der Abdeckung durch das parallele
Umsetzungspaket KP1: **[K-A]**, **[K-C]**, **[K-E]**, **[K-H]** = in KP1 enthalten;
**[K-A+]** = in K-A angelegt, muss für den lokalen Weg ausdrücklich so umgesetzt
werden; **[neu]** = nicht in KP1.

### 10.1 Maßnahmen

| # | Maßnahme | Ort | Erwarteter Gewinn (Schätzung) | Berührte Invariante und wie sie gehalten wird | KP1 |
|---|---|---|---|---|---|
| M1 | Journal v2: ein Anhängeprotokoll je Auftrag, Ordnerhandle einmal geöffnet, Commit-Thread mit Gruppen-fsync; Absicht einer Datei wartet nur auf „ihren“ Commit; „veröffentlicht“ ohne eigenes fsync | file_recovery_receipts.rs:315-390, file_recovery.rs:105-178, record_io.rs:56-71, support_dirs.rs:67-97 | Linux GUI 5000 × 4 KiB von ≈ 33 s auf ≈ 0,3–0,6 s; Windows von ≈ 90 s auf ≈ 3–6 s | Absicht dauerhaft vor Veröffentlichung: Worker wartet auf Commit-Position vor `renameat2`/`MoveFileEx`. Abgeschnittene letzte Zeile = heutige `pending`-Reparatur. Tresorziele: Eintrag einzeln versiegelt | [K-A] |
| M2 | Belegsicht im Speicher: Wiederaufnahmeprüfung (W6) und `after_checked`-Lesen (W22) aus einem In-Memory-Index des Protokolls, kein privater Dateilesevorgang je Datei | missing_retry.rs:99-108, file_recovery.rs:114-116 | −3 PR je Datei (≈ 150 Aufrufe Linux, ≈ 75 CreateFile Windows) | Wiederaufnahme liest beim Start das Protokoll einmal; laufender Auftrag ist alleiniger Schreiber (Sperrdatei job_store.rs:90-92) | [K-A+] |
| M3 | Lokale Pakete: ein Worker nimmt bis zu 64 kleine Dateien (wie `take_small`, queue.rs:222-251) unter einer Erlaubnis, schreibt **eine** Absichtszeile für das Paket und veröffentlicht nacheinander | engine/batch.rs:21-63 (heute nur Share), worker.rs:36-41 | weniger Commits, weniger Erlaubnis- und Sperrverkehr; trägt M1 auf ≥ 20 000 Dateien/s Linux | wie M1; ein Paketfehler: Rest des Pakets geht einzeln (`alone`), wie bei Share-Paketen | [neu] (K-A nennt „Absichtszeile je Paket“, K-E Pakete nur für Share) |
| M4 | Belegbeweise vom gehaltenen Handle statt vom Pfad: Stage-Identität über `local_file_identity_handle(&staged.file)` (publication.rs:52-62), nach dem Umbenennen nur 1 `lstat` des Ziels gegen `dev/ino` des Handles statt `verify` + `stamp` | file_recovery_observer.rs:54-92, file_recovery.rs:117-145 | −≈ 140 Systemaufrufe je Datei (Linux), −≈ 50 CreateFile (Windows) | „Beweis = dasselbe Objekt“: das Handle ist das Objekt; der Austausch nach dem Umbenennen bleibt durch den `lstat`-Vergleich erkannt | [neu] |
| M5 | Quell-Schnappschuss nur mit Journal und aus dem bereits geöffneten Quell-Handle (`source_snapshot_file`, staging.rs:271) statt eigener Pfad-Abfrage | file_recovery_observer.rs:17-23, engine/local.rs:63 | CLI/MCP/Bereitstellen: −≈ 45 Aufrufe je Datei (≈ 35 % des CLI-Aufwands); GUI zusätzlich −1 stabile Identität | Quelle bleibt durch `unchanged_state` nach der Kopie geprüft (staging.rs:335-345) | [neu] |
| M6 | Optikprobe einmal je Quellwurzel bzw. je `st_dev` zwischenspeichern; Führung (`Guard`) nur für optische Quellen | source_media/mod.rs:34-40, os/linux.rs:30-74, os/windows.rs:23-51, staging.rs:100-108, copy/os/windows.rs:234, vfs/os/shared/local_optical.rs:17-25 | Linux −≈ 20 Aufrufe je Datei und je `LocalBackend::stat`; Windows −2 × `GetVolumePathNameW` und −128 KiB Nullung je Datei | Gerätesperre für optische Medien bleibt; neue Disc = neues `diskseq`/neue Volumeidentität → Cache-Schlüssel enthält Gerät und Medienfolge; Mount-Änderung invalidiert | [neu] |
| M7 | Start und Wachstum lokaler Flüsse nach Laufwerksart: Flash 8 (bzw. Kerne × 2, höchstens 16), drehend/USB 2; neue Worker nach freien Erlaubnissen | flow_control.rs:33, flow_registry.rs:36-62, dispatch.rs:122-126 | Hochlauf entfällt; kleine Aufträge erreichen sofort den stetigen Zustand | keine; Rückgang bei Überlast bleibt (flow_control.rs:258-277) | [K-E] (Startfenster 8, Worker nach freien Erlaubnissen); Laufwerksart [neu] |
| M8 | Große Einzeldatei: bei leerer Warteschlange bis `UNBUFFERED_EXTRAS` (3) Zusatzthreads unabhängig vom momentanen Flusslimit, bzw. Helfer dynamisch nachnehmen | engine/local.rs:200-270, local_plan.rs:25 | 10 GiB NVMe: 0–40 % schneller (Messung nötig) | keine; Verschieben/Überschreiben bleiben einthreadig | [K-E] deckt es ab, wenn der lokale Fluss mit 8 startet; dynamisches Nachnehmen [neu] |
| M9 | Windows-Veröffentlichung einer *Kopie* ohne `MOVEFILE_WRITE_THROUGH`; Umbenennen über das gehaltene Stage-Handle (`SetFileInformationByHandle` `FileRenameInfo`, `ReplaceIfExists = false`) statt Pfad-Neuöffnungen (`path_identity`, `preflight_no_replace`, `open_staged`) | copy/os/windows.rs:173-213, 450-466, safe_file.rs:161-207 | je Datei −1 Logflush (Annahme) und −3 bis −4 CreateFile; UNC: −4 Roundtrips | kein Ersetzen: `ReplaceIfExists = false` scheitert atomar an vorhandenem Namen; Verschieben/Überschreiben behalten Write-Through | [neu] |
| M10 | Windows `copy_flags` ohne `std::fs::metadata` (Größe aus dem Quell-Schnappschuss), Probe nur einmal (M6) | copy/os/windows.rs:228-245, 419-432 | −1 CreateFile je Datei | keine | [neu] |
| M11 | Android: Ergebnis der Umbenennungsleiter je `st_dev` merken (z. B. „rename2 nicht unterstützt, Link nicht unterstützt“) und direkt `lstat` + `rename` | android_fs/os/rename.rs:60-89, 129-151 | −2 FUSE-Aufrufe je Datei | geprüftes `rename` gilt heute schon dort; Restrisiko unverändert dokumentiert (rename.rs:15-18) | [neu] |
| M12 | GUI-Vorschau ohne `LocalBackend::stat` für Dateien (Typ ist aus der Auswahl bekannt), nur `symlink_metadata` für Ordner; Fertigmeldung weckt die GUI sofort (`ctx.request_repaint`) | selection.rs:30-47, folder_conflict_ui.rs:26-36, 100 | erstes Byte −16…100 ms; 5000 ausgewählte Dateien −0,1…2 s | Ordnerentscheidung unverändert für Ordner | [neu] |
| M13 | Ordnerreservierung mit einem Datensatz je Reservierung statt ≥ 3 vollständigen Neuschreibungen des Alias-Speichers; besser als Zeilen im Journal v2 | folder_reservation.rs:131-300, folder_alias_store.rs:90-98 | je ausgewähltem Ordner −2 DW; 100 Ordner: −0,4…1,5 s vor dem ersten Byte | Reservierung bleibt dauerhaft vor dem `mkdir`; Behalten-beider-Nummerierung unverändert | [neu] (mit K-A verbindbar) |
| M14 | Verschieben einzelner Dateien auf demselben Datenträger: direkt `renameat2(NOREPLACE)`/`MoveFileExW` (ohne Ersetzen) von der Originalquelle ins Ziel, ohne Quarantäne-Umbenennung und Hardlink-Alias; Absicht je Paket im Journal; Ordner-fsync einmal je Ordner und Commit | move_guard.rs:73-133, direct_move.rs:18-120, durability.rs:5-38, safe_file.rs:100-126 | ≈ 80 → ≥ 5000 Dateien/s Linux; Windows ≈ 30 → ≥ 1000 (Schätzung) | Umbenennen ist atomar: Datei liegt immer an genau einem Namen; „kein Prüfen-dann-Löschen der sichtbaren Quelle“ bleibt, da nichts gelöscht wird; Wiederherstellung über Identität im Absichtseintrag | [neu] |
| M15 | Verschieben über Datenträger: Stages einer Gruppe parallel ohne Einzel-fsync, dann Linux `syncfs` des Ziels (vorhanden: vfs/os/linux_os/local_platform.rs:59-70), Veröffentlichen, Ordner-fsync je Ordner, Quellen entfernen, `syncfs` der Quelle; Windows: Flush je Datei parallel in mehreren Workern; Bereiche auch für dauerhafte große Kopien | safe_file.rs:128-139, 254-271, staging.rs:167-172 | Linux: von ≈ 1 Datei-fsync + 2–3 Ordner-fsync je Datei auf 2–3 `syncfs` je Gruppe | Quelle wird erst gelöscht, nachdem das Ziel dauerhaft ist: Reihenfolge je Gruppe bleibt Stage → Sync → Veröffentlichen → Sync → Quelle löschen | [neu] |
| M16 | CLI/MCP: `se cp` in vorhandenes Ziel (Ordner `Merge`), mit `--force` (Konflikt `Overwrite`) und MCP `keep-both` (gerade reservierter leerer Ordner) über die Engine; `se mv` über Datenträger als Engine-Move | cli/ops_copy.rs:134-183, cli/engine_copy.rs:83-101, 119-212, cli/ops_move.rs:175-200 | von ≤ ⅓ Geräterate auf Engine-Rate; kleine Dateien × 20–100 | Terminal-Zusagen: genau der getippte Name, kein Ersetzen ohne `--force`, Wettlauf = Fehler. Die Engine erfüllt sie bereits für den geeigneten Fall; der Spool sichert heute zusätzlich „ganzer Quellbaum geprüft, bevor das Ziel berührt wird“ (tree_spool.rs:81-83, 145-147). Diese Zusage entfällt auf dem Engine-Weg wie heute schon für `se cp` (docs/GOTCHAS.md laut engine_copy.rs:11-12) und ist ausdrücklich zu entscheiden | [neu] |
| M17 | E8: virtuelles Datenobjekt auf einem eigenen STA-Thread (wie der entfernte Weg) oder mit Free-Threaded-Marshaler, `IStream::Read` ohne GUI-Thread | virtual_clipboard/os/windows.rs:258-283, read_stream.rs | Explorer-Kopie läuft mit Laufwerksgeschwindigkeit statt im Bildtakt (Messung nötig) | Lesen weiter nur über das gehaltene, freigegebene Handle (read_stream.rs:33-41) | [neu] |
| M18 | E9 lange Pfade über die Engine (lokal→Temp, ohne Journal, ohne fsync je Datei, parallel) statt `download_clipboard_snapshot` | clipboard_materialize.rs:26-28, snapshot_download.rs:8-61, download_file.rs:26, 84 | Temp-Bereitstellung von 1 Thread + fsync je Datei auf Engine-Rate | Veröffentlichung ohne Ersetzen bleibt; Tresorquellen bleiben abgelehnt (clipboard_materialize.rs:17-21) | [K-H], sofern K-H diesen lokalen Aufrufer einschließt |
| M19 | GUI: Windows-Wächter verwirft se-eigene Stage-, Quarantäne- und Privatnamen (`.…smart-explorer-…part`, `.…move`, `.se-private-…tmp`) vor jedem `metadata`; nach Abschluss nur die aufgelösten Wurzeln in die Ansicht einfügen statt `rescan()` | app/os/windows/watchers.rs:193-290, transfer_lifecycle.rs:24-32 | GUI-Thread −0,5…1 s je 5000 Dateien; kein Konkurrenz-Scan nach jedem Einfügen | Ansicht bleibt korrekt: aufgelöste Wurzeln kommen aus `Done.roots` (engine/mod.rs:408) | [neu] |
| M20 | Thread-Priorität: vom Nutzer gestartete GUI-/CLI-Kopien mit normaler Priorität, Sync und Hintergrund weiter abgesenkt | worker.rs:25, transfer/os/unix.rs:67-81, transfer/os/windows.rs:69-83 | nur bei CPU-Sättigung messbar | keine | [neu] (Produktentscheidung) |
| M21 | Abgeschlossene Aufträge: Belegordner und Protokoll nach `complete` löschen bzw. verdichten | job_store.rs:333-346 | kein Wachstum von `transfer-jobs`; schnellerer Wiederherstellungskatalog (job_store_catalog.rs:9-33 liest jeden Auftrag) | nur `complete`-Aufträge ohne offene Absicht | [K-A+] |

### 10.2 Zielzustand je Weg

| Weg | Ziel je Datei | Erwartet (Schätzung) | Maß |
|---|---|---|---|
| Linux GUI lokal→lokal | ≈ 10–15 Aufrufe Kopie + Protokollzeile im Speicher + Anteil an einem Gruppen-fsync (M1–M7) | 5000 × 4 KiB ≈ 0,3–0,5 s inkl. Start; 1000 × 1 MiB ≈ Seitencache-Rate; 10 GiB ≥ 95 % Geräterate (M8) | `cp -r` 0,27 s |
| Linux CLI | ≈ 10–15 Aufrufe (M5–M7) | ≈ 15 000–25 000 Dateien/s | `cp -r` |
| Windows GUI | `CopyFile2` + ≈ 3 CreateFile + Umbenennen am Handle + Protokollzeile (M1–M10) | 5000 × 4 KiB ≈ 2–5 s | robocopy `/MT:16` |
| Android GUI | Kopie + 2 FUSE-Aufrufe Veröffentlichung + Protokollzeile (M1–M5, M11) | 5000 × 4 KiB ≈ 3–8 s | `cp` auf demselben Gerät |
| GUI-Verschieben gleicher Datenträger | 1 Umbenennen + Protokollzeile (M14) | ≥ 5000 Dateien/s Linux | `mv` |
| Verschieben über Datenträger | Kopie wie oben + `syncfs` je Gruppe (M15) | ≥ 70 % Kopierrate | `mv` |
| `se mv`/MCP über Datenträger, `se cp` vorhandenes Ziel | Engine (M16) | wie CLI | `mv`, `cp` |
| E8/E9 | M17, M18 | Explorer- bzw. Engine-Rate | Explorer-Kopie der Quelldateien |

Zielzeitleiste Linux GUI 5000 × 4 KiB (nach M1–M7, M12, M13):
```
t=0      Strg+V; Vorschau sofort fertig (kein Ordnerkonflikt) → Wecken
~5 ms    prepare: Protokoll anlegen + 1 Gruppen-fsync (Auftrag, Reservierung)
~10 ms   Discovery + 8 Worker; erstes Byte ≈ 10–20 ms
stetig   Worker: Paket ≤ 64 Dateien → Absichtszeile → Commit (1 fsync für alle
         wartenden Pakete) → Kopien + Umbenennen; CPU/Dateisystem-Journal begrenzt
Ende     ≈ 0,3–0,5 s; Abschlusszeile + 1 fsync; Ansicht fügt Wurzeln ein
```

### 10.3 Was KP1 abdeckt und was neu ist

- **Von KP1 abgedeckt:** M1 [K-A], M7 und M8 [K-E], sofern das Startfenster 8 auch für
  lokale Flüsse gilt, M18 [K-H], sofern K-H den lokalen Langpfad-Aufrufer einschließt.
- **In K-A angelegt, für lokal ausdrücklich nötig:** M2 (keine Belegdateilesung je Datei
  im laufenden Auftrag), M21 (Aufräumen).
- **Neu, nicht in KP1:** M3, M4, M5, M6, M9, M10, M11, M12, M13, M14, M15, M16, M17,
  M19, M20.
- **K-C** betrifft keinen lokalen Weg. **K-G** (ganzer neuer Ordner als Stufenordner
  und ein Umbenennen) ist nicht in KP1; lokal würde es die Belege je Datei für neue
  oberste Ordner ersetzen und ist eine Alternative zu M3 für diesen Fall.

---

## 11. Fragen an die Messung (G5) aus G1

| Frage | Messung | Annahme, die sie prüft |
|---|---|---|
| Wie viel kostet das Journal? | gleicher Auftrag `recoverable: true` gegen `false`, 5000 × 4 KiB, 1000 × 1 MiB, Linux und Windows (Vorlage: test_run.rs `job()` gegen CLI-`JobView`) | Befund 1 |
| Wie viele Systemaufrufe je Datei? | `strace -f -c` CLI-Weg und GUI-Weg | Abschnitt 2.3 |
| Was bringt M5 + M6 allein? | CLI-Weg mit abgeschaltetem Schnappschuss und Probe | Befund 3 |
| Wie lange dauert die Vorphase? | Zeit von `submit_paste` bis erstem `copy_file_range` (Zeitstempel im Fortschritt) | Befund 4 |
| Wie teuer ist `MOVEFILE_WRITE_THROUGH`? | windows-2025: `MoveFileExW` mit und ohne Flag, 5000 Umbenennungen | M9 |
| Wie schnell ist die 10-GiB-Kopie mit 2 gegen 4 Threads? | Fluss-Startgrenze 2 gegen 8, Gerätedurchsatz je 100 ms | Befund 6, M8 |
| Wie langsam ist E8? | Explorer-Einfügen gefilterter Auswahl gegen CF_HDROP derselben Dateien, GUI im Vordergrund | M17 |
| Bremst vfat `flush`? | Loop-Image vfat mit und ohne `flush`, 1 gegen 8 Worker | Abschnitt 5 |
| Wie langsam ist der alte Baumweg? | `se mv` über zwei Dateisysteme (zwei tmpfs/Loop-Images) gegen `mv` | Befund 5 |

---

## 12. Gelesene Dateien

transfer/os/shared/engine: mod.rs, prepare.rs, dispatch.rs, worker.rs, queue.rs, ops.rs,
local.rs, local_plan.rs, discovery.rs, folders.rs (150-410), folders_local.rs,
roots.rs (118-485), folder_reservation.rs (121-432), folder_alias_store.rs (Teile),
job_store.rs, job_store_validation.rs (1-90), job_store_catalog.rs (1-40), record_io.rs,
file_recovery.rs, file_recovery_observer.rs, file_recovery_receipts.rs,
file_recovery_verify.rs (Teile), missing_retry.rs (1-140), file_evidence.rs (156-186,
384-422), stats.rs (Teile), selection.rs, issues.rs (Teile), bottleneck.rs (Teile),
removable_media_task_tests.rs (Kopf). transfer/os/shared: flow.rs, flow_registry.rs,
lane.rs, walk.rs (1-200), walk_listers.rs (1-140), download_file.rs, local_stage.rs,
snapshot_download.rs, hints.rs (Teile). transfer/core: flow_control.rs (1-330),
flow_window.rs (Teile), segments.rs (Konstanten). transfer/os: unix.rs, windows.rs
(Teile). copy: mod.rs, os/shared/safe_file.rs, staging.rs, publication.rs,
publication_proof.rs (272-320), direct_move.rs (1-120), durability.rs, move_guard.rs
(73-133), copy_flags.rs, pipe_copy.rs (1-120); os/linux_os.rs, linux_direct.rs (Teile),
linux_direct_plan.rs, linux_range.rs (1-165), linux_probe.rs (Konstanten),
linux_file_proof.rs (1-60), windows.rs, windows_identity.rs, windows_file_proof.rs
(60-160). android_fs/os/rename.rs. source_media: mod.rs, permit.rs (Teile),
os/linux.rs (1-80), os/windows.rs (1-60), os/unsupported.rs. vfs: os/shared/local.rs
(Teile), local_optical.rs, local_extensions.rs (Teile), core/core.rs (Teile),
core/backend_defaults.rs (75-100), os/linux_os/local_platform.rs (Teile),
os/windows/local_platform.rs (61-86), os/windows/local_writes.rs (49-163).
support_dirs.rs (1-110). creds/os/private_storage_unix.rs, private_storage_windows.rs.
local_access: os/linux_os.rs (Teile), os/linux/directory_handle.rs (59-120),
os/linux/confined.rs (20-120), os/windows/directory_handle.rs (76-115),
os/windows/create.rs (1-50). app: core/transfer_jobs.rs, transfer_route.rs (100-300),
transfer_center.rs (143-240), transfer_lifecycle.rs, folder_conflict_ui.rs (1-175),
frame_update.rs (60-240), scanning.rs (18-167); os/shared/clipboard.rs,
clipboard_provide.rs, clipboard_lifecycle.rs, clipboard_materialize.rs, drag_drop.rs
(Teile); os/linux_os.rs (Teile); os/windows/watchers.rs. virtual_clipboard/os/windows.rs
(Teile), read_stream.rs (1-175). mobile/os/shared/transfer.rs, drive.rs (91-213).
cli: engine_copy.rs, ops_copy.rs (55-192), ops_move.rs (150-200), transfer.rs
(Gliederung), tree_ops.rs (60-160), tree_spool.rs (1-200), tree_apply.rs (1-60,
Gliederung publish_file), folder_conflict.rs (Gliederung), mcp/transfer_tools.rs.
docs: ARCHITEKTUR.md (Abschnitte Einstiege, Datenwege), plaene/…/plan.md,
analyse-plan.md, analyse-kopieren.md.

## 13. Unbestätigt (Annahmen und nicht gelesene Stellen)

- Alle Zeit- und Durchsatzzahlen außer der Basismessung sind Schätzungen nach 0.3.
- `MOVEFILE_WRITE_THROUGH` bei einer Umbenennung auf demselben NTFS-Volume erzwingt
  einen Logflush: Annahme, Messfrage M9.
- vfat-Mount-Option `flush` als udisks-Standard und der Kernelschlaf beim Schließen:
  Annahme zum Kernel- und udisks-Verhalten, nicht im Repository belegt.
- MediaProvider-FUSE lehnt `renameat2`-Flags und `link` ab: Annahme; der Code sieht
  genau diese Leiter vor (android_fs/os/rename.rs:1-21).
- std-Verhalten von `io::copy` zwischen Dateien (`copy_file_range`, Rückfall auf
  `sendfile` bei `EXDEV`): aus der Rust-Standardbibliothek bekannt, nicht im Repository.
- Ob der Windows-Wächter Pfade unter dem App-Datenordner ausfiltert
  (`folder_index::should_skip`, `path_has_skipped_segment`): nicht gelesen.
- Ob Explorer `IStream::CopyTo` oder `Read` nutzt und mit welcher Blockgröße: nicht
  belegt; beides läuft heute über den GUI-STA.
- Löschung abgeschlossener Belegordner: im gelesenen Code nicht gefunden; eine
  Löschung an nicht gelesener Stelle ist möglich.
- Tiefe des Neuscans nach Abschluss (`scan_scope`): nicht gelesen.
- Android-UI (Kotlin) nach Abschluss einer Übertragung: nicht gelesen.
- Kosten von `secure_private_handle` unter Windows (DACL lesen/setzen je Öffnung):
  nicht gelesen, in der CreateFile-Zählung als 1 Öffnung je Aufruf angesetzt.
