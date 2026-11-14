# Bestandsaufnahme: Transport zwischen zwei se-Geräten

Stand: 2026-11-14. Bestandsteil des Plans `docs/plaene/2026-11-14-verbindungen-je-zweck/plan.md` (Abschnitt B „Offen (Bestandsauftrag)“). Das Dokument beschreibt den heutigen Stand und bewertet nichts; die Lösungsfrage (VPN je Zweck oder mehrere Verbindungen) behandelt `recherche.md` und die spätere Planung.

## Geltung, Quellen, Methode

- Lesestand: Arbeitsbaum des Repositorys auf Commit `6e5c6b11`. `git status --short` meldete zum Zeitpunkt der Prüfung 257 Einträge. Von den gelesenen Transportdateien unterscheiden sich `native/crates/engine/src/share/core/node.rs` (+6/−1 Zeilen), `native/crates/engine/src/share/core/node_sessions.rs` (+1 Zeile) und `native/crates/engine/src/share/core/session.rs` (+10 Zeilen) vom Commit; Weitere zitierte Dateien mit Abweichung vom Commit (`git status --short`). Geändert: `docs/ARCHITEKTUR.md`, `native/test-crates-task.py`, `vendor/SMART_EXPLORER_ROUTE_POLICY.md`, `vendor/iroh-1.3.0/src/endpoint.rs`, `vendor/iroh-1.3.0/src/endpoint/connection.rs`, `vendor/iroh-1.3.0/src/tls.rs`, `vendor/iroh-1.3.0/src/tls/verifier.rs`, `native/crates/engine/src/share/core/fs.rs`, `fs_request.rs`, `login_transport.rs`, `node_accept.rs`, `route_policy.rs`, `route_diagnostics.rs`, `service_routes.rs`, `watch_wire.rs`, `native/crates/engine/src/login/os/shared/gateway.rs`. Neu, nicht im Commit: `native/crates/engine/src/share/core/s4_messung_support.rs`, `s4_messung_task_tests.rs`, `native/crates/engine/src/rdp/os/linux_os/gnome_net.rs`, `vendor/iroh-1.3.0/src/endpoint/remote_key_gate.rs`, sowie die Verzeichnisse `docs/plaene/2026-11-08-offene-punkte/` und `docs/plaene/2026-11-14-verbindungen-je-zweck/`. Alle übrigen zitierten Dateien sind gegenüber dem Commit unverändert. Alle Zeilenangaben beziehen sich auf den Arbeitsbaum.
- Keine Ausführung: kein Build, kein Test, kein Commit, keine Suite. Jede Aussage stammt aus Lektüre des Quelltextes oder der Dokumente, mit Datei und Zeile.
- Belegarten: **[C]** Quelltext im Projekt (Datei:Zeile); **[V]** vendorierter Code unter `vendor/` (Datei:Zeile); **[D]** Dokument (Datei und Zeile oder Abschnitt); **[A]** Annahme, ausdrücklich markiert; **[U]** unbekannt, mit dem genau fehlenden Fakt.
- Pfadkonvention: Dateiangaben ohne Präfix beziehen sich auf `native/crates/engine/src/share/core/`, außer in Abschnitt 5 (dort auf `native/crates/engine/src/screen/core/`); ausdrücklich genannte Präfixe: `os/shared/…` = `native/crates/engine/src/share/os/shared/…`, `screen/…` = `native/crates/engine/src/screen/…`, `login/…` = `native/crates/engine/src/login/…`, `rdp/…` = `native/crates/engine/src/rdp/…`, `bisync/…` und `transfer/…` = `native/crates/transfer/src/…`; `vendor/…`, `docs/…`, `native/…` relativ zur Repository-Wurzel.
- Nicht gelesen wurden: alle Dateien, die in Abschnitt 9 als „nicht gelesen“ genannt sind.

## 0. Promptwissen und Bestandswissen

Promptwissen (Nutzerwortlaut 2026-11-14, aus dem Auftrag und aus `plan.md` Abschnitt A):
- Frage 1: ob zwischen zwei Direct-Shares (zwei se-Geräten) ein VPN aufgebaut werden kann, „nur zwischen den Programmen“.
- Frage 2: ob das Streaming und Dateitransfers verbessert.
- Vorgabe: bei „ein VPN für alles“ ungeeignet, mehrere Verbindungen je Zweck; Ziel: jede Komponente optimal.
- Nutzerannahme (`plan.md` Abschnitt A, zu prüfen): „datei transfer, streaming also bildübertragung und so passieren aktuell verschlüsselt“. Prüfergebnis in Abschnitt 2.8.

Bestandswissen: die Abschnitte 1 bis 7. Die Zweitplan-Aussagen aus `plan.md` Abschnitt B (Zeilen 30 bis 35) sind keine Bestandsaussagen dieses Dokuments; Abschnitt 8 nennt das Prüfergebnis je Aussage.

## 1. Komponenten, die Daten zwischen zwei se-Geräten übertragen

Vollständige Liste der im Quelltext gefundenen Übertragungswege zwischen zwei se-Geräten (Stand 2026-11-14):

1. Dateisystem-Anfragen der Freigabe (Lesen, Schreiben, Verzeichnislisten, Sync-Abgleich, Löschen, Umbenennen, Speicheranalyse, Dublettensuche, Mount-Leases, Schnappschuss, Beobachtung, Login-Ziel), Protokoll `share-fs/3`.
2. Übertragungsaufträge der Transfer-Engine auf Freigaben einschließlich der Bulk-Verbindungen (Teil von 1, Zeile 5).
3. Gerätefeed (Metadatenabo: Fensterfamilien, geteilte Dienste, RDP-Kurzstatus), Protokoll `share-screen/1`.
4. LAN-Statuskanal, Protokoll `paired-link/1`.
5. Bildschirmübertragung mit Eingabe, Protokoll `share-screen/1`.
6. TCP-Freigaben, Protokolle `share-tcp/2` und `share-tcp/1`.
7. RDP (läuft über den TCP-Tunnel aus Punkt 6).
8. Exec, Terminal, Protokoll `share-exec/2`.
9. Geräte-Befehle (Apps), Protokoll `share-apps/1`.
10. Räume (Sitzungsart `room` auf dem Dateisystem-Protokoll, kein eigenes Protokoll).
11. Direkte Beziehung und Reparatur (Fähigkeit `DIRECT_RECIPROCAL`, Steuerwert `DirectReciprocal`), auf dem Dateisystem-Protokoll.
12. Maschinenanmeldung, Protokoll `login/1`, eigener Endpunkt.

Nicht Gerät-zu-Gerät und deshalb nicht Teil dieser Liste: Share-Server (Signal-Verbindung, Relay, Adressermittlung über QUIC, `docs/ARCHITEKTUR.md` Zeilen 18 bis 22), SFTP, FTP/FTPS, SMB, WebDAV und Google Drive (externe Dienste), SSH-Befehle (`docs/ARCHITEKTUR.md` Zeile 121) und die lokale Loopback-IPC zwischen Oberfläche und Daemon (`docs/ARCHITEKTUR.md` Zeile 206). [U] Nicht geklärt: ob das Agent-Protokoll `agent_proto` über eine Share-Verbindung läuft; die Aufrufer wurden nicht geprüft.

Die Tabelle nennt je Komponente ALPN oder Kennung, Stromart, Stromanzahl, Rahmen und Puffer, Fluss- oder Pacing-Regeln und den Beleg. Alle Ströme sind QUIC-Ströme der Iroh-Endpunkte (`node.rs:157-168`). QUIC verlangt TLS 1.3 nach RFC 9001 [A: Protokollanforderung, im Quelltext nicht gelesen]; Abschnitt 2.8 prüft den Weg.

| Nr. | Komponente | ALPN / Kennung | Verbindung und Ströme | Rahmen und Puffer | Fluss- und Pacing-Regeln | Beleg |
|---|---|---|---|---|---|---|
| 1 | Dateisystem-Anfragen | `smart-explorer/share-fs/3` | Eine Anfrage öffnet einen neuen bidirektionalen Strom auf der Sitzungsverbindung. Dieselbe Verbindung trägt bis zu 60 Übertragungsströme (64 minus 4 Steuerströme). | Kopf 5 Byte (Länge `u32`, big endian, Tag); Steuerrahmen JSON bis 16 MiB; Datenrahmen bis `fs::CHUNK` + 1 = 256 KiB + 1 Byte; LZ4-Blöcke nur bei ausgehandeltem `lz4_v1` | Transferpuffer je Übertragung (2 + 2) × 256 KiB = 1 MiB; Gastgeber 256 Übertragungsplätze gesamt, 60 je Prinzipal; Wartefrist für Anfragen 60 s; Anfrage bei vollem Gastgeber als „busy“ | `native/crates/engine/src/share/core/node.rs:42`; `node_sessions.rs:143-181, 224-246, 235`; `framing.rs:12-30, 40-46, 104-119`; `fs.rs:19`; `keepalive.rs:9, 14-17`; `server_admission.rs:27, 34, 39`; `server_transfer.rs:25`; `io_deadline.rs:7`; `node_sessions.rs:345-346` (Kommentar) |
| 2 | Verzeichnislisten und Baumwalk | wie 1 | Teil von 1 | Listenstapel höchstens 2048 Einträge oder 512 KiB; Stromrahmen höchstens 4 MiB; Walk höchstens 1 000 000 Knoten, 128 MiB Namen, 512 Ebenen | Fähigkeit `fs_walk_batches_v1` | `list_batch_wire.rs:10-16`; `walk_assembly.rs:7-10`; `node_sessions.rs:349`; `fs_request.rs:67, 78, 231` |
| 3 | Sync-Abgleich (Hash-Walk) | wie 1 | Teil von 1 | Schlüsselpräfixe je Anfrage innerhalb des Anfragelimits 256 KiB; Stream mit Kanal-Gegendruck | Fähigkeiten `hash_walk_filter_v1` und `hash_walk_keys_v1`; Aufrufer im Abgleich `bisync/os/shared/snapshot_hash.rs:246` und `snapshot_local_keys.rs:11` (Kommentare) | `peer_hash_walk.rs:1-2, 22-23, 40-42`; `fs_request.rs:229`; `framing.rs:21` |
| 4 | Übertragung (Transfer-Engine) | wie 1 | Teil von 1 (Lesen `Read`, `ReadAt`, Schreiben `Write`, `WriteNew`, Stapel `PutBatch`, `GetBatch`, Stufen `SegmentedStage*`, `CopyFile`, `RemoveTree`, `FinishStage`, `PromoteStaged`) | wie 1 | Flow höchstens 56 der 60 Plätze, 4 bleiben fürs Blättern; Fenster 16 MiB je Strom; Verbindungsfenster 16 bis 64 MiB je nach Speicherbudget | `fs_request.rs:85, 91, 157, 171, 180, 151`; `docs/TRANSFER_ENGINE.md:175` [D]; `transfer/core/flow_control.rs:29` (Kommentar: alle Ströme einer Verbindung teilen ein Staufenster) |
| 5 | Bulk-Verbindungen für Übertragungen | wie 1 (über `connect_session_until`) | Bis zu 3 zusätzliche Verbindungen je Gegenstelle, nur für Übertragungsströme | wie 1 | Hintergrundaufbau, Frist 20 s; kein Ausbau über das Relay; ein Upload mit Bereichen bleibt auf der ersten Verbindung; Gastgeber braucht Fähigkeit `bulk_connections_v1` | `node_bulk.rs:1-21` (Kopf), `33` (`MAX_EXTRA = 3`), `46` (`BUILD_BUDGET` 20 s), `node_sessions.rs:388` |
| 6 | Gerätefeed (Statuskanal, Metadaten) | `smart-explorer/share-screen/1` (Verbindung über `ScreenLink`) | Eine Metadatensitzung je Gegenstelle mit bestehender Beziehung; Abo über `send_subscribe_feed` | Rahmengröße [U]: `screen/core/wire_control.rs` nicht gelesen; Abo zu groß für den Statuskanal, dann Einzelabfrage | Keine eigene Zeit- oder Pacing-Regel im Feed-Modul (`peer_feed.rs` „No timer of its own“, Zeilen 5 bis 6) | `os/shared/peer_feed.rs:1-9, 52`; `os/shared/peer_feed_link.rs:1-3, 183`; `node_screen_link.rs:66-82` |
| 7 | LAN-Statuskanal | `smart-explorer/paired-link/1` | Verbindung über den Haupt-Endpunkt, ein bidirektionaler Strom je Runde (`open_bi` beim Aufrufer, `accept_bi` beim Empfänger) | Rahmen höchstens 4096 Byte; Runde höchstens 3 s; Sitzung höchstens 45 s; Erneuerung alle 2 s; Fakten gelten 6 s; Probe-Abstand 5 s; höchstens 128 Schnittstellen | Gegenseitige Herausforderung je Runde; die Fakten geben keine Dateisystem-Rechte | `lan_link_wire.rs:7-8`; `os/shared/lan_link_dial.rs:87`; `os/shared/lan_link_exchange.rs:1, 98, 188, 280, 289`; `os/shared/lan_link_transport.rs:28-34`; `lan_link_facts.rs:1` |
| 8 | Bildschirmübertragung mit Eingabe | `smart-explorer/share-screen/1` | Je Betrachter eine Verbindung. Ein bidirektionaler Steuerstrom (vom Gastgeber geöffnet, trägt Hallo und Steuerrahmen). Bildgruppen als QUIC-unidirektionale Ströme vom Gastgeber, der Betrachter nimmt höchstens 4 an. | Gruppenkopf 20 Byte (`SCRV`); Rahmenkopf 21 Byte; Datensatz höchstens 16 MiB; Kachel-Datensatz höchstens 4 MiB | Warteschlange je Ziel 1 MiB, Relay-Schutz 768 KiB; Warteschlangenzeit 100 ms; Strom-Reset bei ältestem unbestätigtem Bild älter als 1 s; höchstens 1024 Bilder in Flug; höchstens 240 Bilder je Sekunde; Bitrate nach Verzögerung und Verlust (Abschnitt 5) | `screen/core/wire.rs:18-43`; `share/core/node_screen.rs:274` (öffnen); `screen/core/gop_stream.rs:1-2, 24, 27, 30, 91, 270`; `screen/core/wire_binary.rs:1-9, 20-25`; `screen/core/pacing.rs:14-27`; `screen/core/rate_control.rs:1-26` |
| 9 | Exec, Terminal | `smart-explorer/share-exec/2` | Verbindung je Ausführungsanforderung über `connect(addr, EXEC_ALPN)` [U: Anzahl der Verbindungen je Ausführung nicht gezählt]; Handshake auf dem ersten bidirektionalen Strom, den der Gastgeber öffnet | Kopf 5 Byte; Handshake höchstens 64 KiB; Größenänderung (Resize) höchstens 256 Byte; Eingabewarteschlange 16; Handshake-Frist 20 s | Frühe Herausforderung vom Gastgeber (Kommentar); Auftragsbetrieb `serve_job` [U: Stromzuordnung nach dem Handshake nicht gelesen] | `exec_protocol.rs:12-39, 105, 134`; `node_routes.rs:61`; `exec_admission.rs:37, 44-52`; `exec_server.rs:28, 30, 132`; `exec_client.rs:181-186` |
| 10 | Geräte-Befehle (Apps) | `smart-explorer/share-apps/1` | Verbindung mit 30 s Frist (`node_apps_link.rs:46`); ein bidirektionaler Strom je Befehl (`node_apps_client.rs:32`) | Paket höchstens 2 MiB minus 16 KiB; Hallo höchstens 16 KiB | Bestätigungen wartender Befehle (Kommentar `node_apps_client.rs:1`) | `apps_wire.rs:10-15`; `node_apps_link.rs:1, 46, 54`; `node_apps_client.rs:1, 32` |
| 11 | TCP-Freigaben | `smart-explorer/share-tcp/2` und `smart-explorer/share-tcp/1` | Je Tunnel eine eigene QUIC-Verbindung. Je lokaler TCP-Verbindung ein bidirektionaler Strom, höchstens 16 je Tunnel, höchstens 64 Tunnel je Gerät | Steuer-JSON höchstens 32 KiB; Kopierpuffer 8 KiB je Richtung; Lebensdauer V1 höchstens 24 h | Zwei Kopierrichtungen; EOF schließt nur die Gegenrichtung; Leerlauf-Budget | `tcp_forward.rs:12-18, 30-32`; `tcp_wire.rs:9, 91, 103`; `node_tcp_link.rs:85-89, 152, 359`; `node_tcp.rs:103, 274, 288`; `tcp_registry.rs:447`; `os/shared/tcp_pump.rs:1-3, 46-70, 89` |
| 12 | RDP | Teil von 11 | Teil von 11: das RDP-Modul nutzt die Tunnel-Typen der TCP-Freigaben | Teil von 11 | Teil von 11 | `rdp/core/admission.rs:4, 8`; `rdp/mod.rs:30`; `rdp/os/linux_os/gnome_net.rs:2` [U: RDP-Dienstport und RDP-Protokoll nicht gelesen] |
| 13 | Räume | Kein eigenes Protokoll; Sitzungsart `room` auf `share-fs/3` | Teil von 1 | Mitgliedergrenzen: 32 wartende, 1024 zugelassene Mitglieder je Raum | Schlüssel `room:<Raum-ID>:<Node-ID>` | `session.rs:310-315`; `node_sessions.rs:388`; `room_relation_members.rs:263, 269` |
| 14 | Direkte Beziehung und Reparatur | `share-fs/3`, Fähigkeit `DIRECT_RECIPROCAL` | Ein bidirektionaler Strom je Reparatur, höchstens 4 gleichzeitige Reparaturen | [U: Rahmen des Reparaturstroms nicht gelesen] | Reparaturen gleichzeitig höchstens 4 | `node_sessions.rs:353-355`; `node.rs:44`; `direct_reciprocal_transport.rs:113` (Strom öffnen); `wire.rs:354` (Steuerenum `Ctrl`) |
| 15 | Maschinenanmeldung | `smart-explorer/login/1` | Eigener Endpunkt `bind_machine` mit eigener Transportkonfiguration; Verbindung über `connect(…, LOGIN_ALPN)` | Steuerrahmen höchstens 64 KiB | Pfadfreigabe aktiv (`application_path_policy(true)`); Fenster und Staukontrolle wie der Haupt-Endpunkt; Leerlauf 20 s (`IROH_CONNECTION_IDLE_TIMEOUT`) | `login/core/gateway_types.rs:8`; `login/core/model.rs:6`; `login_transport.rs:104, 116, 122-135, 213`; `login/os/shared/gateway.rs:247`; `login/os/shared/gateway_client.rs:84` |
| 16 | Beobachtung (Watch) | Teil von 1 | Eigene Stromlebensdauer, Abbruch über den Stromabbau | Höchstens 64 Pfade je Mitteilung | Überlauf für volle Verbraucher | `peer_watch.rs:1, 45`; `fs_request.rs:239`; `watch_wire.rs:9` |
| 17 | Speicherschnappschuss | Teil von 1 | Ein Strom je Anfrage (`open_stream`) | Gesamtgröße höchstens 64 MiB; Datenrahmen höchstens `crate::agent_proto::CHUNK` + 1 [U: Wert von `agent_proto::CHUNK` nicht gelesen] | Keine eigene Regel gelesen | `peer_storage_snapshot.rs:18, 51`; `storage_snapshot.rs:17` |
| 18 | Mount-Leases und Freigabe-Abbau | Teil von 1 | Ein Strom je Anfrage (`open_stream_until`); höchstens 4 gleichzeitige Freigaben | Leasename höchstens 128 Zeichen | Keine eigene Regel gelesen | `peer_lease_release.rs:10, 53`; `mount_lease_client.rs:7` |

## 2. Verbindungsaufbau und Sitzung

### 2.1 Endpunkt

Ein Iroh-Endpunkt je Prozess, angelegt beim Start des Knotens (`native/crates/engine/src/share/core/node.rs:145-203`):
- Tokio-Laufzeit mit mehreren Threads, Name `share-iroh` (`node.rs:145-151`).
- Relay-Optionen aus der Konfiguration des Share-Servers (`node.rs:152`).
- Endpunkt-Aufbau mit `presets::Minimal` und dem Kryptoanbieter `pq_kx::share_crypto_provider()` (`node.rs:157-158`). Der Kommentar nennt die hybride Gruppe `X25519MLKEM768` (`node.rs:155-156`) [C].
- Sieben ALPNs (`node.rs:160-168`): `share-fs/3`, `share-exec/2`, `share-tcp/2`, `share-tcp/1`, `share-apps/1`, `paired-link/1`, `share-screen/1`.
- Relay-Modus (`node.rs:169`), Pfadauswahl `RouteSelector` (`node.rs:170`), Transportkonfiguration mit Pfadpolitik (`node.rs:171`, Abschnitt 3), Schlüssel-Sperrtor vor der Anmeldung (`node.rs:172`).
- Nur-Relay-Schalter: Umgebungsvariable `SE_SHARE_RELAY_ONLY` (`native/crates/engine/src/share/os/shared/transport_options.rs:18, 63-66`); bei Wahr entfernt der Endpunkt die IP-Transporte (`node.rs:181-183`).
- Ausgehende Sitzungen: `sessions` als `HashMap` (`node.rs:198`). Eingehende Sitzungen: `incoming_sessions` (`node.rs:205-206`, Lesezugriff `node_idle.rs:226`).

### 2.2 Ausgehende Sitzung je Schlüssel

- Schlüssel: `"{direct|room}:{Beziehungs-ID}:{Node-ID}"` (`native/crates/engine/src/share/core/session.rs:310-320`). Eine Gegenstelle hat also je Beziehung und je Raum eine eigene Sitzung.
- Wiederverwendung: eine geöffnete Verbindung wird für alle Anfragen desselben Schlüssels benutzt (`node_sessions.rs:293-308`). Eine geschlossene Verbindung wird entfernt und neu aufgebaut (`node_sessions.rs:306`).
- Einzelflug: gleichzeitige Aufbauten desselben Schlüssels laufen über eine gemeinsame Sperre (`node_sessions.rs:260-279`).
- Ablehnungssperre: vor jedem neuen Handshake (`peer_denials::guarded`, `node_sessions.rs:286-288`).
- Die Richtlinien-Generation wird an die Verbindung gebunden (`node_sessions.rs:310-329`, Bindung in Zeile 320). Schlägt sie fehl, wird die Verbindung mit Grund `authorization changed during handshake` geschlossen (Zeilen 321-325).

### 2.3 Aufbau einer Sitzung

`connect_session_until` (`node_sessions.rs:331-427`):
1. Prüfung der erwarteten Node-ID gegen die gepinnte Identität (`node_sessions.rs:337-341`).
2. Adresse aus Verbindungsdaten und lokalen Adressen (`node_sessions.rs:342-343`).
3. Beziehungsart `direct` oder `room` (`node_sessions.rs:344`, Funktion `relation_kind_id` in `session.rs:310-315`).
4. Angeforderte Fähigkeiten: `fs`, `fs_walk_batches_v1`, `TRANSFER_V1`, `route_policy_v1`, bei `direct` zusätzlich `DIRECT_RECIPROCAL` (`node_sessions.rs:347-355`).
5. Nonce (12 Zeichen) und HMAC-Nachweis mit dem Beziehungsgeheimnis (`node_sessions.rs:358-368`).
6. Hallo mit Protokollversion `3` (`node_sessions.rs:369-379`).
7. Verbindung auf `share-fs/3` (`node_sessions.rs:388`), Warten auf die Pfadfreigabe (`node_sessions.rs:391`), bidirektionaler Steuerstrom (`node_sessions.rs:393-394`), Hallo senden (`node_sessions.rs:397`).
8. Antwort `PeerHelloOk` oder `PeerHelloOkRoute` (`node_sessions.rs:399-400`), Protokollmeldung (`node_sessions.rs:405-416`).

### 2.4 Aufrufer

- `open_stream` und `open_stream_until` (`node_sessions.rs:143-181`) liefern einen Strom je Anfrage. Die Generation wird vorher geprüft (`node_sessions.rs:165-167`).
- Schlägt das Öffnen des Stroms fehl, wird die physische Verbindung ersetzt und die Anfrage einmal wiederholt, weil noch keine Nutzlast gesendet wurde (`node_sessions.rs:172-179`).
- Frist je Vorgang: `PEER_OP_TIMEOUT` 60 s (`io_deadline.rs:7`).
- Die Suche `rg` zählte 25 Aufrufstellen von `open_stream` oder `open_stream_until` (ohne Testdateien). Acht davon sind: `backend.rs:113`, `peer_walk.rs:24`, `peer_hash_walk.rs:128`, `peer_watch.rs:45`, `peer_request.rs:109`, `peer_request.rs:225`, `peer_storage_snapshot.rs:51`, `peer_lease_release.rs:53`. [U: Die übrigen 17 Aufrufstellen sind nicht einzeln aufgelistet und nicht gelesen.]

### 2.5 Zweite gleichzeitige Verbindung zur selben Gegenstelle

| Fall | Verhalten | Beleg |
|---|---|---|
| Zwei Anfragen mit demselben Schlüssel (gleiche Beziehung, gleiche Node-ID) | Wiederverwendung der einen Verbindung, kein zweiter Handshake | `node_sessions.rs:293-308` |
| Gleichzeitiger Aufbau mit demselben Schlüssel | Einzelflug: der zweite Aufrufer wartet | `node_sessions.rs:260-279` |
| Beziehung oder Raum mit abweichender ID zur selben Node-ID | Eigene Sitzung, eigene Verbindung | `session.rs:317-320` |
| Gerätefeed, LAN-Statuskanal, Bildschirm, Apps, Exec, TCP-Tunnel, Maschinenanmeldung | Je eine eigene Verbindung auf dem jeweiligen ALPN. Die Maschinenanmeldung läuft auf einem eigenen Endpunkt. | `node_screen_link.rs:82`; `os/shared/lan_link_dial.rs:87`; `node_apps_link.rs:46`; `node_routes.rs:61`; `node_tcp_link.rs:85-89`; `login_transport.rs:104, 213` |
| Bulk-Verbindungen für Übertragungen | Bis zu 3 zusätzliche Dateisystem-Verbindungen; nur bei Verlust oder hoher Rate (Abschnitt 4.5) | `node_bulk.rs:33, 70-88` |
| Obergrenze für die Summe aller Verbindungen je Gegenstelle über alle ALPNs | [U] Die Suche nach Begrenzungen (`MAX_`, `LIMIT`, `max_concurrent`) fand keine solche Obergrenze. Eine vollständige Prüfung aller Verbindungsaufbauten fehlt. | — |

### 2.6 Annahmegrenzen vor der Anmeldung

Eingehende Verbindungen durchlaufen (`native/crates/engine/src/share/core/node_accept.rs`, Zeilen 25 bis 60):
- Verbindungsplätze für den TLS-Handshake: Kapazität 64 (`node.rs:43` `MAX_PENDING_APPLICATION_HANDSHAKES`; `node.rs:213-215`). Reserve 16 (Viertel der Kapazität, `handshake_limits_tls.rs:96`), je Quelle höchstens 16 (`handshake_limits_tls.rs:97`, Prüfung in Zeile 125), Wiederholung über QUIC-Retry ab 32 belegten Plätzen für nicht validierte, fälschbare Absender (`handshake_limits_tls.rs:98`, Prüfung in Zeile 113).
- Nicht gepinnte Absender nutzen höchstens `Kapazität − Reserve` = 48 Plätze; gepinnte Relay-Absender dürfen alle 64 belegen (`handshake_limits_tls.rs:116-120`).
- Fristen des Handshakes vor der Identität 3 s, danach 20 s (`node_accept.rs:232-233`).
- Entzogene Relay-Absender erhalten keinen TLS-Handshake (`node_accept.rs`, Kommentar E17, Zeilen 25 bis 60).
- Strafkonto für Exec-, Bildschirm-, TCP- und Apps-Verbindungen (`node_accept.rs:86-88`); Buchung `NoAuthentication` (`node_accept.rs:161-172`). Werte in `native/crates/engine/src/share/core/auth_penalty.rs:17-21`: Fehler 5 s, keine Anmeldung 1 s, Mindeststrafe 15 s, Höchststrafe 600 s, höchstens 65 536 Quellen.
- Ablehnungssperre je Schlüssel (`native/crates/engine/src/share/core/peer_denials.rs:26-41`): höchstens 64 Einträge, höchstens 16 nicht gepinnte, fünf Pausenstufen, Vergessen nach 15 Minuten.
- Gastgeber-Plätze für Übertragungen: 256 gesamt (`server_admission.rs:27`), 60 je Prinzipal (`server_admission.rs:34`), Wartezeit bei Speicherdruck 100 ms (`server_admission.rs:43`), Wartezeit für alte Clients 45 s (`server_admission.rs:49`).

### 2.7 Leerlauf-Aufräumen und Energiemodus

- Leerlauf-Aufräumen läuft nur im Energiesparbetrieb, wenn die CPU ohnehin wach ist, und schließt Verbindungen, deren Stromaktivität seit mindestens 2 Minuten unverändert ist (`native/crates/engine/src/share/core/node_idle.rs:1-8`, Ruhezeit `IDLE_QUIET_MS` 120 000 ms in Zeile 26). Schließcode `0x4944`, Grund `idle` (`node_idle.rs:23-24`). Verbindungen mit offenen Strömen oder Leasen werden nicht geschlossen (`node_idle.rs:158-160`).
- Jede offene QUIC-Verbindung sendet alle 5 s einen Keepalive und weckt damit das Gerät (`node_idle.rs:4-5`; Keepalive in `keepalive.rs:6`).
- Energiemodus: Haltezeiten 5 s (Keepalive), 15 s (Verbindungsaufbau), 15 s (Signalaktivität) und 60 s (Strom) in `power.rs:25-32`; Drosselung der Haltezeit 5 s und Obergrenze 60 s in `power_hub.rs:11-14`; Probe-Wartezeit 12 s in `power_probe.rs:9`; Umschalten über `set_low_power` (`power.rs:50-51`).

### 2.8 Nutzerannahme zur Verschlüsselung

Prüfung: Alle Ströme zwischen se-Geräten laufen über Iroh-Endpunkte, deren Verbindungen TLS verwenden (`node.rs:157-159`, Endpunkt mit dem Kryptoanbieter der Share-Schicht). Der Endpunkt baut keine unverschlüsselten Verbindungen zu Gegenstellen auf; im Quelltext wurde kein Klartextkanal zwischen Geräten gefunden [A: Suche nach Klartextkanälen war eine Suche nach Verbindungsaufbau und Kryptoanbieter, keine vollständige Prüfung aller Module]. Ausnahmen, die nicht zwischen zwei Geräten liegen:
- Lokale Schleife: der TCP-Listener am Betrachter bindet nur an Loopback (`node_tcp_link.rs:1-2`); die Daemon-IPC läuft über Loopback-TCP (`docs/ARCHITEKTUR.md` Zeile 206).
- Relay-Server: ein Klartext-Rückfall auf dem TLS-Port des Relays (`vendor/iroh-relay-1.0.0/src/server/http_server.rs:1171`, Patch-Marke). Das betrifft die Verbindung zum Relay, nicht den Weg zwischen zwei Geräten; die Klartext-Regel für die Maschinenanmeldung nennt `docs/ARCHITEKTUR.md` Zeile 36.

Die Frage, ob die Verschlüsselung Zeit kostet, ist nicht Teil dieses Bestands; Messwerte fehlen (Abschnitt 7).

## 3. Transporteinstellungen

Die Projektwerte stehen in `native/crates/engine/src/share/core/keepalive.rs`, Funktion `iroh_transport_config_for_routes` (Zeilen 127 bis 146). Sie gelten für den Haupt-Endpunkt (`node.rs:171`) und den Mess-Endpunkt (`s4_messung_support.rs:195-200`). Der Anmelde-Endpunkt setzt eigene Werte (`login_transport.rs:122-130`).

| Einstellung | Projektwert und Stelle | Standardwert in noq-proto 1.3.0 (Stelle) | Wirkung |
|---|---|---|---|
| Keepalive der Verbindung | 5 s (`keepalive.rs:6, 138`) | `keep_alive_interval: None` (`vendor/noq-proto-1.3.0/src/config/transport.rs:591`) | Verbindung bleibt offen; weckt Gerät (Abschnitt 2.7) |
| Keepalive des Pfades | 5 s (`keepalive.rs:7, 139`) | `default_path_keep_alive_interval: None` (`transport.rs:608`) | Pfad bleibt aktiv |
| Leerlauf-Zeitüberschreitung | 20 s (`keepalive.rs:8, 135-137`) | 30 s (`transport.rs:572`) | Verbindung endet ohne Keepalive |
| Bidirektionale Ströme | 64 (`keepalive.rs:9, 140`); 4 bleiben fürs Blättern (`keepalive.rs:14`); 60 für Übertragungen (`keepalive.rs:16-17`) | 100 (`transport.rs:569`) | Grenze der Ströme je Verbindung |
| Unidirektionale Ströme | **0** (`keepalive.rs:10, 141`) | 100 (`transport.rs:570`) | Die Gegenstelle darf keine unidirektionalen Ströme öffnen; siehe Abschnitt 5.4 und 9 (U1) |
| Stromfenster | 16 MiB (`keepalive.rs:22, 142`); bei kleinem Budget wie das Verbindungsfenster begrenzt (`keepalive.rs:92`) | `STREAM_RWND` (`transport.rs:573`) | Bytes in Flug je Strom |
| Verbindungsfenster | Klammer aus Budget/4 zwischen 16 MiB und 64 MiB (`keepalive.rs:90-93, 143`); Budget aus `crate::transfer::memory_budget()` (`keepalive.rs:128`) | `VarInt::MAX` (`transport.rs:574`) | Bytes in Flug je Verbindung |
| Sendefenster | gleich dem Verbindungsfenster (`keepalive.rs:98, 144`) | `8 × STREAM_RWND` (`transport.rs:575`) | Ungeackerte Daten je Verbindung |
| Staukontrolle | Cubic (`keepalive.rs:111-116, 131`); BBRv3 nur bei Umgebungsvariable `SE_SHARE_QUIC_CC=bbr3` (`keepalive.rs:105-121`) | Cubic (`transport.rs:599`) | Pro Verbindung eigener Regler (`node_bulk.rs:5`) |
| Anfangs-MTU | nicht gesetzt | 1200 Byte (`vendor/noq-proto-1.3.0/src/lib.rs:360`; `transport.rs:583-584`) | Start der Pfadgröße |
| Obergrenze der MTU-Suche | nicht gesetzt | 1452 Byte (`transport.rs:872, 904`); harte Grenze 65 527 Byte (`lib.rs:361`; Klammer `transport.rs:878`) | Obergrenze der Suche |
| Datagramm-Empfangspuffer | nicht gesetzt | `Some(STREAM_RWND)` (`transport.rs:594`) | Grenze für eingehende Datagramme |
| Datagramm-Sendepuffer | nicht gesetzt | 1 MiB (`transport.rs:595`) | Grenze für ausgehende Datagramme |
| Datagramme in Produktion | nicht benutzt: `send_datagram` und `read_datagram` kommen außerhalb der Tests nicht vor (rg über `native/crates`) | — | Die Datagramm-Pfade sind gepatcht (`vendor/noq-proto-1.3.0/src/connection/datagrams.rs:1`), das Projekt benutzt sie nicht |
| Segmentierungs-Offload (GSO) | nicht gesetzt | `enable_segmentation_offload: true` (`transport.rs:601`) | Standard aktiv |
| GRO (Empfangs-Offload) | nicht gesetzt | [U] Die Stelle, an der noq GRO aktiviert, wurde nicht gesucht. | — |
| Spin-Bit | nicht gesetzt | `allow_spin: true` (`transport.rs:593`) | Standard |
| Pfadpolitik der Anwendung | `application_path_policy(enabled)` mit `enabled = true` für den Haupt-Endpunkt (`keepalive.rs:130`; `node.rs:171`) | `application_path_policy: false` (`transport.rs:577`) | Anwendungsdaten nur über den zugelassenen Pfad (Abschnitt 4) |
| Ratenbegrenzung je Verbindung | nicht gesetzt | `max_outgoing_bytes_per_second: None` (`transport.rs:588`) | Keine Ratenbegrenzung über noq |
| Verschlüsselungspuffer | nicht gesetzt | 16 KiB (`transport.rs:592`) | Standard |

### 3.1 Vendorierte Pakete und ihre Änderungen

Die Pakete liegen unter `vendor/` (Quellbasis laut `vendor/SMART_EXPLORER_ROUTE_POLICY.md` Zeilen 3 bis 5: Iroh 1.3.0 Commit `0072d7d8…`; noq und noq-proto 1.3.0 Commit `c1f41156…`; die Pakete wurden aus dem Cargo-Cache kopiert). Die Projektänderungen sind an der Kopfzeile „Smart Explorer … patch“ erkennbar. Im Commit enthalten `git grep` 27 Dateien unter `vendor/` mit der Zeichenfolge „smart explorer“, darunter drei Dokumente (`vendor/SMART_EXPLORER_GL_ERRORS.md`, `vendor/SMART_EXPLORER_QAD_LIMITS.md`, `vendor/SMART_EXPLORER_ROUTE_POLICY.md`), also 24 Quelldateien. Die Arbeitsbaum-Liste unten nennt 27 Quelldateien; ob beide Listen übereinstimmen, ist [U] nicht abgeglichen. Nicht im Commit sind: `vendor/iroh-1.3.0/src/endpoint/remote_key_gate.rs` (neu), `native/crates/engine/src/share/core/blocked_keys.rs` (neu) und die Zeile `.remote_key_gate(…)` in `native/crates/engine/src/share/core/node.rs:172` mit der Modulzeile dazu; geändert gegenüber dem Commit sind `vendor/iroh-1.3.0/src/endpoint.rs`, `endpoint/connection.rs`, `tls.rs`, `tls/verifier.rs` und `vendor/SMART_EXPLORER_ROUTE_POLICY.md`. Gefunden wurden:

- `iroh-1.3.0` (10 Dateien): `src/endpoint.rs`, `src/endpoint/connection.rs`, `src/endpoint/quic.rs`, `src/endpoint/remote_key_gate.rs`, `src/socket.rs`, `src/socket/remote_map/remote_state.rs`, `src/socket/remote_map/application_path_pruning.rs`, `src/socket/remote_map/application_path_retirement.rs`, `src/tls.rs`, `src/tls/verifier.rs`.
- `noq-1.3.0` (3 Dateien): `src/lib.rs`, `src/endpoint.rs`, `src/connection.rs`.
- `noq-proto-1.3.0` (11 Dateien): `src/lib.rs`, `src/config/transport.rs`, `src/connection/application_path_policy.rs`, `src/connection/application_path_policy_tests.rs`, `src/connection/application_path_revalidation.rs`, `src/connection/datagrams.rs`, `src/connection/streams/state.rs`, `src/connection/mod.rs`, `src/connection/packet_builder.rs`, `src/frame.rs`, `src/n0_nat_traversal.rs`.
- `iroh-relay-1.0.0` (3 Dateien): `src/quic.rs`, `src/server/http_server.rs`, `src/server.rs`.

Wirkung der Änderungen (aus `vendor/SMART_EXPLORER_ROUTE_POLICY.md` und den Kopfzeilen):
- Anwendungs-Datengate im QUIC-Sender (Zeilen 13 bis 18): ein einzelner zugelassener Pfad trägt Nutzdaten; Erzeugung und Wechsel der Generation schalten das Gate zurück (Zeilen 28 bis 36). Das Gate ist ein Opt-in über die Transportkonfiguration vor dem Verbindungsaufbau (Abschnitt „API and lifetime contract“, ab Zeile 155). Standardnutzer behalten das Verhalten von upstream (Zeilen 41 bis 44).
- Verworfene Alternativen (Zeilen 38 bis 44): Sichtbarkeit „Available/Backup“ allein, leere Auswahl, Prüfung nur auf Anwendungsebene, Filtern verschlüsselter Pakete.
- Anhang „A7“ (Zeile 407, 2026-11-06): redundante IP-Kandidaten nach der Routenentscheidung freigeben (`application_path_pruning.rs`).
- Anhang „M6“ (Zeile 484, 2026-11-08): Schlüssel-Sperrtor vor jeder eigenen Adresse; keine TLS-Wiederaufnahme bei gesetztem Tor (`tls.rs`, `tls/verifier.rs`, `endpoint/remote_key_gate.rs`).
- Anhang „E33“ (Zeile 583): Standard-Resolver ohne öffentlichen Rückfall (`vendor/iroh-1.3.0/src/endpoint.rs:277, 707, 795`).
- Relay-Patches (`iroh-relay-1.0.0`): Klartext-Rückfall auf dem TLS-Port (`vendor/iroh-relay-1.0.0/src/server/http_server.rs:1171`, Zeilen 217 und 271) und Adressprüfung mit Retry für die Adressermittlung (`vendor/iroh-relay-1.0.0/src/quic.rs:1-3`; Limits in `vendor/SMART_EXPLORER_QAD_LIMITS.md`).
- Verifikationsstand (Zeilen 135 bis 143): „Source implementation and static author review complete. No local builds, compilers or tests are permitted.“ Remote-Nachweise sind verlangt. [U] Ob sie stattgefunden haben, ist in diesem Lesegang nicht belegt.
- Rebase-Vertrag (Zeilen 144 bis 154): bei jedem Wechsel des Iroh- oder Noq-Standes jede Änderung gegen den Upstream-Commit prüfen.
- Lizenz (Zeilen 5 bis 11): Die Lizenzhinweise der Upstream-Pakete müssen im Vertrieb mitgeliefert werden; das ist eine Vorgabe des Dokuments [D].

## 4. Routenregeln

### 4.1 Regeln nach Dokumentation

- Reihenfolge: gleiches LAN oder WLAN, dann die übrigen Direktwege, erst danach das Relay als gekennzeichneter Rückfall (`docs/ARCHITEKTUR.md` Zeilen 13 bis 17) [D].
- Startbrücke (Z-R2): Nutzdaten dürfen über einen geprüften Relay-Pfad laufen, solange LAN- und Direktwege aufgebaut werden; danach Wechsel auf den ersten zugelassenen LAN- oder Direktweg ohne Neustart; Routenzustand `relay_start` (`docs/ARCHITEKTUR.md` Zeile 16) [D].
- Direktkandidaten: lokale Schnittstellen, die öffentliche Adresse aus der QUIC-Adressermittlung am Share-Server (UDP 7842) und der Portmapper des Routers (UPnP, PCP, NAT-PMP) (`docs/ARCHITEKTUR.md` Zeilen 18 bis 22) [D]. [U] Der Portmapper wurde im Quelltext nicht gelesen.
- Relays: nur die des konfigurierten Share-Servers (`docs/ARCHITEKTUR.md` Zeilen 21 bis 22) [D].

### 4.2 Umsetzung im Quelltext

- Eine Routenentscheidung je Verbindung: `route_monitor::start(connection, …)` in `node_routes.rs:175-181` startet die Überwachung für jede Verbindung. Die Entscheidung gilt je Generation (`native/crates/engine/src/share/core/route_policy.rs:1-5`: „One finite, platform-independent LAN → direct → relay decision per generation“). Damit gelten die Routenregeln je Verbindung, nicht je Gegenstelle.
- Startbrücke: `native/crates/engine/src/share/core/route_start.rs:1-17`. Nutzdaten dürfen über den geprüften Relay-Pfad fließen, bis die erste LAN- oder Direktzulassung erfolgt. Danach wechselt die Verbindung ohne Neustart; das Relay trägt Nutzdaten nur noch als markierter Rückfall nach der endlichen Direktphase.
- Pfadbindung der Nutzdaten:
  1. Der Sender von noq lässt Anwendungsdaten nur über den einen zugelassenen Pfad (`ROUTE_POLICY.md` Zeilen 28 bis 36; Umsetzung in `vendor/noq-proto-1.3.0/src/connection/application_path_policy.rs`, nicht gelesen [U]).
  2. Vor dem Öffnen eines Stroms und vor dem Hallo wartet der Code auf die Anwendungsroute: `await_route` (`node_routes.rs:184-193`) ruft `route_monitor::wait_application_route` auf. Aufrufstellen: `node_sessions.rs:233` (Öffnen eines Stroms), `node_sessions.rs:391` (Sitzungsaufbau). Die Funktion `wait_application_route` selbst wurde nicht gelesen [U].
  3. Die Prüfung, ob die Pfadpolitik aktiv ist, steht in `route_protocol.rs:108`.
  4. Die Startphase prüft die Richtlinie der Verbindung in `route_start.rs:196`.
- Relay-Ausschluss für Bulk: Zusätzliche Verbindungen entstehen nicht über das Relay; eine zusätzliche Verbindung in der Relay-Startphase trägt keine Übertragung (`node_bulk.rs:16-18`).

### 4.3 Bindung je Verbindung oder je Gegenstelle

Je Verbindung: Pfadüberwachung, Startphase und Pfadzulassung hängen an `Connection` (`node_routes.rs:175-181`; `route_start.rs:196`). Je Gegenstelle gilt nur die Zählung der Bulk-Verbindungen (`node_bulk.rs:49-98`, Schlüssel `key`). Die Entscheidung über Bulk-Verbindungen hängt vom Zustand der ersten Verbindung ab (`node_bulk.rs:16-18`).

### 4.4 Wirkung bei Übertragungen

Für eine Übertragung auf einer Verbindung im Relay-Pfad gilt die Startbrücke; ein Wechsel auf einen LAN- oder Direktpfad erfolgt im selben Strom (`route_start.rs:6-10`: „Streams, their offsets and the retransmission queues belong to the connection, not to a path“).

### 4.5 Bulk-Verbindungen: Auslöser

`node_bulk.rs:33-46` und `70-88`: Alle 1 s wird gemessen (`SAMPLE`, Zeile 35). Auslöser: Verlust von mindestens 1 % (`LOSS_SHARE`, Zeile 38) bei mindestens 1000 gesendeten Datagrammen (`LOSS_MIN_DATAGRAMS`, Zeile 40), oder eine Rate von mindestens 250 MiB/s (`DRIVER_RATE`, Zeile 43). Die Zählung nutzt `connection.stats()` (`node_bulk.rs:71-73`).

## 5. Bildschirmübertragung

### 5.1 Weg eines Bildes

1. Erfassung: Plattformadapter unter `native/crates/engine/src/screen/os/windows/` (Graphics Capture, DXGI, Media Foundation, SendInput), `screen/os/linux_os/` (X11, Wayland-Portal, PipeWire) und `screen/os/android/` (`docs/ARCHITEKTUR.md` Zeile 87) [D]. [U] Die Erfassungsschicht wurde nicht gelesen.
2. Host-Pipeline: `native/crates/engine/src/screen/core/host_pipeline.rs:2-12` (Takt nimmt Bilder mit der gewünschten Rate; der Pacer entscheidet vor Erfassung und Kodierung, ob ein Bild in die Warteschlange passt; `FRAME_ACK` speist Pacer und Staukontrolle).
3. Kodierung: Codecs in `codec_registry.rs:25-29` (OpenH264 Version 2.6.0; Kachelcodec `tiles`).
4. Pacer: `pacing.rs:1-30`, `66-100` (Taktgeber, Warteschlangenwächter, Reset, Keyframe-Anforderungen).
5. Rahmenformat: `wire_binary.rs:1-9` (Bildgruppe: 20 Byte, Kennung `SCRV`, Version, `gop_id`, Art, Codec, Profil, Breite, Höhe; Rahmensatz: 21 Byte Kopf mit Länge `u32`, `frame_id` `u64`, `capture_ts_us` `u64`, Flags `u8`, big endian).
6. Strom: eine QUIC-unidirektionale Strom je Bildgruppe, geöffnet vom Gastgeber (`gop_stream.rs:1-2, 91`: `connection.open_uni()`).
7. Betrachter: nimmt höchstens 4 Ströme an (`gop_stream.rs:27`), Leser-Warteschlange 4 Bilder (`gop_stream.rs:30`), Annahme über `accept_uni` (`gop_stream.rs:270`).
8. Bestätigung: nach dem letzten Satz eines Bildes `FRAME_ACK` oder `PREVIEW_ACK` mit der Ankunftszeit (`gop_stream.rs:3-5`; Steuerwert `FRAME_ACK` = 18 in `wire.rs:33`).
9. Dekodierung und Anzeige: `screen/core/viewer_reconstruct.rs`, `decoded_frame.rs`; Präsentation nach `docs/ARCHITEKTUR.md` Zeile 160 in `native/crates/gui/src/app/core/screen_presented.rs` (GUI) und `ScreenSession.kt` (Android) (`docs/ARCHITEKTUR.md` Zeile 160) [D]. [U] Die Dateien wurden nicht gelesen.

### 5.2 Stau und Paketverlust

- Relay-Pfad: Warteschlange des Relays bis 1 MiB, der Schutz des Senders bleibt bei 768 KiB (`pacing.rs:14-17`).
- Warteschlangenzeit: 100 ms Ziel über eine Umlaufzeit hinaus (`pacing.rs:19`).
- Strom-Reset, wenn das älteste unbestätigte Bild älter als 1 s ist (`pacing.rs:21`); Reset-Code `0x5352` (`gop_stream.rs:24`); der Betrachter stoppt einen defekten Strom sofort (`gop_stream.rs:1-6`).
- Mehr als 1024 Bilder in Flug setzen den Strom ebenfalls zurück (`pacing.rs:22-24`).
- Nach einem Reset beginnt der Strom mit einem Keyframe (`tiles_codec.rs:169`).
- Keyframe-Anforderungen werden zusammengefasst (`pacing.rs:1-8`).

### 5.3 Bitrate und Bildrate

- Zwei Regler, das kleinere Ziel gilt (`rate_control.rs:1-11`): verzögerungsbasiert aus `FRAME_ACK` (Steigung der Verzögerung über die letzten 20 Bilder, `DELAY_WINDOW` in Zeile 26; Überlast setzt das Ziel auf 0,85 × Empfangsrate; sonst Probe mit Faktor 1,25 je max(RTT, Bildabstand)); verlustbasiert aus zwei `Connection::stats()`-Messungen im Abstand 1 s (Verlust über 10 % multipliziert mit (1 − 0,5 p), 2 bis 10 % halten, unter 2 % lösen die Verlustgrenze).
- Grenzen (`rate_control.rs:19-24`): Minimum 150 000 bit/s; Maximum 240 000 000 bit/s; Start 6 000 000 bit/s direkt und 2 000 000 bit/s über Relay.
- Bildrate: höchstens 240 Bilder je Sekunde (`pacing.rs:27`); niedrige Raten senken die Taktung (`rate_control.rs:11`).
- Seitenlänge der Bilder: `MAX_RESOURCE_SIDE` 16 384 (`frame_budget.rs:19`); Arbeitsspeicher 512 MiB je Prozess (`frame_budget.rs:21`).

### 5.4 Eingabe zurück

- Betrachter: Schreibaufgabe `writer` mit Eingaben und Rückmeldungen auf dem Steuerstrom (`viewer_client.rs:374-403`, Abschluss `finish` in Zeile 403). Der Steuerstrom wird vom Betrachter mit `accept_bi` angenommen (`viewer_client.rs:207`).
- Eingabebatch: Steuerwert `INPUT` = 17 (`screen/core/wire.rs:32`); kompakter Batch in big endian; Tastenbelegung nach USB-HID-Codes (`input_model.rs:1-4`). Der Tracker löst alle gedrückten Tasten, Knöpfe und Berührungen, wenn die Eingabe abbricht (`input_model.rs:4`).
- Gastgeber: Die Eingabe wird im Hostprozess ausgeführt (`docs/ARCHITEKTUR.md` Zeile 87 [D]); Befehlswerte des Arbeitsprozesses `INPUT` = 5 und `BOUND_INPUT` = 10 (`screen/core/worker_wire.rs:39-40`).
- Der Gastgeber öffnet den Steuerstrom und sendet zuerst das Serverhallo mit frischer Herausforderung (`share/core/node_screen.rs:274`; Zeilen 1 bis 11 des Kopfes).

### 5.5 Abweichung zum Design: unidirektionale Ströme

Befund (ohne Änderung): Die Produktionskonfiguration setzt `max_concurrent_uni_streams` auf 0 (`share/core/keepalive.rs:10, 141`). noq übergibt diesen Wert als Transportparameter `initial_max_streams_uni` (`vendor/noq-proto-1.3.0/src/transport_parameters.rs:171`); die Dokumentation der Konfiguration verlangt für Protokolle ohne fremd eröffnete Ströme den Wert null (`vendor/noq-proto-1.3.0/src/config/transport.rs:20-22`). Der Bildschirmpfad öffnet aber unidirektionale Ströme vom Gastgeber (`gop_stream.rs:91`) und erwartet bis zu 4 beim Betrachter (`gop_stream.rs:27`). Der Betrachter nutzt denselben Endpunkt (`share/core/node_screen_link.rs:82`, `self.endpoint.connect(addr, SCREEN_ALPN)`; Endpunkt nach `share/core/node.rs:171` mit 0). Folgerung nach dem Code: Der Gastgeber darf auf dieser Verbindung keine unidirektionalen Ströme öffnen. Ob der Bildstrom in der Praxis trotzdem läuft, ist nicht belegt [U: Laufzeitnachweis fehlt; in diesem Auftrag wurde nichts ausgeführt].

## 6. TCP-Freigaben und RDP

### 6.1 Weg eines TCP-Stroms

1. Betrachter: lokaler Listener, nur auf Loopback (`node_tcp_link.rs:1-2`), prüft die Herkunft jeder lokalen Verbindung über `ListenerAdmission` vor dem ersten Byte (`node_tcp_link.rs:1-3`).
2. Tunnelverbindung: `connect_with_opts(address, TCP_ALPN, options)` mit zusätzlichem ALPN `TCP_ALPN_V1` (`node_tcp_link.rs:85-89`). Eine QUIC-Verbindung je Tunnel; sie endet mit dem Tunnel, dem Widerruf oder dem Verbindungsende; ein Tunnel mit Auto-Weiterleitung endet auch mit der letzten lokalen Verbindung (`node_tcp_link.rs:4-6`).
3. Handshake: Der Gastgeber öffnet den ersten bidirektionalen Strom (`node_tcp.rs:103`) und sendet das Serverhallo; der Betrachter nimmt den Strom mit `accept_bi` an (`node_tcp_link.rs:152`); das Hallo ist JSON (`tcp_wire.rs:91, 103`).
4. Zielbindung: ein Tunnel hat genau eine Beziehung und eine unveränderliche, aufgelöste IP-Adresse (`node_tcp.rs:1-2`).
5. Daten: je lokaler TCP-Verbindung ein bidirektionaler Strom, geöffnet vom Betrachter (`tcp_registry.rs:447`, 20 s Frist) und vom Gastgeber angenommen (`node_tcp.rs:288`). Gleichzeitig höchstens 16 Ströme je Tunnel (`tcp_forward.rs:31`; `node_tcp.rs:274`; `node_tcp_link.rs:359`).
6. Kopie: zwei Richtungen mit je 8 KiB Puffer (`os/shared/tcp_pump.rs:1-3, 46-70`; `tcp_forward.rs:32`). EOF in einer Richtung schließt nur die Gegenrichtung (`tcp_pump.rs:1-3`, Schließen in Zeile 69). Die Gegenrichtung läuft unabhängig weiter.
7. Leerlauf: Budget je Verbindung (`tcp_pump.rs:89`).
8. Ende: Widerruf der Freigabe (`node_tcp.rs`, Schleife mit `revoked`), Abbruch durch den Stopp, Verbindungsende und Lebensdauer (`node_tcp.rs:57-58`, Abschluss der Kindaufgaben in Zeile 322 und `tcp_registry.rs:432`).

### 6.2 Verbindung oder gemeinsam

Ein QUIC-Strom je lokaler TCP-Verbindung; mehrere Ströme innerhalb einer Tunnelverbindung. Ein Tunnel teilt keine Verbindung mit `share-fs/3`, `share-exec/2`, `share-apps/1`, `paired-link/1`, `share-screen/1` oder `login/1` (Abschnitt 2.5).

### 6.3 Puffer und Halbschluss

- Kopierpuffer 8 KiB je Richtung (`tcp_forward.rs:32`).
- Halbschluss: unterstützt, EOF schließt nur die Gegenrichtung (`tcp_pump.rs:1-3`).
- Lebensdauer: V1-Tunnel höchstens 24 h (`tcp_forward.rs:18`). [U] Die Lebensdauerregel für Protokollversion 2 wurde nicht gelesen (`tcp_forward.rs:162-192`, Anfragefelder und Ablauf).
- Tunnelgrenze je Gerät: 64 (`tcp_forward.rs:30`).

### 6.4 RDP

Das RDP-Modul nutzt die Typen der TCP-Freigaben (`rdp/core/admission.rs:4, 8`; `rdp/mod.rs:30`, Schließen über `TunnelId`). Der Linux-Adapter liest die TCP-Tabellen des Kernels, um die Gegenstelle des Tunnels zu erkennen (`rdp/os/linux_os/gnome_net.rs:2, 15`). [U] Der Dienstport des RDP-Servers, der Protokollanteil des RDP-Clients und die Verbindung zum nativen Client wurden nicht gelesen.

## 7. Messungen und Messmittel

| Messmittel | Ort | Ergebnis im Repository |
|---|---|---|
| S4-Suite: Einkern-Durchsatz der AEAD-Verfahren (`s4_messung_task_aead_aes_128_gcm`, `s4_messung_task_aead_aes_256_gcm`, `s4_messung_task_aead_chacha20_poly1305`) und Anteil der Verschlüsselung an einer Loopback-Übertragung (`s4_messung_task_share_loopback_anteil`) | `native/crates/engine/src/share/core/s4_messung_task_tests.rs:17, 25, 33, 44`; Schwelle 10 % in `s4_messung_support.rs:24` und Prüfung in `s4_messung_task_tests.rs:40-54`; Suite-Teil `part_s4` in `native/test-crates-task.py:972-976`, Beschreibung in Zeilen 54 bis 59 | Keine Messwerte im Repository gefunden. Die S4-Dateien sind im Commit nicht enthalten: `s4_messung_support.rs` und `s4_messung_task_tests.rs` sind neu (nicht versioniert), und der Suite-Teil `part_s4` fehlt in `git show HEAD:native/test-crates-task.py`. Die Suche galt `docs/plaene/2026-11-08-offene-punkte/` und `docs/TRANSFER_ENGINE.md`; die Treffer enthalten Planung ohne Zahlen (`docs/plaene/2026-11-08-offene-punkte/plan.md:70`, Entscheidung E30). |
| Aussage „noch nie auf Remote-CI gelaufen“ | `plan.md:35` | [U] Im Repository nicht belegt; der Lesegang hatte keinen Zugriff auf die CI. |
| Durchsatztest über geformte Loopback-Leitung (50 ms, 100 Mbit/s) | `native/tests/transfer_throughput.rs` (Datei vorhanden); `docs/TRANSFER_ENGINE.md:967` | `docs/TRANSFER_ENGINE.md:968-972`: „Nicht gemessen: echte Netze der Nutzer …“ und „Diese Fassung ist nicht gebaut und nicht ausgeführt.“ |
| Routendiagnose im Terminal | `native/crates/cli/src/cli/share/status.rs:110-111` (Schlüssel `routes` und `route_measurement_note`), `status.rs:180` (`print_routes`), `status.rs:338` (Funktion) | Hinweistext `native/crates/engine/src/share/core/service_routes.rs:12`: „Frame-Nutzlast … Kein bestätigter Nutzdurchsatz und keine Leitungsrate.“ |
| Routenzustand (Diagnosefakten) | `native/crates/engine/src/share/core/route_diagnostics.rs:1-6` (begrenzte Fakten) | Keine Messwerte |
| Schwellen der Bulk-Verbindungen | `node_bulk.rs:35-43` | Festlegungen, keine Messwerte |
| Durchsatzschätzung 2 bis 5 Gbit/s am Desktop | `docs/plaene/2026-11-14-verbindungen-je-zweck/plan.md:31` (Eigenangabe „Schätzung, keine Messung“) | Keine Messung im Repository gefunden |
| Kennzahlen der Bildschirmübertragung | `native/crates/engine/src/screen/core/metrics.rs` (nicht gelesen) | [U] Ob dort dokumentierte Messwerte liegen, ist nicht geprüft. |

## 8. Prüfung der Zweitplan-Aussagen in plan.md (Abschnitt B, Zeilen 30 bis 35)

| Aussage in plan.md | Prüfergebnis | Beleg |
|---|---|---|
| „Je Gegenstelle genau eine QUIC-Verbindung … für alle Übertragungen“ | **Falsch.** Je Gegenstelle gibt es je Beziehung und Raum eine Sitzung (Schlüssel `direct:…`, `room:…`), dazu je ein eigener Verbindungstyp für LAN, Bildschirm, Apps, Exec, TCP-Tunnel und Maschinenanmeldung, und bis zu drei zusätzliche Dateisystem-Verbindungen für Übertragungen. | Abschnitte 2.2 bis 2.5 |
| „Ein Verbindungs-Treiber-Task je Verbindung“ | [U] Nicht geprüft (noq-Interna). | — |
| „Staukontrollregler Cubic, `share/core/keepalive.rs`“ | Bestätigt: Cubic ist Standard; BBRv3 nur über die Umgebungsvariable `SE_SHARE_QUIC_CC`. | `keepalive.rs:105-121` |
| „MTU-Obergrenze 1452 Byte (`vendor/noq-proto-1.3.0/src/config/transport.rs`)“ | Bestätigt als Obergrenze der MTU-Suche. Anfangswert 1200 Byte; harte Grenze 65 527 Byte. | `transport.rs:872, 904`; `lib.rs:360-361` |
| „Durchsatzgrenze 2–5 Gbit/s am Desktop ist eine Schätzung“ | Bestätigt als Schätzung; keine Messung im Repository. | Abschnitt 7 |
| „Bildschirmstrom läuft über einen zuverlässigen bidirektionalen Strom derselben Verbindung (`node_screen.rs:274`)“ | **Falsch in dieser Form.** Der bidirektionale Strom (`node_screen.rs:274`) trägt Hallo und Steuerung. Die Bildgruppen laufen als unidirektionale Ströme (`gop_stream.rs:91`); dazu besteht der Widerspruch aus Abschnitt 5.5. | `node_screen.rs:274`; `gop_stream.rs:1-2, 91, 27`; `keepalive.rs:10, 141` |
| „TCP-Freigaben: `share/core/node_tcp*.rs`, `tcp_forward.rs`, `tcp_wire.rs` (noch nicht gelesen)“ | Bestätigt; zusätzlich `os/shared/tcp_pump.rs` (Kopierrichtungen, Halbschluss). Gelesen in diesem Bestand. | Abschnitt 6 |
| „Klartext innerhalb der QUIC-Verbindung ist nicht möglich“ | Im Quelltext kein Klartextkanal zwischen Geräten gefunden; die Begründung aus `machbarkeit-unverschluesselt.md` wurde nicht gelesen. | Abschnitt 2.8 |
| „Messung des Verschlüsselungsanteils … noch nie auf Remote-CI gelaufen. Messwerte: keine.“ | Messwerte im Repository nicht gefunden. Der CI-Lauf ist [U]. | Abschnitt 7 |

## 9. Offene Punkte und nicht geklärte Angaben

Jeder Punkt nennt den fehlenden Fakt.

- **U1 (unidirektionale Ströme):** fehlt der Nachweis, ob der Gastgeber auf der Bildschirmverbindung mit `open_uni` scheitert oder blockiert wird. Ein Lauf in diesem Auftrag fand nicht statt. Offen ist auch die Konfiguration der Testendpunkte in `screen/core/batches_rate_reference_transport.rs` (Zeilen 98, 299, 307). Die Suche fand in dieser Datei keine Zeile mit `transport_config` oder `max_concurrent_uni`; die Endpunkte dieser Tests sind nicht geprüft.
- **U2 (Exec):** fehlt die Lektüre von `exec_server.rs` ab Zeile 132 (`serve_job`), um die Stromzuordnung nach dem Handshake zu belegen; fehlt die Zahl der Ausführungen je Verbindung.
- **U3 (Gerätefeed):** fehlt die Lektüre von `screen/core/wire_control.rs` für Rahmengröße und Stromaufteilung.
- **U4 (GRO):** fehlt die Suche nach der Aktivierung von GRO in `vendor/noq-1.3.0` und `vendor/noq-proto-1.3.0`.
- **U5 (Pfadgate im Sender):** fehlt die Lektüre von `vendor/noq-proto-1.3.0/src/connection/application_path_policy.rs` für die genaue Stelle der Nutzdatenbindung.
- **U6 (Summe der Verbindungen je Gegenstelle):** fehlt eine Obergrenze über alle ALPNs; keine im Quelltext gefunden.
- **U7 (direkte Reparatur):** fehlt die Zuordnung, auf welcher Verbindung `direct_reciprocal_transport.rs:113` den Strom öffnet, und der Rahmen des Stroms.
- **U8 (RDP):** fehlen Dienstport, RDP-Protokoll und die Verbindung zum nativen Client (Abschnitt 6.4).
- **U9 (SE_SHARE_QUIC_CC):** fehlt die Antwort, ob die Umgebungsvariable im Betrieb gesetzt sein kann; der Quelltext liest sie in Produktion (`keepalive.rs:119-121`), der Kommentar nennt sie einen Testschalter (`keepalive.rs:102-104`).
- **U10 (Routenpatches):** fehlt ein Nachweis der Remote-Läufe nach `vendor/SMART_EXPLORER_ROUTE_POLICY.md` Zeilen 135 bis 143.
- **U11 (Remote-Läufe S4 und E2E):** fehlen die Ergebnisse; `plan.md:35` behauptet keinen Lauf.
- **U12 (Erfassung und Kodierung):** fehlen die Lektüre der Erfassungsdateien (`native/crates/engine/src/screen/os/windows/`, `native/crates/engine/src/screen/os/linux_os/`, `native/crates/engine/src/screen/os/android/`, `native/crates/engine/src/screen/os/shared/`) und des Kodierpfads auf jeder Plattform.
- **U13 (Agent-Protokoll):** fehlt die Aufruferliste von `agent_proto` über Share-Verbindungen.
- **U14 (Lebensdauer Version 2 der TCP-Freigaben):** fehlt die Lektüre der Lebensdauerregel in `tcp_forward.rs` für Version 2.

Annahmen, die dieses Dokument benutzt:
- [A1] Die Zeilennummern entsprechen dem Arbeitsbaum am 2026-11-14, wie er mit `Read`, `rg` und `sed` gelesen wurde.
- [A2] Die drei geänderten Dateien (`node.rs`, `node_sessions.rs`, `session.rs`) unterscheiden sich von HEAD nur in den Zeilen, die `git diff` zeigt; der Diff wurde nicht vollständig gelesen.
- [A3] Die Dokumente in `docs/` spiegeln den Code nur, soweit in diesem Dokument ausdrücklich belegt.

## 10. Befunde ohne Änderung

Der Auftrag ist eine Bestandsaufnahme; die folgenden Befunde wurden nicht behoben.

- **B1:** Widerspruch zwischen der Transportkonfiguration (0 unidirektionale Ströme) und dem Bildschirmdesign (unidirektionale Bildgruppen). Belege: `share/core/keepalive.rs:10, 141`; `screen/core/gop_stream.rs:27, 91, 270`; `share/core/node_screen_link.rs:82`; `vendor/noq-proto-1.3.0/src/transport_parameters.rs:171`. Nachweis fehlt (U1).
- **B2:** Der Messpfad `s4_messung_support.rs:195-200` nutzt die Produktionskonfiguration und öffnet unidirektionale Ströme (`s4_messung_support.rs:247, 262`). Dieselbe offene Frage wie B1.
- **B3:** `SE_SHARE_QUIC_CC` wird im Produktionscode gelesen (`keepalive.rs:119-121`), obwohl der Kommentar einen Testschalter beschreibt (`keepalive.rs:102-104`). Keine Verletzung nachgewiesen (U9).
- **B4:** Die Datagramm-Pfade des Patches (`vendor/noq-proto-1.3.0/src/connection/datagrams.rs:1`) werden im Projekt nicht benutzt (Abschnitt 3).
- **B5:** Abweichungen der Zweitplan-Aussagen in `plan.md` (Abschnitt 8): Verbindungsanzahl je Gegenstelle und Bildschirmstrom sind falsch dargestellt.
