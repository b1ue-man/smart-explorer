# Analyse G3: Netz-Backends (SFTP, SSH-Agent, SMB, FTP/FTPS, WebDAV, Google Drive, entfernt → entfernt)

Stand: 2026-10-28. Quellstand: eingefrorene Kopie von `a67d7296`. Teil G3 von `analyse-plan.md`.
Methode: nur Schritt 1 des Plans (statisch, Code mit `Datei:Zeile`, alle Pfade relativ zu `native/src/`).
Nichts wurde gebaut, getestet oder ausgeführt. Alle Zeit- und Ratenangaben sind **Schätzungen aus
dem Code mit den in Abschnitt 1 genannten Annahmen**; sie gelten erst nach den Messläufen von G5.
Quellen der Bibliotheken (russh, russh-sftp, smb2, suppaftp, ureq) liegen nicht im Quellbaum; was
über ihr Innenleben gesagt wird, stammt aus `docs/refs/*.md` des Projekts oder ist als **Annahme**
markiert. `analyse-kopieren.md` (Engine, Belege, Flow) wurde an den hier benutzten Stellen am Code
nachgeprüft; Abweichungen stehen in Abschnitt 2.

---

## 0. Die wichtigsten Befunde (Rangfolge nach Wirkung)

1. **Defekt (statisch abgeleitet): Paket-Downloads von SFTP und SMB brechen in jedem GUI- und
   Android-Auftrag ab.** `get_batch` läuft in `rt.block_on(get_all(..))`
   (`sftp/core/batch_get.rs:46`, `smb/core/batch_wire.rs:448`); die Engine-Senke veröffentlicht
   jede Datei innerhalb dieses Aufrufs (`transfer/os/shared/engine/batch_get.rs:350` →
   `download.rs:338ff` → `file_recovery.rs:73` → `job_store_validation.rs:30-31` →
   `SftpBackend::stat` → `connection.rs:83-90` `runtime.block_on`, bzw. `SmbBackend::stat` →
   `session.rs` `self.rt.block_on`). Ein zweites `block_on` im selben Faden lässt tokio mit
   „Cannot start a runtime from within a runtime“ abbrechen. Der Beleg-Pfad existiert nur für
   `recoverable`-Aufträge (`job_store.rs:82`); die Containertests laufen mit `recoverable: false`
   (`native/tests/transfer_task/support.rs:126`), deshalb fiel es nicht auf. Ein Worker-Panik wird
   von `std::thread::scope` (`engine/dispatch.rs:45`) an den Lane-Faden weitergereicht
   (`panic = "unwind"`, `native/Cargo.toml:419`).
2. **Die Belege (Crash-Log) kosten bei Netz-Backends zusätzlich 3 serielle Rundreisen je Datei**
   (Stage-`stat`, Ziel-`stat` vor, Ziel-`stat` nach der Veröffentlichung) plus 4 dauerhafte lokale
   Schreibvorgänge (3 davon unter einer Sperre je Auftrag, siehe `analyse-kopieren.md` 2.).
   In Paketen laufen diese Schritte **je Mitglied nacheinander** vor und nach dem einen
   Paket-Aufruf (`engine/batch_put.rs:76-84`, `:130-147`): ein SFTP-Paket mit 64 Dateien kostet
   im GUI 2 × 64 serielle `LSTAT`-Rundreisen statt ≈ 4. Folge: GUI ≈ 10–15-mal langsamer als
   `se cp` bei kleinen Dateien (Abschnitte 3–5).
3. **FTP listet für jede veröffentlichte Datei den ganzen Zielordner** (Abwesenheitsprüfung
   `ftp/core/metadata.rs:338`, Veröffentlichung `ftp/core/staging.rs:149`), im GUI zweimal, und
   alle `stat` laufen über **eine** Steuerverbindung (`ftp/core/ftp.rs:247-248`). Bei 5000 Dateien
   in einem Ordner: O(n²) Listing-Bytes (≈ 0,9 GB `se cp`, ≈ 1,75 GB GUI) und im GUI eine globale
   Serialisierung von ≈ 8 Rundreisen je Datei.
4. **Mittlere Dateien (≈ 1 MiB) gehen nie in Pakete**: die Grenze ist `Rate × 250 ms ÷ 8`
   (`transfer/core/engine_policy.rs:40-45`, `:206-217`), und die gemessene Paketrate kleiner Dateien
   ist rundreisegebunden (Abschnitt 2.4). Jede 1-MiB-Datei zahlt den Einzelweg (SFTP: 5 RT `se cp`,
   8 RT GUI; SMB 5/8; FTP 9/18).
5. **Jede Datei beginnt ihr Pipelining bei Tiefe 2** (`transfer/os/shared/read_pipeline.rs:31`,
   neuer `Pipeline` je Datei in `sftp/core/pool_writer.rs:362`, `pool_reader.rs:201`,
   `smb/core/reader.rs:84`): eine 1-MiB-Datei braucht ≈ 3 Datenrundreisen statt 1.
6. **Google Drive**: neue Dateien sind serverseitig auf ≈ 3 je Sekunde begrenzt
   (`gdrive/core/backend.rs:262`); se drosselt nur reaktiv über Fehler 403/429
   (`gdrive/core/overload.rs:29-39`). Der Weg in einen bestehenden Ordner (Stufe + Umbenennen)
   braucht 2 schreibende Aufrufe je Datei statt 1 (`gdrive/core/sized_writer.rs`, `promotion.rs:235`).
   Große Uploads senden Abschnitte von 8 bis 64 MiB im Stop-and-wait und lesen die Quelle nicht
   parallel dazu (`gdrive/core/chunk_stream.rs:51-126`).
7. **Ein 10-GiB-Download von FTP läuft fast immer als ein einziger TCP-Strom**: Bereiche brauchen
   2 bereits angemeldete freie Verbindungen (`ftp/core/pool.rs:35`, `:56-67`), und Bereiche dürfen
   keine neue Anmeldung auslösen (Sicherheitsantwort S12). Ein frischer Auftrag hat keine.
8. **Entfernt → entfernt kennt keine Pakete** (`engine/mod.rs:242-247`); jede kleine Datei läuft
   einzeln, bei gleicher SFTP-Gegenstelle mit ≥ 6 seriellen Rundreisen allein für `copy-data`
   (`sftp/core/copy_data.rs:157-196`, beide `OPEN` und beide `CLOSE` nacheinander).

---

## 1. Maßstab und Modellannahmen

Ziel (Plan): Gesamtdurchsatz vom ersten bis zum letzten Byte, jederzeit nur durch das knappste
Mittel begrenzt; Zielwert ≥ 90 % des schnellsten etablierten Werkzeugs auf demselben Weg.

**Annahmen für alle Schätzungen** (keine Messung):

| Kürzel | Leitung | Rundreise (RT) | Nutzrate |
|---|---|---|---|
| L1 | LAN | 0,5 ms | 1 Gbit/s ≈ 112 MiB/s |
| L2 | WAN | 20 ms | 100 Mbit/s ≈ 11,2 MiB/s |
| L3 | schnelles WAN | 20 ms | 1 Gbit/s ≈ 112 MiB/s |
| L4 | langes schnelles WAN | 100 ms | 1 Gbit/s ≈ 112 MiB/s |

- DW (ein dauerhafter privater Beleg-Schreibvorgang, `analyse-kopieren.md` 2.): 1,5 ms unter Linux
  auf NVMe, 5–20 ms unter Windows (Annahme).
- Kernel-Puffergrenzen des Clients (Annahme: Linux-Standard): `tcp_wmem` höchstens 4 MiB,
  `tcp_rmem` höchstens 6 MiB je TCP-Verbindung. Daraus: ein TCP-Strom sendet höchstens
  ≈ 4 MiB je RT (L4: ≈ 40 MiB/s) und empfängt höchstens ≈ 6 MiB je RT (L4: ≈ 60 MiB/s).
- OpenSSH-Server: Fenster je Sitzungskanal 2 MiB (`CHAN_SES_WINDOW_DEFAULT`, auch im Code genannt:
  `sftp/core/channel_pool.rs:2`), `MaxSessions` 10, `sftp-server` ein Prozess je Kanal.
- FTP-Testserver vsftpd ohne `MLST` (wie im Suite-Container, `native/test-transfer-engine-task.sh:431-437`).
- Nextcloud-WebDAV: ≈ 50 ms Serverzeit je Anfrage (Annahme).
- Google Drive: ≈ 150 ms je API-Aufruf (Annahme), ≈ 3 neue Dateien je Sekunde je Konto
  (`gdrive/core/backend.rs:262`).
- „GUI“ = Auftrag mit Belegen (`recoverable: true`: Einfügen, Ziehen, Android); „CLI“ = `se cp`
  (`recoverable: false`, `cli/engine_copy.rs:306-309`, laut `analyse-kopieren.md` 1.).

**Referenzwerkzeuge je Weg** (schnellste etablierte Werkzeuge, optimale Einstellungen):

| Weg | Referenz | Verhalten (Annahme, nicht gemessen) |
|---|---|---|
| SFTP/SSH viele kleine Dateien | `rsync -a` über SSH, `tar c \| ssh tar x` | ein Strom, Dateiliste vorab, Daten ohne Rundreise je Datei; datenrate- oder servergebunden |
| SFTP große Datei | `rsync`, `sftp -B 262144 -R 64`, `rclone --multi-thread-streams` | ein TCP-Strom (rsync/sftp) bzw. mehrere Ströme (rclone) |
| SMB | `robocopy /MT:32` (Windows), `smbclient` `mput` (seriell), `rclone` smb | robocopy: viele Dateien parallel, Server-Offload |
| FTP/FTPS | `lftp mirror -R --parallel=8`, `lftp pget -n 8` | je Datei PASV/STOR/226, keine Stufe; pget: 8 Anmeldungen für eine Datei |
| WebDAV | `rclone --transfers 8` | ein `PUT` je Datei auf den Endnamen (+ ggf. `PROPFIND`) |
| Google Drive | `rclone --transfers 4 --drive-chunk-size 64M` | dieselbe Grenze von ≈ 3 neuen Dateien/s |
| gleiche Gegenstelle | `ssh host cp -a`, SMB-Offload, WebDAV `COPY`, Drive `files.copy` | serverseitig |

---

## 2. Was jedes Netz-Backend in der Engine durchläuft

### 2.1 Weichen

- Eine Datei geht nach Endpunktpaar an `upload`, `download` oder `remote_copy::copy`
  (`engine/ops.rs:262-291`). Pakete gibt es **nur lokal↔entfernt** und nur, wenn das Backend
  `batch_limits` meldet (`engine/mod.rs:242-247`): SFTP (`sftp/core/backend.rs:403-406`), SMB
  (`smb/core/backend.rs:377`), SSH-Agent (`agent/core/backend.rs:218`). FTP, WebDAV, Drive: nie.
- Große Dateien ab 16 MiB (`transfer/core/segments.rs:11-13`) laufen in Bereichen, wenn das Ziel eine
  segmentierte Stufe hat (Upload: SFTP, SMB, Agent; `open_segmented_stage`-Überschreibungen) bzw.
  die Quelle Bereiche lesen kann (Download/Kopie: alle sechs Backends, FTP nur mit 2 freien
  Verbindungen, `ftp/core/ftp.rs:278`). Kopien mit MD5 der Quelle (Drive) bleiben ein Strom
  (`engine/segments_copy.rs:133`).
- Entfernt → entfernt: gleicher Namensraum → zuerst `server_copy_to_stage`
  (`engine/remote_copy.rs:38`, `:65-125`); sonst Strom mit einem eigenen Lesefaden je Datei und
  3 × 256 KiB Puffer zwischen Lese- und Schreibseite (`engine/remote_copy.rs:127-181`,
  `ops.rs:21`); gleiche Verbindung ohne gleichzeitiges Lesen und Schreiben (FTP mit nur einer
  Übertragungsverbindung) → erst ganz in eine lokale Temp-Datei, dann hochladen, ohne Überlappung
  (`engine/remote_copy.rs:49`, `:236-307`).

### 2.2 Rundreisen der Belege bei entferntem Ziel oder entfernter Quelle

| Stelle | Aufruf | Rundreisen je Datei | Beleg |
|---|---|---|---|
| Upload/Kopie `before()` | `validate_copy_target(stage)` → `stat(stage)` | 1 | `file_recovery.rs:83-89`, `file_evidence.rs:223-252` |
| | `stage_record` → `file_identity(stage)` | 0 (SFTP, SMB, FTP, WebDAV: kein Override), 1 (Agent mit Merkmal, `agent/core/folder_reservation.rs:66-73`), ≥ 1 (Drive `sync_meta`, `gdrive/core/folder_reservation.rs:33`) | `file_evidence.rs:162-187` |
| | `previous_presence(Ziel)` → `stat(Ziel)` | 1 (FTP: `SIZE` 550 + Ordner-`LIST`, siehe 6.) | `file_recovery.rs:96`, `file_recovery_verify.rs:15-29` |
| | Quelle `snapshot` (nur entfernte Quelle) | 1 `stat` (+ Identität bei Agent/Drive) | `file_recovery.rs:73`, `job_store_validation.rs:30-59` |
| Veröffentlichung (Standard) | `validate_staged_file` → `stat(stage)` + `rename_no_replace` | 2 | `vfs/core/promotion.rs:83-103`, `:147-156` |
| `after()` | `validate_copy_target(Ziel)` → `stat(Ziel)` (+ Identität) | 1 (+0/1) | `file_recovery.rs:147-166` |
| Paket Upload, je Mitglied vorher | `prepare_batch` → `previous_presence` | 1, **seriell** | `engine/batch_put.rs:76-84`, `file_recovery.rs:184-206` |
| Paket Upload, je Mitglied nachher | `after()` | 1, **seriell** | `engine/batch_put.rs:130-147` |
| Download, nach dem Lesen | `RemoteCheck::finish` → `stat(Quelle)`, wenn kein MD5 bekannt | 1 (auch CLI) | `engine/download.rs:192`, `source.rs:343-365` |
| Download, `before()` | `snapshot(Quelle)` | 1 (+ Identität) | `download.rs:338ff` |

Ohne Belege (CLI) bleiben davon nur `validate_staged_file` (1 RT) und beim Download
`RemoteCheck::finish` (1 RT).

`PreviousTarget::Absent` wird bei der Wiederaufnahme nur so genutzt: ist das Ziel noch abwesend,
gilt nichts als veröffentlicht (`file_recovery_state.rs:135-147`); `Unconfirmed` ist der
vorsichtige Wert (`file_recovery_verify.rs:18-27`). Für Ziele mit atomarem Nicht-Ersetzen
(SFTP v3 `RENAME`, SMB `ReplaceIfExists=0`, WebDAV `Overwrite: F`, Agent) ist das Ergebnis
„Ziel jetzt abwesend ⇒ nicht veröffentlicht“ auch ohne die Vorab-Abfrage richtig, weil die
Veröffentlichung nur auf einen freien Namen gelingt (Grundlage für G3-02).

### 2.3 Flow, Start, Obergrenzen je Backend

| Backend | Start | Obergrenze `transfer_ceiling` | Beleg |
|---|---|---|---|
| alle | 2 oder halbe gelernte Grenze | höchstens 256 (`RESOURCE_CEILING`) | `transfer/core/flow_control.rs:32-33`, `flow_window.rs:19-21` |
| SFTP | 2 | Kanäle × Handles je Kanal (≤ 64), erst nach der ersten Ablehnung eines Kanals bekannt; vorher 256 | `sftp/core/backend.rs:428-431`, `channel_pool.rs:288-301` |
| SSH-Agent | 2 | Plätze je Kanal (angekündigt − 2 Reserve) × Kanäle, sobald die Kanalgrenze bekannt ist | `agent/core/pool.rs` `transfer_ceiling`, `agent_proto/core/features.rs:33`, `:159-166` |
| SMB | 2 | keine (256); Credits begrenzen im Protokoll | kein Override |
| FTP | 2 | Poolgrenze − 1, erst nach einem 421/530; vorher 256 | `ftp/core/ftp.rs:422-427`, `pool.rs:89-91` |
| WebDAV | 2 | keine (256) | kein Override |
| Drive | 2 | keine (256); Überlast halbiert | kein Override |

- Ein Messfenster endet nach ≥ 100 ms **und** ≥ 8 Abschlüssen, sonst nach 1 s
  (`flow_control.rs:35-40`); ein Paket zählt als **eine** Operation. Ein GUI-Paket von 64 Dateien
  dauert bei L2 ≈ 3 s (2.4), also liefert ein Fenster mit 2 Workern meist 0–1 Abschlüsse: der
  Hochlauf über 2 hinaus ist langsam (Folgerung, ungemessen).
- Der Walker startet Lister-Fäden bis zum Flow-Limit der Quelle (`transfer/os/shared/walk.rs:168`);
  jede Auflistung nimmt eine Erlaubnis dieses Flows oder den einen Metadaten-Platz
  (`walk.rs:278`, `flow.rs:200-216`). `Backend::parallelism()` (SFTP 1, SMB 2, FTP 1, WebDAV 2,
  Drive 16) benutzt der Engine-Walker nicht (andere Verwender nicht geprüft).
- Ordner am entfernten Ziel: einer je Ordner unter einer Metadaten-Erlaubnis
  (`engine/folders.rs:261`, `:278`, `:384-399`), Eltern zuerst; SFTP `MKDIR` (1 RT,
  `sftp/core/transfer_ops.rs:54`), SMB 1 RT, WebDAV `MKCOL` 1 RT, FTP `MKD` auf der
  Hauptverbindung (`ftp/core/ftp.rs:176-200`), Drive 1 API-Aufruf.

### 2.4 Paketgröße und Kleindateigrenze

`target = clamp(Rate × 250 ms, 256 KiB, max_bytes)`, `klein ≤ target ÷ 8`
(`engine_policy.rs:40-45`, `:206-217`); die Rate stammt aus fertigen Paketen
(`engine/batch_put.rs:110`). Ein SFTP-Paket aus 64 × 4 KiB dauert im Protokoll ≈ 4 RT; bei L2
also 256 KiB / 80 ms ≈ 3,2 MB/s → `target` ≈ 800 KiB → Grenze ≈ 100 KiB. 1-MiB-Dateien kommen erst
bei gemessenen ≥ 32 MB/s in Pakete, die kleine Dateien wegen der Rundreisen nie liefern.

### 2.5 Prozesse und Fäden

| Teil | Fäden/Aufgaben | Beleg |
|---|---|---|
| Engine je Auftrag | 1 Lane-/Dispatcher-Faden, 1 Discovery-Faden + Lister-Fäden ≤ Flow-Limit, Worker-Fäden (OS-Fäden, je 256 KiB Puffer, niedrigere Priorität) ≤ Flow-Limit; entfernt→entfernt +1 Lesefaden je laufender Datei | `engine/dispatch.rs:45-80`, `worker.rs:23-26`, `remote_copy.rs:144-152` |
| SFTP je Backend | 1 tokio-Laufzeit (Mehrfaden, Kerne ≥ 2), je SSH-Verbindung eine russh-Sitzungsaufgabe (alle Kanäle einer TCP-Verbindung: Ver- und Entschlüsselung in **einer** Aufgabe), je pipelined `READ`/`WRITE` ein `rt.spawn`; Engine-Fäden blockieren in `rt.block_on` | `sftp/core/connection.rs:61-66`, `:180-185`, `pool_writer.rs:96-99`, `pool_reader.rs:29-34` |
| SSH-Agent | Client: Mux über SSH-Exec-Kanäle der SFTP-Laufzeit; Server (se-agent): ein OS-Faden **je Anfrage** | `sftp/core/exec.rs:58-75`, `agent_proto/core/server.rs:168-170` |
| SMB je Backend | 1 tokio-Laufzeit „smb“ (Kerne ≥ 2), **eine** TCP-Verbindung je Sitzung | `smb/core/session.rs:88-95`, `:226-243` |
| FTP | blockierendes suppaftp; je Poolverbindung 1 Steuer-TCP + 1 Keepalive-Faden; je Datei 1 Daten-TCP (+ TLS bei FTPS) | `ftp/core/io_adapters.rs:47-77`, `connection.rs:332-345` |
| WebDAV | blockierendes ureq (HTTP/1.1); vier Agenten (Lesen, Metadaten, Mutation ohne Pool, Schreiben); je Upload 1 zusätzlicher Faden für den Body | `webdav/core/connection.rs:21-48`, `stream_put.rs:184` |
| Drive | blockierendes ureq; 2 Pools (`api`, `stream`) mit je 256 Sockets, neu gebaut nach 60 s Leerlauf | `gdrive/core/http.rs:11-79` |

### 2.6 Vorgefundener Defekt (Befund 1, Einzelheiten)

- SFTP: `run` → `backend.rt.block_on(get_all(&wire, items, sink))` (`sftp/core/batch_get.rs:46`);
  `get_all` ruft `deliver` → `sink.end` (`:160-181`) synchron in der Zukunft auf.
- SMB: `session.block_on(batch::get_all(&wire, items, sink))` (`smb/core/batch_wire.rs:448`),
  `sink.begin/data/end` in `smb/core/batch.rs` (Funktion `get_all`).
- Engine-Senke: `Sink::end` → `check.finish_streamed` → `publish_local`
  (`engine/batch_get.rs:329-350`) → `prepare_part` → `file_recovery::before` → `snapshot(Quelle)`
  → `backend.stat()` → zweites `block_on` auf derselben Laufzeit.
- Betroffen: jeder Download kleiner Dateien von SFTP oder SMB in GUI, Android und „Für andere
  Programme bereitstellen“ (alle `recoverable: true`, `analyse-kopieren.md` 1.). Nicht betroffen:
  `se cp`, MCP über den CLI-Weg (Annahme, nicht geprüft), Agent (eigene Fäden, kein tokio-`block_on`
  im Paketweg) und Share.
- Zweite Folge derselben Bauweise (auch ohne Belege): Während die Senke lokal schreibt und
  veröffentlicht, pollt niemand die laufenden Netz-Futures; Antworten stauen sich, das Netz ruht.

---

## 3. SFTP

**Bibliotheken:** russh 0.64.1 (Features `ring`, `flate2`), russh-sftp 2.3.0, tokio 1.52.3
(`native/Cargo.toml:64-69`, `Cargo.lock`). Algorithmen: AES-256-GCM zuerst bei AES-Hardware, sonst
ChaCha20-Poly1305 (`sftp/core/session.rs:344-368`).

### 3.1 Verbindungen, Kanäle, Fenster, Pipelining

- **Eine SSH-Verbindung** mit einem Haupt-SFTP-Kanal für Auflisten und alle `stat`
  (`sftp/core/backend.rs:201-225`, `connection.rs:83-107`, Frist 20 s je Metadatenaufruf, `:18`).
- **Kanal-Pool** für Übertragungen: jede gleichzeitige Übertragung bekommt einen leeren Kanal oder
  öffnet einen neuen (≈ 4 RT: `CHANNEL_OPEN`, Subsystem, `INIT`/`VERSION`, `limits@openssh.com`),
  bis der Server einen Kanal ablehnt; danach Grenze = gehaltene − 2 (`link_rules.rs:272-284`,
  `:333-359`; `channel_pool.rs:317-391`). Bei OpenSSH (`MaxSessions` 10) also ≈ 7 Pool-Kanäle.
  Danach teilen sich Übertragungen Kanäle bis `min(open-handles, 64)` Leases je Kanal
  (`pool_channel.rs:47-55`). Leerlauf-Kanäle schließen nach 60 s, der zuletzt benutzte bleibt
  (`channel_pool.rs:46`, `:96-117`).
- **Zusatzverbindungen** (je 1 TCP + 1 SFTP-Kanal) nur bei gespeichertem Passwort/Schlüssel ohne
  Passphrase, nie mit SSH-Agent-Anmeldung; einzeln, erst nach 2 s Einrichtung und 1-s-Fenstern,
  nur bei ≥ 10 % Zuwachs (`docs/TRANSFER_ENGINE.md` Abschnitt „SFTP-Kanäle und
  -Zusatzverbindungen“, `sftp/core/link_rules.rs:18-23`, `:107-131`).
- **Fenster:** Empfang 16 MiB je Kanal, Paket 32 KiB, Kanalpuffer 512 Nachrichten
  (`session.rs:33-39`, `:333-341`). Senden begrenzt das Serverfenster (OpenSSH 2 MiB je Kanal).
- **Chunks:** `READ` bis `min(limits.read_len, 256 KiB − 9)`, `WRITE` bis
  `min(limits.write_len, 256 KiB − 21 − Handlelänge)` (`pool_channel.rs:32-38`, `:81-131`).
  OpenSSH meldet 255 KiB (Annahme).
- **Pipelining je Datei:** `Pipeline` beginnt bei 2 Anfragen und wächst um 1 je voll beantwortetem
  Chunk, solange die Antwortzeit ≤ 2 × schnellste + 1 ms ist; Obergrenze 16 MiB ÷ Chunk ≈ 64
  (`read_pipeline.rs:26-99`; Schreiben `pool_writer.rs:347-362`; Lesen `pool_reader.rs:116-120`,
  `:201`). Jede Datei beginnt neu bei 2.
- **Commit:** Kopiestufe ohne `fsync@openssh.com` (`backend.rs:302-308`, `pool_writer.rs:152-158`);
  `flush` wartet alle `WRITE`-Antworten ab, dann `CLOSE` (`pool_writer.rs:147-166`).
- **Kein Pool-Kanal** (Server lehnt schon den ersten ab): alles über den Hauptkanal; der
  Hauptkanal-Schreiber ist russh-sftp `File` (Pipelining-Tiefe der Bibliothek, Annahme: gering).
- **Pakete** (Upload und Download): bis `min(open-handles, 64)` Dateien und 16 MiB, alle Mitglieder
  gleichzeitig auf einem Pool-Kanal (`batch_ops.rs:52-55`, `:99-110`; `batch_put.rs:53-92`).
  Upload-Mitglied: `OPEN EXCL` → alle `WRITE` → `CLOSE` → `RENAME` (4 RT,
  `batch_put.rs:141-243`; `CLOSE` erst nach allen `WRITE`-Antworten, `:209`, `:151`).
  Download-Mitglied: `OPEN` → alle `READ` + 1-Byte-Probe → `CLOSE` (3 RT, `batch_get.rs`
  `member`/`read_all`).
- **Serverkopie** `copy-data` (OpenSSH ≥ 9.0, Annahme): `OPEN` Quelle, dann `OPEN` Stufe
  (nacheinander), Bereiche ab 16 MiB, danach ≈ 10 s je Bereich, `FSTAT`, `CLOSE` Quelle, `CLOSE`
  Stufe (nacheinander) (`copy_data.rs:39-43`, `:149-200`, `:293-333`).
- **Große Uploads in Bereichen** nur, wenn alle Ordner bis `/` dem Konto oder root gehören und für
  Gruppe/andere gesperrt sind (S7, `segmented_stage.rs:1-60`); jeder Bereich: `OPEN` ohne `CREATE`
  + `FSTAT` + `WRITE`s + `CLOSE` auf eigenem Kanal (`pool_writer.rs:300-346`).

### 3.2 Serielle Rundreisen je Datei

| Weg | CLI | GUI | Bestandteile (GUI) |
|---|---|---|---|
| Upload einzeln, ≤ 1 Chunk | 5 | 8 (+4 DW) | `OPEN`, `WRITE`-Antwort, `CLOSE`, `LSTAT` Stufe, `LSTAT` Ziel, `LSTAT` Stufe (Promotion), `RENAME`, `LSTAT` Ziel |
| Upload einzeln, 1 MiB | ≈ 7 | ≈ 10 | wie oben, Datenphase ≈ 3 RT wegen Tiefe 2 → 3 → 4 |
| Upload Paket (N ≤ 64) | ≈ 4 je Paket | 2N + 4 je Paket (+4N DW) | `LSTAT` Ziel je Mitglied vorher und nachher, seriell |
| Download einzeln, ≤ 1 Chunk | 3 | 4 (+4 DW) | `OPEN`, `READ`+EOF, `LSTAT` Quelle (`finish`), `LSTAT` Quelle (`snapshot`); `CLOSE` ohne Warten |
| Download Paket | ≈ 3 je Paket | **Abbruch** (2.6) | – |
| gleiche Gegenstelle (`copy-data`) | ≈ 9 | ≈ 13 (+4 DW) | 6 Serverkopie, `LSTAT` Quelle, `LSTAT`×3 Belege, `LSTAT`+`RENAME`, `LSTAT` |

### 3.3 Zeitlinien (Schätzung)

- **5000 × 4 KiB hoch, L2:** Walker lokal (schnell); erste Datei nach Ordner-`MKDIR` (1 RT je
  Ebene) und Kanalaufbau (≈ 4 RT) ≈ 0,1–0,2 s. CLI: 79 Pakete × (≈ 80 ms + 22 ms Daten) bei
  Flow 2 → 8 → 16 in ≈ 2 s; Gesamt ≈ 2–3 s (Daten allein 1,7 s). GUI: je Paket
  64 × (20 ms + 4,5 ms) + 80 ms + 64 × (20 ms + 1,5 ms) ≈ 3,0 s; die 3 DW je Datei laufen global
  unter einer Sperre (5000 × 4,5 ms = 22,5 s Untergrenze); Gesamt ≈ 25–45 s. Netz-Leerlauf GUI
  ≈ 90–95 % der Zeit.
- **5000 × 4 KiB hoch, L1:** CLI ≈ 0,5–1 s (servergebunden); GUI ≈ 25 s (Sperre). Windows-GUI:
  3 DW × 5–20 ms × 5000 = 75–300 s.
- **1000 × 1 MiB hoch, L3:** Einzelweg; je Datei ≈ 7 (CLI) / 10 (GUI) RT ≈ 140–200 ms + 9 ms
  Daten → ≈ 20 parallele Dateien nötig; Flow-Hochlauf 2 → 32 ≈ 4–5 s. CLI ≈ 12–14 s, GUI
  ≈ 13–16 s (Linux), ≈ 30 s (Windows-Sperre 1000 × 3 × 10 ms). Referenz `rsync` ≈ 9–10 s.
- **1000 × 1 MiB hoch, L2:** linkgebunden ≈ 90 s; 3–4 Dateien parallel genügen; se ≈ 92–95 s.
- **10 GiB hoch, L1/L3:** Bereiche ≥ 8 MiB über Pool-Kanäle, je Kanal 2 MiB Serverfenster
  (L3: 100 MiB/s je Kanal), eine TCP-Verbindung mit ≤ 4 MiB Sendepuffer (L3: ≤ 200 MiB/s) →
  linkgebunden ≈ 92 s. **L4:** eine TCP-Verbindung ≤ 40 MiB/s; Zusatzverbindungen nur ohne
  SSH-Agent/Passphrase, Hochlauf ≈ 3 s je Verbindung → ≈ 100–110 s; mit SSH-Agent-Anmeldung
  ≈ 260 s (≈ 35 % der Leitung; `rsync` hat dieselbe TCP-Grenze, `rclone --multi-thread-streams`
  nicht).
- **10 GiB herunter:** Bereiche je Kanal mit 16 MiB Empfangsfenster; Grenze `tcp_rmem` 6 MiB je TCP
  → L4 ≈ 60 MiB/s je Verbindung.

### 3.4 Leerlauffenster SFTP

1. Je Datei vor dem ersten Byte: `OPEN` (1 RT), bei Kanalwachstum + ≈ 4 RT.
2. Je Datei nach dem letzten Byte: `CLOSE` + Belege + `RENAME` (GUI 6 RT, CLI 3 RT); der Worker
   hält dabei seine Erlaubnis (Einzelweg) bzw. bei Paketen in der `after()`-Schleife keine.
3. Erste Chunks jeder Datei: Tiefe 2 statt der gelernten Tiefe.
4. Pakete: Mitgliedsdateien werden im `block_on`-Faden nacheinander von der Platte gelesen
   (`sftp/core/batch_put.rs:61-80`); laufende Antworten werden in der Zeit nicht verarbeitet
   (bei SSD unter 1 ms je Datei, Annahme).
5. Lange Leitung: eine TCP-Verbindung je SSH-Verbindung; Zusatzverbindungen langsam oder gar nicht.

---

## 4. SSH-Agent (se-agent über SSH-Exec-Kanäle)

- **Transport:** jeder Agent-Kanal ist ein SSH-Exec-Kanal der SFTP-Hauptverbindung
  (`agent/core/deploy.rs:97`, `sftp/core/exec.rs:58-75`), also dieselbe TCP-Verbindung und dasselbe
  2-MiB-Serverfenster je Kanal (Upload). Zusatzverbindungen gibt es für den Agent nicht
  (E13-S15: der Agent bleibt auf der Hauptverbindung, `docs/TRANSFER_ENGINE.md`).
- **Pool:** ein Kanal; ist der am wenigsten belastete Kanal belegt, öffnet ein Hintergrundfaden einen
  weiteren, bis einer scheitert (`agent/core/pool.rs:105-150`).
- **Fluss:** Kredit je Anfrage 1 MiB, Fenster bis 8 MiB je Strom, 64 MiB je Verbindung
  (`agent_proto/core/credit.rs:23-33`); Chunk 256 KiB (`agent_proto/core/types.rs:25`); LZ4 nach
  Regeln (`docs/TRANSFER_ENGINE.md` „Kompression“).
- **Pakete:** 256 Dateien / 16 MiB (`agent_proto/core/batch_limits.rs:10-12`), ein Strom je Paket,
  Server veröffentlicht ohne fsync und ohne Ersetzen (`agent_proto/os/shared/batch_put.rs:1-6`,
  `:106`).
- **Einzeldatei:** `WriteNew` → `Progress` (1 RT) → Daten → Antwort (1 RT)
  (`agent/core/stream.rs:306-340`); `stat` und `FileIdentity` je 1 RT
  (`agent/core/folder_reservation.rs:66-73`).
- **Server:** ein OS-Faden je Anfrage (`agent_proto/core/server.rs:168-170`).
- **Ungenutzt:** `put_tree`/`get_tree` (ganzer Baum als ein Strom) existieren, die Engine ruft sie
  nicht; nur der alte Download-Weg nutzt `get_tree` (`transfer/os/shared/downloads.rs:108`).
  `put_tree` sammelt den Baum erst vollständig (`agent/core/transfer.rs:71-72`).

Rundreisen je Datei (GUI, mit Merkmal `file_identity`): Einzel-Upload ≈ 2 (Daten) + 1 `stat`
Stufe + 1 Identität Stufe + 1 `stat` Ziel + 1 `PromoteNoReplace`/`stat` + 1 `stat` Ziel +
1 Identität Ziel ≈ 8; Paket: 2 RT je Mitglied seriell + ≈ 1 RT je Paket. CLI: Einzel ≈ 3–4, Paket
≈ 1. Zeitlinien wie SFTP, aber ohne Zusatzverbindungen (L4: ≤ 40 MiB/s je TCP).

---

## 5. SMB

**Bibliothek:** smb2 0.26.0 (reines Rust, exakt gepinnt, `native/Cargo.toml:82`), tokio-Laufzeit.

- **Verbindung:** eine TCP-Verbindung und eine Sitzung je Backend, kein Multichannel
  (`smb/core/session.rs:226-243`). Alle Anfragen teilen sich die vom Server gewährten Credits.
- **Lesen:** Chunk `min(smb2::DOWNLOAD_CHUNK_SIZE, MaxReadSize)`, Tiefe per `Pipeline` (Start 2) bis
  zur Credit-Kapazität (`smb/core/reader.rs:26`, `:76-95`); `CLOSE` im `Drop` **blockierend**
  (`reader.rs` `Drop`), also 1 RT seriell nach jeder Datei (`engine/download.rs:190`).
- **Schreiben:** 1 MiB sammeln, dann `FileWriter::write_chunk` (Write-behind-Fenster der Bibliothek,
  `docs/refs/smb2.md` Abschnitt `FileWriter`); Kopierstufe ohne FLUSH: `abort()` wartet auf alle
  Antworten und schließt (`smb/core/io.rs:24`, `:131`, `:184-195`).
- **Pakete:** 256 Dateien / 16 MiB (`smb/core/batch.rs:31-32`); je Mitglied ein Compound
  `CREATE+WRITE+CLOSE` (1 RT bis `MaxWriteSize`) und ein Compound-`RENAME` (1 RT), alle Mitglieder
  gleichzeitig (`batch.rs:170-236`).
- **Metadaten:** `stat` = Compound `CREATE+QUERY_INFO×2+CLOSE` (1 RT, `docs/refs/smb2.md`);
  Umbenennen ohne Ersetzen = Compound mit `ReplaceIfExists=0` (1 RT).
- **Serverkopie:** `FSCTL_SRV_COPYCHUNK` nur innerhalb einer Freigabe: Quelle öffnen, Stufe anlegen,
  Kopieren (16 MiB je Anfrage), Länge der Quelle, beide schließen, alles nacheinander (≥ 6 RT,
  `smb/core/server_copy.rs:167-210`, `smb/core/backend.rs:111-143`).

| Weg | CLI | GUI |
|---|---|---|
| Upload einzeln ≤ `MaxWriteSize` | 5 (`CREATE`, `WRITE`-Antwort, `CLOSE`, `stat`, `RENAME`) | 8 (+4 DW) |
| Upload Paket (N) | ≈ 2 je Paket | 2N + 2 (+4N DW) |
| Download einzeln | 4 (`CREATE`, `READ`, `CLOSE` im Drop, `stat`) | 5 (+4 DW) |
| Download Paket | ≈ 1–2 je Paket | **Abbruch** (2.6) |

Zeitlinien (Schätzung): 5000 × 4 KiB hoch L1: CLI ≈ 1–3 s (servergebunden), GUI ≈ 25 s (Sperre);
Referenz `robocopy /MT:32` ≈ 1–3 s, `smbclient mput` ≈ 10–15 s. 1000 × 1 MiB L1: CLI/GUI ≈ 10–11 s
(linkgebunden), Windows-GUI ≈ 30 s. 10 GiB L1: Bereiche über eigene Handles auf der einen
TCP-Verbindung, ≈ 92 s. L4: eine TCP-Verbindung → ≤ 40 MiB/s hoch, ≤ 60 MiB/s herunter (Windows-
Clients nutzen dort Multichannel, Annahme).

---

## 6. FTP/FTPS

**Bibliotheken:** suppaftp 10.0.2 (blockierend, `rustls-ring`), rustls 0.23.46
(`native/Cargo.toml:78`, `:91`).

- **Verbindungen:** eine Hauptverbindung (`primary`) für Auflisten und **jedes `stat`**
  (`ftp/core/ftp.rs:226`, `:237`, `:247-248`); ein Pool weiterer Anmeldungen für Übertragungen, je
  gleichzeitiger Übertragung eine neue Anmeldung, bis 421/530 (`ftp/core/pool.rs:159-195`). Eine
  Anmeldung: TCP, Begrüßung, `AUTH TLS` + TLS (FTPS), `USER`, `PASS`, `TYPE I` (≈ 6–9 RT,
  `connection.rs:351-404`). Leerlauf 60 s (`pool.rs:28`). Ohne Ablehnung ist die Obergrenze 256
  (2.3): der Flow kann bis zu 256 Anmeldungen auslösen (Risiko bei Servern mit Sperrlisten).
- **Daten:** eine neue Daten-TCP-Verbindung je Datei (passiv), bei FTPS mit TLS-Handshake
  (Sitzungswiederaufnahme der rustls-Konfiguration, Annahme); `STOR`/`RETR` gehen vor dem
  TCP-Aufbau hinaus (`streams.rs:101-139`). Kein Befehls-Pipelining.
- **Stufe:** zufälliger Name ohne Probe (`staging.rs:33`), streamendes `STOR` mit bekannter Länge
  (`ftp.rs:341-355`, `streams.rs:300-427`).
- **`stat`:** `MLST` (falls angeboten) → sonst `SIZE` + `MDTM` (2 RT) → bei 550 oder fehlender
  Unterstützung **`LIST` des Elternordners** (`metadata.rs:309-361`, `metadata_probe.rs`). Für
  ein noch nicht vorhandenes Ziel endet jedes `stat` im Ordner-`LIST`.
- **Veröffentlichen ohne Ersetzen (nicht atomar, dokumentierte Ausnahme):** Stufe und Ziel prüfen
  (gleicher Ordner ohne `MLST`: ein Ordner-`LIST`; mit `MLST`: `MLST` Stufe + `stat` Ziel, also
  wieder `LIST`), dann `RNFR`/`RNTO` (`staging.rs:134-190`, auf der Pool-Verbindung,
  `ftp.rs:166-171`).
- **Bereiche (Download):** nur mit ≥ 2 freien, schon angemeldeten Pool-Verbindungen
  (`pool.rs:35`, `:56-67`, `:140-147`), nie mit neuer Anmeldung (S12). **Kein** segmentierter Upload.
- **Entfernt → gleicher Server:** keine Serverkopie; zwei Pool-Verbindungen streamen, bei
  Poolgrenze 1 über eine lokale Temp-Datei ohne Überlappung (`ftp.rs:441-445`,
  `engine/remote_copy.rs:236-307`).

| Weg (vsftpd, gleicher Ordner) | CLI | GUI |
|---|---|---|
| Upload einzeln | ≈ 9 RT (+TLS) + 1 Ordner-`LIST` | ≈ 18 RT (+TLS) + 2 Ordner-`LIST`, davon ≈ 8 RT + 1 `LIST` auf der **einen** Hauptverbindung |
| Download einzeln | ≈ 5 (2 auf Hauptverbindung) | ≈ 7 (4 auf Hauptverbindung) |

Zeitlinien (Schätzung, 5000 Dateien in **einem** Ordner, ≈ 70 B je `LIST`-Zeile):
- **5000 × 4 KiB hoch, L2:** CLI: Listing-Bytes ≈ 5000 × 175 KB ≈ 0,9 GB → ≈ 75 s Leitung, plus je
  Datei ≈ 9 RT über ≈ 16 Verbindungen → ≈ 80–90 s. GUI: Hauptverbindung je Datei ≈ 175 ms seriell
  → ≈ 15 min; Listing-Bytes ≈ 1,75 GB. Referenz `lftp mirror -R -P 8` ≈ 50–65 s (ohne Stufe).
- **5000 × 4 KiB hoch, L1:** CLI ≈ 8–10 s (Listing-Bytes und Serverarbeit), GUI ≈ 50–60 s;
  `lftp` ≈ 1–2 s.
- **1000 × 1 MiB hoch, L2:** CLI ≈ 95 s (linkgebunden); GUI ≈ 165–170 s (Hauptverbindung).
- **10 GiB hoch:** ein `STOR`-Strom, eine TCP-Verbindung (L4 ≤ 40 MiB/s). **10 GiB herunter:**
  im frischen Auftrag ein Strom (L4 ≤ 60 MiB/s ≈ 180 s); `lftp pget -n 8` ≈ 95 s.
- Verteilt auf viele Ordner mit wenigen Dateien schrumpft der `LIST`-Anteil (O(n²) je Ordner).

---

## 7. WebDAV

**Bibliothek:** ureq 2.12.1 (HTTP/1.1, rustls/ring, `native/Cargo.toml:59`), roxmltree für PROPFIND.

- **Agenten:** Lesen (`GET`, Pool 100 je Host), Metadaten (`PROPFIND`, ohne Weiterleitung, Pool),
  Mutation (**ohne Pool**: `DELETE`, leeres `PUT`, reservierte Ordner-`MKCOL`), Schreiben (`PUT`,
  `MOVE`, `COPY`, `MKCOL`, Pool) (`webdav/core/connection.rs:21-48`, `webdav.rs:59`,
  `transfer_ops.rs:150-158`, `folder_reservation.rs:11`).
- **Upload:** ein `PUT` mit `Content-Length` und `If-None-Match: *`, Body aus einem 256-KiB-Pipe in
  einem eigenen Faden (`stream_put.rs:1-19`, `:184`); keine segmentierte Stufe, keine Pakete.
- **Veröffentlichen:** `MOVE` mit `Overwrite: F` (`stage_move.rs:47-80`).
- **Serverkopie:** `COPY` mit `Overwrite: F`, danach `PROPFIND` der Stufe (`transfer_ops.rs:65-84`).
- **Download:** `GET`, Bereiche über `Range` (`transfer_ops.rs:88-148`).

| Weg | CLI | GUI |
|---|---|---|
| Upload einzeln | 3 (`PUT`, `PROPFIND` Stufe, `MOVE`) | 6 |
| Download einzeln | 2 (`GET`, `PROPFIND` falls kein MD5) | 3 |
| gleiche Gegenstelle (`COPY`) | ≈ 5 | ≈ 9 |

Zeitlinien (Schätzung, Nextcloud 50 ms Serverzeit, 8 parallele Dateien, L2): 5000 × 4 KiB:
CLI ≈ 5000 × 3 × 70 ms / 8 ≈ 130 s, GUI ≈ 260 s; `rclone` (1 `PUT` je Datei) ≈ 45 s. 10 GiB hoch:
ein `PUT`-Strom auf einer TCP-Verbindung; herunter: Bereiche parallel.

---

## 8. Google Drive

**Bibliothek:** ureq 2.12.1 (HTTP/1.1), Drive REST v3.

- **Neue Datei ≤ 5 MB:** ganz in den Speicher (oder Temp-Datei, wenn das Speicherbudget fehlt),
  dann **ein** Multipart-Create (`gdrive/core/sized_writer.rs:19`, `:191-201`, `:331-353`).
  Netz ruht, solange die Datei gelesen wird, Platte ruht während des Sendens.
- **Neue Datei > 5 MB:** fortsetzbarer Upload, Abschnitt 8 MiB, Verdopplung bis 64 MiB solange ein
  Abschnitt < 1 s dauert (`resumable.rs:9`, `chunk_stream.rs:17-22`, Verdopplung nach `send` `:78-126`); jeder Abschnitt
  wartet seine Antwort ab, bevor der Puffer neu gefüllt wird (Stop-and-wait, keine Überlappung mit
  dem Lesen, `chunk_stream.rs:51-115`).
- **IDs:** `generateIds` in Stapeln 10 → 1000 (`id_pool.rs:12-14`, `:42-67`).
- **Weg in einen von diesem Auftrag angelegten Ordner („fresh“):** Create mit Endnamen, keine Stufe
  (`engine/publish.rs:54-60`, `gdrive/core/backend.rs:229-231`).
- **Weg in einen bestehenden Ordner (Stufe):** Create unter Stufennamen + Eindeutigkeitsprüfung
  (`sized_writer.rs:231`), Veröffentlichung: Sperren je Name, `files.list` Stufe und Ziel,
  `PATCH` (Umbenennen), `files.list` zur Kontrolle (`promotion.rs:120-260`). **2 schreibende
  Aufrufe je Datei** (Create, `PATCH`).
- **`stat`:** Pfad auflösen (Cache oder `files.list` je Segment) + `files.get`
  (`listing.rs:53-83`).
- **Grenze:** ≈ 3 neue Dateien/s je Konto (Hinweis `backend.rs:262`); 403-Ratenlimit/429/503 →
  Überlast → Flow halbiert, Wartezeit `Retry-After` oder 1 s ± 50 % (`overload.rs:29-39`,
  `docs/TRANSFER_ENGINE.md` „Überlast“). Keine vorausschauende Drosselung.
- **Download:** Bereiche parallel; bei bekanntem MD5 liest `PrefixHasher` die schon geschriebene
  Teildatei zum Hashen zurück (`engine/segments_download.rs:207-214`).

API-Aufrufe je Datei (Schätzung): „fresh“ CLI 1 (+ ID-Anteil), GUI ≈ 4; Stufe CLI ≈ 6, GUI ≈ 10.

Zeitlinien: 5000 × 4 KiB ≈ 28 min bei 3/s für jedes Werkzeug; Stufen-Weg ≈ 28–56 min (56, falls
Google `PATCH` gegen dieselbe Schreibgrenze rechnet, **Annahme**). 10 GiB hoch bei 1 Gbit:
je 64-MiB-Abschnitt ≈ 0,13 s Lesen + 0,57 s Senden + ≈ 0,15 s Antwort → ≈ 75 MiB/s ≈ 65 % der
Leitung; bei 100 Mbit ≈ 95 %.

---

## 9. Entfernt → entfernt

| Fall | Weg | Daten über den Client | Rundreisen je kleiner Datei (CLI / GUI) |
|---|---|---|---|
| SFTP gleicher Server | `copy-data` | nein | ≈ 9 / 13 |
| SFTP ohne `copy-data` | Strom (Lesefaden + Schreiber) | ja, beide Richtungen | ≈ 2 + 5 / 3 + 8 |
| SMB gleiche Freigabe | `COPYCHUNK` | nein | ≈ 9 / 13 |
| SMB andere Freigabe, gleicher Server | Strom | ja | ≈ 6 + 5 / 7 + 8 |
| FTP gleicher Server | Strom über zwei Pool-Verbindungen; bei Poolgrenze 1: Temp-Datei ohne Überlappung | ja | ≈ 5 + 9 / 9 + 18 (+ `LIST`) |
| WebDAV gleicher Server | `COPY` | nein | ≈ 5 / 9 |
| Drive gleiches Konto | `files.copy` (zählt als neue Datei, Annahme) | nein | ≈ 3–7 Aufrufe |
| verschiedene Backends | Strom | ja | Quelle (`OPEN`, `READ`, `stat`) + Ziel (Einzel-Upload) |

- Keine Pakete (`engine/mod.rs:242-247`); jede Datei hält einen Worker und einen Lesefaden.
- Großdateien: Bereiche (`segments_copy.rs:123-177`) außer bei MD5-Quellen (Drive).
- Zeitlinie SFTP gleicher Server, 5000 × 4 KiB, L2: CLI ≈ 5000 × 9 × 20 ms / Flow (≈ 32–64)
  ≈ 15–30 s, GUI ≈ 30–40 s; `ssh host cp -a` < 1 s.

---

## 10. Gesamttabelle (Schätzungen)

| Weg, Fall, Leitung | Referenz | se CLI | se GUI | Verhältnis GUI | Leerlauf knappstes Mittel (GUI) | Ursachen (Beleg) | Maßnahme | erwarteter Gewinn | berührte Invariante |
|---|---|---|---|---|---|---|---|---|---|
| SFTP hoch 5000 × 4 KiB, L2 | rsync ≈ 2–3 s | ≈ 2–3 s | ≈ 25–45 s | ≈ 0,07–0,1 | Netz ≈ 90–95 % | 2.2, 3.2; Sperre `analyse-kopieren.md` A | G3-02, G3-03, A | GUI ≈ CLI | Belege, Nicht-Ersetzen |
| SFTP herunter 5000 × 4 KiB | rsync ≈ 2–3 s | ≈ 2–3 s | Abbruch | 0 | – | 2.6 | G3-01 | Weg funktioniert, Netz läuft beim Veröffentlichen weiter | Reihenfolge der Senke |
| SFTP hoch 1000 × 1 MiB, L3 | rsync ≈ 9–10 s | ≈ 12–14 s | ≈ 13–16 s | ≈ 0,65 | Netz ≈ 30 % | 2.4, 3.1 Tiefe 2, Start 2 | G3-05, G3-06, G3-09 | ≈ 9–10 s | keine |
| SFTP hoch 10 GiB, L4, SSH-Agent-Anmeldung | rclone 8 Ströme ≈ 95 s | ≈ 260 s | ≈ 260 s | ≈ 0,35 | Netz ≈ 65 % | eine TCP-Verbindung, keine Zusatzverbindung | G3-10 (Entscheidung) | ≈ 100 s | E13-S15 |
| SMB hoch 5000 × 4 KiB, L1 | robocopy /MT ≈ 1–3 s | ≈ 1–3 s | ≈ 25 s | ≈ 0,1 | Netz und Server ≈ 90 % | 2.2, Sperre | G3-02, G3-03, A | GUI ≈ CLI | Belege |
| SMB herunter 5000 × 4 KiB | robocopy ≈ 1–3 s | ≈ 1–3 s | Abbruch | 0 | – | 2.6 | G3-01 | Weg funktioniert | – |
| SMB hoch/herunter 1000 × 1 MiB, L1 | ≈ 9 s | ≈ 10–11 s | ≈ 10–11 s (Windows ≈ 30 s) | ≈ 0,85 (0,3) | Netz ≈ 15 % (70 %) | Einzelweg 5/8 RT, `CLOSE` im Drop | G3-05, G3-07, A | ≈ 9 s | keine |
| FTP hoch 5000 × 4 KiB, ein Ordner, L2 | lftp -P 8 ≈ 50–65 s | ≈ 80–90 s | ≈ 15 min | ≈ 0,06 | Netz ≈ 90 % (Hauptverbindung seriell) | 6. `LIST` je Datei, `stat` nur über `primary` | G3-04, G3-02 | CLI ≈ 50 s, GUI ≈ CLI | FTP-Nicht-Ersetzen (nicht atomar) |
| FTP hoch 1000 × 1 MiB, L2 | ≈ 90–95 s | ≈ 95 s | ≈ 170 s | ≈ 0,55 | Netz ≈ 45 % | Hauptverbindung | G3-04, G3-02 | ≈ 95 s | wie oben |
| FTP herunter 10 GiB, L4 | lftp pget -n 8 ≈ 95 s | ≈ 180 s | ≈ 180 s | ≈ 0,5 | Netz ≈ 50 % | ein Strom, S12 | G3-11 (Entscheidung) | ≈ 95 s | S12 |
| WebDAV hoch 5000 × 4 KiB (Nextcloud, L2) | rclone ≈ 45 s | ≈ 130 s | ≈ 260 s | ≈ 0,17 | Server: 6 statt 1 Anfrage | 7. | G3-02, G3-12 | CLI ≈ 90 s, GUI ≈ CLI | Nicht-Ersetzen, keine halben Dateien |
| Drive hoch 5000 × 4 KiB in bestehenden Ordner | rclone ≈ 28 min | ≈ 28–56 min | ≈ 28–56 min | ≈ 0,5–1 | Schreibgrenze | 2 Schreibaufrufe je Datei, reaktive Drosselung | G3-08 | ≈ 28 min | Drive-Eindeutigkeit |
| Drive hoch 10 GiB, 1 Gbit | rclone 64M ≈ 85 % Leitung (Annahme) | ≈ 65 % | ≈ 65 % | ≈ 0,75 | Netz ≈ 35 % | Stop-and-wait ohne Überlappung | G3-08c | ≈ 85–90 % | Fortsetzbarkeit |
| SFTP gleicher Server 5000 × 4 KiB, L2 | ssh cp < 1 s | ≈ 15–30 s | ≈ 30–40 s | ≈ 0,03 | Netz/Server fast nur Wartezeit | 9., keine Pakete | G3-06c, G3-13 | ≈ 2–5 s | Nicht-Ersetzen |

---

## 11. Zielentwurf und Maßnahmen

Gemeinsamer Zielentwurf für alle Netz-Backends: (1) **keine serielle Rundreise je Datei, die nicht
das Protokoll selbst verlangt**; Belege nehmen Metadaten aus den Antworten des Schreibwegs;
(2) **viele Dateien je Rundreise** (Pakete auch für mittlere Dateien und für entfernt → entfernt);
(3) **gelernte Tiefe und Parallelität ab dem ersten Byte** (Pipeline-Tiefe je Kanal, Flow-Start je
Backend); (4) **Netz läuft weiter, während lokal veröffentlicht wird** (Senke außerhalb der
Laufzeit); (5) Großdateien über so viele TCP-Ströme, wie Leitung und Server tragen.
Voraussetzung für die GUI-Zahlen ist Maßnahme A aus `analyse-kopieren.md` (Gruppen-Commit der
Belege); ohne sie bleibt die Sperrgrenze von ≈ 4,5 ms (Linux) bzw. 15–60 ms (Windows) je Datei.

### G3-01 Paket-Download von SFTP und SMB: Senke außerhalb der Laufzeit (Defekt)
- **Ort:** `sftp/core/batch_get.rs:34-47` und `get_all`; `smb/core/batch_wire.rs:436-449` und
  `smb/core/batch.rs` `get_all`.
- **Änderung:** `get_all` als Aufgabe auf der Backend-Laufzeit starten (`rt.spawn`), fertige Dateien
  in Reihenfolge über einen begrenzten Kanal (höchstens die Bytes des Pakets, die die Engine schon
  reserviert hat, `engine/batch_get.rs:62`) an den aufrufenden Faden geben; der Faden ruft
  `sink.begin/data/end/failed` außerhalb jedes `block_on`. Abbruch: Senkenfehler → Aufgabe
  abbrechen (`JoinHandle::abort`) und Kanal schließen.
- **Gewinn:** der Weg funktioniert im GUI; Netz und lokale Veröffentlichung überlappen.
- **Invarianten:** Liefer-Reihenfolge bleibt (Index-Reihenfolge wie heute `deliver`), `failed`/
  `changed`-Logik unverändert, kein Mitglied wird nach einem Senkenfehler mehr geliefert.
- **Test:** Engine-Lauf `recoverable: true` gegen den SFTP- und den Samba-Container
  (`native/tests/transfer_containers.rs` mit einem Job wie `paste_job`).

### G3-02 Belege ohne zusätzliche Rundreisen
- **Orte:** `transfer/os/shared/engine/file_recovery.rs:63-99`, `:105-178`, `:184-206`;
  `file_recovery_verify.rs:15-29`; `file_recovery_state.rs:135-153`; `file_evidence.rs:223-252`;
  `vfs/core/promotion.rs:95-103`, `:147-156`; Backend-Schreiber (`sftp/core/pool_writer.rs`,
  `smb/core/io.rs`, `webdav/core/stream_put.rs`, `gdrive/core/sized_writer.rs`).
- **Änderungen:**
  a. Für Ziele mit protokollseitigem Nicht-Ersetzen (`staged_write_capabilities().create`, SFTP,
     SMB, WebDAV, Agent) statt `previous_presence` einen neuen Wert `PreviousTarget::NoReplace`
     schreiben, den `previous_matches` wie `Absent` behandelt. −1 RT je Datei.
  b. Der Kopierstufen-Schreiber liefert beim `flush` die Metadaten der Stufe: SFTP `FSTAT` direkt
     hinter dem letzten `WRITE` gesendet (keine zusätzliche serielle RT); SMB `CLOSE` mit
     `SMB2_CLOSE_FLAG_POSTQUERY_ATTRIB` (Größe und Zeiten in der Antwort, MS-SMB2); Drive die
     JSON-Antwort des Create; WebDAV Größe aus dem `PUT` und `ETag` (Zeit unbekannt → dort bleibt
     ein `PROPFIND`, solange der Server keine Zeit im `PUT`-Ergebnis meldet). Neuer
     Backend-Aufruf, z. B. `committed_meta()` am Schreiber; `validate_copy_target(stage)` und
     `validate_staged_file` nutzen diese Werte. −2 RT je Datei.
  c. `after()` leitet bei Umbenennungen, die das Objekt behalten (SFTP `RENAME`, SMB
     `FileRenameInformation`, Agent), den veröffentlichten Beleg aus dem Stufenbeleg mit neuem Pfad
     ab; WebDAV und Drive nehmen die Antwort von `MOVE`/`PATCH`. −1 RT je Datei.
- **Gewinn:** SFTP/SMB-Einzelupload GUI 8 → 4 RT (CLI 5 → 4); WebDAV GUI 6 → 2–3; FTP GUI −9 RT
  und −1 Ordner-`LIST` je Datei; Drive GUI −2 bis −3 Aufrufe.
- **Invarianten:** „Absicht dauerhaft vor Veröffentlichung“ bleibt (der Beleg wird weiter vor
  `RENAME` geschrieben); „keine verlorene Antwort autorisiert ein erneutes Senden“ bleibt, weil
  Wiederaufnahme weiter über `previous_matches`/`doubt` entscheidet; bei einem fremden Ziel
  zwischen Beleg und Absturz entscheidet wie heute der Inhaltsvergleich (`doubt`). Prüfen mit den
  vorhandenen Tests `file_recovery_doubt_tests.rs`, `file_recovery_tests.rs` um den Wert
  `NoReplace` erweitert.

### G3-03 Pakete ohne Schleifen je Mitglied
- **Orte:** `engine/batch_put.rs:76-84`, `:130-167`; `engine/batch_get.rs` `Sink::end`;
  `vfs::BatchPutOutcome` (Erweiterung `Published { path, meta: Option<VfsMeta> }`);
  `sftp/core/batch_put.rs:141-156`; `smb/core/batch.rs:170-236`; Agent-Antwortrahmen.
- **Änderung:** eine Absichtszeile je Paket (setzt A voraus), kein `stat` je Mitglied vorher
  (G3-02a), Nachweis je Mitglied aus `BatchPutOutcome` (SFTP: `FSTAT` vor `CLOSE` im
  Mitglied-Future, SMB: Close-Attribute im Compound, Agent: Metadaten im Ergebnis).
- **Gewinn:** GUI 5000 × 4 KiB SFTP/SMB/Agent von ≈ 25–45 s auf ≈ CLI (≈ 1–3 s).
- **Invarianten:** wie G3-02; Paketfehlerbehandlung (`Attempt`, `Failures`) unverändert.

### G3-04 FTP: Metadaten parallel und ohne Ordner-`LIST` je Datei
- **Orte:** `ftp/core/ftp.rs:247-248` (`stat`), `:166-171` (`publish`); `ftp/core/metadata.rs:309-361`;
  `ftp/core/staging.rs:134-190`; `ftp/core/pool.rs:140-147`.
- **Änderungen:**
  a. `stat` zuerst auf einer freien Pool-Verbindung (`lease_idle`), nur ohne freie auf `primary`.
  b. Namensmenge je Auftrag und Ordner: ein `LIST` beim ersten Veröffentlichen in einen Ordner
     (bei einem vom Auftrag angelegten Ordner leer), danach um eigene Veröffentlichungen ergänzt;
     neu gelistet, wenn älter als ein fester Zeitraum (Vorschlag 2 s) oder wenn `RNTO` scheitert.
     Vor `RNFR`/`RNTO` zusätzlich `SIZE <Ziel>` (1 RT): 213 → belegt.
- **Gewinn:** O(n) statt O(n²) Listing-Bytes; GUI ohne globale Serialisierung. 5000 × 4 KiB L2:
  CLI ≈ 80–90 s → ≈ 50 s, GUI ≈ 15 min → ≈ 50–60 s.
- **Invariante:** FTP-Veröffentlichen ohne Ersetzen bleibt die dokumentierte nicht atomare Probe;
  das Zeitfenster zwischen Probe und `RNTO` wächst von „ein `LIST`“ auf höchstens die
  Auffrischfrist. Die `SIZE`-Probe des genauen Namens fängt fremde Dateien, die seit dem `LIST`
  entstanden sind (nicht fremde Ordner oder Links: `RNTO` auf einen Ordner scheitert,
  Links bleiben ein Restrisiko wie heute). Entscheidung der Frist: Nutzer/Sicherheit (Annahme:
  bisher keine feste Vorgabe).

### G3-05 Pakete für mittlere Dateien
- **Ort:** `transfer/core/engine_policy.rs:206-217`.
- **Änderung:** Kleindateigrenze `max(target ÷ 8, min(max_bytes ÷ 8, 2 MiB))`; Paketziel bei
  winzigen Dateien nach Anzahl (`max_files`) statt nur nach Bytes.
- **Gewinn:** 1000 × 1 MiB SFTP/SMB/Agent L3 ≈ 13–16 s → ≈ 9–10 s (63 Pakete à 16 MiB, ≈ 4 RT +
  Daten je Paket).
- **Invarianten:** keine (Paketsemantik unverändert); Speicher je Paket bleibt ≤ 16 MiB.

### G3-06 SFTP: weniger Rundreisen je Datei und je Serverkopie
- **Orte:** `sftp/core/pool_writer.rs:147-166`; `sftp/core/batch_put.rs:146-156`, `:203-213`;
  `transfer/os/shared/read_pipeline.rs:44-56`; `sftp/core/pool_channel.rs` (Feld für gelernte Tiefe);
  `sftp/core/copy_data.rs:150-200`.
- **Änderungen:**
  a. `CLOSE` direkt hinter dem letzten `WRITE` senden, nicht nach dessen Antwort (SFTP v3 verarbeitet
     Anfragen zu derselben Datei in Eingangsreihenfolge, draft-ietf-secsh-filexfer-02 §6.1);
     Veröffentlichung erst nach erfolgreichem `WRITE` **und** `CLOSE`. −1 RT je Datei und Paket.
  b. `Pipeline::new` mit Starttiefe aus dem Kanal (zuletzt erreichte Tiefe und schnellste Antwort).
     1-MiB-Datei: Datenphase ≈ 3 → 1 RT.
  c. `copy-data`: beide `OPEN` gleichzeitig senden, beide `CLOSE` gleichzeitig (`join`). ≈ 6 → 4 RT.
- **Invarianten:** keine halben Dateien unter dem Zielnamen (Umbenennen erst nach bestätigtem
  `WRITE`+`CLOSE`); Speicherbudget der Tiefe bleibt (`read_pipeline.rs:81-99`).

### G3-07 SMB: Compound auch im Einzelweg, `CLOSE` ohne Warten
- **Orte:** `smb/core/backend.rs:74-109`, `:145-175`; `smb/core/io.rs`; `smb/core/reader.rs` `Drop`.
- **Änderungen:** Stufe mit bekannter Länge ≤ `MaxWriteSize`: Compound `CREATE+WRITE+CLOSE`
  wie im Paket; Lesen ≤ `MaxReadSize`: Compound `CREATE+READ+CLOSE`; im `Drop` des Lesers
  `close()` auf der Laufzeit starten statt blockieren. Gelernte Lesetiefe wie G3-06b.
- **Gewinn:** Einzel-Upload ≤ 1 MiB 5 → 2 RT (CLI), Download 4 → 2 RT.
- **Invarianten:** exklusives Anlegen (`FileCreate`) bleibt; Länge wird weiter geprüft (Compound
  meldet geschriebene Bytes); ein Handle-Leck bei Verbindungsabbruch endet mit der Sitzung.

### G3-08 Google Drive
- **Orte:** `transfer/os/shared/engine/publish.rs:54-60`; `gdrive/core/sized_writer.rs`;
  `gdrive/core/promotion.rs:120-260`; `gdrive/core/overload.rs`; `gdrive/core/chunk_stream.rs`;
  `transfer/os/shared/engine/segments_download.rs` (`PrefixHasher`).
- **Änderungen:**
  a. Taktgeber für neue Objekte je Konto (Startrate 3/s, nach 403/429 senken, bei Erfolg langsam
     steigern), damit Worker nicht in Ratenfehler laufen und der Flow nicht halbiert.
  b. Weg „fresh“ auch in bestehende Ordner: vor dem Create `files.list` nach Name, freien Namen
     („Name (n)“) wählen, Create mit Endnamen, danach Eindeutigkeit prüfen; bei Doppel eigenes
     Objekt in den Papierkorb und nächsten Namen. Ein schreibender Aufruf je Datei statt zwei.
  c. Zwei Puffer im fortsetzbaren Upload: ein Faden sendet Abschnitt k, während der Worker k+1 füllt.
  d. MD5 beim Bereichsdownload aus den Puffern der Bereiche in Präfixreihenfolge statt Rücklesen.
- **Gewinn:** Stufen-Weg ≈ 2-mal so viele Dateien/s (falls `PATCH` zur Grenze zählt, Annahme);
  keine verlorenen Fehlversuche; 10 GiB bei 1 Gbit ≈ 65 % → ≈ 85–90 %.
- **Invarianten:** Drive hat in keinem Weg eine atomare Namensreservierung; die heutige Garantie
  (eindeutiger Name nach Abschluss, eigene ID zurückrollbar, `promotion.rs:230-275`) bleibt durch
  Vor- und Nachprüfung gleich; „keine halbfertige Datei“ gilt, weil Drive-Objekte erst vollständig
  erscheinen (K6, `publish.rs:1-6`); Fortsetzbarkeit des Uploads bleibt (Sitzungs-URI unverändert).

### G3-09 Flow-Start je Backend
- **Orte:** `transfer/os/shared/flow_registry.rs:27-62`; `vfs::Backend` (neuer Hinweis
  `transfer_start`).
- **Änderung:** Startwert SFTP/SMB/WebDAV/Agent 8, FTP 4 (Anmeldungen kosten 6–9 RT), Drive 4,
  höchstens `transfer_ceiling`; Paketmitglieder zählen als Abschlüsse im Messfenster
  (`analyse-kopieren.md` E).
- **Gewinn:** Hochlauf ≈ 2–5 s kürzer je Auftrag; wichtig für Aufträge unter 30 s.
- **Invarianten:** Überlast halbiert weiter; FTP-Grenze aus 421/530 bleibt.

### G3-10 SFTP: schnellere Zusatzverbindungen (Entscheidung nötig)
- **Orte:** `sftp/core/link_rules.rs:18-23`, `:107-131`; `sftp/core/pool_growth.rs`.
- **Änderung:** Messung ab dem ersten Byte statt nach festen 2 s, Fenster 0,5 s; weiterhin eine
  Anmeldung nach der anderen. Mit Entscheidung: Zusatzverbindungen auch bei SSH-Agent-Anmeldung.
- **Gewinn:** L4 10 GiB: Hochlauf ≈ 9 s → ≈ 3 s; mit SSH-Agent ≈ 260 s → ≈ 100 s.
- **Invariante:** Sicherheitsantwort E13-S15 (eine Anmeldung je Host und Port gleichzeitig, gleiche
  Host-Schlüssel, Sperre 15 min). Die Agent-Erweiterung ändert E13-S15 (i) und braucht eine
  neue Sicherheitsentscheidung.

### G3-11 FTP: Bereiche für große Downloads (Entscheidung nötig)
- **Orte:** `ftp/core/pool.rs:35`, `:56-67`; `ftp/core/ftp.rs:278-306`.
- **Änderung (nur nach Entscheidung zu S12):** für eine Datei ≥ 16 MiB bis zu N zusätzliche
  Anmeldungen erlauben (wie `lftp pget -n`).
- **Gewinn:** L4 10 GiB ≈ 180 s → ≈ 95 s.
- **Invariante:** S12 („nie eine neue Anmeldung für Bereiche“).

### G3-12 WebDAV
- **Orte:** `webdav/core/transfer_ops.rs:150-158`, `stage_move.rs:47-80`, `connection.rs:36-42`.
- **Änderungen:** G3-02 (kein `PROPFIND` vor `MOVE` und nach `MOVE`, Metadaten aus `PUT`/`MOVE`);
  für Nextcloud/ownCloud segmentierte Stufe über deren Chunk-Upload (parallele Teil-`PUT` in einen
  Upload-Ordner, ein abschließendes `MOVE`); `DELETE` der Stufe nach Fehlern weiter ungepoolt
  (ureq wiederholt sonst, `docs/TRANSFER_ENGINE.md`).
- **Gewinn:** 6 → 2–3 Anfragen je Datei im GUI; L4-Upload großer Dateien über mehrere TCP-Ströme.
- **Invarianten:** Nicht-Ersetzen über `Overwrite: F`; keine halben Dateien (Chunk-Upload wird erst
  beim `MOVE` sichtbar).

### G3-13 Pakete für entfernt → entfernt
- **Orte:** `engine/mod.rs:242-247`, `engine/batch.rs:77-89`, `sftp/core/copy_data.rs`.
- **Änderung:** gleiche SFTP-Gegenstelle: Serverkopie-Pakete (bis 64 `copy_to_stage`-Ketten auf
  einem Kanal, wie `put_all`); verschiedene Backends: Quelle `get_batch` → Ziel `put_batch` über
  einen begrenzten Puffer.
- **Gewinn:** SFTP gleicher Server 5000 × 4 KiB L2 ≈ 15–30 s → ≈ 2–5 s.
- **Invarianten:** wie Pakete heute (unbekanntes Ergebnis nie wiederholen).

### Reihenfolge der Umsetzung (Vorschlag)
G3-01 (Defekt) → A aus `analyse-kopieren.md` → G3-02 → G3-03 → G3-04 → G3-05 → G3-06 → G3-07 →
G3-09 → G3-08 → G3-12 → G3-13; G3-10 und G3-11 erst nach Entscheidung.

---

## 12. Messfälle für G5 (Anteil G3)

- **Gegenstellen auf CI (Linux):** OpenSSH-Container (`atmoz/sftp`, schon in der Suite), Samba
  (`docs/refs/samba-container.md`), vsftpd (schon in der Suite) und ein FTP-Server mit `MLST`
  (ProFTPD oder Pure-FTPd), WebDAV (Apache `mod_dav` oder `rclone serve webdav`), Drive über den
  vorhandenen HTTP-Nachbau (`gdrive/core/task_http.rs`, `gui_task_http.rs`) mit eingespeisten
  403-Ratenfehlern und Verzögerung.
- **Leitung:** `tc netem` auf Loopback mit 0,5 ms, 20 ms, 100 ms (Vorlage `native/rdt_task/speed_net.py`).
- **Fälle je Backend:** 5000 × 4 KiB (ein Ordner und 50 Ordner), 1000 × 1 MiB, 1 × 10 GiB (CI:
  1 × 2 GiB), hoch, herunter, gleiche Gegenstelle; jeweils `recoverable: true` und `false`.
- **Zähler statt Zeit (deterministisch, als Regressionswächter):** Anfragen je Datei mit
  `webdav/core/request_count_server.rs`, FTP-Befehle mit `ftp/core/data_overlap_server.rs`,
  SFTP-/SMB-Anfragen mit `sftp/core/batch_fake.rs`, `smb/core/batch_fake.rs`. Annahmesignale:
  SFTP-Einzelupload GUI ≤ 4 serielle Anfragen; Paket ≤ 4 RT unabhängig von N; FTP: ≤ 1 `LIST` je
  Ordner und Frist; Drive: 1 schreibender Aufruf je Datei.
- **Referenzen auf denselben Daten:** `rsync -a`, `sftp -B 262144 -R 64`, `rclone` (sftp, smb,
  webdav), `smbclient`, `lftp mirror -R -P 8`, `lftp pget -n 8`, `curl` (WebDAV-`PUT`).
- **Defekt G3-01:** ein Lauf SFTP → lokal und SMB → lokal mit `recoverable: true` und ≥ 2 kleinen
  Dateien muss heute abbrechen und nach der Änderung vollständig ankommen.

---

## 13. Unbestätigt, offen, Entscheidungen

- **Nicht gemessen:** alle Zeiten, Raten, Prozente und Leerlaufanteile.
- **Bibliotheksinterna (Annahmen):** russh-sftp verarbeitet gleichzeitige Anfragen auf einem
  `SftpSession`/`RawSftpSession` über Anfrage-IDs; Pipelining-Tiefe des russh-sftp-`File` auf dem
  Hauptkanal; Größe von `smb2::DOWNLOAD_CHUNK_SIZE` und Standardfenster von `FileWriter`;
  Sitzungswiederaufnahme von rustls auf FTPS-Datenverbindungen; ureq-Poolverhalten unter Last.
- **Serververhalten (Annahmen):** OpenSSH `copy-data` ab 9.0; OpenSSH `RENAME` von Dateien über
  `link()`+`unlink()` (atomar ohne Ersetzen), von Ordnern über `stat`+`rename` (nicht atomar);
  `limits@openssh.com` Werte; Google zählt `PATCH` gegen die Schreibgrenze oder nicht.
- **Nicht geprüft:** ob MCP-Kopien `recoverable` setzen; andere Verwender von
  `Backend::parallelism()`; der Hauptkanal-Schreiber von SFTP (`io_adapters.rs`) im Detail;
  `smb/core/server_copy.rs` über die Schrittfolge hinaus; Android-spezifische Pfade der Backends.
- **Entscheidungen des Nutzers bzw. der Sicherheit:** G3-10 (Zusatzverbindungen bei
  SSH-Agent-Anmeldung, E13-S15), G3-11 (Bereiche mit neuer FTP-Anmeldung, S12), G3-04
  (Auffrischfrist der FTP-Namensmenge), G3-02 (`PreviousTarget::NoReplace` als neue Belegsemantik).

---

## 14. Gelesene Dateien

`docs/ARCHITEKTUR.md` (Ausschnitte), `docs/TRANSFER_ENGINE.md`, `docs/refs/smb2.md` (Ausschnitte),
`native/Cargo.toml` (Abhängigkeiten, Profile), `native/Cargo.lock` (Versionen),
`native/test-transfer-engine-task.sh` (Ausschnitte), `native/tests/transfer_containers.rs` (Kopf),
`native/tests/transfer_task/support.rs` (`job`).
`native/src/`: `transfer/os/shared/engine/{mod, worker (1-200), ops, upload, publish, download
(1-336), remote_copy, batch, batch_put (1-200), batch_get, segments, segments_download (174-256),
segments_copy (123-163), file_recovery (1-260), file_evidence (150-300), file_recovery_verify (15-30),
file_recovery_state (120-155), file_recovery_receipts (46-60, 312-330), job_store (77-89),
job_store_validation (25-80), source (329-366), prepare, folders (380-410), dispatch (40-100),
view (37-45)}.rs`; `transfer/core/{segments, flow_control (25-60), engine_policy (30-60, 192-218)}.rs`;
`transfer/os/shared/{read_pipeline, walk (1-60, 140-200, 265-310), flow (200-240)}.rs`;
`vfs/core/{core (60-300), file_identity, extensions (55-80), promotion (83-170),
backend_defaults (216-223)}.rs`;
`sftp/core/{backend, connection, session (20-45, 95-200, 305-345), channel_pool, link_rules
(240-376), pool_channel (1-140), pool_writer, pool_reader, pipelined_read, batch_ops, batch_put,
batch_get, copy_data, transfer_ops (54-87), segmented_stage (1-60, 376-383), exec (50-120)}.rs`;
`agent/core/{backend (1-40, 200-493), pool, batch_put (1-40, 121-181), stream (302-340),
folder_reservation (66-88), deploy (150-187), transfer (60-110)}.rs`;
`agent_proto/core/{credit (1-115), features (55-90, 140-167), batch_limits (Konstanten), server
(60-200)}.rs`; `agent_proto/os/shared/{batch_put (1-40), put_tree (1-30)}.rs`;
`smb/{mod}.rs`, `smb/core/{backend, session, reader, io, batch (1-260), batch_wire (420-449),
server_copy (167-210)}.rs`;
`ftp/core/{ftp, pool, staging, metadata (309-370), metadata_probe, connection (330-405), streams
(95-145, 300-427)}.rs`;
`webdav/core/{connection, webdav (40-120, 280-470), stream_put (1-60), stage_move, transfer_ops}.rs`;
`gdrive/core/{backend (115-140, 190-360), sized_writer, chunk_stream, http, id_pool (1-67),
resumable (Konstanten), promotion (100-260), listing (53-83), overload (1-60), folder_reservation
(16-46), cache (159-166)}.rs`.
Außerhalb des Quellstands gelesen (nur dort vorhanden): `analyse-plan.md`, `analyse-kopieren.md`
dieses Plan-Ordners.
