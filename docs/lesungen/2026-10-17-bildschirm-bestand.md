# Bildschirm-Bestand (Strang S4)

Stand 2026-10-17, Lesestand da2646d2

Dieser Bericht gehört zum Strang S4 „Bildschirm-Streaming und Fernsteuerung“ (`docs/plaene/2026-10-17-rechte-desktop-tresore/strang-bildschirm.md`), der Ergebnis E5 aus `docs/plaene/2026-10-17-rechte-desktop-tresore/plan.md`, Abschnitt A, umsetzt. Er hält nur fest, was im Code und in den lokalen Dokumenten steht. Er bewertet nichts und empfiehlt nichts.

Kennzeichnung:
- **Befund**: Der genannte Code oder das genannte Dokument sagt genau dies. Pfade sind relativ zur Repository-Wurzel `<Repository-Wurzel>`.
- **Dokumentaussage**: Nur das genannte Dokument behauptet es; die Quelle, auf die es sich beruft, wurde für diesen Bericht nicht gelesen.
- **Annahme**: Schlussfolgerung ohne direkten Beleg; im Text ausdrücklich als Annahme bezeichnet.
- **Lücke**: fehlende Information, mit genauer Benennung des fehlenden Fakts.

Zeilenangaben beziehen sich auf den Arbeitsbaum. Für alle gelesenen Quellpfade und Dokumente zeigte `git status --short` zum Zeitpunkt der Lektüre keine Änderung gegenüber `da2646d2`.

## 1. Ziel und Fragen

Ziel: Bestand für die spätere Entwurfsphase des Strangs S4 zusammentragen: Transport zwischen Geräten, Rechte je Gerät und Raum, Darstellung auf dem Desktop, Plattformanschlüsse auf Android sowie vorhandene Bausteine für Aufnahme, Kodierung und Eingabe.

Fragen:
- F1: Iroh- und QUIC-Versionen; Verbindungsaufbau (ALPN, Endpoint, Relay, Transportkonfiguration); Aushandlung von Fähigkeiten.
- F2: QUIC-Datagramme (konfiguriert oder genutzt); verwendete Stream-Arten; Rahmenaufbau am Beispiel des Exec-Protokolls.
- F3: Erteilung und Prüfung von Rechten je Gerät oder Raum; Fundstellen, an denen eine neue Fähigkeit ansetzen würde.
- F4: Durchsatz-, Latenz- und Relay-Grenzen aus den gelesenen Dokumenten.
- F5: Desktop-Darstellung (eframe-Backend, Texturen, Viewports, Bildwiederholung).
- F6: Android (Aufruf Kotlin nach Rust, Ereignisabfrage, Binärpfad, Vordergrunddienst-Typen, Navigation).
- F7: vorhandener Code für Bildschirmaufnahme, Video-Kodierung, Eingabe-Injektion oder Datagramme.

## 2. Befunde je Frage

### F1 — Versionen, Verbindungsaufbau, Fähigkeiten

**Versionen.**
- Befund: `native/Cargo.toml:112` setzt `iroh = { version = "1.3.0", default-features = true }`. Der Kommentar in `native/Cargo.toml:107-111` nennt Iroh 1.0 als Grundlage, 1.1.0 als Fix für GHSA-7cq4-mhxw-xw78, GHSA-vwh7-9883-83xj und GHSA-jx4g-cg2x-jc35, und „1.3.0 also lifts rustls“.
- Befund in `native/Cargo.lock`: `iroh` 1.3.0 (Zeile 3340), `iroh-base` 1.3.0 (3390), `iroh-relay` 1.3.0 (3459), `noq` 1.3.0 (4384), `noq-proto` 1.3.0 (4406), Eintrag `noq-udp` (4432; Version in der Lektüre nicht ausgegeben).
- Befund: Die Abhängigkeitsliste von `iroh` im Lock enthält `noq`, `noq-proto` und `noq-udp` (Cargo.lock ab Zeile 3339).
- Befund: Kein Paket `quinn` im Lock (Suche nach einem Namenseintrag mit `quinn`: 0 Treffer).

**Endpoint-Aufbau** (`native/src/share/core/node.rs`).
- Befund: Imports `iroh::endpoint::{presets, Connection, VarInt}` und `iroh::{Endpoint, RelayMode}` (node.rs:6-7).
- Befund: Share-Protokoll-ALPN `pub(super) const ALPN: &[u8] = b"smart-explorer/share-fs/3";` (node.rs:38).
- Befund: Exec-Protokoll-ALPN `pub(crate) const EXEC_ALPN: &[u8] = b"smart-explorer/share-exec/2";` (`native/src/share/core/exec_protocol.rs:11`).
- Befund: LAN-Link-ALPN `super::lan_link_wire::LAN_LINK_ALPN` (node.rs:155). Der Wert steht in `lan_link_wire.rs`; diese Datei ist nicht gelesen (Lücke).
- Befund (node.rs:149-158): `Endpoint::builder(presets::Minimal)`, `.crypto_provider(pq_kx::share_crypto_provider())`, `.secret_key(identity.iroh_secret.clone())`, `.alpns(vec![ALPN, EXEC_ALPN, LAN_LINK_ALPN])`, `.relay_mode(relay_mode)`, `.transport_config(iroh_transport_config())`.
- Befund (node.rs:147-148), Kommentar: „The ring provider of `presets::Minimal` plus the hybrid post-quantum group X25519MLKEM768 in front (pq_kx.rs).“
- Befund (node.rs:159-166): optional `ca_tls_config`; bei `transport_options.relay_only` ruft der Aufbau `clear_ip_transports()` auf; danach `builder.bind()` und `EndpointRoutes::start(&rt, &endpoint, relay_configured)`.

**Relay.**
- Befund (node.rs:141-146): ohne Relay-URLs `RelayMode::Disabled`; mit URLs `RelayMode::custom(transport_options.relay_urls.clone())`.
- Befund (`native/src/share/os/shared/transport_options.rs:45-60`): `load(server)` nimmt die Umgebungsvariable `SE_SHARE_RELAY_URL`, wenn sie nach Trim nicht leer ist; sonst `config.relay_urls()`.
- Befund (transport_options.rs:55-57): `SE_SHARE_RELAY_ONLY` setzt `relay_only` nur, wenn der getrimmte Wert genau `1` ist; jeder Wert außer `1` lässt `relay_only` auf `false`.
- Befund (node_sessions.rs:381-387): Statuszeile „Iroh-Session authentifiziert: … via {transport} in … ms“. Der Wert `transport` kommt aus `transport_label(&connection)`; dessen Definition ist nicht gelesen (Lücke).
- Dokumentaussage (`docs/lesungen/2026-09-28-share-host-transport-throughput.md` §6, Zeilen 181-183): `transport_label` liefert `"relay"`, wenn der gewählte Pfad ein Relay-Pfad ist, sonst `"direct"`.
- Dokumentaussage (`docs/SHARE_SERVER.md:394-395`): „Iroh still tries direct peer paths first.“
- Dokumentaussage (`docs/SHARE_SERVER.md:445-446`): `connected_relay` beschreibt eine authentifizierte Iroh-Datensitzung über den verschlüsselten Transport-Relay.

**Transportkonfiguration.**
- Befund: `iroh_transport_config()` stammt aus `core/keepalive.rs` (node.rs:20). Die Datei ist nicht gelesen; ihr Inhalt im Stand `da2646d2` ist in diesem Bericht nicht belegt (Lücke).
- Dokumentaussage (`docs/lesungen/2026-09-28-share-host-transport-throughput.md` §6, Zeilen 175-180; das Dokument zitiert `keepalive.rs:57-70` und die Konstanten `keepalive.rs:5-9`): `max_idle_timeout` 20 s, `keep_alive_interval` 5 s, `default_path_keep_alive_interval` 5 s, `max_concurrent_bidi_streams` 64, `max_concurrent_uni_streams` 0; keine Einstellung der Stream- oder Verbindungsfenster und der Überlastkontrolle.
- Dokumentaussage (`docs/refs/quic-sftp-throughput.md`, Abschnitt A.1, Zeilen 23-29): dieselben Felder; die Fenster bleiben auf den Standardwerten von `noq_proto::TransportConfig::default()`.
- Befund (`native/src/share/os/shared/lan_link_transport.rs:173-181`) für LAN-Link-Verbindungen: `set_max_concurrent_bi_streams(1)` (Zeile 175), `set_max_concurrent_uni_streams(0)` (176), `set_receive_window(16 * 1024)` (177); höchstens 12 Kanäle (179).
- Annahme: Der Aufruf `set_receive_window` auf einer Verbindung wirkt auf das Verbindungsfenster. Im Lesestand nicht belegt.

**Zeitgeber (Dokumentaussage für Iroh 1.0.1).** `docs/lesungen/2026-10-01-android-share-host-idle-activity.md`, Abschnitte 3.1 bis 3.5 (Zeilen 358-444). Die Werte stammen aus dem Quellstand von Iroh 1.0.1, nicht aus 1.3.0 (Lock):
- Relay-Ping 15 s (§3.1, Zeile 360); Ping-Timeout `clamp(3 x last_rtt, 500 ms, 5 s)` (363-364); Wiederverbindung mit Backoff von 10 ms bis 16 s (370); Verbindungsaufbau-Timeout 10 s (371).
- Pfad-Heartbeat 5 s, `PATH_MAX_IDLE_TIMEOUT` 15 s, `RELAY_PATH_MAX_IDLE_TIMEOUT` 30 s (§3.3, Zeilen 410-411).
- Netzbericht periodisch in zufälligen Abständen von 20 bis 26 s (§3.2, Zeilen 380-381); Berichts-Timeout `NET_REPORT_TIMEOUT` 10 s (385-386).
- QAD-Verbindung mit `keep_alive_interval` 25 s und `max_idle_timeout` 35 s (§3.2, Zeile 400).

**Aushandlung von Fähigkeiten, FS-Sitzung.**
- Befund (node_sessions.rs:331-338): Der Client fordert `"fs"`, `"fs_walk_batches_v1"` und `TRANSFER_V1_CAPABILITY` an; bei `kind == "direct"` zusätzlich `DIRECT_RECIPROCAL_CAPABILITY` (Zeilen 336-337).
- Befund (node_sessions.rs:352-361): `PeerHello { protocol_version: 3, relation_kind, relation_id, device_id, public_key, node_id, session_nonce, session_proof, requested_capabilities }`.
- Befund (node_sessions.rs:378-392): `Ctrl::PeerHelloOk` führt zum Erfolg; `Ctrl::FsResp` mit `FsResponse::Err { msg }` führt zu `PermissionDenied`.
- Befund (node_sessions.rs:341-351): Sitzungsbeweis `hmac_proof(&endpoint.relation_secret, &payload)`; `session_nonce` stammt aus `random_token(12)`.
- Befund (node.rs:396-403): `transfer_capabilities()` liefert `FsTransferCapabilities::host()`, bei `legacy_transfer_host()` die Standardwerte. Die Antwort mit diesen Merkmalen wird in nicht gelesenen Dateien gebildet (Lücke).

**Aushandlung, Exec-Sitzung** (`native/src/share/core/exec_protocol.rs`).
- Befund (Zeilen 36-44): `ExecServerHello { protocol_version: u16, capability: String, challenge, server_device_id, server_public_key, server_fingerprint, server_node_id }`. `ExecServerHello::new` setzt `EXEC_PROTOCOL_VERSION` und `EXEC_CAPABILITY` (Zeilen 46-63).
- Befund (Zeilen 67-79): `ExecClientHello { protocol_version, capability, relation_kind, relation_id, device_id, device_name, public_key, fingerprint, node_id, client_nonce, proof }`.
- Befund (Zeilen 82-88): `ExecHelloOk { authorization: ExecAuthorization, provider: ExecProviderStatus, features: Vec<String> }`. Das Feld `features` ist optional (`serde(default, skip_serializing_if = "Vec::is_empty")`); der Kommentar lautet „Optional host capabilities; hosts that predate them send none.“
- Befund (`native/src/share/core/exec_terminal.rs:11-13`): `EXEC_TERMINAL_FEATURE = "exec_terminal_v1"`.
- Befund (exec_terminal.rs:65-72): `host_features(terminal_supported)` liefert `["exec_terminal_v1"]`, sonst eine leere Liste.
- Befund (exec_terminal.rs:74-81): `host_denial` liefert den Fehlercode `terminal_unsupported`, wenn ein Start ein Terminal verlangt und der Host keines bereitstellt.
- Befund (exec_terminal.rs:86-98): `require_host_terminal` liefert `Ok`, wenn kein Terminal verlangt oder `exec_terminal_v1` angekündigt ist; sonst `Err` mit `TERMINAL_UNSUPPORTED`.

**Weitere Merkmalsnamen.**
- `remove_tree_v1`: 0 Treffer in allen Dateien der Leseliste (Quellen und Dokumente). Fundstelle: Lücke.
- `idle_keepalive_v1`: Befund in `native/src/share/core/power.rs:37` (Kommentar) und `docs/SHARE_SERVER.md:270-282` (Nachrichten `set_idle`, `keepalive_ack`, `idle_ack`, `keepalive`). Laut Dokument gilt das Merkmal im `hello` zum Signalisierungsserver; es betrifft also nicht die Gerät-zu-Gerät-Verbindung. Dokumentaussage (SHARE_SERVER.md:285-290): Standardintervall 180 s über `SE_SHARE_IDLE_KEEPALIVE_SECS`; der Client kann per `keepalive_secs` ein kürzeres Intervall vorschlagen, mindestens 30 s. Die Konstante `IDLE_KEEPALIVE_CAPABILITY = "idle_keepalive_v1"` steht in `native/src/share/core/wire.rs:31` (Trefferzeile aus einer Suche; Datei nicht gelesen).

### F2 — Datagramme, Stream-Arten, Rahmenaufbau

**Datagramme.**
- Befund: Keine Treffer für `datagram`, `Datagram`, `send_datagram`, `read_datagram` und `max_datagram` in den Leseliste-Dateien unter `native/src/share/core/` und `native/src/share/os/shared/`. Kein Datagramm-Aufruf in `node.rs:149-158` und `lan_link_transport.rs:173-181`.
- Dokumentaussage (`docs/refs/quic-sftp-throughput.md`, Abschnitt A.4, Zeilen 99-100; das Dokument zitiert `noq-proto` 1.0.1, `transport.rs:576-577`): `datagram_receive_buffer_size` = `Some(STREAM_RWND)` (1 250 000 Byte); `datagram_send_buffer_size` = 1 048 576 Byte.
- Annahme: Ein `Some(...)`-Wert für die Empfangspufferung bedeutet, dass Datagramme in der Standardkonfiguration aktiviert sind. Die Annahme stammt aus der Tabelle des Dokuments für noq-proto 1.0.1 und ist für die Lock-Version 1.3.0 nicht geprüft.
- Lücke: ob `core/keepalive.rs` (Quelle von `iroh_transport_config()`) Datagrammwerte setzt, ist nicht gelesen.

**Stream-Arten.**
- Befund, bidirektional, FS-Sitzung: `connection.open_bi()` für den Sitzungsaufbau (`node_sessions.rs:374-377`; der Steuerrahmen `PeerHello` folgt in Zeile 378) und für Peer-Streams (`node_sessions.rs:220-225`).
- Befund, bidirektional, LAN-Link: höchstens ein gleichzeitiger Stream (`lan_link_transport.rs:175`).
- Befund, unidirektional, LAN-Link: `set_max_concurrent_uni_streams(0)` (`lan_link_transport.rs:176`).
- Dokumentaussage, unidirektional, Gerät-zu-Gerät: `max_concurrent_uni_streams` = 0 (`docs/lesungen/2026-09-28-share-host-transport-throughput.md` §6, Zeile 178).
- Befund: Keine Aufrufe von `open_uni`, `accept_uni` oder `accept_bi` in den Leseliste-Dateien. Dokumentaussage (`docs/lesungen/2026-09-28-share-host-transport-throughput.md` §1, Zeilen 27-28): die Annahmeschleife über `accept_bi` steht in `server.rs:99-123`; diese Datei ist nicht gelesen.
- Lücke: Stream-Art des Exec-Pfads. `connect_exec` (`node.rs:361-366`) baut die Verbindung mit `EXEC_ALPN` auf. Ob danach `open_bi` oder `accept_bi` verwendet wird, steht in `exec_client.rs` und `exec_server.rs`; beide Dateien sind nicht gelesen.

**Rahmenaufbau des Exec-Protokolls** (`native/src/share/core/exec_protocol.rs`; Beispiel `exec_terminal`).
- Befund (Zeilen 14, 419-427, 431-437): Kopf 5 Byte, nämlich 1 Byte Tag, danach die Länge als `u32` in Big-Endian; danach die Nutzlast. Schreiben: Tag, Länge, Nutzlast, dann `flush()`.
- Befund (Zeilen 438-440): Vor dem Einlesen der Nutzlast prüft `read_payload` `len > limit` und liefert dann den Fehler „exec frame exceeds its byte limit“.
- Befund, Tags Handshake (Zeilen 16-19): `SERVER_HELLO` 1, `CLIENT_HELLO` 2, `HELLO_OK` 3, `HELLO_ERROR` 4.
- Befund, Tags Client (Zeilen 20-26): `START` 16, `STDIN` 17, `STDIN_EOF` 18, `CANCEL` 19, `PING` 20, `RESULT_ACK` 21, `RESIZE` 22.
- Befund, Tags Server (Zeilen 27-33): `STARTED` 32, `STDOUT` 33, `STDERR` 34, `TERMINAL` 35, `EXEC_ERROR` 36, `PONG` 37, `RESULT_ACKNOWLEDGED` 38.
- Befund, Varianten Client (Zeilen 99-125): `Start { start, digest }`, `Stdin { exec_id, data }`, `StdinEof { exec_id }`, `Cancel { exec_id }`, `Ping { exec_id, sequence }`, `ResultAck { exec_id }`, `Resize { exec_id, size }`.
- Befund, Varianten Server (Zeilen 128-136): `Started { exec_id }`, `Stdout { exec_id, data }`, `Stderr { exec_id, data }`, `Terminal(ExecTerminal)`, `Error(ExecWireError)`, `Pong { exec_id, sequence }`, `ResultAcknowledged { exec_id }`.
- Befund, Datenrahmen (Zeilen 396-409 und 411-417): Die Nutzlast beginnt mit den 32 Bytes der Ausführungskennung als ASCII (`exec_id.as_str().as_bytes()`), danach folgen die Rohdaten. Die Obergrenze der Rohdaten ist `MAX_EXEC_DATA_BYTES`; der Fehlertext in Zeile 403 lautet „exec data chunk exceeds 64 KiB“. Beim Empfang muss die Länge zwischen 32 und `32 + MAX_EXEC_DATA_BYTES` liegen; die Kennung stammt aus den ersten 32 Bytes (Zeilen 411-416).
- Befund, JSON-Rahmen (Zeilen 371-382): `send_json(writer, tag, value, limit)` serialisiert und prüft die Byte-Grenze `limit`. Die Grenzen aus den Aufrufen: Handshake `MAX_HANDSHAKE_BYTES` (Zeile 12, 64 KiB); Resize `MAX_RESIZE_BYTES` (Zeile 13, 256 Byte); Start `MAX_EXEC_START_BYTES` (Zeile 266; Wert nicht gelesen); `Started` 128 (Zeile 323); `Pong` 256 (Zeile 333); `ResultAcknowledged` 128 (Zeile 336); `Terminal` und `Error` `MAX_HANDSHAKE_BYTES` (Zeilen 327 und 330).
- Befund, Empfangsgrenzen: Client-Rahmen `MAX_EXEC_START_BYTES.max(MAX_EXEC_DATA_BYTES + 32)` (Zeile 286); Server-Rahmen `MAX_HANDSHAKE_BYTES.max(MAX_EXEC_DATA_BYTES + 32)` (Zeile 345).
- Befund, Reihenfolge: `ServerProtocolState` (Zeile 151) mit `accept` (Zeile 157) prüft die Reihenfolge der Client-Rahmen je Stream; die Phasen stehen in `enum Phase` (Zeile 139). Gelesen sind nur die Deklarationen in den Zeilen 139 und 151 und die Prüfstellen in den Zeilen 157, 168, 183-187, 195 und 203 (Suchtreffer).
- Befund, Leser (`native/src/share/core/exec_frame_reader.rs`): Kanal mit Kapazität 16 (Zeile 8). `client_frames` (Zeilen 29-44) und `server_frames` (Zeilen 46-61) starten je eine Tokio-Aufgabe, die Rahmen in einer Schleife liest und beim ersten Fehler endet (Zeilen 35-40 und 52-58). `Drop` bricht die Aufgabe ab (Zeilen 23-27).
- Befund, Terminal-Parameter (`native/src/share/core/exec_terminal.rs`): `ExecTerminalSize { cols: u16, rows: u16 }` ist für 1 bis 32767 gültig (Zeilen 17-18 und 23-38; der Kommentar in Zeile 17 nennt das vorzeichenbehaftete 16-Bit-`COORD` von ConPTY). `ExecTerminalRequest { term, size }`: `term` hat 1 bis 64 Zeichen aus ASCII-Buchstaben, Ziffern und `.`, `_`, `+`, `-` (Zeilen 19 und 49-62).
- Dokumentaussage, FS-Rahmen (`docs/lesungen/2026-09-28-share-host-transport-throughput.md` §4, Zeilen 120-129): `MAX_FRAME` 16 MiB (`framing.rs:11`); `fs::CHUNK` 256 KiB (`fs.rs:15`) als Einheit in beiden Richtungen; kein Bestätigungsrahmen je Datenblock; `STREAM_BUFFER_CHUNKS` = 2 (512 KiB). Die Quelldateien sind nicht gelesen.

### F3 — Rechte je Gerät und Raum

**Erteilen (Fundstellen).**
- Befund (`native/src/share/core/relation_rights.rs:15-25`): `SessionAuthorization { exports: ShareExportConfig, may_write: bool }` ist das Ergebnis der Prüfung eines eingehenden Dateisystem-Streams.
- Befund (relation_rights.rs:28-32): Direkt-Beziehung: `may_write` aus `DirectGrant.write`; `exports` aus `default_direct_exports` (Dokumentkommentar Zeilen 18-19). Die Herkunft des Feldes `default_direct_exports` ist nicht gelesen (Lücke).
- Befund (relation_rights.rs:35-39): Raum-Beziehung: `may_write` aus `room.policy.members_may_write`; `exports` aus `room.exports`.
- Befund (relation_rights.rs:21-23), Kommentar: „A writing request is allowed only for `may_write && root.access == ReadWrite`.“
- Befund (relation_rights.rs:45-51): `RelationScope::Direct` und `RelationScope::Room { room_id }` („One room (`RoomProfile.room_id`)“).
- Befund (relation_rights.rs:56-67): `PrincipalKey { public_key, node_id }`; ein Eintrag trifft zu, wenn der öffentliche Schlüssel oder die Knoten-ID gleich ist; leere Werte treffen nie zu.
- Befund (`native/src/share/core/node_accept.rs:100-126`): `known_endpoint` erkennt bekannte Gegenstellen über akzeptierte Direkt-Grants (`auth.direct_grants`), akzeptierte Direkt-Kontakte (`auth.direct_contacts`) und Räume mit `auto_join`, deren Mitglieder zugelassen sind (`auth.rooms`).
- Befund (`native/src/share/core/service.rs`): `enable_exec(target: ExecGrantTarget)` (Zeilen 87-92) und `disable_exec(target)` (94-99) senden über `exec_grant_command` (Zeile 114) die Befehle `ShareCmd::EnableExec { target }` und `ShareCmd::DisableExec { target }`. `apply_persisted_exec_grant` (Zeilen 101-113) enthält `ShareCmd::ApplyExecGrant` (Zeile 107). Die Typen `ExecGrantTarget` und `ExecGrantMutation` sind nicht gelesen (Lücke).

**Prüfen (Fundstellen).**
- Befund (node_accept.rs:46-79): Strafen nach fehlgeschlagener Authentifizierung gelten für Exec immer und für Dateisystem-Sitzungen nur bei unbekannten Gegenstellen (Kommentar Zeilen 48-50). Anmeldefrist 20 s für bekannte, 3 s für unbekannte Gegenstellen (Zeilen 131-132; Auswahl in Zeile 64).
- Befund (`native/src/share/core/node_restrictions.rs:11-30`): `bind_incoming_principal` übernimmt die Autorisierungs-Generation (`policy.snapshot`), holt einen Verbindungsplatz (`connection_ticket`), prüft `require_sharing_active()` und bindet mit `bind_admitted`.
- Befund (`native/src/share/core/node_policy.rs:19-35`): `Generation::check()` liefert `PermissionDenied` mit dem Text „Share-Autorisierung wurde entzogen“, wenn sich die Revision des Geltungsbereichs geändert hat.
- Befund (`native/src/share/os/shared/host_stream.rs:36`, 54, 79, 84): `authority.register_cancel(&cancel)` vor der Arbeit; `authority.check_read()` vor jedem Heartbeat und vor jeder Antwort. Der Typ `FsAccess` (`fs_access.rs`) ist nicht gelesen.
- Befund (`native/src/share/core/exec_protocol.rs:67-84`): Die Exec-Anmeldung trägt `relation_kind`, `relation_id` und `proof` (Client) sowie `authorization: ExecAuthorization` (Host). Die Prüfstelle auf Host-Seite (`exec_server.rs`) ist nicht gelesen (Lücke).

**Entziehen (Fundstellen).**
- Befund (relation_rights.rs:70-90): `RestrictionReason` mit den Varianten `Removed`, `Blocked`, `Reconfirm`, `IdentityChanged`, `WriteRevoked`, `ExecRevoked`, `ExportsNarrowed`, `RelationInactive`, `Unattributed`.
- Befund (relation_rights.rs:94-99): `RightsRestriction { relation: RelationScope, principal: Option<PrincipalKey>, reason }`; `RestrictionSet` mit `push`, `merge`, `is_empty`, `items` und `affects` (Zeilen 125-175).
- Befund (node_restrictions.rs:57-71): `invalidate_sessions` verwendet `RestrictionReason::Unattributed` für alle Beziehungen; `invalidate_restrictions` liest die Berechtigungs-Epoche.
- Befund (node_restrictions.rs:73-134): `invalidate_restrictions_at` erhöht die Sitzungs-Epoche und ruft `policy.invalidate(...)` auf. Es schließt nicht gebundene Verbindungen, entfernt Einträge aus `incoming_sessions` und `sessions`, ruft `mount_leases.invalidate(restrictions)` und `exec_registry().restrict_authorization(epoch, restrictions)` auf und schließt die betroffenen Verbindungen mit dem Code `0x5345` und dem Text „authorization changed“ (Zeilen 119-121).
- Befund (relation_rights.rs:1-6, 13): Die Einschränkungen aus Profiländerungen berechnet `authorization_restrictions` aus `relation_rights_diff.rs`; diese Datei ist nicht gelesen (Lücke).

**Gruppen im Code.** Der Plan nimmt an, dass „in the groups“ die Share-Räume meint. Der Code enthält dafür `RelationScope::Room` (relation_rights.rs:49-50) und Räume mit `auto_join` und zugelassenen Mitgliedern (node_accept.rs:121-125). Das ist eine Übereinstimmung, kein Beleg für das Produktverhalten.

**Fundstellen für eine neue Fähigkeit (ohne Bewertung).**
- Merkmalsliste der Gegenstelle: `node_sessions.rs:331-338` (`requested_capabilities`); Host-Merkmale: `node.rs:396-403`.
- Exec-Merkmale: `exec_terminal.rs:11-13`, `65-72`, `86-98`; `exec_protocol.rs:82-88` (`features`).
- Neuer Verbindungsweg (ALPN): `node.rs:38` (Konstante), `152-156` (Liste `alpns`); `node_accept.rs:42-45` (Sonderweg LAN-Link), `75-79` (Fehlerart je ALPN), `137-147` (Verteilung in `dispatch_connection`).
- Befehlsweg von der Oberfläche: `service.rs:49-85` (`ShareService::cmd`; Antwortfrist 5 s in Zeile 50; Stop-Barriere in Zeilen 51-56). Die Definition von `ShareCmd` steht in `types.rs` (nicht gelesen).
- Einschränkungen und Entzug: `relation_rights.rs:70-90` (Gründe), `node_restrictions.rs:61-134` (Schließen der Sitzungen).

### F4 — Durchsatz-, Latenz- und Relay-Grenzen aus den Dokumenten

**Keine Messwerte.** Die gelesenen Dokumente enthalten keine gemessenen Durchsatz-, Latenz- oder Bitratenwerte. Suche nach `measured`, `gemessen`, `Messung`, `bench`, `MB/s`, `MiB/s`, `Mbit`, `Mbps` in den gelesenen Dokumenten: Treffer nur als Grenzen oder Begründungen (`docs/SHARE_SERVER.md:184` und `261`; `docs/refs/quic-sftp-throughput.md:28`, `83`, `87`) und in den offenen Fragen des QUIC-Dokuments (Zeilen 414-416): „No numeric throughput model beyond noq-proto's „100 Mbps / 100 ms“ rationale and the qualitative `bidi_streams * stream_receive_window` memory rule exists; concrete target numbers for a given higher-bandwidth/higher-latency link are an implementer sizing decision, not a crate default.“

**Standardwerte der QUIC-Schicht** (Dokumentaussage, Quelle laut Dokument `noq-proto` 1.0.1; `docs/refs/quic-sftp-throughput.md`, Abschnitt A.4, Zeilen 79-101):
- Das Dokument nennt die Standardkonfiguration „tuned for a 100Mbps link with a 100ms round trip time“ (Zeilen 81-83); `EXPECTED_RTT` 100 ms und `MAX_STREAM_BANDWIDTH` 12 500 000 Byte/s (Zeilen 86-87).
- `stream_receive_window` 1 250 000 Byte (Zeile 95); `receive_window` `VarInt::MAX` (96); `send_window` 10 000 000 Byte (97); Überlastkontrolle Cubic (101).
- `max_idle_timeout` 30 s (Zeile 94); `initial_rtt` 333 ms (`docs/lesungen/2026-10-01-android-share-host-idle-activity.md` §3.4, Zeile 423).

**Projektwerte** (Dokumentaussage; Quelle `keepalive.rs`, nicht gelesen):
- `max_idle_timeout` 20 s, `keep_alive_interval` 5 s, `default_path_keep_alive_interval` 5 s, `max_concurrent_bidi_streams` 64, `max_concurrent_uni_streams` 0 (`docs/lesungen/2026-09-28-share-host-transport-throughput.md` §6, Zeilen 175-178).
- Iroh-Überschreibungen (`docs/refs/quic-sftp-throughput.md`, A.5, Zeilen 117-122): `keep_alive_interval` 5 s, `default_path_keep_alive_interval` 5 s, `default_path_max_idle_timeout` 15 s, `max_concurrent_multipath_paths` 8, `server_handshake_migration` true.

**LAN-Link** (Befund): Empfangsfenster der Verbindung 16 KiB (`native/src/share/os/shared/lan_link_transport.rs:177`); höchstens 12 registrierte Kanäle (Zeile 179).

**Dateiübertragung** (Dokumentaussage, `docs/lesungen/2026-09-28-share-host-transport-throughput.md` §4, Zeilen 123-129): ein Block von 256 KiB (`fs::CHUNK`, `fs.rs:15`) in beiden Richtungen; höchstens 2 Blöcke (512 KiB, `STREAM_BUFFER_CHUNKS=2`) laufen dem Gesendeten voraus; keine Bestätigung je Block; Gegendruck über die QUIC-Stream-Flusskontrolle.

**Signalisierungsserver** (Befund, `docs/SHARE_SERVER.md:183-186`): registrierte Verbindungen nehmen höchstens 128 Nachrichten/s und 2 MiB/s an; der Anfangsstoß erlaubt 1 282 Nachrichten (etwa 20 MiB). Das betrifft die Signalisierung, nicht den Dateiverkehr.

**Relay-Grenzen** (Befund, `docs/SHARE_SERVER.md:241-268`):
- Global höchstens 512 TCP-Verbindungen; je Quelle höchstens 64 (IPv4-Adresse oder IPv6-`/64`) (Zeilen 241-243).
- Annahme-Token: 256/s mit globalem Burst 512; 32/s mit Burst 64 je Quelle; LRU-Cache mit 4096 Einträgen (Zeilen 245-247).
- Gemeinsame Aufbaufrist 30 s (Zeilen 248-249).
- Nach der Iroh-Authentisierung höchstens 4 Verbindungen je authentifizierter Endpoint-ID und 512 authentifizierte Relay-Verbindungen insgesamt (Zeilen 250-252); Gnadenliste 10 Minuten (Zeilen 254-255).
- Je Relay-Verbindung höchstens 64 MiB/s empfangenes Chiffrat mit 8 MiB Burst (Zeilen 261-262).
- Je Ziel-Warteschlange höchstens 512 Pakete und 1 MiB zurückgehaltene Nutzlast; gemeinsame Obergrenze 64 MiB für alle Ziel-Warteschlangen (Zeilen 262-264).

**Relay-Zeitgeber** (Befund, `docs/SHARE_SERVER.md:321-328`): Der Relay pingt eine Verbindung 15 s plus 1 bis 5 s Jitter nach dem letzten empfangenen Rahmen an, danach alle K Sekunden plus 1 bis 5 s Jitter; K ist das Leerlauf-Intervall (Standard 180 s, Zeile 285). Jede Ping-Antwort muss innerhalb von 30 s kommen.

**Relay-Lage** (Befund und Dokumentaussage):
- Standard-Relay-Port ist der Signalisierungsport `N + 1` (`docs/SHARE_SERVER.md:27-28`). Im Standard beendet eine ungültige Relay-Bindung, ein Laufzeitfehler beim Relay-Start oder ein belegter Relay-Port den Prozess `se-share-server` mit einem Exitcode ungleich null (`docs/SHARE_SERVER.md:36-39`).
- Der Relay sieht Routing-Metadaten und Chiffrat, nicht Dateinamen, Dateiinhalte, Exportkonfiguration oder Beziehungsgeheimnisse (`docs/SHARE_SERVER.md:5-8`).
- Der Server-Lock führt `iroh-relay` 1.0.0 als lokal gepatchte Vendor-Kopie (`docs/SHARE_SERVER.md:642-645`, `652`); die Native-Clients nutzen `iroh` 1.3.0 (`native/Cargo.toml:112`). Dokumentaussage (Zeilen 669-676): die Frames sind zwischen 1.0.0 und 1.3.0 gleich, geprüft am 2026-10-16 gegen veröffentlichte Crates; die Quellen sind nicht gelesen.
- Dokumentaussage (`docs/lesungen/2026-10-01-android-share-host-idle-activity.md` §3.1, Zeilen 373-374): eine Relay-Verbindung beginnt erst, nachdem ein Netzbericht einen bevorzugten Relay ergeben hat.

**Zusammenfassung der Grenzen** (Befund):
- Keine Messwerte in den gelesenen Dokumenten (siehe oben).
- Die Standardwerte der QUIC-Schicht gelten laut Dokument für `noq-proto` 1.0.1; das Lock führt 1.3.0. Der Quellstand der Lock-Version ist nicht gelesen (Lücke).
- Die Relay-Durchsatzgrenze von 64 MiB/s gilt für den Server-Relay. Ob und wie sie den direkten Pfad berührt, ist nicht belegt.

### F5 — Desktop-Darstellung

**eframe-Backend** (Befund):
- `native/Cargo.toml:237` beginnt den Abschnitt `[target.'cfg(not(target_os = "android"))'.dependencies]`; Zeile 244: `eframe = { version = "0.29", default-features = false, features = ["accesskit", "default_fonts", "glow", "wgpu"] }`. Beide Backend-Features sind aktiviert.
- `native/Cargo.lock`: `eframe` 0.29.1 (Zeile 1830), `egui` 0.29.1 (1867), `egui-wgpu` 0.29.1 (1880), `egui-winit` 0.29.1 (1900), `egui_glow` 0.29.1 (1932), `wgpu` 22.1.0 (7980), `winit` 0.30.13 (8674); `glow` in den Versionen 0.13.1 (2581) und 0.14.2 (2593).
- `native/src/lib.rs:114-121`: `eframe::NativeOptions { viewport: ViewportBuilder::default().with_inner_size([1400.0, 900.0]).with_min_inner_size([900.0, 600.0]).with_icon(window_icon()).with_title("Smart Explorer"), ..Default::default() }`. Das Feld `renderer` wird nicht gesetzt.
- Lücke: Welches Backend der Standardwert von `NativeOptions` für eframe 0.29.1 wählt, ist in der Leseliste nicht belegt; die Quelle des Crates ist nicht gelesen.
- `native/src/lib.rs:126-134`: `eframe::run_native("Smart Explorer", options, Box::new(|cc| { … App::new(just_updated, initial_path) … app.configure_appearance(&cc.egui_ctx) … Ok(Box::new(app)) }))`.

**Texturen** (Befund):
- In `native/src/app/core/frame_update.rs` und `native/src/app/core/state.rs` gibt es keinen Treffer für `TextureHandle`, `load_texture`, `ColorImage`, `TextureOptions` und `set_visuals`. Der Ort der Texturerzeugung ist damit in der Leseliste nicht belegt (Lücke).
- `frame_update.rs:107-109`: `if self.icon_cache.drain(ctx) { ctx.request_repaint(); }`. Ob der Symbol-Cache Texturen erzeugt, ist nicht gelesen (Lücke).

**Viewports und Fenster** (Befund):
- Ein Hauptfenster: `run_native` mit genau einer Konfiguration (`lib.rs:126-134`).
- Viewport-Kommandos in den Leseliste-Dateien: `ctx.send_viewport_cmd(egui::ViewportCommand::Close)` (`frame_update.rs:11`) und `egui::ViewportCommand::Maximized(true)` (Zeile 68).
- In `lib.rs`, `frame_update.rs` und `state.rs` gibt es keinen Aufruf von `show_viewport_deferred`, `show_viewport_immediate` oder `ViewportId`. Das Feld `transfer_center.window_open` (`frame_update.rs:226`) ist vorhanden; sein Typ und seine Darstellung sind nicht gelesen (Lücke).

**Bildwiederholung** (Befund, `native/src/app/core/frame_update.rs`):
- Sofort: `request_repaint()` nach dem ersten Maximieren (Zeilen 66-69) und nach dem Abschluss des Symbol-Caches (Zeilen 107-109).
- Nach Frist, einzeln:
  - 5 s, wenn `crate::autostart::is_enabled()` zutrifft (Zeilen 35-38).
  - 150 ms, solange `view_dirty` gesetzt ist (Zeilen 140-142).
  - 80 ms, solange eine Ordnersuche wartet (Zeilen 146-151).
  - 150 ms, solange eine Textfilter-Eingabe wartet (Zeilen 155-160).
  - 500 ms, solange `self.notice` gesetzt ist (Zeilen 174-179).
- Gestuft ab Zeile 200 (die ersten Glieder der Bedingung vor Zeile 200 sind nicht gelesen):
  - 50 ms, wenn eines der Glieder in den Zeilen 200-215 zutrifft: `job_connect_rx`, `connecting`, `agent_activate_rx`, `remote_versions_rx`, `rollback_rx`, `trash_rx`, `trash_worker`, `clip_prepare_rx`, `share_open_rx`, `!edit_save_rx.is_empty()`, `picker.connecting || picker.listing`, `cloud_authing`.
  - 250 ms, wenn `share_worker_running`, `share_profiles.auto_connect`, `!remote_edits.is_empty()`, `quickshare.is_some()` oder `transfer_center.running_count() > 0` zutrifft (Zeilen 217-225; Kommentar 223-224).
  - 1 s, wenn `transfer_center.window_open` oder `clipboard_file_ops_supported()` mit einer Auswahl aus einer nicht lokalen Quelle zutrifft (Zeilen 226-236; Kommentar 233-235).

### F6 — Android: Aufruf, Ereignisse, Binärpfad, Dienste, Navigation

**Aufruf von Kotlin nach Rust** (Befund).
- `android/app/src/main/java/app/smartexplorer/android/core/NativeBridge.kt:11-16`: `object NativeBridge` mit genau drei `external`-Funktionen: `init(context: Context, configJson: String): String?` (Zeile 12), `call(method: String, argsJson: String): String?` (Zeile 14) und `pollEvents(timeoutMs: Int): String?` (Zeile 16).
- Befund (NativeBridge.kt:5-10 und 18-20): Jede Funktion liefert UTF-8-JSON, entweder `{"ok": …}` oder `{"err": {"kind", "message"}}`. Fehlt die Antwort, liefert sie `null`; `orError` macht daraus einen Fehlerumschlag.
- Befund (`native/android-bridge/src/lib.rs`): JNI-Namen `Java_app_smartexplorer_android_core_NativeBridge_init` (Zeile 116), `…_call` (139) und `…_pollEvents` (154). Parameter: `init(context: JObject, config_json: JString)`, `call(method: JString, args_json: JString)`, `pollEvents(timeout_ms: jint)`. Rückgabe jeweils `JString` (Zeilen 116-121, 139-144, 154-158).
- Befund (lib.rs:133, 148, 161): die Funktionen rufen `smart_explorer::mobile::init`, `::call` und `::poll_events` auf. Das Modul liegt unter `native/src/mobile/`; seine Dateien sind nicht gelesen (Lücke).
- Befund (`android/app/src/main/java/app/smartexplorer/android/core/Core.kt:81-89`): `suspend fun call(method, args: JsonObject)` ruft `NativeBridge.call(method, args.toString())` auf `Dispatchers.IO` auf und wandelt die Antwort mit `orError` um. `request<T>` (Zeilen 97-103) dekodiert auf `Dispatchers.Default`.
- Befund (Core.kt:126-143): `initialize(app)` ruft `NativeBridge.init(app, InitConfig.build(app).toString())` auf; die Antwort wertet `unwrap` aus (Zeilen 126-137).

**Ereignisabfrage** (Befund).
- Core.kt:160: ein Daemon-Thread „core-events“ führt `pumpEvents()` aus.
- Core.kt:177-205: die Schleife ruft `NativeBridge.pollEvents(POLL_TIMEOUT_MS)` auf (Zeile 182; `POLL_TIMEOUT_MS` = 1 000, Zeile 41), danach `CoreEvents.parse` (Zeile 187). Ereignisse vom Typ `task` gehen an `taskStore.upsert` (Zeile 190), `wake` an `WakeKeeper.hold(ms)` (Zeile 192); jedes Ereignis wird mit `eventFlow.tryEmit(event)` veröffentlicht (Zeile 195). Bei Fehlern oder leeren frühen Antworten wartet die Schleife `POLL_RETRY_MS` = 1 000 ms (Zeilen 197-202; Konstante Zeile 42).
- `CoreEvents.kt:37-48` dekodiert genau diese Typen: `task`, `share`, `shareRequest` (mit `count`), `jobs`, `syncProblem` (mit `jobId`, `title`, `text`), `edits`, `openUrl` (mit `url`), `error` (mit `action`, `message`), `volumes`, `wake` (mit `ms`, nur bei Werten größer als 0). Ein Typ, der nicht in dieser Liste steht, liefert `null` (Zeile 48).

**Binärpfad.** Befund: Es gibt keinen Binärpfad auf der Brücke.
- Die drei JNI-Funktionen tauschen nur `JString`, `JObject` und `jint` aus (`native/android-bridge/src/lib.rs:116-121`, `139-144`, `154-158`); in `lib.rs` gibt es kein `jbyteArray`.
- `NativeBridge.kt`, `Core.kt` und `CoreEvents.kt` enthalten kein `ByteArray`. `ByteArray` kommt vor in `android/app/src/main/java/app/smartexplorer/android/core/SecretsKey.kt` (Zeilen 77, 82, 124, 132) und in `android/app/src/main/java/app/smartexplorer/android/ui/share/ExecProgramsDialog.kt:45` (Größenprüfung); keine dieser Stellen gehört zur Kern-Brücke.
- Lücke: ob `smart_explorer::mobile` Binärdaten über einen Weg liefert, der nicht in den gelesenen Dateien steht, ist nicht gelesen.

**Vordergrunddienste** (Befund, `android/app/src/main/AndroidManifest.xml`).
- `TaskForegroundService` (`.service.TaskForegroundService`), `foregroundServiceType="dataSync"` (Zeilen 93-96).
- `BackgroundService` (`.service.BackgroundService`), `foregroundServiceType="specialUse"`, mit `<property android:name="android.app.PROPERTY_SPECIAL_USE_FGS_SUBTYPE">` (Zeilen 100-108).
- `androidx.work.impl.foreground.SystemForegroundService`, `foregroundServiceType="dataSync"` (Zeilen 110-114).
- Zur Laufzeit ruft `BackgroundService.kt:87` `ServiceCompat.startForeground(this, Notifications.ID_BACKGROUND, content.build(), ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)` auf. Der Laufzeittyp von `TaskForegroundService` ist nicht gelesen (Lücke).
- Vollständige Liste der 16 Berechtigungen mit dem Präfix `android.permission.` (Zeilen 6-46): `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_DATA_SYNC`, `FOREGROUND_SERVICE_SPECIAL_USE`, `POST_NOTIFICATIONS`, `RECEIVE_BOOT_COMPLETED`, `SCHEDULE_EXACT_ALARM`, `MANAGE_EXTERNAL_STORAGE`, `INTERNET`, `ACCESS_NETWORK_STATE`, `ACCESS_WIFI_STATE`, `CHANGE_WIFI_MULTICAST_STATE`, `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`, `WAKE_LOCK`, `PACKAGE_USAGE_STATS`, `QUERY_ALL_PACKAGES`, `REQUEST_INSTALL_PACKAGES`. Nicht enthalten sind `CAMERA`, `RECORD_AUDIO`, `MEDIA_PROJECTION` und `FOREGROUND_SERVICE_MEDIA_PROJECTION`.
- Dokumentaussage (`docs/refs/android-platform.md`, Zeile 22): Die Typen `systemExempted, camera, health, location, mediaPlayback, mediaProjection, microphone, phoneCall, remoteMessaging` sind mit dem Vermerk „(not relevant here)“ aufgeführt. Dokumentaussage (Zeile 56): `mediaProjection` ist laut Dokument bei BOOT_COMPLETED-Starts nicht zulässig.
- Suche in `docs/refs/android-platform.md` und `docs/refs/android-apis.md` nach MediaProjection, screen capture, Bildschirm, SurfaceView, MediaCodec, AccessibilityService und dispatchGesture: außer `android-platform.md` Zeilen 22 und 56 kein Treffer.

**Navigation** (Befund, `android/app/src/main/java/app/smartexplorer/android/ui/AppNav.kt`).
- Zeile 14: `enum class MainTab { Files, Sync, Share, More }`.
- Zeilen 16-30: `sealed interface NavRequest` mit sieben Varianten: `OpenLocation(location)`, `SelectTab(tab)`, `ShowTransfers`, `OpenJobConflicts(jobId)`, `ShowShareRequests`, `ShowUpdate`, `ShowBackgroundSettings`.
- Zeilen 40-138: `object AppNav` ist ein Anfragebus. `send` (57-62), `consume` (65-67), `flushQueued` (70-77); Flows `requests` (53) und `pending` (54). Der Kommentar in Zeilen 35-38 nennt `AppRoot` als Sammler; `AppRoot` ist nicht gelesen.
- Intents: `intentFor` (80-91) und `fromIntent` (97-111) mit privatem Token (Zeilen 97-99 und 133-138). `tabFor` (114-120) ordnet jede Anfrage einem Reiter zu.
- Suche (Groß-/Kleinschreibung beachtet): 0 Treffer für `NavHost`, `composable`, `rememberNavController`, `navigate`, `route`, `Route` und `Screen` in `AppNav.kt`. `native/Cargo.toml:233-234` nennt die Android-Oberfläche „a Compose app“.
- Launcher-Aktivität `MainActivity` (Manifest Zeilen 61-69): `android:exported="true"`, Filter `MAIN` und `LAUNCHER`; weitere Filter für `SEND` (71-75), `SEND_MULTIPLE` (76-80) und `VIEW` (84-88).

### F7 — Vorhandener Code für Aufnahme, Kodierung, Eingabe, Datagramme (Suche)

Suchumfang: `native/src` und `android/app/src` (Auftragsvorgabe). Nicht durchsucht: `native/android-bridge/` und `share-server/`; die vollständige Liste der übrigen Verzeichnisse der Repository-Wurzel wurde in diesem Lauf nicht erhoben (Lücke).

**Auftragsbegriffe** (Befund):
- 0 Treffer: `MediaProjection`, `SendInput`, `uinput`, `xdotool`, `XTest`, `pipewire`, `DXGI`, `GraphicsCapture`, `MediaCodec`, `h264`, `openh264`, `vpx`, `av1`, `ffmpeg`, `gstreamer`, `datagram` (jeweils ohne Beachtung der Schreibweise).
- `screenshot`: 10 Treffer, alle in `android/app/src/androidTest/java/app/smartexplorer/android/task/UiTaskTest.kt`, Zeilen 29, 51, 58, 70, 76, 83, 89, 95, 101, 108. Die Hilfsfunktion `screenshot(name)` (Zeile 58) gehört zu Oberflächentests in der Instrumentierung.
- `capture`: 480 Treffer in 111 Dateien. Prüfung nur auf den Zeileninhalt (ohne Dateipfad): 9 Zeilen enthalten `capture` zusammen mit einem der Wörter `screen`, `window`, `display`, `monitor`, `frame`, `desktop`, `video`, `image`, `bitmap`, `pixel`, `surface`, `framebuffer` oder `bildschirm`. Davon sind `UiTaskTest.kt:6` (Import `captureToImage`) und `UiTaskTest.kt:60` (`compose.onRoot().captureToImage().asAndroidBitmap()`) Compose-Testcode der Instrumentierung. Die übrigen 7 Zeilen betreffen `captured.frames()` (`native/src/agent_proto/core/delete_speed_task_tests.rs:67` und `transfer_engine_task_bounds_tests.rs:79`), `displayed action` (`native/src/bisync/os/shared/preview.rs:234`), `captured.retained_location().display()` (`native/src/analytics/os/shared/checked_recycle.rs:41`), `display_truncated` (`native/src/daemon/os/shared/mount_host_process.rs:178`), `Freedesktop` (`native/src/analytics/os/linux_trash.rs:1`) und den Kommentar „Windows capture“ (`native/src/local_access/os/windows/quarantine.rs:1`). Keine Zeile beschreibt eine Bildschirmaufnahme.
- `RTP` als ganzes Wort: 0 Treffer. `rdev` ohne Schreibweisenbeachtung: 43 Treffer; echte Gerätenummern `rdev()` nur in `native/src/vfs/os/linux_os/volume_id.rs` (3 Zeilen) und `native/src/vfs/os/linux_os/mountinfo.rs` (1 Zeile); die übrigen sind Wortteile wie `peerDeviceId` oder `PeerDeviceKey` (Stichprobe der ersten zwölf Treffer).

**Erweiterte Suche über die Auftragsbegriffe hinaus** (Befund; Begriffe: `BitBlt`, `PrintWindow`, `GetDC`, `XGetImage`, `XShm`, `PixelCopy`, `ImageReader`, `keybd_event`, `mouse_event`, `injectInputEvent`, `dispatchGesture`, `AccessibilityService`, `MediaRecorder`, `VirtualDisplay`, `SurfaceView`, `TextureView`, `webrtc`, `WebRTC`, `NV12`, `yuv`, `YUV`, `Desktop Duplication`, `IDXGI`, `Graphics.Capture`, `enigo`, `libei`, `RemoteDesktop`, `xdg-desktop-portal`, `ScreenCast`):
- 0 Treffer für alle Begriffe außer `GetDC`, `RTP` und `rdev`.
- `RTP` (Groß-/Kleinschreibung unabhängig): 45 Treffer, alle als Wortteil (zum Beispiel `Startpfad`); als ganzes Wort 0 Treffer.
- `rdev`: 43 Treffer, Einzelheiten siehe oben.
- `GetDC`: 3 Treffer in `native/src/icons/os/windows.rs` (Zeilen 5, 132, 163). Zeile 132 lautet `let hdc = GetDC(None);`, gefolgt von einem Bitmap-Puffer und `GetDIBits` (Zeilen 132-134). Der Verwendungszweck ist nur aus diesen Zeilen ablesbar; die Datei ist nicht vollständig gelesen.

**Ergebnis F7** (Befund): In `native/src` und `android/app/src` gibt es keinen Code für Bildschirmaufnahme, Video-Kodierung, Eingabe-Injektion oder Datagramme. Die Lücke liegt außerhalb dieser Verzeichnisse (siehe oben).

## 3. Wiederverwendbare Bausteine

Die Liste nennt vorhandene Bausteine mit Fundstelle und Bindung. „Bindung“ gibt an, an welchen Typ oder welche Umgebung der Baustein gebunden ist. Eine Eignung für einen Entwurf wird nicht bewertet.

| Baustein | Fundstelle | Bindung |
|---|---|---|
| Endpoint-Aufbau mit ALPN-Liste, Relay-Modus, Krypto-Provider und Transportkonfiguration | `native/src/share/core/node.rs:140-166` | `iroh`-Typen; `pq_kx` und `keepalive` nicht gelesen |
| Eingehende Verbindungen: Akzeptschleife, Anmeldefristen, Strafen, ALPN-Verteilung | `native/src/share/core/node_accept.rs:13-147` | `ShareIrohNode`; Ziele `server::handle_connection` und `exec_server::handle_connection` nicht gelesen |
| Peer-Hello mit Fähigkeitsliste und Sitzungsbeweis | `native/src/share/core/node_sessions.rs:326-392` | `PeerHello` und `Ctrl` aus `wire.rs` (nicht gelesen) |
| Rahmencodec des Exec-Protokolls: Tag, Länge, Grenze, JSON- und Datenrahmen | `native/src/share/core/exec_protocol.rs:371-450` | `ExecId` und Exec-Rahmen; `read_payload` und `write_payload` (Zeilen 419-444) sind nur an `R: AsyncRead + Unpin` beziehungsweise `W: AsyncWrite + Unpin` gebunden |
| Rahmenleser als Hintergrundaufgabe mit Kanal der Kapazität 16 | `native/src/share/core/exec_frame_reader.rs:12-61` | generisch über `T`; instanziiert mit `ClientFrame` und `ServerFrame` |
| Serve-Muster für Streams: Heartbeat alle 250 ms, Kanal der Kapazität 2, Abbruch, Wachhalten | `native/src/share/os/shared/host_stream.rs:24-103` | `SendStream`, `FsAccess`, `FsResponse` |
| Terminal-Größe und TERM-Prüfung | `native/src/share/core/exec_terminal.rs:17-62` | Exec (`ExecStart`) |
| Rechte: Geltungsbereich, Einschränkungsmenge, Generation, Entzug | `native/src/share/core/relation_rights.rs:15-175`, `node_policy.rs:19-67`, `node_restrictions.rs:11-134` | `PeerPrincipal`, `ShareExportConfig`, `DirectGrant`, `RoomProfile` |
| Leistungszustand des Clients (Ruhemodus, Weckprüfung, Haltezeit) | `native/src/share/core/power.rs:1-89` | prozessweit; der Android-Host ruft `set_low_power`, `request_probe` und `set_activity_hook` auf (`power.rs:3-7`) |
| Befehlsweg der Oberfläche mit Antwortfrist und Stop-Barriere | `native/src/share/core/service.rs:49-85` | `ShareCmd` (`types.rs`, nicht gelesen) |
| Android-JSON-Brücke: `init`, `call`, `pollEvents`, Ereignispumpe | `NativeBridge.kt`, `Core.kt:81-205`, `CoreEvents.kt`, `native/android-bridge/src/lib.rs:114-163` | Kern-API nach `api.md §1` (Datei nicht gelesen, Pfad nicht ermittelt) |
| eframe-Startkonfiguration | `native/src/lib.rs:114-134` | eframe 0.29 |
| Bildwiederholung nach Zustand | `native/src/app/core/frame_update.rs:200-237` (Ausschnitt) | `egui::Context::request_repaint_after` |
| Android-Navigationsbus mit Token-geschützten Intents | `android/app/src/main/java/app/smartexplorer/android/ui/AppNav.kt:14-139` | `MainActivity`, `Intent` |
| Vordergrunddienst-Deklarationen | `android/app/src/main/AndroidManifest.xml:6-8`, `93-114` | Dienstklassen (`TaskForegroundService`, `BackgroundService`) |

## 4. Betroffene Verträge

**Netz**
- ALPN-Kennungen: `smart-explorer/share-fs/3` (`node.rs:38`), `smart-explorer/share-exec/2` (`exec_protocol.rs:11`), `LAN_LINK_ALPN` (Wert nicht gelesen).
- FS-Anmeldung: `PeerHello.protocol_version` = 3 und `requested_capabilities` (`node_sessions.rs:352-361`); Antwort `PeerHelloOk` (Zeile 380).
- Exec-Anmeldung: `protocol_version` und `capability` in `ExecServerHello` und `ExecClientHello` (`exec_protocol.rs:36-79`); Merkmalsliste `ExecHelloOk.features` (Zeilen 82-88); Merkmal `exec_terminal_v1` (`exec_terminal.rs:13`).
- Exec-Rahmen: Tags (`exec_protocol.rs:16-33`), Kopf 5 Byte (Zeile 14), Grenzen (Zeilen 12, 13, 286, 345).
- Relay: Client `iroh-relay` 1.3.0 (`Cargo.lock:3459`); Server `iroh-relay` 1.0.0, lokal gepatcht (`docs/SHARE_SERVER.md:642-676`).
- Signalisierung: Merkmal `idle_keepalive_v1` (`docs/SHARE_SERVER.md:270-300`; `native/src/share/core/wire.rs:31`, Trefferzeile).
- Umgebungsvariablen: `SE_SHARE_RELAY_URL` und `SE_SHARE_RELAY_ONLY` (`transport_options.rs:9-10`, `48`, `55`); `SE_SHARE_IDLE_KEEPALIVE_SECS` (`docs/SHARE_SERVER.md:285`).

**Rechte**
- `SessionAuthorization { exports, may_write }` (`relation_rights.rs:15-25`).
- `RestrictionReason` (`relation_rights.rs:70-90`), `RightsRestriction` (94-99), `RestrictionSet` (125-175).
- `Generation` und `SessionPolicy` (`node_policy.rs:19-67`).
- Sitzungsentzug mit Schließcode `0x5345` (`node_restrictions.rs:73-134`, Schließen in Zeile 120; `node.rs:367-371` für die Anmeldung).
- Exec: `ExecHelloOk.authorization` (`exec_protocol.rs:83`); `ExecRevoked` (`relation_rights.rs:81-82`); Befehle `EnableExec`, `DisableExec`, `ApplyExecGrant` (`service.rs:91`, `98`, `107`).

**IPC**
- `ShareService::cmd(&self, c: ShareCmd) -> Result<ShareCmdResult, String>` (`service.rs:49`); Antwortfrist 5 s (Zeile 50); Ereigniskanal `events: Receiver<ShareEvent>` (Zeile 24).
- Befehle, die im Lesestand vorkommen: `Stop` (Zeile 51), `EnableExec`, `DisableExec`, `ApplyExecGrant` (Zeilen 91, 98, 107).
- Lücke: die vollständige Liste der `ShareCmd`-Varianten (`types.rs`) und der Daemon-IPC-Weg (`native/src/daemon/`) sind nicht gelesen.

**Bridge (Android)**
- JNI-Funktionen `init`, `call`, `pollEvents` (`NativeBridge.kt:12-16`; `native/android-bridge/src/lib.rs:116-158`), alle mit String-Rückgabe.
- Ereignistypen (`CoreEvents.kt:37-47`): `task`, `share`, `shareRequest`, `jobs`, `syncProblem`, `edits`, `openUrl`, `error`, `volumes`, `wake`.
- Abruf: `POLL_TIMEOUT_MS` = 1 000 ms (`Core.kt:41`); Wiederholung `POLL_RETRY_MS` = 1 000 ms (Zeile 42).

**GUI (Desktop)**
- Hauptfenster: `ViewportBuilder` mit Größe, Mindestgröße, Symbol und Titel (`native/src/lib.rs:114-121`).
- eframe-Features `glow` und `wgpu` (`native/Cargo.toml:244`).
- Viewport-Kommandos `Close` und `Maximized` (`frame_update.rs:11`, `68`); Bildwiederholung (`frame_update.rs:34-236`).

**GUI (Android)**
- Anfragen `NavRequest` (`AppNav.kt:16-30`); Intents mit Token (`AppNav.kt:80-111`, `133-138`).
- Launcher-, Teilen- und Anzeige-Filter von `MainActivity` (`AndroidManifest.xml:61-88`).
- Vordergrunddienste und ihre Berechtigungen (`AndroidManifest.xml:6-8`, `93-114`; `BackgroundService.kt:87`).

**Nicht gelesen:** CLI und MCP (`native/src/cli/`), Daemon-IPC (`native/src/daemon/`) und die Netzprotokoll-Dokumentation außerhalb der genannten Dateien. Eine Inventur der Oberflächen ist nicht Teil dieses Berichts.

## 5. Lücken und offene Punkte

**Ungelesene Quellen mit direkter Frage**
1. `native/src/share/core/keepalive.rs`: Inhalt von `iroh_transport_config()` im Stand `da2646d2`; ob Datagrammwerte gesetzt sind; Stream- und Verbindungsfenster; Überlastkontrolle. (F1, F2, F4)
2. `native/src/share/core/lan_link_wire.rs`: Wert von `LAN_LINK_ALPN`. (F1)
3. `native/src/share/core/wire.rs`: Werte von `TRANSFER_V1_CAPABILITY` und `IDLE_KEEPALIVE_CAPABILITY`; Typen `PeerHello`, `Ctrl`, `FsResponse`, `FsTransferCapabilities`. (F1, F2)
4. `super::exec_types` (importiert in `exec_protocol.rs:6-9`): Werte von `MAX_EXEC_DATA_BYTES`, `MAX_EXEC_START_BYTES`, `EXEC_PROTOCOL_VERSION`, `EXEC_CAPABILITY`; Typen `ExecAuthorization`, `ExecProviderStatus`, `ExecStart`, `ExecTerminal`, `ExecId`. (F1, F2)
5. `native/src/share/core/exec_server.rs` und `exec_client.rs`: Prüfung der Exec-Anmeldung auf Host-Seite; Stream-Art des Exec-Pfads; Ort der Erzeugung von `features`. (F2, F3)
6. `server.rs` (Ziel von `server::handle_connection`; Annahmeschleife über `accept_bi`). (F2, F3)
7. `native/src/share/core/node_idle.rs` und `node_wake.rs` (eingebunden in `node.rs:29-34`): Zeitgeber des Knotens. (F1)
8. `native/src/share/core/endpoint_routes.rs`: `EndpointRoutes::start`, `watch_addr`, `online`. (F1)
9. `native/src/share/core/pq_kx.rs`: `share_crypto_provider` mit X25519MLKEM768. (F1)
10. `native/src/share/core/relation_rights_diff.rs`: `authorization_restrictions` (`relation_rights.rs:13`). (F3)
11. `native/src/share/core/fs.rs`, `types.rs` (`ShareCmd`, `ShareEvent`, `ShareStatus`, `DirectGrant`, `RoomProfile`, `ShareAuthState`), `session.rs` (`PeerPrincipal`, `transport_label`, `relation_secret`), `crate::share::fs_access` (`FsAccess`), `crate::share::framing`. (F3)
12. `native/src/share/core/exec_grant_runtime.rs` und der Ort von `ExecGrantTarget`. (F3)
13. Desktop: `native/src/app/core/state.rs` ist nur per Suche gelesen; die Stelle der Texturerzeugung ist nicht gefunden (Lücke: kein Treffer für `TextureHandle`, `load_texture`, `ColorImage`, `TextureOptions` in den beiden Leseliste-Dateien; eine Suche über `native/src` lag außerhalb des Auftrags und wurde nicht ausgeführt). (F5)
14. Desktop: Standardwert `renderer` von `eframe::NativeOptions` in eframe 0.29.1 (Quelle des Crates nicht gelesen). (F5)
15. Desktop: Typ und Darstellung von `transfer_center.window_open` (`frame_update.rs:226`); Typ des `icon_cache` und seine Methode `drain` (`frame_update.rs:107`). (F5)
16. Desktop: die Bedingungsglieder der 50-ms-Stufe, die vor Zeile 200 beginnen (`frame_update.rs`, Zeilen vor 200). (F5)
17. Android: `MainActivity`, `AppRoot` (Sammler der Navigation), die Compose-Bildschirme, `TaskForegroundService` (Klasse und Laufzeittyp), `InitConfig`, `SyncApi`, `WakeKeeper`, die Definition der Ereignisvarianten `CoreEvent`, `api.md` (Pfad nicht ermittelt; genannt in `NativeBridge.kt:6-7`), `smart_explorer::mobile` (Dateien unter `native/src/mobile/`). (F6)
18. `remove_tree_v1`: 0 Treffer in der Leseliste; Fundstelle außerhalb der Leseliste ist nicht ermittelt. (F1)
19. Suchbereich: `native/android-bridge/`, `share-server/` und die übrigen Verzeichnisse der Repository-Wurzel wurden für F7 nicht durchsucht. (F7)

**Widersprüche und veraltete Angaben in Dokumenten**
- A. `docs/refs/iroh-authenticated-lan-paths.md:3` nennt „die fest verwendete `iroh = 1.0.0` API“ (Prüfung 2026-10-03). `native/Cargo.toml:112` und `native/Cargo.lock:3340` führen `iroh` 1.3.0.
- B. `docs/lesungen/2026-09-28-share-host-transport-throughput.md` nennt `node.rs:24` (ALPN, Zeile 173), `node.rs:94` (Exec-ALPN, Zeile 174), `node.rs:96` (Zeile 31), `node.rs:86-91` (Relay-Modus, Zeile 185) und `node.rs:97-99` (`clear_ip_transports`, Zeile 186). Im Stand `da2646d2` stehen diese Stellen bei `node.rs:38`, `152-156`, `158`, `141-146` und `162-164`.
- C. `docs/lesungen/2026-10-01-android-share-host-idle-activity.md` nennt `node.rs:93-108` (Zeile 329) und `node.rs:25,100-102` (Zeile 334) sowie „Iroh 1.0.1“. Im Stand `da2646d2` liegt der Endpoint-Aufbau bei `node.rs:140-166`.
- D. `docs/refs/quic-sftp-throughput.md` nennt „iroh 1.0.1 / noq“ (Zeile 19) und „noq-proto-1.0.1“ (Zeile 64); das Lock führt `noq-proto` 1.3.0 (`Cargo.lock:4406`).
- E. `docs/SHARE_SERVER.md:642-676`: Der Server-Lock führt `iroh-relay` 1.0.0 als lokale Vendor-Kopie; die Kompatibilität mit 1.3.0 ist eine Dokumentaussage vom 2026-10-16 ohne gelesene Quelle.

**Offene Punkte für die Entwurfsphase** (nur benannt, nicht entschieden)
- Datagramm-Standardwerte für die Lock-Version `noq-proto` 1.3.0 und die Einstellungen in `keepalive.rs` (Punkte 1 und 2 oben).
- Stream-Art und Prüfstelle des Exec-Pfads (Punkt 5).
- Ort der Texturerzeugung im Desktop und Standardbackend von eframe 0.29.1 (Punkte 13 und 14).
- Oberflächen, auf denen eine neue Fähigkeit erscheinen müsste (CLI, Desktop-GUI, Android-GUI, Android-Bridge, Daemon-IPC, MCP, Netzprotokoll, Dokumentation): nur die Fundstellen für GUI, Bridge und Netz stehen in diesem Bericht; CLI, MCP und Daemon-IPC sind nicht gelesen.

## 6. Grenzen der Lektüre

**Vollständig gelesen:** `docs/refs/iroh-authenticated-lan-paths.md` (Zeilen 1-24); `native/src/share/core/node_accept.rs` (1-147); `native/src/share/core/node_restrictions.rs` (1-135); `native/src/share/core/exec_terminal.rs` (1-103); `native/src/share/core/exec_frame_reader.rs` (1-101); `native/src/share/core/power.rs` (1-89); `native/src/share/os/shared/host_stream.rs` (1-103); `android/app/src/main/java/app/smartexplorer/android/core/NativeBridge.kt` (1-21); `…/core/CoreEvents.kt` (1-58); `…/ui/AppNav.kt` (1-139); `docs/plaene/2026-10-17-rechte-desktop-tresore/strang-bildschirm.md` (1-35).

**Teilweise gelesen** (Zeilen, Suchtreffer oder Deklarationen): `docs/plaene/2026-10-17-rechte-desktop-tresore/plan.md` (1-62, Abschnitt A); `native/Cargo.toml` (100-116, 233-282; Rest per Suche); `native/Cargo.lock` (Einträge per Suche und die Abhängigkeitsliste von `iroh`); `native/src/share/core/node.rs` (1-40, 125-200, 352-403); `node_sessions.rs` (214-232, 326-392); `node_policy.rs` (1-68); `relation_rights.rs` (1-100; Deklarationen 125-175 per Suche); `exec_protocol.rs` (1-140, 318-345, 371-450; Ordnungsregeln nur per Suche); `service.rs` (20-100; Rest per Suche); `lan_link_transport.rs` (160-200; Rest per Suche); `transport_options.rs` (1-63); `lan_link_dial.rs` (nur Suche); `native/src/lib.rs` (65-137); `frame_update.rs` (Ausschnitte mit Kontext und 200-237); `state.rs` (nur Suche); `Core.kt` (81-89, 97-103, 113-143, 177-205 als Suchausschnitte); `native/android-bridge/src/lib.rs` (114-163; Deklarationen 1-113 per Suche); `BackgroundService.kt` (nur Suche); `AndroidManifest.xml` (nur per Suche; Trefferzeilen 6-151); `docs/refs/android-platform.md` (Suche, Zeilen 21-22 und 53-58); `docs/refs/android-apis.md` (nur Suche); `docs/SHARE_SERVER.md` (1-50, 180-193, 236-268, 319-333, 640-685; Rest per Suche); `docs/lesungen/2026-09-28-share-host-transport-throughput.md` (1-8, 172-212, 285-299; Rest per Suche); `docs/refs/quic-sftp-throughput.md` (19-160, 406-417; Rest per Suche); `docs/lesungen/2026-10-01-android-share-host-idle-activity.md` (327-447; Überschriften und Suche).

**Über den Auftrag hinausgehende Suchen** (Ergebnisse nur als Trefferzeilen verwendet):
- `idle_keepalive_v1` mit dem Glob `native/src/share/core/*.rs` und `native/src/share/os/shared/*.rs`. Treffer außerhalb der Leseliste: `signal_idle.rs:1`, `signal_handshake.rs:184`, `wire.rs:31`, `wire.rs:83`, `wire.rs:93`, `signal_schedule.rs:6`. Diese Dateien wurden nicht geöffnet.
- Erweiterte Begriffsliste für F7 (siehe dort) in `native/src` und `android/app/src`.
- Groß-/Kleinschreibung-unabhängige Zählung von `capture` und `rdev` in `native/src` und `android/app/src`.

**Methode:** Zeilen wurden mit `awk` und `cut` gelesen, Treffer mit `grep -n` (teils mit `-i`, `-w`, `-c`) ermittelt. Die Suche nach Texturbegriffen blieb, wie im Auftrag vorgegeben, auf `frame_update.rs` und `state.rs` beschränkt. Graphify wurde nicht abgefragt (optional laut Auftrag). Es wurde nichts gebaut, getestet, installiert, committet oder gepusht.

**Annahmen dieses Berichts** (im Text einzeln gekennzeichnet): Datagramm-Aktivierung aus `Some(...)` (F2); Semantik von `set_receive_window` auf Verbindungsebene (F1); Zuordnung von `noq` als QUIC-Schicht der Iroh-Linie (F1, belegt durch die Abhängigkeitsliste im Lock).

**Keine Daten der Geräte des Nutzers** wurden übernommen.

