# Übertragungs-Engine – Umsetzung

Spec: `spec.md` · Recherche/Durchsatz-Analyse: `recherche.md` · Lesungen: `docs/lesungen/2026-09-28-*.md`
· Refs: `docs/refs/{windows-virtual-files,gdrive-ureq-throughput,quic-sftp-throughput}.md`.
Plan-Kritik (28 Befunde, 2026-09-28) ist eingearbeitet; Befundnummern stehen als „K#“ an den Stellen.

## Regeln für alle Blöcke (verbindlich, zusätzlich zu AGENTS.md)

- Keine lokalen Builds/Tests (`cargo`, `rustc`, Gradle …). Erlaubt: Lesen, Editieren, rustfmt im
  Stdin-Modus als Syntax-/Formatprüfung:
  `sudo -n /root/.cargo/bin/rustfmt --edition 2021 --emit stdout < DATEI > TMP` (leere Ausgabe auf
  stderr = Syntax ok; Ergebnis zurückkopieren). Jede angefasste Rust-Datei wird vollständig
  rustfmt-formatiert (die Suite prüft jede im Batch geänderte Datei ganz).
- Crate-Quellen zum Nachschlagen: `sudo -n cat /root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/<crate>-<version>/…`
  (Versionen in `native/Cargo.lock`). Jede API gegen Quelle/Ref schreiben, nicht aus dem Gedächtnis.
- Neue/wesentlich geänderte Dateien < 500 Zeilen und < 50 KiB; vorher kohäsive Teile ausgliedern
  (mehrere Dateien liegen knapp darunter – im Block genannt).
- `core/` bleibt plattformfrei; OS-Spezifisches nur in `os/{windows,linux_os,…}`-Adaptern.
- Keine `unwrap`/`expect`/`panic!` in Produktivpfaden; Fehler als `Result` mit deutscher Meldung im
  Stil der Umgebung.
- **Zahlen:** Jede Konstante (Puffer, Parallelität, Fenster, Schwellen, Fristen) bekommt im Kommentar
  ihren Grund aus Protokoll, Plattform oder Messung. Keine geratenen Deckel; Parallelität regeln die
  Flows.
- **Erlaubnisse (K2):** Nie mit gehaltener Flow-Erlaubnis auf etwas warten, das selbst eine
  Erlaubnis, Speicher, eine Zugriffsfreigabe oder einen vollen Kanal braucht. Reihenfolge:
  Speicher reservieren → Erlaubnis(se) nehmen → Operation → `finish`. Ordneranlage nimmt eine
  Metadaten-Erlaubnis (`acquire_meta`), nie eine Datei-Erlaubnis. Lokal↔Remote regelt nur der
  Remote-Flow; der lokale Volume-Flow gilt nur für Lokal→Lokal.
- **Überlast (K13):** Ratenlimits/„zu viele Anfragen/Verbindungen“ als `vfs::congestion_error(msg,
  retry_after)` melden (nicht als Text, nicht als `QuotaExceeded`). `StorageFull`/`QuotaExceeded`
  sind Dauerfehler. Interne Wiederholschleifen dürfen Überlast nicht länger als eine Frist
  verstecken: die erste Überlast geht als Congestion an den Aufrufer.
- **Speicher (K7):** Alles, was Dateiinhalte puffert (Pakete, Vorausladen, nicht streamende Leser),
  reserviert vorher mit `transfer::reserve_memory`/`try_reserve_memory`.
- Tests: Präfix `transfer_engine_task_`, neben dem Code (bestehende Test-Muster der Module).
  Tests dürfen keine externen Dienste brauchen (Loopback-Server/Fixtures des Repos nutzen).
  Werden bestehende Tests umbenannt/entfernt: im Bericht nennen (Suite-Skripte pflegt T).
- Nicht committen, nicht pushen, keine Agents starten. Nur die eigenen Dateien ändern; fehlt etwas
  außerhalb des Blocks: im Bericht melden, nicht selbst ändern.
- Bericht: gelesene Dateien, geänderte/neue Dateien, Kernänderungen, Entscheidungen (mit Grund),
  Abweichungen vom Plan, offene Punkte, für die Suite zu startende Tests (Namen).

## Status

| Block | Inhalt | Zustand |
|---|---|---|
| W1 | Grundlagen und Verträge (unten) | fertig; ea7a8da, 8e47d9b, 4d2b6be, 315a00b, 6659839 |
| A | Engine + lokale Kopie + Lane | fertig; 704539f, ba79b8a, 1df938d (UNC-Serverkopie), 2678692 (Überlast als Gegendruck, Paketregeln, Download-Ziel einmal aufgelöst) |
| B1 | Share-Peer-Protokoll (Iroh): Pakete, Aufnahmegrenze, Server-Kopie, QUIC-Fenster | fertig; 704539f, 1acc4d6 (Zulassung je Prinzipal 60, Host 256, Fristen je Block, begrenzter Statusspeicher, Stufen-Aufräumen) |
| B2 | Agent-Protokoll, Dienst-Weiterleitung, AgentBackend, IPC-Flusskontrolle | fertig; 704539f, 9038053 (begrenzte Pakete/Frames, nur erzeugte Stufen verwerfen), Dauerfehler am Ziel (voll/Kontingent/schreibgeschützt) aus Agent-Texten erkannt; eingecheckte Agent-Binaries alt (neue Frames erst nach dem Release) |
| C | Google Drive | fertig; 704539f, ba79b8a |
| D | SFTP, SMB, FTP, WebDAV, ZIP | fertig; d7d3eb4, f3f1abd (`copy-data`/COPYCHUNK, Kopien ohne fsync/FLUSH, Abbrechen) |
| F | Windows: virtuelle Dateien für Remote (Zwischenablage, Ziehen) | fertig; 704539f, 7bdb662 |
| H | App-Integration, Übertragungsfenster, Android | fertig; 704539f, 315a00b |
| G | Sync: paralleles Spiegeln, Zwei-Wege mit Regelung | fertig; 5889a99, 310890d (Überlast abwarten, paralleles Einlesen mit gemeinsamem Stopp), 7bdb662 (Keep-both nur vor dem Commit wiederholen) |
| T | Task-Suite, Workflow, Doku, Graph | fertig: Suite `native/test-transfer-engine-task.sh` + `transfer-engine-task.yml` (16fad4d, Stufen laufen seit 082c774 alle durch), Doku `docs/TRANSFER_ENGINE.md`, Graph df20171; Release v0.5.166 (Runs 36525861000, 36535178233) |

Reviews (Sicherheit, Sync, Engine, COM) sind eingearbeitet (9038053 … 2678692). Suite-Läufe:
36516591958 und 36518722644 fanden Fehler (Tests, FTP-Upload seriell auf der Blätter-Verbindung,
Ordner vor Dateien, Sync-Index-Fallback, Stufen-Aufräumen hinter altem Agent), alle behoben;
36523120834 wurde auf Wunsch vor dem Ende des Linux-Jobs gestoppt: bis dahin alles grün bis auf den
SFTP-Download über die geformte Leitung (danach behoben in b495581, ungetestet); die Share-Raum-E2E
(parallele Downloads) blieb unbestätigt – offen als TODO TE2.

## W1 Grundlagen – Verträge (fest; Abweichungen melden)

**vfs** (`native/src/vfs/core/core.rs`, `batch.rs`, `congestion.rs`, `meta.rs`):
- Trait-Methoden mit Default: `open_write_copy_stage_sized(path, size)` (überschreibende Writer
  scheitern bei `flush`, wenn nicht genau `size` Bytes kamen) · `server_copy_to_stage(src, stage,
  size) -> Option<u64>` · `open_write_fresh(path, size) -> Option<Writer>` (einschrittiges Anlegen
  für ID-Provider in vom Job angelegten Ordnern, K6) · `open_read_at(path, id, offset) ->
  Option<Reader>` (Fortsetzen, K8) · `transfer_hint() -> Option<String>` (feste Grenze für die
  Anzeige) · `create_dir(path)` (eine Ebene, vorhandener echter Ordner = Ok) · `create_dir_new(path)`
  (`AlreadyExists` bei belegtem Namen; Default probt, Protokolle überschreiben exklusiv, K14) ·
  `discard_copy_stage(stage)` (nur eigene, nie veröffentlichte Stufe; sonst `Unsupported`, K17) ·
  `flow_key(path)` · `transfer_ceiling(path)` · `concurrent_read_write()` · `batch_limits(dir)`,
  `put_batch`, `get_batch` (Typen `BatchLimits{max_files,max_bytes}`, `BatchPut{path,size}`,
  `BatchPutOutcome{Published(String),Failed(io::Error)}`, `BatchGet{path,id,size}`, `BatchSink`).
- `vfs::{congestion_error(msg, retry_after) -> io::Error, congestion_of(&io::Error) ->
  Option<&Congestion>, Congestion{retry_after, message}}`.
- `CachingBackend` leitet alles weiter (Invalidierung bei Schreiboperationen); `UncBackend` und
  `LocalBackend` implementieren `create_dir`, `create_dir_new`, `discard_copy_stage`,
  `open_read_at`, `flow_key` (Volume).

**transfer** (re-exportiert aus `crate::transfer`):
- Flow: `flow(key, ceiling)`, `flow_for(&dyn Backend, path)`, `local_flow(path)`;
  `Flow::{acquire(&cancel), acquire_for(job, &cancel), acquire_meta(&cancel), try_acquire(),
  has_spare(), snapshot(), key()}`; `FlowPermit::{progress(bytes), finish(OpOutcome), abandon()}`
  (Drop = Failed); `acquire_pair(&one, Some(&other), job, &cancel) -> Option<PermitPair>` (zweite
  nur per `try_acquire`, sonst erste zurück und warten – kein Horten, K2); `ANONYMOUS_JOB`;
  `classify_error` (Congestion, TimedOut, WouldBlock, „too many concurrent“ = Überlast). Erlaubnisse
  gehen reihum zwischen Jobs (K24); Auflistungen haben einen reservierten Platz.
- Walker `walk::walk(&dyn Lister, &[WalkRoot{path,rel}], &WalkOptions, &Arc<Flow>, &cancel, &emit)
  -> bool`; `WalkOptions{filter, folders, flatten, access: Option<Arc<AccessGate>>,
  allow_backslash}` (`Default`); Ereignisse `Dir{path,rel}`, `File{path,rel,size,mtime_ms,id,md5}`,
  `Omitted{path}`, `Problem{path,message}`, `AccessRefused{path}` (Walk stoppt). Wurzeln: einzeln
  parallel per `stat`, bei mehr ausgewählten Einträgen eines Ordners als die Flow-Grenze eine
  Auflistung. Gleichnamige Dateien mit verschiedenen IDs (Drive) → beide, die weitere unter
  „Name (n)“ (lesen per ID, K15). Listers: `BackendLister(&dyn Backend)`, `LocalLister`.
- `AccessGate::new(root)`, `request() -> AccessAnswer{Granted, Refused, Unavailable(Option<String>)}`
  (einmal je Job, außerhalb jeder Erlaubnis; `Refused` beendet den Job, K16), `refused()`.
- Speicher: `memory_budget()`, `reserve_memory(bytes, &cancel)`, `try_reserve_memory(bytes)` →
  `MemoryReservation` (Budget = verfügbarer Speicher/4, 64 MiB … 2 GiB).
- Job: `Endpoint{Local, Remote(BackendHandle)}` mit `same_namespace()` (gleiche
  `namespace_identity`) und `case_sensitive(root)`; `PairItem{source, rel, size: Option<u64>,
  mtime_ms, id}` (`PairItem::new(source, rel)`); `JobItems{Roots{paths, base}, Pairs(Vec<PairItem>)}`;
  `Layout{Tree, Flatten}`; `TransferJob{source, target, target_dir, items, layout, filter, conflict,
  mode, source_label, target_label, resume: Option<Vec<ResolvedRoot>>}` mit `kind()`, `validate()`
  (lehnt auch Ziel in einer Quelle ab, K1), `source_containing_target()`; `path_within()`.
- Typen: `TransferProgress` + `note: Option<String>`, `log_path: Option<String>`;
  `TransferIssue{path, message}`; `ResolvedRoot{source, rel}`; `TransferMsg::Done{progress, errors,
  canceled, issues, roots}`.
- Lane (K9): keine feste Grenze. `TransferLane::new()`, `submit(request, launch) -> Result<(),
  String>`, `poll() -> Vec<FinishedTransfer{cancel_requested, outcome, issues, roots, job}>`,
  `cancel`, `cancel_all`, `shutdown`; `ActiveTransfer.job` hält den Job einer Engine-Übertragung
  (für „Fehlende übertragen“). `MAX_ACTIVE_TRANSFERS` existiert nur noch für Androids Task-Slots
  (H entfernt es). `TransferRequest::Job(Box<TransferJob>)` ruft
  `engine::run_job(job, &tx, &cancel)` (`transfer/os/shared/engine/mod.rs`, heute ein Stub, der
  klar scheitert).
- Auswahlquelle für Explorer-Übergaben: `SelectionSource{backend, paths, filter, label}` mit
  `list_all(&cancel, &on_found) -> SelectionListing{entries, problems, omitted, complete}` und
  `open(&entry)`; `ListedEntry{rel, path, id, size, size_known, mtime_ms, is_dir}`.
- Externe Übergaben: `register_external(label) -> Arc<ExternalTransfer>` (`set_files_total`,
  `add_bytes`, `file_done`, `error`, `set_note`, `finish`), `external_snapshots()`.

**App-Adapter** (Block F füllt Windows): `set_remote_clipboard(SelectionSource) -> Result<u32,
String>`, `drag_out_remote(SelectionSource) -> Result<DragOutOutcome, String>`,
`remote_clipboard_supported()` in `app/os/windows/platform.rs` (rufen
`crate::virtual_clipboard::set_remote_clipboard` bzw. `crate::dragout::drag_out_remote`, heute
Stubs in `virtual_clipboard/os/remote.rs`, `dragout/os/remote.rs`) und Linux-Stubs in
`app/os/linux_os.rs`.

## Block A – Engine, lokale Kopie, Lane (Agent, Opus)

**Dateien (eigen):** `native/src/transfer/**` (W1-Dateien nur bei echten Fehlern ändern und melden),
`native/src/copy/**`, `native/Cargo.toml` (nur `windows-sys`-Features, falls nötig).
**Nicht:** `app/`, `mobile/`, Backends. **Liefert G:** Ordner-Register (exportiert).

**Ziel:** `transfer::engine::run_job(job, &tx, &cancel)` führt jeden `TransferJob` aus: sofortiger
Start, Walker parallel zu den Workern, adaptive Parallelität über Flows, alle Endpunkt-Kombinationen.

1. **Einstieg:** `run_job` prüft `job.validate()`; bei Lokal→Lokal zusätzlich die kanonische
   Prüfung „Ziel in Quelle“ wie `copy::path_guard::validate_directory_target` (K1). Die Engine gibt
   dem Walker die angelegten Zielwurzeln als Ausschlussmenge (Schutz, falls der Namensraum doch
   überlappt). Alte Lane-Varianten (`Upload`, `UploadPairs`, `Download`, `RemoteCopy`) werden auf
   Jobs abgebildet; `upload_paths_progress`, `upload_pairs_progress`, `download_paths_progress`,
   `copy_remote_paths_progress` behalten Signaturen und delegieren. `upload_file`,
   `upload_reader_progress`, `download_to_id`, `download_clipboard_snapshot`, Temp-Funktionen
   bleiben. Nicht mehr genutzte Sammel-Funktionen (`download_remote_clipboard_items`,
   `download_remote_paths_for_clipboard`, `RemoteEntryCollector`, `upload_plan::collect_*`)
   bleiben, bis H sie nicht mehr aufruft; Liste im Bericht (T räumt am Ende auf).
2. **Wurzeln/Namen (K14):** relative Ziele wie bisher (`base` → `copy::relative::rel_from_root`,
   sonst Name; `Flatten` = Dateiname). Remote-Ziel: **eine** Auflistung des Zielordners je Job für
   alle Wurzelnamen, dann exklusives `create_dir_new` bzw. Stufen-Veröffentlichung, bei
   `AlreadyExists` „Name (n)“ (`vfs::remote_util::numbered_remote_name`). Lokales Ziel: Ordner
   zusammenführen, Dateien nach `conflict`. `Pairs`: Validierung wie `upload_pairs::collect_pairs`
   (keine `.`/`..`/`:`/leeren/Backslash/NUL-Komponenten, keine doppelten Ziele, keine
   Datei-und-Ordner-Kollision). **Fortsetzen (K8):** `job.resume` gibt die Zielwurzeln vor: keine
   neue Reservierung, vorhandene Ordner zusammenführen, vorhandene Dateien gleicher Größe
   überspringen (`skipped`), andere Größe → Problem „Ziel existiert mit anderer Größe – nicht
   ersetzt“. `Done.roots` meldet die aufgelösten Wurzeln jedes Laufs.
3. **Discovery:** W1-Walker in eigenem Thread; Ereignisse in einen begrenzten Kanal (Kapazität
   begründen: Speicher begrenzt, Summen früh). `folders` = ungefiltert und `Tree`; Filter wie W1;
   `allow_backslash` = Lokal→Lokal auf Unix (K26). Lokale Quelle: `LocalLister` + ein `AccessGate`
   je Job (auch für Datei-Öffnungen der Worker; `AccessRefused`/`Refused` beendet den Job mit
   klarer Meldung, bereits Kopiertes bleibt). Summen = bisher gefunden, `discovering` bis fertig.
4. **Ordner-Register (für A und G):** „einmal anlegen“ (Single-Flight je Pfad, Eltern zuerst):
   Remote `create_dir`, lokal mit Link-Schutz (`copy::path_guard`-Logik); nimmt `acquire_meta`,
   Worker warten ohne Erlaubnis auf ihren Elternordner (K2). Leere Ordner bei ungefilterten Bäumen
   wie heute. Als `pub(crate)`-API exportieren (G nutzt es, K23).
5. **Worker:** wachsen auf Bedarf (Warteschlange nicht leer und Flow hat freie Erlaubnis), enden bei
   Leerlauf. Erlaubnisse per `acquire_pair(src_flow, dst_flow, job_id, cancel)` nach der Regel
   „nur Remote-Flow bei Lokal↔Remote“. `progress(bytes)` beim Streamen, `finish(classify_error)`.
   Job-ID je `run_job` eindeutig (Fairness).
6. **Datei-Operationen:**
   - Lokal→Remote: Quelle mit Beobachtungsprüfung (vorher/nachher, wie `UploadSource`; Lesen über
     `local_access`); `open_write_fresh` in vom Job angelegten Ordnern, wenn `Some` (K6); sonst
     private Stufe ohne Existenzprobe (Zufallsname; `AlreadyExists` → neuer Name),
     `open_write_copy_stage_sized`, `promote_copy_stage`, Namenskonflikt → „Name (n)“ (begrenzte
     Versuche wie heute). Fehlgeschlagene/abgebrochene Stufen per `discard_copy_stage` entfernen;
     `Unsupported`/Fehler → Stufe gesammelt melden (K17).
   - Remote→Lokal: `.part` im Zielordner (bleibt bei Wiederholung offen: Fortsetzen per
     `open_read_at` ab dem geschriebenen Stand, K8), Länge nach `read_size` (Export-Dateien ohne
     Längenprüfung), Speicherplatz-Vorprüfung, No-Replace-Veröffentlichung, Konfliktpolitik,
     **kein fsync für neue Dateien**, fsync vor `Overwrite`. `download_name` auf jeder Ebene.
   - Remote→Remote: verschiedene Verbindungen → direkt streamen (Lese-Thread mit begrenztem
     Puffer, Größe begründen); gleiche Namensraum-Identität → `server_copy_to_stage` über das
     **Ziel**-Handle, sonst Streamen falls `concurrent_read_write()`, sonst Temp-Brücke.
   - Änderungserkennung Remote-Quelle (K21): gelesene Länge ≠ gelistete Größe → Fehler; Drive: MD5
     beim Streamen gegen `md5` aus dem Walker; sonst nachgelagertes `stat` (Größe/Zeit/ID) nach dem
     Lesen, bevor veröffentlicht wird.
   - Lokal→Lokal (K12): `copy::safe_file::transfer_file` je Datei, Kernel-Kopie **über Handles**:
     Linux `std::io::copy(&mut File, &mut File)` (nutzt `copy_file_range`) in die exklusiv erzeugte
     Stufe; Windows `CopyFile2` mit `COPY_FILE_FAIL_IF_EXISTS` auf einen frischen Zufallsnamen,
     danach Öffnen mit `FILE_FLAG_OPEN_REPARSE_POINT` und Identitätsprüfung
     (`path_matches_identity`); Rückfall auf die bisherige Schleife bei Fehlern wie
     `PermissionDenied`. Kein `sync_all`/`sync_parent` bei neuer Kopie; volle Synchronisation bei
     `Move` und `Overwrite`. Verschieben ungefilterter Wurzeln zuerst als eine No-Replace-Umbenennung
     (gleiches Volume, Ziel frei); sonst dateiweise mit Quarantäne und Aufräumen leerer
     Quellordner (heutige Semantik).
   - Pakete: wenn `batch_limits` (Ziel beim Upload, Quelle beim Download) `Some` liefert, kleine
     Dateien bündeln; Klein-Schwelle und Paketgröße adaptiv aus gemessener Rate (Ziel ≈ 250 ms je
     Paket) innerhalb der Limits. Upload streamt die Dateien nacheinander von der Platte in
     `put_batch` (kein Vorab-Puffer; Änderung der Quelle → Eintrag scheitert, K7); Download über
     `get_batch` mit `BatchSink` direkt in `.part`-Dateien. Paket-Ergebnis je Datei zählen;
     mehrdeutiger Paketfehler (`Err`) → Dateien „Ergebnis unbekannt“, nie blind wiederholen.
7. **Wiederholung/Abbruch:** je Datei höchstens eine Wiederholung bei vorübergehenden Fehlern
   (TimedOut, ConnectionReset/Aborted, BrokenPipe, UnexpectedEof, NotConnected, Congestion mit
   `retry_after`), nur solange nichts veröffentlicht wurde, Backoff mit Zufallsanteil (bzw.
   `retry_after`). Dauerfehler (`StorageFull`, `QuotaExceeded`, Zugriff verweigert am Ziel) beenden
   den Job mit Klartext. Leistungsschalter: viele Fehler in Folge ohne jeden Erfolg (Schwelle an die
   Zahl laufender Operationen gekoppelt, begründen) → Job endet mit klarer Meldung.
8. **Fortschritt/Protokoll (K20):** alle ~150 ms: Summen, Bytes auch während laufender Dateien,
   `rate_bps` über ~3 s, bis zu 3 aktive Namen, `parallel`, `discovering`, `skipped`, `omitted`,
   `errors`, `source/target`, `note` (`transfer_hint()` der Verbindung, „wartet auf <Verbindung>“,
   wenn der Flow ausgeschöpft ist und der Job nichts laufen hat). Jede Issue als JSON-Zeile
   (`{"path":…,"message":…}`) in eine Protokolldatei unter dem App-Datenordner
   (`crate::support_dirs` nutzen; Name mit Zeit und Job-ID), `log_path` ab der ersten Issue;
   `Done.issues` = die ersten 100, `errors` = deren Anzeigezeilen. Abbruch: Walker und Worker
   stoppen an Blockgrenzen, lokale `.part` entfernt, `Done{canceled}`.
9. **copy-Modul:** `start_copy_expanded/from_paths/pairs` behalten Signaturen, laufen über
   `run_job` (Lokal→Lokal) und übersetzen `TransferMsg` in `CopyMsg` (Fehler als (Pfad, Text)).
   Einzel-Slot-Admission bleibt Sache der Aufrufer.
10. **Tests:** Wurzel-/Paar-Namen, Ziel-in-Quelle (lokal kanonisch, Remote gleicher Namensraum über
    zwei Handles), Konfliktpolitik lokal, Remote nie ersetzen, Filter, leere Ordner, Links als
    Problem, App-Papierkorb, Quelländerung (Länge, MD5), Abbruch räumt `.part`, Wiederholung nur vor
    Veröffentlichung, Fortsetzen (`resume` überspringt, `open_read_at`), Leistungsschalter,
    Dauerfehler beendet Job, Pakete mit Fake-Backend (inkl. mehrdeutig), Server-Kopie/Brücke,
    Verschieben als Umbenennung, parallele Worker (> 1 gleichzeitig), tiefer Baum bei Grenze 1/2
    terminiert (K2), Fairness zweier Jobs auf einem Flow, Fehlerprotokoll-Datei, Zugriff abgelehnt
    beendet den Job.

## Block B1 – Share-Peer-Protokoll (Agent, Opus)

**Dateien (eigen):** `native/src/share/core/{wire.rs, fs.rs, fs_response.rs, server.rs,
server_transfer.rs, server_capabilities.rs, fs_capabilities.rs, backend.rs, peer_request.rs,
peer_read.rs, peer_writer.rs, fs_copy.rs, keepalive.rs, blocking.rs, types.rs}` + neue Dateien unter
`share/core/` (z. B. `server_batch.rs`, `peer_batch.rs`, `batch_wire.rs`). Knapp an 500 Zeilen:
`types.rs` (499), `fs.rs` (439), `server.rs` (422) – vorher ausgliedern. **Nicht:** `agent*`,
`daemon/`, `transfer/`, `app/`.

1. **Aufnahmegrenze (K4):** `MAX_BLOCKING_OPERATIONS` (blocking.rs) begründen oder durch eine
   begründete Grenze ersetzen; der Host meldet sie in `FsResponse::Capabilities` (neues
   `#[serde(default)]`-Feld); `PeerBackend::transfer_ceiling` folgt daraus (alte Hosts: bisheriges
   Verhalten). Wartende Öffnungen am Host melden „eingereiht“ statt die 60-s-Frist laufen zu lassen
   (oder die Client-Frist beginnt erst nach `Ready`) – Lösung begründen.
2. **Pakete:** neue `FsRequest`-Varianten Paket-Upload (Kopf mit Einträgen Pfad+Länge, **Teilung
   nach der Größe des kodierten Kopfs**, nicht nach Anzahl – K18a; danach die Bytes als DATA-Frames
   hintereinander, `WriteDone`) und Paket-Download (je Datei Kopf mit Länge oder Fehler, DATA,
   Ende mit Ergebnis inkl. Quelländerung; Abschluss). Host legt je Datei eine private Stufe
   exklusiv an, schreibt genau die Länge, veröffentlicht ohne Ersetzen, bei belegtem Namen
   nummeriert (gemeinsame Nummerierungsfunktion `vfs::remote_util::numbered_remote_name`, K18c).
   Autorisierung, Lease- und Pfadprüfung **je Eintrag** exakt wie Einzelanfragen; Grenzen je
   Paket serverseitig erzwungen; Abbruch/Stream-Reset hinterlässt keine halben Dateien. Ein Paket
   belegt einen Platz der Aufnahmegrenze. Client-Nonce im Stufennamen jedes Eintrags, damit ein
   mehrdeutiger Fehler per Statusabfrage („veröffentlicht als …“) geklärt werden kann (K18e);
   Fähigkeitsflag in `Capabilities`; Clients nutzen Pakete nur nach positivem Nachweis.
3. **PeerBackend:** `batch_limits/put_batch/get_batch`, `server_copy_to_stage` (`CopyFile` in die
   Stufe; Frist nach Größe begründen), `create_dir`/`create_dir_new` (eine Anfrage),
   `discard_copy_stage` (eigene Stufe), `open_read_at` (Offset), `flow_key` (Peer-Identität),
   `transfer_ceiling` (aus Aufnahmegrenze), Einzel-Upload ohne überflüssige Roundtrips, wo sicher.
   Ratenlimit/„busy“ als `congestion_error`.
4. **QUIC-Fenster** (`keepalive.rs::iroh_transport_config`): Strom-, Verbindungs-Empfangsfenster und
   Sendefenster für große Bandbreite×RTT (Werte aus `recherche.md` §3.1 bzw. Ref
   `quic-sftp-throughput.md`, Speicherfolgen dokumentieren; Speicherbudget beachten).
5. **Tests:** Codec-Rundreise der neuen Frames; Share-Loopback (`share/core/copy_paste_task_fixture.rs`):
   Paket hoch/runter inkl. Konflikt-Nummerierung, Autorisierung je Eintrag, Abbruch mitten im
   Paket, alter Host ohne Flag → Einzelweg, Aufnahmegrenze in Capabilities, Server-Kopie.

## Block B2 – Agent-Protokoll, Dienst, AgentBackend, IPC (Agent, Opus)

**Dateien (eigen):** `native/src/agent_proto/**`, `native/src/agent/core/**`,
`native/src/daemon/os/shared/{backend_server.rs, request_workers.rs, backend_transfer.rs,
ipc_client.rs, ipc_share_client.rs}` + neue Dateien dort (z. B. `backend_batch.rs`). Knapp an 500:
`agent_proto/core/server.rs` (492), `agent/core/mux.rs` (472), `daemon/os/shared/ipc_client.rs`
(471), `agent/core/backend.rs` (455) – vorher ausgliedern. **Nicht:** `share/`, `sftp/`,
`transfer/`, `app/`.

**Wichtig (K18d):** `agent_proto/**` wird auch in den abhängigkeitsfreien `se-agent` kompiliert (nur
std, rayon, libc; `se-agent/Cargo.toml`) – keine `crate::vfs`-Typen dort; eigene Protokolltypen.

1. **Head-of-Line (K3):** Kreditbasierte Flusskontrolle je Anfrage im Agent-Protokoll
   (`Frame::Credit{id, bytes}`, Hello-Label `+credit-v1`): ein Leser blockiert nie beim Zustellen,
   weil kein Sender mehr als seinen Kredit schickt; Kanäle ≥ Kredit. Gilt für GUI↔Dienst
   (`ipc_client.rs`/`backend_server.rs`) und Client↔`se-agent`. Alte Gegenstellen ohne Label:
   bisheriges Verhalten. Test: langsamer Verbraucher + parallele Auflistung → Auflistung < 1 s,
   Verbindung bleibt.
2. **Pakete:** Paket-Frames (Upload/Download) im Codec, Label `+batch-v1`; Client sendet sie nur
   bei Label. `se-agent`-Server behandelt Pakete auf dem lokalen Dateisystem mit denselben Regeln wie
   Einzel-Frames (No-Replace, Stufen, Link-Schutz, Root-Confinement); Upload streamt Einträge
   nacheinander (Trailer je Eintrag „gültig/geändert“ statt Vorab-Puffer, K7); Teilung nach
   Kopfgröße (K18a); Client-Nonce je Eintrag (K18e).
3. **Dienst-Weiterleitung:** `backend_server` reicht Paket-Frames an `backend.put_batch/get_batch`
   (PeerBackend, B1) weiter; kann der Peer keine Pakete, meldet der Dienst die Paketfähigkeit nicht
   (kein serielles Emulieren, K18b). Anfragegrenzen (`MAX_REQUEST_WORKERS`, `MAX_ACTIVE_REQUESTS`)
   begründen statt pauschal 64: sie folgen aus der Kreditsteuerung und dem Speicherbudget.
4. **Exec-Kanal-Pool (K5):** `AgentBackend` öffnet bei Bedarf weitere `sftp.open_exec_streams`-Kanäle
   (je Kanal eigenes SSH-Fenster), verteilt Anfragen über sie; Grenze = was der Server annimmt
   (MaxSessions) minus zwei Reserven (Haupt-SFTP-Kanal, kurzlebiger Posix-Rename-Kanal, K25);
   gelernte Grenze als `transfer_ceiling`.
5. **AgentBackend:** `batch_limits/put_batch/get_batch` über Frames (nur mit Label),
   `server_copy_to_stage` (Copy-Frame), `create_dir`/`create_dir_new`, `discard_copy_stage`,
   `open_read_at` (falls der Agent Offsets kann; sonst `None`), `flow_key` (Host+Nutzer),
   Congestion statt Text.
6. **Tests:** Codec-Rundreise; Kredit-Steuerung (langsamer Verbraucher blockiert andere nicht); alter
   Agent/Dienst ohne Label → Einzelweg; Agent-Server lokal: Paket inkl. Konflikt, Abbruch, Link;
   Pool-Grenze bei Ablehnung.

## Block C – Google Drive (Agent, Opus)

**Dateien (eigen):** `native/src/gdrive/**`. **Nicht:** anderes.

1. Ein gepoolter `ureq::Agent` für alle API-Aufrufe (Metadaten, Mutationen, Upload-Sitzungen) mit
   `max_idle_connections_per_host` passend zur Parallelität (ureq-Standard 1 – Ref), gleiche
   TLS-/Timeout-Konfiguration; Antworten vollständig lesen. Kein automatisches Wiederholen von
   POST/PATCH (ureq tut es nicht; Ref).
2. Pfad-Cache entkoppelt persistieren (schmutzig markieren, höchstens alle paar Sekunden im
   Hintergrund, Serialisierung außerhalb der Sperren); wo die Journal-Logik synchrones Speichern
   verlangt (`persist_path_cache_checked`), bleibt es synchron; beim letzten Klon/Programmende
   ausstehende Änderungen schreiben.
3. Globale Sperren `create_lock`/`mutation_lock` durch Schlüssel-Sperren (je Zielordner/Zielname,
   ggf. Quelle+Ziel in fester Reihenfolge) ersetzen, ohne Duplikat-/Journal-Garantien zu schwächen
   (Begründung je Sperre im Bericht).
4. **Schreibrate (K6):** `open_write_fresh`: einschrittiges Anlegen in vom Job angelegten Ordnern
   (Multipart ≤ 5 MB bzw. Resumable mit ID aus dem Vorrat `files.generateIds`), also ein
   Schreibaufruf je neuer Datei; `open_write_copy_stage_sized` streamt ohne Temp-Spool wo möglich;
   `transfer_hint()` = „Google Drive nimmt höchstens etwa 3 neue Dateien pro Sekunde an“ (Ref §2).
   `create_dir_new` über reservierte ID. `discard_copy_stage` per eigener ID. `open_read_at` per
   Range. Listings liefern `content_md5` (schon heute) – Walker trägt sie weiter.
5. **Überlast (K13):** Ratenlimits (`rateLimitExceeded`, `userRateLimitExceeded`, 429, 503) als
   `congestion_error` mit `Retry-After`, die **erste** ohne interne Wiederholung an den Aufrufer;
   `storageQuotaExceeded`/`dailyLimitExceeded` sind Dauerfehler (`QuotaExceeded`/`StorageFull`).
   Backoff mit Zufallsanteil dort, wo intern wiederholt wird (Lesen).
6. Tests mit dem Fake-Drive-Server (`gdrive/core/gui_task_http.rs`, `*_tests.rs`):
   Pool-Wiederverwendung (eine Verbindung für viele Aufrufe), Cache-Schreiben gebündelt, parallele
   Ordneranlage ohne Duplikate, ein Schreibaufruf je neuer Datei (`open_write_fresh`), Multipart,
   Congestion-Abbildung, Dauerfehler, bestehende Tests grün.

## Block D – SFTP, SMB, FTP, WebDAV, ZIP (Agent, Opus)

**Dateien (eigen):** `native/src/sftp/**`, `native/src/smb/**`, `native/src/ftp/**`,
`native/src/webdav/**`, `native/src/zipfs/**`. **Nicht:** anderes.

0. **Refs zuerst (K25):** russh-Fenster (`russh::client::Config`: `window_size`,
   `maximum_packet_size`, Kanalfenster) und suppaftp-Fehlerabbildung (421/530) aus den Crate-Quellen
   belegen und in `docs/refs/quic-sftp-throughput.md` bzw. einer neuen Ref `docs/refs/ftp-pool.md`
   festhalten (nur diese Dateien unter `docs/refs/` anlegen/ändern, INDEX-Zeile ergänzen).
1. **SFTP:** Lesen mit vielen ausstehenden READs (`RawSftpSession`, Blockgröße wie
   `File::poll_read`, Tiefe aus Kanalfenster; Ref), Kanal-Pool (weitere SFTP-Kanäle bei Bedarf;
   Grenze = was der Server annimmt minus zwei Reserven für Agent-Exec und Posix-Rename, K25;
   gelernte Grenze als `transfer_ceiling`), SSH-Fenster des Clients für Bandbreite×RTT (gilt auch
   für Agent-Exec-Kanäle, K5), `create_dir`/`create_dir_new` (eine Anfrage), `discard_copy_stage`,
   `open_read_at` (Offset), `concurrent_read_write` begründet. Reconnect/Keepalive unverändert.
2. **SMB:** mehrere ausstehende Lese-/Schreibblöcke je Datei im Rahmen der Credits,
   Laufzeit-Threads nach Kernzahl, `create_dir`/`create_dir_new`, `open_read_at`;
   Wiederhol-/Sicherheitslogik unverändert.
3. **FTP:** Verbindungs-Pool (Operationen leihen eine Verbindung; wächst auf Bedarf; Ablehnung 421
   bzw. 530 **nur nach einer erfolgreichen Anmeldung** als gelernte Grenze, K25; `congestion_error`);
   eine Verbindung bleibt dem Blättern reserviert (Pool-Grenze = gelernte Grenze − 1, K24);
   `concurrent_read_write` wahr ab ≥ 2 Verbindungen; `create_dir` (ein MKD), `create_dir_new`
   (MKD scheitert bei Existenz); `open_read_at` (REST); Keepalive/Reconnect je Verbindung.
4. **WebDAV:** gepoolte Verbindungen für Lesen und Mutationen, die ureq nie wiederholt (PUT mit
   Inhalt, MOVE, MKCOL, COPY); DELETE und leeres PUT ungepoolt; `open_write_copy_stage_sized` streamt
   PUT mit Content-Length und `If-None-Match: *` ohne Temp-Spool; 429/503 als `congestion_error` mit
   `Retry-After`; `create_dir` (MKCOL, 405 → Prüfen), `create_dir_new` (MKCOL auf freien Namen),
   `server_copy_to_stage` (COPY Overwrite:F in die Stufe), `open_read_at` (Range).
5. **ZIP (K7):** `ZipBackend::open_read` streamt Einträge (geparstes Archiv je Leser klonen statt neu
   parsen, kein `read_to_end`); was doch puffern muss, reserviert Speicher; `transfer_ceiling`
   begründen.
6. Tests je Backend mit vorhandenen Skript-/Loopback-Servern (`webdav/core/*task_tests.rs`,
   `ftp/core/connection_tests.rs`, `smb/core/tests.rs`, ZIP-Fixtures): Streaming-PUT exakte Länge,
   gepoolte Mutation, 503 → Congestion, FTP-Pool-Wachstum/Grenze/Reserve, 530 vor Anmeldung ≠
   Grenze, SFTP-Leser-Reihenfolge (Einheit), ZIP-Streaming ohne Vollpuffer.

## Block F – Windows: virtuelle Dateien für Remote (Agent, Opus)

**Dateien (eigen):** `native/src/virtual_clipboard/**`, `native/src/dragout/**`,
`native/src/app/os/windows/platform.rs` (nur die Remote-Adapter und neue Funktionen für F),
`native/src/app/os/linux_os.rs` (nur passende Stubs), `native/Cargo.toml` (nur `windows`-Features
`Win32_System_Com_Marshal`, `Win32_System_Threading`, falls nötig). **Nicht:** `transfer/`,
`app/core`, `app/os/shared`.

1. Remote-Datenobjekt (FILEGROUPDESCRIPTORW + FILECONTENTS/ISTREAM + Preferred DropEffect) auf einem
   eigenen STA-Thread mit Nachrichtenschleife (endet, wenn es nicht mehr Zwischenablage-Besitzer
   ist bzw. beim Ziehen freigegeben wurde; nie `OleFlushClipboard`). Objekte sind agil (Ref §5).
2. Beschreiberliste erst bei der ersten Anfrage des Explorers: `SelectionSource::list_all`, einmal,
   zwischengespeichert; Ordner mit `FD_ATTRIBUTES` + `FILE_ATTRIBUTE_DIRECTORY`, Größen nur wenn
   `size_known`, Zeiten; relative Pfade ≥ 260 UTF-16-Einheiten weglassen und melden. **Fehlerpfad
   (K19):** scheitert `GlobalAlloc` für den Deskriptor → `STG_E_MEDIUMFULL`, externe Übergabe mit
   Notiz „Auswahl zu groß für den Explorer – bitte in Smart Explorer einfügen“ und Fehler.
3. Inhalte als sequentielle IStreams (Read/Stat/Seek(0/aktuell)); Vorausladen der nächsten Dateien in
   Listenreihenfolge parallel innerhalb des Speicherbudgets (`transfer::reserve_memory`) und unter
   dem Flow der Verbindung; große Dateien gestreamt mit Vorauslesepuffer (Größe begründen).
4. `IDataObjectAsyncCapability` für Ziehen und Einfügen (Explorer extrahiert dann im Hintergrund,
   sein UI-Thread wartet nicht auf die Auflistung – K19; in Tests prüfen, was prüfbar ist, Rest als
   offene Beobachtung melden). Ziehen: GUI-Thread ruft `DoDragDrop` mit gemarshaltem Proxy
   (`CoMarshalInterThreadInterfaceInStream`/`CoGetInterfaceAndReleaseStream`).
5. Fortschritt über `register_external` (Label, Summen, Bytes, Dateien, Fehler, Notiz, `finish`).
6. **Für andere Programme bereitstellen (K10):** neue Windows-Funktion
   `set_clipboard_files_after_download(...)` ist **nicht** nötig: H lädt über die Engine herunter
   und setzt danach CF_HDROP mit dem vorhandenen `set_clipboard_files`-Adapter. F liefert nur die
   virtuellen Wege.
7. Adapter wie W1; Linux-Stubs melden „nicht verfügbar“. Bestehende lokale virtuelle Zwischenablage
   und CF_HDROP bleiben unverändert.
8. Tests (nur Windows, `#[cfg(windows)]`): In-Prozess-OLE-Rundreise (Objekt setzen, Beschreiber
   lesen, Inhalte streamen, Verzeichnis-Einträge, Langpfad-Auslassung, Deskriptor-Fehlerpfad),
   Vorausladen mit Fake-Backend innerhalb des Budgets.

## Block G – Sync (Agent, Opus; Welle 2)

**Dateien (eigen):** `native/src/sync/**`, `native/src/bisync/os/shared/{apply.rs, snapshot.rs}` +
neue Dateien dort. Knapp an 500: `apply.rs` (460), `snapshot.rs` (473) – vorher ausgliedern.
**Nicht:** sonstige bisync-Dateien außer zwingend nötig (melden).

1. Einweg-Spiegeln (`sync::start_sync`): Kopieren parallel zum Scannen mit Workern über Flows
   (`flow_for` je Seite, `acquire_pair`, Regeln wie A), Ordner über das Ordner-Register von A (K23),
   **Zielordner einmal je Quellordner auflisten** und vergleichen statt `stat` + `mkdir_all` je Datei
   (auf FTP ist `stat` eine LIST des Elternordners → bisher quadratisch); das Prüf-`stat` vor dem
   Veröffentlichen bleibt. Alle bisherigen Prüfungen je Datei unverändert (Quelle vorher/nachher,
   Ziel-Erwartung, Stufe, create/replace-Publikation), Auslassungen (Links/Junctions als geschützte
   Auslassung mit Gegenstücken und Baseline-Einträgen, AGENTS.md), Budget-Fehler, Löschdurchlauf erst
   nach fehlerfreiem Kopierdurchlauf. Fortschritt wie bisher (+ gleichmäßige Aktualisierung).
2. Zwei-Wege-Anwendung (`apply.rs`): gleichzeitige Aktionen über die Flows beider Seiten statt
   `min(parallelism)`, Obergrenze weiter `opts.max_transfers`, wenn gesetzt; Sicherheitslogik je
   Aktion unverändert. Walk (`snapshot.rs`) darf Flows für die Listenparallelität nutzen, Semantik
   (Budget, Duplikate, Auslassungen, Hash-Wiederverwendung) unverändert.
3. Flows sind prozesslokal: Sync-Jobs im Dienst regeln getrennt von der GUI (K24) – dokumentieren.
4. Tests: Parallel-Spiegeln gleiche Ergebnisse wie seriell (Fake-Backend), Link-Auslassung parallel,
   Löschdurchlauf-Sperre bei Fehlern, Auflistung statt `stat` je Datei (Aufrufzähler), gleiche
   Relativpfade auf zwei Remotes getrennt, bestehende sync/bisync-Tests grün.

## Block H – App-Integration, Übertragungsfenster, Android (Agent, Opus)

**Dateien (eigen):** `native/src/app/core/**`, `native/src/app/os/shared/**` (ohne die
F-Adapter), `native/src/mobile/**`, `android/app/src/main/java/app/smartexplorer/android/ui/transfers/**`.
Knapp an 500: `app/core/state.rs` (493), `app/core/frame_keyboard.rs` (490),
`app/core/remote_context_menu.rs` (489) – vorher ausgliedern. **Nicht:** `transfer/`, Backends,
`virtual_clipboard/`, `dragout/`.

1. **Zwischenablage für alle Plattformen:** Strg+C/Kontextmenü speichert Quelle (lokal/Backend +
   Label), Einträge, Filter bzw. Paar-Schnappschuss (`PairItem` mit Größe/Zeit/ID aus dem Baum im
   Speicher), Ausschneiden (nur lokal). Windows: lokal wie bisher (CF_HDROP bzw. virtuelle Dateien
   der gefilterten Auswahl), Remote über `set_remote_clipboard` (F). Gültigkeit über die
   Sequenznummer. Linux: intern. Kein Vorab-Download, keine „bitte danach erneut einfügen“-Sperre.
2. **Für andere Programme bereitstellen (K10, Windows):** Kontextmenü-Befehl für Remote-Auswahl:
   Engine-Download (Job) in einen Temp-Ordner der Sitzung mit Fortschritt in der Übertragungsliste,
   danach CF_HDROP dieser Dateien (vorhandene Adapter); ersetzt den bisherigen impliziten
   Vorab-Download ausdrücklich.
3. **Einfügen/Ablegen/„Kopieren nach…“/„Herunterladen nach…“** erzeugen `TransferRequest::Job`
   (Remote-Ziel aus der Ordnerauswahl über dieselbe Standort-/Verbindungsgrenze wie das Sync-Setup
   auflösen; „verbindet…“ als erster Zustand, klarer Fehler bei nicht erreichbarem Ziel; gleiche
   Relativpfade auf zwei Remotes getrennt – K28). „Kopieren nach…“ schließt nach dem Start.
   Remote-Verschieben weiter abgelehnt. Ziehen aus Remote in den Explorer über `drag_out_remote`.
4. **Übertragungsfenster (Spec B/C):** laufende und fertige Übertragungen und externe Übergaben
   (beim ersten Sehen übernehmen, damit Hinweise nicht mit dem Weak-Handle verschwinden, K20);
   Fortschritt, Rate, Restzeit (nur ohne `discovering`), aktive Dateien, Parallelität, `note`,
   Fehlerliste (`issues`) mit „Alle Fehler kopieren“ und „Protokoll öffnen“ (`log_path`),
   „Fehlende übertragen“ (neuer Job aus `FinishedTransfer.job` mit `resume = roots`), Abbrechen,
   Zielordner öffnen, Entfernen. Statuszeilen-Chips bleiben, Knopf „⇅ Übertragungen (n)“. Fehler
   fertiger Übertragungen zusätzlich ins Fehler-Protokoll. Kontextmenüs zeigen Kopieren/Einfügen auch
   auf Linux.
5. **Android:** `fs.transfer` und gefilterte Uploads als `TransferRequest::Job` mit `Roots`, Filter
   und `base` (kein `collect_recursive`-Vorab-Scan; Paare nur für Schnappschüsse im Speicher, K22);
   Task-Slots (`mobile/core/slots.rs`, `MAX_ACTIVE_TRANSFERS`) entfernen, weil die Flows regeln (K9);
   `drive.rs` meldet während `discovering` „Suche Dateien… N gefunden“; `TransfersBar.progressLine`
   zeigt eine vorhandene Nachricht laufender Aufgaben zusätzlich an. Lokal→Lokal über
   `copy::start_copy_*` (A) unverändert angebunden. Die Android-Scan-Frage (Windows-artiges Scannen
   auf Android richtig?) ist nicht Teil dieses Batches (T trägt sie in `docs/TODO.md` ein).
6. **Tests:** Zwischenablage-Zustand (Gültigkeit/Sequenz, Ausschneiden, Linux intern), Routing
   aller Kombinationen zu Jobs, „Fehlende übertragen“ baut den richtigen Job, Übertragungsliste
   (Modell), externe Übergabe bleibt nach Ende sichtbar, Android-Nachricht und Job-Routing
   (Rust-Host-Test), Remote-Ziel aus der Ordnerauswahl (zwei Remotes, gleicher Relativpfad).

## Block T – Suite, Workflow, Doku (Hauptagent)

- Eine Suite `native/test-transfer-engine-task.sh` (Modus `--check` = enger Compile-Check; ohne =
  volle Suite) über `transfer-engine-task.yml` mit Jobs Linux (Rust-Tests Präfix + betroffene
  Module, Share-/Agent-/WebDAV-/FTP-/SFTP-Loopback bzw. Container, rustfmt/clippy nur Batch-Zeilen,
  Agent-Bundles aus der Quelle), Windows (Präfix-Tests inkl. OLE, Windows-Target), Android (Build +
  JVM + Gerätesuite `fs.transfer`). Timeout ≥ 30 min. Einmal auslösen, auswerten, fixen, erneut
  nur dieselbe Suite.
- **Durchsatz messen (K11):** Suite-Schritt mit `tc netem` (z. B. 50 ms, 100 Mbit/s) auf Loopback mit
  OpenSSH-Container, Share-Loopback und Fake-Drive; 10 000 × 4 KiB und 1 × 1 GiB (bzw. kleiner, wenn
  die Runner-Zeit es verlangt – begründen): erste Datei < 1 s, Kleindateien deutlich schneller als
  der alte Pfad (Faktor messen und berichten), Großdatei ≥ ~80 % der Rohrate, Drive-Schreibaufrufe
  je Datei = 1, Regler mit ±10 % Rauschen ≥ 80 % des Optimums (Einheitstest vorhanden).
- Bestehende Suite-Listen pflegen (umbenannte/entfernte Tests in `android/test-android-task.sh`,
  `native/test-filter-transfer-task.sh` usw.).
- Doku: README (Übertragungen/Explorer), `docs/TODO.md` (Batch + offene Android-Scan-Frage),
  `docs/ARCHITEKTUR.md` (Engine, Flow, Walker), neues `docs/TRANSFER_ENGINE.md` (Verhalten, Grenzen
  je Protokoll, Nachweise), `docs/RELEASING.md` falls Agent-Bundles betroffen.
- Graph-Neuaufbau nach AGENTS.md; Commits je Meilenstein mit `[task candidate]` als letzte Zeile.

## Abnahme (Gesamtablauf über die Suite)

- F1/F2: Kopieren speichert nur; Einfügen startet ohne Vorab-Scan (erste Datei vor Ende der Suche,
  Test mit langsamem Fake-Lister); alle Endpunkt-Kombinationen; Ziel in Quelle abgelehnt.
- F3: OLE-Rundreise Remote (Windows), Deskriptor-Fehlerpfad.
- F5: Parallelität > 1 wird genutzt und regelt zurück (Flow-Tests), Pakete über Loopback-Share und
  Agent, Drive-Pool/Cache/ein Schreibaufruf je Datei, SFTP-Vorauslesen (Container), FTP-Pool,
  WebDAV-Streaming-PUT, netem-Messung (K11).
- F6: Fortschrittsfelder (discovering, rate, active, parallel, note, log_path) im Engine-Test,
  Fehlerprotokoll-Datei.
- F7: Nie-Ersetzen, Stufen, Quelländerung, Wiederholung nur vor Veröffentlichung, Fortsetzen,
  Leistungsschalter, Abbruch ohne `.part`-Reste, Zugriff abgelehnt beendet den Job.
- F8: Android-Gerätesuite `fs.transfer` grün; Sync-Tests grün; gespeicherte Orte, Ordnerauswahl und
  Remote-Pfade unverändert (bestehende Tests).
