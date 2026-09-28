# Übertragungs-Engine – Umsetzung

Spec: `spec.md` · Recherche/Durchsatz-Analyse: `recherche.md` · Lesungen: `docs/lesungen/2026-09-28-*.md`
· Refs: `docs/refs/{windows-virtual-files,gdrive-ureq-throughput,quic-sftp-throughput}.md`.

## Regeln für alle Blöcke (verbindlich, zusätzlich zu AGENTS.md)

- Keine lokalen Builds/Tests (`cargo`, `rustc`, Gradle …). Erlaubt: Lesen, Editieren, rustfmt im
  Stdin-Modus als Syntax-/Formatprüfung:
  `sudo -n /root/.cargo/bin/rustfmt --edition 2021 --emit stdout < DATEI > TMP` (leere Ausgabe auf
  stderr = Syntax ok; Ergebnis zurückkopieren). Jede angefasste Rust-Datei wird vollständig
  rustfmt-formatiert (die Suite prüft angefasste Dateien).
- Crate-Quellen zum Nachschlagen: `sudo -n cat /root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/<crate>-<version>/…`
  (Versionen in `native/Cargo.lock`). Jede API gegen Quelle/Ref schreiben, nicht aus dem Gedächtnis.
- Neue/wesentlich geänderte Dateien < 500 Zeilen und < 50 KiB; vorher kohäsive Teile ausgliedern.
- `core/` bleibt plattformfrei; OS-Spezifisches nur in `os/{windows,linux_os,…}`-Adaptern.
- Keine `unwrap`/`expect`/`panic!` in Produktivpfaden; Fehler als `Result` mit deutscher Meldung im
  Stil der Umgebung.
- Tests: Präfix `transfer_engine_task_`, neben dem Code (bestehende Test-Muster der Module).
  Tests dürfen keine externen Dienste brauchen (Loopback-Server/Fixtures des Repos nutzen).
- Nicht committen, nicht pushen, keine Agents starten. Nur die eigenen Dateien ändern; fehlt etwas
  außerhalb des Blocks: im Bericht melden, nicht selbst ändern.
- Bericht: gelesene Dateien, geänderte/neue Dateien, Kernänderungen, Entscheidungen (mit Grund),
  offene Punkte, für die Suite zu startende Tests (Namen).

## Status

| Block | Inhalt | Zustand |
|---|---|---|
| W1 | Grundlagen: Trait-Erweiterungen, Flow-Regelung, Walker, Job-Typen, Auswahlquelle, externe Übergaben | fertig (unkompiliert) |
| A | Engine + lokale Kopie | offen |
| B | Share-/Agent-Protokoll: Pakete, Server-Kopie, Fenster, Anfrage-Grenzen | offen |
| C | Google Drive | offen |
| D | SFTP, SMB, FTP, WebDAV | offen |
| F | Windows: virtuelle Dateien für Remote (Zwischenablage, Ziehen) | offen |
| G | Sync: paralleles Spiegeln, Zwei-Wege mit Regelung | offen |
| H | App-Integration, Übertragungsfenster, Android | offen |
| T | Task-Suite, Workflow, Doku, Graph | offen |

## W1 Grundlagen (fertig) – Verträge für alle Blöcke

- `vfs::Backend` (Datei `native/src/vfs/core/core.rs`) neue Methoden mit Default:
  - `open_write_copy_stage_sized(&self, path, size) -> VfsResult<Box<dyn Write + Send>>` (Default:
    `open_write_copy_stage`). Überschreibende Writer müssen bei `flush` scheitern, wenn nicht genau
    `size` Bytes kamen.
  - `server_copy_to_stage(&self, src, stage, size) -> VfsResult<Option<u64>>` (Default `Ok(None)`).
  - `create_dir(&self, path) -> VfsResult<()>` (Default `mkdir_all`): genau eine Ebene, Eltern
    existieren, vorhandener echter Ordner = Ok.
  - `flow_key(&self, path) -> String` (Default: Schema + Zeiger), `transfer_ceiling(&self, path) ->
    Option<usize>` (Default `None`), `concurrent_read_write(&self) -> bool` (Default `true`).
  - `batch_limits(&self, dir) -> Option<BatchLimits>` (Default `None`), `put_batch(&self, &[BatchPut],
    &mut dyn Read) -> VfsResult<Vec<BatchPutOutcome>>`, `get_batch(&self, &[BatchGet], &mut dyn
    BatchSink) -> VfsResult<()>` (Default `Unsupported`). Typen in `vfs/core/batch.rs`, re-exportiert
    als `crate::vfs::{BatchGet, BatchLimits, BatchPut, BatchPutOutcome, BatchSink}`.
  - `CachingBackend` leitet alle weiter (Invalidierung bei Schreiboperationen), `UncBackend` leitet
    `create_dir`/`flow_key` an `LocalBackend` weiter; `LocalBackend`: `create_dir` (eine Ebene, keine
    Links), `flow_key` = Volume (`local:c:`, `unc://srv/share`, `local:dev<N>`).
  - Wrapper, die weitere Backends einpacken (`AgentBackend`→inner, `RootedBackend`, `MountProxy`),
    leiten nur weiter, wo der Block es braucht (Block B für Agent/Share).
- `crate::transfer` (neu, re-exportiert):
  - Flow: `flow(key, ceiling) -> Arc<Flow>`, `flow_for(&dyn Backend, path)`, `local_flow(path)`,
    `Flow::{acquire(&cancel) -> Option<FlowPermit>, try_acquire, has_spare, snapshot}`,
    `FlowPermit::{progress(bytes), finish(OpOutcome)}` (Drop = Failed), `acquire_pair(&a, Some(&b),
    &cancel) -> Option<PermitPair>`, `classify_error(&io::Error) -> OpOutcome`
    (`QuotaExceeded`/`TimedOut`/`WouldBlock` und Texte wie „too many concurrent“, „HTTP 429“ =
    Überlast). Backends melden Ratenlimits/„zu viele Verbindungen“ als `io::ErrorKind::QuotaExceeded`.
  - Walker `transfer::walk::walk(&dyn Lister, &[WalkRoot], &WalkOptions, &Arc<Flow>, &cancel,
    &(dyn Fn(WalkEvent) -> bool + Sync)) -> bool` mit `BackendLister`, `LocalLister` (eine
    Rechteanfrage je Walk), Ereignisse `Dir/File/Omitted/Problem`, Ordner vor Inhalt, Filtersemantik
    wie bisher (`RemoteFilterCtx`), Links/Spezialdateien/ungültige/doppelte Namen als `Problem`
    (Übertragung läuft weiter).
  - Job-Typen `Endpoint {Local, Remote(BackendHandle)}`, `JobItems {Roots{paths, base}, Pairs}`,
    `Layout {Tree, Flatten}`, `TransferJob {source, target, target_dir, items, layout, filter,
    conflict, mode, source_label, target_label}` mit `kind()`, `validate()`.
  - `TransferKind` + `Local`, `Move`; `TransferProgress` + `discovering, skipped, rate_bps, active,
    parallel, source, target` (Konstruktor unverändert).
  - `SelectionSource {backend, paths, filter, label}` mit `list_all(&cancel, &on_found) ->
    SelectionListing {entries: Vec<ListedEntry>, problems, omitted, complete}` und `open(&entry)`;
    `ListedEntry {rel, path, id, size, size_known, mtime_ms, is_dir}` (Ordner vor Inhalt, Export-Namen
    per `download_name`).
  - Externe Übergaben: `register_external(label) -> Arc<ExternalTransfer>` (`set_files_total`,
    `add_bytes`, `file_done`, `error`, `set_note`, `finish`), `external_snapshots()`.

## Block A – Engine und lokale Kopie (Agent, Opus)

**Dateien (eigen):** `native/src/transfer/**` (W1-Dateien nur bei echten Fehlern ändern und melden),
`native/src/copy/**`. **Nicht:** `app/`, `mobile/`, Backends.

**Ziel:** `transfer::engine::run_job(job, &tx, &cancel)` führt jeden `TransferJob` aus: sofortiger
Start, Walker parallel zu den Workern, adaptive Parallelität über Flows, alle Endpunkt-Kombinationen.

1. **Einstieg/Lane:** `TransferRequest::Job(Box<TransferJob>)` ergänzen (`kind/item_count/label/
   announcement/thread_name` aus dem Job; Ansage z. B. „⇄ Übertrage N Element(e) → Ziel…“). Alte
   Varianten (`Upload`, `UploadPairs`, `Download`, `RemoteCopy`) bleiben und werden auf Jobs
   abgebildet (Android nutzt sie). `upload_paths_progress`, `upload_pairs_progress`,
   `download_paths_progress`, `copy_remote_paths_progress` behalten Signaturen und delegieren.
   `upload_file`, `upload_reader_progress`, `download_to_id`, `download_clipboard_snapshot`,
   Temp-Funktionen bleiben (Öffnen/Bearbeiten/Import nutzen sie). Nicht mehr genutzte
   Sammel-Funktionen (`download_remote_clipboard_items`, `download_remote_paths_for_clipboard`,
   `RemoteEntryCollector`, `upload_plan::collect_*`) entfernen, sobald nichts mehr darauf zeigt
   (App-Aufrufer ersetzt Block H; Liste der entfernten Exporte im Bericht).
2. **Wurzeln/Namen:** `Roots{paths, base}` → relative Ziele (`base` = relativ wie
   `copy::relative::rel_from_root`, sonst Name; `Flatten` = Dateiname). Remote-Ziel: erste
   Pfadkomponente je Wurzel einmal frei reservieren (heutiges `DestinationNames`, „Name (2)“), danach
   nur Anlegen ohne Ersetzen. Lokales Ziel: Ordner zusammenführen, Dateien nach `conflict`.
   `Pairs`: Validierung wie heute `upload_pairs::collect_pairs` (keine `.`/`..`/`:`/leeren/Backslash/
   NUL-Komponenten, keine doppelten Ziele, keine Datei-und-Ordner-Kollision), Wurzelkomponenten wie
   oben reserviert.
3. **Discovery:** W1-Walker in eigenem Thread; Ereignisse in einen begrenzten Kanal (Kapazität so,
   dass Speicher begrenzt bleibt, Summen aber früh stimmen; Wahl begründen). `folders` = ungefiltert
   und `Tree`; Filter wie W1. Summen (`files_total/bytes_total`) = bisher gefunden, `discovering`
   bis Walker fertig. Lokale Quelle: `LocalLister` (eine Rechteanfrage; Ablehnung stoppt den Job mit
   klarer Meldung, bereits Kopiertes bleibt).
4. **Ordner:** Register „einmal anlegen“ (Single-Flight, Eltern zuerst): Remote `create_dir`, lokal
   mit Link-Schutz (`copy::path_guard`-Logik). Leere Ordner bei ungefilterten Bäumen wie heute.
5. **Worker:** wachsen auf Bedarf (Warteschlange nicht leer, Flow hat freie Erlaubnis), enden bei
   Leerlauf. Jede Datei-Operation hält eine Erlaubnis je beteiligter Verbindung (`acquire_pair`),
   meldet Bytes (`progress`) und Ergebnis (`finish(classify_error)`). Worker blockieren nie mit
   gehaltener Erlaubnis auf dem Kanal.
6. **Datei-Operationen:**
   - Lokal→Remote: Quelle mit Beobachtungsprüfung (vorher/nachher, Wachstum, wie `UploadSource`;
     Lesen über `local_access`), private Stufe ohne Existenzprobe (Zufallsname; `AlreadyExists` →
     neuer Name), `open_write_copy_stage_sized`, `promote_copy_stage`; Namenskonflikt beim
     Veröffentlichen → nummerierter Name (begrenzte Versuche wie heute). Stufen bei Fehlern wie heute
     nicht blind löschen (Meldung nennt die Stufe).
   - Remote→Lokal: `.part` im Zielordner, Länge nach `read_size` (Export-Dateien ohne Längenprüfung),
     Speicherplatz-Vorprüfung, Veröffentlichen per No-Replace-Umbenennung, Konfliktpolitik
     (`Rename`/`Skip`/`Overwrite`), **kein fsync für neue Dateien**, fsync vor `Overwrite`.
     `download_name` für Export-Dateien (Drive-Docs) auf jeder Ebene.
   - Remote→Remote: verschiedene Verbindungen → direkt streamen (Lese-Thread mit begrenztem Puffer,
     wenn sich das lohnt; Schwelle begründen); gleiche Verbindung → `server_copy_to_stage` wenn
     `Some`, sonst Streamen falls `concurrent_read_write()`, sonst Temp-Brücke (heutige Logik).
   - Lokal→Lokal: `copy::safe_file::transfer_file` je Datei mit: Kernel-Kopie (`std::fs::copy` in die
     frische Stufe; Rückfall auf die bisherige Schleife mit `local_access` bei Fehlern wie
     `PermissionDenied`), kein `sync_all`/`sync_parent` bei neuer Kopie, volle Synchronisation bei
     `Move` und `Overwrite`. Verschieben ungefilterter ganzer Wurzeln zuerst als eine
     No-Replace-Umbenennung (gleiches Volume, Ziel frei); sonst dateiweise mit Quarantäne und
     anschließendem Aufräumen leerer Quellordner (heutige Semantik).
   - Pakete: wenn `batch_limits` auf Ziel (Upload) oder Quelle (Download) `Some` liefert, kleine
     Dateien bündeln (Kleinheit und Paketgröße adaptiv aus gemessener Rate: Ziel ≈ 250 ms je Paket,
     innerhalb der Limits); Upload liest die Dateien vorab in den Speicher (mit Beobachtungsprüfung);
     Share→Share = `get_batch` → `put_batch` im Speicher. Paket-Ergebnis je Datei zählen.
     Mehrdeutiger Paketfehler (`Err`) → Dateien als Fehler „Ergebnis unbekannt“, nie blind wiederholen.
7. **Wiederholung/Abbruch:** je Datei höchstens eine Wiederholung bei vorübergehenden Fehlern
   (TimedOut, ConnectionReset/Aborted, BrokenPipe, UnexpectedEof, NotConnected, QuotaExceeded), nur
   solange nichts veröffentlicht wurde, mit kurzem Backoff mit Zufallsanteil. Leistungsschalter:
   viele Fehler in Folge ohne jeden Erfolg (Schwelle an die Zahl laufender Operationen gekoppelt,
   begründen) → Job endet mit klarer Meldung und letztem Fehler.
8. **Fortschritt:** alle ~150 ms: Summen, erledigte Dateien/Bytes (Bytes auch während laufender
   Dateien), `rate_bps` über gleitende ~3 s, bis zu 3 aktive Namen, `parallel` (laufende Operationen
   des Jobs), `discovering`, `skipped`, `omitted`, `errors` (Fehlerliste begrenzt wie heute),
   `source/target` aus dem Job. Abbruch: Walker und Worker stoppen an Blockgrenzen, lokale `.part`
   entfernt, `Done{canceled}`.
9. **copy-Modul:** `start_copy_expanded/from_paths/pairs` behalten Signaturen, laufen über
   `run_job` (Lokal→Lokal) und übersetzen `TransferMsg` in `CopyMsg` (Fehler als (Pfad, Text)).
   Einzel-Slot-Admission bleibt Sache der Aufrufer.
10. **Tests (Präfix):** Wurzel-/Paar-Namen, Konfliktpolitik lokal, Remote nie ersetzen, Filter,
    leere Ordner, Links als Problem, App-Papierkorb-Auslassung, Quelländerung, Abbruch räumt `.part`,
    Wiederholung nur vor Veröffentlichung, Leistungsschalter, Pakete mit Fake-Backend (inkl.
    mehrdeutigem Fehler), gleiche Verbindung Server-Kopie/Brücke, Verschieben als Umbenennung,
    parallele Worker (> 1 gleichzeitig bei freier Flow-Grenze), bestehende Tests angepasst.

## Block B – Share- und Agent-Protokoll (Agent, Opus)

**Dateien (eigen):** `native/src/share/core/{wire.rs, fs_response.rs, server.rs, server_transfer.rs,
server_capabilities.rs, fs_capabilities.rs, backend.rs, peer_request.rs, peer_read.rs, peer_writer.rs,
fs_copy.rs, keepalive.rs, blocking.rs}` + neue Dateien unter `share/core/` (z. B. `server_batch.rs`,
`peer_batch.rs`), `native/src/agent_proto/**`, `native/src/agent/core/**`,
`native/src/daemon/os/shared/{backend_server.rs, request_workers.rs, backend_transfer.rs}` + neue
Datei dort (z. B. `backend_batch.rs`). **Nicht:** `transfer/`, `app/`, andere Backends.

1. **Peer-Protokoll (Iroh):** neue `FsRequest`-Varianten für Paket-Upload (Kopfzeile mit Einträgen
   Pfad+Länge, danach die Bytes hintereinander als DATA-Frames, dann `WriteDone`; Host legt je Datei
   private Stufe exklusiv an, schreibt genau die Länge, veröffentlicht ohne Ersetzen, bei belegtem
   Namen nummeriert; Antwort mit Ergebnis je Datei) und Paket-Download (je Datei Kopf mit Länge oder
   Fehler, DATA, Ende mit Ergebnis inkl. Quelländerung; Abschluss). Autorisierung, Lease- und
   Pfadprüfung **je Eintrag** exakt wie Einzelanfragen; Grenzen je Paket (Anzahl, Bytes) serverseitig
   erzwungen; Abbruch/Stream-Reset hinterlässt keine halben Dateien (Stufe des laufenden Eintrags weg,
   Veröffentlichte bleiben). Fähigkeit: neues `#[serde(default)]`-Flag in
   `FsResponse::Capabilities`; Clients nutzen Pakete nur nach positivem Nachweis (einmal je
   Backend-Sitzung ermitteln, mit Rückfall bei Fehler).
2. **PeerBackend:** `batch_limits/put_batch/get_batch`, `server_copy_to_stage` (`CopyFile` in die
   Stufe; Frist nach Größe, weil der Host ohne Fortschritts-Frames lokal kopiert – Formel begründen),
   `transfer_ceiling` (QUIC-Stromgrenze der Verbindung abzüglich Reserve), `create_dir` (eine
   Anfrage). Einzel-Upload ohne überflüssige Roundtrips, wo sicher.
3. **QUIC-Fenster** (`keepalive.rs::iroh_transport_config`): Strom-Empfangsfenster, Verbindungs-
   Empfangsfenster und Sendefenster für große Bandbreite×RTT (Werte aus `recherche.md` §3.1 bzw.
   Ref, Speicherfolgen dokumentieren).
4. **Agent-Protokoll:** Paket-Frames (Upload/Download) im Codec; Fähigkeit per Hello-Versionslabel
   (`+batch-v1`), zusätzlich Label für die höhere Anfragegrenze; Client sendet neue Frames nur bei
   Label. `se-agent`-Server (`agent_proto/core/server.rs` + os/shared) behandelt Pakete auf dem
   lokalen Dateisystem mit denselben Sicherheitsregeln wie Einzel-Frames (No-Replace, Stufen,
   Link-Schutz, Root-Confinement). `MAX_ACTIVE_REQUESTS` 64.
5. **Hintergrund-Dienst:** `backend_server` behandelt die Paket-Frames durch Weiterreichen an
   `backend.put_batch/get_batch` (PeerBackend) mit Rückfall auf Einzeloperationen, wenn der Peer
   keine Pakete kann (der Client bekommt so immer Pakete, sobald der Dienst sie kann);
   `MAX_REQUEST_WORKERS` 64; Hello-Label wie oben.
6. **AgentBackend:** `batch_limits/put_batch/get_batch` über Frames (nur mit Label),
   `transfer_ceiling` aus der Anfragegrenze der Gegenstelle (neu 64, alt 16 Dienst/8 Agent) minus
   Reserve fürs Blättern, `server_copy_to_stage` (Copy-Frame), `flow_key`/`create_dir` sinnvoll.
7. **Tests:** Codec-Rundreise der neuen Frames; Share-Loopback (vorhandenes
   `share/core/copy_paste_task_fixture.rs`): Paket hoch/runter inkl. Konflikt-Nummerierung,
   Autorisierung je Eintrag, Abbruch mitten im Paket, alter Host ohne Flag → Einzelweg; Dienst-
   Weiterleitung; Agent-Server lokal.

## Block C – Google Drive (Agent, Opus)

**Dateien (eigen):** `native/src/gdrive/**`. **Nicht:** anderes.

1. Ein gepoolter `ureq::Agent` für alle API-Aufrufe (Metadaten, Mutationen, Upload-Sitzungen) mit
   `max_idle_connections_per_host` passend zur Parallelität (ureq-Standard ist 1 – Ref), gleiche
   TLS-/Timeout-Konfiguration wie heute; Antworten vollständig lesen, damit Verbindungen zurück in
   den Pool gehen. Kein automatisches Wiederholen von POST/PATCH (ureq tut es nicht; Ref).
2. Pfad-Cache entkoppelt persistieren: schmutzig markieren, höchstens alle paar Sekunden im
   Hintergrund schreiben, Serialisierung außerhalb der Sperren, kompaktes JSON; wo die Journal-Logik
   synchrones Speichern verlangt (`persist_path_cache_checked`), bleibt es synchron; beim letzten
   Klon/Programmende ausstehende Änderungen schreiben.
3. Globale Sperren `create_lock`/`mutation_lock` durch Schlüssel-Sperren (je Zielordner/Zielname,
   ggf. Quelle+Ziel in fester Reihenfolge) ersetzen, ohne die Duplikat-/Journal-Garantien zu
   schwächen (Begründung je Sperre im Bericht).
4. ID-Vorrat (`files.generateIds` mit größerem `count`), Multipart-Upload für kleine Dateien
   (Grenze laut Ref), Resumable mit größeren Blöcken für große; `open_write_copy_stage_sized` streamt
   ohne Temp-Spool wo möglich; Anzahl API-Aufrufe je Datei minimieren, ohne No-Replace-/
   Duplikat-Garantien zu ändern.
5. Backoff mit Zufallsanteil; Ratenlimit-Antworten als `io::ErrorKind::QuotaExceeded`.
6. Tests mit dem vorhandenen Fake-Drive-Server (`gdrive/core/gui_task_http.rs`,
   `*_tests.rs`): Pool-Wiederverwendung (eine Verbindung für viele Aufrufe), Cache-Schreiben
   gebündelt, parallele Ordneranlage ohne Duplikate, Multipart, Jitter-Grenzen, bestehende Tests grün.

## Block D – SFTP, SMB, FTP, WebDAV (Agent, Opus)

**Dateien (eigen):** `native/src/sftp/**`, `native/src/smb/**`, `native/src/ftp/**`,
`native/src/webdav/**`. **Nicht:** anderes.

1. **SFTP:** Lesen mit vielen ausstehenden READs (`RawSftpSession` auf eigenem Kanal, Blockgröße wie
   `File::poll_read`, Tiefe aus Kanalfenster; Ref), Kanal-Pool (weitere SFTP-Kanäle bei Bedarf, bis
   der Server ablehnt; gelernte Grenze als `transfer_ceiling`), SSH-Kanalfenster des Clients für
   Bandbreite×RTT (russh-Config prüfen), `create_dir` (eine Anfrage), `concurrent_read_write`
   begründet setzen. Reconnect-/Keepalive-Verhalten unverändert.
2. **SMB:** mehrere ausstehende Lese-/Schreibblöcke je Datei im Rahmen der Credits, Laufzeit-Threads
   nach Kernzahl, `create_dir`; Wiederhol-/Sicherheitslogik unverändert.
3. **FTP:** Verbindungs-Pool (Operationen leihen eine Verbindung; wächst auf Bedarf; Ablehnung
   421/530 → gelernte Grenze als `transfer_ceiling`, Fehlerart `QuotaExceeded`); ein Leser hält nur
   seine Verbindung; `concurrent_read_write` wahr, sobald ≥ 2 Verbindungen möglich sind, sonst falsch;
   `create_dir` (ein MKD); Keepalive/Reconnect je Verbindung wie heute.
4. **WebDAV:** gepoolte Verbindungen (`max_idle_connections_per_host` passend) für Lesen und für
   Mutationen, die ureq nie wiederholt (PUT mit Inhalt, MOVE, MKCOL, COPY); DELETE und leeres PUT
   ungepoolt; `open_write_copy_stage_sized` streamt PUT mit Content-Length und `If-None-Match: *`
   ohne Temp-Spool; 429/503 mit `Retry-After`/Backoff wiederholen, wo nachweislich nicht verarbeitet;
   `create_dir` (ein MKCOL, 405 → Prüfen); `server_copy_to_stage` (COPY Overwrite:F in die Stufe).
5. Tests je Backend mit vorhandenen Skript-/Loopback-Servern (z. B. `webdav/core/*task_tests.rs`,
   `ftp/core/connection_tests.rs`, `smb/core/tests.rs`): Streaming-PUT exakte Länge, gepoolte
   Mutation, 503-Wiederholung, FTP-Pool-Wachstum/Grenze, SFTP-Leser-Reihenfolge (Einheit).

## Block F – Windows: virtuelle Dateien für Remote (Agent, Opus)

**Dateien (eigen):** `native/src/virtual_clipboard/**`, `native/src/dragout/**`,
`native/src/app/os/windows/platform.rs` (nur neue Adapterfunktionen), `native/src/app/os/linux_os.rs`
(nur passende Stubs), `native/Cargo.toml` (nur `windows`-Features `Win32_System_Com_Marshal`,
`Win32_System_Threading`, falls nötig). **Nicht:** `transfer/`, `app/core`, `app/os/shared`.

1. Remote-Datenobjekt (FILEGROUPDESCRIPTORW + FILECONTENTS/ISTREAM + Preferred DropEffect) auf einem
   eigenen STA-Thread mit Nachrichtenschleife (endet, wenn es nicht mehr Zwischenablage-Besitzer ist
   bzw. beim Ziehen freigegeben wurde; nie `OleFlushClipboard`). Methoden threadsicher (Objekte sind
   agil, Ref §5).
2. Beschreiberliste erst bei der ersten Anfrage des Explorers: `SelectionSource::list_all`
   (W1), einmal, zwischengespeichert; Ordner mit `FD_ATTRIBUTES` + `FILE_ATTRIBUTE_DIRECTORY`, Größen
   nur wenn `size_known`, Zeiten; Einträge mit relativem Pfad ≥ 260 UTF-16-Einheiten weglassen und
   melden (externe Übergabe: `set_note` + Fehlerzähler).
3. Inhalte als sequentielle IStreams (Read/Stat/Seek(0/aktuell)), Vorausladen der nächsten Dateien
   in Listenreihenfolge parallel innerhalb eines Speicherbudgets (Werte begründen), große Dateien
   gestreamt mit Vorauslesepuffer.
4. `IDataObjectAsyncCapability` für Ziehen und Einfügen; Ziehen: GUI-Thread ruft `DoDragDrop` mit
   gemarshaltem Proxy (`CoMarshalInterThreadInterfaceInStream`/`CoGetInterfaceAndReleaseStream`).
5. Fortschritt über `register_external` (Label, Summen, Bytes, Dateien, Fehler, Notiz, `finish`).
6. Adapter: `set_remote_clipboard(SelectionSource) -> Result<u32, String>` (Sequenznummer nach dem
   Setzen), `drag_out_remote(SelectionSource) -> Result<DragOutOutcome, String>`; Linux-Stubs melden
   „nicht verfügbar“. Bestehende lokale virtuelle Zwischenablage und CF_HDROP bleiben unverändert.
7. Tests (nur Windows, `#[cfg(windows)]`): In-Prozess-OLE-Rundreise (Objekt setzen, Beschreiber
   lesen, Inhalte streamen, Verzeichnis-Einträge, Langpfad-Auslassung), Vorausladen mit Fake-Backend.

## Block G – Sync (Agent, Opus)

**Dateien (eigen):** `native/src/sync/**`, `native/src/bisync/os/shared/{apply.rs, snapshot.rs}`.
**Nicht:** sonstige bisync-Dateien außer zwingend nötig (melden).

1. Einweg-Spiegeln (`sync::start_sync`): Kopieren parallel zum Scannen mit Workern über Flows
   (`flow_for` je Seite, `acquire_pair`), Ordner einmal anlegen (eigenes Register oder
   `create_dir`-Kette), alle bisherigen Prüfungen je Datei unverändert (Quelle vorher/nachher,
   Ziel-Erwartung, Stufe, create/replace-Publikation), Auslassungen, Budget-Fehler, Löschdurchlauf
   erst nach fehlerfreiem Kopierdurchlauf. Fortschritt wie bisher (+ gleichmäßige Aktualisierung).
2. Zwei-Wege-Anwendung (`apply.rs`): Anzahl gleichzeitiger Aktionen über die Flows beider Seiten
   statt `min(parallelism)`, Obergrenze weiter `opts.max_transfers`, wenn gesetzt; Sicherheitslogik
   je Aktion unverändert. Walk (`snapshot.rs`) darf Flows für die Listenparallelität nutzen, Semantik
   (Budget, Duplikate, Auslassungen, Hash-Wiederverwendung) unverändert.
3. Tests: Parallel-Spiegeln gleiche Ergebnisse wie seriell (Fake-Backend), Löschdurchlauf-Sperre bei
   Fehlern, bestehende sync/bisync-Tests unverändert grün.

## Block H – App-Integration, Übertragungsfenster, Android (Agent, Opus)

**Dateien (eigen):** `native/src/app/core/**`, `native/src/app/os/shared/**` (ohne die F-Adapter),
`native/src/mobile/os/shared/{transfer.rs, drive.rs, edits.rs, import.rs}`,
`android/app/src/main/java/app/smartexplorer/android/ui/transfers/TransfersBar.kt`.

1. In-App-Zwischenablage für alle Plattformen: Strg+C/Kontextmenü speichert Quelle (lokal/Backend +
   Label), Einträge, Filter bzw. Paar-Schnappschuss (rekursive Filteransicht, aus dem Baum im
   Speicher), Ausschneiden (nur lokal). Windows: Veröffentlichung wie bisher für lokal (CF_HDROP bzw.
   virtuelle Dateien der gefilterten Auswahl im Hintergrund), für Remote über
   `set_remote_clipboard` (Block F). Gültigkeit über die Sequenznummer (auch während einer
   laufenden Hintergrund-Veröffentlichung, solange die Sequenz unverändert ist). Linux: intern.
   Kein Vorab-Download mehr, keine „bitte danach erneut einfügen“-Sperre.
2. Einfügen/Ablegen/„Kopieren nach…“/„Herunterladen nach…“ (Remote-Ziel aus der Ordnerauswahl →
   Remote→Remote) erzeugen `TransferRequest::Job` über die Lane; der Dialog „Kopieren nach…“
   schließt nach dem Start. Remote-Verschieben weiter explizit abgelehnt. Ziehen aus Remote in den
   Explorer über `drag_out_remote` (Block F), GUI blockiert nicht mehr.
3. Übertragungsfenster (Spec B/C): laufende, wartende, fertige Übertragungen und externe Übergaben;
   Fortschritt, Rate, Restzeit (nur ohne `discovering`), aktive Dateien, Parallelität,
   Fehlerliste zum Kopieren, Abbrechen, Zielordner öffnen, Entfernen. Statuszeilen-Chips bleiben,
   plus Knopf „⇅ Übertragungen (n)“. Fehler fertiger Übertragungen zusätzlich ins Fehler-Protokoll.
   Kontextmenüs zeigen Kopieren/Einfügen auch auf Linux.
4. Android: `fs.transfer` nutzt weiter die Lane-Varianten; `drive.rs` meldet während
   `discovering` die Nachricht „Suche Dateien… N gefunden“ (und entfernt/ersetzt sie danach);
   `TransfersBar.progressLine` zeigt eine vorhandene Nachricht laufender Aufgaben zusätzlich an.
   Lokal→Lokal über `copy::start_copy_*` (Block A) unverändert angebunden.
5. Tests: Zwischenablage-Zustand (Gültigkeit/Sequenz, Ausschneiden, Linux intern), Routing aller
   Kombinationen zu Jobs, Übertragungsliste (Modell, nicht Pixel), Android-Nachricht (Rust-Host-Test).

## Block T – Suite, Workflow, Doku (Hauptagent)

- Eine Suite `native/test-transfer-engine-task.sh` (+ Windows-Teil `…ps1`) über einen Workflow
  `transfer-engine-task.yml` mit Jobs Linux (Rust-Tests Präfix + betroffene Module, Share-/Agent-/
  WebDAV-/FTP-/SFTP-Loopback bzw. Container, rustfmt/clippy nur angefasste Dateien, Agent-Bundles aus
  der Quelle bauen), Windows (Präfix-Tests inkl. OLE, Windows-Target-Check), Android (Build + JVM +
  Gerätesuite `fs.transfer`). Timeout ≥ 30 min. Einmal auslösen, auswerten, fixen, erneut nur
  dieselbe Suite.
- Doku: README (Übertragungen/Explorer), `docs/TODO.md` (Batch + offene Android-Scan-Frage),
  `docs/ARCHITEKTUR.md` (Engine, Flow, Walker), neues `docs/TRANSFER_ENGINE.md` (Verhalten,
  Grenzen je Protokoll, Nachweise), `docs/RELEASING.md` falls Agent-Bundles betroffen.
- Graph-Neuaufbau nach AGENTS.md; Commits je Meilenstein mit `[task candidate]` als letzte Zeile.

## Abnahme (Gesamtablauf über die Suite)

- F1/F2: Kopieren speichert nur; Einfügen startet ohne Vorab-Scan (erste Datei vor Ende der Suche,
  Test mit langsamem Fake-Lister); alle Endpunkt-Kombinationen.
- F3: OLE-Rundreise Remote (Windows).
- F5: Parallelität > 1 wird genutzt und regelt zurück (Flow-Tests), Pakete über Loopback-Share,
  Drive-Pool/Cache, SFTP-Vorauslesen (Container), FTP-Pool, WebDAV-Streaming-PUT.
- F6: Fortschrittsfelder (discovering, rate, active, parallel) im Engine-Test.
- F7: Nie-Ersetzen, Stufen, Quelländerung, Wiederholung nur vor Veröffentlichung, Leistungsschalter,
  Abbruch ohne `.part`-Reste.
- F8: Android-Gerätesuite `fs.transfer` grün; Sync-Tests grün.
