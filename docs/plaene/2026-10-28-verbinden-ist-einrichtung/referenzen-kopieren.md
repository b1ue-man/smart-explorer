# Referenz-Kopierwerkzeuge gegen den Kopierpfad von Smart Explorer

Stand: 2026-10-29 (alle Abrufe und Klone an diesem Tag). Nur lesend gearbeitet: nichts gebaut, ausgeführt, getestet, gemessen, installiert, committet. Fremde Quelltexte liegen als flache Klone der genannten Commits und wurden nur gelesen.

Kennzeichnung: **(Beleg)** = im Quelltext oder in der Herstellerdokumentation gelesen, mit Stelle. **(Annahme)** = nicht gelesen oder nur abgeleitet; gilt als unbestätigt. **(Schätzung)** = Zahl ohne Messung, Begründung daneben. Zahlen aus README-Dateien oder Benchmarkdateien der Projekte sind **Selbstauskünfte der Projekte** und von mir nicht überprüft.

Unsere Garantien, auf die sich die Spalte „vereinbar“ bezieht:
- **K1** kein Überschreiben ohne Erlaubnis (Veröffentlichung schlägt bei vorhandenem Namen fehl).
- **K2** Stage-Datei und atomares Umbenennen (Zielname erscheint erst, wenn die Datei vollständig ist).
- **K3** Abbruchsicherheit (Abbruch hinterlässt keine halbe Datei unter dem Zielnamen).
- **K4** Wiederaufnahme nach Absturz über das Journal (Absicht vor Veröffentlichung dauerhaft, Stage bekannt und entfernbar).
- **K5** Nachweis, dass die Quelle während der Kopie unverändert war.

---

## 1. Gelesene Dateien und Quellen

### 1.1 Dateien im Repository (nur die erlaubten)

Vollständig gelesen: `docs/plaene/2026-10-28-verbinden-ist-einrichtung/analyse-g1-lokal.md` (620 Zeilen); `native/src/copy/os/windows_fast.rs`, `windows_fast_probe.rs`, `windows_held_rename.rs`, `windows.rs`, `linux_os.rs`, `linux_range.rs`, `linux_direct.rs`; `native/src/transfer/os/shared/engine/local.rs`, `local_plan.rs`, `local_packet.rs`, `local_packet_stage.rs`; `native/src/transfer/os/shared/flow_registry.rs`.
Teilweise gelesen: `native/src/transfer/os/windows.rs` Zeilen 1-180 (LOCAL_STARTS_PER_CORE, `volume_is_rotational`, `lower_thread_priority`); `native/src/transfer/os/unix.rs` Zeilen 1-130 (LOCAL_STARTS_PER_CORE, `WORKER_NICE`, `lower_thread_priority`).
Nicht lesbar (ausserhalb der Erlaubnis, daher Lücken in Abschnitt 6): `staging.rs`, `safe_file.rs`, `copy_flags.rs`, `windows_optical.rs`, `walk_listers.rs`, `queue.rs`, `file_recovery*.rs`, `publication_proof.rs`, `android_fs/rename.rs`.
Hinweis: `analyse-g1-lokal.md` beschreibt den Stand VOR den Änderungen (eingefrorene Kopie von `a67d7296`). Wo ich daraus zitiere, steht „Analyse“.

### 1.2 Fremde Quellen (Kürzel für die Tabellen)

| Kürzel | Quelle | Version / Commit | Wie gelesen |
|---|---|---|---|
| BC | https://github.com/articulite/BetterCopy (Rust, MIT, Windows) | a3e239bb6bbe60b79df93e6e04ad5e113c40934c, 2026-08-07 | Klon, `better_copy_core/src/{engine,walker,profiler}.rs`, `README.md`, `seed.md`, `bench/*.md` |
| BC-F1 | https://github.com/RADINMNX2/BetterCopy | fc1fe9d00fd277782573951035851defa881eda0, 2026-09-18 | Klon, nur Identität und Dateiliste geprüft |
| BC-F2 | https://github.com/ithesalah2026/BetterCopy | 96ce554ce6e4c65f1c0ddbf089dc42f1f66d6979, 2026-10-07 | Klon, nur Identität und Dateiliste geprüft |
| FC | https://github.com/shirouzu/FastCopy (C++, GPLv3), FastCopy 3.63 | 72655443c775af09a69cc2fd2aa6a602b4d987c9, 2019-02-25 | Klon, `src/fastcopy.cpp`, `fastcopyr.cpp`, `fastcopyw.cpp`, `cfg.cpp`, `fastcopy.h` |
| FCH | https://fastcopy.jp/help/fastcopy_eng.htm (Hilfe und Versionsgeschichte, Kopfzeile des Abrufs: 2026/08/18) | Version 5.12 laut Verteilerliste | Abruf, Wortlaut der Versionsgeschichte |
| UC | https://github.com/alphaonex86/Ultracopier (C++/Qt, GPL3), Pfadpräfix `plugins/CopyEngine/Ultracopier-Spec/` | 6f80b901c5941aaee7ca812c2db3e7b903472da1, 2026-10-11 | Klon, `IO_URING.md`, `pipeline/PIPELINING_DESIGN.md`, `win-iocp/TransferThreadWin.cpp`, `uring/TransferThreadUring.cpp`, `CopyEngineFactory.cpp` |
| CU | https://github.com/coreutils/coreutils, `src/` | 7a3fdd3185ab1b07686294b8b9b91ae29414c535, 2026-10-27 | Klon, `copy.c`, `copy-file-data.c`, `ioblksize.h`, `NEWS` |
| GL | https://github.com/coreutils/gnulib `lib/savedir.h`, `lib/savedir.c` | master am 2026-10-29 (letzte Änderung a8482cee, 2026-01-01) | Einzeldatei-Abruf |
| XC | https://github.com/tarka/xcp (Rust, GPL3), v0.24.9 | b1db3808ff9d47cf4268bec21a99e863d6a06c43, 2026-10-27 | Klon, `libxcp/src/{operations,drivers/parfile,drivers/parblock}.rs`, `libfs/src/{common,linux}.rs`, `src/options.rs` |
| FCP | https://github.com/Svetlitski/fcp (Rust), v0.2.1 | f8db0603ff0fb66d34d3c461b8e6239b72e1cad2, 2026-04-22 | Klon, `src/{lib,filesystem}.rs`, `README.md` |
| WCP | https://github.com/wheybags/wcp (C++, io_uring) | 57c5e64fb65dd8c282309b7cd9d26484fc9b7e56, 2021-05-21 | Klon, `readme.md`, `src/{CopyQueue,CopyRunner,Util,Config}.cpp`, mitgelieferte `liburing/examples/io_uring-cp.c` |
| RS | https://github.com/RsyncProject/rsync | fd96bc7e8a9c320b40fa30a40fe6c36b516ae2e0, 2026-10-26 | Klon, `receiver.c`, `rsync.c`, `rsync.h`, `syscall.c` |
| RC | https://github.com/rclone/rclone (Go) | 1053463755dea408a4abd42bf2ce917a3884a62a, 2026-10-28 | Klon, `fs/config.go`, `fs/operations/copy.go`, `backend/local/local.go` |
| RUST-K | https://github.com/rust-lang/rust `library/std/src/sys/io/kernel_copy/linux.rs` | master am 2026-10-29 (letzte Änderung 73e5f54a, 2026-05-12) | Einzeldatei-Abruf |
| RUST-W | https://github.com/rust-lang/rust `library/std/src/sys/fs/windows.rs` | master am 2026-10-29 (letzte Änderung deb577aa, 2026-10-25) | Einzeldatei-Abruf |
| ML-ROBO | https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/robocopy | `updated_at` 2026-02-16 | Abruf |
| ML-CF2 | https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-copyfile2 und `.../ns-winbase-copyfile2_extended_parameters` | `updated_at` 2025-07-01 bzw. 2024-02-22 | Abruf |
| ML-CFEX | https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-copyfileexw | `updated_at` 2025-07-01 | Abruf |
| ML-BC | https://learn.microsoft.com/en-us/windows/win32/fileio/block-cloning | `updated_at` 2025-10-15 | Abruf |
| ML-ODX | https://learn.microsoft.com/en-us/windows-hardware/drivers/storage/offloaded-data-transfer | `updated_at` 2025-03-25 | Abruf |
| ML-DEV | https://learn.microsoft.com/en-us/windows/dev-drive/ | `updated_at` 2026-06-01 | Abruf |
| ML-FIBH | https://learn.microsoft.com/en-us/windows/win32/api/minwinbase/ne-minwinbase-file_info_by_handle_class | `updated_at` 2024-02-22 | Abruf |
| ML-CREATE | https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew | `updated_at` 2025-07-01 | Abruf |
| MAN-CFR | https://man7.org/linux/man-pages/man2/copy_file_range.2.html | man-pages 6.19, 2026-05-14 | Abruf |
| MAN-OPEN | https://man7.org/linux/man-pages/man2/open.2.html (Abschnitt O_TMPFILE) | man-pages 6.19, 2026-02-08 | Abruf |
| TC | https://www.codesector.com/teracopy | Seite zeigt „TeraCopy 4.0.5“ | Abruf |

Nicht als Beleg gelesen (nur Suchtreffer): npm-Paket `better-copy`, C-Werkzeug `fast-copy`, `UltraFastCopy`, `ExtremeCopy`, `RapidCopy`, ein rclone-Forumsthread zu robocopy (gelesen, enthält keine Technikangaben).

---

## 2. „BetterCopy“: Befund

**Es gibt genau ein Kopierwerkzeug dieses Namens, das zu deiner Beschreibung passt:** https://github.com/articulite/BetterCopy (nicht als Fork angelegt, erstellt 2026-07-14, 91 Sterne laut GitHub-API am 2026-10-29, MIT-Lizenz, Rust, nur Windows). Beschreibung des Projekts: „Better copy utility for modern Windows computers“; „auto-tuning, parallel copy and delete engine“, ausgelöst über Strg+Umschalt+V (Einfügen) und Strg+Umschalt+Entf (Löschen) nur bei fokussiertem Explorer oder Desktop (Beleg: BC `README.md` Zeilen 1-13, 38-55).

Weitere Treffer mit diesem Namen:
| Kandidat | URL | Befund |
|---|---|---|
| Fork 1 | https://github.com/RADINMNX2/BetterCopy | Fork von articulite, erstellt 2026-09-08, 0 Sterne; gleicher Inhalt, dazu `better_copy_native/` (Win32-Fassung) und `installer_app/` (Beleg: Dateiliste des Klons) |
| Fork 2 | https://github.com/ithesalah2026/BetterCopy | Fork, erstellt 2026-10-06; gleicher Inhalt, zusätzlich nur ein CI-Workflow |
| npm | https://www.npmjs.com/package/better-copy | Suchtreffer, nicht gelesen; nach der Trefferbeschreibung eine Node-Bibliothek zum Kopieren von Ordnern, kein schnelles Kopierwerkzeug (Annahme) |
| anders benannt | FastCopy, TeraCopy, Ultracopier, UltraFastCopy, ExtremeCopy | keine Treffer auf „BetterCopy“; falls du eines davon gemeint hast, stehen FastCopy, TeraCopy und Ultracopier unten |

Ob du genau dieses Projekt meintest, ist **Annahme**; es ist das einzige mit dem Namen.

### Was BetterCopy tatsächlich tut (Quelltext, Stand a3e239b)

1. **Pro Datei ein einziger Aufruf `CopyFileExW(src, dst, …, COPY_FILE_FAIL_IF_EXISTS)`**, ab 256 MiB zusätzlich `COPY_FILE_NO_BUFFERING` (Beleg: `engine.rs` Zeilen 25-26, 84-102). Kein eigener Lese-/Schreibpfad, keine Stage-Datei, kein Umbenennen, kein Journal. Der Kernel (`CopyFileEx`) erzeugt die Zieldatei direkt unter dem Zielnamen.
2. **Parallelität:** `DeviceClass::SsdNvme` und `Unknown` = 16 Threads, `BotUsb` = 2, `Hdd` = 1, Netzlaufwerk = 2 (Beleg: `profiler.rs` Zeilen 71-79, 174). Große Dateien (ab 1 MiB) laufen in einem eigenen Pool mit höchstens 2 Workern, kleine in einem Pool mit allen Threads (Beleg: `walker.rs` Zeilen 152, 255; `engine.rs` Zeilen 421-475).
3. **Kleine Dateien werden nach Zielordner gruppiert; ein Thread nimmt einen ganzen Ordner** (Beleg: `engine.rs` Zeilen 310-319, 447-475; Begründung im Kommentar: „NTFS directory index locks“, ohne Beleg, also Behauptung des Autors).
4. **Baumaufnahme vorab und seriell:** rekursives `fs::read_dir` plus `fs::symlink_metadata` je Eintrag (ein Öffnen je Eintrag unter Windows, vgl. RUST-W `windows.rs` Zeilen 1523-1535), Zielordner vorab per `create_dir_all`, Ordnerzeiten am Ende (Beleg: `walker.rs` Zeilen 86, 97; `engine.rs` Zeilen 279-290, 489-490). Reparse-Punkte werden übersprungen (Beleg: `walker.rs` Zeilen 97-106).
5. **Verschieben:** gleiche Platte = einzelnes `MoveFileExW(WRITE_THROUGH | COPY_ALLOWED)`; sonst kopieren, Größe des Ziels prüfen, Quelle löschen (Beleg: `engine.rs` Zeilen 323-370, 496-525).

### Einschränkungen der Belege von BetterCopy

- **Benchmarks (alle Selbstauskunft, n=1, 100.000 Dateien à 8 KB in 500 Ordnern, NVMe):** README: BetterCopy 21,71 s gegen `robocopy /MT:32` 35,43 s (1,63×), `/MT:32 /J` 38,39 s (Beleg: `README.md` „Performance“). `bench/benchmark_results-2.md` (Ordner von Defender ausgenommen): 19,175 s gegen 34,374 s (1,79×) und 36,412 s (1,90×); die Robocopy-Zeit blieb gleich (34,37 s gegen 34,42 s), BetterCopy wurde 17,6 % schneller. `bench/benchmark_results-3.md` (Sandwich-Reihenfolge, Ordner ausgenommen): 16,18 s und 20,19 s gegen 33,29 s und 35,32 s.
- **Widerspruch im Projekt:** die README sagt „Defender enabled“; `benchmark_results.md` Zeile 14 sagt, `bcopy.exe` sei von Defender ausgenommen, `benchmark_results-2.md` Zeile 9 und `-3.md` Zeile 15 sagen, der Benchmark-Ordner sei ausgenommen. Welche Messung welcher Einstellung entspricht, ist nicht eindeutig.
- **README-Behauptung ohne Deckung im Code:** „One-handle-rule implementation with selective pre-allocation and cached metadata writes“ ist als erledigt abgehakt (`README.md` Zeile 190); im Quelltext finden sich weder Vorbelegung noch Handle-Metadaten (Suche nach `preallocat`, `SetEndOfFile`, `FileAllocation`, `SetFileInformation` ohne Treffer in `better_copy_core/src`). Die genannten Techniken stehen nur als aufgeschobene Ideen in `seed.md` Zeilen 350-376 („Relative-handle creates“, „One handle, all operations“).
- **Sicherheitsmerkmale:** keine Stage-Datei, kein atomares Umbenennen, kein Journal, Abbruch löscht die halbe Zieldatei über den `CopyFileEx`-Abbruchweg (Beleg: ML-CFEX „Return value“: bei `PROGRESS_CANCEL` wird die teilweise kopierte Zieldatei gelöscht), bei Absturz bleibt die halbe Datei unter dem Zielnamen.

---

## 3. Vergleichsmatrix und Tabellen je Werkzeug

### 3.1 Dieselben Kriterien für alle Werkzeuge (Windows)

| Kriterium | BetterCopy | FastCopy 3.63 | Ultracopier | robocopy | Smart Explorer (Schnellweg ≤ 1 MiB) |
|---|---|---|---|---|---|
| Baumdurchlauf | seriell, vorab, `read_dir` + `symlink_metadata` je Eintrag (BC walker.rs:86, 97) | seriell je Ordner, `FindFirstFileEx` mit LARGE_FETCH (FC fastcopy.cpp:92, 640; fastcopyr.cpp:311) | Scan-Thread, Liste vorab (UC `ListThreadScan.cpp`, nicht im Detail gelesen) | nicht dokumentiert | Entdeckungs-Thread überlappend (Analyse 2.2); Windows-Listenfunktion nicht lesbar |
| Dateien gleichzeitig | 16 Threads (BC profiler.rs:71-79) | Schreiben: 1 Thread; Lesen: 1 Thread; Quellen öffnen: 16 Threads, bis 256 geöffnete Quellen (FC fastcopy.h:95; cfg.cpp:170; fastcopy.cpp:897-914) | `inodeThreads` = Kerne, höchstens 16 (UC CopyEngineFactory.cpp:289) | `/MT:n`, 1-128, Standard 8 (ML-ROBO) | Kerne × 4, begrenzt auf 8-16 (flow_registry.rs:25-26, 129-139; transfer/os/windows.rs:20) |
| Ziel | direkt unter Zielnamen (`CopyFileEx`) | direkt, `CREATE_ALWAYS` (FC fastcopyw.cpp:487) | direkt, `CREATE_ALWAYS`/`OPEN_ALWAYS` (UC win-iocp/TransferThreadWin.cpp:322-336) | direkt (Annahme) | Stage `CREATE_NEW`, danach Umbenennen am Handle (windows_fast.rs:197-206; windows_held_rename.rs:79-124) |
| fsync/Flush je Datei | nein | nein (kein `FlushFileBuffers` im Schreibpfad, nur im Löschpfad `fastcopyd.cpp:345`) | nein, begründet (UC TransferThreadWin.cpp:818-822) | nein (Annahme) | nein für neue Kopie (Analyse 3.1; windows.rs:181-199) |
| Metadaten | Zeiten und Attribute durch `CopyFileEx` | `SetFileTime(Handle)` + `SetFileAttributesW(Pfad)` (FC fastcopyw.cpp:577, 595) | `SetFileTime` (UC TransferThread.cpp:1279) | Standard `/COPY:DAT` (ML-ROBO) | Zeiten und Attribute am Handle (windows_fast.rs:238-241) |
| Puffer / Kernelkopie | Kernel (`CopyFileEx`) | 256 MiB Hauptpuffer (64 Bit), 20 überlappte E/A à 1 MiB, ab 5.11 2 MiB (FC cfg.cpp:150-154; FCH) | 8 Puffer, IOCP (UC TransferThreadWin.cpp:105, 244-259) | Kernel (Annahme) | 256 KiB-Puffer, 1 Lesen + 1 Schreiben (windows_fast.rs:50, 256-295) |

### 3.2 Dieselben Kriterien für alle Werkzeuge (Linux und plattformübergreifend)

Zählung der Systemaufrufe je neuer 4-KiB-Datei, Standardoptionen, gleiches Dateisystem ohne Reflink, aus dem Quelltext abgeleitet (ohne `strace`; Unsicherheit ± 2):

| Kriterium | GNU `cp -r` | xcp 0.24.9 | fcp 0.2.1 | wcp | rsync (lokal) | rclone (lokal) | Smart Explorer |
|---|---|---|---|---|---|---|---|
| Baumdurchlauf | Ordner komplett einlesen, nach Inode sortiert (CU copy.c:387; GL savedir.h:39-46) | `WalkDir` seriell, `lstat` je Eintrag (XC operations.rs:189-202) | `read_dir`, Dateityp aus `d_type`, rayon-parallel je Ordner (FCP lib.rs:62-81; filesystem.rs:51-56) | `getdents64` mit 32-MiB-Puffer, `d_type` (WCP CopyQueue.cpp:593; Util.cpp:96-105) | Dateiliste, nach Namen sortiert, 3 Prozesse (Annahme zur Sortierung) | Lister + Checker (8) | Entdeckungs-Thread, `read_dir` + `statx` je Eintrag (Analyse 2.2) |
| Dateien gleichzeitig | 1 | 4 (XC options.rs:54-55) | Kerne (rayon) | 1 Thread, io_uring-Tiefe | 1 | 4 Transfers (RC fs/config.go:80-83) | 8 (4 Kerne × 2, mind. 8) bis 16 (flow_registry.rs:25-26, 129-139; unix.rs:25) |
| Ziel | `O_CREAT|O_EXCL` am Zielnamen (CU copy.c:947-948) | `File::create` = Abschneiden (XC operations.rs:55) | `create(true).truncate(true)` (FCP filesystem.rs:66-75) | `O_CREAT|O_TRUNC` (WCP CopyQueue.cpp:739) | Temp `.name.XXXXXX` + Umbenennen (RS receiver.c:413-461; rsync.c:882-945) | Temp-Name `.<crc32>…` + Umbenennen, ausser `--inplace` (RC fs/operations/copy.go:92-107) | Stage `.…part` + `renameat2(NOREPLACE)` (linux_os.rs:273-284, 416-418) |
| Aufrufe je Datei | ≈ 10-11: `newfstatat`, [`fstatat` Ziel], `open`, `fstat`, `openat`, `ioctl(FICLONE)`, `fstat`, `copy_file_range` ×2, `close` ×2 (CU copy.c:744, 753, 947, 1003, 1021; copy-file-data.c:135) | ≈ 16: `lstat` (Walker), `open`, `fstat`, `open`, `ftruncate`, `ioctl(FICLONE)`, `fstat`, `copy_file_range`, `flistxattr`, `fstat`, `fchmod`, `fstat`, `futimens`, `close` ×2 (XC operations.rs:46-56, 96-146; common.rs:31-78; linux.rs:78-83, 255-269) | ≈ 8-9: `open`, `statx`, `open`, `copy_file_range` (std), `fchmod`, `close` ×2 (FCP filesystem.rs:43; RUST-K kernel_copy linux.rs:274-300, 640-647) | ≈ 5-6: `statx` (Walker), `open` ×2, Ring-Anteil, `close` ×2 (WCP CopyQueue.cpp:625-640, 738-739) | > 20 über Pipes zwischen Generator/Sender/Empfänger (Annahme, nicht gezählt) | > 20 (Go-Laufzeit, nicht gezählt) | Analyse 2.3: ≈ 100 (CLI-Weg) vor den Änderungen; heute nicht lesbar (staging.rs) |
| fsync | nein | nur `--fsync` (XC operations.rs:141-144) | nein | nein | nur `--fsync` (RS receiver.c:679) | nein (Annahme) | nein für neue Kopie |
| Metadaten | nur wenn `-p`/`-a` (CU copy.c:1053) | Rechte, Zeiten, xattr, Eigentümer optional (XC operations.rs:131-140) | nur Modus über `fs::copy` | Modus über `open` | Rechte, Zeiten am Temp vor Umbenennen (RS rsync.c:911) | Zeit (RC local.go:1818) | `fchmod`, Zeiten (linux_os.rs:226-259) |
| Kernelkopie | `copy_file_range` ohne Puffer (CU copy-file-data.c:126-135) | `copy_file_range` je 1 MiB (XC operations.rs:69-79) | `copy_file_range` via std | `read`/`write` über io_uring mit O_DIRECT-Quelle (WCP CopyRunner.cpp:55, 144-145, 167) | Pipe, 32 KiB (RS rsync.h:158-160) | Go `io.Copy` | `copy_file_range` je 8 MiB (linux_os.rs:342, 398-409) |

### 3.3 BetterCopy

| Technik | Quelle | kleine Dateien | große Dateien | macht Smart Explorer das schon? | vereinbar mit K1-K5? |
|---|---|---|---|---|---|
| `CopyFileExW` mit `FAIL_IF_EXISTS` direkt auf den Zielnamen | BC engine.rs:84-102 | kürzester Weg: 2 Öffnungen, kein Umbenennen | ab 256 MiB ungepuffert | Für ≤ 1 MiB nein (eigener Handle-Weg windows_fast.rs:168-193); für den Rest ja: `CopyFile2` mit `FAIL_IF_EXISTS` (windows.rs:403-446) auf den Stage-Namen | K1 ja; K2 nein; K3 teilweise (Kernel löscht bei Abbruch, nicht bei Absturz); K4 nein; K5 nein |
| 16 Threads bei SSD/NVMe/unbekannt, 1 bei HDD, 2 bei Netz | BC profiler.rs:71-79, 174 | Zahl der gleichzeitigen Erzeugungen | Large-Pool höchstens 2 | ja: Start Kerne×4, 8-16; rotierend = 2 (flow_registry.rs:129-139; windows.rs(transfer):20) | ja |
| getrennte Pools klein (< 1 MiB) / gross, gross höchstens 2 Worker | BC walker.rs:152; engine.rs:421-475 | grosse Dateien blockieren keine kleinen | verhindert Seek-Konkurrenz | teilweise: Pakete ≤ 1 MiB (local_packet.rs:55), grosse Dateien mit Zusatzthreads aus freien Erlaubnissen (local.rs:229-276); gemeinsame Flusserlaubnisse | ja |
| Zuweisung nach Zielordner (ein Thread = ganzer Ordner) | BC engine.rs:310-319, 447-475 | weniger gleichzeitige Erzeugungen im selben Verzeichnis (Wirkung als Behauptung des Autors, Annahme) | keine | nein (Paket = bis 64 aufeinanderfolgende Einträge der Warteschlange, local_packet.rs:79-109; Warteschlangenreihenfolge nicht lesbar) | ja (nur Reihenfolge) |
| Zielordner vorab anlegen | BC engine.rs:279-290 | spart `mkdir`-Prüfungen im Worker | keine | Register im Speicher, Anlage beim ersten Treffer (Analyse W2) | ja |
| Ordnerzeiten einmal am Ende | BC engine.rs:489-490 | keine Zeitänderung des Ordners nach jedem Eintrag | keine | unbekannt (Ordner-Zeitpfad nicht lesbar) | ja |
| Zwei Benchmarkreihen mit und ohne Defender-Ausnahme | BC bench/benchmark_results*.md | Robocopy ändert sich nicht (34,37 s gegen 34,42 s), BetterCopy gewinnt 17,6 % | keine | nicht anwendbar (Ausnahmen hinzufügen verbietet sich) | nicht anwendbar |

### 3.4 FastCopy 3.63 (Quelltext) und neuere Versionen (nur Hilfetext)

Der veröffentlichte Quelltext endet bei Version 3.63 (shirouzu/FastCopy, 2019-02-25). Das Repository https://github.com/FastCopyLab/FastCopy enthält auf `main` und `release` keinen Quelltext (Commit 50e64b4 „add src export-ignore“). Für 4.x und 5.x gibt es nur Hilfetext und Versionsgeschichte (FCH). Ob die Abläufe von 3.63 in 5.12 gelten, ist **Annahme**.

| Technik | Quelle | kleine Dateien | große Dateien | macht Smart Explorer das schon? | vereinbar? |
|---|---|---|---|---|---|
| Ziel direkt: `CreateFile(dst, GENERIC_WRITE, CREATE_ALWAYS, SEQUENTIAL_SCAN [| NO_BUFFERING ab 64 KiB] [| OVERLAPPED])` | FC fastcopyw.cpp:468-487, 537; cfg.cpp:171 | kein Stage, kein Umbenennen | Direct I/O ab 64 KiB (NTFS) / 128 KiB (FAT) | nein (Stage; Schreib-Handle ohne SEQUENTIAL_SCAN, windows_fast.rs:199-205) | K1 nein (`CREATE_ALWAYS` überschreibt); K2-K4 nein |
| Quellen vorab öffnen mit 16 Threads, bis 256 offene Quellen | FC fastcopy.h:95; cfg.cpp:170; fastcopyr.cpp:496-503, 989-1030 | verbirgt die Öffnungslatenz (Netz, kalte Platte) | keine | teilweise: jeder Worker öffnet seine Quelle selbst (windows_fast.rs:82-92); 16 Worker parallel | ja |
| Ein Lese- und ein Schreibthread, grosser Puffer, 20 überlappte E/A à 1 MiB | FC fastcopy.cpp:897-914; cfg.cpp:150-154; FCH „I/O unit“ | keine | Warteschlangentiefe 20 bis nahe Gerätegrenze | nein: `CopyFile2` (Tiefe nicht dokumentiert) oder Ring 4 × 1 MiB mit einem Lesethread (windows.rs:241-244, 320-337) | ja (Stage bleibt möglich) |
| Verzeichnis mit `FindFirstFileEx(…, FIND_FIRST_EX_LARGE_FETCH)` | FC fastcopy.cpp:92, 640; fastcopyr.cpp:311; FCH „Directory Fetch“ | schnellere Auflistung (Hersteller: bei hoher Latenz oder sehr vielen Einträgen) | keine | nein: Rust-std `read_dir` ruft `FindFirstFileExW` mit `FindExInfoBasic` und Flags 0 auf (RUST-W windows.rs:1265-1272, Kommentar 1263-1264: „lets be conservative“); ob unser Lister std nutzt, ist nicht lesbar (Annahme) | ja |
| Nullbyte-Dateien werden nicht geöffnet | FC fastcopyr.cpp:1046 | spart eine Öffnung + Abfragen je leerer Datei | keine | nein (Quelle wird für jede Datei geöffnet, windows_fast.rs:82-92; Leerdatei-Sonderfall im lesbaren Code nicht vorhanden) | ja (K5 über Pfadabfrage) |
| Vorbelegung `SetFilePointer`+`SetEndOfFile` nur bei Datei > Lesestück | FC fastcopyw.cpp:643-654 | keine Kosten für kleine | zusammenhängende Cluster | ja, besser: `FileAllocationInfo` nur bei > 256 KiB (windows_fast.rs:263-265, 299-310) | ja |
| Zeiten per `SetFileTime(Handle)`, Attribute per `SetFileAttributesW(Pfad)` | FC fastcopyw.cpp:577, 595 | eine zusätzliche Pfadöffnung je Datei | keine | besser: beides am Handle (`set_basic`, windows_fast.rs:238-241) | ja |
| Versionsgeschichte 5.0.0: „20% to 30% speed increase in high-speed SSD environments due to pre-allocation of space during privileged mode“ | FCH | Mechanismus nicht genannt (Annahme: `SetFileValidData`) | grosse Dateien auf schnellen SSD | nein | nur ohne `SetFileValidData` (legt alte Plattendaten offen; braucht `SE_MANAGE_VOLUME_NAME`) |
| 5.9.0: „For SSDs, default is Diff HDD mode (parallel RW) even between the same SSD drives“ | FCH | keine | Lesen und Schreiben als getrennte Threads auch bei gleicher SSD | nicht anwendbar (kein getrennter Lesethread im Schnellweg) | ja |
| 5.7.0: „Add Exclusion Microsoft Defender. (Copying zip, exe, dll … is significantly faster)“ (Wortlaut an der Stelle „…“ gekürzt; im Original steht dort ein Abkürzungswort für weitere Dateiarten, die der Hersteller nicht nennt) | FCH | Hersteller nennt grosse Wirkung | keine | nein | nein (Sicherheitseinstellung des Nutzers, nicht Aufgabe der Anwendung) |
| 5.11.0: Standard-E/A-Grösse von 1 MiB auf 2 MiB | FCH | keine | „improves performance further in some situations“ | nein (256 KiB / 1 MiB) | ja |

### 3.5 TeraCopy (Code Sector)

Quelle TC: Die Produktseite nennt „Multi-threaded copy for faster transfers“ mit „Up to 2 threads“ in der kostenlosen und „3 or more threads“ in der Pro/Business-Fassung, Warteschlange, Überspringen, Wiederholen, Prüfsummen. Zu asynchroner E/A, Puffern, kleinen Dateien, SSD: **keine Angabe**. Quelltext: auf der Seite keine Aussage zur Veröffentlichung; **Annahme: nicht veröffentlicht**. Folge: Techniken von TeraCopy sind nicht belegbar, eine Bewertung entfällt. Smart Explorer startet bei 8-16 gleichzeitigen Operationen (flow_registry.rs:129-139), also über der kostenlosen TeraCopy-Fassung.

### 3.6 robocopy und die Windows-Kopierschnittstellen

| Technik | Quelle | kleine Dateien | große Dateien | macht Smart Explorer das schon? | vereinbar? |
|---|---|---|---|---|---|
| `/MT:n`: Mehrfach-Threads, n = 1-128, Standard 8; Ausgabe auf `/log` umleiten | ML-ROBO „Copy options“ | Hauptmittel für kleine Dateien | wenig | ja: 8-16 | ja |
| `/J`: ungepufferte E/A, „recommended for large files“ | ML-ROBO | BetterCopy-Messung: `/MT:32 /J` 35,3-38,4 s gegen `/MT:32` 33,3-35,4 s, also langsamer bei 8 KB (Selbstauskunft, BC bench/*.md) | empfohlen | `CopyFile2` mit `COPY_FILE_NO_BUFFERING` ab ¼ des verfügbaren RAM (windows.rs:451-464; local_plan.rs:18, 29-31) | ja |
| `/NOOFFLOAD`, `/NOCLONE` („Doesn't attempt block cloning as an optimization“) | ML-ROBO | Standard: Offload und Blockklonen werden versucht | Klonen auf ReFS | Nur über `CopyFile2` (windows.rs:403-446); der Schnellweg für ≤ 1 MiB prüft nur „Netzwerkvolume“, nicht ReFS (windows_fast.rs:176-184; windows_fast_probe.rs:35-54) | ja |
| Standard `/COPY:DAT`, `/DCOPY:DA`: Daten, Attribute, Zeitstempel; keine ACL | ML-ROBO | wenige Metadatenaufrufe | keine | ja: Stage erbt ACL des Ordners (windows_fast.rs:195-196), Zeiten und Attribute (windows_fast.rs:238-241) | ja |
| `/IOMAXSIZE`, `/IORATE`, `/THRESHOLD`, `/LFSM` (Drosselung, Wenig-Platz-Modus; „incompatible with /MT“) | ML-ROBO | keine | nur Drosselung | nicht benötigt | ja |
| Aufrufe je Datei, Aufzählung von Quelle und Ziel, Verzeichniszeiten | nicht dokumentiert, Quelltext nicht veröffentlicht (Annahme; keine Quelle gefunden) | unbekannt | unbekannt | nicht vergleichbar | nicht beurteilbar |
| `CopyFile2`-Flags: `COPY_FILE_DISABLE_PRE_ALLOCATION` („Do not preallocate the destination file size before performing the copy“, ab Windows 10 19041), `COPY_FILE_ENABLE_LOW_FREE_SPACE_MODE` („No overlapped I/Os are used. ODX and SMB offload are not attempted.“), `COPY_FILE_NO_OFFLOAD`, `COPY_FILE_NO_BUFFERING`, `COPY_FILE_RESTARTABLE` („This is slower“), `COPY_FILE_REQUEST_COMPRESSED_TRAFFIC` (SMB), `COPY_FILE_ENABLE_SPARSE_COPY` (Windows 11 22H2) | ML-CF2 (Parameterseite) | Folgerung aus den Flagbeschreibungen: `CopyFile2` belegt das Ziel standardmässig vor und verwendet standardmässig überlappte E/A (Annahme als Schluss, die Standardwerte stehen nicht ausdrücklich in der Doku) | Vorbelegung und Überlappung nützen grossen Dateien | `COPY_FILE_FAIL_IF_EXISTS | COPY_SYMLINK` + Kompressionsanforderung bei Netz (windows.rs:410-446, 451-464); `copy_flags.rs` nicht lesbar, ob `DISABLE_PRE_ALLOCATION` gesetzt wird | ja |
| `CopyFileEx` erhält „extended attributes, OLE structured storage, NTFS alternate data streams, security resource attributes, and file attributes“; Sicherheit wird nicht kopiert | ML-CFEX „Remarks“ | Grundlage für unsere Prüfung „nur ::$DATA, keine EA, keine Ressourcenattribute“ (windows_fast_probe.rs:22-31) | keine | ja | ja |
| Blockklonen: nur ReFS; „Starting with the Windows 11 Moment 5 update (KB5034848) … block cloning occurs natively in supported Windows copy operations“; Dev Drive ab 24H2 / Server 2025 | ML-BC; ML-DEV | Klonen kleiner Dateien möglich, Wirkung unbekannt | sehr gross (Metadatenoperation) | nur über `CopyFile2` (siehe Lücke 6.4) | ja |
| ODX: braucht Speicherarrays mit T10-ODX; Mindestgrösse 256 KB im Kopiermodul | ML-ODX | keine (unter 256 KB nie) | nur SAN | nicht anwendbar | ja |
| Defender „performance mode“ nur auf vertrauenswürdigem Dev Drive (ReFS); Dev Drive nicht auf C: und nicht auf Wechseldatenträgern | ML-DEV | Wirkung gross (Selbstauskunft BC: 17,6 % bei BetterCopy) | gering | Nutzerentscheidung, nicht Anwendung | nicht anwendbar |
| `GetFileInformationByHandleEx` mit `FileIdBothDirectoryInfo` / `FileIdExtdDirectoryInfo` (mehrere Einträge je Aufruf), `FileStreamInfo`, `FileNameInfo`; `FileRenameInfoEx`, `FileDispositionInfoEx` | ML-FIBH | Auflistung mit EA-Grösse und Datei-ID je Eintrag in einem Aufruf (Strukturinhalt aus Windows-SDK-Wissen, nicht in dieser Sitzung gelesen: Annahme) | keine | `FileStreamInfo`, `FileNameInfo` ja (windows_fast_probe.rs:57-89; windows_fast.rs:120-161); Verzeichnisaufzählung nein | ja |

### 3.7 Ultracopier (Qt, GPL3)

| Technik | Quelle | kleine Dateien | große Dateien | macht Smart Explorer das schon? | vereinbar? |
|---|---|---|---|---|---|
| Ziel direkt (`CREATE_ALWAYS`, ohne Stage) | UC win-iocp/TransferThreadWin.cpp:322-336 | kein Umbenennen | keine | nein | K1 nein bei vorhandener Datei; K2, K4 nein |
| Pipeline mit 8 Puffern: überlappte Lese-/Schreibaufträge über IOCP (Windows) bzw. io_uring (Linux); `openat`, `close`, `fsync` einzeln mit Abwarten | UC IO_URING.md (Abschnitte „Ring lifecycle“, „Pipelined“); TransferThreadWin.cpp:105, 542-561 | Autoren-Eigenmessung: Mehrfach-Threads „only roughly halved the IOCP small-file gap vs robocopy (io_uring 2.5×→1.4× rsync)“ (PIPELINING_DESIGN.md Abschnitt 1) | Tiefe 8 | nein | ja |
| Ziel vorbelegen ab 1 MiB: `fallocate(KEEP_SIZE)` bzw. `FileAllocationInfo`, ausdrücklich nicht `SetEndOfFile` (sonst bleibt nach Absturz eine volle Länge mit Nullen) | UC PIPELINING_DESIGN.md Abschnitt 4; TransferThreadWin.cpp:388-401 | keine (kleine übersprungen) | weniger Fragmentierung | Windows: `FileAllocationInfo` ab 256 KiB (windows_fast.rs:263-265); Linux: `set_len` (sparse) vor Bereichskopie (linux_os.rs:383-384) | ja |
| Kein `fsync` je Datei, mit Begründung („THE dominant … slower“) | UC TransferThreadWin.cpp:818-822; TransferThreadUring.cpp:826-830 | gross | gross | ja | ja |
| Entwurf: Metadaten-Operationen (open/close/mkdir/chmod/utime) in eigenem tiefen Pool (8 × Kerne) getrennt von Datenoperationen; Ordnerzeiten nur einmal am Ende | UC PIPELINING_DESIGN.md Abschnitte 1-4 | Ziel des Entwurfs (laut Dokument noch „staged“, nicht umgesetzt) | keine | nein | ja |
| Option `native_copy`: `CopyFileExW` / `copy_file_range` | UC pipeline/TransferThreadPipelined.cpp:366-477 | wie BetterCopy bzw. `cp` | wie BetterCopy | ja | ja |

### 3.8 GNU coreutils `cp` (ab 9.x)

| Technik | Quelle | kleine Dateien | große Dateien | macht Smart Explorer das schon? | vereinbar? |
|---|---|---|---|---|---|
| `copy_file_range` in Schleife ohne Zwischenpuffer, wenn nicht nach Löchern gesucht wird | CU copy-file-data.c:126-170; NEWS „cp, install and mv now use the copy_file_range syscall if available“ | 2 Aufrufe (Daten, dann 0 als Ende) | Kernel-Offload, Reflink, serverseitig (NFS) | ja, über `std::io::copy` (linux_os.rs:398-409) | ja |
| `ioctl(FICLONE)` je Datei (Standard `--reflink=auto`) | CU copy.c:223-224, 1003 | 1 Aufruf, schlägt auf ext4 fehl | Klon auf btrfs/XFS | Kernel-`copy_file_range` klont selbst (MAN-CFR „reflinks“); `FICLONERANGE`-Probe nur für grosse Dateien (linux_direct.rs:260-277) | ja |
| Löchersuche mit `SEEK_DATA`/`SEEK_HOLE` nur, wenn `st_blocks` < `st_size`/512 (dünn besetzt) | CU copy-file-data.c:480-533 | kein Aufwand für dichte Dateien | Löcher bleiben Löcher | **nein**: im lesbaren Linux-Code keine Löcherbehandlung (Suche in linux_os.rs, linux_range.rs, linux_direct.rs ohne Treffer); `copy_file_range` darf Löcher ausfüllen (MAN-CFR „NOTES“: „may expand any holes“) | ja; siehe Lücke 6.2 |
| Verzeichnis komplett einlesen und nach Inode sortieren (`SAVEDIR_SORT_FASTREAD`) | CU copy.c:387; GL savedir.h:39-46 | bessere Inode-Tabellen-Lokalität bei kalter Platte | keine | nein (Annahme: Reihenfolge des Dateisystems; Lister nicht lesbar) | ja |
| Ziel `openat(O_WRONLY|O_CREAT|O_EXCL)` direkt | CU copy.c:947-948 | kein Umbenennen | keine | nein | K1 ja; K2-K4 nein |
| `fadvise(SEQUENTIAL)` nur wenn Reflink/Sparse nicht auf „auto“ | CU copy-file-data.c:573-575 | im Standardfall nicht aktiv | – | nein | ja |
| Einthreadig, ein Datei nach der anderen | CU copy.c (rekursiv) | – | – | nein, wir parallel | ja |

### 3.9 xcp

| Technik | Quelle | kleine Dateien | große Dateien | macht Smart Explorer das schon? | vereinbar? |
|---|---|---|---|---|---|
| Treiber `parfile` (Standard): 1 Walker-Thread + 4 Worker (`--workers`, 0 = Kerne) | XC drivers/parfile.rs:50-87; options.rs:50-55, 118 | parallele Dateikopie | Engpass bei grossen Dateien (Kommentar parfile.rs:17-18) | ja (8-16) | ja |
| Treiber `parblock` (experimentell): Blockparallelität, 1 MiB-Blöcke | XC drivers/parblock.rs:17-20; options.rs:61 | höherer Aufwand | „better performance for large files“ (Selbstauskunft) | ja, Bereichsthreads (linux_range.rs:51-165) | ja |
| `ftruncate(len)` auf das Ziel vor dem Kopieren | XC libfs/common.rs:144-146; operations.rs:56 | 1 Aufruf | Dateigrösse sofort gesetzt (bei Absturz volle Länge mit Nullen: die Gefahr, die Ultracopier vermeidet) | `set_len` nur bei Bereichskopie (linux_os.rs:384) | nur mit K3/K4-Absicherung |
| Rechte, Zeiten, xattr nach der Kopie | XC operations.rs:131-146; common.rs:31-78 | 5-6 Aufrufe | – | Rechte und Zeiten ja, xattr unbekannt (staging.rs nicht lesbar) | ja |
| `File::create` schneidet vorhandene Datei ab | XC operations.rs:55 | – | – | nein | K1 nein |

### 3.10 fcp

| Technik | Quelle | kleine Dateien | große Dateien | macht Smart Explorer das schon? | vereinbar? |
|---|---|---|---|---|---|
| Ordner einmal einlesen; Dateityp aus dem Verzeichniseintrag (`d_type`), kein `stat` je Eintrag; rayon verteilt die Einträge auf alle Kerne | FCP lib.rs:62-81; filesystem.rs:51-56 | kein Aufruf zur Typbestimmung | 13 × 512 MB: 1,4× schneller als `cp` (Selbstauskunft, XFS, 16 CPUs) | teilweise: `read_dir` + `statx` je Eintrag in der Entdeckung (Analyse 2.2); die Grösse wird für die Paketentscheidung gebraucht (local_packet.rs:80) | ja |
| `std::fs::copy` je Datei | FCP filesystem.rs:43 | siehe 3.2 | – | wir nutzen denselben Standardbibliothekspfad (`io::copy`) | ja |
| Messung: Linux-Kernelquellbaum 675 ms gegen `cp` 6,02 s (ca. 10×) auf XFS, 16 CPUs, SSD | FCP `README.md` „Linux“ | Selbstauskunft | – | – | – |
| `create(true).truncate(true)`: überschreibt | FCP filesystem.rs:66-75 | – | – | nein | K1 nein |

### 3.11 wcp und die io_uring-Beispiele

| Technik | Quelle | kleine Dateien | große Dateien | macht Smart Explorer das schon? | vereinbar? |
|---|---|---|---|---|---|
| io_uring: Lesen (`IOSQE_IO_LINK`) mit anschliessendem Schreiben, Quelle mit `O_DIRECT`, mehrere Dateien gleichzeitig in der Warteschlange | WCP CopyRunner.cpp:55, 144-145, 167; CopyQueue.cpp:738 | Selbstauskunft: 200.000 × 1 KiB in 3,92 s (51.021 Dateien/s) gegen `cp -r` 9,23 s und `rsync --inplace` 8,09 s (eigene Maschine, NVMe, 64 GB RAM) | 20 × 512 MiB: 3,97 s gegen `cp -r` 8,44 s | nein | ja, wenn Journal-Reihenfolge gewahrt |
| Ordner mit `getdents64`, 32-MiB-Puffer, `statx` nur bei `DT_UNKNOWN` | WCP CopyQueue.cpp:593, 625-640; Util.cpp:96-105 | nahezu kein Aufruf zur Typbestimmung | – | nein | ja |
| Ziel `O_WRONLY|O_CREAT|O_TRUNC` mit Quellmodus; keine Zeiten, keine xattr (kein Aufruf im Quelltext, Annahme: nicht kopiert) | WCP CopyQueue.cpp:739 | – | – | nein | K1 nein |
| Projektstand: „probably not robust enough for daily use“, Linux ≥ 5.6, 2021 | WCP `readme.md` | – | – | – | – |
| Beispiel `io_uring-cp.c`: eine Datei, Tiefe 64 × 32 KiB `readv`/`writev` | WCP liburing/examples/io_uring-cp.c:18-19, 73-75 | kein Baumwerkzeug | – | – | – |

### 3.12 rsync und rclone (beide lokal)

| Technik | Quelle | kleine Dateien | große Dateien | macht Smart Explorer das schon? | vereinbar? |
|---|---|---|---|---|---|
| rsync: Temp-Datei `.name.XXXXXX` per `mkstemp`, Attribute am Temp setzen, `rename` auf den Zielnamen | RS receiver.c:413-461; rsync.c:882-945 (`robust_rename` Zeile 918) | dieselbe Form wie unsere Stage; dazu 3 Prozesse und Pipe-Transport | – | ja (Stage + Umbenennen) | ja (K2) |
| rsync: kein `fsync` ausser `--fsync`; Vorbelegung nur mit `--preallocate`; Transport über Pipe in 32-KiB-Stücken; kein `copy_file_range`/`sendfile` | RS receiver.c:492-496, 679; rsync.h:158-160 (Suche nach `copy_file_range`, `sendfile`, `FICLONE` ohne Treffer) | – | langsam gegenüber Kernelkopie | wir schneller (gemessen: 2,43×, Suite) | – |
| rclone: `--transfers` 4, `--checkers` 8, `--multi-thread-cutoff` 256 MiB, `--multi-thread-streams` 4 | RC fs/config.go:75-83, 464-472 | – | Mehrfachströme erst ab 256 MiB | Bereichsthreads ab 16 MiB (Flash) / gemessen (linux_range.rs; local_plan.rs:12-25) | ja |
| rclone: lokales Backend meldet `PartialUploads: true`, daher Temp-Name `.<crc32>…` und Umbenennen, ausser `--inplace`; Schreiben mit `O_WRONLY|O_CREATE|O_TRUNC` | RC fs/operations/copy.go:92-107; backend/local/local.go:500, 1736-1750 | gleiche Form wie unsere Stage | – | ja | ja |

### 3.13 Rust-Standardbibliothek (von uns genutzt) – was `io::copy` je Aufruf kostet

| Technik | Quelle | Wirkung | macht Smart Explorer das schon? |
|---|---|---|---|
| `io::copy` zwischen zwei `File`: je Aufruf zweimal `File::metadata` (Leser und Schreiber), dann `copy_file_range` bis 0 zurückkommt | RUST-K kernel_copy/linux.rs:274-300, 640-647, 722-745 | ≈ 4 Aufrufe (2 × statx, 2 × `copy_file_range`) statt 1 | ja, so wird heute kopiert (linux_os.rs:398-409); die Schleife beendet sich nach einem kurzen Stück (linux_os.rs:408), spart aber nur den zweiten `io::copy`-Aufruf |
| Windows `read_dir`: `FindFirstFileExW(FindExInfoBasic, …, 0)` ohne LARGE_FETCH; `DirEntry::metadata` ohne weitere Öffnung; `fs::metadata(pfad)`: Öffnung mit Zugriff 0 + `GetFileInformationByHandle(Ex)` | RUST-W windows.rs:195-205, 1265-1272, 1523-1535 | jeder `metadata(pfad)` kostet eine Öffnung | unbekannt, ob Lister `DirEntry::metadata` nutzen |

---

## 4. Rangliste der Techniken, die wir NICHT nutzen

Reihenfolge nach erwartetem Gewinn, innerhalb jeder Gruppe absteigend. Alle Gewinne sind **Schätzungen** ohne Messung; die Spalte „Belegstärke“ sagt, woher die Richtung der Schätzung kommt. Eine technische Empfehlung steht am Ende jeder Gruppe.

### 4.1 (a) Windows, viele kleine Dateien

| Rang | Technik | Betroffene Dateien bei uns | Geschätzter Gewinn und Begründung | Risiko / Vereinbarkeit | Belegstärke |
|---|---|---|---|---|---|
| 1 | **Ziel ohne Stage direkt unter dem Zielnamen anlegen** (robocopy, FastCopy, BetterCopy, Ultracopier; BC engine.rs:84-102) | `windows_fast.rs` (Zeilen 197-206), `windows_held_rename.rs` (ganze Datei), Stage-Behandlung im Journal (`local_packet_stage.rs` Zeilen 178-225), `staging.rs` (nicht lesbar) | 5-20 % je Datei (Schätzung): spart pro Datei den Umbenennungsaufruf `NtSetInformationFile(FileRenameInformation)`, der den Verzeichnisindex zweimal ändert (Eintrag Stage entfernen, Zielname einfügen: NTFS-Verhalten Annahme) und zusätzliche USN-Ereignisse erzeugt (Annahme). Gegenindiz: Ultracopier schreibt direkt und bleibt laut eigener Dokumentation unter robocopy (UC PIPELINING_DESIGN.md Abschnitt 1) | **Hoch.** Ohne Stage entfällt K2; K4 bricht: nach einem Absturz liegt eine halbe oder leere Datei unter dem Zielnamen, vom Nutzerinhalt nicht unterscheidbar (wir können vor dem Anlegen nur den Namen, nicht die Datei-ID im Journal ablegen). Variante „Ziel mit `DELETE`-Zugriff ohne `FILE_SHARE_DELETE` anlegen, Löschvormerkung (`FileDispositionInfo`) setzen, schreiben, Vormerkung aufheben“ vermeidet fremde Zugriffe und löscht bei Prozessende von selbst, ändert aber nichts am Absturzproblem (Annahme zur Aufhebbarkeit: Doku `FILE_DISPOSITION_INFO` nicht gelesen). **Nicht empfohlen** | Gering (Richtung plausibel, Grösse unbekannt) |
| 2 | **Aufträge nach Zielordner bündeln** (BetterCopy: ein Thread = ein Ordner) | Warteschlange/Verteiler (`queue.rs` nicht lesbar), `local_packet.rs` (`members`, Zeilen 79-91) | 0-25 % bei Bäumen mit vielen Ordnern (Schätzung): BetterCopy erreicht 4.300-6.200 Dateien/s, robocopy mit `/MT:32` bleibt bei 2.750-3.000 Dateien/s auf demselben Datenträger (BC bench/*.md, Selbstauskunft); die Ursache „Verzeichnisindex-Sperre“ ist eine Behauptung des Autors (BC engine.rs:310, seed.md:363-369), ohne Beleg. Bei einem flachen Ordner mit allen Dateien ohne Wirkung | Gering: ändert nur die Reihenfolge; Ordnerlisten bleiben gültig; K1-K5 unberührt | Mittel (Messung vorhanden, Ursache Annahme) |
| 3 | **Pro Datei weniger Abfragen am Handle**: eine `NtQueryInformationFile(FileAllInformation)` statt `GetFileInformationByHandle` + `FileEaInformation` + `FileNameInfo`; `remote_volume` je Volume statt je Datei (zweimal je Datei heute); EA-Grösse aus der Verzeichnisaufzählung | `windows_fast.rs` Zeilen 68-75, 120-161; `windows_fast_probe.rs` Zeilen 26-139; `windows_held_rename.rs` Zeile 52 | 1-3 % je Datei (Schätzung): gezählt sind heute 1 (`information`) + 4 (`plain_local_source`) + 1 (`remote_volume` im Umbenennen) + 2 (`handle_has_path`) = 8 Abfragen an offenen Handles; Abfragen an offenen Handles lösen keine Dateisystem-Filter-Öffnung aus (Annahme), Kosten daher nur der Kernel-Aufruf | Gering; K5 und die Sicherheitsprüfungen (nur `::$DATA`, keine EA, keine Ressourcenattribute) müssen gleich bleiben | Mittel |
| 4 | **Leere Quelldateien nicht öffnen** (FastCopy fastcopyr.cpp:1046) | Aufrufer von `open_copy_source` (`staging.rs`, nicht lesbar); `windows_fast.rs` Zeilen 82-92 | 0-5 % gesamt, je leerer Datei 30-50 % (Schätzung: spart 1 Öffnung + 6-8 Abfragen); Wirkung hängt vom Anteil leerer Dateien im Baum ab | Gering; K5 über Pfadabfrage (Grösse 0 vor und nach der Anlage) | Mittel |
| 5 | **`FILE_FLAG_SEQUENTIAL_SCAN`** auf Quell- und Stage-Handle (FastCopy fastcopyw.cpp:468, fastcopyr.cpp:1053; Ultracopier TransferThreadWin.cpp:246, 328; Doku ML-CREATE: „The system can use this as a hint to optimize file caching“) | `windows_fast.rs` Zeilen 83-88, 199-205 | 0-2 % (Schätzung): eine einzige Lese-/Schreibanforderung ≤ 256 KiB je Datei | Keines | Gering |
| 6 | **Verzeichnisaufzählung mit `FIND_FIRST_EX_LARGE_FETCH`** oder `FileIdBothDirectoryInfo` (FastCopy; ML-FIBH) | `walk_listers.rs` (nicht lesbar); Rust-`read_dir` setzt Flags 0 (RUST-W windows.rs:1265-1272) | 0-3 % (Schätzung): überlappt mit dem Kopieren; Hersteller nennt Wirkung bei hoher Latenz oder sehr vielen Einträgen | Gering | Gering |
| 7 | **`COPY_FILE_DISABLE_PRE_ALLOCATION`** im `CopyFile2`-Rückfall für kleine Dateien (ML-CF2) | `windows.rs` Zeilen 451-464 (`copy_flags.rs` nicht lesbar) | < 1 % (Schätzung): betrifft nur kleine Dateien mit Datenströmen/EA, die den Schnellweg verlassen | Gering | Gering |
| – | Defender-Ausnahmen (FastCopy 5.7.0; BetterCopy `bench`) | – | BetterCopy: 17,6 % (Selbstauskunft); robocopy: 0 % | **Nicht übernehmbar**: Sicherheitseinstellung des Nutzers; Dev-Drive-Leistungsmodus ist Nutzerentscheidung (ML-DEV). Eine Meldung in der Oberfläche („Durchsatz durch Virenscanner begrenzt“) wäre möglich, ist aber keine Kopiertechnik | – |

**Empfehlung (a):** Vor jeder Umbauarbeit zwei Fragen am Messlauf beantworten (Abschnitt 5, Hypothesen H1 und H2): Start des Flusslimits auf dem Runner und Anzahl der Dateizugriffe je Datei ausserhalb der lesbaren Dateien. Aus der Liste lohnt sich danach zuerst Rang 2 (Reihenfolge, risikolos), dann Rang 3 und 4 (kleine Bausteine). Rang 1 bleibt abgelehnt, solange K2 und K4 gelten.

### 4.2 (b) Linux, viele kleine Dateien

| Rang | Technik | Betroffene Dateien bei uns | Geschätzter Gewinn und Begründung | Risiko / Vereinbarkeit | Belegstärke |
|---|---|---|---|---|---|
| 1 | **io_uring-Stapel** für `openat`, `statx`, `read`/`write`, `close` mehrerer Dateien in einem `io_uring_enter` (wcp; Ultracopier uring) | neuer Zweig in `linux_os.rs`; Zusammenspiel mit `local_packet.rs` (Pakete bis 64 Dateien) | 10-30 % bei CPU-gebundenen Kleinstdateien auf 4 Kernen (Schätzung): wcp misst 2,35× gegenüber einthreadigem `cp -r` (Selbstauskunft, 51.021 gegen 21.668 Dateien/s); gegenüber unserem schon parallelen Pfad kleiner. Ultracopier berichtet nur „io_uring 2.5×→1.4× rsync“ (UC PIPELINING_DESIGN.md) | **Mittel bis hoch.** Kernel ≥ 5.6 (WCP `readme.md`); in Containern mit Standard-Seccomp und auf Android möglicherweise gesperrt (Annahme, nicht gelesen); grosse Komplexität; K4 verlangt „Absicht dauerhaft vor Veröffentlichung“ (Ketten über `IOSQE_IO_LINK` möglich, aber aufwendig). Android läuft über denselben Pfad (`linux_os.rs`) | Mittel (Messungen der Autoren, nicht unsere) |
| 2 | **`O_TMPFILE` + `linkat`** statt benannter Stage + `renameat2` (MAN-OPEN) | `linux_os.rs` (Zeilen 273-284, 344-354, 416-418), `android_fs/rename.rs` (nicht lesbar), Stage-Beweise `local_packet_stage.rs` Zeilen 178-225 | 5-15 % je Datei (Schätzung): statt Erzeugen des Stage-Namens + Umbenennen (3 Änderungen im Elternverzeichnis unter dessen Sperre) nur 1 Eintrag beim `linkat`; spart zusätzlich den `lstat` vor dem Umbenennen (`preflight_no_replace`, linux_os.rs:282). Gegenposten: `O_TMPFILE` führt bei ext4 eine Waisenliste (Annahme) | **Gering bis mittel.** `linkat` schlägt bei vorhandenem Namen mit `EEXIST` fehl (K1 ja, atomar); nach Absturz bleibt keine Stage (K3 stärker); K4 muss einen Eintrag „namenlose Stage“ kennen. Nicht unterstützt auf NFS, FAT/exFAT, manchen FUSE (MAN-OPEN nennt nur ext2/3/4, UDF, Minix, tmpfs, XFS, Btrfs, F2FS, ubifs): Rückfall auf die benannte Stage nötig. `linkat` über `/proc/self/fd/N` mit `AT_SYMLINK_FOLLOW`, wenn `CAP_DAC_READ_SEARCH` fehlt (MAN-OPEN) | Mittel |
| 3 | **`copy_file_range` direkt mit bekannter Grösse statt `io::copy`** | `linux_os.rs` Zeilen 371-412 (Nachbau der Rückfälle aus RUST-K linux.rs:742-800 nötig: `EXDEV`, `EINVAL`, `EPERM`, `ENOSYS`, `EOPNOTSUPP`, Rückgabe 0 bei `/proc`) | 2-5 % (Schätzung): spart 3 von ≈ 4 Aufrufen im Kopierschritt (2 × `statx`, 1 × `copy_file_range` mit Ergebnis 0); `statx`/EOF-Aufrufe sind billig, die teuren Aufrufe (Anlegen, Umbenennen) bleiben | Gering; Rückfälle müssen vollständig übernommen werden (sonst K-Verlust bei Dateisystemen, die `copy_file_range` verweigern) | Hoch (Quelltext gelesen) |
| 4 | **Einträge nach Inode sortiert abarbeiten** (`cp`: `savedir(…, SAVEDIR_SORT_FASTREAD)`) | `walk_listers.rs` (nicht lesbar), Warteschlange | 0 % bei warmem Zwischenspeicher, 0-25 % bei kalter Quelle auf ext4 (Schätzung: Inode-Tabellenblöcke werden in Reihenfolge statt in Hash-Reihenfolge gelesen) | Gering; ändert die Reihenfolge der Ergebnisanzeige und evtl. die Nummerierung bei „Beide behalten“ (zu prüfen) | Mittel (Verhalten von `cp` gelesen, Wirkung Annahme) |
| 5 | **Redundante Vorprüfung vor dem Umbenennen weglassen**: `preflight_no_replace` (lstat) vor `renameat2(RENAME_NOREPLACE)` (eigene Beobachtung) | `linux_os.rs` Zeilen 282-283 | 1-2 % (Schätzung: 1 von ≈ 30 Aufrufen) | Gering: bei `EEXIST` des atomaren Umbenennens klassifizieren; für Dateisysteme ohne `NOREPLACE` Rückfall behalten | Hoch (Quelltext) |
| 6 | **Typ aus `d_type`, `statx` nur wo nötig** (fcp, wcp) | `walk_listers.rs` (nicht lesbar) | ≤ 2 % (Schätzung): die Grösse wird für Paketentscheidung und Fortschritt gebraucht, nur Ordner kämen ohne `statx` aus | Gering | Gering |

**Empfehlung (b):** Rang 3 und 5 sind klein, sicher und lokal. Rang 2 (`O_TMPFILE`) ist die einzige Technik fremder Werkzeuge, die unsere Stage-Garantie nicht schwächt, sondern verstärkt, und zugleich Verzeichnisoperationen spart; sie verdient eine Messung, bevor Rang 1 (io_uring) angefasst wird.

### 4.3 (c) Grosse Dateien

| Rang | Technik | Betroffene Dateien bei uns | Geschätzter Gewinn und Begründung | Risiko / Vereinbarkeit | Belegstärke |
|---|---|---|---|---|---|
| 1 | **Windows: ungepuffert schon ab etwa 256 MiB statt ab ¼ des freien Arbeitsspeichers** (BetterCopy: 256 MiB; robocopy `/J`: „recommended for large files“; Microsoft `COPY_FILE_NO_BUFFERING`: „recommended for very large file copies“; FastCopy: Schreiben ungepuffert ab 64 KiB) | `local_plan.rs` Zeilen 18, 29-31; `local.rs` Zeilen 132-143; `windows.rs` Zeilen 451-464; `copy_flags.rs` (nicht lesbar) | 0-30 % für Dateien zwischen 256 MiB und ¼ RAM (Schätzung): gepufferte Kopie füllt den Zwischenspeicher und läuft in die Schreibdrosselung; Messwerte fehlen. Gegenposten: liegt die Quelle im Zwischenspeicher (gerade geschrieben), ist die gepufferte Kopie schneller | Gering; `CopyFile2` behandelt Restbytes; K1-K5 unberührt | Gering (Richtung aus Herstellerempfehlungen, Grösse unbekannt) |
| 2 | **Windows: tiefe überlappte Lese-/Schreib-Warteschlange selbst bauen** (FastCopy: 20 überlappte E/A à 1 MiB, ab 5.11 2 MiB, getrennter Lese- und Schreibthread; Ultracopier: 8 Puffer über IOCP) statt `CopyFile2` | `windows.rs` Zeilen 239-337 (Ring 4 × 1 MiB mit einem Lesethread existiert für Verschieben/Überschreiben) | 0-40 % bei NVMe mit grossen Dateien (Schätzung): die Tiefe von `CopyFile2` ist nicht dokumentiert; die Doku des Flags `COPY_FILE_ENABLE_LOW_FREE_SPACE_MODE` setzt voraus, dass `CopyFile2` sonst überlappte E/A nutzt | **Mittel bis hoch.** Verlust von Blockklonen und SMB-Offload, wenn `CopyFile2` ersetzt wird: `CopyFile2` für ReFS und Netz beibehalten. K2 bleibt (Stage) | Gering |
| 3 | **Linux: tiefere Anforderungswarteschlange mit io_uring** (wcp: Lesen und Schreiben verkettet, mehrere Dateien/Blöcke in der Luft) statt 4 Threads × 4 MiB | `linux_direct.rs` Zeilen 381-401; `linux_range.rs` | 0-20 % auf schnellen NVMe (Schätzung): unsere Auslegung begründet die 4 × 4 MiB mit `fio`-Messungen (local_plan.rs:20-25); wcp misst 20 × 512 MiB in 3,97 s gegen `cp -r` 8,44 s (Selbstauskunft), ein Vergleich mit unserem Pfad liegt nicht vor | Mittel (siehe 4.2 Rang 1) | Gering |
| 4 | Windows `FileAllocationInfo`-Vorbelegung im `CopyFile2`-Weg steuern | `windows.rs` Zeilen 451-464 | < 5 % (Schätzung) | Gering | Gering |
| – | `SetFileValidData` (FastCopy 5.0.0 „pre-allocation … privileged mode“) | – | Hersteller: 20-30 % auf schnellen SSD | **Abgelehnt**: legt bei Absturz alte Plattendaten offen, braucht Administratorrechte; auch BetterCopy und Ultracopier lehnen es ab (BC seed.md:412; UC PIPELINING_DESIGN.md Abschnitt 4) | – |
| – | ODX, `COPY_FILE_RESTARTABLE` | – | ODX nur mit T10-fähigen Speicherarrays und ab 256 KB (ML-ODX); `RESTARTABLE` „slower“ (ML-CF2) | nicht anwendbar | – |
| – | Reflink / Blockklonen | – | Linux: `copy_file_range` und `FICLONERANGE`-Probe vorhanden (linux_os.rs:371-412; linux_direct.rs:256-277); Windows: `CopyFile2` klont auf ReFS (ML-BC) | bereits vorhanden | – |

**Empfehlung (c):** Linux ist schon weiter als alle untersuchten Werkzeuge (Bereichsthreads, `O_DIRECT`, Klonprobe). Auf Windows ist der Schwellenwert für ungepufferte Kopien (Rang 1) die einzige Änderung mit klarer Richtung und geringem Risiko; sie braucht eine Messung auf 256 MiB-4 GiB.

---

## 5. Was robocopy konkret anders macht als unser Windows-Pfad

**Vorweg die harte Grenze des Belegs:** robocopy ist nicht quelloffen, und in Microsofts Dokumentation steht nicht, welche Systemaufrufe es je Datei absetzt (ML-ROBO beschreibt nur Optionen). Die Spalte „robocopy“ unten ist deshalb **Annahme**, abgeleitet aus: (i) der dokumentierten Option `/NOOFFLOAD`/`/NOCLONE` (robocopy nutzt Offload und Blockklonen von sich aus, also dieselbe Kernelkopie wie `CopyFileEx`/`CopyFile2`), (ii) dem Verhalten von BetterCopy, das nachweislich `CopyFileExW` aufruft und mit denselben Daten schneller ist als robocopy. Die Spalte „Smart Explorer“ ist **Beleg** aus den gelesenen Dateien, soweit sie lesbar sind.

### 5.1 Gegenüberstellung je neuer Datei ≤ 1 MiB auf lokalem NTFS

| Schritt | robocopy (Annahme) | Smart Explorer Schnellweg (Beleg) |
|---|---|---|
| Quelle öffnen | 1 Öffnung (in `CopyFileEx`) | 1 `CreateFile(GENERIC_READ, Teilen alles, BACKUP_SEMANTICS|OPEN_REPARSE_POINT)` (windows_fast.rs:83-88) |
| Eigenschaften der Quelle | kernelintern, Umfang unbekannt | `GetFileInformationByHandle` (windows_fast.rs:89 → 68-75); dazu `plain_local_source`: `NtQueryVolumeInformationFile(FileFsDeviceInformation)`, `GetFileInformationByHandleEx(FileStreamInfo)`, `NtQueryInformationFile(FileEaInformation)`, `GetSecurityInfo(ATTRIBUTE_SECURITY_INFORMATION)` (windows_fast_probe.rs:41-48, 61-68, 97-103, 121-130) |
| Ziel anlegen | 1 Öffnung des Zielnamens | 1 `CreateFile(Stage, CREATE_NEW, R|W|DELETE, Teilen R|W)` (windows_fast.rs:199-205) plus Namensprüfung im Benutzermodus (windows_fast.rs:198) |
| Vorbelegen | wahrscheinlich ja (Flag `DISABLE_PRE_ALLOCATION` existiert, ML-CF2) | nur > 256 KiB (windows_fast.rs:263-265) |
| Daten | kernelinterne Lese-/Schreibaufrufe (überlappt laut Flagbeschreibung `LOW_FREE_SPACE`) | 1 `ReadFile` + 1 `WriteFile` (windows_fast.rs:277, 283); Abbruch nach der beobachteten Grösse, kein EOF-Lesen (windows_fast.rs:289-291) |
| Zeiten, Attribute | in `CopyFileEx` | `set_basic(&writer, …)` am Handle (windows_fast.rs:238-241); Aufrufzahl in `windows_optical.rs` nicht lesbar |
| Veröffentlichen | entfällt (Name steht schon) | `NtQueryVolumeInformationFile` (nochmals Netzwerkprüfung, windows_held_rename.rs:52) + `NtSetInformationFile(FileRenameInformation)` (windows_held_rename.rs:101-107): Verzeichniseintrag der Stage entfernen und Zielname einfügen (Annahme zum NTFS-Verhalten) |
| Quelle unverändert | – | `handle_has_path`: `GetFileInformationByHandleEx(FileNameInfo)` + `GetFileInformationByHandle` (windows_fast.rs:120-161), sonst `path_state`: 1 Öffnung mit `FILE_READ_ATTRIBUTES` + Abfragen (windows_fast.rs:97-112) |
| Schliessen | 2 × `CloseHandle`; Virenscanner beim Schliessen des geschriebenen Handles (Annahme) | 2 × `CloseHandle`; dieselbe Lage |
| Absicht/Beweise | keine | Paket-Absicht (`prepare_batch_known`, local_packet_stage.rs:178-225), Stage-Beweis je Mitglied (`stage_proof`, Zeile 182), Quellmomentaufnahme (`snapshot`, Zeile 72), Beleg nach Veröffentlichung (`PacketObserver`); Öffnungen und Abfragen dort in nicht lesbaren Dateien |
| Öffnungen je Datei | 2 (Annahme) | 2 + 0 (wenn `handle_has_path` gelingt) oder 1 (`path_state`) + Öffnungen im Journal (unbekannt) |
| Abfragen an offenen Handles | wenige (Annahme) | 8 Abfragen (1 + 4 + 1 + 2) plus `set_basic` (1-2 Setzaufrufe) |
| Gleichzeitigkeit | `/MT:n`, Standard 8 (ML-ROBO) | Start Kerne × 4, mindestens 8, höchstens 16 (flow_registry.rs:25-26, 129-139; transfer/os/windows.rs:20); bei erkannter rotierender Platte Start 2 (flow_registry.rs:130-132) |

### 5.2 Befund und Hypothesen zum Faktor ≈ 2

Aus den Belegen lässt sich **nicht** beweisen, warum robocopy auf dem Messlauf etwa doppelt so schnell war. Belegt ist:

1. robocopy ist nicht die Obergrenze. BetterCopy ruft ebenfalls nur `CopyFileExW` auf, nutzt 16 Threads und erreicht auf derselben Messung 4.300-6.200 Dateien/s gegen 2.750-3.000 Dateien/s von `robocopy /MT:32` (BC bench/*.md; Selbstauskunft). Die Robocopy-Rate blieb unverändert, als Defender den Ordner ausnahm (34,37 s gegen 34,42 s), BetterCopy gewann 17,6 % (BC bench/benchmark_results-2.md).
2. Alle Werkzeuge, die Windows schnell kopieren (robocopy, FastCopy, BetterCopy, Ultracopier), legen das Ziel direkt unter dem Zielnamen an. Wir legen eine Stage an und benennen um. Das ist der strukturelle Unterschied je Datei, den die Quelltexte zeigen.
3. Gegenindiz: Ultracopier legt ebenfalls direkt an, nutzt bis zu 16 Threads und bleibt nach eigener Dokumentation unter robocopy (UC PIPELINING_DESIGN.md Abschnitt 1: Mehrfach-Threads hätten die Lücke zu robocopy „roughly halved“). Direkt-Anlegen allein reicht also nicht aus.

Hypothesen in der Reihenfolge ihrer Prüfbarkeit (jede ist mit der vorhandenen Messsuite entscheidbar, ich habe nichts ausgeführt):

| Nr. | Hypothese | Grundlage | Prüfung (Frage an die Messung) |
|---|---|---|---|
| H1 | Der Fluss startet auf dem Runner mit 2 statt 16, weil `volume_is_rotational` bei virtuellen Datenträgern `Some(true)` meldet (`IncursSeekPenalty`) | flow_registry.rs:129-132 (`None` = allgemeiner Start 2 bei rotierend); transfer/os/windows.rs:49-124; die Antwort des Runners ist nicht bekannt | Flusslimit nach den ersten 100 ms im Messprotokoll ausgeben; Rückgabewert von `volume_is_rotational` für das Zielvolumen |
| H2 | Die Arbeit ausserhalb der lesbaren Dateien (Paket-Absicht, `stage_proof`, `snapshot`, Belege nach Veröffentlichung, Quelle-unverändert-Prüfung) macht je Datei weiterhin viele Öffnungen | Analyse 2.3: ≈ 450 CreateFile je Datei im GUI-Weg vor M1-M4; Stand heute nicht lesbar | Zahl der `CreateFile`-Aufrufe je Datei im Messlauf zählen (Prozessüberwachung des Runners oder Zähler im Test) |
| H3 | Stage + Umbenennen verdoppelt Verzeichnisindex-Änderungen und USN-Ereignisse je Datei | Struktur (Beleg), Kosten (Annahme) | Messung mit und ohne Umbenennen (nur Messvariante, kein Lieferweg), `fsutil usn readjournal` auf dem Runner |
| H4 | Alle Worker schreiben in dasselbe Zielverzeichnis, NTFS serialisiert die Erzeugungen | BetterCopy-Hypothese (seed.md:363-369), kein Beleg | Dieselbe Messung mit 5.000 Dateien in 1 Ordner gegen 5.000 Dateien in 500 Ordnern |
| H5 | Der Virenscanner behandelt `robocopy.exe` (von Microsoft signiert) anders als unsere Datei | keine Quelle gefunden (Annahme) | Messung mit und ohne Defender-Echtzeitschutz auf einem Wegwerf-Runner |
| H6 | 8 Abfragen je Datei an offenen Handles (plus `set_basic`) kosten messbar | Zählung aus dem Quelltext (Beleg), Kosten klein (Annahme: 1-3 % je Datei) | Variante ohne `plain_local_source` messen (nur Messvariante) |

---

## 6. Offene Lücken

1. **Aufrufe je Datei auf unserer Seite nach den Änderungen** sind für beide Plattformen nicht vollständig bekannt: `staging.rs`, `safe_file.rs`, `publication_proof.rs`, `file_recovery*.rs` und `windows_optical.rs` waren nicht lesbar. Zahlen der Analyse (≈ 100 Aufrufe Linux CLI-Weg, ≈ 450 CreateFile Windows GUI-Weg) stammen vom Stand vor den Änderungen.
2. **Dünn besetzte Dateien auf Linux:** im lesbaren Code gibt es keine Löcherbehandlung, `copy_file_range` darf Löcher ausfüllen (MAN-CFR „NOTES“), `cp` und xcp erhalten sie (CU copy-file-data.c:480-533; XC libfs/linux.rs:78-83, 207-217). Ob `staging.rs` das abfängt, ist unbekannt. Mögliche Verschlechterung gegenüber `cp` für VM- und Datenbankdateien: Annahme, nicht geprüft.
3. **Zeitstempel auf Windows:** der Schnellweg setzt Zeiten und Attribute der Quelle auf das Ziel (`set_basic(&writer, Some(information))`, windows_fast.rs:238-241). Ob `CopyFile2` für die Erstellungszeit dasselbe tut, ist in der Dokumentation nicht belegt (ML-CF2, ML-CFEX schweigen); BetterCopy behauptet, `CopyFileExW` setze die Erstellungszeit auf „jetzt“ (BC seed.md:279-280, Selbstauskunft). Ein Unterschied zwischen kleinen Dateien (Schnellweg) und grossen (CopyFile2) wäre möglich. Nicht geprüft.
4. **ReFS und Dev Drive:** `CopyFile2` klont auf ReFS (ML-BC „natively in supported Windows copy operations“); der Schnellweg für ≤ 1 MiB prüft nur auf Netzwerkvolume, nicht auf ReFS (windows_fast.rs:176-184; windows_fast_probe.rs:35-54). Ob der Aufrufer das ausschliesst, ist nicht lesbar. Mögliche Folge: kleine Dateien auf ReFS werden byteweise statt geklont kopiert, anders als vor A7.
5. **Gewinnschätzungen** in Abschnitt 4 sind ungemessen. Die einzigen Zahlen in den Quellen sind Selbstauskünfte (BetterCopy, wcp, fcp, Ultracopier) mit unbekannter Vergleichbarkeit (eine Maschine, n=1, Defender-Zustand widersprüchlich beschrieben).
6. **FastCopy 4.x/5.x:** Quelltext nicht veröffentlicht oder nicht gefunden; Aussagen zu Abläufen stützen sich auf 3.63 (2019) und den Hilfetext; ob 5.12 Einzelheiten anders löst, ist unbekannt.
7. **TeraCopy und Windows-Explorer:** keine Quelltexte, keine technische Dokumentation gefunden; keine Aussage möglich. Welche `CopyFile2`-Flags der Explorer setzt, ist nirgends dokumentiert.
8. **robocopy:** Systemaufrufe je Datei, Verzeichnisaufzählung von Quelle und Ziel, Behandlung der Verzeichniszeiten: nicht belegt. Die Spalte „robocopy“ in 5.1 ist Annahme.
9. **Windows-Verzeichnisaufzählung unserer Entdeckung** (`walk_listers.rs`): ob Rust-std `read_dir` mit `DirEntry::metadata` oder `fs::metadata(pfad)` je Eintrag benutzt wird, ist nicht lesbar; jeder `fs::metadata(pfad)` kostet eine Öffnung (RUST-W windows.rs:1523-1535).
10. **Kernelverhalten**, auf das einzelne Schätzungen bauen, ohne dass ich Kernel- oder NTFS-Quelltext gelesen habe: Sperre des Elternverzeichnisses bei Erzeugen und Umbenennen (Linux `i_rwsem`), NTFS-Verzeichnisindex und USN-Ereignisse beim Umbenennen, Waisenliste von `O_TMPFILE` auf ext4, Filter-Verhalten von Defender bei Abfragen an offenen Handles, Sperrung von io_uring in Containern und auf Android. Alle sind **Annahme**.
11. **Aktueller Vergleich mit `cp -r` und `robocopy` nach den Änderungen:** liegt nicht vor; der Windows-Schnellweg ist ungemessen (Auftrag).
12. **Unbewertet geblieben:** Android (Pfad `linux_os.rs` gilt dort mit FUSE-Einschränkungen laut Analyse 3.3), Kopien über SMB/UNC (der Schnellweg schliesst sie aus, windows_fast.rs:177-178), optische Datenträger (Sonderweg `windows_optical.rs`).

## 7. Umsetzung im selben Release (Entscheidungen des Hauptagenten, 2026-10-29)

Umgesetzt, ohne weiteren Testlauf (Nutzervorgabe 2026-10-29):

| Technik | Quelle | Stelle |
|---|---|---|
| FILE_FLAG_SEQUENTIAL_SCAN auf Quelle und Stage | FastCopy fastcopyw.cpp | `native/src/copy/os/windows_fast.rs` |
| Leere Dateien ohne Lesen/Schreiben | FastCopy fastcopyr.cpp:1046 | `windows_fast.rs` |
| Volumen-Fakten je Seriennummer zwischengespeichert | eigene Ableitung (Abfragen je Datei, Abschnitt 5) | `windows_volume_facts.rs` |
| Blockklonen auf ReFS/Dev Drive über CopyFile2 | Microsoft Learn Block Cloning | `windows_fast.rs`, `windows_volume_facts.rs` |
| COPY_FILE_DISABLE_PRE_ALLOCATION bis 1 MiB, Rückfall ohne Flag | Microsoft Learn COPYFILE2_EXTENDED_PARAMETERS | `windows_kernel_copy.rs` |
| Ungepuffert ab 256 MiB (oder ¼ Speicher) | BetterCopy engine.rs, robocopy /J, Microsoft Learn | `copy/os/shared/copy_flags.rs` |
| Zeiten wie CopyFile2 (zur Laufzeit ermittelt, Learn schweigt) | Lücke 6.3 | `windows_time_rules.rs` |
| Start 8 auf als drehend gemeldeten Laufwerken | robocopy /MT-Standard 8 | `transfer/os/shared/flow_registry.rs` |
| copy_file_range mit bekannter Länge, std-Rückfälle | coreutils copy-file-data.c, Rust std kernel_copy | `copy/os/linux_plain.rs` |
| Löcher von Sparse-Dateien erhalten (SEEK_DATA/SEEK_HOLE) | coreutils copy-file-data.c:480-533 | `copy/os/linux_sparse.rs` |
| renameat2(NOREPLACE) ohne vorherigen Blick | coreutils, Abschnitt 4b Rang 5 | `copy/os/linux_publish.rs` |
| Einträge in Inode-Reihenfolge statt Hash-Reihenfolge | coreutils savedir SAVEDIR_SORT_FASTREAD | `native/src/local_access/os/linux_os.rs` |

Ausgeschlossen, mit Grund:
- Direkt unter den Zielnamen schreiben (robocopy, FastCopy, BetterCopy, Ultracopier, cp, xcp, fcp, wcp): bricht No-Replace, Stage und Absturz-Wiederaufnahme.
- SetFileValidData: legt alte Plattendaten offen.
- Defender-Ausnahmen: Sicherheitseinstellung des Nutzers.
- ODX-Sonderweg: nur T10-SAN; CopyFile2 nutzt ODX ohnehin.
- io_uring (wcp, Ultracopier): neue Abhängigkeit; in Android-Apps und Docker per seccomp gesperrt; ohne Testlauf nicht verantwortbar. Für einen späteren Batch mit Messung vorgemerkt.
- O_TMPFILE plus linkat: ändert den Journal- und Wiederaufnahmevertrag. Für einen späteren Batch mit Testlauf vorgemerkt.
- Eigene überlappte Warteschlange statt CopyFile2 (FastCopy): verliert Blockklonen und SMB-Offload; CopyFile2 arbeitet bereits überlappt.
- FindFirstFileExW(LARGE_FETCH): Der Windows-Lister liest schon stapelweise über GetFileInformationByHandleEx(FileIdExtdDirectoryInfo) mit 64 KiB bis 1 MiB Puffer, das ist mehr als LARGE_FETCH.
- Bündeln nach Zielordner (BetterCopy): Der Walker reiht die Dateien eines Ordners hintereinander ein, FIFO-Pakete sind daher schon meist einordnerig. Ein Umsortieren der Warteschlange hätte die Paketgröße gesenkt.
- d_type statt statx: Größe und Zeiten werden für Dateien und Ordner gebraucht; der Typ kommt schon aus d_type.

Nicht gemessen: Alle Gewinne sind Schätzungen aus Abschnitt 4. Messung im nächsten Batch mit Suite-Lauf.
