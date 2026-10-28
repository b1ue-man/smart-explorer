# Analyse G2: Gerät zu Gerät (Direct Share, Raum) – Aufrufketten, Parameter, Zeitlinien, Zielentwurf

Stand: 2026-10-28. Quellstand: eingefrorene Kopie von `a67d7296` (Teil G2 des Analyseplans
`analyse-plan.md`). Rein lesende statische Analyse: nichts gebaut, nichts ausgeführt, nichts
gemessen. Jede Zeit- oder Ratenangabe ist eine Schätzung aus dem Code plus allgemeinem
Plattformwissen und ist als **(Schätzung)** markiert; die Messung liefert G5. Pfade relativ zu
`native/src/`, Fremdcode unter `vendor/` bzw. `share-server/`.

Baut auf `analyse-kopieren.md` (erster Durchgang) auf. Dessen Zählungen je Datei (Belege mit
4 dauerhaften Schreibvorgängen „DW“, Roundtrips „RT“) werden hier nicht wiederholt, sondern
geprüft, korrigiert (Abschnitt 9) und um die Transport-, Proxy- und Hostschicht ergänzt.

---

## 0. Die wichtigsten Befunde

1. **Jeder Share-Weg aus GUI, Android-App, `se cp` und MCP läuft über einen Loopback-Proxy.**
   Die Engine läuft im aufrufenden Prozess; ihr Share-Backend ist ein `AgentBackend` über *eine*
   Loopback-TCP-Verbindung zum Worker (`daemon/os/shared/ipc_open_client.rs:41-67`), auch auf
   Android im selben Prozess (`mobile/os/shared/pool.rs:204-207`). Der Worker startet je
   Anfrage einen neuen OS-Thread (`daemon/os/shared/backend_server.rs:188-190`). Je Byte kommen
   zwei Kopien und zwei Kernelübergänge hinzu, je Anfrage ein Threadstart und vier Threadwechsel.
2. **Über diesen Proxy gibt es für Share keine Pakete (PutBatch/GetBatch).** Der Worker kündigt
   Pakete nur an, wenn `backend.batch_limits(root_display())` etwas liefert
   (`backend_server.rs:65-76`). Für `PeerBackend` ist `root_display()` = `"/"`
   (`share/core/backend.rs:157-159`), und `"/"` ist ein synthetischer Container ohne Pakete
   (`share/core/peer_transfer.rs:127-132`, `fs_capabilities.rs:46-51`). Deshalb meldet der
   `AgentBackend` keine Pakete (`agent/core/backend.rs:218-224`, `agent_proto/core/features.rs:144-146`).
   Folge: Jede kleine Datei nimmt den Einzelweg mit 8 RT (Upload) bzw. 4 RT (Download). Das ist
   statisch belegt; ein Test für Pakete über den Proxy fehlt.
   **Damit wirken die KP2-Teile K-B (Paketantworten) und K-D (Paketstufen) für Oberflächenwege
   erst, wenn dieser Fehler behoben ist.**
3. **Die Hostseite prüft Rechte und Pfade je Anfrage und je Datenblock mit vollem Aufwand.**
   - `AccessAuthority::check()` → `session.authorize()` (`share/core/fs_authority.rs:53-75`,
     `session.rs:150-156`) läuft unter der globalen `auth`-Sperre:
     - bei Direct: HMAC-Prüfung, SHA-256-Fingerabdruck, Kopie der Exportliste;
     - bei **Räumen**: zusätzlich ein Lesen des Raumgeheimnisses aus dem Geheimnisspeicher
       (`share/os/shared/profile_store.rs:232-234, 343-355`). Unter Linux/Android heißt das
       Ordner öffnen, `flock`, Datei lesen und AEAD-Öffnen (`creds/os/linux_file_store.rs:96-135`),
       unter Windows `CredReadW`.
   - Lesen: je 256-KiB-Block drei solche Prüfungen (`fs_guard_backend.rs:454-466` zweimal,
     `server_read.rs:107` einmal).
   - Jede Pfadprüfung `TargetPolicy::read` (`share/os/shared/fs_host_policy.rs:64-91`) macht
     `canonicalize`, `is_dir`, öffnet den Ordner und geht dann bis zur Wurzel hoch:
     - Linux: mit `..`-Öffnungen (`local_access/os/linux/private_ancestors.rs:12-40`);
     - Windows: mit gepinnten Vorfahren-Handles (`local_access/os/windows/directory_handle.rs:76-95`).
4. **Pro Gegenstelle gibt es genau eine QUIC-Verbindung.** Damit hat sie einen
   Verbindungs-Treiber-Task (ein Kern für Paketbau, AES-GCM und Senden) und einen
   Cubic-Regler. Die Nutzdaten sind auf genau einen zugelassenen Pfad beschränkt
   (`vendor/SMART_EXPLORER_ROUTE_POLICY.md`), die MTU-Obergrenze ist 1452 Byte
   (`vendor/noq-proto-1.3.0/src/config/transport.rs:900-908`). Auf 1 Gbit reicht das;
   auf 10 Gbit ist das die erwartete Decke **(Schätzung: 2–5 Gbit/s Desktop, deutlich weniger
   auf Telefonen)**.
5. **Fenster und Bandbreite-Verzögerungs-Produkt sind kein Engpass.** Stromfenster 16 MiB,
   Verbindungsfenster 16–64 MiB (`share/core/keepalive.rs:22-25, 90-100`). Grenzfall:
   10 Gbit bei ≥ 50 ms. Der Relay-Weg ist serverseitig auf 64 MiB/s je Client begrenzt
   (`share-server/src/relay.rs:23-24, 167-182`), läuft als QUIC über TCP und beginnt erst nach
   Ablauf der Direktfrist (`share/core/route_policy.rs:49-71, 117-133`).
6. **Die kleinen Dateien begrenzen weiterhin vor allem die Belege (K-A, nicht KP2).**
   3 DW je Datei laufen unter der Auftragssperre. Dazu kommen 8 bzw. 4 Anfragen je Datei, jede
   über Proxy, Worker-Thread, QUIC-Strom und volle Hostprüfung. Schätzung für 5000 × 4 KiB über
   LAN: Linux-Client ≈ 200 Dateien/s, Windows-Client ≈ 20–70 Dateien/s; `rsync` schafft
   ≈ 2000–5000 Dateien/s.
7. **Android als Host oder Ziel arbeitet auf FUSE.** Der Speicherzugriff läuft über
   `MANAGE_EXTERNAL_STORAGE` auf MediaProvider-FUSE; SAF und MediaStore werden für
   Übertragungen nicht genutzt.
   - Jede Pfadprüfung des Hosts geht über FUSE.
   - Jede Veröffentlichung läuft die Umbenennungsleiter durch (`android_fs/os/rename.rs:1-22`):
     ohne `RENAME_NOREPLACE` und ohne Hardlinks bleiben Prüfung plus `rename`.
   - Ein `ContentObserver` auf allen MediaStore-Dateien stößt während einer Übertragung
     Sync-Nachholläufe an (`android/.../system/MediaStoreSync.kt:26-46`).
8. **LZ4 greift bei kleinen Dateien nie und arbeitet am Proxy doppelt.**
   - Die Regel braucht je Strom ein volles Messfenster von 1 s (`transfer/core/compress_policy.rs:1-19`),
     und jeder Strom beginnt mit neuer Regel (`peer_writer.rs`, `lz4_stream.rs:43-47`).
   - Pakete komprimieren nie.
   - Am Proxy kann dieselbe Datei für die Loopback-Strecke gepackt und im Worker wieder
     entpackt werden (`agent/core/stream.rs:190-192`, `backend_server.rs:80-89`).

---

## 1. Prozesse, Threads, Verbindungen

### 1.1 Wo die Engine läuft und welches Backend sie sieht

| Einstieg | Engine-Prozess | Share-Backend der Engine | Weg zum QUIC |
|---|---|---|---|
| GUI Einfügen, OS-Zwischenablage in se, Ziehen hinein | GUI | `AgentBackend` (`daemon::open_share_backend`, `app/core/share.rs:301`) | Loopback-TCP → Worker → `PeerBackend` |
| Android-App (`mobile/os/shared/transfer.rs` → `drive::run_job`) | App-Prozess | `AgentBackend` (`mobile/os/shared/pool.rs:204-207`) | Loopback-TCP **in denselben Prozess** → eingebetteter Worker → `PeerBackend` |
| `se cp`/`se mv` | CLI | `AgentBackend` (`connect/os/shared/resolution.rs:31-34`) | wie GUI |
| MCP `copy`/`move` (`cli/mcp/transfer_tools.rs:22-60`) | `se mcp` | wie CLI | wie GUI |
| Explorer zieht entfernte Dateien (Windows, virtuelle Dateien) | GUI | `AgentBackend` | wie GUI, je Datei ein `Read` |
| se-Laufwerk auf Share (G4) | Worker | `PeerBackend` direkt (`daemon/os/shared/mount_source.rs:51-53`; der Kommentar dort meidet den Loopback-Proxy ausdrücklich) | direkt |

Der Worker liefert für `OpenShare` den nackten `PeerBackend` aus (`daemon/os/shared/ipc.rs:160-166`,
`share/core/service.rs:135-148`). Für alle Benutzereinstiege gilt damit die Kette
**Engine → AgentBackend → Loopback-TCP → Worker → PeerBackend → QUIC → Host-Worker → Dateisystem**.

### 1.2 Threads und Tasks je Seite

| Ort | Thread/Task | Anzahl | Beleg |
|---|---|---|---|
| Client-Engine (GUI/CLI/App) | Engine-Worker (je Datei bzw. Teil) | bis zum Flow-Deckel (Proxy meldet `service_slots` = min(56 + Reserve, 64)) | `agent_proto/core/features.rs:80-86`, `agent/core/backend.rs:461-464` |
| Client-Engine | Walker (Discovery) | 1 je Auftrag | `engine/discovery.rs` (G1) |
| Client-AgentBackend | Schreib-Thread (Lanes) + Lese-Thread | je 1 je Proxyverbindung | `agent/core/mux.rs:1-20` |
| Worker (Client-Gerät) | Lese-Thread der Proxyverbindung | 1 | `backend_server.rs:124-140` |
| Worker | **neuer OS-Thread je Anfrage** (`daemon-backend-request-{id}`) | bis 64 gleichzeitig (`CREDIT_REQUEST_LIMIT`) | `backend_server.rs:188-190`, `agent_proto/core/credit.rs:21-36` |
| Worker | Tokio-Laufzeit `share-iroh` (Mehrfädig, Standard = Kernzahl) | Kerne | `share/core/node.rs:141-147` |
| Worker | noq-Verbindungstreiber | **1 Task je Verbindung**, eine Verbindung je Gegenstelle | `node_sessions.rs:278-312` |
| Host-Worker | Tokio `share-iroh` | Kerne | wie oben |
| Host-Worker | 1 Tokio-Task je angenommenem Strom | je Strom | `share/core/server.rs:210-256` |
| Host-Worker | 1 `spawn_blocking`-Thread je Übertragung (Read/Write/Batch/Range) | ≤ 60 je Gerät, ≤ 256 je Host | `blocking.rs:67-80`, `server_admission.rs` (HOST 256, PRINCIPAL 60) |
| Host-Worker | Steuerpool (Stat, FileIdentity, CreateDir, Promote …) | 32 Erlaubnisse auf `spawn_blocking` | `blocking.rs:17` |

### 1.3 Ein Byte auf dem Weg (Kopien, Sperren, Threadwechsel)

**Upload, Client-Gerät (GUI → Host)**

1. Engine-Thread liest die Datei in einen 256-KiB-Puffer (Kernel→User-Kopie 1).
2. `AgentBackend` baut einen `Frame::Data`, gegebenenfalls mit LZ4 (`agent/core/stream.rs:190-192`; Kopie 2), und legt ihn in die Lane-Warteschlange (Threadwechsel 1).
3. Der Mux-Schreib-Thread schreibt auf den Socket (Kopie 3, Kernel).
4. Der Worker-Lese-Thread liest den Frame (Kopie 4) und leitet ihn über einen `sync_channel` mit Rückstau 32 an den Anfrage-Thread weiter (Threadwechsel 2, `agent_proto/core/server_session.rs:59`).
5. Der Anfrage-Thread schreibt in `PeerWriter::write` je 256 KiB ein `block_on_route(send_tagged)` (`share/core/peer_writer.rs:217-252`). Dabei:
   - `send_tagged` macht drei `write_all` (Länge, Tag, Nutzlast; `framing.rs:74-88`);
   - jedes `write_all` sperrt die Verbindungs-Mutex von noq und weckt den Treiber (`vendor/noq-1.3.0/src/send_stream.rs:139-175`; Kopie 5 in den Sendepuffer);
   - `flush` ist wirkungslos (`send_stream.rs:324-326`).
6. Der noq-Treiber auf einem Tokio-Worker (Threadwechsel 3) baut Pakete mit höchstens 1452 Byte, verschlüsselt sie (Kopie 6) und sendet mit GSO, wo die Plattform es kann (`vendor/iroh-1.3.0/src/socket/transports/ip.rs:237-247`).

**Upload, Host**

1. Der noq-Treiber entschlüsselt.
2. Der Strom-Task liest mit `recv_tagged_limited` in einen null-initialisierten `vec![0; n]` (`framing.rs:137-154`; memset und Kopie).
3. Der Block geht über ein `mpsc` mit Kapazität 2 an den Blocking-Thread (`server_transfer.rs:25, 58, 121`).
4. `GuardedWriter::write` macht je Block eine Rechteprüfung (`fs_guard_backend.rs:470-484`), dann folgt `write` in die Stufe.

**Download** läuft spiegelbildlich, mit diesen Kosten:

- Host liest in einen frisch allozierten, null-initialisierten 256-KiB-`Vec` je Block (`server_read.rs:150`).
- Je Block drei `authorize`, zwei davon in `GuardedReader` (`fs_guard_backend.rs:454-466`) und eins in `server_read.rs:107`.
- `send_tagged`.
- Client-Worker: `PeerReader` in einen neuen `Vec` je Rahmen, dann `copy_from_slice` in den Puffer (`peer_read.rs:66-115`).
- `Frame::Data` über den Proxy; der Worker kann LZ4 packen, wenn das Schreiben zur GUI „blockiert“.
- GUI-Lese-Thread → Engine-Thread → `write`/`pwrite`.

**Summe je Byte auf dem Client-Gerät (Schätzung):** etwa 7 Speicherkopien und 2 zusätzliche
Kernelübergänge gegenüber 3 Kopien ohne Proxy, dazu AES-GCM und ein Speicher-Nullsetzen je
Block. Bei 1 Gbit unkritisch. Bei 10 Gbit und auf Telefonen kosten Kopien und der eine
Proxy-Socket einen spürbaren Teil eines Kerns bzw. der Speicherbandbreite.

---

## 2. Transport: iroh/noq-Parameter und ihre Wirkung

| Parameter | Wert | Beleg | LAN 1 Gbit / 10 Gbit | hohe RTT / Relay |
|---|---|---|---|---|
| Verbindungen je Gegenstelle | 1 (Sitzungs-Cache je `session_key`) | `node_sessions.rs:278-312` | ein Treiber-Task = ein Kern für Paketbau + AES-GCM; **10 Gbit: Decke (Schätzung 2–5 Gbit/s)** | ein Cubic-Fluss; bei Verlust wie ein einzelner TCP-Strom |
| Bidi-Ströme | 64 (Uni 0), davon 4 für Blättern | `keepalive.rs:9-17` | 60 Übertragungsströme | ok |
| Stromfenster | 16 MiB | `keepalive.rs:22` | ≫ BDP (1 Gbit × 0,5 ms = 62 KB) | 1 Gbit × 100 ms = 12,5 MB < 16 MiB: ok |
| Verbindungsfenster / Sendefenster | `budget/4` begrenzt auf 16–64 MiB; Sendefenster = Verbindungsfenster | `keepalive.rs:90-100`; Budget = ¼ des verfügbaren Speichers, 64 MiB–2 GiB (`transfer/os/shared/memory.rs:13-37`) | ok | 10 Gbit × 50 ms = 62,5 MB ≈ 64 MiB: Grenzfall; bei < 256 MiB freiem Speicher nur 16 MiB |
| Ältere Gegenstellen | noq-Standard 1,25 MB Stromfenster | `keepalive.rs:19-21, 88-89` | ok | Upload je Strom ≤ 1,25 MB/RTT (bei 30 ms ≈ 42 MB/s); Teile parallel helfen |
| Staukontrolle | Cubic; BBRv3 nur per Testschalter `SE_SHARE_QUIC_CC` | `keepalive.rs:105-121` | ok ohne Verlust | WLAN/WAN mit Verlust: ein Fluss, kein `-P N` |
| MTU | Start 1200, Suche bis **1452** (IPv6-sicher), Wiederholung alle 600 s | `noq-proto/src/lib.rs:360`, `config/transport.rs:583-585, 900-908` | IPv4-LAN könnte 1472; Jumbo-Rahmen ungenutzt; ~3 % Kopf | ok |
| GSO/GRO | an (`enable_segmentation_offload: true`), IP-Transport nutzt `max_gso_segments` | `transport.rs:601`, `iroh/.../ip.rs:237-247` | Linux ja; Windows/Android plattformabhängig (ungeprüft) | Relay: über WebSocket/TCP, kein GSO |
| Multipath | iroh: bis 8 Pfade; **Nutzdaten nur auf genau einem zugelassenen Pfad** | `iroh/src/endpoint/quic.rs:156-166`, `vendor/SMART_EXPLORER_ROUTE_POLICY.md` | keine Bündelung zweier Schnittstellen | – |
| Keepalive / Leerlauf | 5 s / 20 s | `keepalive.rs:6-8` | – | – |
| Datagramme | nicht für Dateidaten (nur Ströme) | `keepalive.rs:10` | – | – |
| Relay-Server | 64 MiB/s Empfang je Client, 8 MiB Burst, 4 Verbindungen je Endpunkt | `share-server/src/relay.rs:14-24, 163-182` | – | **Decke ≈ 537 Mbit/s** inkl. QUIC-Kopf; QUIC über TCP (Head-of-line, doppelte Regelung) |
| Pfadzulassung | LAN sofort, Direkt nach LAN-Ende, Relay erst nach `direct_until` = Sammeln (3×PTO) + NAT-Wiederholungen + Antwort (3×PTO) + Validierung (3×PTO); Prüftakt 25 ms; späterer LAN/Direkt-Pfad wird übernommen | `route_policy.rs:49-71, 117-133, 141-160`, `share/os/shared/route_monitor.rs:19` | Zeit bis zum ersten Byte ≈ 1–3 RTT nach Validierung | **Relay-only: erstes Byte erst nach Sekunden (Schätzung 1–5 s)** |
| Laufzeit | Tokio mehrfädig, Kernzahl Threads | `node.rs:141-147` | Treiber teilen sich die Worker | – |

**Folgerung zu Fenstern und BDP:** Bei keiner realistischen LAN-, Direkt- oder Relaystrecke
begrenzen Fenster den Durchsatz. Ausnahmen sind Uploads zu älteren Hosts und 10 Gbit ab
≥ 50 ms RTT. Die Decken liegen anderswo:

- CPU je Verbindung (ein Treiber);
- der Proxy;
- die Relay-Rate;
- vor allem die Kosten je Datei (Abschnitte 4–5).

---

## 3. Rahmen und Zeitfristen je Anfrage

- **Jede Anfrage ist ein neuer bidirektionaler Strom.** Sie trägt einen JSON-Steuerrahmen
  (`Ctrl::Fs`, `framing.rs:40-42`) und eine JSON-Antwort. Das Öffnen braucht keinen RT, aber
  je Anfrage fallen an:
  - Client: `open_stream_until`: Sharing aktiv?, `policy.snapshot`, Sitzungs-Mutex,
    `await_route` mit Route-Registry-Mutex und Watch-Prüfung, `open_bi` in einem eigenen
    `block_on` (`node_sessions.rs:143-236`); danach ein weiteres `block_on_route` für Senden und
    Empfangen (`peer_request.rs:147-189`). Mindestens zwei Eintritte in die Laufzeit aus einem
    Fremd-Thread mit Wecken über Threadgrenzen.
  - Host: `tokio::spawn` je Strom (`server.rs:235`), JSON-Parse, `session.authorize`,
    `AccessAuthority::new` (zweites `authorize_state` + `policy.snapshot`), Kopie der
    Exportliste (`fs_access.rs:44-60`). Danach je Anfrageart: Steuerpool oder
    Übertragungszulassung mit `spawn_blocking`.
- **Zeitfristen:** `io_deadline::run`/`run_for` legt je Rahmen einen Timer an (Rahmen- und
  Blockebene, µs-Kosten, kein Engpass). `peer_stream::frame` tickt beim Streaming alle 100 ms
  für den Abbruch (`peer_stream.rs:62-81`).
- **Telemetrie:** Erfolgsmeldungen nur stichprobenartig (`peer_telemetry.rs:13-32`), kein Engpass.

---

## 4. Aufrufketten je Datei: der tatsächliche Oberflächenweg

Weil der Proxy keine Pakete anbietet (Befund 2), nimmt **jede** Datei aus GUI, App, CLI und MCP
den Einzelweg. Jede Zeile „RT“ ist hier: Engine-Thread → AgentBackend-Anfrage → Loopback →
Worker-Lese-Thread → **neuer Worker-Thread** → `PeerBackend`-Anfrage → neuer QUIC-Strom → Host
(Strom-Task, `authorize`, `resolve`, Pfadpolitik, Steuerpool-Thread) → Antwort zurück über
dieselbe Kette.

### 4.1 Upload einer Datei unter 16 MiB (lokal → Share), Einzelweg

| # | Schritt | Kosten | Beleg |
|---|---|---|---|
| 1 | Elternordner, falls neu: `CreateDir` | 1 RT je neuer Ebene | `peer_transfer.rs:169-184`, `engine/folders.rs` |
| 2 | Quelle öffnen und prüfen (lstat, open, fstat, Prüfung) | lokal | `engine/source.rs:48-71` |
| 3 | `try_split`: kleiner als 16 MiB → nein | – | `engine/segments_upload.rs:116`, `transfer/core/segments.rs:11-13` |
| 4 | `publish::open` → `open_write_copy_stage_unsynced` → Vorgabe `open_write_copy_stage_sized` → `open_write_new` → `open_owned_writer` (`WriteNew`): Anfrage, **auf `Ready` warten** | 1 RT; Host: `resolve_write` (≈ 4 `canonicalize` + `exists`) + `TargetPolicy::new` (2 `canonicalize`) + `policy.read`/`write` (canonicalize, `is_dir`, Ordner öffnen, Aufstieg zur Wurzel) + `open_write_new` (kein fsync) | `engine/publish.rs:76-110`, `vfs/core/core.rs:92-115`, `share/core/peer_stages.rs:300-312`, `peer_request.rs:329-410`, `server_transfer.rs:197-257`, `fs.rs:161-210`, `fs_local_paths.rs:12-48`, `fs_host_policy.rs:24-91` |
| 5 | Daten in 256-KiB-Rahmen, je Rahmen ein `block_on` | 1 MiB = 4 Rahmen | `peer_writer.rs:217-252` |
| 6 | `flush` → `WriteDone`, **auf `Ok` warten** (Host: erneutes `resolve_write`, `flush`) | 1 RT | `peer_writer.rs:139-184`, `server_transfer.rs:273-289` |
| 7 | `before()`: Stat Stufe, FileIdentity Stufe, Stat Ziel; `start_file` = 3 DW unter Auftragssperre | 3 RT + 3 DW | erster Durchgang 2.1/2.2 |
| 8 | `PromoteNoReplace` (Host: Steuerpool, `resolve` ×2, guarded `promote_no_replace`) | 1 RT | `peer_transfer.rs:188-222`, `server_fs.rs:281-295` |
| 9 | `after()`: Stat + FileIdentity + 1 DW | 2 RT + 1 DW | erster Durchgang |

**Summe:** 8 RT (ohne Ordner) + 4 DW. Dazu je RT ein Worker-Threadstart, zwei
Loopback-Durchgänge und hostseitig etwa 10–15 `canonicalize`-Äquivalente plus ein Aufstieg zur
Wurzel.

### 4.2 Download einer Datei unter 16 MiB (Share → lokal), Einzelweg

| # | Schritt | Kosten | Beleg |
|---|---|---|---|
| 1 | `existing()` und `read_size` (für Peer ohne RT) | lokal | `engine/download.rs:123-137` |
| 2 | Freier Platz (`statfs`) und `.part` anlegen | lokal | `download.rs:138-146` |
| 3 | `try_split`: kleiner als 16 MiB → nein | – | `segments_download.rs:184-189` |
| 4 | `Read` öffnen; Host: `resolve` + `stat_item` + guarded `open_read` (Politik und Rechte) + 3 `authorize` je Block | 1 RT bis zum ersten Byte | `peer_request.rs:244-326`, `server_read.rs:73-123, 189-230` |
| 5 | Daten | – | `peer_read.rs:55-117` |
| 6 | `check.finish` → Stat der Quelle (unverändert seit der Auflistung) | 1 RT | `engine/source.rs:343-388` |
| 7 | `publish_local`: `before()` mit Stat + FileIdentity der Quelle und 3 DW; Veröffentlichen ohne Ersetzen; `after()` mit 1 DW | 2 RT + 4 DW | `download.rs:270-336` |

**Summe:** 4 RT + 4 DW.

### 4.3 Große Dateien (ab 16 MiB): Teile

- **Teilung:** ab 2 × 8 MiB. Teillänge = clamp(10 × Aufbauzeit × Rate, 8 MiB, max(8 MiB, 30 s × Rate))
  (`transfer/core/segments.rs:11-70`).
- **Eigentümer und Helfer:** Der Eigentümer-Worker überträgt Teile. Solange der Flow freie
  Erlaubnisse hat, legt er Tickets aus; Helfer nehmen weitere Teile, zuerst Speicher, dann
  Erlaubnis (`engine/segments.rs:268-380`).
- **Download:**
  - Teil: `ReadAt{offset,len}` mit 1 RT bis zum ersten Byte, `pwrite` in `.part`
    (`segments_download.rs:46-104`, `peer_stage_ranges.rs:63-89`);
  - Host: je Teil ein neuer Lese-Thread mit 512 KiB Vorlauf (`server_read.rs:25, 81-87`);
  - am Ende `settle` als 1-ms-Schlafschleife, bis kein Helfer die Datei mehr hält
    (`segments_download.rs:252-256`).
- **Upload:**
  - `SegmentedStageOpen` (1 RT); je Teil ein Strom mit `StageWriteAt`, **Warten auf `Ready`**
    (1 RT), Daten, `finish` mit Warten auf `Ok` (1 RT) (`peer_stage_ranges.rs:94-131, 196-245, 292-340`);
  - danach `SegmentedStageFinish` (1 RT) und die Veröffentlichungsfolge aus 4.1 Schritte 7–9.
- **Über den Proxy:** Teile werden durchgereicht, weil der Worker `range-v1` und `lz4-v1`
  ankündigt (`daemon/os/shared/backend_ranges.rs:43-58`). Jeder Teil kostet einen
  Worker-Thread.
- **Alle Teile laufen über dieselbe QUIC-Verbindung**, also einen Treiber und einen
  Cubic-Fluss. Mehr Teile helfen gegen Lese- und Schreibwartezeit, nicht gegen die
  Verbindungsdecke.

### 4.4 Pakete (nur bei direktem `PeerBackend`)

Die Paketwege aus dem ersten Durchgang (2.2, 2.3, 4) sind korrekt beschrieben, aber aus keinem
Benutzereinstieg erreichbar (Befund 2). Ergänzungen:

- **PutBatch:** Der Client sendet Kopf und Daten, ohne auf `Ready` zu warten
  (`peer_batch_put.rs:195-238`); das ist gut. Danach folgt `WriteDone` und 1 RT bis zu den
  Ergebnissen (`peer_batch_put.rs:125-190`).
- **GetBatch, Host:** je Eintrag
  - drei Ereignisse (`Begin`, `Data`, `Done`), jedes ein eigener `send_tagged` mit drei
    `write_all` und eigenem `check_read` (`server_batch_get.rs:57-88`);
  - `resolve`, `stat_item` vorher, `read_size`, guarded `open_read_id`, `stat_item` nachher
    (`server_batch_get.rs:113-145, 173-180`);
  - Schätzung: ≈ 10 `authorize` und ≈ 5 `TargetPolicy::read` je Eintrag.
- **GetBatch, Client:** drei `block_on` je kleinem Eintrag (`peer_batch_get.rs:119-185`).

### 4.5 Hostseite: Kosten je Anfrage und je Block

| Prüfung | Häufigkeit | Inhalt | Beleg |
|---|---|---|---|
| `session.authorize` | je Strom (`server.rs:290`), je `AccessAuthority::check` (je `resolve`, je guarded Operation, **2 × je gelesenem Block**, 1 × je Block im Leseloop, 1 × je geschriebenem Block, je Paketeintrag) | globale `auth`-Mutex; Direct: Grant-Suche, SHA-256-Fingerabdruck, HMAC, Kopie der Exportliste; **Raum: Geheimnis aus dem Geheimnisspeicher** (Linux/Android: Ordner öffnen, `flock`, Datei lesen, AEAD; Windows: `CredReadW`) | `fs_authority.rs:53-75`, `session.rs:229-301`, `profile_store.rs:343-355`, `creds/os/linux_file_store.rs:96-135`, `creds/os/windows.rs:61-67` |
| `TargetPolicy::new` | je `resolve`/`guard_target` und je `require_target_write` | `canonicalize` beider privater Wurzeln (App-Daten, Cache) | `fs_host_policy.rs:24-43`, `fs.rs:265-292` |
| `TargetPolicy::read` | je guarded Operation (stat, read_size, item_id, open_*, …) | `canonicalize` (bzw. des nächsten existierenden Elternordners), `is_dir`, `DirectoryHandle::open_root`, `is_within_any`: Linux steigt mit `..`-Öffnung + `fstat` je Ebene auf; Windows öffnet das Ziel, kanonisiert erneut, pinnt jeden Vorfahren und vergleicht Datei-IDs | `fs_host_policy.rs:64-91, 149-162`, `local_access/os/linux/private_ancestors.rs:12-40`, `local_access/os/windows/directory_handle.rs:76-95`, `local_access/os/windows/private_ancestors.rs:11-35` |
| `fs::resolve` | je Anfrage | Kopie der Exportkonfiguration; `secure_local_target` zweimal (Wurzel `canonicalize` ×2, Ziel `exists` + `canonicalize`) | `fs.rs:161-210`, `fs_local_paths.rs:12-48` |

**Schätzung je Host-Anfrage, Tiefe 8:**

| Host | Kosten je Anfrage | Folge für eine Datei mit 8 Anfragen |
|---|---|---|
| Linux | 60–150 Systemaufrufe ≈ 50–200 µs | – |
| Windows | 30–60 Handle-Öffnungen, `GetFinalPathNameByHandle` und ID-Abfragen, mit Defender-Filter **≈ 0,5–2 ms** | 4–16 ms Host-CPU |
| Android, geteilter Speicher | jeder Lookup über MediaProvider-FUSE, soweit nicht im Kernel-Cache; **≈ 1–5 ms** | 8–40 ms Host-CPU |

**Räume, Schätzung:** 3 Geheimnislesungen je 256 KiB × 30–200 µs unter einer globalen Sperre.
Das ergibt eine prozessweite Decke von etwa 0,4–2,5 GB/s je nach Plattform; unter Windows
eher am unteren Ende. Für kleine Dateien kommen ≈ 10 Lesungen je Datei hinzu.

### 4.6 Ordner

- **Einzeln:** `CreateDir` ist je Ebene 1 RT über den Proxy (`peer_transfer.rs:169-176`).
  Host: Steuerpool und die volle Pfadprüfung aus 4.5.
- **Sammelanlage (K-F, KP2):** spart RT. Die Hostkosten je Ordner bleiben ohne die
  Hostmaßnahme Z-H2 (Abschnitt 10).

---

## 5. Zulassung und Flusssteuerung (alle Stufen)

| Stufe | Grenze | Beleg | Wirkung |
|---|---|---|---|
| Engine-Flow | Start 2 (oder Hälfte des Gelernten), Deckel = Proxy-Slots | `transfer/core/flow_control.rs:32-57` (erster Durchgang) | Hochlauf in Fenstern von ≥ 100 ms; über Relay mit Fenstern von 1 s mehrere Sekunden |
| Proxy-Slots | min(Host-Deckel 56 + Browse-Reserve, 64) | `agent_proto/core/features.rs:80-86` | – |
| Proxy-Kredit | 1 MiB Start, 8 MiB Höchstfenster je Anfrage, 64 MiB je Verbindung | `agent_proto/core/credit.rs:21-36` | über Loopback nie bremsend |
| `PeerBackend::transfer_slots` | min(Host-Zulassung 60, 60) − 4 = 56 | `peer_transfer.rs:148-158` | – |
| QUIC-Ströme | 64 bidirektional | `keepalive.rs:9` | – |
| Host-Zulassung | 60 je Gerät, 256 je Host, je Übertragung 1 MiB Pufferbudget | `server_admission.rs` | `Busy` statt Warteschlange für v1-Clients |
| Host-Lesevorlauf | 2 Blöcke (512 KiB) je Strom | `server_transfer.rs:25` | HDD/USB-Quellen: wenig Vorlauf, das Betriebssystem-Readahead hilft |
| Host-Steuerpool | 32 für alle Clients | `blocking.rs:17` | Stat/FileIdentity/CreateDir/Promote aller Kleindateien aller Clients teilen sich 32 |

---

## 6. Android

**Speicherzugriff**

- Geteilter Speicher wird über Pfade unter MediaProvider-FUSE angesprochen
  (`MANAGE_EXTERNAL_STORAGE`, `android/app/src/main/AndroidManifest.xml:20-21`).
- **SAF und MediaStore werden für Übertragungen nicht genutzt.** SAF erscheint nur im eigenen
  `DocumentsProvider` für andere Apps (G4), MediaStore nur im Beobachter (siehe unten).

**Kosten je Datei auf FUSE (Schätzung; Plattformwissen, im Code nicht prüfbar)**

- Anlegen, Umbenennen und Löschen laufen über den MediaProvider-Dienst, der seine Datenbank
  fortschreibt.
- Lesen und Schreiben der Bytes ist mit FUSE-Passthrough billig (Android 12+ und
  Kernelunterstützung); ohne Passthrough geht jeder Block über den Dienst (zusätzliche Kopien).

**Veröffentlichen ohne Ersetzen** (`android_fs/os/rename.rs:1-22, 68-90`):

1. `renameat2(RENAME_NOREPLACE)` schlägt auf FUSE ohne `rename2` fehl;
2. `symlink_metadata`;
3. `hard_link` schlägt fehl (keine Hardlinks auf Android-Speicher);
4. Existenzprüfung;
5. `rename`.

Das sind bis zu fünf FUSE-Operationen je Datei. Der Fehlschlag wird nur protokolliert, nicht je
Datenträger gemerkt.

**Android als Client (Telefon fügt auf dem PC ein)**

- Die Engine läuft im App-Prozess mit `AgentBackend` über Loopback in denselben Prozess
  (`mobile/os/shared/pool.rs:204-207`). Der eingebettete Worker könnte direkt angesprochen
  werden, wie es `daemon/os/shared/mount_source.rs:51-53` für Laufwerke tut.
- Die Belege (4 DW) liegen auf internem Speicher (f2fs/ext4), nicht auf FUSE.
- Das Quell-`lstat`/`open` läuft über FUSE.

**Android als Host (PC holt vom Telefon):** Jede Anfrage durchläuft die Pfadprüfung aus 4.5
auf FUSE. Tokio-Threads = Kerne. Zusätzlich Energie- und Ruhemodus (`share/core/power.rs`),
nicht durchsatzbestimmend.

**MediaStore-Beobachter** (`android/app/src/main/java/app/smartexplorer/android/system/MediaStoreSync.kt:26-46`)

- Er beobachtet `MediaStore.Files` mit Nachfahren. Jede neu geschriebene Datei im geteilten
  Speicher löst nach 1 s Entprellung `probe()` und `BackgroundController.enqueueCatchUp(...)`
  aus.
- Während eines Downloads auf das Telefon können dadurch Sync-Nachholläufe parallel Bäume
  durchlaufen. Das konkurriert um CPU und FUSE-Durchsatz **(Wirkung ungemessen)**.

---

## 7. Weitere Einstiege

- **Explorer zieht entfernte Dateien (OS-Zwischenablage, Ziehen hinaus; Windows)**
  - Explorer liest Datei für Datei. se lädt im Voraus, parallel unter dem Flow: zwei
    Flow-Fenster (`virtual_clipboard/os/remote/prefetch.rs:20`), je Datei 16 MiB Vorlauf
    (`fetch.rs:21-28`), je Datei ein eigener `Read`-Strom über den Proxy, keine Pakete.
  - Decke: Explorers eigene Kosten je Datei (Anlegen, Schließen, Defender) und 1 RT je Datei
    im Vorlauf.
- **`se cp`/`se mv`/MCP**
  - Die Engine wird nur genutzt, wenn das Ziel fehlt (`cli/engine_copy.rs:97-109`), dann ohne
    Belege (`recoverable: false`).
  - Sonst (Zusammenführen in einen bestehenden Ordner, Überschreiben; MCP-Standard
    `folder_conflict: merge`, `cli/mcp/transfer_tools.rs:27-35`) läuft der ältere Weg
    **strikt nacheinander je Datei** (`cli/tree_apply.rs:22-150`) über den Proxy mit mehreren
    RT je Datei. Das ist über Share der langsamste Weg.
- **Räume:** nur lesend (`ARCHITEKTUR.md`, Zeile „Laufwerk speichern“: „Räume nur lesend“; Host lehnt Mutationen bei `may_write = false` ab, `server.rs:333-347`), also nur Download
  vom Raumgerät. Gleiche Kette wie Direct, plus Geheimnislesungen je Prüfung (4.5).

---

## 8. Zeitlinien und Vergleich mit dem Maßstab

### 8.1 Annahmen

**Strecken**

| Kürzel | Strecke | Annahme |
|---|---|---|
| L1 | 1-Gbit-LAN | RTT 0,3 ms |
| L10 | 10-Gbit-LAN | RTT 0,2 ms |
| W | WLAN | 400–800 Mbit/s, RTT 3–5 ms, Verluste |
| R | Relay | RTT 60 ms, Decke 64 MiB/s |

**Kosten (Schätzung)**

| Kostenart | Linux | Windows | Android |
|---|---|---|---|
| Proxy je Anfrage | 0,1–0,2 ms | 0,3–0,6 ms | 0,2–0,5 ms |
| Host je Anfrage | 0,05–0,2 ms | 0,5–2 ms | 1–5 ms (FUSE) |
| DW (Client-App-Daten) | 1–3 ms | 5–20 ms | 2–6 ms |

**Maßstab (allgemeine Erfahrungswerte; G5 misst auf CI)**

| Werkzeug | Wert |
|---|---|
| `iperf3`, ein TCP-Strom | L1 ≈ 940 Mbit/s, L10 ≈ 9,4 Gbit/s |
| `scp` große Datei, L1 | ≈ 110 MB/s |
| `rsync` 5000 × 4 KiB über SSH, LAN | ≈ 1–3 s |
| `scp -r` 5000 × 4 KiB, LAN | ≈ 5–15 s |

### 8.2 5000 × 4 KiB (20 MB), GUI-Einfügen lokal → Direct-Host

| Phase | Ablauf | Dauer (Schätzung) | Leerlauf |
|---|---|---|---|
| Verbindung (bereits offen vom Blättern) | sonst: iroh-Wahl, QUIC-Handshake 1 RTT (PQ-Gruppe → größerer ClientHello), Hello/Sitzungsnachweis 1 RTT, Pfadzulassung, `Capabilities` für Transfer- und Hostmerkmale (2 RT), Proxy-`OpenShare`/`HelloOk` | L1: 5–50 ms; R: 1–5 s | Netz und Datenträger |
| Erstes Byte | Walker liefert erste Datei; `CreateDir` des Zielordners; `WriteNew` + `Ready` | L1: ≈ 5 ms nach Start | – |
| Hochlauf | Flow 2 → Verdopplung je Fenster (≥ 100 ms) | 0,2–0,5 s; Decke ist schon bei ≈ 3–4 Workern erreicht | – |
| Gleichlauf | je Datei 8 RT + 4 DW; 3 DW unter der Auftragssperre: **≤ 1/(3 × DW)** | Linux ≈ 150–300 Dateien/s, Windows ≈ 15–70, Android ≈ 55–170 | Netz > 99 % leer (0,9 MB/s); Host-Datenträger fast leer; Client wartet auf fsync |
| Endphase | letzte `after()`-Phasen, Leerlaufende der Worker 250 ms, Fortschritt 150 ms | ≈ 0,3–0,5 s | – |
| **Gesamt** | | Linux ≈ 17–35 s, Windows ≈ 70–330 s, Android ≈ 30–90 s | – |
| Maßstab | `rsync` ≈ 1–3 s | **Verhältnis ≈ 1–15 %** | – |

**Mit Belegen v2 (K-A)** wäre die Sperre weg. Dann bestimmen 8 RT je Datei. Mit 56 Workern
ergibt das auf L1 grob 56 / (8 × 0,6 ms) ≈ 10 000 Dateien/s rechnerisch; hostseitig begrenzt der
Steuerpool (32) × Hostkosten: Linux ≈ 20 000/s, **Windows-Host ≈ 1600–6000 Anfragen/s
≈ 200–750 Dateien/s**, Android-Host noch weniger. Erst Pakete über den Proxy (Z-P1) und die
Hostmaßnahmen (Z-H1, Z-H2) heben das.

### 8.3 1000 × 1 MiB (1 GiB)

| Phase | Ablauf | Dauer (Schätzung) | Leerlauf |
|---|---|---|---|
| Erstes Byte, Hochlauf | wie oben | 0,2–0,5 s | Netz teilweise |
| Gleichlauf L1 | je Datei 8,4 ms Daten + ≈ 5 ms RT + DW; Sperrdecke Linux 150–300 MiB/s > 119 MiB/s | Linux: **netzgebunden ≈ 9–10 s (≈ 85–95 %)**; Windows-Client: Sperrdecke 15–70 MiB/s → 15–70 s (≈ 15–60 %) | Linux: Netz voll; Windows: Netz 40–85 % leer |
| Gleichlauf L10 | Sperrdecke Linux 150–300 MiB/s | ≈ 3,5–7 s statt 0,9 s (**≈ 13–25 %**) | Netz ≥ 75 % leer |
| Gleichlauf R | je Datei 8 RT × 60 ms ≈ 0,5 s seriell; 56 Worker → ≈ 110 Dateien/s > Relaydecke | relaygebunden ≈ 16–17 s + Hochlauf 3–6 s (Fenster 1 s) + erstes Byte 1–5 s | Anlauf |

### 8.4 1 × 10 GiB

| Phase | Ablauf | Dauer (Schätzung) |
|---|---|---|
| Erstes Byte | Upload: `SegmentedStageOpen` + `StageWriteAt`/`Ready` (2 RT); Download: `ReadAt` (1 RT) | L1 < 5 ms; R 1–5 s (Pfadzulassung) + 120 ms |
| Hochlauf | Helfer wachsen mit Flow-Erlaubnissen (2 → …); Teile 8 MiB (L1) bis 64 MiB (R) | L1 0,2–0,5 s; R 3–6 s |
| Gleichlauf L1 | Netz | ≈ 92–95 % von `iperf3` → ≈ 93–96 s |
| Gleichlauf L10 | eine QUIC-Verbindung (ein Treiber), Proxy (ein Socket, ≈ 7 Kopien je Byte), Host-Leser mit 512 KiB Vorlauf je Teil | **≈ 2–5 Gbit/s (20–50 %)** → 17–43 s statt ≈ 9 s |
| Gleichlauf R | Relaydecke 64 MiB/s, QUIC über TCP | ≈ 160–170 s + Anlauf |
| Endphase | letzter Teil (≤ eine Teillänge), `settle` (1-ms-Schleife), `SegmentedStageFinish`/Veröffentlichung (4–6 RT), Belege (4 DW) | L1 ≈ 0,1 s; R ≈ 1–2 s |

**Leerlauf:** Beim Download wartet je Teil der Host-Leser auf den Datenträger, während das Netz
wartet, nur mit 512 KiB Vorlauf. Bei USB/HDD-Hosts mit vielen parallelen Teilen auf einer Platte
entstehen Kopfsprünge; die Teile sind deshalb ≥ 8 MiB (`segments.rs:5-11`).

---

## 9. Korrekturen und Ergänzungen zum ersten Durchgang

1. **Pakete:** Der erste Durchgang beschreibt Pakete als Normalfall für lokal ↔ Share. Für alle
   Benutzereinstiege sind sie wegen des Proxys aus (Befund 2). Der Ausfall „Paketprüfung auf
   `/`“ (erster Durchgang, „Everything that turns packets off“) gilt nur für direkte
   `PeerBackend`-Läufe. Der Proxy ignoriert den Ordner ganz (`agent/core/backend.rs:218-224`)
   und kündigt nichts an.
2. **„RT“** ist auf allen Benutzereinstiegen ein Proxy-Durchgang mit Threadstart im Worker plus
   QUIC-Strom plus volle Hostprüfung, nicht nur ein Netz-Roundtrip.
3. **`file_identity_v1`** setzt jeder aktuelle Host (`share/core/wire_capabilities.rs:74`). Die
   FileIdentity-RTs fallen also an (offene Frage des ersten Durchgangs geklärt).
4. **Hostseitige Rechteprüfung** (`session.authorize`) ist nicht „nur Speicher“. Bei Räumen liest
   sie das Geheimnis aus dem Geheimnisspeicher, bei jedem Aufruf und unter der globalen Sperre.
   Sie läuft auch je Datenblock (4.5).
5. **Pfadprüfung des Hosts** kostet mehr als „2–3 `canonicalize`“: je guarded Operation kommt
   ein Aufstieg bis zur Wurzel hinzu, unter Windows mit gepinnten Vorfahren-Handles.
6. **`se cp` und Share:** Ja, Share-Ziele gehen über `resolve_endpoint` → Proxy
   (`connect/os/shared/resolution.rs:31-34`). Ein Vergleich CLI ↔ GUI auf Share ist auf CI
   möglich, aber nur bei fehlendem Ziel über die Engine.

---

## 10. Zielentwurf und konkrete Änderungen

**Zielbild:**

- Zwischen den Geräten fließt ein ununterbrochener Datenstrom; Metadaten und Veröffentlichung
  laufen gebündelt neben den Daten statt dazwischen.
- Auf jedem Gerät liegt genau eine Prozessgrenze zwischen Engine und Netz, besser keine.
- Die Hostseite prüft Recht und Pfad je Anfrage in O(1), bei unveränderter Strenge.
- Bei mehr als einem Kern pro Verbindungsdecke werden Verbindungen parallelisiert.

KP2-Abdeckung: **K-B** (Belege in Host-Antworten), **K-D** (Paketstufen ohne fsync, Elternordner
einmal je Paket), **K-F** (`CreateDirs`). „neu“ = von KP2 nicht abgedeckt.

### 10.1 Client und Proxy

| Nr | Änderung | Dateien | Erwarteter Gewinn (Schätzung) | Invarianten und wie sie gehalten werden | KP2 |
|---|---|---|---|---|---|
| Z-P1 | **Pakete über den Proxy einschalten.** `service_version` fragt die Paketfähigkeit unabhängig von `root_display()` ab (neue `PeerBackend`-Methode „Host kann Pakete“ = `host_batch_limits`, ohne den Container-Ausschluss). `AgentBackend::batch_limits(dir)` übernimmt die Container-Regel für Peer-Namensräume (kein Paket für `"/"` und die Verbindungswurzel), oder der Worker beantwortet sie je Anfrage. Die Pakete selbst bleiben Sache des Hosts. | `daemon/os/shared/backend_server.rs:65-76`, `share/core/peer_transfer.rs:115-132`, `agent/core/backend.rs:218-224` | Voraussetzung, damit K-B/K-D/K-E für Oberflächenwege wirken. Kleindateien: von 8 bzw. 4 RT je Datei auf 1 Paket-RT je bis 256 Dateien plus Beleg-RTs (die K-B entfernt) | Kein Protokollwechsel zum Host. Ältere Worker kündigen nichts an, dann gilt der heutige Einzelweg. Paketsemantik (kein Überschreiben, Behalten-beider, Nonce-Status bei unklarer Antwort) bleibt im `PeerBackend` | **neu** (macht K-B/K-D wirksam) |
| Z-P2 | **Engine dort ausführen, wo der Share-Knoten lebt.** Desktop: GUI, CLI und MCP schicken für Aufträge mit Share-Seite einen Auftrag (`TransferJob`) per IPC an den Worker; Rückkanal: Fortschritt, Probleme, Abbruch. Android: `mobile/os/shared/pool.rs` nutzt im selben Prozess den Worker-eigenen `ShareHost` direkt (wie `mount_source.rs:51-53`). | `app/core/transfer_route.rs`, `transfer/os/shared/lane.rs`, `daemon/os/shared/ipc*.rs` (neuer Auftragsdienst), `mobile/os/shared/pool.rs:204-207`, `cli/engine_copy.rs` | je Anfrage −0,1–0,6 ms und −1 Threadstart; je Byte −2 Kopien, −2 Kernelübergänge, −1 Socket-Serialisierung; ohne doppelte LZ4-Arbeit. 10 Gbit: hebt die Proxy-Decke; Telefone: weniger CPU je MB | Absturzfestigkeit: Belege liegen im selben App-Datenordner, der Worker schreibt sie. Ein GUI-Absturz bricht den Auftrag nicht mehr ab (stärker als heute). Abbruch über IPC; der Worker setzt `stop`. Wiederaufnahme unverändert (Belege, `.part`, Teilledger). Rechte: der Worker prüft dieselben lokalen Quellrechte (Access-Gate `engine/mod.rs:238-241`); nötig ist dieselbe Benutzeridentität von GUI und Worker (heute gegeben). Ältere Worker: Fähigkeitskennzeichen im IPC-Hello, sonst heutiger Proxyweg | **neu** |
| Z-P3 | Kleine Proxy-Verbesserungen, falls Z-P2 später kommt: Thread-Pool statt `spawn` je Anfrage; kein LZ4 auf der Loopback-Strecke; mehrere Proxy-Verbindungen je Backend ab hoher Rate | `backend_server.rs:124-226`, `agent/core/stream.rs:190-192, 336`, `daemon/os/shared/backend_ranges.rs:48-58` | −20–100 µs je Anfrage; −CPU | keine (rein lokal) | neu |
| Z-P4 | **Ein-RT-Veröffentlichung für Einzeldateien:** `WriteNew` ohne Warten auf `Ready` senden (Daten direkt hinterher, wie PutBatch); `WriteDone` trägt das Ziel und „ohne Ersetzen“; der Host veröffentlicht und antwortet mit Ergebnis und Belegen (Größe, Zeit, Identität, vorher vorhanden?). Alternativ: Einzeldatei als PutBatch mit einem Eintrag bis 16 MiB | `share/core/peer_request.rs:329-410`, `peer_writer.rs:139-184`, `server_transfer.rs:44-305`, `engine/publish.rs` | Einzelupload von 8 RT (mit K-B ≈ 4) auf 1 RT; über Relay ≈ 0,4 s weniger je Datei und Worker | Nur hinter einem neuen Fähigkeitskennzeichen; ältere Hosts behalten den heutigen Ablauf. Die Absicht muss vor `WriteDone` dauerhaft sein: die Engine schreibt `start_file` vor dem Senden von `WriteDone` (wie heute vor `put_batch`). Eine verlorene Antwort klärt ein Statusabruf wie `PutBatchStatus`, nie ein Neusenden. Kein Überschreiben: Host `promote_no_replace` mit Nummerierung | K-B deckt nur die Belege, **Zusammenlegung neu** |
| Z-P5 | **Rahmen ohne Zusatzkopien und Mehrfachsperren:** Kopf und Tag in einem `write`, Nutzlast als `Bytes` per `write_chunk`/`write_all_chunks`; Empfang über `read_chunk` statt `vec![0; n]` + `read_exact`; Pufferpool im Host-Leser statt `vec![0; CHUNK]` je Block | `share/core/framing.rs:74-154`, `peer_read.rs`, `server_read.rs:150`, `server_batch_get.rs:156` | −1 Kopie, −1 Nullsetzen, −2 Mutex-Sperren je Block auf beiden Seiten; 10 Gbit / Telefon: −15–30 % CPU je Byte (Schätzung) | Rahmenformat auf der Leitung unverändert; reine Implementierung | neu |
| Z-P6 | **Massenverbindungen:** ab gemessener CPU-Sättigung des Treibers oder bei Verlust bis 2–4 zusätzliche QUIC-Verbindungen je Gegenstelle nur für Übertragungen; Teile und Paketströme verteilt; Bereichsstufen auf ihrer Verbindung (die Stufentabelle hängt an der Verbindung, `peer_stage_ranges.rs:133-136`) | `share/core/node_sessions.rs`, `peer_transfer.rs`, `peer_stage_ranges.rs` | L10: ×1,5–2,5 bis Platten- oder Netzgrenze; WLAN/WAN mit Verlust: mehr Gesamtdurchsatz wie `iperf3 -P` | Jede Verbindung durchläuft Hello, Sitzungsnachweis und Pfadzulassung (Direkt zuerst, Relay nur als Rückfall). Die Gerätezulassung (60 je Gerät) bleibt je Gerät. Ältere Hosts: nur eine Verbindung (Fähigkeitskennzeichen) | neu |
| Z-P7 | MTU-Obergrenze 1472 für validierte IPv4-LAN-Pfade (bleibt 1452 für IPv6/sonst); GSO/USO-Zustand je Plattform in Diagnose melden | `share/core/keepalive.rs:127-146` (`MtuDiscoveryConfig`) | ≈ +1,4 % Nutzlast je Paket, weniger Pakete/s | Black-Hole-Erkennung von noq bleibt | neu |
| Z-P8 | **LZ4-Regel je Verbindung statt je Strom** (Messfenster und Verhältnis über Ströme teilen), Pakete komprimierbar machen, kein LZ4 bei lokalem Verbraucher-Rückstau | `share/core/framing/lz4_stream.rs:40-47`, `transfer/core/compress_policy.rs`, `agent/core/stream.rs` | Relay/WLAN, komprimierbare kleine Dateien: bis ×(1/Verhältnis) auf der Strecke | Pack-Kennzeichen `lz4_v1` bleiben; Blöcke stehen weiter einzeln | neu |

### 10.2 Host (alle Plattformen)

| Nr | Änderung | Dateien | Erwarteter Gewinn (Schätzung) | Invarianten und wie sie gehalten werden | KP2 |
|---|---|---|---|---|---|
| Z-H1 | **Rechteprüfung in O(1):** `ShareAuthState` erhält einen Revisionszähler, der bei jeder Änderung von Grants, Räumen, Exporten, `direct_online`, Geheimnissen oder Sharing-Zustand steigt. `AccessAuthority` merkt sich (Revision, `SessionAuthorization`) und rechnet `authorize_state` nur nach Revisionswechsel neu. Raumgeheimnis im Speicher des Auth-Zustands (einmal beim Laden bzw. Ändern), nicht je Prüfung aus dem Geheimnisspeicher | `share/core/fs_authority.rs:53-75`, `session.rs:150-301`, `types.rs` (`ShareAuthState`), `profile_store.rs:232-234` | Räume: von 3 Geheimnislesungen je 256 KiB und ≈ 10 je Datei auf atomare Vergleiche; Direct: −HMAC und −Exportkopie je Block; die globale `auth`-Sperre ist nicht mehr der Engpass | Autorisierung je Anfrage und je Block bleibt (Revisionsvergleich vor jeder Aktion). Ein Entzug erhöht die Revision unter derselben Sperre, unter der heute veröffentlicht wird; die nächste Prüfung rechnet neu und scheitert. Das Geheimnis liegt ohnehin bei jeder Prüfung kurz im Speicher; dauerhafte Haltung nur in `Zeroizing` und entfernt bei Raumaustritt | **neu** |
| Z-H2 | **Pfadauflösung mit gehaltenen Handles:** Exportwurzel einmal je Verbindung oder Revision öffnen und kanonisieren; private Wurzeln einmal je Prozess (`OnceLock`); Ziele relativ zum gehaltenen Elternhandle öffnen (Linux `openat2(RESOLVE_BENEATH\|RESOLVE_NO_SYMLINKS)` mit Rückfall auf `O_NOFOLLOW`-Kette; Windows handle-relativ `NtCreateFile` wie `local_access/os/windows/confined*.rs`); Aufstieg zur Wurzel nur noch, wenn sich das Elternhandle ändert. Je Paket/Ordner einmal | `share/os/shared/fs_host_policy.rs:24-162`, `fs_local_paths.rs:12-48`, `share/core/fs.rs:161-292`, `fs_guard_backend.rs:38-50` | Linux-Host: 60–150 Syscalls → ≈ 5–10 je Anfrage; Windows-Host: 0,5–2 ms → ≈ 0,05–0,2 ms; Android-FUSE: größter Gewinn (jeder Lookup sparen) | Einschluss in die Freigabe: handle-relativ ohne Symlinkfolgen ist strenger als heutiges `canonicalize`-dann-öffnen (kein TOCTOU). Private App-Wurzeln bleiben ausgeschlossen (Identitätsvergleich der gehaltenen Handles). Links bleiben Grenzen | K-D deckt „Elternordner einmal je Paket“ nur für PutBatch; **Rest neu** |
| Z-H3 | Paketstufen ohne fsync für Kopien | `share/core/local.rs` (Stufe), `server_batch_put.rs` | wie erster Durchgang D | wie dort | **K-D** |
| Z-H4 | Belege in Antworten (PutBatch/GetBatch/ReadAt/Read/Promote) | `server_batch_*.rs`, `server_read.rs`, `server_fs.rs` | −2–3 RT je Datei | wie erster Durchgang B | **K-B** |
| Z-H5 | `CreateDirs` | `peer_extensions/folder_reservation`, `server_fs.rs:241-252` | −1 RT je Ordner | wie erster Durchgang F | **K-F** |
| Z-H6 | **GetBatch-Rahmen bündeln:** bei kleinen Einträgen `Begin`+`Data`+`Done` als ein Rahmen (eine JSON-Kopfzeile mit Länge, Daten, Ergebnis im Folgerahmen oder im nächsten Kopf); `check_read` einmal je Eintrag statt je Ereignis; die Stat-Paare vorher/nachher über ein gehaltenes Handle (`fstat`) statt pfadbasiert | `server_batch_get.rs:57-180`, `peer_batch_get.rs:139-185` | Kleindatei-Paket: −2/3 der Rahmen, −2 `block_on` je Eintrag, −2 Pfadoperationen je Eintrag | Änderungserkennung (Größe+Zeit vorher/nachher) bleibt, über das Handle sogar genauer. Neues Format nur hinter Kennzeichen | neu |
| Z-H7 | Lesevorlauf je Strom an Medium und Rate anpassen (2 → bis 8 Blöcke bei Rotations- und USB-Medien bzw. hoher Rate), innerhalb des Zulassungsbudgets | `server_transfer.rs:25`, `server_admission.rs` (`TRANSFER_BUFFER_BYTES`) | HDD/USB-Hosts: weniger Leerlauf | Speicherbudget bleibt Obergrenze | neu |

### 10.3 Android

| Nr | Änderung | Dateien | Gewinn | Invarianten | KP2 |
|---|---|---|---|---|---|
| Z-A1 | = Z-P2 Android-Teil (kein Loopback im selben Prozess) | `mobile/os/shared/pool.rs` | siehe Z-P2 | siehe Z-P2 | neu |
| Z-A2 | Umbenennungsleiter merkt sich je Datenträger (`st_dev`) die fehlschlagenden Stufen und beginnt direkt mit der ersten tragfähigen | `android_fs/os/rename.rs` | −2–3 FUSE-Operationen je Veröffentlichung | Restrisiko „Prüfung, dann `rename`“ unverändert dokumentiert; kein Ersetzen nicht leerer Ordner | neu |
| Z-A3 | MediaStore-Hinweise während laufender Übertragungen in geteilten Speicher sammeln und nach Ende einmal melden (aufschieben, nicht verwerfen) | `android/.../system/MediaStoreSync.kt:26-46`, Ereignis „Übertragung aktiv“ aus der Engine | keine parallelen Nachholläufe während der Übertragung | Sync-Korrektheit: Nachholen findet garantiert nach Ende statt; das native Polling bleibt | neu |
| Z-A4 | Hostseite auf FUSE: Z-H2 | – | größter Einzelgewinn für Telefon als Host | – | neu |

### 10.4 Relay und Pfad

| Nr | Änderung | Gewinn | Invariante | KP2 |
|---|---|---|---|---|
| Z-R1 | Relay-Rate des eigenen Servers konfigurierbar (heute fest 64 MiB/s, `share-server/src/relay.rs:23-24`) | selbst betriebene Server mit mehr Bandbreite | Server bleibt Vermittler; Daten direkt zuerst | neu |
| Z-R2 | Produktentscheidung: Nutzdaten über Relay schon während der Direktsuche erlauben und auf LAN/Direkt wechseln, sobald validiert (die Politik wechselt heute schon im 25-ms-Takt nach oben) | erstes Byte bei reinem Relay: Sekunden → 1 RTT | „Relay nur als sichtbar gekennzeichneter Rückfall“ (`ARCHITEKTUR.md`, Kernprinzip) wäre berührt. **Entschieden 2026-10-28** (Nutzer: „entscheide selbst“): Relay als Startbrücke, danach nur Rückfall; umgesetzt in Paket D1 (`share/core/route_start.rs`, Zustand `relay_start`) | entschieden |

### 10.5 Einstiege

| Nr | Änderung | Gewinn | KP2 |
|---|---|---|---|
| Z-E1 | `se cp`/`se mv`/MCP mit Zusammenführen oder Überschreiben über die Engine (`FolderConflict::Merge` vorhanden) statt `cli/tree_apply.rs` (nacheinander) | Share: von 1 Datei gleichzeitig auf Flow-Nebenläufigkeit | neu (überschneidet G1) |
| Z-E2 | Explorer-Vorlauf (virtuelle Dateien) nutzt für Kleindateien GetBatch (nach Z-P1) | −1 RT je kleiner Datei im Vorlauf | neu |

### 10.6 Reihenfolge nach Wirkung (Schätzung)

1. K-A (Belege v2, G1/KP1);
2. **Z-P1** (Pakete über den Proxy) zusammen mit K-B/K-D/K-E;
3. **Z-H1** und **Z-H2** (Hostprüfungen);
4. **Z-P2** (Engine im Worker bzw. ohne Loopback auf Android);
5. Z-P4 (1-RT-Einzeldatei);
6. Z-P5 und Z-P6 (10 Gbit, Telefone);
7. Z-A2 und Z-A3;
8. Z-P8, Z-H6, Z-H7;
9. Z-R1 und Z-R2 (Entscheidung).

**Zielwerte nach 1–4 (Schätzung)**

| Arbeitslast | Zielwert |
|---|---|
| 5000 × 4 KiB, L1, Linux ↔ Linux | ≥ 3000–8000 Dateien/s (Host-Dateisystem und Paket-RT bestimmen; ≈ rsync) |
| 5000 × 4 KiB, Windows-Host | ≥ 1500 Dateien/s (Defender ungerechnet) |
| 1000 × 1 MiB | auf L1 netzgebunden, auf L10 ≥ 60 % |
| 10 GiB | auf L1 ≥ 95 %, auf L10 mit Z-P5/Z-P6 ≥ 70 % |

---

## 11. Was G5 für G2 messen soll

1. **Pakete über den Proxy:** GUI-gleicher Auftrag (`run_view` mit `daemon::open_share_backend`-Backend)
   gegen die Loopback-Fixture; Zähler für PutBatch/GetBatch/Stat/FileIdentity/CreateDir je
   Datei. Erwartung heute: 0 Pakete.
2. **Proxy-Kosten:** derselbe Auftrag mit nacktem `PeerBackend`
   (`CopyPastePeerFixture::for_transfer_engine_task`, erster Durchgang Abschnitt 6) gegen
   `AgentBackend`. Die Differenz in Dateien/s und CPU-Zeit je MiB ist der Proxyanteil.
3. **Hostprüfungen:** Zähler für `session.authorize` und `TargetPolicy::read` je Datei und je
   MiB (Testzähler); Linux `strace -f -c` des Hosts je 1000 Stat-Anfragen; Windows-Lauf mit und
   ohne Defender-Ausnahme.
4. **Raum gegen Direct:** gleicher Download; Unterschied = Geheimnislesungen.
5. **Verbindungsdecke:** 10 GiB über Loopback (`tc netem` 0/1/20 ms), `iperf3 -c 127.0.0.1` als
   Bezug (Loopback ist kein 10-GbE-Bezug, zeigt aber die CPU-Decke je Verbindung);
   `SE_SHARE_QUIC_CC=bbr3` als Variante.
6. **Relay:** lokaler share-server mit Relay, `relay_only`, 1 GiB; erstes Byte und Rate.
7. **Android (APK-Suite):** Host auf geteiltem Speicher gegen App-internen Speicher; Zeit je
   Veröffentlichung; ob `renameat2`/`link` scheitern (`android-fs.log`).

---

## 12. Gelesene Dateien

- **Transport:** `share/core/{keepalive, node (100-260, 400-440), node_sessions (100-320), node_routes (1-60, 184-194), route_policy (40-180), peer_stream, framing, lz4_stream, peer_read, peer_writer, peer_request, peer_transfer, peer_stage_ranges (1-400), peer_batch_put (60-240), peer_batch_get (25-204), peer_telemetry (13-40)}.rs`; `share/os/shared/route_monitor.rs (1-80)`.
- **Host:** `share/core/{server (1-100, 195-492), server_fs, server_fs_io, server_transfer, server_read, server_batch_get, server_admission, blocking (1-120), fs_access (44-188), fs_authority, fs_guard_backend (20-240, 440-487), fs_guard_stream (1-60), fs (155-292), session (150-330), batch_wire (1-80), wire_capabilities (13-100), fs_capabilities (46-60), server_stage_ranges (60-140), node_idle (150-175)}.rs`; `share/os/shared/{fs_host_policy (1-175), fs_local_paths (1-60), profile_store (232-257, 343-375)}.rs`.
- **Proxy:** `daemon/os/shared/{ipc_open_client (41-71), ipc (140-170, 270-300), ipc_host (360-386), backend_server (60-240), mount_source (40-60), backend_ranges (43-58)}.rs`; `agent/core/{backend (218-224, 456-467), mux (1-50), pool (1-40), stream (grep)}.rs`; `agent_proto/core/{credit (1-80), server (80-200), features (64-146), lz4_frames (116-200)}.rs`.
- **Engine:** `transfer/core/{segments (1-120), compress_policy (1-200)}.rs`; `transfer/os/shared/{memory (1-125), lane (1-60)}.rs`; `transfer/os/shared/engine/{segments, segments_download, upload, publish (1-140), download (115-200), mod (236-252)}.rs`.
- **Plattform:** `creds/os/{linux_file_store (1-135), windows (61-67)}.rs`; `local_access/os/linux/{directory_handle (59-94), private_ancestors (12-40)}.rs`; `local_access/os/windows/{directory_handle (76-95, 295-305), private_ancestors (11-36)}.rs`; `android_fs/{mod, os/rename (1-90)}.rs`; `keep_awake/os/shared/holds.rs (71-83)`; `virtual_clipboard/os/remote/{prefetch (1-60), fetch (1-40)}.rs`.
- **Einstiege:** `mobile/os/shared/pool.rs (190-215)`; `connect/os/shared/resolution.rs (20-45)`; `cli/{engine_copy (97-215), tree_apply (1-30), mcp/transfer_tools (22-60)}.rs`.
- **Fremdcode:** `vendor/iroh-1.3.0/src/{endpoint/quic.rs (90-200), socket.rs (110-146), socket/transports.rs (415-470), socket/transports/ip.rs (237-247), socket/transports/relay.rs (grep), relay/actor.rs (grep)}`; `vendor/noq-proto-1.3.0/src/config/transport.rs (427-611, 786-910)`; `vendor/noq-1.3.0/src/send_stream.rs (61-175, 286-326)`; `vendor/SMART_EXPLORER_ROUTE_POLICY.md (1-60)`; `share-server/src/{relay (1-35, 160-195), rate_limits (1-30)}.rs`.
- **Android:** `android/app/src/main/AndroidManifest.xml`; `android/.../system/MediaStoreSync.kt (1-60)`.
- **Dokumentation:** `docs/ARCHITEKTUR.md`.

## 13. Unsicher bzw. ungeprüft

- Alle Zeit-, Raten- und Syscall-Zahlen sind Schätzungen. Messung in G5.
- Befund 2 (keine Pakete über den Proxy) ist statisch aus `service_version` → `batch_limits("/")`
  abgeleitet. Nicht geprüft: ob ein späterer Aufruf die Ankündigung erneuert. Gelesen ist nur
  `OnceLock` je Proxyverbindung in `backend_server.rs:265`.
- Decke einer noq-Verbindung (CPU) auf Desktop und Telefon: Erfahrungswert aus
  quinn-Messungen, nicht für noq 1.3 geprüft.
- GSO/GRO auf Windows (USO/URO) und Android: nicht geprüft, ob `noq-udp` sie dort aktiviert.
- Ob der Raum-Geheimnisspeicher unter Windows je `CredReadW` messbar teuer ist
  (Größenordnung 10–200 µs angenommen).
- Android-FUSE: ob MediaProvider auf den Zielgeräten FUSE-Passthrough nutzt; Kosten von
  `lookup`/`create`/`rename` je Datei; ob `renameat2(RENAME_NOREPLACE)` dort fehlschlägt
  (laut Kommentar in `rename.rs` ja).
- Wie lange die Pfadzulassung bis zum Relay-Rückfall real wartet: Summe der
  NAT-Wiederholungsintervalle von iroh nicht nachgelesen.
- `source_media::probe` und die Bereichsstufen-Journale (`agent_proto/os/shared/range_stage.rs`,
  `journal` in `server_stage_ranges.rs:72`): Kosten je Teil nicht gelesen.
- Ob der Proxy-Lese-Thread bei vielen gleichzeitigen Downloads (eine Socket, ein Lese-Thread)
  vor 10 Gbit sättigt.
