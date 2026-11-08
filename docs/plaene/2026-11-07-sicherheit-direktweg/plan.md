# Plan Sicherheit und Direktweg, Stand 2026-11-07

Gliederung: 1 Ziel; 2 Anforderungen mit Quelle; 3 Bestand (an der Quelle geprüft); 4 Recherche; 5 Entscheidungen mit verworfenen Alternativen; 6 Meilensteine; 7 Zu erhaltendes Verhalten; 8 Sicherheit; 9 Arbeitspakete, Flächen und Reihenfolge; 10 Fragen an den Nutzer (nicht blockierend).

Pfadkürzel: `ENG` = `native/crates/engine/src`, `SC` = `ENG/share/core`, `SOS` = `ENG/share/os/shared`, `SRV` = `share-server/src`, `VND` = `vendor`. Alle Projektpfade relativ zu `/home/CodeProjects/smartexplorer/smart-explorer`.

## 1 Ziel

Ein Batch, der (a) die Sicherheitsreviews aller von außen erreichbaren Eingänge und lokalen Steuerflächen des Programms durchführt und jeden berechtigten Befund ohne Funktionsverlust behebt (s1), (b) die Wegwahl und Übertragung auf Fehler prüft, die den Direktweg verhindern oder verlangsamen, und sie behebt (s2), (c) die belegte Lücke schließt, dass Geräte hinter NAT ihre öffentliche Adresse beim eigenen Server nicht erfahren und darum ohne UPnP/PCP/NAT-PMP keinen Direktweg über verschiedene Netze und Mobilfunk aufbauen können (s3), und (d) die Machbarkeit einer je Übertragung während des Laufs umschaltbaren unverschlüsselten Übertragung mit belegtem Ergebnis dokumentiert (s4). Danach eine Task-Suite über Remote-CI und ein Release (Manager).

Konkrete Lieferungen: QAD-Dienst im `se-share-server` mit Ressourcengrenzen und Client-Konfiguration (Paket `qad`); sieben Review-Befundlisten (Pakete `r-*`); Korrekturen je Bereich (Pakete `f-*`, `f-rest`); Tests und Registerzeilen für jede Änderung (Paket `suite`); Plan im Repository (Paket `plandoku`); Dauerdokumentation und Machbarkeitsbericht s4 im Repository (Paket `doku`).

## 2 Anforderungen mit Quelle

Quelle aller Anforderungen: Nutzernachrichten vom 2026-11-07, Nachricht 1 (s1 bis s4) und Nachricht 2 (s5).
- s1: „sicherzustellen, dass das Programm selbst noch alles sicher ist, so sicher wie möglich. Ohne dass du jetzt die Funktionalität einschränkst.“ „dass externer Zugriff, der nicht erwünscht ist, nicht möglich ist.“
- s2: „dass die Datenübertragungen so schnell und effizient wie möglich in allen Fällen passieren auf dem möglichst direkten Weg. Der Server dient weiterhin als DNS-Server de facto oder als Adress-Server für die Share-Geräte. Und soll möglichst Nicht als Relay verwendet werden, außer es ist wirklich zwingend erforderlich und keine Direktverbindung ist möglich.“
- s3: „Dabei muss eine Direktverbindung nach Möglichkeit über WLAN, über LAN, über unterschiedliche WLANs, über unterschiedliche Netzwerke mit dem Handy im Mobilfunknetz. Aufgebaut werden, also in allen Situationen, in denen sich die Endgeräte befinden können, soll es möglich sein, eine Direktverbindung aufzubauen.“
- s4: „Datenübertragung passiert in der Regel aktuell verschlüsselt, das gefällt mir auch.“ „Will ich das, dass wenn die Datenübertragung dadurch verlangsamt wird, man die Möglichkeit hat, Daten unverschlüsselt zu übertragen? Äh. Vielleicht will ich das. Schau mal, ob das umsetzbar ist, dass man das dann nach Bedarf umschalten kann, während eine Übertragung läuft. Für diese eine Übertragung.“
- s5 (Nachricht 2, Ablaufvorgabe für den Manager, kein Planinhalt): ab 85 % des Nutzungslimits startet der Manager keinen neuen Agenten.

## 3 Bestand (an der Quelle geprüft)

Grundlage: die Bestandsaufnahme des Batches (nicht im Repository). Jede handlungsbestimmende Angabe wurde an der Quelle nachgelesen; Ergebnis:

| Nr | Angabe aus der Bestandsaufnahme | Prüfung an der Quelle (dieser Lauf) | Ergebnis |
|---|---|---|---|
| B1 | Relay des Servers ohne QUIC-Adressdienst (QAD) | `SRV/relay.rs:130-131` (`ServerConfig::default()`, nur `config.relay` gesetzt); `VND/iroh-relay-1.0.0/src/server.rs:117-125` (`quic: Option<QuicConfig>`, `Default` = `None`) | bestätigt |
| B2 | Client erwartet QAD am Relay auf UDP 7842 | `SC/node.rs:150-151` (`RelayMode::custom(urls)`); `VND/iroh-1.3.0/src/endpoint.rs:2051-2054` (`RelayMap::from_iter`); `VND/iroh-relay-1.0.0/src/relay_map.rs:272-286` (`From<RelayUrl>` setzt `quic: Some(RelayQuicConfig::default())`); `VND/iroh-1.3.0/src/net_report/reportgen.rs:648-659` (Port 0 → `DEFAULT_RELAY_QUIC_PORT`); `VND/iroh-relay-1.0.0/src/server.rs:531` (7842) | bestätigt |
| B3 | Öffentliche Adresse (`global_v4/v6`) kommt nur aus QAD; daneben Portmapper, lokale Interfaces, konfigurierte Adressen | `VND/iroh-1.3.0/src/socket.rs:1823-1890` (`update_direct_addresses`); `VND/iroh-1.3.0/src/net_report/report.rs:32-34,88-127` (nur QAD-Berichte setzen `global_v4/v6`) | bestätigt; offene Frage 1 aus der Bestandsaufnahme damit beantwortet: ohne QAD lernt ein Gerät hinter NAT seine öffentliche Abbildung nur über den Portmapper |
| B4 | QAD-Sonde nutzt denselben Socket wie die Peer-Verbindungen und dieselbe CA-/Pin-Konfiguration | `VND/iroh-1.3.0/src/socket.rs:1037-1050` (`QuicConfig { ep: endpoint.clone(), client_config: tls_config.clone() }`); `VND/iroh-1.3.0/src/net_report.rs:826-860` (`create_conn(relay_addr, host)`, Servername = Host der Relay-URL) | bestätigt; gepinnte selbstsignierte Serverzertifikate (`with_server_trust`) gelten auch für QAD |
| B5 | QAD-Server von iroh-relay: ALPN `/iroh-qad/0`, 0 Streams, `send_observed_address_reports(true)`, keine Verbindungsobergrenze, unbegrenztes `JoinSet` | `VND/iroh-relay-1.0.0/src/quic.rs:9-10,97-160` | bestätigt; zusätzlich: der QAD-Client hält die Verbindung mit 25 s Keepalive und 35 s Leerlauffrist offen (`quic.rs:285-302`), je Gerät bis zu eine Verbindung je Adressfamilie |
| B6 | QAD braucht TLS 1.3; ohne eigenes `server_config` übernimmt es das TLS des Relays, ohne beides schlägt der Start fehl | `VND/iroh-relay-1.0.0/src/server.rs:528-553,986-1000` | bestätigt; ein Fehler beim QAD-Start bricht `Server::spawn` ganz ab (Fehler `SpawnError::QuicSpawn`) |
| B7 | Peer-Adressberichte (`send/receive_observed_address_reports`) setzt se nicht | `grep` in `native/crates`: keine Treffer; `VND/iroh-1.3.0/src/endpoint/quic.rs:461-473` | bestätigt |
| B8 | Portmapper aktiv, nirgends konfiguriert | `native/Cargo.toml:70` (laut Bestandsaufnahme; `default-features = true`); `grep portmapper_config native/crates`: keine Treffer | bestätigt |
| B9 | Maschinenanmeldung nutzt standardmäßig die öffentlichen n0-Relays | `SC/login_transport.rs:43-53` (`use_default_relays` → `RelayMode::Default`); `ENG/login/core/gateway_types.rs:17-28` (Standard `true`) | bestätigt; Wirkung: Anmelde-Endpunkt erhält QAD von n0, der Share-Endpunkt nicht |
| B10 | Client-Relay-URLs: nur `https`, `http` nur bei Klartext-Erlaubnis; `SE_SHARE_RELAY_URL`, `SE_SHARE_RELAY_ONLY` | `SOS/transport_options.rs:17-18,53-103` | bestätigt |
| B11 | Suite-Pflicht: `crates-task.yml` führt alle Rust-Tests, `--locked`, Teil `servers` mit `share-server` | `.github/workflows/crates-task.yml:1-25`; `native/test-crates-task.py:346,565` | bestätigt; neue Cargo-Abhängigkeiten würden eine Lock-Änderung verlangen, die ohne lokales Cargo nicht sicher erzeugbar ist |
| B12 | Verschlüsselung der Share-Wege: QUIC/TLS 1.3, kein Klartextpfad in iroh/noq | Bestandsaufnahme s4.1 (Quellen `VND/iroh-1.3.0/src/tls.rs`, `VND/noq-proto-1.3.0/src/crypto.rs:1-9`) | übernommen; nicht erneut zeilengenau gelesen (nicht handlungsbestimmend für Code dieses Batches, nur für den Machbarkeitsbericht) |
| B13 | Pro Gegenstelle eine QUIC-Verbindung für alle Übertragungen (Sitzungs-Cache) | Bestandsaufnahme s4.3 (`SC/session.rs:301-304`, `SC/node_sessions.rs:265-330`) | übernommen; für s4 entscheidend: Paketschlüssel gelten je Verbindung, nicht je Stream oder Übertragung |
| B14 | `--require-key-login` standardmäßig aus (Altclients) | Bestandsaufnahme s1.2 (`SRV/config.rs:49-58`) | übernommen; Prüfung im Review `r-server` |

Weitere Angaben aus der Bestandsaufnahme (s1.2 Angriffsflächen, s1.3 Kontrollen, s2.3 Wegwahl) sind Prüfgegenstand der Reviews und werden dort an der Quelle geprüft.

## 4 Recherche

Stufe 1 (Ansatz):
- QAD ist der Weg, über den iroh seit 0.32 die öffentliche Adresse lernt; früher STUN (Quelle: iroh-Blog „noq, noq, who's there?“, https://iroh.computer/blog/noq-announcement; docs.rs iroh 0.95.1, https://docs.rs/crate/iroh/=0.95: „relay server provides QUIC Address Discovery (QAD) and hole-punching assistance“). Am Quelltext belegt: B2–B5.
- Vergleich der Wege zur öffentlichen Adresse: (1) QAD am eigenen Relay (vom Client bereits erwartet, B2); (2) STUN-Dienst: iroh 1.3.0 hat keinen STUN-Client mehr (Suche `stun` nur in Kommentaren `socket.rs:1846`); (3) Peer-Adressberichte (B7): liefern die beobachtete Adresse nur über einen bereits bestehenden IP-Pfad, über den Relay-Pfad keine öffentliche IP – lösen das Henne-Ei-Problem nicht; (4) n0-Relays (wie B9): Drittanbieter, widerspricht „Server dient als Adress-Server“ und dem Direkt-zuerst-Prinzip der eigenen Infrastruktur. Wahl: (1).
- Unverschlüsseltes QUIC: IETF-Entwurf draft-banks-quic-disable-encryption-00 (N. Banks, 2020, abgelaufen, experimentell; https://datatracker.ietf.org/doc/html/draft-banks-quic-disable-encryption-00): Transportparameter `disable_1rtt_encryption`, beim Handshake für die ganze Verbindung ausgehandelt; entfernt Header-Schutz, Payload-Schutz und AEAD-Tag (16 Byte je Paket), Integrität dann nur über UDP-Prüfsumme; Zielumgebung „trusted paths“; Gefahr: ein Angreifer in der Mitte kann versuchen, die Erweiterung zu erzwingen. Folgerung: per Verbindung, nicht während des Laufs und nicht je Übertragung umschaltbar; ohne Integrität.
- TLS 1.3 kennt Integrity-only-Suiten (RFC 9150, `TLS_SHA256_SHA256`), QUIC verlangt aber AEAD und daraus abgeleiteten Header-Schutz (RFC 9001, Abschnitt 5); rustls bietet RFC 9150 nicht an (Annahme, nicht am rustls-Quelltext gelesen). Folgerung: kein Weg innerhalb der bestehenden QUIC-Verbindung.

Stufe 2 (Lücken):
- QAD-Ressourcen: Die Client-Verbindungen sind langlebig (B5); eine Obergrenze muss also bestehende Verbindungen zählen, nicht nur Handshakes. `noq::ServerConfig::max_incoming` (`VND/noq-proto-1.3.0/src/config/mod.rs:327`) begrenzt nur wartende Eingänge. Folgerung: Zählung im Annahmeloop von `VND/iroh-relay-1.0.0/src/quic.rs` nötig (Vendor-Patch), weil `quic::server` `pub(crate)` ist (`quic.rs:17`) und ein eigener QAD-Server im `share-server` eine direkte `noq`-Abhängigkeit bräuchte (B11: Lock-Änderung ohne lokales Cargo).
- QAD bei Klartext-Servern: QUIC braucht TLS (B6); der Client prüft das Zertifikat gegen den Host der Relay-URL (B4). Bei `http://`-Relays ist QAD nicht möglich; der Client soll dann keine QAD-Sonde starten (`RelayConfig::new(url, None)`, `relay_map.rs:249-256`), statt bei jedem Netzbericht auf eine Zeitüberschreitung zu laufen.
- Reverse-Proxy-Betrieb (`docs/SHARE_SERVER.md:340-361`): Der Proxy beendet TLS über TCP; UDP 7842 läuft nicht über ihn. QAD braucht dann ein eigenes Zertifikat am `share-server` (Betreiber gibt `--tls-cert/--tls-key` zusätzlich an) und eine Freigabe von UDP 7842 in der Firewall. Dokumentationspflicht (Paket `doku`).
- s4 Leistungsbezug: Bestandsschätzung „ein Treiber-Task = ein Kern für Paketbau + AES-GCM; 2–5 Gbit/s Desktop“ (`docs/plaene/2026-10-28-verbinden-ist-einrichtung/analyse-g2-geraete.md:40-47`); eine Messung „mit/ohne Verschlüsselung“ gibt es im Repository nicht (Bestandsaufnahme s4.2). Annahme (nicht gemessen): AES-GCM mit AES-NI bzw. ARMv8-Kryptobefehlen kostet einen Bruchteil der Paketverarbeitung; der Gewinn eines Klartextwegs ist bis 1 Gbit/s nicht messbar und darüber durch die Paketverarbeitung je Verbindung begrenzt.

## 5 Entscheidungen mit verworfenen Alternativen

- E1 (s3) Öffentliche Adresse über QAD am eigenen `se-share-server`. Verworfen: n0-Relays (Drittanbieter, widerspricht der Server-Rolle aus s2); Peer-Adressberichte allein (lösen den Erstkontakt nicht); eigener STUN-Dienst (iroh 1.3.0 hat keinen STUN-Client).
- E2 QAD über `iroh_relay::server::ServerConfig::quic` mit dem TLS des Relays; Grenzen per Vendor-Patch in `VND/iroh-relay-1.0.0/src/quic.rs`, Patchhinweis in neuer Datei `vendor/SMART_EXPLORER_QAD_LIMITS.md`. Verworfen: eigener QAD-Server im share-server mit direkter `noq`-Abhängigkeit (Lock-Änderung unter `--locked` ohne lokales Cargo, B11); QAD ohne Grenzen (unbegrenzte langlebige Verbindungen = Speicher-DoS).
- E3 Grenzen abgeleitet, nicht frei gewählt: global höchstens `2 × RELAY_MAX_ACTIVE_CONNECTIONS` (= 1024; je Gerät eine Verbindung je Adressfamilie), je Quell-IP höchstens `2 × RELAY_MAX_TCP_CONNECTIONS_PER_SOURCE` (= 128; CGNAT bündelt viele Geräte hinter einer IP); Annahme-Rate wie das Relay (`RELAY_ACCEPTS_PER_SECOND`/`_PER_SOURCE`, `SRV/relay.rs:18-21`). Überzählige Eingänge werden mit `refuse()` abgewiesen. Leerlauffrist des Servers 35 s (gleich der Client-Frist, `quic.rs:298-300`).
- E4 Bindung: UDP auf derselben IP wie das Relay, Port 7842 (der Port, den jeder Client ohne Konfiguration erwartet, B2); Umgebung `SE_IROH_QAD_BIND` überschreibt, `SE_IROH_QAD_DISABLE=1` schaltet ab; ohne Relay-TLS kein QAD (Logzeile mit Grund). Verworfen: Port N+2 neben Signaling/Relay (Clients müssten den Port kennen; alte Clients fänden ihn nie).
- E5 Fehler beim QAD-Start beendet den Server nicht: Relay und Signaling starten ohne QAD, die Logzeile nennt den Grund. Begründung: QAD ist eine Wegverbesserung, keine Schutzfunktion; ein Abbruch würde bestehende Installationen (belegter UDP-Port) unbenutzbar machen („ohne Funktionalität einschränken“). Verworfen: fail-closed wie der Relay-Start.
- E6 Client: für `http://`-Relays `RelayConfig::new(url, None)` (keine QAD-Sonde), für `https://` QAD mit Standardport. Gilt für den Share-Endpunkt (`SC/node.rs`) und den Anmelde-Endpunkt mit eigenen Relay-URLs (`SC/login_transport.rs`). Die Standardwahl der n0-Relays für die Maschinenanmeldung (B9) bleibt unverändert (gespeicherte Bedeutung von `use_default_relays`); Bewertung im Review `r-lokal`.
- E7 Portmapper bleibt an (bringt Direktwege hinter UPnP/PCP/NAT-PMP-Routern; eingehende Pakete brauchen weiterhin TLS-authentifizierte Peers). Dokumentation in `docs/SHARE_SERVER.md`/`docs/ARCHITEKTUR.md`. Verworfen: abschalten (nähme Direktwege).
- E8 (s4) Kein unverschlüsselter Nutzdatenweg in diesem Batch; Lieferung ist der Machbarkeitsbericht. Ergebnis der Prüfung: (a) Innerhalb der bestehenden QUIC-Verbindung unmöglich, weil Schlüssel je Verbindung gelten und mehrere Übertragungen eine Verbindung teilen (B13), QUIC AEAD verlangt und iroh/noq keinen Klartextpfad haben (B12). (b) Technisch umsetzbar nur als zweiter Datenkanal je Übertragung: eine eigene, über die verschlüsselte Verbindung ausgehandelte QUIC-Verbindung mit Null-Paketschlüssel (Vendor-Patch an `noq-proto` nach draft-banks) oder ein TCP-Strom im LAN, mit Integritätsschutz je Block (Schlüssel aus dem TLS-Exporter der verschlüsselten Verbindung) und Umschaltung zwischen Blockbereichen über die vorhandenen Bereichsanfragen (`ranges_v1`). (c) Erwarteter Gewinn gering (Annahme, Abschnitt 4), Sicherheitsverlust hoch (Inhalt für Relay-Betreiber, WLAN-Mithörer und Router sichtbar; Klartext über das Relay widerspräche `docs/SHARE_SERVER.md:3-8`). Empfehlung: zuerst die Kosten der Verschlüsselung auf den Zielgeräten messen; Umsetzung nur auf Nutzerentscheidung (Abschnitt 10). Verworfen: sofortige Umsetzung (Nutzer: „Vielleicht will ich das. Schau mal, ob das umsetzbar ist“; Kryptografiegrenze mit hohem Risiko); per Verbindung nach draft-banks (nicht je Übertragung, nicht während des Laufs, abgelaufener Entwurf).
- E9 (s1) Reviews nach Angriffsfläche geschnitten, je Bereich ein Review- und ein Korrekturpaket mit derselben Fläche; Korrekturen nur für Befunde, die die Funktionalität nicht einschränken; ein Befund, dessen Abhilfe Funktion kostet, wird nicht umgesetzt, sondern als Nutzerfrage gemeldet. Verworfen: ein Gesamtreview (Fläche > 1000 Dateien, nicht in einem Kontext prüfbar); Review und Korrektur im selben Agenten (Projektregel: Analyse und Umsetzung getrennt).
- E10 Task-Suite: `crates-task.yml` (Teil `suite`), weil sie seit 2026-11-07 jeden Rust-Test führt (`docs/RELEASING.md`, „Task suites since 2026-11-07“); keine neue Workflow-Datei. Tests mit Umgebungsbedarf tragen `#[ignore = "<need>; Suite crates-task"]` nur, wenn crates-task diese Umgebung in `IGNORED_GROUPS` baut; QAD-Tests laufen ohne Umgebungsbedarf auf Loopback. Verworfen: eigene Suite (keine echten Geräte/Netze nötig).

## 6 Meilensteine

Jeder Meilenstein nennt Dateien und das prüfbare Ergebnis; geprüft wird alles gemeinsam im einen Suite-Lauf (Paket `suite` schreibt die Tests, der Manager startet die Suite).

- M1 QAD-Dienst im Server (Paket `qad`). Dateien: `SRV/relay.rs`, neue Datei `SRV/relay_qad.rs`, `VND/iroh-relay-1.0.0/src/quic.rs`, neue Datei `vendor/SMART_EXPLORER_QAD_LIMITS.md`. Ergebnis: Mit `--tls-cert/--tls-key` meldet der Server eine Logzeile „QAD listening on <udp-adresse>“, `Server::quic_addr()` ist gesetzt; ein QAD-Client (`iroh_relay::quic::QuicClient`) auf Loopback erhält als beobachtete Adresse genau die Adresse seines eigenen Sockets.
- M2 QAD-Abschaltung und Fehlerverhalten (Paket `qad`). Dateien wie M1. Ergebnis: `SE_IROH_QAD_DISABLE=1` → kein UDP-Listener, Signaling und Relay laufen; belegter UDP-Port → Logzeile mit Grund, Signaling und Relay laufen, Prozess beendet sich nicht; ohne TLS → kein QAD, Logzeile „QAD needs TLS“.
- M3 QAD-Grenzen (Paket `qad`). Datei `VND/iroh-relay-1.0.0/src/quic.rs`. Ergebnis: bei erreichter globaler bzw. Quell-Grenze wird die nächste Verbindung abgewiesen und eine bestehende bleibt erhalten; die Zahl der gehaltenen Verbindungen überschreitet die Grenze nie; nach dem Ende einer Verbindung wird ein neuer Eingang wieder angenommen.
- M4 Client-QAD-Wahl (Paket `qad`). Dateien `SC/endpoint_routes.rs`, `SOS/transport_options.rs`, `SC/node.rs`, `SC/login_transport.rs`. Ergebnis: Relay-Karte aus `https://`-URL enthält `quic: Some(Port 7842)`, aus `http://`-URL `quic: None`; das Verhalten von `SE_SHARE_RELAY_URL`/`SE_SHARE_RELAY_ONLY` und der Pins bleibt unverändert (bestehende Tests `review_task_plain_relays_are_dropped_without_the_plaintext_permission`, `review_task_relay_override_keeps_urls_and_reads_addresses` bleiben grün).
- M5 Direkter Kandidat über QAD (Paket `suite`, Integrationstest). Ergebnis: Ein se-Endpunkt mit Relay-URL eines lokal gestarteten `se-share-server` (TLS mit Testzertifikat, Pin) führt nach `bind` unter seinen Direktadressen einen Eintrag vom Typ `Qad` (Iroh-Typ `DirectAddrType::Qad`); ohne QAD am Server fehlt er. Wenn die öffentliche API diesen Typ nicht zugänglich macht, prüft der Test über `Endpoint::net_report()`/`global_v4` (Paket `suite` wählt den zugänglichen Weg und nennt ihn).
  - Gewählter Weg (Stand 2026-11-07): Weder der Adresstyp `Qad` noch `Endpoint::net_report()`/`global_v4` sind für die Engine erreichbar; `net_report()` gibt es nur mit dem Iroh-Feature `unstable-net-report` (`VND/iroh-1.3.0/src/endpoint.rs`, Zeile 1460), das `native/crates/engine/Cargo.toml` nicht einschaltet. Ersatz: Der Linux-Test `qad_direct_task_relay_with_a_quic_port_is_probed_and_one_without_is_not` (`SC/qad_direct_task_tests.rs`) baut die Relay-Karte mit der Produktionsfunktion `relay_mode` für das Testrelay aus `iroh::test_utils::run_relay_server_with(true)` (nur dessen QUIC-Port eingesetzt) und verlangt, dass der Zähler `qad_connections` des Testrelays über 0 steigt; die Gegenprobe mit `RelayConfig::new(url, None)` verlangt `qad_incoming` und `qad_connections` gleich 0. Damit ist belegt, dass der Endpunkt die QAD-Sonde aufbaut; dass `global_v4` gesetzt wird, ist nicht belegt. Die Antwort des eigenen Servers (beobachtete Adresse gleich der Socket-Adresse des Clients) prüfen `qad_reports_the_clients_own_socket_address` (`SRV/relay_qad_tests.rs`) und `qad_limits_end_to_end` (`SRV/relay_qad_limits_tests.rs`).
- M6..M12 Review-Korrekturen je Bereich (Pakete `f-zugang`, `f-exec`, `f-fs`, `f-server`, `f-lokal`, `f-entdeckung`, `f-weg`, `f-rest`). Dateien: Fläche des Pakets (Abschnitt 9). Ergebnis je Befund: in der Befundliste des Korrekturpakets genannte prüfbare Erwartung (Eingabe, erwartete Abweisung oder erwartetes Verhalten), die das Paket `suite` als Test abbildet.
- M13 Machbarkeitsbericht s4 und Dauerdoku (Paket `doku`). Dateien: neue Datei `docs/plaene/2026-11-07-sicherheit-direktweg/machbarkeit-unverschluesselt.md`, `docs/SHARE_SERVER.md`, `docs/ARCHITEKTUR.md`, `docs/TODO.md`, `README.md` nur falls Installationsschritte (Firewall UDP 7842) dort stehen. Ergebnis: Stale-Suche nach „QAD“, „7842“, „ciphertext“ zeigt konsistente Aussagen; TODO enthält eine Zeile für die s4-Nutzerentscheidung und für die Messung der Verschlüsselungskosten.
- M14 Plan im Repository (Paket `plandoku`). Datei: neue Datei `docs/plaene/2026-11-07-sicherheit-direktweg/plan.md` (Inhalt dieses Plans). Ergebnis: Datei existiert, enthält Abschnitte 1–10.
- M15 Suite (Paket `suite`). Dateien: Testdateien neben den geänderten Modulen, `share-server/tests/relay_startup.rs`, `docs/tests/register.tsv`. Ergebnis: jeder Meilenstein M1–M12 hat mindestens einen Test oder ist als „nur Doku“ begründet; `native/test-register.py` findet zu jedem Test eine Zeile.

## 7 Zu erhaltendes Verhalten

- Alle Datenwege (Dateien, Laufwerke, Sync über Direktfreigaben, Exec/Terminal, Bildschirm, TCP-Weiterleitung, Programme, LAN-Link, Maschinenanmeldung) über denselben Endpunkt und dieselbe Routenpolitik LAN → Direkt → Relay inklusive Startbrücke Z-R2 (`docs/ARCHITEKTUR.md:13-18`, `SC/route_start.rs`).
- Relay-Start fail-closed (`docs/SHARE_SERVER.md:55-58`), `SE_IROH_RELAY_DISABLE`, `SE_IROH_RELAY_BIND`, Port N+1; Signaling-Port N; Klartext nur mit `--allow-plaintext`/„Unverschlüsselt erlauben“; TLS-Fehler fallen nie auf Klartext zurück.
- Relay nur für registrierte Endpunkte (`SRV/relay_access.rs`), alle Relay-Grenzen (`SRV/relay.rs:14-30`).
- Wire-Formate, Capability-Flags, `RouteStatus`-Kompatibilität (`relay_start`-Flag), `RemoteRouteGuarantee`; Altclients ohne `key_login_v1` im Kompatibilitätsmodus; Exec-Grants älterer Gegenstellen.
- Gespeicherte Bedeutungen: Serveradressen, `use_default_relays`, `relay_pins`, Exec-Grants, Freigaben, Explorer-Orte mit Backend- und Verbindungsidentität für lokal, UNC, SFTP/SSH-Agent, FTP/FTPS, WebDAV, Google Drive, Direct/Room Share.
- Verschlüsselung aller Share-Nutzdaten Ende-zu-Ende (Standard bleibt, Nutzer: „das gefällt mir auch“).
- Jede Oberfläche (CLI `se`, Desktop-GUI, Android-App, Android-Bridge, Daemon-IPC, MCP, Netzprotokolle, Doku) zeigt geänderte Fähigkeiten gleich (`docs/oberflaechen/INDEX.md`).
- Abbruch, Rechteentzug, Backups, Konflikte, Wiederholungen, Link-Grenzen beim Löschen.

## 8 Sicherheit

Je sicherheitsrelevanter Eigenschaft dieses Auftrags:

### 8.1 QAD-Dienst (neuer Eingang am Server, UDP)
- Vertrauensgrenze: beliebiger Internet-Absender → QUIC-Listener des `se-share-server`. Der Dienst gibt nur die beobachtete Absenderadresse an genau diesen Absender zurück; keine Streams (0 bidi, 0 uni), keine Daten anderer Clients.
- Berechtigungsprüfung vor jeder Wirkung: keine Identität verfügbar (QAD-Client schickt kein Client-Zertifikat); deshalb keine Wirkung außer der Adressmeldung. Annahme erst nach Prüfung von Rate und Grenzen (E3) im Annahmeloop vor `accept`.
- Fehler: Start-Fehler → QAD aus, Rest läuft (E5, bewusst nicht fail-closed, weil keine Schutzfunktion). Fehler einer Verbindung → nur diese endet; ein Panic einer Verbindungsaufgabe darf den Server nicht beenden (heute `panic!` in `quic.rs:137`; Patch: Fehler protokollieren, Verbindung verwerfen).
- Abbruch/Aufräumen: Verbindungszähler wird beim Ende jeder Verbindungsaufgabe verringert (Drop-Guard); Server-Shutdown schließt alle (vorhanden, `quic.rs:155-170`).
- Grenzen: global 1024 Verbindungen, je Quell-IP 128, Annahmerate wie Relay, Leerlauf 35 s, `max_incoming` und Puffer von noq auf Standard; 0 Streams.
- Missbrauch und Abwehr: Verstärkung (QUIC-Anti-Amplification 3×, RFC 9000 §8; Retry nicht nötig); Verbindungsflut (Grenzen, Rate); Adressausspähung Dritter (unmöglich, Meldung nur an den Absender); gefälschte Adresse an Clients durch Angreifer in der Mitte (TLS mit Zertifikats-/Pinprüfung, B4).
- Abnahmesignal Suite: Tests M1–M3 (beobachtete Adresse = eigene, Grenze hält, Abschaltung, Portkonflikt ohne Prozessende).

### 8.2 Client-QAD-Wahl
- Vertrauensgrenze: Client → Relay-Host über UDP. Prüfung: dieselbe CA-/Pin-Prüfung wie der Relay-Weg (B4); `http://`-Relays ohne QAD (E6).
- Fehler: QAD-Sonde scheitert → keine öffentliche Adresse, Verhalten wie heute (fail-safe, kein Rückfall auf ungeprüfte Quellen).
- Grenzen: von iroh (eine Verbindung je Adressfamilie).
- Missbrauch: veröffentlichte Presence enthält jetzt die öffentliche Adresse; Presence ist signiert und geht nur an berechtigte Beziehungen (Bestandsaufnahme s2.2). Folge: Gegenstellen erfahren die öffentliche IP (heute schon bei Portmapper). Im Review `r-entdeckung` prüfen, dass Presence nur an akzeptierte Kontakte geht.
- Abnahmesignal: Test M4, M5.

### 8.3 Review-Korrekturen (s1)
- Vertrauensgrenzen je Bereich: Peer → QUIC-Eingang (r-zugang); gekoppelter Peer → Exec/Screen/TCP/Apps (r-exec); gekoppelter Peer → Dateisystem des Hosts (r-fs); Internet → Signaling/Relay/QAD (r-server); lokaler Prozess/anderer Benutzer/App → IPC, MCP, Updater, Android-Komponenten (r-lokal); LAN/Server → Entdeckung, Presence, Kopplung (r-entdeckung); Netz → Wegwahl, Relay-Nutzung (r-weg).
- Jede Befundliste nennt je Eingang: Vertrauensgrenze, Prüfung vor Wirkung, Fehlerverhalten (fail closed?), Abbruch/Widerruf/Aufräumen, Grenzen für Speicher/Zeit/Anzahl, Missbrauchsfälle mit Abwehr, vorgeschlagenes Abnahmesignal.
- Korrekturen schränken keine Funktion ein; eine Abhilfe mit Funktionsverlust wird als Nutzerfrage gemeldet (E9).
- Abnahmesignal: je Befund ein Test (Paket `suite`), der den Angriff nachstellt und die Abweisung prüft, plus der Erhalt des berechtigten Falls.

### 8.4 s4 unverschlüsselte Übertragung
- Keine Umsetzung in diesem Batch (E8); der Machbarkeitsbericht nennt für eine spätere Umsetzung: Vertrauensgrenze (jedes Netzglied zwischen den Geräten), Pflicht-Integritätsschutz je Block, nur nach ausdrücklicher Wahl je Übertragung, nie über das Relay, Rückschaltung jederzeit, Anzeige „⚠ unverschlüsselt“ auf allen Oberflächen, Gegenstelle muss zustimmen.
- Abnahmesignal: Dokument existiert und nennt Ergebnis, Bedingungen und Messplan.

## 9 Arbeitspakete, Flächen und Reihenfolge

Flächen der Bereiche (Präfixe von Dateinamen; „Fläche X“ in den Aufträgen):
- Fläche zugang: `SC/node.rs`, `SC/node_*.rs`, `SC/session*.rs`, `SC/auth_penalty*.rs`, `SC/fair_admission*.rs`, `SC/handshake_limits*.rs`, `SC/pq_kx*.rs`, `SC/wire_capabilities*.rs`, `SC/identity*.rs`, `SC/authorization_policy*.rs`, `SC/access_*.rs`, `SC/relation_*.rs`, `SC/direct_*.rs`, `SC/removed_direct_peers*.rs`, `SOS/access_*.rs`, `SOS/direct_*.rs`, `SOS/connection_rights_actions.rs`, `SOS/identity_store*.rs`, `ENG/share/os/linux_os/identity_lock.rs`, `ENG/share/os/windows/identity_lock.rs`.
- Fläche exec: `SC/exec*.rs`, `SC/tcp_*.rs`, `SC/apps_wire.rs`, `SC/loopback_services.rs`, `SOS/exec_*.rs`, `SOS/tcp_*.rs`, `SOS/auto_forward.rs`, `SOS/consent_launch.rs`, `ENG/share/os/{linux_os,windows,android}/exec*`, `ENG/share/os/linux_os/proc_net_tcp.rs`, `ENG/screen/`, `ENG/rdp/`, `ENG/remote_window/`, `ENG/apps/`.
- Fläche fs: `SC/fs*.rs`, `SC/export_*.rs`, `SC/profile*.rs`, `SC/host_*.rs`, `SC/storage_*.rs`, `SC/mount_*.rs`, `SC/remove_tree_wire*.rs`, `SC/walk*.rs`, `SC/stage_*.rs`, `SC/range_*.rs`, `SC/folder_reservation.rs`, `SC/file_identity_rpc.rs`, `SC/hash_walk_wire.rs`, `SC/watch_wire.rs`, `SC/duplicate_wire.rs`, `SC/list_batch_wire.rs`, `SC/abgleich_*.rs`, `SC/analysis_*.rs`, `SOS/fs_*.rs`, `SOS/host_*.rs`, `SOS/profile_*.rs`, `SOS/storage_*.rs`, `SOS/removal.rs`, `SOS/analysis_*.rs`, `ENG/share/os/linux_os/fs_path_adapter.rs`, `ENG/share/os/windows/fs_path_adapter.rs`, `ENG/share/os/android/host_job_share.rs`.
- Fläche server: `share-server/src/`, `share-server/tests/`, `VND/iroh-relay-1.0.0/src/`, `vendor/SMART_EXPLORER_QAD_LIMITS.md`.
- Fläche lokal: `ENG/daemon/`, `ENG/updater/`, `ENG/login/`, `ENG/mcp_launcher/`, `SC/login_transport.rs`, `native/crates/core/src/creds/`, `native/crates/cli/src/cli/shell_session/`, `native/crates/cli/src/cli/mcp/`, `native/crates/cli/src/cli/private_file.rs`, `native/crates/mobile/src/`, `native/android-bridge/`, `android/app/src/main/AndroidManifest.xml`, `android/app/src/main/res/xml/`.
- Fläche entdeckung: `SC/discovery_*.rs`, `SC/signal_*.rs`, `SC/lan_*.rs` ohne `SC/lan_link_*`, `SC/crypto.rs`, `SC/server*.rs`, `SC/room_*.rs`, `SC/tracked_*.rs`, `SC/peer_*.rs`, `SC/legacy_*.rs`, `SC/device_*.rs`, `SOS/discovery_*.rs`, `SOS/lan_*.rs` ohne `SOS/lan_link_*`, `SOS/peer_feed*.rs`, `SOS/server_address*.rs`, `SOS/poll_status.rs`, `SOS/legacy_*.rs`.
- Fläche weg: `SC/route_*.rs`, `SC/endpoint_routes*.rs`, `SC/keepalive*.rs`, `SC/quic_*.rs`, `SC/lan_link_*.rs`, `SC/batch*.rs`, `SC/framing*.rs`, `SC/lz4_*.rs`, `SC/speed_*.rs`, `SC/connection_*.rs`, `SC/io_deadline.rs`, `SOS/route_*.rs`, `SOS/transport_options.rs`, `SOS/relay_dns.rs`, `SOS/lan_link_*.rs`, `ENG/share/os/{linux_os,windows,android}/route_*`, `VND/iroh-1.3.0/`, `VND/noq-1.3.0/`, `VND/noq-proto-1.3.0/`, `vendor/SMART_EXPLORER_ROUTE_POLICY.md`.
- Alle übrigen Dateien unter `ENG/share/` gehören keinem Bereich; ein Befund dort geht in den Abschnitt „Ohne Bereich“ der Befundliste und wird im Paket `f-rest` behoben.
- Lesend dürfen alle Review- und Korrekturpakete zusätzlich jede Datei öffnen, die ihre Fläche direkt aufruft oder die ihre Fläche aufruft (Aufrufer/Aufgerufene), und `docs/ARCHITEKTUR.md`, `docs/SHARE_SERVER.md`, `docs/oberflaechen/`, `docs/tests/register.tsv`, `AGENTS.md`. Begründung: Sicherheitsgrenzen liegen oft an der Aufrufstelle; ohne die Gegenseite ist ein Befund nicht entscheidbar. Ändern dürfen sie nur ihre Fläche.

Pakete (Rolle; abhängig von):

| Paket | Rolle | abhängig von | Fläche änderbar |
|---|---|---|---|
| plandoku | umsetzung (Doku) | – | `docs/plaene/2026-11-07-sicherheit-direktweg/plan.md` |
| qad | umsetzung | – | M1–M4-Dateien |
| r-zugang | analyse | – | nur Befundliste `r-zugang` (nicht im Repository) |
| r-exec | analyse | – | nur Befundliste `r-exec` (nicht im Repository) |
| r-fs | analyse | – | nur Befundliste `r-fs` (nicht im Repository) |
| r-server | analyse | qad | nur Befundliste `r-server` (nicht im Repository) |
| r-lokal | analyse | – | nur Befundliste `r-lokal` (nicht im Repository) |
| r-entdeckung | analyse | – | nur Befundliste `r-entdeckung` (nicht im Repository) |
| r-weg | analyse | qad | nur Befundliste `r-weg` (nicht im Repository) |
| f-zugang | umsetzung | r-zugang, qad | Fläche zugang |
| f-exec | umsetzung | r-exec | Fläche exec |
| f-fs | umsetzung | r-fs | Fläche fs |
| f-server | umsetzung | r-server, qad | Fläche server |
| f-lokal | umsetzung | r-lokal, qad | Fläche lokal |
| f-entdeckung | umsetzung | r-entdeckung | Fläche entdeckung |
| f-weg | umsetzung | r-weg, qad | Fläche weg |
| f-rest | umsetzung | alle f-* | nur die in „Ohne Bereich“ genannten Dateien |
| suite | umsetzung (Tests) | qad, alle f-*, f-rest | Testdateien, `docs/tests/register.tsv` |
| doku | umsetzung (Doku) | qad, alle f-*, f-rest | Doku-Dateien M13 |



Nach allen Paketen (Manager, nicht Teil der Aufträge): Integration und Commits, Graph-Neubau nach `AGENTS.md`, Push, ein Lauf `crates-task.yml` (part `suite`), Fehlerschleife, dann ein Release über `release.yml`.

## 10 Fragen an den Nutzer (nicht blockierend)

- Q1 (s4): Soll nach dem Machbarkeitsbericht eine je Übertragung zuschaltbare unverschlüsselte Übertragung gebaut werden (nur Direktweg, nie Relay, mit Integritätsschutz, Zustimmung beider Geräte), oder zuerst nur die Messung der Verschlüsselungskosten auf den eigenen Geräten? Der Batch läuft ohne Antwort weiter; ohne Antwort wird nichts davon gebaut.
- Q2 (Betrieb, nur Bericht an den Nutzer, nicht ins Repository): Für QAD muss am eigenen Server UDP 7842 erreichbar sein und der `se-share-server` ein eigenes TLS-Zertifikat haben, auch hinter einem Reverse-Proxy. Ob der laufende Server das erfüllt, ist eine Geräteangabe und wird nur dem Nutzer berichtet.
- Q3 (aus Reviews, falls vorhanden): Befunde, deren Abhilfe Funktion kosten würde (E9), sammelt der Manager aus den Befundlisten und legt sie dem Nutzer vor.

## 11 Nachtrag: Nutzerentscheidungen aus Nachricht 3 und 4 (2026-11-07)

Quellen: Nutzernachrichten 3 und 4 vom 2026-11-07 (Antworten auf die Rückfragen der Reviews `r-fs`, `r-lokal`, `r-entdeckung` und `r-server`). Wortlaut, Anlass, Wirkung, verworfene Alternativen und angenommene Risiken jeder Entscheidung: [entscheidungen.md](entscheidungen.md).

### 11.1 Neue Anforderungen
- s6 (Nachricht 3, zu r-fs F1): „der entsperrte tresor wird nicht frei gegeben. der gesperrte tresor gerne, also dass eine andere maschiene den gesperrten tresor entsperren kann und dann darauf zugreifen, aber der entsperrte wird nicht frei gegeben.“
- s7 (Nachricht 3, zu r-lokal 1): eigener Update-Signaturschlüssel „gerne als git geheimniss“; kein Windows-Entwicklerzertifikat.
- s8 (Nachricht 3): r-lokal 2 „B“; r-entdeckung F1 „ja“; r-entdeckung F2 nur ohne Verlust anderer Kontakte (online wie offline) und mit echtem Sicherheitsvorteil.
- s9 (Nachricht 3): „der shareserver muss neue frei aufnehmen können, darf aber natürlich nicht frei alles was mit ihm verbunden ist exposen.“ „ich hätte aber eigentlich gerne dass der server seine rolle nicht erweitert und rein als resolution server zwischen den mit ihm verbundenen geräten dient, keine zugänge verwaltet oder so.“
- s11 (Nachricht 4): „ok dann nimm den server als relay, nicht n0, also den server nicht fest einprogrammiert, die konfigurierte verbindung im programm. ich hätte generell ungerne third party server integriert in dem flow.“
- s12 (Nachricht 4): „alles machen, ich denke es laufen keine mit so alten versionen. keine liste, fang an.“

### 11.2 Bestand zu „keine Drittanbieter-Server im Verbindungsablauf“ (an der Quelle geprüft)
Suche über `native/crates`, `android/app/src/main`, `native/android-bridge`, `share-server/src`, `se-agent/src` nach `RelayMode::Default|Staging`, n0-Hostnamen, öffentlichen Resolvern, Pkarr/DNS-Discovery und allen `http(s)://`-Hosts außerhalb von Tests:
- T1 Maschinenanmeldung: `SC/login_transport.rs:43-53` nutzte bei `use_default_relays` die n0-Relays (`RelayMode::Default`); Standard `true` (`ENG/login/core/gateway_types.rs:17-28`). Einzige Stelle mit `RelayMode::Default` außerhalb von Tests.
- T2 DNS-Rückfall: `SOS/relay_dns.rs:18-22` setzte neben dem System-Resolver `DnsResolver::new()` ein; das ist `Builder::default().with_system_defaults()` (iroh-dns 1.3.0, `src/dns.rs:748-750`) mit `FallbackMode::Deferred` als Standard (`src/dns.rs:224-252`) und öffentlichen Rückfall-Resolvern Cloudflare 1.1.1.1, Google 8.8.8.8, Quad9 9.9.9.9 (n0-dns-resolver 0.1.0, `src/public_resolvers.rs:42-60`). Sie wurden befragt, sobald alle System-Nameserver scheiterten oder die Systemkonfiguration nicht lesbar war. Quelle: die Crates in den Versionen, die `native/Cargo.lock` sperrt (Einträge `iroh-dns`, `n0-dns-resolver`).
- Kein Drittanbieter im Verbindungsablauf (geprüft, keine Änderung): `presets::Minimal` ohne Adress-Lookup-Dienst (`SC/node.rs:157`); Captive-Portal- und Latenzsonden von iroh zielen auf die konfigurierten Relays; `SOS/system.rs:14-16` ruft `UdpSocket::connect("8.8.8.8:80")` nur zur Routenwahl auf (UDP-`connect` sendet kein Paket); der Portmapper spricht nur mit dem eigenen Router.
- Außerhalb des Verbindungsablaufs (bleiben, nicht Teil von s11): Update-Feed GitHub (`ENG/updater/`), OpenH264-Download von Cisco (`ENG/screen/os/native_ffi/openh264_pins.rs:26-47`, Lizenzbedingung des Codecs), Google-Drive-Backend (vom Nutzer eingerichtet).

### 11.3 Entscheidungen (Ergänzung zu Abschnitt 5)
- E11 (s6, r-fs F1): Entsperrte Tresor-Laufwerke (Einhängepunkte aus `crate::vault::vault_mount_roots()`, zur Laufzeit gelesen) sind für alle Dateioperationen gekoppelter Geräte gesperrt, auch für Watch (R-FS-2) und als Freigabewurzel. Der gesperrte Tresor (verschlüsselter Container-Ordner) bleibt freigebbar, lesbar und beschreibbar; eine andere Maschine kann ihn über die Freigabe öffnen und selbst entsperren. Verworfen: Option B (Klartext über Freigabe).
- E12 (s7, r-lokal 1 Option A): Ed25519-Signatur über ein Release-Manifest (Version; je Payload Name, Größe, SHA-256). Privater Schlüssel als GitHub-Actions-Geheimnis `SE_UPDATE_SIGNING_KEY`; öffentlicher Schlüssel als Konstante im Programm (Prüfung mit `ring::signature::ED25519`, `ring` ist bereits Abhängigkeit von `native/crates/engine/Cargo.toml:37`, keine Lock-Änderung). Eingebauter GitHub-Feed: Staging nur mit gültiger Signatur; ältere Releases ohne Signatur nur nach ausdrücklicher Bestätigung je Update (gleiche Regel wie unsignierte `http://`-Feeds, r-lokal-3); eigene Feeds: Prüfung, wenn eine Signatur vorliegt, sonst Hinweis „unsigniert“ (r-lokal-2). Downloadgröße aus dem Manifest erzwungen (r-lokal-4). Verworfen: Windows-Authenticode (kein Zertifikat vorhanden, schützt nicht den Feed); Option B Offline-Schlüssel (Nutzer wählte Git-Geheimnis).
  Vorbedingung vor der Umsetzung: Schlüsselpaar außerhalb des Repositorys erzeugen (`openssl genpkey -algorithm ed25519`), privaten Schlüssel als Geheimnis `SE_UPDATE_SIGNING_KEY` im Repository setzen, den rohen öffentlichen Schlüssel (32 Byte, Hex) der Umsetzung übergeben. Der private Schlüssel kommt in keine Datei und keinen Bericht.
- E13 (s8, r-lokal 2 B): r-lokal-5 wird nicht umgesetzt; als bewusst angenommenes Risiko dokumentiert.
- E14 (s11, r-lokal 3 A, für alle Einrichtungen): Die Maschinenanmeldung nutzt als Relay ausschließlich die Relays des im Programm konfigurierten Share-Servers (mit dessen Pins und Klartext-Regel, `SOS/transport_options.rs::load`), nie n0 und nie eine fest einprogrammierte Adresse. Das gilt auch für bestehende Einrichtungen: Ein gespeichertes `use_default_relays: true` bedeutet ab jetzt „Relays des konfigurierten Servers“ (Feldname und Dateiformat bleiben, damit bestehende `gateway-network.json`-Dateien lesbar bleiben). Ohne konfigurierten Server: nur Direktwege. Verteilte Zieladressen (`GatewayTarget`) tragen die Relay-URLs des Servers des Gateways; ein Altziel mit `use_default_relays: true` wählt auf dem anrufenden Gerät die Relays dessen konfigurierten Servers. Begründung der Abweichung von der früheren Regel „bestehende Einrichtungen unverändert“: s11 schließt Drittanbieter-Server im Ablauf allgemein aus.
- E15 (s11, allgemein): Kein Drittanbieter-Server in einem Verbindungsweg. Umsetzung für T2: Der iroh-Resolver in `SOS/relay_dns.rs` nutzt nur die Systemkonfiguration mit `FallbackMode::Never` (keine öffentlichen Resolver); der System-Resolver (`getaddrinfo`) bleibt erste Quelle. T1 über E14. Neues Paket `f-dritte`.
- E16 (s8, r-entdeckung F1 Option 1): r-entdeckung-6 umsetzen (abgelehnte und widerrufene Altanfragen ohne aktive Berechtigung bei voller Liste verdrängbar; Sperre bleibt als `Ignored`-Grant).
- E17 (s8/s12, r-entdeckung F2, Block „Code-Erneuerung ohne Kontaktverlust“): Beim Entfernen eines Kontakts erneuert das Gerät seine Direkt-Beziehungsdaten (Lookup-ID, Direktgeheimnis) mit einem Übergang, der keinen anderen Kontakt verliert: alte Daten „auslaufend“, gültig nur für Identitäten mit aktivem Grant; neue Nachricht „Beziehungsdaten erneuert“ über gepinnte Sitzungen mit Bestätigung; Offline-Kontakte erhalten sie bei der nächsten Verbindung; unter der alten Lookup-ID eine Presence ohne Adressliste; Altgegenstellen ohne die Nachricht bleiben auf den alten Daten mit verkürzter Presence; Capability-Flag für die neue Nachricht. Der Server bekommt keine neue Rolle (eine zweite Lookup-ID innerhalb der vorhandenen Grenze `SRV/limits.rs:20`). Zusätzlich zu prüfen und, wo möglich, umzusetzen: ob iroh einem entfernten (in `SC/removed_direct_peers.rs` gesperrten) Schlüssel Direktadressen vor der Ablehnung durch die Anwendung mitteilt; wenn ja, Verbindung dieses Schlüssels sofort nach dem TLS-Abschluss schließen, bevor NAT-Traversal-Adressen gesendet werden, oder den Rest als Befund melden. Zwei Pakete: `code-erneuerung` (Kern, Wire, Identität, Sitzung, Presence, LAN, Daemon-IPC) und `code-oberflaechen` (Anzeige „Code wird erneuert, n Geräte stehen aus“ und Steuerung auf CLI, Desktop-GUI, MCP, Android, Bridge). Verworfen: vorhandene Erneuerung automatisch auslösen (verliert Kontakte, Bedingung des Nutzers verletzt); Geheimnis je Kontakt (größerer Protokollwechsel, eigener Batch).
- E18 (s12, r-server F1 Option 2): Schlüsselanmeldung ist Standard (`--require-key-login` an). Das Kompatibilitätsverhalten bleibt als ausdrücklicher Betreiberschalter `--allow-legacy-clients` erhalten (keine Fähigkeit entfernt, nur der Standard geändert); `--require-key-login` bleibt als Eingabe gültig. Clients bis v0.5.169 erhalten die vorhandene Meldung „this server requires key login; update Smart Explorer“ (`SRV/state.rs:129`). Neue Geräte werden weiter ohne Zutun aufgenommen (V1), der Server verwaltet keine Zugänge (V3).
- E19 (s12, r-server F2 Option 1): keine Liste zugelassener Schlüssel; R-SERVER-09 wird nicht umgesetzt. Kein Paket darf dem Server eine Zugangsverwaltung, Kontenführung oder neue Rolle geben (s9).
- E20 Freigabe der Release-Signatur im Release-Weg: neues Paket `signatur-release` ändert `.github/workflows/release.yml` und `native/publish-release-assets.ps1`: Manifest erzeugen, mit `SE_UPDATE_SIGNING_KEY` signieren (fehlt das Geheimnis, bricht der Workflow vor der Veröffentlichung ab), beide Dateien als Assets hochladen, Signatur vor der Veröffentlichung gegen den öffentlichen Schlüssel prüfen. Die Asset-Zahl steigt von 26 auf 28; `docs/RELEASING.md` und `README.md` zieht `doku` nach; die Stellen in `AGENTS.md`, die „26 assets“ nennen, meldet `doku` dem Manager (Projektanweisung, nicht Teil eines Subagenten-Auftrags).

### 11.4 Meilensteine (Ergänzung zu Abschnitt 6)
- M16 (f-fs, E11): Tresor-Einhängepunkt `T` unter Freigabe `A`: `ListDir`/`Stat`/`Read`/`Write`/`Watch` auf `A/<Pfad zu T>/…` → `PermissionDenied`, Freigabewurzel in `T` → abgewiesen; gesperrter Tresor-Container-Ordner unter `A` → lesbar und beschreibbar wie jede Datei.
- M17 (f-lokal, E12): eingebauter Feed ohne oder mit falscher Signatur → kein Staging; gültige Testsignatur → Staging; Download länger als Manifestgröße → Abbruch, `.download` entfernt; `http://`-Feed ohne Signatur ohne Bestätigung → Fehler „unsigned http feed“.
- M18 (f-lokal, E14): Gateway mit konfiguriertem Share-Server → Relay-Karte enthält genau dessen Relays; gespeichertes `use_default_relays: true` → keine n0-URL in der Relay-Karte; ohne Server → `RelayMode::Disabled`; kein Aufruf von `RelayMode::Default` im Produktionscode.
- M19 (f-dritte, E15): Resolver des Share-Endpunkts enthält keinen öffentlichen Rückfall-Nameserver; Auflösung über Systemkonfiguration und `getaddrinfo` unverändert.
- M20 (f-entdeckung, E16): volle Altanfrageliste mit 24 abgelehnten Einträgen → neue Altanfrage wird angenommen, Sperre des abgelehnten Geräts bleibt.
- M21 (code-erneuerung, E17): Kontakt entfernen → neue Lookup-ID und neues Geheimnis; verbleibender Kontakt online → erhält „Beziehungsdaten erneuert“, bestätigt, verbindet danach mit neuen Daten; verbleibender Kontakt offline → verbindet später mit alten Daten, erhält die neuen, verliert keinen Zugang; entfernter Kontakt → keine Sitzung, `watch_direct` mit altem Beweis erhält keine Adressliste mehr; Altgegenstelle ohne Capability → bleibt auf alten Daten verbunden.
- M22 (code-oberflaechen, E17): jede Oberfläche zeigt den Übergangszustand und die Zahl ausstehender Geräte gleich (Werte aus demselben Zustand).
- M23 (f-server, E18): Client ohne `key_login_v1` gegen Standardserver → `KeyLoginRequired`; mit `--allow-legacy-clients` → Verhalten wie bisher; Client mit `key_login_v1` → unverändert angenommen.
- M24 (signatur-release, E20): Release-Workflow bricht ohne Geheimnis vor der Veröffentlichung ab; mit Geheimnis enthält der Release `release-manifest` und `release-manifest.sig`, die Prüfung im Publish-Schritt besteht. Prüfung erst im Dry-Run oder Release (Manager), nicht in der Suite.

### 11.5 Sicherheit (Ergänzung zu Abschnitt 8)
- Update-Signatur (E12, E20): Vertrauensgrenze Feed/Release-Assets → Updater. Prüfung vor jeder Wirkung: Signatur und Manifest vor dem Staging, Hash und Größe je Payload vor dem Einsetzen. Fehler: fail closed (kein Staging) für den eingebauten Feed; Bestätigung je Update für unsignierte Altversionen und Klartext-Feeds. Grenzen: Downloadgröße = Manifestgröße. Missbrauch: ausgetauschte Assets, fremder Feed, MITM bei `http://` → Signatur ungültig; Übernahme des Workflows samt Geheimnis bleibt Restrisiko (Nutzerwahl Option A). Abnahme: M17, M24.
- Relays der Maschinenanmeldung (E14): Vertrauensgrenze Gerät → eigener Server; Pins und TLS-Regeln des Servers gelten. Fehler: ohne Server keine Relays (fail safe, kein Rückfall auf n0). Abnahme: M18.
- DNS ohne öffentliche Resolver (E15): keine Anfrage an Dritte; ein falsches DNS-Ergebnis führt höchstens zu einem Verbindungsversuch, den TLS mit Pins abweist. Abnahme: M19.
- Code-Erneuerung (E17): Vertrauensgrenze ehemaliger Kontakt ↔ Gerät. Prüfung: alte Daten nur für Identitäten mit aktivem Grant, neue Daten nur über gepinnte, authentifizierte Sitzung; Annahme der neuen Daten nur mit Bestätigung. Fehler: Abbruch eines Übergangs lässt alte Daten für berechtigte Kontakte gültig (kein Kontaktverlust), nie für entfernte. Widerruf: entfernter Kontakt verliert Lookup-Zugang und Adressliste sofort; Aufräumen: alte Daten werden gelöscht, sobald jeder berechtigte Kontakt bestätigt hat. Grenzen: höchstens eine laufende Erneuerung plus eine auslaufende Lookup-ID je Gerät; Zahl gleichzeitiger Übergänge begrenzt; Nachrichtengröße begrenzt. Missbrauch: Wiedereinspielen alter Erneuerungsnachrichten (Nonce, Generationszähler), Herabstufung durch Altgegenstelle (Capability nur Zusatz, alte Daten nur für Grant-Inhaber). Abnahme: M21. Umsetzung (Abweichung, Begründung in [entscheidungen.md](entscheidungen.md), E17): höchstens vier gleichzeitig auslaufende Lookup-IDs je Gerät, weil eine weitere Entfernung während eines Übergangs sonst einen offline Kontakt aussperren würde. Umsetzung (Abweichung zum Widerruf im LAN, 2026-11-07): Solange eine Lookup-ID ausläuft, kündigt das Gerät im LAN zusätzlich unter deren rotierender ID an, mit seiner Adressliste versiegelt mit dem alten Direktgeheimnis (`v=3`) und im Klartext mit Signatur (`v=2`) (`SOS/lan_presence_auth.rs`, `retiring_announcements`; `SOS/lan_presence_retiring.rs`, `append_records`). Ein entferntes Gerät im selben LAN, das das alte Geheimnis kennt, erkennt das Gerät daran und liest diese Adressliste, bis die Lookup-ID endet; Altgegenstellen ohne Erneuerungsunterstützung halten sie dauerhaft offen. Grund: Kontakte, die das Gerät nur im LAN erreichen, verlören sonst den Kontakt (Nutzerbedingung „ohne Verlust“). Über den Server erhält ein entferntes Gerät unter der alten Lookup-ID weiter nur die Presence ohne Adressliste.
- Schlüsselanmeldung als Standard (E18): Missbrauch „als Altclient ausgeben“ entfällt im Standard. Abnahme: M23.

### 11.6 Zu erhaltendes Verhalten (Ergänzung zu Abschnitt 7)
- Gesperrte Tresore bleiben über Freigaben erreichbar und auf einer anderen Maschine entsperrbar (s6).
- Neue Geräte werden vom Server ohne Zutun aufgenommen; der Server verwaltet keine Zugänge (s9).
- MCP darf weiter Rechte mit `--yes` vergeben (E13).
- Eigene Update-Feeds bleiben nutzbar; ältere Releases bleiben nach Bestätigung installierbar.
- Kein Kontakt geht durch die Code-Erneuerung verloren, auch kein offline Kontakt und keine Altgegenstelle.

### 11.7 Pakete (Ergänzung und Änderung zu Abschnitt 9)

| Paket | Rolle | abhängig von | änderbar |
|---|---|---|---|
| f-fs (geändert) | umsetzung | r-fs | Fläche fs |
| f-lokal (geändert) | umsetzung | r-lokal, qad | Fläche lokal; zusätzlich Update-Bestätigung in `native/crates/gui/src/` (Update-Dialog) und `native/crates/cli/src/cli/update.rs`, `update_copy.rs` |
| f-entdeckung (geändert) | umsetzung | r-entdeckung | Fläche entdeckung |
| f-server (geändert) | umsetzung | r-server, qad | Fläche server |
| f-dritte (neu) | umsetzung | f-weg | `SOS/relay_dns.rs` |
| signatur-release (neu) | umsetzung | f-lokal | `.github/workflows/release.yml`, `native/publish-release-assets.ps1` |
| code-erneuerung (neu) | umsetzung | r-entdeckung, f-entdeckung, f-zugang, f-lokal | Fläche code-erneuerung (unten) |
| code-oberflaechen (neu) | umsetzung | code-erneuerung, f-lokal | Fläche code-oberflaechen (unten) |
| f-rest (geändert) | umsetzung | alle f-*, f-dritte, code-erneuerung, code-oberflaechen, signatur-release | genannte Dateien |
| suite (geändert) | umsetzung | alle Umsetzungspakete | Tests, Register |
| doku (geändert) | umsetzung | alle Umsetzungspakete | Doku |

- Fläche code-erneuerung (Ausnahme vom kleinsten Zuschnitt, Begründung: Identität, Sitzungsprüfung, Presence, LAN-Ankündigung und Wire-Nachricht bilden eine Integrationsgrenze; getrennt würde jede Teiländerung die anderen brechen): `SC/identity*.rs`, `SOS/identity_store*.rs`, `SC/session_authorize*.rs`, `SC/session_denial*.rs`, `SC/direct_*.rs`, `SOS/direct_*.rs`, `SC/legacy_direct_*.rs`, `SC/removed_direct_peers*.rs`, `SC/signal_publish*.rs`, `SC/signal_presence*.rs`, `SC/lan_privacy*.rs`, `SC/lan_presence*.rs`, `SOS/lan_presence*.rs`, `SC/wire_capabilities*.rs`, `SC/node_accept*.rs`, `SC/types.rs`, `ENG/daemon/` (nur IPC-Anfragen und -Antworten zum Übergangszustand), neue Dateien mit Präfix `SC/code_renewal_` oder `SOS/code_renewal_`.
- Fläche code-oberflaechen: `native/crates/gui/src/` (Dateien, die Direkt-Code, Kontakte oder Geräte zeigen), `native/crates/cli/src/cli/share/`, `native/crates/cli/src/cli/mcp/`, `native/crates/mobile/src/`, `native/android-bridge/src/`, `android/app/src/main/java/`, `android/app/src/main/res/values*/strings.xml`.
- Abhängigkeiten über gemeinsame Dateien: `code-erneuerung` folgt auf `f-entdeckung` und `f-lokal` (gemeinsame Dateien `SC/lan_presence*`, `SC/signal_*`, `ENG/daemon/`); `code-oberflaechen` folgt auf `code-erneuerung` und `f-lokal` (gemeinsam `native/crates/cli/src/cli/mcp/`, `native/crates/mobile/src/`, `native/android-bridge/src/`); `signatur-release` folgt auf `f-lokal` (Manifestformat). `f-rest` deckt zusätzlich die Abschnitte „Fremder Bereich“ aller Befundlisten ab (zum Beispiel r-entdeckung-9 in Fläche exec, die Herabstufungsregel in `ENG/daemon/os/shared/lan_runtime_presence.rs`).

### 11.8 Fragen an den Nutzer (Stand)
- Q1 (s4) bleibt offen und nicht blockierend.
- Q2 bleibt Bericht an den Nutzer (UDP 7842, TLS am Server).
- Q3: entschieden durch Nachricht 3 und 4 (Abschnitt 11.3). Keine weitere Nutzerentscheidung nötig, um weiterzuarbeiten.

## Ergänzung 1 (2026-11-07)

Eingang: die Lückenliste der Integrationsprüfung mit den Lücken L-A1 bis L-A6 und L-B1 bis L-B3. Die Bewertung je Lücke steht in E1.1, die Entscheidungen E21 bis E27 in E1.2, die Meilensteine M25 bis M27 und die geänderte Abnahme M15 in E1.3, die Sicherheitseigenschaften in E1.4, die Pakete in E1.5. Alle Projektpfade sind relativ zum Projektwurzelordner; die Kürzel `ENG`, `SC`, `SOS`, `SRV` und `VND` gelten wie im Kopf dieses Dokuments.

### E1.1 Bewertung
Bestätigt sind alle neun Lücken: L-A1, L-A2, L-A3, L-A4, L-A5, L-A6, L-B1, L-B2 und L-B3. Die Abschnitte C, D und E des Eingangs enthalten keine Lücke. Die Punkte F1 bis F7 des Eingangs haben keinen Lückencharakter. F1, F2 und F5 bleiben offene Prüfpunkte des Projekts: F1 die Anzahl der Assets in `AGENTS.md`, F2 das Geheimnis `SE_UPDATE_SIGNING_KEY` vor dem Dry-Run oder dem Release, F5 die Commit-Zuordnung von `native/crates/engine/src/rdp/README.md` und dieses Plandokuments. F3 wird zur Kenntnis genommen, F4 und F7 verletzen keine Regel, F6 verlangt der Plan nicht. Eine CI-Auswertung lag nicht vor; deshalb ist kein Infrastrukturfehler zu bewerten.

### E1.2 Entscheidungen (Ergänzung zu Abschnitt 5 und 11.3)
- E21 (L-A4): Die QAD-Grenzen und die Wiederholung werden aus `SRV` über eine als Testhilfe gekennzeichnete Schnittstelle im Vendor-Patch geprüft: `iroh_relay::quic::qad_testing`, nur mit Feature `server`. Die Schnittstelle baut eine QAD-Sonde mit einem eigenen `noq::Endpoint` und gibt einen engen Zugang zu `Admission` frei. Verworfen: Dev-Abhängigkeit `noq` in `share-server/Cargo.toml` (verlangt eine Änderung von `share-server/Cargo.lock`, die ohne Cargo-Lauf nicht sicher erzeugbar ist); eigener Testlauf der Vendor-Crate in der Suite (eigenes Lock aus der Veröffentlichung ohne den Patch, voller Dev-Abhängigkeitsbaum, Änderung an `native/test-crates-task.py` außerhalb der Paketflächen). Die Testhilfe ändert kein Serververhalten; die Eigenschaften aus 8.1 bleiben unverändert.
- E22 (L-A5): M5 prüft den Client-Teil im Engine-Test mit dem Testrelay aus `iroh::test_utils::run_relay_server_with(true)` und mit `Endpoint::net_report()`. Die QAD-Wahl kommt aus der Produktionsfunktion `relay_config` (`SC/endpoint_routes.rs`); im Test wird nur der Port auf den QUIC-Port des Testservers gesetzt. Gegenprobe ohne QAD-Konfiguration. Der Server-Teil liegt in den Tests M1 bis M3 aus Abschnitt 8.1. Annahme, nicht am Quelltext nachgeprüft: Die Zertifikatsprüfung des Testrelays ist mit Feature `test-utils` abschaltbar oder über eine CA-Konfiguration erreichbar; ist das nicht so, nennt der Test den Grund und den Ersatz (Server-Tests M1 bis M3).
  - Umsetzung (Abweichung, 2026-11-07): `Endpoint::net_report()` ist ohne das Iroh-Feature `unstable-net-report` nicht verfügbar; der Test belegt die Sonde über den Zähler `qad_connections` des Testrelays und die Gegenprobe über `qad_incoming`/`qad_connections` gleich 0 (Einzelheiten unter M5 in Abschnitt 6). Die Zertifikatsprüfung des Testrelays umgeht der Test mit `CaTlsConfig::insecure_skip_verify()`.
- E23 (L-B1): `connect_control` verbindet über `ipc_proof::connect_token` (`ENG/daemon/os/shared/ipc_proof.rs`) mit einer Zeitgrenze von 2 Sekunden. Fehler bleiben `io::Error` wie bisher. Verworfene Alternativen: keine.
- E24 (L-B2; Ergänzung zu E14 in `entscheidungen.md`): Netz-Nachführung installierter Gateways ohne neue Maschinenidentität. Die neue Funktion `provision_network` in `ENG/login/os/shared/provision.rs` schreibt nur `gateway-network.json` in ein neues privates Bündelverzeichnis; der CLI-Befehl `se login provision-network DIRECTORY [--json]` steht neben `se login provision`. Die Installationsskripte bleiben unverändert: `native/install-login-service.sh` nimmt die Datei als siebtes Argument, `native/install-login-service.ps1` über `-NetworkFile`. Kein MCP- und kein GUI-Eintrag, weil `provision` heute ebenfalls nur über die CLI erreichbar ist. Verworfen: bestehende Gateways dauerhaft nur über Direktwege (Funktionsverlust, widerspricht der Vorgabe, die Funktionalität nicht einzuschränken); automatische Auflösung im Gateway-Dienst (der Dienst liest kein Nutzerprofil, der konfigurierte Server ist dort nicht bekannt). Bis der Betreiber den Befehl und das Skript ausführt, bleibt ein bestehendes Gateway auf Direktwegen; `docs/TODO.md` hält das fest (Paket doku-2).
- E25 (L-B3; Ergänzung zu G1 in `entscheidungen.md`): Das Budget automatisch gebundener Dienste kommt aus dem Betriebssystem. Linux und Android heben das weiche Limit für Dateideskriptoren einmal je Prozess auf das harte Limit an; das Budget beträgt ein Viertel des danach geltenden weichen Limits, mindestens 256. Windows kennt keine solche Grenze; dort gibt es keine Obergrenze automatischer Listener außer der Grenze je Gerät. Die feste Grenze von 256 entfällt. Die Grenze von 512 Diensten je Gerät (`SC/loopback_services.rs`, `MAX_SERVICES`) bleibt auf allen Plattformen. Verworfen: die Aktion „Öffnen“ auf dem Desktop (neuer IPC-Befehl zwischen Oberfläche und Worker sowie Einträge in CLI, MCP und GUI); eine Rückfrage an den Nutzer (nicht nötig, weil die Abhilfe ohne Funktionsverlust möglich ist). Restfall: Bei hartem Limit von 1024 Deskriptoren oder weniger beträgt das Budget 256; Dienste oberhalb von 256 werden dann abgewiesen. Gegenüber dem Stand HEAD ist das auf solchen Systemen eine Einschränkung. Annahme, nicht gemessen: Bei einem solchen Limit scheitert das Binden oberhalb des Deskriptorlimits ohnehin.
- E26 (L-A6): `native/test-share-mixed-version-e2e.sh` startet den Server mit `--allow-legacy-clients` (Zeile 208). Die Änderung liegt in Paket f-server-2, weil der Schalter zu E18 in `entscheidungen.md` gehört.
- E27 (Task-Suite): Die Task-Suite besteht aus neun Testpaketen mit getrennten änderbaren Dateien und einem Registerpaket (suite-2-register), das als einziges `docs/tests/register.tsv` ändert. Prüfung mit `native/test-register.py`, einem reinen Textparser ohne Cargo-Aufruf. Begründung: `docs/tests/register.tsv` wäre sonst eine gemeinsame änderbare Datei paralleler Pakete.

### E1.3 Meilensteine (Ergänzung zu Abschnitt 6 und 11.4)
- M25 (Paket f-lokal-2): Ein lokaler Prozess ohne Besitzbeweis an der IPC-Adresse erhält bei `connect_control` keinen Token. `provision_network` schreibt nur `gateway-network.json` mit den Relays und Pins des konfigurierten Servers; `provision` bleibt unverändert.
- M26 (Paket f-exec-2): Der Desktop bindet automatisch bis zum Budget aus dem Betriebssystem; es gibt keine feste Zahl 256; `os/shared` enthält keinen Plattform-Import.
- M27 (Paket f-server-2): `qad_testing` ändert kein Serververhalten; `native/test-share-mixed-version-e2e.sh` übergibt in Zeile 208 `--allow-legacy-clients`.
- M15 (geändert): erfüllt, wenn jede Erwartungskennung der Suite-Planung genau einen Status trägt (`geschrieben`, `bestehend` oder `nicht abbildbar` mit Grund) und `native/test-register.py` ohne Befund endet.

### E1.4 Sicherheit (Ergänzung zu Abschnitt 8 und 11.5)
- E23: Vertrauensgrenze lokaler Prozess an der veröffentlichten IPC-Adresse → Client. Prüfung vor jeder Wirkung: Besitzbeweis, bevor der Token gesendet wird. Fehler: fail closed, kein Token wird gesendet. Abnahme: M25 in Paket suite-2-lokal.
- E24: Vertrauensgrenze Gerät → eigener Server; Pins und Klartext-Regel aus `transport_options::load` gelten unverändert. Fehler: ohne konfigurierten Server keine Relays (fail safe, kein Rückfall auf n0). Die Ortsprüfung des Bündels entspricht der von `provision`. Abnahme: M25.
- E25: Missbrauch durch Deskriptor-Erschöpfung über viele gelistete Dienste bleibt begrenzt: unter Unix ist das Budget ein Viertel des weichen Limits, mindestens 256; je Gerät gilt höchstens 512. Fehler beim Anheben des Limits: es bleibt das bisherige Limit (fail safe). Abnahme: Budget-Test in Paket suite-2-exec.
- E21: Die Testhilfe ändert keine Grenze und keinen Annahmeweg; die Eigenschaften aus 8.1 bleiben unverändert.

### E1.5 Pakete (Ergänzung zu Abschnitt 9 und 11.7)
| Paket | abhängig von |
|---|---|
| f-lokal-2 | – |
| f-exec-2 | – |
| f-server-2 | – |
| suite-2-server | f-server-2 |
| suite-2-weg | – |
| suite-2-zugang | – |
| suite-2-erneuerung | – |
| suite-2-oberflaechen | – |
| suite-2-lokal | f-lokal-2 |
| suite-2-fs | – |
| suite-2-entdeckung | – |
| suite-2-exec | f-exec-2 |
| suite-2-register | suite-2-server, suite-2-weg, suite-2-zugang, suite-2-erneuerung, suite-2-oberflaechen, suite-2-lokal, suite-2-fs, suite-2-entdeckung, suite-2-exec |
| doku-2 | f-lokal-2, f-exec-2, f-server-2 |
| plandoku-2 | – |

Zuordnung Lücke → Paket: L-A1 → suite-2-server, suite-2-weg, suite-2-zugang, suite-2-erneuerung, suite-2-oberflaechen, suite-2-lokal, suite-2-fs, suite-2-entdeckung, suite-2-exec, suite-2-register; L-A2, L-A3, L-A5 → suite-2-weg; L-A4 → f-server-2, suite-2-server; L-A6 → f-server-2, doku-2; L-B1 → f-lokal-2, suite-2-lokal; L-B2 → f-lokal-2, suite-2-lokal, doku-2; L-B3 → f-exec-2, suite-2-exec, doku-2; Plandokument der Ergänzung → plandoku-2.

Parallele Pakete haben keine gemeinsame änderbare Datei. Testpakete ändern nur Testdateien und `mod`-Zeilen in den ihnen zugeordneten Dateien. Überschneidungen sind ausdrücklich einem Paket zugeordnet: `SC/peer_denials*`, `SC/service_access_notices*`, `SC/legacy_*` und `SC/backend_capabilities*` an suite-2-entdeckung; `SC/node_tcp*` an suite-2-exec; `SC/relation_proof*` an suite-2-zugang; `SC/login_transport*` an suite-2-lokal. `docs/tests/register.tsv` ändert allein suite-2-register. Die Fix-Pakete ändern getrennte Produktionsdateien; ein Testpaket, das eine Produktionsdatei eines Fix-Pakets prüft, hängt von diesem Fix-Paket ab (Spalte „abhängig von“).

### E1.6 Fragen an den Nutzer
Keine neue Frage.

## Ergänzung 2 (2026-11-07)

Ausgangslage: Lücken L2-1 bis L2-5 aus der Lückenliste Integration, Durchgang 2 (Stand 2026-11-07). Bewertung je Lücke mit Datei und Stelle in E2.1 und E2.2; Entscheidungen E28 bis E31; Meilensteine M28 bis M31 und M15 (erneut geändert).

### E2.1 Bewertung
- Bestätigt am Quelltext: L2-1 (Ankündigung unter auslaufender Lookup-ID, `SOS/lan_presence_auth.rs` Zeilen 173 und 212 bis 243, `SOS/lan_presence_retiring.rs` `append_records`), L2-2 (Übernahme erneuerter Beziehungsdaten, `SOS/direct_reciprocal_persistence.rs` `renew_contact_material` und `renewable_contact`), L2-3 (Abweisung entfernter Geräte am Relay-Eingang, `SC/node_accept.rs` `relay_names_withdrawn`), L2-4 (`native/crates/cli/src/cli/update.rs` mit 500 Zeilen; die Regel verlangt weniger als 500), L2-5 (`docs/plaene/2026-11-07-sicherheit-direktweg/entscheidungen.md` E17, Punkt „Wirkung“ und Risiko (1)).
- L2-2 Teil b (Schreibpfad in den Geheimnisspeicher) ist nicht abschließend belegt: Ob Speichern und Laden eines Testprofils ohne gespeicherte Geheimnisse den Sicherheitsspeicher erreicht, ist Annahme; der Nachweis erfolgt bei der Umsetzung, sonst als „nicht abbildbar“ mit Fundstelle.
- Hinweise ohne Lückencharakter und ohne Paket, aus der Lückenliste übernommen und nicht erneut geprüft:
  - Die Abschnitte „mandatory release“ und die Liste erwarteter Assets in `AGENTS.md` nennen 26 Assets; nach E20 sind es 28.
  - Das Repository-Geheimnis `SE_UPDATE_SIGNING_KEY` ist vor einem Dry-Run oder Release zu prüfen; die Arbeitsumgebung kann es nicht lesen.
  - `ENG/share/os/linux_os/proc_net_tcp.rs` und `ENG/share/os/windows/tcp_table.rs` haben je zwei rustfmt-Abweichungen (Signaturen `connection_owned_by` und `connection_account`), die schon in HEAD bestehen; keine Regel dieses Batches verlangt die Bereinigung.
  - Mount-Host-Verbindungen (`ENG/daemon/os/shared/mount_client_host.rs` `host_stream`, `mount_lock_participant.rs` `round`) senden `launch_token`, `session_token` und `backend_token` an die veröffentlichte Daemon-IPC-Adresse ohne Besitzbeweis des Empfängers. Das Review `r-lokal` führte den Eingang E4 als „geprüft ohne Befund“; ein Befund ist nicht belegt. Ob dieser Eingang ein Review erhält, ist offen.
  - Teilabbildungen mit zutreffendem Grund (Produktions-Testschnittstelle, Gerät, Root oder Administrator, Netz zu GitHub nötig) stehen in der Lückenliste. Rest cr-F5b (`outgoing_material`) ist flächenbedingt offen. Der positive Raumzweig des Ablehnungs-Gates (suite-2-zugang O2) ist ungeprüft und ist keine Erwartungskennung der Suite-Planung.
  - Die Einfügungen dieses Durchgangs in `plan.md` gehören in den Plan-Commit.

### E2.2 Entscheidungen (Ergänzung zu Abschnitt 5, 11.3 und E1.2)
- E28 (L2-1, L2-2, L2-3): Die drei offenen Abnahmesignale von E17 werden als reine Testergänzungen ohne Produktionsänderung geschrieben: Kindmodule der privaten Funktionen (`SOS/lan_presence_auth.rs` vorhandenes `task_tests`, `SOS/lan_presence.rs` vorhandenes `task_tests`, neue Kindmodule von `SOS/direct_reciprocal_persistence.rs` und `SC/node_accept.rs`). Verworfen: Testkonstruktor `LanAuthenticator::for_test` und `renew_contact_material_with(..)` (Produktionsänderung ohne Nutzen, weil die Kindmodule die privaten Teile erreichen). Der Schreibpfad in den Sicherheitsspeicher bleibt ohne Test (kein Test des Crates schreibt den Sicherheitsspeicher des Systems).
- E29 (L2-4): `native/crates/cli/src/cli/update.rs` wird durch Verschieben des Testmoduls nach `native/crates/cli/src/cli/update_tests.rs` unter 500 Zeilen gebracht (Muster `update_copy.rs` und `update_copy_tests.rs`). Verworfen: Kürzen von Produktionscode (Verhaltensrisiko ohne Nutzen).
- E30 (L2-5): `entscheidungen.md` E17 wird an den Code angeglichen; das Verhalten im LAN bleibt, weil Kontakte, die das Gerät nur im LAN erreicht, sonst verloren gingen (Nutzerbedingung „ohne Verlust“, Abschnitt 11.5, Abweichungssatz). Verworfen: Ankündigung unter der auslaufenden ID entfernen (Kontaktverlust im LAN, verletzt 11.6 „Kein Kontakt geht durch die Code-Erneuerung verloren“); Geheimnis je Kontakt (in E17 bereits verworfen, eigener Batch).
- E31: Registerzeilen der neuen und verschobenen Tests ändert allein das Paket suite-3-register (wie E27).

### E2.3 Meilensteine (Ergänzung zu Abschnitt 6, 11.4 und E1.3)
- M28 (Paket suite-3-erneuerung): cr-F4d `geschrieben` (Ankündigung unter auslaufender ID: entsteht mit dem Registereintrag, versiegelt mit dem alten Geheimnis und nicht mit dem aktuellen, verschwindet mit dem Ende der ID; `append_records` erzeugt `se3-` und `se-` je Eintrag); cr-F2d `geschrieben` für `renewable_contact` (ein `Some`-Fall, sechs `None`-Fälle) und für die Abweisung eines entfernten Geräts in `renew_contact_material` oder mit belegtem Grund `nicht abbildbar`; cr-F4c `geschrieben` (`relay_names_withdrawn`: entfernter Schlüssel wahr, anderer Schlüssel, IP-Eingang und zurückgesetzte Ansicht falsch).
- M29 (Paket f-cli-3): `native/crates/cli/src/cli/update.rs` unter 500 Zeilen; Tests unverändert in `native/crates/cli/src/cli/update_tests.rs`.
- M30 (Paket suite-3-register): `native/test-register.py` ohne Befund.
- M31 (Paket plandoku-3): E17 in `entscheidungen.md` nennt die Adressliste im LAN unter Risiko (1); `plan.md` trägt Ergänzung 2.
- M15 (erneut geändert): erfüllt, wenn zusätzlich cr-F2d, cr-F4c und cr-F4d in der Erwartungsdatei der Suite suite-3-erneuerung nach M28 stehen.

### E2.4 Sicherheit (Ergänzung zu Abschnitt 8, 11.5 und E1.4)
- Keine Eigenschaft aus 8, 11.5 und E1.4 wird geschwächt: E28 und E29 ändern keinen Produktionspfad; E30 ändert nur ein Dokument.
- E17 Widerruf (11.5): Die Abnahme deckt jetzt den Relay-Eingang (entfernter Schlüssel erhält keinen TLS-Handshake ohne Pin), die Abweisung der Wiedereinschreibung eines entfernten Geräts über eine Erneuerung und das Ende der LAN-Ankündigung mit der auslaufenden ID. Vertrauensgrenze, Prüfung und Fehlerverhalten wie 11.5; bekannte Abweichung im LAN wie 11.5 (Abweichungssatz) und `entscheidungen.md` E17 Risiko (1).
- Testgrenzen: prozessweite Registeransicht und Umgebungsvariablen werden in jedem Test über Drop-Wächter zurückgesetzt; kein Test berührt den Sicherheitsspeicher des Systems und kein Netz.

### E2.5 Pakete (Ergänzung zu Abschnitt 9, 11.7 und E1.5)
| Paket | abhängig von |
|---|---|
| suite-3-erneuerung | – |
| f-cli-3 | – |
| suite-3-register | suite-3-erneuerung, f-cli-3 |
| plandoku-3 | – |

Zuordnung Lücke → Paket: L2-1, L2-2, L2-3 → suite-3-erneuerung, suite-3-register; L2-4 → f-cli-3, suite-3-register; L2-5 → plandoku-3; Plandokument der Ergänzung → plandoku-3.

Parallele Pakete haben keine gemeinsame änderbare Datei: suite-3-erneuerung ändert Testdateien und zwei `mod`-Anfügungen unter `ENG/share/`; f-cli-3 ändert `native/crates/cli/src/cli/update.rs` und legt `native/crates/cli/src/cli/update_tests.rs` an; plandoku-3 ändert zwei Dateien unter `docs/plaene/2026-11-07-sicherheit-direktweg/`. suite-3-register ist das einzige Paket, das `docs/tests/register.tsv` ändert; es läuft nach suite-3-erneuerung und f-cli-3.

### E2.6 Fragen an den Nutzer
Keine neue Frage.

## Ergänzung 3 (2026-11-07)

Ausgangslage: Fehlerursachen F1 und F2 aus dem Remote-Suite-Lauf 42425595186 auf Commit 4bc43f14 (Kompilierfehler in der Engine-Crate und drei fehlgeschlagene QAD-Tests). Bewertung je Fehlerursache mit Datei und Stelle in E3.1; Entscheidungen E32 und E33; Meilensteine M32 bis M34; Pakete f-entdeckung-4 und suite-4-server sowie das Plandokument dieser Ergänzung.

### E3.1 Bewertung
- F1 bestätigt: Kompilierfehler E0034 in `native/crates/engine/src/share/core/lan_privacy_seal.rs` Zeile 48 (Funktion `seal_key`, Aufruf `Hmac::<Sha256>::new_from_slice(secret)`). Ursache: Die Datei importiert `KeyInit` aus `chacha20poly1305` (Zeile 11) und `Mac` aus `hmac` (Zeile 13). Beide Traits bieten `new_from_slice` für `Hmac<Sha256>` an, weil beide Crates denselben `crypto-common` nutzen; der unqualifizierte Aufruf ist daher mehrdeutig. Die Engine-Crate bricht damit ab. Die Crates darüber (`gui`, `cli`, Programme, `android-bridge`) sind im Lauf nicht kompiliert worden.
- Die Schwesterdateien `native/crates/engine/src/share/core/lan_privacy.rs` Zeile 37 und `native/crates/engine/src/share/core/signal_publish.rs` Zeile 58 rufen `new_from_slice` ohne `KeyInit` im Geltungsbereich auf und sind vom Fehler nicht betroffen. Dateien unter `native`, `share-server` und `android-bridge`, die `new_from_slice`, `KeyInit` und `Mac` zugleich enthalten: Die übrigen mit beiden Traits im Geltungsbereich (`native/crates/core/src/cryptomator/core/content_chunks.rs`, `vault_config.rs`) rufen bereits vollqualifiziert `<… as KeyInit>::new_from_slice` auf.
- F2 bestätigt, Testfehler und kein Produktfehler: In `share-server/src/relay_qad_tests.rs` Zeilen 131 bis 138 (Funktion `block_on_within`) wertet `runtime.block_on(tokio::time::timeout(TEST_LIMIT, future))` das `timeout(..)` vor `block_on` aus. `tokio::time::timeout` legt dabei sofort den Zeitgeber an, der eine laufende Runtime braucht; ohne Runtime bricht er mit „no reactor running“ ab. Betroffen sind genau die drei Tests `qad_limits_end_to_end` (`share-server/src/relay_qad_limits_tests.rs` Zeile 282), `qad_reports_the_clients_own_socket_address` (`relay_qad_tests.rs` Zeile 362) und `qad_restart_retries_only_bind_errors` (`relay_qad_tests.rs` ab Zeile 281, über `bind_failure` Zeile 264 und `quic_failure` Zeile 273). Die Feature `time` von `tokio` ist in `share-server/Cargo.toml` Zeile 28 aktiv.
- Infrastrukturfehler: keiner im Eingang. Die Warnungen zu `fetch_update` sind keine Fehler, werden ohne `-D warnings` nur gemeldet und haben kein Paket; belegt ist die Fundstelle `native/crates/engine/src/share/os/shared/route_monitor.rs` Zeile 58, die vollständige Fundstellenliste ist unbekannt.

### E3.2 Entscheidungen (Ergänzung zu Abschnitt 5, 11.3, E1.2 und E2.2)
- E32 (F1): Der Aufruf wird vollqualifiziert auf `<Hmac<Sha256> as Mac>::new_from_slice(secret)` festgelegt. Verhalten und Fehlerfall (`None`) bleiben unverändert. Verworfen: `KeyInit` aus dem Import entfernen und `ChaCha20Poly1305::new` an zwei Stellen vollqualifizieren (drei geänderte Zeilen statt einer, gleicher Nutzen). Annahme A4: die Version von `hmac` in `native/Cargo.toml` bietet `Mac::new_from_slice` mit `Result<Self, InvalidLength>`; die Umsetzung prüft das am Quelltext der Abhängigkeit. Ist A4 falsch, wird `<Hmac<Sha256> as KeyInit>::new_from_slice` verwendet, wie in `native/crates/core/src/cryptomator/core/content_chunks.rs` Zeile 85.
- E33 (F2): Der Timeout entsteht in einem async-Block, den `block_on` ausführt, so dass der Zeitgeber innerhalb der Runtime angelegt wird. Verworfen: `runtime.enter()` vor dem Aufruf (zusätzlicher Wächter ohne Nutzen); Entfernen der Zeitgrenze (Tests könnten unbegrenzt hängen). Geändert wird nur Testcode.
- Kein Registerpaket: keine Testdatei und kein Testname ändert sich; `docs/tests/register.tsv` bleibt gültig.

### E3.3 Meilensteine (Ergänzung zu Abschnitt 6, 11.4, E1.3 und E2.3)
- M32 (f-entdeckung-4): `smart_explorer_engine` kompiliert ohne E0034; die LAN-Versiegelung `v=3` bleibt unverändert.
- M33 (suite-4-server): Die drei Tests `qad_limits_end_to_end`, `qad_reports_the_clients_own_socket_address` und `qad_restart_retries_only_bind_errors` laufen ohne Panik „no reactor running“. Offen: ob sie inhaltlich bestehen; das zeigt erst der nächste Lauf derselben Suite.
- M34 (plandoku-4): `docs/plaene/2026-11-07-sicherheit-direktweg/plan.md` trägt Ergänzung 3.
- Offen nach F1: Die Rust-Tests der Jobs windows-gnu, workspace (ubuntu-24.04 und windows-2025) und android (arm64-v8a und x86_64) sind im Lauf nicht ausgeführt worden, und die Crates oberhalb der Engine sind nicht kompiliert worden. Ob sie weitere Fehler enthalten, ist unbekannt; das zeigt erst der nächste Lauf derselben Suite.

### E3.4 Sicherheit (Ergänzung zu Abschnitt 8, 11.5, E1.4 und E2.4)
- Keine Eigenschaft aus Abschnitt 8, 11.5, E1.4 und E2.4 wird geschwächt: E32 ändert nur die Auflösung des Trait-Aufrufs; Schlüsselableitung (HMAC-SHA256 über Domäne, Lookup-ID, Epoche und Ablauf) und Ablehnung falscher Schlüssellängen bleiben. E33 ändert nur Testcode.

### E3.5 Pakete (Ergänzung zu Abschnitt 9, 11.7, E1.5 und E2.5)
| Paket | Rolle | abhängig von |
|---|---|---|
| f-entdeckung-4 | umsetzung | – |
| suite-4-server | umsetzung | – |
| plandoku-4 | umsetzung | – |

Zuordnung: F1 → f-entdeckung-4; F2 → suite-4-server; Plandokument dieser Ergänzung → plandoku-4. Die drei Pakete ändern keine gemeinsame Datei und laufen parallel.

### E3.6 Fragen an den Nutzer
Keine neue Frage.

## Ergänzung 4 (2026-11-08)

Ausgangslage: Fehlerursache F1 aus dem Remote-Suite-Lauf 42428751187 auf Commit aaf46d7b (Kompilierfehler in der Engine-Crate). Bewertung in E4.1; Entscheidung E34; Meilensteine M35 und M36; Pakete suite-5-zugang und plandoku-5.

### E4.1 Bewertung
- F1 bestätigt: Kompilierfehler E0599 in `native/crates/engine/src/share/os/shared/lan_link_transport_task_fixture.rs` Zeile 177 (Funktion `permits`). Der Aufruf `node.handshake_slots.available_permits()` existiert nicht, weil das Feld `handshake_slots` seit Commit e5bb16d5 vom Typ `Arc<TlsHandshakeSlots>` ist (`native/crates/engine/src/share/core/node.rs` Zeile 68). `TlsHandshakeSlots` in `native/crates/engine/src/share/core/handshake_limits_tls.rs` (Zeilen 91 bis 161) hat keine Methode `available_permits`; die Testmethode `held` (Zeilen 155 bis 160) liefert die belegten Plätze. Dieser Aufruf steht als einzige Stelle unter `native/crates/engine/src`.
- Im Lauf tritt genau dieser Fehlercode auf, in den Jobs windows-gnu, workspace (windows-2025) und workspace (ubuntu-24.04). Ein Infrastrukturfehler liegt im Eingang nicht vor. Warnungen (etwa `fetch_update`, unbenutzte Importe) sind keine Fehlerursache dieser Ergänzung.

### E4.2 Entscheidungen (Ergänzung zu Abschnitt 5, 11.3, E1.2, E2.2 und E3.2)
- E34 (F1): `permits` liest als ersten Wert die insgesamt belegten TLS-Handshake-Plätze über `held(&TlsSource::Other(String::new())).0`. Nur die Gesamtzahl wird gelesen; die Quelle ist dafür beliebig, und `held` verändert keinen Zustand. Der Vergleich mit dem Ausgangswert in `assert_status_only` prüft weiter, dass ein Status-Austausch keinen Handshake-Platz behält. `TlsSource` wird aus `crate::share::handshake_limits` importiert, wie in `native/crates/engine/src/share/core/login_transport_admission.rs` Zeile 12. Die Fixture liegt unter `crate::share` und unter `#[cfg(test)]`; dadurch sind `TlsSource` und `held` erreichbar. Verworfen: eine neue `#[cfg(test)]`-Methode in `handshake_limits_tls.rs` (zweite Datei, gleicher Nutzen); das Streichen des ersten Tupelwerts (die Prüfung, dass der Status-Weg keinen Handshake-Platz behält, ginge verloren).
- Annahme: Das gemischte Tupel (erster Wert belegt, zweiter und dritter Wert frei) ist für den Gleichheitsvergleich mit dem Ausgangswert gleichwertig; ein Kommentar im Testcode macht das sichtbar.
- Kein Registerpaket: Testnamen und Testdateien ändern sich nicht; `docs/tests/register.tsv` bleibt gültig.

### E4.3 Meilensteine (Ergänzung zu Abschnitt 6, 11.4, E1.3, E2.3 und E3.3)
- M35 (suite-5-zugang): `smart_explorer_engine` (Testbinärdatei der Bibliothek) kompiliert ohne E0599. Die S09-Prüfung „Status belegt keinen Platz“ vergleicht die belegten TLS-Handshake-Plätze, die freien Reparaturplätze und die freien Übergangsplätze mit dem Ausgangswert. Ob die Tests der Engine-Crate bestehen, zeigt erst der nächste Lauf derselben Suite.
- M36 (plandoku-5): `docs/plaene/2026-11-07-sicherheit-direktweg/plan.md` trägt den Abschnitt Ergänzung 4.

### E4.4 Sicherheit (Ergänzung zu Abschnitt 8, 11.5, E1.4, E2.4 und E3.4)
- Keine Eigenschaft aus Abschnitt 8, 11.5, E1.4, E2.4 und E3.4 wird geschwächt. E34 ändert nur Testcode. Die Regeln der TLS-Handshake-Plätze (ein Viertel je Quelle, Wiederholung ab der Hälfte, Reserve für gepinnte Relay-Sender) bleiben unverändert.

### E4.5 Pakete (Ergänzung zu Abschnitt 9, 11.7, E1.5, E2.5 und E3.5)
| Paket | Rolle | abhängig von |
|---|---|---|
| suite-5-zugang | umsetzung | – |
| plandoku-5 | umsetzung | – |

Zuordnung: F1 → suite-5-zugang; Plandokument dieser Ergänzung → plandoku-5. Die beiden Pakete ändern keine gemeinsame Datei und laufen parallel.

### E4.6 Fragen an den Nutzer
Keine neue Frage.

## Ergänzung 5 (2026-11-08)

Eingang: CI-Auswertung des Remote-Suite-Laufs 42432482161 auf Commit 165955d9; fehlgeschlagen sind nur workspace (ubuntu-24.04) und workspace (windows-2025), jeweils im Schritt „Every crate, every test“. Pfadkürzel: `DOS` = `native/crates/engine/src/daemon/os/shared`, `SOS` = `native/crates/engine/src/share/os/shared`, `TS` = `native/crates/transfer/src/sync/os/shared`, `GC` = `native/crates/gui/src/app/core`. Bewertung je Fehlerstelle mit Datei und Stelle, Entscheidungen E35 bis E40, Annahmen A8 bis A10 und offene Punkte: siehe E5.1 bis E5.6.

### E5.1 Bewertung
- Fehlende Joblogs (agent, servers, android arm64-v8a, android x86_64): nachgeholt. Alle vier Jobs sind erfolgreich und melden jeweils ein bestandenes Testergebnis. Keine Fehlerstelle.
- F1 bestätigt (beide workspace-Jobs): Der Hilfsthread von `worker_job_task_progress_cancel_and_end_travel_over_ipc` (`DOS/transfer_job_client_tests.rs` Zeilen 149–155) beantwortet den IPC-Besitznachweis nicht, den `ask` (`DOS/transfer_job_client.rs` Zeile 161) seit fc0b80c1 (Paket f-lokal) über `ipc_proof::connect_token` zuerst sendet. Testcode.
- F2 und F3 bestätigt (nur windows-2025): Ein Windows-Host beobachtet keine lokale Freigabewurzel. `SOS/host_watch.rs` nutzt nur `watch_confined`, das ohne `watch_path` mit Unsupported endet (`native/crates/transfer/src/watch/os/shared/service.rs` Zeilen 148–153); unter Windows ist `watch_path` immer None (`native/crates/core/src/local_access/os/windows/directory_handle.rs` Zeilen 71–73). Vorbestehend (f0d22e17/02215055); die Suite-Tests aus 5eb6d963 setzen eine Beobachtung voraus. Testcode; die Produktlücke wird als offener Punkt geführt (E40).
- F4 bestätigt (nur windows-2025): `GC/update_unsigned_ui_tests.rs` Zeile 126 sucht `"\n    }\n"` im per `include_str!` eingebundenen Quelltext, den der Windows-Checkout mit CRLF tragen kann (`.gitattributes` enthält keine Regel für `*.rs`). Testcode.
- F5 (nur windows-2025): Der Abbruchgrund in `result.errors` steht in keinem Log, weil der Test ihn nicht ausgibt. Diese Angabe bleibt fehlend. An der Quelle bestimmt und bestätigt: Die Revalidierung in `TS/sync_delete.rs` (Zeile 117) vergleicht auch Verzeichnis-Kandidaten mit `current` einschließlich `mtime_ms`; deren Beobachtung stammt aus der Ordnerauflistung, weil `observe` seit bfedc70a nur für Dateien per `stat` läuft (Zeile 85). Unter Windows kommt diese Zeit aus dem Index des Elternordners, `stat` aus dem Verzeichnis selbst. Weicht sie ab, bricht die Löschung mit „nothing deleted“ ab. Vorbestehend; kein Paket dieser Ergänzung hat die Dateien vorher geändert.
- Annahmen: A8: Der Runner checkt mit `core.autocrlf=true` aus. Im Log nicht belegt; E37 ist ohne diese Annahme korrekt. A9: Der Abbruch aus F5 in diesem Lauf stammt von der Verzeichnis-Revalidierung. Kein Gegenbeleg, kein Laufbeleg. A10: NTFS führt die Zeit eines Unterverzeichnisses im Index des Elternordners nicht sofort nach (dieselbe Einschränkung, die bfedc70a für Dateien nennt). Ohne Webbeleg.
- Kein vorübergehender Infrastrukturfehler im Eingang.

### E5.2 Entscheidungen (Ergänzung zu Abschnitt 5, 11.3, E1.2, E2.2, E3.2 und E4.2)
- E35 (F1): Der Test beantwortet den Nachweis nach `control.accept()` mit `crate::daemon::ipc_proof::answer_for_test(&mut cancel, "token")`, wie `DOS/ipc_open_client.rs` Zeile 176.
- E36 (F2, F3): Beide Tests bleiben auf allen Plattformen. Unter Windows prüfen sie `ErrorKind::Unsupported` statt einer Beobachtung; die PermissionDenied-Abweisungen in und auf dem Tresor-Laufwerk bleiben auf allen Plattformen geprüft. Verworfen: `#[cfg(not(windows))]` am Test, weil unter Windows dann keine Prüfung mehr stattfände.
- E37 (F4): Der Test sucht in `FRAME_KEYBOARD.replace("\r\n", "\n")`. Verworfen: eine `.gitattributes`-Regel, weil sie das Checkout-Verhalten einer Produktdatei für einen Test ändern würde.
- E38 (F5, Produkt): `observe` beobachtet jeden Kandidaten, auch Verzeichnisse, per `stat` und lehnt ab, wenn sich die Art geändert hat oder ein Link oder eine Sonderdatei vorliegt; die Revalidierung vergleicht dann `stat` mit `stat`. Kein Vergleich wird gelockert. Verworfen: Verzeichnisse in der Revalidierung über `current_directory` prüfen, weil dabei Größe und Zeit wegfielen.
- E39 (F5, Test): `delete_extra_removes_orphans` gibt `result.errors` und `result.stats` in jeder Assertion aus. `mirror_deletion_compares_with_the_file_not_a_stale_listing` erhält einen zweiten Fall mit veralteter Auflistung des Verzeichnisses `gone` (Trockenlauf, `stats.deleted == 2`, keine „nothing deleted“-Meldung). Die Registerzeile nennt Dateien und Ordner; Testnamen bleiben.
- E40 (Produktlücke): `docs/TODO.md` führt die vorbestehende Lücke „Windows-Host beobachtet keine lokale Freigabe“ als offenen Punkt `WIN-SHARE-WATCH`. Keine Produktänderung in dieser Ergänzung.

### E5.3 Meilensteine (Ergänzung zu Abschnitt 6, 11.4, E1.3, E2.3, E3.3 und E4.3)
- M37 (suite-6-tests): F1 besteht in beiden workspace-Jobs; F2, F3 und F4 bestehen unter windows-2025; `delete_extra_removes_orphans` zeigt bei einem Fehlschlag den Abbruchgrund; beide Fälle von `mirror_deletion_compares_with_the_file_not_a_stale_listing` bestehen auf allen Plattformen (Fall 2 zusammen mit M38).
- M38 (f-spiegel-6): Die Spiegel-Löschung beobachtet Verzeichnisse per `stat`; ein verwaistes Verzeichnis mit veralteter Auflistungszeit wird gelöscht; jede Änderung nach der Beobachtung bricht weiter ohne Löschung ab.
- M39 (doku-6): `docs/TODO.md` trägt `WIN-SHARE-WATCH`.
- M40 (plandoku-6): `docs/plaene/2026-11-07-sicherheit-direktweg/plan.md` trägt Ergänzung 5.

### E5.4 Sicherheit (Ergänzung zu Abschnitt 8, 11.5, E1.4, E2.4, E3.4 und E4.4)
- Keine Eigenschaft aus Abschnitt 8, 11.5, E1.4, E2.4, E3.4 und E4.4 wird geschwächt. E35, E36, E37 und E39 ändern nur Testcode; die IPC-Besitzprüfung bleibt unverändert. E38 wechselt im Spiegel-Löschpfad nur die Quelle der Beobachtung zu `stat`; alle Vergleichsfelder, der Abbruch ohne Löschung, die leere Auflistung vor `remove_dir`, die Versionierung und der Ausschluss von Links und Sonderdateien bleiben erhalten. E40 ist Dokumentation.

### E5.5 Pakete (Ergänzung zu Abschnitt 9, 11.7, E1.5, E2.5, E3.5 und E4.5)
| Paket | Rolle | abhängig von |
|---|---|---|
| suite-6-tests | umsetzung | – |
| f-spiegel-6 | umsetzung | – |
| doku-6 | umsetzung | – |
| plandoku-6 | umsetzung | – |

Zuordnung: F1, F2, F3, F4 und der Testteil von F5 → suite-6-tests; Produktteil von F5 → f-spiegel-6; Produktlücke Windows-Beobachtung → doku-6; Plandokument → plandoku-6. Änderbare Projektdateien: suite-6-tests `DOS/transfer_job_client_tests.rs`, `SOS/host_watch_task_tests.rs`, `GC/update_unsigned_ui_tests.rs`, `TS/sync_tests.rs` und eine Zeile in `docs/tests/register.tsv`; f-spiegel-6 `TS/sync_delete.rs`; doku-6 `docs/TODO.md`; plandoku-6 `docs/plaene/2026-11-07-sicherheit-direktweg/plan.md`. Keine gemeinsame änderbare Datei, daher alle vier Pakete in einer Welle.

### E5.6 Fragen an den Nutzer
Keine neue Frage.

## Stand (2026-11-08)

- Version: 0.5.196 (`native/Cargo.toml`, Zeile 36, Wert `version = "0.5.196"`; Tag `v0.5.196` zeigt auf `29cd6ea8`).
- Release-Commit: `29cd6ea8` „Release Smart Explorer v0.5.196“ auf `main`; Vorgänger `c189077e`.
- Remote-Task-Suite: Lauf `42439947911` auf `c189077e`, alle sieben Jobs erfolgreich. Vorher fehlgeschlagen: `42425595186`, `42428751187`, `42432482161`.
- Release-Lauf: `42444153827` von `.github/workflows/release.yml`, erfolgreich.
- GitHub Release `v0.5.196` mit 28 Assets, darunter neu `release-manifest` und `release-manifest.sig`; `releases/latest/download/version.txt` antwortet `0.5.196`.
- Release-Signatur (M24, E20): Release-Lauf `42444153827` erfolgreich, also war `SE_UPDATE_SIGNING_KEY` gesetzt (der Schritt „Require the release signing key“ in `.github/workflows/release.yml` bricht sonst vor jedem Bau ab; Hinweis in E2.1); die Signatur des veröffentlichten `release-manifest` ist am 2026-11-08 mit dem ins Programm eingebauten öffentlichen Ed25519-Schlüssel geprüft: gültig.
- Mit Lauf `42439947911` bestanden sind die Prüfpunkte, die laut den Ergänzungen erst der nächste Suite-Lauf zeigt: M33 (QAD-Tests inhaltlich), „Offen nach F1“ in Ergänzung 3 (Tests der Jobs windows-gnu, workspace und android), M35 (Tests der Engine-Crate), M37 und M38. Annahme A9 (Ergänzung 5) bleibt Annahme: Der erfolgreiche Lauf zeigt nicht, woher der Abbruch im Lauf `42432482161` kam.
- Offen nach dem Release: die Teile, die nur echte Geräte und Netze belegen; sie stehen in `docs/TODO.md` (unter anderem `SEC-ENTDECK-CODE-MESSUNG`, `SEC-DNS-ANDROID`, `SEC-UNBELEGT-VERSCHIEDEN`, `SEC-UNBELEGT-LAN-ENTDECK`).
- Die Befunde und Entscheidungen der Ergänzungen 1 bis 5 bleiben unverändert.
