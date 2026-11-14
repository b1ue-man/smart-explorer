# Verbindungen je Zweck zwischen zwei se-Geräten (VPN-Frage)

Stand: 2026-11-14. Planungstiefe 4 (Änderung der Datenwege über Share-Kern, Bildschirm, TCP-Weiterleitung, Transfer, Vendor-QUIC und Bedienwege der Bildschirmverbindung).

## A Erstplan aus dem Nutzerauftrag

Nutzer, 2026-11-14, wörtlich:

- „ist es möglich zwischen zwei direct shares, also zwei se geräten, ein vpn auf zu bauen? nur zwischen den programmen? ich will dass du das herausfindest sowie ob das in streaming und datei transfers zu besseren ergebnissen führen würde.“
- „wenn ein vpn direkt zwischen den geräten laufen kann, auch für sowas wie se, tcp freigaben und was ich auch immer vergessen habe, und das vorteile in sicherheit, verlässlichkeit, geschwindigkeit oder konsistenz bringen kann, wünsche ich mir dass du das ohne weitere anstöße von mir planst, wie du das am idealsten aufbaust um möglichst schnelle konstante verbindungen zu haben, unter recherche was es gibt und eignungfeststellungen, und dann umsetzt.“
- „wenn "ein vpn für alles" nicht geeignet ist, kannst du auch mehrere zwischen den gleichen se aufbauen, basierend auf für was, das ist deine wahl. das ziel ist das jede einzelne komponente optimal funktioniert, nicht das alles das gleiche nutzt und dann alles nicht ideal ist. wenn alloes das gleiche nutzen kann und ideal ist, währe es natürlich auch super.“
- „ich will dass agenten dieses modelks mit dem effort level xhigh die reviews machen“ (dieses Modell: Claude Fable 5.2).
- „ich hätte für bildschirm übertragungen gerne auch die option diese unverschlüsselt laufen zu lassen, in der hoffnung dass sie dann schneller und latenzärmer laufen.“
- „die bildübertragung selbst bedarf auch weiterer optimierung, also wenn du siehst dass da potential ist, mach es“
- Rücknahme der Nachricht 5 (2026-11-14, Wortlaut laut Koordinator, `manager-stand.txt` des Auftrags): zuerst zur LAN-Sperre des Klartext-Bildkanals „um die sperre hab ich nicht gebeten“, danach „ah dann nehme ich es zurück, dann kannst von mir aus auch das unverschlüsselt ganz weg lassen“. Folge: Die unverschlüsselte Bildschirmübertragung entfällt ganz; jede Bildschirmverbindung bleibt wie in v0.5.196 voll verschlüsselt (C4).

Annahme des Nutzers, zu prüfen: „datei transfer, streaming also bildübertragung und so passieren aktuell verschlüsselt. das kostet zeit, ist sicher aber ineffizient in dem ziel die maximale datenmenge, vor allem beim streaming, zu erreichen.“

Weiter geltende frühere Nutzervorgaben (Wortlaut aus dem Auftrag des Koordinators): 2026-11-08 „kein zusätzlicher se command oder s9nsyiger sche8ss“ (der Nutzer führt keine Handgriffe aus); 2026-10-24 „so wenig wie möglich zwischen A und B, server als router nur und relay als fallback“; 2026-10-28 „maximal möglicher Datentransferspeed von Anfang zu Ende“.

Beauftragte Ergebnisse:

1. Antwort: Ist ein VPN nur zwischen zwei se-Programmen möglich?
2. Antwort: Verbessert es Streaming und Dateitransfer?
3. Bei belegtem Vorteil in Sicherheit, Verlässlichkeit, Geschwindigkeit oder Konsistenz: Plan und Umsetzung des idealen Aufbaus, je Komponente optimal.
4. ~~Eine vom Nutzer wählbare unverschlüsselte Bildschirmübertragung (Nachricht 5), mit Messung, die den erhofften Gewinn an Geschwindigkeit und Latenz belegt oder widerlegt.~~ Vom Nutzer am 2026-11-14 zurückgenommen (Wortlaut oben); entfällt samt Messung M5.
5. Optimierung der Bildübertragung, wo Potential belegt ist (Nachricht 6).

Eigene Folgerung (nicht Nutzerwortlaut): Das Ziel „jede einzelne Komponente optimal“ gilt für jede Komponente, die Daten zwischen zwei se-Geräten überträgt; der Nutzer nennt Dateitransfer, Streaming, se, TCP-Freigaben als Beispiele. „VPN“ steht im Auftrag für einen Verbindungsaufbau zwischen den zwei Programmen; ob dessen beste Form ein VPN-Protokoll oder eine andere Verbindungsform ist, entscheidet die Recherche. Die Optimierung der Bildübertragung umfasst jede Station: Aufnahme, Kodierung, Paketierung, Transport, Dekodierung, Anzeige, Eingabe zurück.

Grenzen (frühere Nutzervorgaben und Projektregeln): keine Handgriffe des Nutzers; Share-Server bleibt reiner Resolutionsserver; Daten direkt zwischen den Geräten, Relay nur gekennzeichneter Rückfall; das Relay sieht nie Klartext (`docs/SHARE_SERVER.md` Zeilen 3 bis 8: „the relay sees routing metadata and ciphertext“); keine lokalen Builds oder Tests; ein Release je Batch, gemeinsam mit dem Vorauftrag `docs/plaene/2026-11-08-offene-punkte/`.

## B Zweitplan aus dem Bestand

Korrektur 2026-11-14 nach `bestand.md` (am Quelltext nachgeprüft): Die Aussage „je Gegenstelle genau eine QUIC-Verbindung für alle Übertragungen“ der ersten Lektüre ist falsch. Sitzungen sind je Beziehung und Node-ID geschlüsselt; Bildschirm, LAN-Status, Apps, Exec, TCP-Tunnel und Maschinenanmeldung haben je eigene Verbindungen; Dateiübertragungen dürfen bis zu drei Zusatzverbindungen öffnen (`share/core/node_bulk.rs:33`, `MAX_EXTRA = 3`). Der Bildschirm sendet je Bildgruppe einen unidirektionalen Strom (`screen/core/gop_stream.rs:91`); der Betrachter gibt unidirektionale Ströme zur Laufzeit frei (`screen/core/viewer_client.rs:281`), der von `bestand.md` gemeldete Widerspruch zur Produktionskonfiguration (`keepalive.rs:10`) besteht deshalb nicht. Getrennte Verbindungen je Zweck gibt es also schon; offen ist, wie jede davon geregelt ist.

Erste Lektüre 2026-11-14 (die ersten drei Punkte sind durch die Korrektur oben überholt):

- Je Gegenstelle genau eine QUIC-Verbindung (iroh 1.3.0, noq 1.3.0, TLS 1.3 mit `X25519MLKEM768`) für alle Übertragungen: `docs/plaene/2026-10-28-verbinden-ist-einrichtung/analyse-g2-geraete.md` Punkt 4; `docs/plaene/2026-11-07-sicherheit-direktweg/machbarkeit-unverschluesselt.md` 3.1.
- Ein Verbindungs-Treiber-Task je Verbindung (ein Kern für Paketbau, AEAD, Senden), ein Staukontrollregler (Cubic, `share/core/keepalive.rs`), MTU-Obergrenze 1452 Byte (`vendor/noq-proto-1.3.0/src/config/transport.rs`). Durchsatzgrenze 2–5 Gbit/s am Desktop ist eine Schätzung, keine Messung.
- Bildschirmstrom läuft über einen zuverlässigen bidirektionalen Strom derselben Verbindung (`share/core/node_screen.rs:274`, `open_bi`).
- TCP-Freigaben: `share/core/node_tcp*.rs`, `tcp_forward.rs`, `tcp_wire.rs` (noch nicht gelesen).
- Klartext innerhalb der QUIC-Verbindung ist nicht möglich; ein Klartextkanal wäre ein zweiter Kanal (Machbarkeit 2026-11-07).
- Messung des Verschlüsselungsanteils (`s4_messung_task_*`, Schwelle 10 %) ist im pausierten Batch 2026-11-08 geschrieben, aber noch nie auf Remote-CI gelaufen. Messwerte: keine.

Offen (Bestandsauftrag): siehe `bestand.md`.

### B2 Nachprüfung für C und D (2026-11-14, Quelltext des Arbeitsbaums gelesen)

Kennzeichen in C und D: **[Q]** am Quelltext des Arbeitsbaums am 2026-11-14 bestätigt (Datei:Zeile); **[b]** aus `bestand.md` übernommen, nicht erneut geprüft; **[r]** aus `recherche.md` (Abschnitt bzw. Quelle Q-Nummer); **[S]** Schätzung; **[A]** Annahme.

- Transportwerte [Q]: `keepalive.rs:9-10` (64 bidirektionale, 0 unidirektionale Ströme), `:22-25` (Stromfenster 16 MiB, Verbindungsfenster bis 64 MiB), `:90-100` (Fenster sind Kredit, keine Belegung), `:111-121` (Cubic; BBRv3 nur über `SE_SHARE_QUIC_CC`), `:127-146` (eine Konfiguration für den ganzen Endpunkt, mit `application_path_policy`).
- Die Konfiguration wirkt heute je Endpunkt, nicht je Zweck [Q]: Alle Verbindungsaufbauten rufen `endpoint.connect(…)` ohne eigene Transportkonfiguration auf (`node_sessions.rs:388`, `node_routes.rs:61`, `node_apps_link.rs:46`, `node_screen_link.rs:57` und `:82`, `os/shared/lan_link_dial.rs:87`); nur TCP nutzt `connect_with_opts` (`node_tcp_link.rs:89`), ebenfalls ohne Transportkonfiguration. Die Annahme verwendet die Standardkonfiguration (`node_accept.rs:58`, `incoming` als Future). Laufzeitanpassungen gibt es nur für unidirektionale Ströme und Fenster (`viewer_client.rs:281`, `os/shared/peer_feed_link.rs:109`, `os/shared/lan_link_transport.rs:188-190`).
- iroh 1.3.0 erlaubt je Verbindung eine eigene Transportkonfiguration [Q]: Client `ConnectOptions::with_transport_config` (`vendor/iroh-1.3.0/src/endpoint.rs:1839-1856`, Verwendung `:1179-1192`); Annahme `Incoming::accept_with(Arc<ServerConfig>)` (`vendor/iroh-1.3.0/src/endpoint/connection.rs:167-174`) mit `Endpoint::create_server_config_builder` (`endpoint.rs:1815-1818`); die ALPN eines eingehenden Initial-Pakets ist nur „best effort“ lesbar (`Incoming::decrypt`, `connection.rs:229`; `DecryptedInitial::alpns`, `vendor/noq-proto-1.3.0/src/endpoint.rs:1329-1334`: „not guaranteed to succeed if the ClientHello spans multiple packets“).
- Staukontrolle [Q]: Der Regler ist je Pfad ein `Box<dyn Controller>` aus einer `ControllerFactory` (`vendor/noq-proto-1.3.0/src/congestion.rs:17-153`); `on_ack` erhält den `RttEstimator` (`congestion.rs:37-45`); ein Wechsel des Reglers zur Laufzeit ist nicht vorgesehen (`noq-1.3.0/src/connection.rs:856`, nur Lesen).
- Treiber [Q]: je Verbindung ein `ConnectionDriver` (`vendor/noq-1.3.0/src/connection.rs:79`, `:262`), ein `EndpointDriver` für das Empfangen und Verteilen (`vendor/noq-1.3.0/src/endpoint.rs:180`, `:489`, `:1034-1043`). Sendebündel sind Konstanten: `MAX_TRANSMIT_DATAGRAMS = 20`, `MAX_TRANSMIT_SEGMENTS = 10` (`connection.rs:2123`, `:2130`, Verwendung `:1709`, `:1757`). Das Relay fasst Segmente zu einem Relay-Paket von höchstens 64 KiB zusammen (`vendor/iroh-relay-1.0.0/src/protos/relay.rs:23`, `:212-247`).
- Strompriorität [Q]: `SendStream::set_priority(i32)` (`vendor/noq-1.3.0/src/send_stream.rs:222-234`); im Projekt nirgends benutzt (Suche `set_priority` über `native/crates/engine/src`, `native/crates/transfer/src`: kein Treffer).
- Bulk-Verbindungen [Q]: `node_bulk.rs:33-46` (höchstens 3 zusätzliche, Auslöser Verlust ≥ 1 % bei ≥ 1000 Datagrammen oder Rate ≥ 250 MiB/s, Kommentar „estimate, Z-P6“), `:118-121` und `:141-146` (nie über das Relay).
- Bildschirm [Q]: Gastgeber öffnet den Steuerstrom (`node_screen.rs:274`); Bildgruppen auf unidirektionalen Strömen (`gop_stream.rs:1-8`, `:91`), höchstens 4 beim Betrachter (`gop_stream.rs:27`); Taktgeber, Warteschlangenwächter, Reset veralteter Ströme (`pacing.rs:1-27`); verzögerungs- und verlustbasierte Bitrate nach GCC über Cubic (`rate_control.rs:1-11`, `:19-47`); Teilung des Sendekredits je Gegenstelle (`sender_budget.rs:1`). Kodierer: Media Foundation mit `CODECAPI_AVLowLatencyMode`, CBR, ohne B-Bilder (`screen/os/windows/mf_media.rs:190-204`); OpenH264 mit `UsageType::ScreenContentRealTime`, `Complexity::Low` (`screen/os/native_ffi/openh264_encode.rs:124-130`); Android `KEY_LATENCY` 1, `KEY_PRIORITY` 0, `KEY_LOW_LATENCY` (`android/…/screen/ScreenCodecLatency.kt:20-35`, `ScreenEncoder.kt:375-378`). Auf Linux gibt es nur OpenH264 (Software); kein VA-API, kein NVENC (Suche „vaapi|libva|nvenc“: kein Treffer). Aufnahme: DXGI mit `GetFrameDirtyRects` (`screen/os/windows/capture_dxgi.rs:282`); unveränderte Bilder werden nicht kodiert (`screen/os/shared/host_worker_produce.rs:9-15`, `StageFrame::Unchanged`); PipeWire ohne DMA-BUF (Suche „dmabuf“: kein Treffer). Zeiger: unter X11 in das CPU-Bild gemischt (`screen/os/linux_os/x11_cursor.rs:1-4`), unter Windows über `SetIsCursorCaptureEnabled(true)` im Bild (`screen/os/windows/capture_retirement.rs:77`). Anzeige: wgpu `AutoVsync` mit einem Bild Latenz, Option „Niedrigste Latenz“ = `AutoNoVsync` ab dem nächsten Start; `Mailbox` bewusst nie gesetzt, weil eframe die Fläche nicht vorher fragt (`gui/src/app/core/screen_viewer_app.rs:1-8`, `:49-60`).
- TCP-Freigaben [Q]: Kopierpuffer 8 KiB je Richtung (`tcp_forward.rs:28-32`); jede Leseoperation und jede Schreiboperation läuft einzeln durch `route_protocol::run_with_route` mit Routenabo, Zeitgeber und `connection.closed()` (`os/shared/tcp_pump.rs:53-87`, `route_protocol.rs:99-139`).
- Exec [Q]: je Auftrag eine eigene Verbindung (`node_routes.rs:61`), Abschluss mit `close` nach dem Auftrag (`exec_client.rs:209`); der Gastgeber öffnet den Strom (`exec_admission.rs:51`).
- Verschlüsselung [Q]: ring-Anbieter mit `X25519MLKEM768` als erster Gruppe (`share/core/pq_kx.rs:49-62`); die 1-RTT-Suite stellt die S4-Messung zur Laufzeit fest (`s4_messung_support.rs:75`, `negotiated_aead`). Der TLS-Exporter ist an der Verbindung verfügbar (`Connection::export_keying_material` in `vendor/iroh-1.3.0/src/endpoint/connection.rs`) und im Projekt bisher unbenutzt (Suche: kein Treffer außerhalb `vendor/`).
- noq-proto-Kryptoschnittstelle [Q]: `crypto::Session::write_handshake` und `next_1rtt_keys` liefern `PacketKey` (`vendor/noq-proto-1.3.0/src/crypto.rs:30`, `:76-79`, `:157-174`); iroh baut `QuicClientConfig`/`QuicServerConfig` aus rustls in `vendor/iroh-1.3.0/src/tls.rs:96-155`.
- Messmittel [Q]: Netem-Profile `lan`, `wan`, `fat`, `loss`, `slow` und die Netem-Einrichtung (`native/rdt_task/speed_net.py:26-33`, `:55-120`); S4-Teil der Suite (`native/test-crates-task.py:972-993`, `.github/workflows/crates-task.yml:290-340`); `se screen bench` mit Latenz p50/p95/max, Bitrate, Schlüsselbildern und Erholzeiten (`screen/core/bench.rs:1-11`, `docs/oberflaechen/bildschirm.md` Abschnitt 3).
- Messwerte an se [Q, CI-Protokolle über die GitHub-API gelesen am 2026-11-14]: Kein Lauf hat bisher Share-Durchsatz gemessen. Die Fälle `speed_task_share_*` der Suite `rechte-desktop-tresore-task.yml` wurden in den Läufen 39677969206, 39659786027, 39644649820 als „nicht ausgeführt“ (Nutzerentscheidung 2026-10-21) und in Lauf 39607246651 als „Bau gescheitert“ protokolliert. Der letzte Lauf von `crates-task.yml` (42439947911, Commit `c189077e`, 2026-11-08) enthält den Teil `s4` noch nicht; `part_s4` steht nur im Arbeitsbaum.
- Befund ohne Änderung: `recherche.md` Abschnitt 1 (fünfter Punkt) und Abschnitt 8.1 (C1, C4) beschreiben noch „eine Verbindung je Gegenstelle“ und den Bildschirm „auf einem zuverlässigen bidirektionalen Strom“. Beides ist am Quelltext widerlegt (oben). Die Bewertungen in `recherche.md` 8.3 bleiben als Literaturbefunde gültig; ihre Ausgangslage ersetzt dieser Abschnitt. `recherche.md` wird in diesem Auftrag nicht geändert.

## C Entscheidungsplan

### C0 Ausgangslage, aus der jede Entscheidung folgt

1. Getrennte Verbindungen je Zweck bestehen schon (B2). Sie teilen aber eine Transportkonfiguration: denselben Regler (Cubic), dieselben Fenster und dieselben Sendebündel. Getrennte Verbindungen trennen Fenster und Reglerzustand, nicht die Warteschlange im Engpass [r 8.3 „Bildschirmstrom“, F aus Q29, Q31].
2. Ein verlustbasierter Regler (Cubic) einer Massenübertragung im selben Engpass verdrängt einen verzögerungsbasierten Bildregler (Q42, Q43 [r 6.1]). Der Bildregler von se ist verzögerungsbasiert nach GCC (`rate_control.rs:1-11` [Q]). Eine Dateiübertragung zur selben oder einer anderen Gegenstelle über denselben Zugang drückt deshalb die Bildrate und erhöht die Bildlatenz [F]. Gemessen an se ist das nicht (B2: keine Messwerte).
3. Die Grenze des Durchsatzes einer QUIC-Verbindung ist die Paketverarbeitung eines Kerns, nicht die Verschlüsselung: Q01 („more time is being spent sending UDP packets than encrypting their payloads“), Q32 (Empfängerverarbeitung, ACKs), Q33 (ein Sender-Kern bei 100 %) [r 4.2, 5.1]. Einkern-AES-GCM liegt bei 8,5 bis 29,8 Gbit/s (Q26 [r 4.1]).

### C1 Antwort auf die VPN-Frage

1. **Ist ein VPN nur zwischen zwei se-Programmen möglich?** Ja. WireGuard als Bibliothek mit eingebettetem TCP/IP-Stack läuft ohne Adminrechte und ohne Systemadapter (onetun mit smoltcp und boringtun, Q11; Tailscale `tsnet` mit gVisor, Q14 [r 2]). Mit Systemadapter verlangt es Administrator (Windows, Q21), `CAP_NET_ADMIN` (Linux [A], r 3) und auf Android einen Systemdialog bei genau einem aktiven VPN je Nutzer (Q22, Q23 [r 3]).
2. **Hat se das schon in anderer Form?** Ja. Zwischen zwei se-Programmen laufen heute gegenseitig authentifizierte (iroh-Geräteschlüssel; Beziehungsnachweis per HMAC, `node_sessions.rs:358-368` [b]), mit TLS 1.3 und hybridem `X25519MLKEM768` verschlüsselte (`pq_kx.rs:49-62` [Q]) QUIC-Verbindungen, getrennt je Zweck über eigene ALPN (`node_accept.rs:82-85` [Q], `node.rs:160-168` [b]). Die TCP-Freigaben leisten, was ein Programm-VPN für fremde Programme leistet (lokaler Port, ein QUIC-Strom je TCP-Verbindung, `tcp_forward.rs:1-2`, `tcp_pump.rs:1-3` [Q]); das ist dieselbe Bauform wie onetun und `tsnet` (Q11, Q14), aber ohne Userspace-TCP und ohne TCP-in-TCP (Q54, Q06, Q49 [r 7]).
3. **Wird Streaming oder Dateitransfer mit einem VPN besser?** Nach den Quellen nein, in keinem der vier Kriterien:
   - Geschwindigkeit: Läuft QUIC durch WireGuard, kommt eine zweite AEAD-Schicht hinzu; ersetzt WireGuard QUIC, ersetzt ein Userspace-TCP ohne SACK (smoltcp, Q12) die QUIC-Verlustbehandlung [r 4 Befund 1, 2.3]. Die Grenze liegt in der Paketverarbeitung (C0 Punkt 3).
   - Konsistenz beim Streaming: WireGuard hat weder Staukontrolle noch Fehlerkorrektur (Q06 [r 8.4]); genau das sind die Mittel der Streaming-Referenzen (Moonlight, Parsec, RDP-UDP, WebRTC [r 6.8]).
   - Verlässlichkeit: kein Gewinn belegt; Android verliert das VPN des Nutzers oder wird von ihm verdrängt (Q22, Q23).
   - Sicherheit: se hat bereits Ende-zu-Ende-Verschlüsselung mit Geräteidentität; ein VPN brächte eine zweite Identitäts- und Schlüsselebene (r 2.4, Folgerung).
4. **Ein VPN für alles oder mehrere?** Mehrere Verbindungen je Zweck, wie heute, aber künftig **je Zweck eigen geregelt** (C2 E1 bis E7). Das ist die belegte Form „jede einzelne Komponente optimal“ (r 8.4): Massenübertragung durchsatzorientiert, aber bei laufender interaktiver Sitzung nachgiebig; Bild verzögerungsbasiert und vorrangig; Steuerströme vor Übertragungsströmen.
5. **Zur Nutzerannahme „Verschlüsselung kostet Zeit“:** Die Quellen stützen sie nicht (C0 Punkt 3); die S4-Messung des Vorauftrags (E30 in `docs/plaene/2026-11-08-offene-punkte/plan.md`) liefert die Zahl je Plattform. Die zunächst gewünschte unverschlüsselte Bildschirmübertragung hat der Nutzer am 2026-11-14 zurückgenommen (A, C4); sie wird nicht gebaut.

### C2 Querschnittsentscheidungen

Standard-Ausschlussgründe, auf die die Komponenten in C3 verweisen:

- **X-VPN** (C5 WireGuard im Programm, C9 `tsnet`): zweite AEAD-Schicht oder Userspace-TCP ohne SACK (Q12); keine Staukontrolle und keine Fehlerkorrektur (Q06); Messwerte für Windows und Android fehlen [r 2, 9 Punkt 2]; `tsnet` verlangt eine Go-Laufzeit mit FFI, Ursache von über 85 % der Android-Absturzberichte bei Mullvad (Q10 [r 2.2]).
- **X-ADAPTER** (C6): Administrator, Treiber, Android-Dialog und „only one VPN“ (Q21, Q22, Q23); widerspricht „kein zusätzlicher se command oder s9nsyiger sche8ss“ (2026-11-08).
- **X-OVERLAY** (C7 Nebula, C8 ZeroTier): zweite Identitäts- und Adressebene neben den iroh-Schlüsseln (r 2.4); libzt unter BSL 1.1 (Q17).
- **X-WEBRTC** (C10): zweiter ICE-, DTLS- und SRTP-Stack neben iroh, Rust-Reife ungeklärt; GCC unterliegt demselben Verdrängungsproblem (Q42, Q43 [r 8.3]).
- **X-EIGEN** (C11 eigenes UDP-Protokoll mit eigenem Verbindungsaufbau): Referenzcode GPL-3.0 (r 6.2); ein eigener UDP-Socket neben iroh braucht eigene NAT-Durchdringung und auf Windows eine neue Firewallfreigabe für eingehende Pakete [A: Verhalten der Windows-Firewall bei einem zweiten Socket nicht geprüft]; widerspricht „keine Handgriffe“.
- **X-TCP** (C12 Kernel-TCP mit TLS): Vorteil nur oberhalb von etwa 5 Gbit/s belegt (Q33 [r 8.3]); Kopfblockade und Latenzspitzen bei Verlust (Q45); Neubewertung nach M1 (E5).
- **X-MOQ** (C13): Entwurf, kein RFC (Q53).
- **X-MASQUE** (C14): verschachtelte Staukontrolle (Q55, Q56); Nutzen nur für UDP-Weiterleitung, die se nicht anbietet.

**E1 Zweckprofil je Verbindung.** Jede Verbindung eines se-Endpunkts bekommt beim Aufbau ein Zweckprofil (`Files`, `FilesBulk`, `Screen`, `Feed`, `Exec`, `Apps`, `Tcp`, `LanLink`). Ergebnisanforderungen:
- Es gibt genau eine Funktion, die aus dem Zweck die Transportkonfiguration baut; sie übernimmt alle heutigen Werte aus `iroh_transport_config_for_routes` unverändert (Keepalive 5 s, Leerlauf 20 s, Ströme 64/0, Fenster, `application_path_policy(true)`) und ändert je Zweck nur Regler und Sendebündel.
- Ausgehend setzt jede Aufbaustelle aus B2 das Profil über `ConnectOptions::with_transport_config`.
- Eingehend nimmt der Annahmepfad jede Verbindung mit `Incoming::accept_with` und einer eigenen Serverkonfiguration an. Weil die ALPN vor dem Handshake nur bestmöglich lesbar ist (B2), trägt der Regler einen **spät gebundenen Zweck**: Die Fabrik der Verbindung hält einen eigenen Zweckplatz; der Annahmepfad setzt ihn unmittelbar nach dem Handshake aus `connection.alpn()`. Bis dahin verhält sich der Regler wie Cubic heute.
- Die Werte `SE_SHARE_QUIC_CC` bleiben ein Messschalter mit unverändertem Verhalten (`keepalive.rs:102-121`).
- Beleg: B2 (iroh-Verträge), r 8.2 Zeile C2. Verworfen: (a) Regler über eine globale Fabrik ohne Verbindungsbezug, weil der Regler den Zweck nicht kennt (`ControllerFactory::build(now, mtu)`, `congestion.rs:152` [Q]); (b) Zweck allein aus `Incoming::decrypt().alpns()`, weil das bei mehrteiligem ClientHello scheitert (`noq-proto endpoint.rs:1329-1334` [Q]) und der hybride Schlüsselaustausch das ClientHello über 1200 Byte bringt [A: Größe nicht gemessen]; (c) eigener Endpunkt je Zweck, weil jeder Endpunkt eigene Relay-Anmeldung, Adressermittlung und Lochstanzung bräuchte (iroh-Endpunkt je Prozess, `node.rs:145-203` [b]).

**E2 Nachgiebige Massenübertragung bei interaktiver Nutzung.** Verbindungen der Profile `Files` und `FilesBulk` nutzen einen Regler „Cubic mit Rückzug“: Solange im Prozess eine interaktive Sitzung läuft (Bildschirm als Gastgeber oder Betrachter, Exec mit Terminal, TCP-Tunnel mit aktiver Pumpe), begrenzt er sein Fenster so, dass die gemessene Warteschlangenverzögerung das Ziel T nicht übersteigt; ohne interaktive Sitzung ist er Cubic wie heute. Ergebnisanforderungen nach RFC 6817 (LEDBAT, abgerufen 2026-11-14, https://www.rfc-editor.org/rfc/rfc6817.html; ergänzende Quelle, nicht in `recherche.md`):
- Ziel der zusätzlichen Warteschlangenverzögerung T höchstens 100 ms (RFC 6817 §2.5 „TARGET MUST be 100 milliseconds or less“); Wert aus M2, Kandidaten 15, 25 und 50 ms [S].
- Grundverzögerung als Minimum über eine Historie von Minutenminima (RFC 6817 §2.4.2, §2.5 BASE_HISTORY 10).
- Nie aggressiver als Cubic: Fenster = Minimum aus Cubic-Fenster und Rückzugsfenster (RFC 6817 §4.2).
- Der Zustand „interaktiv aktiv“ gilt prozessweit, weil der gemeinsame Engpass meist der eigene Zugang ist (WLAN, Uplink) und damit auch Übertragungen zu anderen Gegenstellen den Bildstrom treffen [F].
- Während „interaktiv aktiv“ wächst die Zahl der Bulk-Verbindungen nicht (`node_bulk.rs:125-127`), weil mehrere Regler zusammen aggressiver sind (Kommentar `node_bulk.rs:36-37` „like iperf3 -P“ [Q]).
- Übernommen zusätzlich: Glättung der aktuellen Verzögerung über mehrere Proben statt Einzelwert, weil `latest_rtt` je ACK schwankt [S; der Parameter CURRENT_FILTER aus RFC 6817 ist in diesem Auftrag nicht nachgelesen, A].
- Nicht übernommen aus RFC 6817: Einweglaufzeit mit Zeitstempeln (QUIC liefert nur die Rundlaufzeit über `RttEstimator`; Grenze: Rückweg-Warteschlangen erscheinen als Verzögerung, im Messbericht M2 als bekannte Grenze geführt).
- Verworfen: (a) BBRv3 für alle (experimentell, Q43 Bandbreitenüberschätzung in tiefen Puffern [r 5.1]; bleibt Messschalter); (b) Ratenbegrenzung der Massenübertragung auf Anwendungsebene je Gegenstelle (wirkt nur, wenn die Rate bekannt ist; der Regler sieht die Verzögerung direkt); (c) Rückzug immer, auch ohne interaktive Sitzung (gibt gegen fremde TCP-Flüsse nach, RFC 6817 §1.2 Ziel 3, und widerspricht „maximal möglicher Datentransferspeed“ 2026-10-28); (d) eigene Verbindung allein ohne Reglerwechsel (beseitigt die gemeinsame Warteschlange nicht, C0 Punkt 1).

**E3 Interaktive Präsenz.** Ein prozessweites Register zählt laufende interaktive Sitzungen; jede Sitzung hält einen Eintrag, solange sie läuft (Freigabe beim Ende, auch bei Abbruch). Eingetragen werden: Gastgeber-Bildpipeline, Betrachter-Bildleser (nur Sitzungsarten `view` und `control`, nicht `probe`, `preview` und Metadaten), Exec-Aufträge mit Terminal auf beiden Seiten, jede laufende TCP-Pumpe. Planfestlegung: Alle TCP-Tunnel zählen als interaktiv, weil die Tunnel aus „Bildschirm steuern: Erlauben“ entstehen (Fernzugriff, RDP, `os/shared/auto_forward.rs:1-10` [Q]) und eine Erkennung „Massendienst im Tunnel“ ohne Protokollwissen nicht belegbar ist. Verworfen: Erkennung über den Zielport (bekannt ist nur der RDP-Port; jeder freigegebene Dienst auf einem anderen Port fiele heraus).

**E4 Strompriorität.** Übertragungsströme der Transfer-Engine (`open_transfer_stream_until`, `node_bulk.rs:104-155`) bekommen Priorität −1; alle anderen Ströme der Dateisitzung (Listen, Baumwalk, Hash-Walk, Watch, Schnappschuss, Leases, Reparatur, Räume) behalten 0. Auf der Bildschirmverbindung erhält der Steuerstrom beim Gastgeber Priorität 1 über den Bildgruppen (0), damit `PONG`, Antworten und Zustandsrahmen nicht hinter Bilddaten warten; die Uhrensynchronisierung `PING`/`PONG` speist die Latenzmessung (`gop_stream.rs:7-8` [Q]). Beleg: `set_priority` (B2), RFC 9000 §2 (Ströme unabhängig, Q30). Nicht übernommen: viele Prioritätsstufen (noq-Dokumentation: „Using many different priority levels per connection may have a negative impact on performance“, `send_stream.rs:228-229` [Q]). Messung M3.

**E5 Sendebündel und Bulk-Schwelle.** Vendor-Patch an noq: `MAX_TRANSMIT_SEGMENTS` und `MAX_TRANSMIT_DATAGRAMS` werden Werte der Transportkonfiguration mit unveränderten Vorgaben 10 und 20 (Verfahren wie der Upstream-PR #645, Q36 [r 5.1]). Profil `Files`, `FilesBulk` und `Tcp` setzen 40 und 80 [S: Wert aus Q36; Entscheidung durch M1]; `Screen`, `Feed`, `Exec`, `Apps`, `LanLink` behalten 10 und 20, weil größere Bündel Bursts erzeugen [S]. Grenze: 40 × 1452 Byte = 58 080 Byte bleibt unter der Relay-Paketgrenze 64 KiB (B2). `DRIVER_RATE` in `node_bulk.rs:43` wird nach M1 auf 80 % der gemessenen Einzelverbindungsrate der langsamsten gemessenen Plattform gesetzt [Planfestlegung]. Bei Ergebnis von M1 „vier Verbindungen nicht mindestens 15 % schneller als eine auf keiner Plattform“ bleibt `MAX_EXTRA` 3, weil Verlustfall-Nutzen (Auslöser `LOSS_SHARE`) davon unberührt ist; bei „vier Verbindungen auf Loopback unter 2,5 Gbit/s auf Linux x86_64“ wird X-TCP in einem eigenen Planabschnitt neu bewertet. Verworfen: Erhöhung der MTU-Obergrenze über 1452 Byte (Internet und WLAN bieten 1500 Byte [r 5.1 F]; Jumbo-Frames im Heimnetz [A] selten; IPv4-Gewinn 1472 statt 1452 Byte rund 1,4 % [S]); Multipath-Nutzung mehrerer Pfade gleichzeitig (widerspricht der Routenregel „ein zugelassener Pfad trägt Nutzdaten“, `vendor/SMART_EXPLORER_ROUTE_POLICY.md` Zeilen 13 bis 18 [b]; noq „still young“, Q40).

**E6 ACK-Frequenz.** noq bietet `ack_frequency_config` (`vendor/noq-proto-1.3.0/src/config/transport.rs:268` [Q]). Q32 nennt ACK-Verarbeitung als Ursache des QUIC-Rückstands [r 5.2]. Gebaut wird nur der Messschalter (Testunterstützung) in M1; eingeschaltet für `Files`, `FilesBulk`, `Tcp`, wenn M1 auf mindestens zwei Plattformen ≥ 10 % mehr Durchsatz oder ≥ 10 % weniger CPU-Zeit je GiB zeigt und M2 mit Rückzug weiterhin besteht [Planfestlegung] (Paket B5).

**E7 TCP-Pumpe.** Ergebnisanforderungen: (a) eine Pumpe abonniert den Routenstatus einmal je Pumpe und pausiert bei Routenwechsel wie heute (P28-Verhalten aus `tcp_pump.rs:2-3` bleibt), statt je Lese- und Schreibaufruf einen neuen Zeitgeber und ein neues Abo zu erzeugen; (b) Kopierpuffer wachsen von 8 KiB auf 64 KiB, sobald ein Lesevorgang den Puffer füllt, gedeckelt durch ein prozessweites Pufferbudget von 16 MiB (heutiger Höchstwert, `tcp_forward.rs:28-29`); ohne freies Budget bleibt der Strom bei 8 KiB. Beleg: Aufrufkette in B2 [Q]; Wirkung [S], Entscheidung durch M4 (Bedingung: mindestens 20 % mehr Durchsatz auf Loopback auf Linux x86_64 oder mindestens 20 % weniger CPU-Zeit je GiB; sonst bleibt (b) bei 8 KiB, (a) bleibt wegen der Gleichwertigkeit des Verhaltens). Verworfen: TCP-Tunnel über VPN (X-VPN), UDP-Weiterleitung (kein Bedarf).

**E8 Klartext-Bildkanal (Nutzervorgabe 5): vom Nutzer am 2026-11-14 zurückgenommen; entfällt (A, C4).**

**E9 Bildübertragung je Station (Nutzervorgabe 6), Kurzform; Ausarbeitung in C5.**

**E10 Exec- und Apps-Verbindung je Auftrag.** Heute ein Handshake je Auftrag (B2). Gebaut wird nur die Messung M8 (Zeit bis zum ersten Ausgabebyte, Anteil des Verbindungsaufbaus). Bedingung für Paket B3 (Wiederverwendung einer Exec-Verbindung für mehrere Aufträge, Protokollversion `share-exec/3` mit Aushandlung): Verbindungsaufbau ≥ 50 % der Zeit bis zum ersten Byte auf dem Profil `wan` [Planfestlegung]. Für Apps keine Wiederverwendung: Befehle sind selten (ein Befehl je Nutzeraktion, `node_apps_client.rs:1` [b]) [S].

**E11 Bildstrom auf Datagrammen mit FEC (r 8.3 C3).** Nicht jetzt gebaut; heute läuft bereits die zweitbeste belegte Form C4 (Strom je Bildgruppe, Reset veralteter Ströme, B2). Bedingung für Paket B1 aus M7 (Profil `loss` und eigenes Profil `loss1`: 1 % Verlust, 40 ms RTT, 100 Mbit/s): p99 der Bildlatenz liegt mehr als 2 × RTT über p50 oder es gibt mehr als einen Stillstand über 200 ms je Minute [Planfestlegung]. Grund der Zurückstellung: Ströme wiederholen verlorene Pakete nach einer RTT; im LAN (RTT wenige ms) ist der Abstand zu FEC klein [F aus RFC 9002, Q29], im WAN nicht gemessen [r 9 Punkt 13].

**E12 Verschlüsselung der übrigen Komponenten.** Unverändert verschlüsselt. Die Klartextfrage für Dateien entscheidet weiter E30 des Vorauftrags mit der S4-Messung (Schwelle 10 %); dieser Plan ändert E30 nicht.

### C3 Je Komponente (alle 18 Zeilen aus `bestand.md` Abschnitt 1)

Spalten: heute / Grenze des knappsten Mittels / betrachtete Lösungen (Kandidaten aus `recherche.md` 8.1) / gewählt mit Beleg / verworfen mit Grund.

1. **Dateisystem-Anfragen** (`share-fs/3`).
   - Heute: eine Sitzungsverbindung je Beziehung und Gegenstelle, ein bidirektionaler Strom je Anfrage, 64 Ströme mit 4 Steuerreserve (`keepalive.rs:9-17` [Q]), Cubic, Fenster 16/16–64 MiB [Q].
   - Grenze: Rundlaufzeit je Anfrage und Warteschlange im Engpass während paralleler Übertragungen [F].
   - Betrachtet: C1, C2, C5, C6, C7, C8, C9, C12.
   - Gewählt: C2 mit E1 (Profil `Files`), E2 (Rückzug), E4 (Steuerströme vor Übertragungen). Beleg: r 8.3 „Viele kleine Dateien“; `set_priority` (B2).
   - Verworfen: C5, C9 (X-VPN); C6 (X-ADAPTER); C7, C8 (X-OVERLAY); C12 (X-TCP, kein Strommultiplex ohne eigenes Protokoll, r 8.3).
2. **Verzeichnislisten und Baumwalk.**
   - Heute: Strom auf der Sitzungsverbindung, Stapel bis 2048 Einträge [b].
   - Grenze: Antwortzeit beim Blättern während einer Übertragung [F].
   - Betrachtet: C1, C2; eigene Verbindung nur für Steuerung.
   - Gewählt: E4 (Priorität 0 über Übertragungen −1) und E2. Beleg wie 1. Messung M3.
   - Verworfen: eigene Steuerverbindung je Gegenstelle (zweiter Handshake und zweite Routenentscheidung je Beziehung; Gewinn gegenüber Priorität plus Rückzug nicht belegt, Entscheidung durch M3: liegt die Listenlatenz mit E2 und E4 während einer Übertragung über 2 × RTT + 50 ms, wird die Steuerverbindung in einem Planabschnitt nachgetragen [Planfestlegung]); X-VPN, X-ADAPTER, X-OVERLAY.
3. **Sync-Abgleich (Hash-Walk).** Wie 2 (Steuerpriorität 0, Profil `Files`). Grenze: Rundlaufzeit und Dateisystem des Gastgebers (r 8.3 „Kosten im Namensraum“, L3). Verworfen wie 2.
4. **Übertragung (Transfer-Engine).**
   - Heute: bis 56 parallele Ströme je Verbindung, Bulk-Verbindungen (5), Cubic [b/Q].
   - Grenze: Paketverarbeitung eines Kerns je Verbindung (Q33) und die Leitungsrate.
   - Betrachtet: C1, C2, C5, C6, C7, C8, C9, C10, C11, C12, C13, C14.
   - Gewählt: C2 mit E2, E4 (−1), E5 (Sendebündel 40/80), E6 (bedingt). Beleg: Q36, Q37 (größere Bündel +14 bis +28 %), Q33 (Kernbindung), RFC 6817.
   - Verworfen: X-VPN, X-ADAPTER, X-OVERLAY, X-WEBRTC (kein Massentransport), X-EIGEN, X-TCP (Neubewertung nach M1), X-MOQ, X-MASQUE.
5. **Bulk-Verbindungen.**
   - Heute: bis 3 zusätzliche Verbindungen, Schwellen geschätzt (`node_bulk.rs:33-46` [Q]).
   - Grenze: Kern je Verbindung; Verlustteilung je Regler.
   - Betrachtet: C2 (bestehend), mehr oder weniger Verbindungen, gemessene Schwelle.
   - Gewählt: unverändertes Verfahren mit Profil `FilesBulk`, gemessener `DRIVER_RATE` (E5), kein Wachstum bei interaktiver Präsenz (E2). Beleg: Q33, ESnet „4-8 streams“ (L3 [r 5.1]).
   - Verworfen: Erhöhung von `MAX_EXTRA` ohne Messung (keine Quelle misst mehrere QUIC-Verbindungen gegen eine, r 9 Punkt 9).
6. **Gerätefeed (Metadaten, `share-screen/1`).**
   - Heute: eigene Metadatensitzung je Gegenstelle, unidirektionale Ströme 0 (`peer_feed_link.rs:109` [Q]).
   - Grenze: keine Durchsatzgrenze; kleine Nachrichten [F].
   - Betrachtet: C1, C2.
   - Gewählt: Profil `Feed` (Cubic, Bündel 10/20), sonst unverändert. Beleg: E1.
   - Verworfen: Zusammenlegung mit der Bildverbindung (Bildverbindung entsteht nur bei offener Ansicht; der Feed läuft dauerhaft, `peer_feed.rs:1-9` [b]).
7. **LAN-Statuskanal (`paired-link/1`).**
   - Heute: eigene Verbindung, 1 bidirektionaler Strom, Empfangsfenster 16 KiB (`lan_link_transport.rs:188-190` [Q]).
   - Grenze: keine; Runden alle 2 s [b].
   - Betrachtet: C1, C2.
   - Gewählt: Profil `LanLink`, sonst unverändert.
   - Verworfen: jede Änderung des Takts (kein Befund).
8. **Bildschirmübertragung mit Eingabe.**
   - Heute: eigene Verbindung je Betrachter, Bildgruppen auf unidirektionalen Strömen, GCC-artige Bitrate über Cubic, Steuerstrom mit Eingabe (B2).
   - Grenze: Latenz an der Engpass-Warteschlange (C0 Punkt 2) und Stationszeiten (C5).
   - Betrachtet: C1, C2, C3, C4 (Bestand), C5, C6, C7, C8, C9, C10, C11, C12, C13.
   - Gewählt: C4 bleibt; Profil `Screen`; E2 schützt den Bildstrom vor Massenübertragungen; E4 Steuerstrom zuerst; Stationsverbesserungen (C5); C3 bedingt (E11). Beleg: r 6.8, Q42, Q43, RFC 6817.
   - Verworfen: X-VPN, X-ADAPTER, X-OVERLAY, X-WEBRTC, X-EIGEN, X-TCP (Parsec verzichtet wegen Latenzspitzen auf TCP, Q45), X-MOQ.
9. **Exec, Terminal (`share-exec/2`).**
   - Heute: eine Verbindung je Auftrag (B2).
   - Grenze: Verbindungsaufbau je Auftrag (Rundlaufzeiten) [S].
   - Betrachtet: C1, C2, Wiederverwendung.
   - Gewählt: Profil `Exec`; Terminal zählt als interaktiv (E3); Wiederverwendung bedingt (E10). Beleg: r 8.3 „Interaktive Befehle“ (schwach, keine Messung).
   - Verworfen: X-VPN, X-ADAPTER, X-OVERLAY, X-TCP (Kernel-TCP bei Verlust).
10. **Geräte-Befehle (Apps, `share-apps/1`).**
    - Heute: Verbindung je Befehl mit 30 s Frist (`node_apps_link.rs:46` [Q]).
    - Grenze: Verbindungsaufbau [S].
    - Betrachtet: C1, C2, Wiederverwendung.
    - Gewählt: Profil `Apps`; sonst unverändert (E10).
    - Verworfen: Wiederverwendung (E10: selten).
11. **TCP-Freigaben (`share-tcp/2`, `/1`).**
    - Heute: Verbindung je Tunnel, Strom je TCP-Verbindung, 8 KiB-Pumpe mit Routenwrapper je Aufruf (B2).
    - Grenze: CPU je Kopieraufruf [S] und Leitungsrate.
    - Betrachtet: C1, C2, C5, C6, C14.
    - Gewählt: C2 mit Profil `Tcp` (Cubic, Bündel 40/80), E7, Tunnel zählen als interaktiv (E3). Beleg: Q38 (TCP über QUIC bei Verlust schneller), Q54, Q06, Q49 (kein TCP-in-TCP).
    - Verworfen: X-VPN (Userspace-TCP), X-ADAPTER, X-OVERLAY, X-MASQUE.
12. **RDP.** Läuft über 11 (`rdp/core/admission.rs:4-8` [Q]). Gewählt: alles aus 11; zusätzlich schützt E2 die RDP-Sitzung vor gleichzeitigen Dateiübertragungen. Verworfen: RDP-Shortpath-artiger UDP-Kanal (X-EIGEN; RDP-Client und -Server sind Fremdprogramme mit TCP-Anschluss).
13. **Räume.** Wie 1 (Sitzungsart `room` auf `share-fs/3`, `session.rs:310-315` [b]); Profil `Files`. Verworfen wie 1.
14. **Direkte Beziehung und Reparatur.** Wie 1, Strompriorität 0. Verworfen wie 1.
15. **Maschinenanmeldung (`login/1`).**
    - Heute: eigener Endpunkt mit eigener Transportkonfiguration (`login_transport.rs:104-135` [b]).
    - Grenze: keine Durchsatzanforderung (Steuerrahmen bis 64 KiB [b]).
    - Gewählt: unverändert. Grund: eigener Endpunkt, eigene Regeln (Klartext-Regel der Anmeldung, `docs/ARCHITEKTUR.md` Zeile 36 [b]); Profilmechanik würde nur Cubic wiederholen.
    - Verworfen: Aufnahme in E1 (kein Nutzen, zusätzliche Änderung an einer vom Vorauftrag geänderten Datei).
16. **Beobachtung (Watch).** Wie 2 (Priorität 0).
17. **Speicherschnappschuss.** Wie 2 (Priorität 0); Rahmen bis 64 MiB gesamt [b].
18. **Mount-Leases und Freigabe-Abbau.** Wie 2 (Priorität 0).

### C4 Unverschlüsselte Bildschirmübertragung (Nutzervorgabe 5): vom Nutzer zurückgenommen

Der Nutzer hat die Option am 2026-11-14 zurückgenommen: „ah dann nehme ich es zurück, dann kannst von mir aus auch das unverschlüsselt ganz weg lassen“ (A). Es gibt deshalb keine unverschlüsselte und keine nur integritätsgeschützte Bildübertragung: Jede Bildschirmverbindung bleibt wie in v0.5.196 voll verschlüsselt (TLS 1.3, AEAD, `pq_kx.rs` [Q]); Bilddaten, Eingabe und Steuerrahmen laufen auf derselben verschlüsselten Bildschirmverbindung.

Bereits geschriebene Teile der Option hat das Rückbaupaket PX am 2026-11-14 aus dem Arbeitsbaum entfernt: die integritätsgeschützte ALPN im Vendor-Patch (P1), der Zweck `ScreenKlar` mit seiner ALPN und Routenregel (P2, P4), der Klartextkanal mit Ticket, Hallo-Fähigkeit und Statusangaben (P4) sowie die zugehörigen Tests. Recht, IPC, CLI, MCP (P5a) und die Oberflächen (P5b, P5c) der Option entfallen.

Weiter gültig ist der Befund C0 Punkt 3 (die Verschlüsselung ist nicht die Grenze eines Bildstroms). Die Klartextfrage für Dateiübertragungen entscheidet unverändert E30 des Vorauftrags (E12).

**Messung M5:** entfällt.

### C5 Bildübertragung je Station (Nutzervorgabe 6)

| Station | Heute (Beleg) | Potential mit Beleg | Gewählte Verbesserung |
|---|---|---|---|
| Aufnahme | DXGI mit Dirty-Rects (`capture_dxgi.rs:282` [Q]); WGC; X11 mit XShm und Bildvergleich; PipeWire mit Shared Memory; unveränderte Bilder werden nicht kodiert (`host_worker_produce.rs:9-15` [Q]) | DMA-BUF unter PipeWire spart eine Kopie nur mit GPU-Kodierer (r-Ref A3; ohne Hardwarekodierer folgt ein Rücklesen) [F] | Keine Änderung; Stationszeit wird gemessen (M6). DMA-BUF nur zusammen mit B2. |
| Zeiger (Teil von Aufnahme und Anzeige) | Zeiger in das Bild gemischt (X11 `x11_cursor.rs:1-4`, Windows `capture_retirement.rs:77` [Q]); jede Zeigerbewegung kostet Aufnahme, Kodierung, Transport, Dekodierung und Anzeige | Zeigerform und -position getrennt übertragen und beim Betrachter selbst zeichnen: Desktop Duplication liefert den Zeiger getrennt (`PointerPosition`, `GetFramePointerShape`), das Portal bietet `cursor_mode` Metadata (`docs/refs/bildschirm-streaming-2026-10-17.md` A2, A3); beim Steuern kennt der Betrachter die Zeigerposition selbst | **Gebaut (P6):** Fähigkeit `cursor_overlay_v1`; Gastgeber X11 (XFixes ohne Mischen) und Windows (WGC ohne Zeigeraufnahme, Position und Form über `GetCursorInfo`); Betrachter Desktop und Android zeichnen den Zeiger als Überlagerung, beim Steuern an der lokalen Zeigerposition. Wayland und Android-Gastgeber bleiben bei „Zeiger im Bild“ (Grund: PipeWire-Metadaten-Parser fehlt, `pipewire_*.rs` ohne `SPA_META_Cursor` [Q: Suche ohne Treffer]; Android-Gastgeber ohne Zeiger [A]). Wirkung: Zeigerbewegung umgeht die volle Bildlatenz (M6 misst sie); Wahrnehmung auf CI nicht messbar. |
| Kodierung | MF niedrige Latenz, CBR, ohne B-Bilder; OpenH264 Bildschirm-Echtzeit; Android `KEY_LATENCY` 1 (B2); Linux nur Software | (a) Hardwarekodierung unter Linux (VA-API, `cros-libva` BSD-3, r-Ref B1/B4); (b) Referenzinvalidierung/LTR oder Intra-Refresh statt Schlüsselbild nach Verlust (NVENC, MF LTR optional, MediaCodec `KEY_INTRA_REFRESH_PERIOD`, Moonlight RFI, r-Ref C3, C4) | Messung M6 (Kodierzeit p95 je Auflösung 1080p, 1440p, 2160p mit OpenH264 auf Linux x86_64 und ARM64). **B2** (VA-API, zur Laufzeit geladen, Rückfall OpenH264) wenn p95 bei 1440p über 16,7 ms (60 Bilder/s) liegt [Planfestlegung]. **B4** (Schlüsselbild-Ersatz) wenn M7 Erholzeit p95 über 2 × RTT + 2 Bildintervalle zeigt [Planfestlegung]. Grenze: CI-Runner ohne GPU; B2 wäre auf CI nur im Rückfallweg prüfbar und wird so berichtet. |
| Paketierung | Bildgruppenkopf 20 Byte, Satzkopf 21 Byte, Kachel-Umpacken (`wire_binary.rs` [b], `gop_stream.rs:103-140` [Q]) | Kopfanteil bei 1080p-Bildern < 0,1 % [S] | Keine Änderung. |
| Transport | C4 über Cubic, GCC-artige Bitrate, Reset veralteter Ströme (B2) | (a) Verdrängung durch Massenverkehr (C0 Punkt 2); (b) Steuerrahmen hinter Bilddaten; (c) Datagramme mit FEC (r 8.3 C3) | **Gebaut:** E2 (Rückzug), E3, E4 (Steuerstrom Priorität 1). **Bedingt:** B1 nach E11/M7. |
| Dekodierung | Desktop eigener Dekodier-Thread, MF-Hardware unter Windows möglich (`decoder_hardware`, `worker_wire.rs:214-215` [Q]); Android `KEY_LOW_LATENCY` (`ScreenDecoder.kt:347` [Q]) | Linux nur Software-Dekodierung [Q: kein VA-API]; Stationszeit unbekannt | Messung M6 (Dekodierzeit p95); Linux-Hardwaredekodierung nur zusammen mit B2 nachgetragen, wenn Dekodierzeit p95 bei 1440p über 8 ms liegt [Planfestlegung]. |
| Anzeige | wgpu `AutoVsync`, höchstens ein Bild in der Warteschlange; Option „Niedrigste Latenz“ (`screen_viewer_app.rs:1-8`, `:49-60` [Q]) | `Mailbox` ausgeschlossen (eframe fragt die Fläche nicht, eine Fläche ohne `Mailbox` beendet das Programm, ebenda); Umschalten erst beim nächsten Start | Keine Änderung der Latenzwege. Grund: bestehende, belegte Abwägung; kein neuer Befund. |
| Eingabe zurück | Eingabebatches auf dem verschlüsselten Steuerstrom (`viewer_client.rs:374-403` [Q]); Auslösung im Gastgeberprozess | Eingaben warten nicht auf Bilddaten (Betrachterrichtung trägt nur Steuerung); Rückrichtung: Gastgeber-Antworten hinter Bildgruppen | E4 (Gastgeber-Steuerstrom Priorität 1); Zeigerüberlagerung (siehe Zeile Zeiger); E2 schützt die Eingabe vor Upload-Staus des Betrachters. Eingabe bleibt verschlüsselt wie die ganze Bildschirmverbindung (C4). |

Messinstrument für alle Stationen: `se screen bench` erhält eine Stationsaufschlüsselung (Aufnahme, Kodierung, Senden bis letzter Satz übergeben, Netz bis Ankunft, Dekodierung) als zusätzliche JSON-Felder `stages_ms` mit p50/p95 je Station (Erweiterung des bestehenden Befehls, kein neuer Befehl). Die Anzeige bis Bildschirm ist auf CI nicht messbar (kein Bildschirm); das bleibt als Grenze im Bericht.

### C6 Referenztechniken: übernommen und nicht übernommen

| Referenz und Stelle | Technik | Entscheidung |
|---|---|---|
| RFC 6817 §2.4.2, §2.5, §4.2 | Rückzug auf Warteschlangenverzögerung, Grundverzögerung aus Minima, nie aggressiver als TCP | Übernommen als E2 |
| RFC 6817 §2.4.2 (Einweglaufzeit) | Einweg-Zeitstempel | Nicht übernommen: QUIC-Regler sieht nur `RttEstimator` (`congestion.rs:37-45` [Q]) |
| noq PR #636/#645 (Q36), Nesquic (Q37) | größere Sendebündel je Systemaufruf | Übernommen als E5 (Verfahren: einstellbar, Vorgabe unverändert) |
| Tailscale (Q01, Q02, Q03) | UDP-GSO/GRO | Bereits vorhanden (`enable_segmentation_offload` Vorgabe an, `transport.rs:601` [b]; GRO-Lesen `noq-1.3.0/src/runtime/tokio.rs:104` [r 1]); Wirkung auf Windows und Android ungeklärt (r 9 Punkt 7) |
| Tailscale (Q02) | größere Tunnel-MTU | Nicht übernommen (E5) |
| RFC 9000 §2, noq `set_priority` | Strompriorität | Übernommen als E4 |
| ESnet/Globus (L3) | 4 bis 8 parallele Ströme bzw. Verbindungen | Bestand (Bulk bis 4 Verbindungen, bis 56 Ströme); Schwelle gemessen (E5) |
| Moonlight/Sunshine (r 6.2, Q47) | FEC je Bild, RFI, LAN unverschlüsselt als Vorgabe | FEC/RFI bedingt (B1, B4); „LAN unverschlüsselt“ nicht übernommen (vom Nutzer zurückgenommen, C4) |
| Parsec BUD (Q45) | Stau erkennen bevor er entsteht | Bestand (verzögerungsbasierter Regler `rate_control.rs`); E2 ergänzt die Gegenseite |
| RDP Shortpath (Q49) | UDP für Grafik, kein TCP-VPN | Bestand (QUIC/UDP); kein TCP-in-TCP |
| WebRTC GCC (Q42, Q44) | verzögerungsbasierte Rate, adaptive Schwelle | Bestand (`rate_control.rs:1-11`) |
| Desktop Duplication, Portal (Ref A2, A3) | Zeiger getrennt | Übernommen als Zeigerüberlagerung (C5) |
| NVENC/MF LTR/MediaCodec (Ref B1, C3) | Referenzinvalidierung, Intra-Refresh | Bedingt (B4) |
| VA-API (Ref B1, B4) | Hardwarekodierung Linux | Bedingt (B2) |
| draft-banks (Machbarkeit 3.3) | Abschalten der Paketsicherung | Nicht übernommen (C4) |
| onetun, tsnet, WireGuard (Q04, Q11, Q14) | Programm-VPN | Nicht übernommen (C1, X-VPN) |
| MASQUE (Q55, Q56) | UDP/IP über QUIC | Nicht übernommen (X-MASQUE) |

### C7 Messungen und Entscheidungsregeln

Alle Messungen laufen im Teil `verbindungen` der einen Suite (D, Paket P8) auf Remote-CI, im selben Lauf je Variante „vorher“ (heutiges Verhalten über Testschalter) und „nachher“. Zahlenwerte von Schwellen sind Planfestlegungen dieses Plans, keine Quellenwerte.

| Messung | Aufbau | Vorher / nachher | Entscheidet |
|---|---|---|---|
| M1 Massendurchsatz | Loopback ohne Formung, 1 GiB, Linux x86_64, Linux ARM64, Windows x86_64; 1 und 4 Verbindungen; Bündel 10/20 und 40/80; ACK-Frequenz aus/an | Cubic 10/20 1 Verbindung = vorher | E5 (Bündel, `DRIVER_RATE`, X-TCP-Neubewertung), E6 (B5) |
| M2 Konstanz | Linux, Profile `lan` und `wan` aus `rdt_task/speed_net.py`; Bildstrom 1080p/60 (Testbild) allein, mit gleichzeitigem 1-GiB-Download und Upload zur selben Gegenstelle; Rückzug aus/an mit T 15, 25, 50 ms | Cubic ohne Rückzug = vorher | T; Abnahme E2: Bildlatenz p95 mit Übertragung höchstens 50 ms über allein [Planfestlegung]; Massendurchsatz allein mit Rückzug-Regler mindestens 95 % von Cubic [Planfestlegung] |
| M3 Blättern während Übertragung | Linux `lan`, `wan`; Listenanfrage-Latenz p50/p95 während 1-GiB-Übertragung | Priorität aus, Rückzug aus = vorher | Abnahme E4; Nachtrag Steuerverbindung (C3 Zeile 2) |
| M4 TCP-Tunnel | Loopback ohne Formung, alle drei Plattformen; 1 GiB durch einen Tunnel | Kopiermodus „8 KiB je Aufruf mit Routenwrapper“ = vorher | E7 (b) |
| M5 Klartextbild | entfällt (C4, vom Nutzer zurückgenommen) | — | — |
| M6 Bildstationen | `se screen bench --local` und über ein Gerätepaar im Prozess, 1080p/1440p/2160p, OpenH264 und Kacheln; Linux x86_64, ARM64, Windows (MF Software) | nur nachher (Instrument neu) | B2, Linux-Dekodierung |
| M7 Bild unter Verlust | Linux `loss` (0,1 %, 100 ms RTT) und `loss1` (1 %, 40 ms RTT, 100 Mbit/s, neues Profil im Suite-Teil) | — | B1 (E11), B4 |
| M8 Exec-Aufbau | Linux `lan`, `wan`; 20 Aufträge `true` | — | B3 (E10) |
| M9 ACK-Frequenz | Teil von M1 | aus = vorher | B5 |
| M10 Verschlüsselungsanteil | S4 des Vorauftrags unverändert | — | E30 des Vorauftrags |

Ausgabe: je Messfall eine Zeile `VERBINDUNGEN-MESSUNG fall=<M> plattform=<…> variante=<…> <größe>=<wert> …`; der Suite-Teil schreibt sie in die Zusammenfassung und wertet die Abnahmeschwellen von E2, E4 und E7 als bestanden/nicht bestanden aus; die Bedingungen für B1 bis B5 erscheinen als Zeilen `VERBINDUNGEN-BEFUND bedingung=<B…> erfuellt=<ja|nein> …` und lassen die Suite nicht scheitern.

### C8 Sicherheitseigenschaften vorher/nachher

| Eigenschaft | Heute | Nach diesem Plan |
|---|---|---|
| Verschlüsselung | Alle Verbindungen TLS 1.3, AEAD (`pq_kx.rs` [Q]) | Unverändert für alle Zwecke, auch für den Bildschirm (C4) |
| Authentifizierung | Geräteschlüssel und Beziehungsnachweis je Zweckverbindung | Unverändert |
| Rechteprüfung | `authorize.rs`, Generationenbindung (`node_sessions.rs:310-329` [b]) | Unverändert |
| Routenregeln | LAN → direkt → Relay als markierter Rückfall | Unverändert |
| Relay sieht keinen Klartext | ja | ja |
| Herabstufung | nicht möglich (keine Klartextform) | unverändert nicht möglich (keine Klartextform, C4) |
| Annahmegrenzen vor der Anmeldung | `handshake_limits_tls.rs`, Strafkonto (`node_accept.rs:24-60`, `:79-93` [Q]) | Unverändert; `accept_with` ersetzt nur die Konfiguration, die Prüfungen davor bleiben in derselben Reihenfolge |

### C9 Zu erhaltendes Verhalten

- Jeder Zweck behält ALPN, Hallo, Rechteprüfung, Fristen, Leerlauf-Aufräumen (`node_idle.rs` [b]), Energiemodus und Fehlercodes.
- `application_path_policy(true)` gilt für jede Profilkonfiguration (Abnahme durch Test, P2).
- Ältere Gegenstellen: Profile wirken nur lokal (Regler, Bündel, Priorität sind senderseitig) und brauchen keine Aushandlung. Zeigerüberlagerung nur bei ausgehandelter Fähigkeit; ohne sie verhält sich jede Seite wie v0.5.196.
- Bulk-Verbindungen bleiben nie über das Relay; Ranged Uploads bleiben auf der ersten Verbindung (`node_bulk.rs:14-18` [Q]).
- TCP-Tunnel: Halbschluss, Routenpause (P28), Leerlauf 120 aktive Sekunden, Zählerfelder (`tcp_pump.rs` [Q]) bleiben.
- Bildschirm: alle Bedienwege aus `docs/oberflaechen/bildschirm.md` bleiben; „Niedrigste Latenz“ bleibt.
- Speicher: das TCP-Pufferbudget bleibt höchstens 16 MiB je Prozess (heutiger Höchstwert).

## D Arbeitsplan

### D0 Vorbedingungen (Manager, ohne Agent)

1. Die zwei Review-Agenten des Vorauftrags sind beendet; ihre Änderungen liegen im Arbeitsbaum. Dieser Auftrag setzt auf den uncommitteten Stand des Vorauftrags auf (unter anderem `node.rs`, `node_accept.rs`, `node_sessions.rs`, `session.rs`, `login_transport.rs`, `route_policy.rs`, `route_diagnostics.rs`, `vendor/iroh-1.3.0/src/{endpoint.rs,endpoint/connection.rs,tls.rs,tls/verifier.rs}`, `native/test-crates-task.py`, `.github/workflows/crates-task.yml`, `docs/tests/register.tsv`, `README.md`, `docs/ARCHITEKTUR.md`, `docs/SHARE_SERVER.md`, `docs/TODO.md`, `docs/oberflaechen/bildschirm.md`, `docs/oberflaechen/fernzugriff.md`, `android/…/ui/screen/ScreenViewScreen.kt`, `android/…/service/AccessApprovalNotifier.kt`).
2. Der Manager misst die Zeilenzahlen aller änderbaren Rust-Dateien dieses Plans neu (Stand der Planung in Klammern bei jedem Paket) und trägt Abweichungen hier ein.
3. Commits: Vorauftrag und dieser Auftrag gehen in ein Release. Meilenstein-Commits entstehen erst nach P8, jeder mit seinen Registerzeilen (`docs/tests/register.tsv`), damit `native/test-register.py` in jedem Commit stimmt. Kein Build, kein Test lokal.

### D1 Reihenfolge

| Welle | Pakete (parallel innerhalb der Welle) | Abhängigkeit |
|---|---|---|
| 1 | P1 | D0 |
| 2 | P2 | P1 |
| 3 | P3, P4 | P2 |
| 4 | P5a, P6 | P4 |
| 4b | PX Rückbau der Klartextoption nach der Rücknahme durch den Nutzer (C4): entfernt alle Teile der Option aus P1, P2, P4; erhält Sendebündel, Zweckprofile, Rückzug, Präsenz, Prioritäten und P6 | P5a, P6 |
| 5 | P7 (P5b und P5c entfallen, C4) | PX, P5a, P3, P6 |
| 6 | R (Reviews nach Nutzervorgabe 2026-11-14: Agenten des Modells Claude Fable 5.2 mit Effort xhigh) | alle P |
| 7 | P8, danach P9 | R |
| 8 | Manager: Commits, Push, ein Lauf `crates-task.yml` (`part=suite`, Vorauftrag und dieser Auftrag zusammen), Auswertung der Zeilen `VERBINDUNGEN-*`, Nachsteuerung N1, bedingte Pakete B1 bis B5, Wiederholung derselben Suite, ein Release über `release.yml` | P9 |

Parallele Pakete teilen keine änderbaren Dateien (geprüft in der Tabelle D3). Rust-Dateien bleiben unter 500 Zeilen; wo eine Datei knapp darunter liegt, nennt das Paket die Extraktion vor der Änderung. Die Auswahl von Modell und Effort der Umsetzungsagenten trifft der Manager nach dieser Planung (Arbeitsweise `modellwahl`); dieser Plan legt sie nicht fest.

### D2 Pakete

Pfadkürzel: `E/` = `native/crates/engine/src/`, `V/` = `vendor/`, `G/` = `native/crates/gui/src/app/core/`, `C/` = `native/crates/cli/src/cli/`, `M/` = `native/crates/mobile/src/mobile/os/shared/domains/`, `A/` = `android/app/src/main/java/app/smartexplorer/android/`.

#### P1 Vendor-QUIC: einstellbare Sendebündel

- Stand 2026-11-14: umgesetzt (Bericht `p1-bericht.md` des Auftrags). Der zweite Teil, integritätsgeschützte 1-RTT-Paketschlüssel für eine ALPN-Liste (`crypto/integrity_only.rs`, `crypto/rustls.rs`, iroh `Builder::integrity_only_alpns`, `TlsConfig::with_integrity_only_alpns`, ihre Tests und der Teil des Patchvertrags), diente nur der zurückgenommenen Klartextoption (C4) und ist mit PX entfernt; `V/noq-proto-1.3.0/src/crypto/rustls.rs` ist wieder unverändert, `V/iroh-1.3.0/src/{endpoint.rs,tls.rs}` tragen nur noch die Änderungen des Vorauftrags (M6, E33).
- Ergebnis: `TransportConfig` von noq-proto hat Setter für die Segmentzahl je Sendeaufruf und die Datagrammzahl je Treiberdurchlauf mit Vorgaben 10 und 20; `noq` liest sie statt der Konstanten; iroh reicht sie über `QuicTransportConfigBuilder` durch. Jede geänderte Datei trägt den Kopfvermerk „Smart Explorer … patch“.
- Lesbar: `V/noq-1.3.0/src/{connection.rs,send_stream.rs}`, `V/noq-proto-1.3.0/src/{config/transport.rs,connection/mod.rs}`, `V/iroh-1.3.0/src/endpoint/quic.rs`, `V/iroh-relay-1.0.0/src/protos/relay.rs`, `V/SMART_EXPLORER_ROUTE_POLICY.md`, `native/Cargo.toml`.
- Geändert: `V/noq-1.3.0/src/connection.rs`, `V/noq-proto-1.3.0/src/config/transport.rs`, `V/noq-proto-1.3.0/src/connection/mod.rs` (Abweichung: Lesemethoden für noq), `V/iroh-1.3.0/src/endpoint/quic.rs`, neu `V/SMART_EXPLORER_QUIC_PROFILES.md` (Patchvertrag: Zweck, Stellen, Rebase-Regel, Verifikationsstand), neu `E/share/core/verbindungen_vendor_task_tests.rs` (Rückfall-Testdatei).
- Nicht anfassen: `V/iroh-1.3.0/src/tls/verifier.rs`, `V/iroh-1.3.0/src/endpoint/remote_key_gate.rs` (Vorauftrag), alle Dateien unter `native/` außer der Rückfall-Testdatei.
- Verträge: Vorgaben von noq/iroh bleiben für jeden Nutzer ohne Setter gleich (10/20); Schlüsselsperrtor M6 und Resolver E33 des Vorauftrags bleiben unverändert; Relay-Bündel bleiben unter 64 KiB (Prüfung im Patch: Segmentzahl × Segmentgröße ≤ `MAX_PACKET_SIZE`, sonst Kappung).
- Abnahmesignal: Vendor-Unit-Tests `smart_explorer_transmit_batch_tests` (in den Vendor-Crates; im Workspace nicht baubar, weil `native/Cargo.lock` ihre Dev-Abhängigkeiten nicht auflöst, `p1-bericht.md` Abweichung 2) und die Engine-Tests `verbindungen_task_vendor_transmit_defaults_setters_and_relay_cap`, `verbindungen_task_vendor_iroh_transmit_setters_reach_noq_config` in `E/share/core/verbindungen_vendor_task_tests.rs`: Vorgaben 10/20, Setter-Werte in der noq-Konfiguration, Relay-Kappung.
- Oberflächen: keine.
- Messung: Grundlage für M1 (Bündel).

#### P2 Zweckprofile, Rückzugsregler, interaktive Präsenz

- Ergebnis: E1, E2, E3 (Register), E4 für Dateisitzungen, E5 (Profile setzen Bündel), Testschalter für M1 und M2 (Regler, T, Bündel, ACK-Frequenz) hinter `#[cfg(any(test, feature = "test-support"))]`.
- Lesbar: `E/share/core/{keepalive.rs,node.rs,node_accept.rs,node_sessions.rs,node_routes.rs,node_apps_link.rs,node_screen_link.rs,node_tcp_link.rs,node_bulk.rs,node_idle.rs,route_protocol.rs,session.rs,handshake_limits_tls.rs,auth_penalty.rs,exec_protocol.rs,apps_wire.rs,tcp_forward.rs,lan_link_wire.rs}`, `E/share/os/shared/{lan_link_dial.rs,lan_link_transport.rs,peer_feed_link.rs}`, `E/screen/core/wire.rs`, `V/noq-proto-1.3.0/src/congestion.rs`, `V/noq-proto-1.3.0/src/congestion/cubic.rs`, `V/iroh-1.3.0/src/endpoint.rs`, `V/iroh-1.3.0/src/endpoint/connection.rs`, RFC 6817 (§2.4, §2.5, §4).
- Änderbar: `E/share/core/keepalive.rs` (268; Basisbau wird wiederverwendbar, Deklaration der neuen Module per `#[path]` wie `route_diagnostics.rs:52-56`), neu `E/share/core/transport_purpose.rs`, neu `E/share/core/yield_controller.rs`, neu `E/share/core/interactive_presence.rs`, `E/share/core/node_accept.rs` (265; `accept_with`, Zweckplatz nach Handshake), `E/share/core/node_sessions.rs` (499; nur Ersatz der Zeile `:388`, Zeilenzahl bleibt ≤ 499), `E/share/core/node_routes.rs` (194), `E/share/core/node_apps_link.rs` (144), `E/share/core/node_screen_link.rs` (285), `E/share/core/node_tcp_link.rs` (479; nur Erweiterung der vorhandenen `ConnectOptions` in `:84`, höchstens +1 Zeile), `E/share/os/shared/lan_link_dial.rs` (121), `E/share/core/node_bulk.rs` (228; Priorität −1, kein Wachstum bei Präsenz, `DRIVER_RATE` als benannter Wert für N1).
- Nicht anfassen: `E/share/mod.rs` (590 Zeilen), `E/share/core/node.rs`, `E/share/core/login_transport.rs`, `E/share/core/route_policy.rs`, alle Bildschirm-, TCP-Pumpen- und Oberflächendateien.
- Verträge: alle Aufrufer von `open_stream`, `open_stream_until`, `open_transfer_stream_until` (25 Aufrufstellen laut `bestand.md` 2.4) unverändert; `iroh_transport_config_for_routes` bleibt als Endpunkt-Grundkonfiguration (`node.rs:171`, `s4_messung_support.rs:195-200`); Messschalter `SE_SHARE_QUIC_CC` verhält sich wie bisher (Tests `keepalive.rs:203-239` bleiben gültig); Annahmegrenzen in derselben Reihenfolge (C8).
- Abnahmesignal: Unit-Tests mit Präfix `verbindungen_task_`: jedes Profil hat `application_path_policy` an und die heutigen Keepalive-, Leerlauf-, Strom- und Fensterwerte; Profil je ALPN korrekt (auch spät gebunden); Rückzugsregler: Fenster ≤ Cubic-Fenster immer, Fenster sinkt bei Warteschlangenverzögerung > T nur mit Präsenz, Grundverzögerung aus Minima; Präsenz zählt herauf und herunter, auch bei Abbruch (Drop); Bulk wächst nicht bei Präsenz; Transferströme haben Priorität −1.
- Oberflächen: keine Bedienänderung. Diagnose: keine neue Ausgabe (Regler- und Profilzustand sind interne Transporteigenschaften ohne Einstellung; `se share status --json` bleibt unverändert) [Planfestlegung].
- Messung: M1, M2, M3 (Schalter dieses Pakets).

#### P3 TCP-Pumpe

- Ergebnis: E7 (a) und (b); Präsenz je laufender Pumpe (E3); Testschalter „Kopiermodus vorher“ für M4.
- Lesbar: `E/share/os/shared/tcp_pump.rs`, `E/share/core/{tcp_forward.rs,tcp_registry.rs,node_tcp.rs,node_tcp_link.rs,route_protocol.rs,power.rs}`, `E/share/os/shared/route_monitor.rs`, `E/share/core/interactive_presence.rs` (aus P2).
- Änderbar: `E/share/os/shared/tcp_pump.rs` (99), `E/share/core/tcp_forward.rs` (263; Pufferbudget-Werte), neu `E/share/os/shared/tcp_copy_budget.rs`.
- Nicht anfassen: `E/share/core/node_tcp_link.rs`, `E/share/core/node_tcp.rs`, `E/share/core/tcp_registry.rs`, alle Dateien aus P2 und P4.
- Verträge: Signatur `pump(connection, socket, send, recv, counters)` mit allen Aufrufern (`node_tcp.rs`, `tcp_registry.rs`); Halbschluss; Routenpause P28; Leerlauf 120 aktive Sekunden; Zähler `active`, `read`, `written`; Energiehalt je Datenfluss.
- Abnahmesignal: Tests `verbindungen_task_tcp_*`: Halbschluss in beide Richtungen, Routenpause hält Leerlaufzeit an, Puffer wächst nur bei vollem Lesen und nur mit Budget, Budget kehrt nach Ende vollständig zurück, Präsenz während der Pumpe.
- Oberflächen: keine.
- Messung: M4.

#### P4 Bildtransport: Präsenz und Steuerpriorität

- Stand 2026-11-14: umgesetzt (Bericht `p4-bericht.md` des Auftrags). Der Klartextkanal (C4 Punkte 1 bis 5 und 7: `E/screen/core/klar_channel.rs`, `klar_host.rs`, `klar_viewer.rs`, `klar_channel_tests.rs`, `E/share/core/node_screen_klar.rs`, `verbindungen_klar_route_tests.rs`, Zweck `ScreenKlar` in `transport_purpose.rs`, `node_accept.rs`, LAN-Regel in `route_start.rs`, `route_policy.rs`, `E/share/os/shared/route_monitor.rs`, Feld `picture_klar` im Server-Hallo, Zusatzangabe neben dem Client-Hallo, Trägerwahl in `gop_stream.rs`, Zustand `picture_encrypted` mit Gründen) ist nach der Rücknahme (C4) mit PX entfernt.
- Ergebnis: E3 für Bildschirm (Gastgeber-Pipeline in `host_server.rs`, Betrachter-Bildleser in `viewer_client_tasks.rs`; Regel `gop_stream::picture_session`: nur `view` und `control`, nicht `probe`, `preview` und Metadaten), Re-Export des Präsenzregisters in `E/share/core/node_screen.rs` (`enter_interactive`, `InteractiveGuard`, `InteractiveKind`), E4 (`gop_stream::CONTROL_STREAM_PRIORITY = 1`, `prioritize_control`, gesetzt in `node_screen.rs` direkt nach `open_bi` des Gastgeber-Steuerstroms).
- Stand 2026-11-14 nach der Review (Bericht `review-bild-bericht.md` des Auftrags): Die Präsenz folgt dem, was die Sitzung überträgt, nicht nur der Sitzungsart des Hallos. `gop_stream::PicturePresence` hält sie, beim Gastgeber in der Bildpipeline (`E/screen/core/host_pipeline_actor.rs`, nicht mehr in `host_server.rs`), beim Betrachter in der Schreibaufgabe des Steuerstroms (`viewer_client_tasks.rs`, sie sieht jedes `SET_MODE`). Eine `view`- oder `control`-Sitzung zählt ab ihrem Beginn; eine Vorschau zählt, solange `SET_MODE` sie zur Vollansicht angehoben hat (Kachel der Gruppenansicht, die zur Ansicht wird), und nicht mehr, sobald sie wieder Vorschau ist; `probe` und Metadaten nie. Tests zusätzlich `verbindungen_task_bild_presence_follows_a_preview_raised_to_the_full_view`; `verbindungen_task_bild_control_stream_above_picture_groups` öffnet jetzt eine echte Bildgruppe (`GopWriter::open`) und vergleicht deren Priorität mit der des Steuerstroms.
- Vorbereitung im selben Paket: Extraktion der Leser- und Schreibaufgaben aus `E/screen/core/viewer_client.rs` (492) in neu `E/screen/core/viewer_client_tasks.rs`; sie hält die Präsenz des Betrachters und trägt seit P6 auch den Zeigerwunsch, `viewer_client.rs` bleibt damit unter 500 Zeilen.
- Geändert: `E/screen/core/host_server.rs`, `E/screen/core/viewer_client.rs`, neu `E/screen/core/viewer_client_tasks.rs`, `E/screen/core/gop_stream.rs`, neu `E/screen/core/gop_stream_bild_tests.rs`, `E/share/core/node_screen.rs`.
- Nicht anfassen: `E/screen/core/host_session.rs`, `E/screen/core/host_pipeline_actor.rs`, `E/share/core/node.rs`, alle Oberflächendateien.
- Verträge: Bildgruppenformat und `FRAME_ACK` unverändert; Hallo unverändert (keine neuen Felder durch P4); Reset-Code `0x5352`; Endgründe der Sitzung unverändert; Relay-Wächter (`pacing.rs:13-17`) unverändert.
- Abnahmesignal: Tests `verbindungen_task_bild_presence_only_for_view_and_control` und `verbindungen_task_bild_control_stream_above_picture_groups` (`E/screen/core/gop_stream_bild_tests.rs`): Präsenz nur bei `view` und `control`; Gastgeber-Steuerstrom hat Priorität 1 über den Bildgruppen (0).
- Oberflächen: keine.
- Messung: M2 (Bildseite), M7.

#### P5a Stationsausgabe der Messung

- Stand 2026-11-14: umgesetzt (Bericht `p5a-bericht.md` des Auftrags). Recht `screen_klar`, IPC-Felder `picture_klar` und `picture_encrypted`, CLI-Option `--picture-klar` und das MCP-Feld `picture_klar` gehörten zur zurückgenommenen Klartextoption (C4) und entfallen; P5a hat seine begonnenen Teile davon selbst zurückgenommen.
- Ergebnis: Textausgabe von `se screen bench` zeigt je Station eine Zeile `stage_ms` aus dem JSON-Feld `stages_ms` (Feld liefert P7 in `BenchReport`); ohne `stages_ms` ist die Ausgabe zeichengleich zu vorher.
- Geändert: `C/screen/output.rs`, `C/screen/bench.rs` (nur Moduldoc), `C/screen/tests.rs`.
- Nicht anfassen: `C/mcp/catalog.rs`, `E/share/core/access_catalog.rs`, alle GUI- und Android-Dateien.
- Verträge: bestehende JSON-Felder und Textzeilen unverändert; neue Felder nur ergänzend.
- Abnahmesignal: Test `verbindungen_task_cli_bench_stages_in_text`.
- Oberflächen: CLI (Text und JSON).
- Messung: Ausgabe für M6 (`stages_ms`).

#### P5b Desktop-Oberfläche der Klartextoption

- Entfällt: Das Paket enthielt nur die Desktop-Bedienung der vom Nutzer zurückgenommenen Klartextoption (C4); nicht begonnen.

#### P5c Android-Oberfläche der Klartextoption

- Entfällt: Das Paket enthielt nur die Android-Bedienung und die Bridge-Felder der vom Nutzer zurückgenommenen Klartextoption (C4); nicht begonnen.

#### P6 Zeigerüberlagerung

- Ergebnis: Fähigkeit `cursor_overlay_v1`; Gastgeber X11 und Windows senden Zeigerform (bei Wechsel) und Position (bei Bewegung, gebündelt je Bildtakt) als Steuerrahmen und mischen den Zeiger nicht mehr ins Bild; Betrachter Desktop (wgpu und glow) und Android zeichnen den Zeiger als Überlagerung, beim Steuern an der lokalen Zeigerposition; ohne Fähigkeit wie heute.
- Stand 2026-11-14 nach der Review (Bericht `review-bild-bericht.md` des Auftrags), umgesetzt und im Hallo als Feld `cursor_overlay` = 1 angekündigt. Gegenüber dem Paketstand geändert: (1) Der Hostprozess merkt sich den Wunsch des Betrachters und setzt ihn um, sobald seine Aufnahme den Zeiger trennen kann (Windows legt die Aufnahmesitzung erst mit dem ersten Bild an); die Sitzung fragt nach jedem Quellenwechsel (`SET_MODE` mit Quelle) die neue Quelle erneut. (2) Vor jeder Umschaltung sendet der Gastgeber eine Position „nichts zu zeichnen“, damit kein Betrachter einen Zeiger des vorigen Zustands behält. (3) Die Sitzung gibt Zeigerrahmen weiter, sobald der Hostprozess sie meldet (höchstens eine Position je aufgenommenem Bild), nicht erst beim nächsten Bildtakt. (4) Windows: kann die Aufnahmesitzung ihre Zeigeraufnahme nicht schalten (vor Windows 10 Version 2004), bleibt der Zeiger im Bild und die Überlagerung aus; nach dem Umschalten einer laufenden Sitzung ohne neues Bild binnen 150 ms startet die Aufnahmesitzung neu; invertierende Zeigerpunkte werden schwarz mit weißem Rand gezeichnet. (5) Desktop-Betrachter: „an der lokalen Zeigerposition“ gilt genau dann, wenn die Eingabe der Ansicht an den Gastgeber geht (Schalter „Steuern“ an, Eingabe bereit, Zeiger über dem unverdeckten Bild); Vollansicht ↔ Kachel schaltet die Überlagerung ab und an. Offen bleibt wie geplant: Wayland-Gastgeber (zwei Zeiger beim Steuern), dazu `bildkette.md` B-13 und B-16.
- Lesbar: `E/screen/core/{wire_control.rs,wire_control_codec.rs,wire_hello.rs,host_session_control.rs,host_capture.rs}`, `E/screen/os/linux_os/{x11_cursor.rs,x11_capture.rs}`, `E/screen/os/windows/{capture_wgc.rs,capture_retirement.rs,capture_dxgi.rs}`, `G/{screen_view_widget.rs,screen_view_widget_render.rs,screen_render_wgpu.rs,screen_render_glow.rs,screen_session.rs}`, `M/screen_view_run.rs`, `A/ui/screen/ScreenViewScreen.kt`, `docs/refs/bildschirm-streaming-2026-10-17.md` A1 bis A3.
- Änderbar: `E/screen/core/wire_control.rs` (357), `E/screen/core/wire_hello.rs` (nach P4; Fähigkeitsfeld), `E/screen/core/host_session_control.rs` (226), neu `E/screen/core/cursor_overlay.rs`, `E/screen/os/linux_os/x11_cursor.rs` (120), `E/screen/os/linux_os/x11_capture.rs` (386), neu `E/screen/os/windows/cursor_overlay.rs`, `E/screen/os/windows/capture_retirement.rs` (136), `G/screen_view_widget_render.rs` (63), `G/screen_render_wgpu.rs` (208), `G/screen_render_glow.rs` (304), `G/screen_session.rs` (480; höchstens +8 Zeilen, sonst neu `G/screen_cursor.rs`), `M/screen_view_run.rs` (431), `A/ui/screen/ScreenViewScreen.kt` (vom Vorauftrag geändert; auf dessen Stand).
- Nicht anfassen: `E/screen/os/windows/capture_dxgi.rs` (499; Zeigerdaten kommen aus `GetCursorInfo` in der neuen Datei, nicht aus DXGI), Wayland- und PipeWire-Dateien, Android-Gastgeberdateien, alle Dateien von P5a bis P5c.
- Verträge: Bild und Eingabe ohne Fähigkeit unverändert; Steuerrahmen-Codes bestehender Rahmen unverändert; Sitzung „Ansehen“ zeigt die Gastgeberposition, „Steuern“ die lokale.
- Abnahmesignal: Tests `verbindungen_task_zeiger_*`: Rahmenkodierung, Aushandlung, X11 mischt bei Fähigkeit nicht (Test über die vorhandene X11-Testumgebung der Suite), Windows setzt die Zeigeraufnahme bei Fähigkeit aus; Überlagerung im egui-Layout-Rahmen sichtbar.
- Oberflächen: Desktop, Android (Betrachter).
- Messung: M6 liefert die umgangene Bildlatenz.

#### P7 Messtests und Stationsaufschlüsselung

- Ergebnis: Messtests M1 bis M4 und M6 bis M8 (M5 entfällt, C4) als ignorierte Tests (`#[ignore = "Messung; Suite crates-task"]`, Präfix `verbindungen_messung_task_`) mit Ausgabezeilen nach C7; Stationszeiten in `se screen bench` (`stages_ms`).
- Lesbar: `E/share/core/{s4_messung_support.rs,s4_messung_task_tests.rs,transport_purpose.rs,yield_controller.rs,interactive_presence.rs,node_bulk.rs}`, `E/share/os/shared/tcp_pump.rs`, `E/screen/core/{bench.rs,metrics.rs,batches_rate_reference_transport.rs,batches_rate_reference_tasks.rs,gop_stream.rs}`, `native/rdt_task/speed_net.py`.
- Änderbar: neu `E/share/core/verbindungen_messung_support.rs`, neu `E/share/core/verbindungen_messung_task_tests.rs`, neu `E/screen/core/verbindungen_bild_messung_task_tests.rs`, `E/share/core/route_diagnostics.rs` (56; nur zwei `mod`-Zeilen wie für S4), `E/screen/core/mod.rs` (nach P6; eine `mod`-Zeile), `E/screen/core/bench.rs` (455; höchstens +25 Zeilen, sonst neu `E/screen/core/bench_stages.rs`), `E/screen/core/metrics.rs` (293).
- Nicht anfassen: S4-Dateien, `C/screen/output.rs` (P5a), alle Produktionsdateien anderer Pakete.
- Vertrag aus P5a: `BenchReport` erhält das Feld `stages_ms` (Station → p50/p95 in Millisekunden, beim Serialisieren ausgelassen, wenn leer); `C/screen/output.rs` liest es aus dem serialisierten Bericht.
- Verträge: S4 bleibt unverändert; Messtests laufen nur im Suite-Teil, nie in einem gewöhnlichen `cargo test`.
- Abnahmesignal: jede Messung liefert genau eine Zeile je Fall und Variante; Laufzeit je Fall höchstens 60 s wie S4 (`s4_messung_support.rs:15`).
- Oberflächen: CLI (`se screen bench` JSON über P5a).
- Messung: ist das Messinstrument.

#### R Reviews

Nach Integration aller P-Pakete und vor P8: Reviews durch Agenten des Modells Claude Fable 5.2 mit Effort xhigh (Nutzervorgabe 2026-11-14). Prüfumfang: P1 (Sendebündel und Rebase-Vertrag), P2 und P3 (Regler, Annahmepfad, Pumpe), P4 und P6 (Präsenz, Steuerpriorität, Zeiger), P5a (Stationsausgabe), PX (vollständiger Rückbau der Klartextoption). Befunde behebt das jeweilige Autorpaket; der Manager prüft die Integration.

#### P8 Task-Suite (eigenes Paket)

- Ergebnis: Teil `verbindungen` in `native/test-crates-task.py` mit Job-Matrix Linux x86_64 (mit Netem), Linux ARM64, Windows x86_64 in `.github/workflows/crates-task.yml` (Timeout ≥ 30 min je Job, Speed-Profil wie S4); Netem-Profile `lan`, `wan`, `loss` per Import aus `native/rdt_task/speed_net.py` (`PROFILES`, ohne Änderung der Datei) plus Profil `loss1` im Suite-Teil; Auswertung der Zeilen `VERBINDUNGEN-MESSUNG` und `VERBINDUNGEN-BEFUND` mit den Abnahmeschwellen aus C7; die neuen Unit-Tests der Pakete laufen im bestehenden Teil `workspace`; Registerzeilen für jeden neuen Test aller Pakete in `docs/tests/register.tsv` (Spalten Datei, Test, Urteil, Kategorie, Schützt, Suite).
- Lesbar: `native/test-crates-task.py`, `.github/workflows/crates-task.yml`, `docs/tests/register.tsv`, `docs/tests/README.md`, `native/test-register.py`, `native/rdt_task/speed_net.py`, alle Testdateien aus P1 bis P7.
- Änderbar: `native/test-crates-task.py`, `.github/workflows/crates-task.yml`, `docs/tests/register.tsv`.
- Nicht anfassen: Teile und Jobs des Vorauftrags (`s4`, `e2e`, `workspace`-Gruppen), `native/rdt_task/*`.
- Verträge: ein Einstieg, ein Workflow, `part=suite` startet alle Jobs; Vorauftrag-Teile unverändert.
- Abnahmesignal: `native/test-register.py` findet zu jedem Test eine Zeile und umgekehrt (statische Prüfung im Suite-Lauf); der Teil `verbindungen` schreibt eine Zusammenfassung mit allen Messfällen.
- Oberflächen: Dokumentation des Teils im Kopf von `native/test-crates-task.py` und `crates-task.yml`.
- Messung: führt M1 bis M4 und M6 bis M9 aus.

#### P9 Dauerhafte Dokumentation (eigenes Paket)

- Ergebnis: `docs/ARCHITEKTUR.md` (Zweckprofile, Rückzug, Präsenz, Zeigerüberlagerung); `docs/oberflaechen/bildschirm.md` (Präsenz, Steuerpriorität, `stages_ms`); `docs/oberflaechen/fernzugriff.md` (TCP-Pumpe, Tunnel als interaktiv); `docs/SHARE_SERVER.md` (Relay sieht weiterhin nie Klartext); `docs/TRANSFER_ENGINE.md` (Rückzug, Prioritäten, Sendebündel, `DRIVER_RATE` gemessen); `docs/GOTCHAS.md` (`accept_with` mit spät gebundenem Zweck, Sendebündel und Relay-64-KiB-Grenze); `docs/TODO.md` (bedingte Pakete B1 bis B5 mit Messergebnis, Schließen erledigter Punkte); `README.md` (nur falls die Funktionsliste eine der Änderungen nennt).
- Stand 2026-11-14: umgesetzt für den tatsächlichen Stand des Arbeitsbaums nach P1 bis P7, P2b, PX und den zwei Reviews (Bericht `p9-bericht.md` des Auftrags, Abschnitt E dieses Plans). Gegenüber dem Paketentwurf geändert: Zusätzlich geschrieben wurden `docs/oberflaechen/share-exec.md` (Exec mit Terminal als interaktive Sitzung), `docs/refs/INDEX.md` (Einträge für `recherche.md` und `bildkette.md`, Nutzervorgabe an P9), der Verweis auf `vendor/SMART_EXPLORER_QUIC_PROFILES.md` in `native/Cargo.toml` (nur Kommentar), `README.md` (ein Absatz zur Zeigerüberlagerung und zu `stages_ms`) und dieser Plan. `docs/SHARE_SERVER.md` war für die Relay-Wahl schon ergänzt und wurde nur gegen den Quelltext geprüft. Die Bedingung der Messwerte („trägt die Messwerte des Suite-Laufs ein“) ist offen, weil kein Suite-Lauf stattgefunden hat; `docs/TODO.md` Zeile `VERB-MESSUNG` hält fest, welche Zahlen Schätzungen sind. `docs/RELEASING.md` bleibt unverändert (kein neuer Release-Schritt).
- Lesbar: alle genannten Dokumente, Pakete P1 bis P8, `V/SMART_EXPLORER_QUIC_PROFILES.md`, die Berichte des Auftrags.
- Änderbar: die genannten Dokumente, `docs/refs/INDEX.md`, `docs/oberflaechen/share-exec.md`, `native/Cargo.toml` (nur der Kommentar zum Vendor-Patch).
- Nicht anfassen: `docs/RELEASING.md` (kein neuer Release-Schritt), die Dateien unter `docs/refs/` außer dem Index, `recherche.md`, `bestand.md`, `docs/tests/register.tsv`, `native/test-crates-task.py`, `.github/workflows/crates-task.yml` (Paket P8).
- Verträge: Dokumentationsregeln aus `AGENTS.md` (Kontextprüfung, Suche nach veralteten Aussagen).
- Abnahmesignal: Suche nach „eine Verbindung je Gegenstelle“, „bidirektionalen Strom“ (Bildschirm) und „unverschlüsselt“ ohne widersprüchliche Treffer in den acht Dokumenten.
- Oberflächen: Dokumentation.
- Messung: trägt die Messwerte des Suite-Laufs ein.

### D3 Prüfung „parallele Pakete teilen keine änderbaren Dateien“

- Welle 3: P3 (`tcp_pump.rs`, `tcp_forward.rs`, `tcp_copy_budget.rs`) und P4 (Bildschirm-, `node_screen*.rs`, `node_accept.rs`, `route_start.rs`, `transport_purpose.rs`): keine Überschneidung.
- Welle 4: P5a (Katalog, Autorisierung, IPC, CLI, MCP) und P6 (`wire_control.rs`, `wire_hello.rs`, `host_session_control.rs`, Zeiger-Dateien, GUI-Render, `screen_session.rs`, `screen_view_run.rs`, `ScreenViewScreen.kt`): keine Überschneidung. `wire_hello.rs` ändert P4 vor P6 (Welle 3 vor Welle 4).
- Welle 5: nur P7 (neue Messdateien, `route_diagnostics.rs`, `screen/core/mod.rs`, `bench.rs`, `metrics.rs`); P5b und P5c entfallen (C4). `screen/core/mod.rs` ändert P6 in Welle 4 vor P7.

### D4 Nachsteuerung und bedingte Pakete nach dem ersten Suite-Lauf

- **N1 (Manager):** T aus M2 in `yield_controller.rs`, `DRIVER_RATE` aus M1 in `node_bulk.rs`, Sendebündel aus M1 in `transport_purpose.rs`, Puffer-Endwert aus M4 in `tcp_forward.rs`; je ein Commit; derselbe Suite-Lauf bestätigt.
- **B1 Bild auf Datagrammen mit FEC** (Bedingung E11): eigener Planabschnitt (Datagrammgröße, FEC-Anteil, Referenzinvalidierung) vor Umsetzung; Dateien voraussichtlich `E/screen/core/gop_stream.rs`, neu `E/screen/core/datagram_*.rs`, `wire_hello.rs`.
- **B2 Linux-Hardwarekodierung** (Bedingung C5): VA-API zur Laufzeit geladen (keine Linkabhängigkeit), Rückfall OpenH264; neue Dateien unter `E/screen/os/linux_os/`; auf CI nur Rückfallweg prüfbar.
- **B3 Exec-Wiederverwendung** (Bedingung E10): `share-exec/3` mit Aushandlung; Dateien `E/share/core/{exec_client.rs,exec_admission.rs,exec_server.rs,node_routes.rs}` nach eigener Extraktion (`exec_client.rs` 492, `exec_server.rs` 459).
- **B4 Schlüsselbild-Ersatz** (Bedingung C5): MF LTR, MediaCodec Intra-Refresh; Dateien `E/screen/os/windows/mf_media.rs`, `A/screen/ScreenEncoder.kt`.
- **B5 ACK-Frequenz an** (Bedingung E6): eine Einstellung in `transport_purpose.rs`.
- Jedes bedingte Paket, dessen Bedingung erfüllt ist, gehört zu diesem Auftrag und läuft vor dem Release; seine Tests kommen in denselben Suite-Teil; danach wiederholt der Manager dieselbe Suite einmal.

### D5 Abschluss

Ein Release über `.github/workflows/release.yml` mit dem vollständigen `main`-SHA nach erfolgreicher Suite (Vorauftrag und dieser Auftrag gemeinsam), danach Aktualisierung der lokalen Installation nach den Projektregeln. Der Abschlussbericht an den Nutzer nennt: Antwort auf die VPN-Frage (C1), Messwerte M1 bis M4 und M6 bis M9 je Plattform, die Rücknahme der Klartextoption (C4) und welche bedingten Pakete gebaut wurden.

## E Erreichter Stand (2026-11-14, Arbeitsbaum nach P1 bis P7, P2b, PX, beiden Reviews, P9 und P8)

Grundlage: die Berichte des Auftrags (`p1` bis `p7`, `p2b`, `px`, `review-bild`, `review-transport`; spätere Berichte gehen
früheren vor, `px-bericht.md` gilt für alles zur zurückgenommenen Klartextoption) und der Quelltext des Arbeitsbaums. Nichts davon
wurde gebaut, kompiliert, getestet, committet oder veröffentlicht; jede Aussage zur Übersetzbarkeit und zum Verhalten stammt
aus dem Lesen des Quelltexts und der vendorierten Bibliotheken. Die Dokumentation steht in `docs/ARCHITEKTUR.md` (Abschnitt
„Verbindungen je Zweck“), `docs/GOTCHAS.md` (Abschnitt „Connections per purpose“), `docs/TRANSFER_ENGINE.md` (Abschnitt vom
2026-11-14), `docs/oberflaechen/{bildschirm,fernzugriff,share-exec}.md`, `docs/SHARE_SERVER.md` (QAD und Heimrelay), `docs/TODO.md`
(Zeilen `VERB-*`), `docs/refs/INDEX.md` und `README.md`.

**Umgesetzt im Arbeitsbaum.**

| Baustein | Stand |
|---|---|
| E1 Zweckprofile (P2) | `transport_purpose.rs`: sechs Zwecke `Files`, `Screen`, `Exec`, `Apps`, `Tcp`, `LanLink`; ausgehend an allen sechs Aufbaustellen, eingehend über `accept_with` mit spät gebundenem Zweck. Abweichung vom Entwurf: kein eigener Zweck `FilesBulk` (Zusatzverbindungen laufen mit `Files`, gleiche Werte) und kein eigener Zweck `Feed` (der Gerätefeed läuft als `Screen`). |
| E2 Rückzugsregler (P2, Review Transport) | `yield_controller.rs` nach RFC 6817 mit multiplikativer Verringerung nach LEDBAT++ §4.2. Review-Befunde behoben: Flugmenge vor der Bestätigung statt danach (sonst hätte das Fenster nie wachsen können), inneres Cubic-Fenster ruht, solange das Rückzugsfenster gilt. T = 25 ms ist eine Schätzung bis M2. |
| E3 Präsenzregister (P2, P2b, P3, P4, Review Bild) | Arten `ScreenHost`, `ScreenViewer`, `ExecTerminal` (Client und Gastgeber), `TcpPump`; Bildschirmsitzungen folgen `SET_MODE`; keine weitere Zusatzverbindung unter Präsenz. |
| E4 Stromprioritäten (P2, P2b, P4) | Dateidaten −1 (Client alle Ströme aus `open_transfer_stream_until`, Gastgeber `Read`, `ReadAt`, `GetBatch`), Steuerstrom des Bildschirm-Gastgebers +1, Bildgruppen 0. |
| E5 Sendebündel (P1, Review Transport) | Vendor-Patch `vendor/SMART_EXPLORER_QUIC_PROFILES.md` in noq-proto, noq und iroh; `Files` und `Tcp` 40/80, `Screen`, `Exec`, `Apps` und `LanLink` 10/20; Relay-Kappung rechnet seit der Review mit 36 Byte Rahmenanteil. Verweis in `native/Cargo.toml` neben dem auf `SMART_EXPLORER_ROUTE_POLICY.md`. |
| E7 TCP-Pumpe (P3, Review Transport) | Routenabo je Pumpe, Beobachtung je Sekunde, Leerlauf in aktiver Zeit, Puffer 8 KiB wachsend auf 64 KiB im Budget von 16 MiB. |
| C5 Zeigerüberlagerung (P6, Review Bild) | Fähigkeit `cursor_overlay`, Rahmen 47 bis 49, Hostprozess-Rahmen `CURSOR`; X11 und Windows mit Graphics Capture (ab Windows 10 Version 2004); Desktop-Betrachter und Android-Betrachter; Nachziehen des Wunsches nach Quellenwechsel und nach `session_changed`. |
| C5 Messung (P5a, P7) | `stages_ms` mit `capture_to_arrival` in `se screen bench`; Messtests `verbindungen_messung_task_*` mit der Ausgabezeile `VERBINDUNGEN-MESSUNG fall=<M> plattform=<os-arch> variante=<Name> profil=<Strecke> <Größe>=<Wert> …`. |
| Verbindungsabbrüche (Debug-Auftrag, Review Transport) | Ursache: QAD antwortet für den Host und nicht je Relay-URL; der Signalisierungsport von `se-share-server` wurde als zweiter Kandidat desselben Hosts mit QAD Heimrelay und veröffentlichte Relay-Adresse. Behebung in `endpoint_routes.rs` und `login_transport.rs` (Kandidaten desselben Hosts ohne QAD, QAD erst für das verbundene Heimrelay). Dokumentiert in `docs/SHARE_SERVER.md`. |

**Zurückgenommen (Nutzer 2026-11-14).** Die unverschlüsselte Bildübertragung (C4) hat der Nutzer zurückgenommen; PX hat alle ihre
Teile entfernt, und die Reviews fanden in Code, Tests, Vendor, Register und Suite-Skripten keinen Rest. Es gibt keine solche Funktion
und keine geplante. P5b und P5c entfielen. Die Klartextfrage für Dateiübertragungen (E30 des Vorauftrags, TODO `SEC-S4-KLARTEXT`) ist
eine getrennte Frage und unverändert.

**Nicht erreicht und offen (mit Beleg in `docs/TODO.md`).**

1. Wayland-Gastgeber zeigen beim Steuern weiter zwei Zeiger (`VERB-WAYLAND-ZEIGER`); ebenso Graphics Capture vor Windows 10 Version 2004
   und jeder Gastgeber bis v0.5.196.
2. Hostprozess und Daemon müssen aus derselben Installation stammen (`VERB-HOSTPROZESS-VERSION`).
3. `se screen bench` gegen ein Gerät liefert nur `decode` und `capture_to_arrival` (`VERB-BENCH-STATIONEN`).
4. Vier Zahlenwerte sind Schätzungen bis zum Suite-Lauf: T, `DRIVER_RATE`, Sendebündel 40/80 und die Puffergröße der Pumpe mit ihrem Budget (`VERB-MESSUNG`);
   die Messungen M2 und M3 nutzen Stellvertreter für den Bildstrom und die Listenanfrage.
5. Eingehende Verbindungen bekommen die großen Sendebündel nur bei lesbarer ALPN im ersten Initial-Paket (`VERB-EINGANG-BUENDEL`).
6. Das Verhalten von Graphics Capture beim Umschalten der Zeigeraufnahme auf stehendem Bildschirm ist eine Annahme (`VERB-WGC-UMSCHALTEN`).
7. Die bedingten Pakete B1 bis B5 sind nicht gebaut; ihre Bedingungen entscheidet der Suite-Lauf (`VERB-BEDINGT`).
8. Der Umbau der Bildkette nach `bildkette.md` ist der nächste Batch (`VERB-BILDKETTE`, Release 2); BK0 bis BK12 sind nicht begonnen.
9. Das Paket P8 ist umgesetzt (Stand 2026-11-14): Registerzeilen in `docs/tests/register.tsv`, die Teile `verbindungen` und `verbindungen-gesamt` in `native/test-crates-task.py` mit den Regeln in `native/crates_task_verbindungen.py`, Workflow `.github/workflows/crates-task.yml`, Abschnitt in `docs/RELEASING.md`. Danach sind die Lücken des zweiten Integrationsdurchgangs des Vorauftrags geschlossen (Paket E2, Testzeilen im Register und in den Listen der Suite). Nicht gebaut, nicht getestet, nicht committet.

**Nächste Schritte.** Integration; Commits je Meilenstein mit ihren Registerzeilen; Push; ein Lauf `crates-task.yml` mit `part=suite`;
Auswertung der Zeilen `VERBINDUNGEN-*`; Nachsteuerung N1; derselbe Lauf noch einmal zur Bestätigung; ein Release über `release.yml`. Ein Eintrag
der Messwerte in ARCHITEKTUR, TRANSFER_ENGINE und `VERB-MESSUNG` folgt nach dem Suite-Lauf.

## Schleifenstand

- Ziel: Transportaufbau je Komponente, der „möglichst schnelle konstante verbindungen“ liefert (Nutzer 2026-11-14), und die Optimierung der Bildübertragung je Station (Nutzer 2026-11-14, Nachricht 6). Die unverschlüsselte Bildschirmübertragung (Nachricht 5) hat der Nutzer am 2026-11-14 zurückgenommen („ah dann nehme ich es zurück, dann kannst von mir aus auch das unverschlüsselt ganz weg lassen“); sie entfällt (C4).
- Kriterien (Herkunft Nutzer 2026-11-14: „sicherheit, verlässlichkeit, geschwindigkeit oder konsistenz“; frühere Vorgaben), Status:
  - Geschwindigkeit je Komponente an der Grenze des knappsten Mittels: umgesetzt im Arbeitsbaum (C3, E5, E7, C5); Nachweis M1, M4, M6 offen.
  - Konstanz (Bildschirm unabhängig von gleichzeitigem Dateitransfer): umgesetzt (E2, E3, E4); Nachweis M2 offen.
  - Sicherheit nicht schwächer als heute: erfüllt (C8); jede Verbindung bleibt voll verschlüsselt, auch die Bildschirmverbindung (C4).
  - Keine Handgriffe, keine Adminrechte, kein Systemadapter: im Plan erfüllt (C1, X-ADAPTER).
  - Windows, Linux, Android: im Plan erfüllt; Zeigerüberlagerung ohne Wayland- und Android-Gastgeber (Grund in C5, offen als `VERB-WAYLAND-ZEIGER`).
  - Direktweg- und Relay-Regeln bleiben: erfüllt (C8, C9).
- Möglichkeiten (bewertet in C): VPN im Programm (verworfen, C1); VPN mit Systemadapter (verworfen); getrennte Verbindungen je Zweck mit eigener Regelung (gewählt, E1 bis E5); QUIC-Datagramme mit FEC für das Bild (bedingt, E11); parallele Verbindungen für große Übertragungen (Bestand, Schwelle gemessen); Paketgröße und Bündel (Bündel gewählt, MTU verworfen); Klartextbild (vom Nutzer zurückgenommen, C4); Stationsverbesserungen (Zeigerüberlagerung gewählt, Hardwarekodierung und Schlüsselbild-Ersatz bedingt).
- Aktiver Stand (2026-11-14, nach P8): P1, P2, P2b, P3, P4, P5a (reduziert), P6, PX, P7, beide Reviews (Bild, Transport), P9 und P8 sind im Arbeitsbaum umgesetzt beziehungsweise geschrieben, ebenso die Schließung der Lücken des zweiten Integrationsdurchgangs des Vorauftrags; nichts gebaut, getestet, committet oder veröffentlicht; P5b und P5c entfallen. Einzelheiten und offene Punkte: Abschnitt E.
- Nächster Schritt: Integration, Commits, Push, ein Suite-Lauf, Nachsteuerung N1, ein Release (D1 Welle 8, D5). Der Umbau der Bildkette (`bildkette.md`) folgt als eigener Batch nach dem Release.
- Schleifenstatus: aktiv (Umsetzung läuft; Wirkungsnachweise warten auf den Suite-Lauf).
