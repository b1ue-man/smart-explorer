# ApplicationPathPolicy: kapselungserhaltende Konstruktion

Stand: 2026-10-26. Ausgangskandidat:
`7fb96ef00bfb864509c4b0918aac211f7eafb5b6`. Fehlerbeleg: gemeinsame
Remote-Suite `40344852826`, bereitgestellte `failure-summary.json`. Dieser
Bericht gehört zum laufenden Gesamtbatch und behauptet keinen neuen Release.

## Auftrag und Erstplan aus dem delegierten Ergebnisauftrag

Der Hauptagent hat ausschließlich die Behebung des kandidatenbelegten E0451
bei der Initialisierung von `ApplicationPathPolicy` delegiert. Der Nutzervertrag
verlangt vom selben Fixautor fünf Ergebnisse: fehlerhafte Funktion, direkte und
indirekte Verknüpfungskarte der berührten Grenze, regressionsschützender Fixplan,
Umsetzung und eigene Anti-Regressionsreview. Dieser Bericht führt diese fünf
Ergebnisse zusammen.

Vorgaben des Ergebnisauftrags: identischer Startzustand, identische
Opt-in-/Disabled-Semantik, identischer RNG-Verbrauch und RNG-Zeitpunkt, unverändert
private Zähler sowie kein Protokoll- oder Grantwechsel. Die einzige Schreibfläche
besteht aus `vendor/noq-proto-1.3.0/src/connection/mod.rs`,
`vendor/noq-proto-1.3.0/src/connection/application_path_policy.rs` und diesem
Bericht. Builds, Compiler, Formatter, Tests, Commits, Pushes, Graphupdates und
Releasekommandos sind für diesen Subagenten verboten. Der Hauptagent übernimmt
den Compiler- und Funktionsnachweis in derselben Remote-Suite.

Aus dem Prompt bekannt waren der Kandidat, der fehlgeschlagene Struct-Update-
Ausdruck in `Connection::new` und die drei privat gebliebenen Zähler. Vor der
Quelllektüre fehlten der genaue Defaultzustand, die RNG-Reihenfolge und die
betroffenen Integrationsverträge. Der Erstplan bestand darin, diese drei
Grundlagen in der freigegebenen Fläche zu klären und anschließend eine
modulinterne Initialisierung ohne Zustandsänderung einzusetzen. Eine neue
Nutzerentscheidung war dafür nicht erforderlich.

## Fehlerhafte Funktion und belegter Bestand

`noq_proto::connection::Connection::new` initialisiert seine Policy im Elternmodul
mit `ApplicationPathPolicy { connection: application_policy_connection,
..ApplicationPathPolicy::default() }`. Die drei Felder `network_change_count`,
`application_tx_bytes` und `application_rx_bytes` sind im Kindmodul
`application_path_policy` privat. Ein Struct-Update benötigt auch für nicht
ausgeschriebene Felder Sichtbarkeit. Die Remote-Suite meldet daher E0451 für
Linux und Windows sowohl beim Bibliotheks- als auch beim `se`-Development-Build;
die bereitgestellten Android-Fehlerzeilen nennen denselben Fehler. Die
bereitgestellte Vorbereitung meldet eine unveränderte `dependencies.json`.

Die fehlerhafte Stelle liegt hinter der Erzeugung von Initial-, Handshake- und
Data-Paketräumen sowie hinter der Initialisierung des lokalen CID-Zustands. Die
Kennung wird genau dort mit
`if config.application_path_policy { rng.random() } else { 0 }` erzeugt. Danach
folgen im `Connection`-Literal `CryptoState::new(..., &mut rng)` und der bestehende
Spinbit-Zweig. Diese Position und dieser Bedingungsausdruck bleiben erhalten.

Die beabsichtigte bisherige Initialisierung auf Basis des abgeleiteten `Default`
hat folgenden vollständigen Zustand; ausschließlich `connection` überschreibt
seinen Defaultwert:

| Feld | Startwert | Erhaltener Vertrag |
|---|---|---|
| `generation` | `0` | Erste Generation unverändert |
| `revision` | `0` | Erste Token-/Transmitrevision unverändert |
| `admitted` | `None` | Keine anfängliche Anwendungsfreigabe |
| `connection` | Übergebene `u64` statt Default `0` | Bestehende verbindungsgebundene Tokenkennung |
| `pending_validation` | Leere `BTreeSet` | Keine zusätzlich erfundene Validierungsrunde |
| `requires_new_generation` | `false` | Keine zusätzlich erfundene Netzänderung |
| `network_change_count` | `0` | Privater Diagnosezähler bleibt privat |
| `application_tx_bytes` | `0` | Privater TX-Zähler bleibt privat |
| `application_rx_bytes` | `0` | Privater RX-Zähler bleibt privat |
| `path_payload_bytes` | Leere `BTreeMap` | Keine geerbten pfadbezogenen Zähler |

## Verknüpfungskarte der konkret berührten Initialisierungsgrenze

Die vollständige Karte dieser Grenze umfasst Konfiguration, Konstruktion,
Policyzustand, seine direkten Verbraucher, Noq-/Iroh-Fassaden und den vorhandenen
App-/Suite-Anschluss. Sie ist keine Behauptung einer neuen Gesamtreview sämtlicher
Produktmodule. Die Fundorte beziehen sich auf den genannten Ausgangskandidaten;
die nachfolgende Umsetzung ergänzt ausschließlich die Policy-eigene
Konstruktormethode und ersetzt den einen Struct-Update-Aufruf.

| Verbindung | Konkrete Stellen und erhaltene Wirkung |
|---|---|
| Opt-in → Transportkonfiguration | `config/transport.rs::TransportConfig::application_path_policy` setzt das Flag; `Default` setzt es auf `false`; `Debug` zeigt es. `iroh/src/endpoint/quic.rs::QuicTransportConfigBuilder::application_path_policy` reicht es an Noq weiter; `build` bewahrt die Konfiguration im `Arc`. `ServerConfigBuilder::set_transport_config` reicht denselben Konfigurationswert weiter. |
| Client und Server → gemeinsamer Konstruktor | `noq-proto/src/endpoint.rs::Endpoint::connect` übergibt `config.transport`; `Endpoint::accept` übergibt `server_config.transport`. Beide rufen `Endpoint::add_connection`; dort entstehen RNG-Seed und qlog-Kontext, und dort wird `Connection::new` aufgerufen. |
| Konstruktion → Policy | `connection/mod.rs::Connection::new` erzeugt die bedingte `u64`-Kennung. Die neue `ApplicationPathPolicy::new` setzt ausschließlich `connection` und übernimmt sämtliche verbleibenden Felder aus demselben abgeleiteten `Default`. Der RNG bleibt Eigentum von `Connection::new`. |
| Policy → Beobachtung und Tokenbindung | `application_path_policy.rs::Connection::application_path_policy` liest Generation, Revision, private Gesamtzähler, Validierung und Pfadzähler; die Token enthalten die unveränderte Kennung. `check_application_path_token` prüft Verbindung, Generation, Revision und Pfadinkarnation. Die öffentlichen Snapshot-/Token-/Fehler-/Update-/Stamp-Typen bleiben unverändert. |
| Policy → Freigabe und Sperre | `update_application_path_policy` verarbeitet `BeginGeneration`, `Block`, `Allow` und `RetireRedundantPath`. Disabled/Closed/Stale-/Validierungs-/Revisionsexhaustionsprüfungen bleiben bestehen. `application_path_policy_enabled` liest unverändert das Konfigurationsflag. `application_data_allowed`, `application_data_allowed_on_path` und `application_transmit_stamp` nutzen denselben Policyzustand. |
| Policy → Serialisierungsentscheidungen | `connection/mod.rs::poll_transmit_path_space` berücksichtigt freigegebene Nutzdaten; `can_send_1rtt` fragt den Pfadgate ab; `populate_packet` lässt DATAGRAM-/STREAM-Verbrauch einschließlich Retransmits und 0-RTT nur nach den bestehenden Builderprüfungen zu. Der Fix ändert weder diese Entscheidungen noch Queueverhalten oder Paketkodierung. |
| Policy → Netzwechsel, Migration und Pfadleben | `open_path_ensure` und `handle_timeout` starten die bestehende Wiederholungsvalidierung; `close_path_inner` über `abandon_path`, die Handshake-Migration in `handle_packet` und `migrate` invalidieren betroffene Pfade. `process_payload` und `handle_network_change` invalidieren Netzepochen. `handle_path_response_on_path` entfernt nur nach passender Antwort die ausstehende Validierung; `discard_path` entfernt die zugehörigen Pfadzähler. |
| Policy → endliche Revalidierung und Kapazität | `application_path_revalidation.rs` enthält `application_validation_reference_pto`, `start_application_path_validation`, `start_application_path_validation_on`, `retry_application_path_validation`, `application_path_validation_failed`, `application_path_usable_for_retirement`, `application_path_retirement_allowed` und `retire_redundant_application_path`. Leere Startmengen, keine anfängliche Freigabe und bestehende PTO-/Timer-/Retirement-Schutzbedingungen bleiben erhalten. |
| Policy → Nutzdatenstatistik | `process_payload` zählt erfolgreich verarbeitete STREAM-/DATAGRAM-Bytes über `record_application_payload_rx`. `record_application_payload_tx` und `record_application_payload_rx` schreiben die weiterhin privaten Gesamtzähler und die Pfadmap; beide bleiben bei deaktivierter Policy wirkungslos. Der Konstruktor startet alle Zähler weiterhin bei null. |
| Noq-proto → Noq-Treiber | `noq-proto/src/lib.rs` exportiert die bestehenden öffentlichen Policytypen. `noq/src/connection.rs::Connecting::new` und `State::new` übernehmen den bestehenden `proto::Connection`; die Policy-API arbeitet weiter unter dem Treibermutex. `update_application_path_policy` invalidiert weiterhin gepufferte verschlüsselte I/O; `State::drive_transmit` vergleicht weiterhin den Stamp vor einem Pending-I/O-Retry. |
| Noq → Iroh → Appbeobachtung | `iroh/src/endpoint/quic.rs` exportiert die Policytypen; `iroh/src/endpoint/connection.rs` delegiert Snapshot, Update und Datenzulassung unverändert an Noq. `native/src/share/os/shared/route_adapter.rs::RouteAdapter::observe` ordnet bestehende Pfad-IDs den Snapshots und opaken Token zu; `RouteSelector::select` bleibt eine Präferenzentscheidung ohne Grant. |
| App-/Transportgrenze → vorhandene gemeinsame Suite | `native/src/share/core/batches_route_task_tests.rs::pair` aktiviert dieselbe Policy auf beiden Loopback-Endpunkten. `allow` benutzt frische Tokens. `batches_task_b5_actual_stream_and_datagram_gate_survives_network_epoch` verlangt anfangs null Payload, Übertragung nach Allow, erneute Sperre beim Netzwechsel, Zurückweisung alter Tokens, Wiederaufnahme desselben Streams und getrennte Gegenrichtungsfreigabe. `batches_task_b5_route_cancellation_closes_blocked_transport_without_payload` erhält Abbruch und Null-Payload. `native/review-task-manifest.json` wählt `batches_`, verlangt `batches_task_b5_` und bindet P28 sowie K4 an diesen Pfad. |

## Stufe eins: Ansatz und Primärquellen

Aus dem Bestand folgt ein begrenzter Konstruktorfix. Die Policy bekommt im
definierenden Modul `pub(super) fn new(connection: u64) -> Self`. Diese Methode
verwendet `Self { connection, ..Self::default() }`. Der Elternaufrufer ersetzt
seinen Struct-Update-Ausdruck durch `ApplicationPathPolicy::new` und übergibt die
bereits an derselben Stelle erzeugte Kennung.

Abgerufen am 2026-10-26:

- [Rust Reference: Functional update syntax](https://doc.rust-lang.org/reference/expressions/struct-expr.html#functional-update-syntax): Ein Update übernimmt nicht benannte Felder aus seinem Basiswert; sämtliche Felder müssen am Ausdruck sichtbar sein.
- [Rust Error Index: E0451](https://doc.rust-lang.org/error_codes/E0451.html): Eine Funktion im definierenden Modul ist der dokumentierte Weg, private Felder bei der Konstruktion zu erhalten.
- [Rust Reference: Visibility and privacy](https://doc.rust-lang.org/reference/visibility-and-privacy.html#pubin-path-pubcrate-pubsuper-and-pubself): Private Felder sind im definierenden Modul zugreifbar; `pub(super)` macht die Initialisierungsmethode für das Elternmodul sichtbar.

Die alternative Sichtbarkeitserweiterung der drei Zähler ist ausgeschlossen,
weil sie Kapselung aufweichen würde. Eine Zufallserzeugung in der neuen Methode
ist ausgeschlossen, weil sie RNG-Verantwortung und den deaktivierten Zweig
unnötig verändern würde. Eine ausgeschriebene zehnfache Feldinitialisierung
würde den Default duplizieren; die modulinterne Wiederverwendung erhält denselben
einzigen Defaultvertrag. Zusätzliche Abhängigkeiten, Locks, Allokationsarten,
Protokollfelder und Berechtigungsentscheidungen werden nicht benötigt.

Die Wahl wurde auf Korrektheit, Sicherheit, Effizienz, Zuverlässigkeit und Wartung
bewertet: legaler Privacy-Kontext, unveränderte private Zähler, reine Übergabe
einer vorhandenen Zahl, identischer Default und eine einzige interne
Initialisierungsgrenze. Es gibt keinen zusätzlichen Netzwerk- oder
Zugangsdatenbedarf für diese Entscheidung.

## Stufe zwei: endgültiger Fixplan vor Quelländerung

| Meilenstein | Betroffene Dateien | Abhängigkeit | Konkretes erwartetes Ergebnis |
|---|---|---|---|
| M1: Policy-eigene Initialisierung | `application_path_policy.rs` | Gesicherter Default und Rust-Privacy-Vertrag | `new(connection)` ist `pub(super)` sichtbar, enthält keine RNG-/Gate-/Protokollaktion und setzt exakt den vorhandenen Default plus übergebener Kennung. Alle drei privaten Zähler behalten ihre Deklaration. |
| M2: Ein vorhandener Aufrufer | `connection/mod.rs` | M1 | Der Elternkonstruktor ruft `ApplicationPathPolicy::new(application_policy_connection)` auf. Die bedingte RNG-Ziehung sowie davor und danach liegende RNG-Verbraucher bleiben wortgleich und an ihrer ursprünglichen Position. |
| M3: Eigenreview und Übergabe | Dieser Bericht und die beiden Quellhunks | M1, M2 | Dieselben zehn Startfelder, opt-in/disabled, Token-/Grant-/Stamp-Grenzen, Client-/Serverpfad und sämtliche Tabellenverbindungen sind statisch auf Regressionen geprüft. Der Bericht nennt verbleibenden Compiler-/Funktionsnachweis ausdrücklich als offen; Main bestätigt ihn ausschließlich in derselben Remote-Suite. |

Zweite Lückenrecherche vor der Quelländerung: Die zunächst fehlenden direkten
Konstruktoraufrufer wurden durch die zusätzliche Lesefreigabe für `endpoint.rs`
und `endpoint/quic.rs` geklärt. Beide Seiten verwenden dieselbe zentrale
Initialisierung. Die Sichtbarkeit von `pub(super)` ist durch die Rust Reference
geklärt. Der Konstruktor benötigt weder `enabled` noch einen RNG-Parameter,
weil Opt-in und die bedingte Kennungsziehung bereits im bestehenden Aufrufer
liegen. Kein offener API-, Plattform-, Sicherheits- oder Protokollpunkt verändert
diesen Plan.

Vor Quelländerung festgelegter Reviewgegenstand: ausschließlich M1/M2 und ihre
oben kartierten Vertragsfolgen. Prüffragen sind legale Sichtbarkeit,
Startwertgleichheit, unveränderte RNG-Position/-Zahl, unveränderte öffentliche
Signaturen und unveränderte Gate-/Validierungs-/Statistiklogik. Quellen sind der
Kandidat, die drei Rust-Primärquellen und die gemeinsame Suite-Auswahl.

## Umsetzung und eigene Anti-Regressionsreview

M1 ist umgesetzt: `ApplicationPathPolicy::new(connection: u64) -> Self` befindet
sich im definierenden Kindmodul. Die Methode setzt `connection` und verwendet
für sämtliche verbleibenden Felder `Self::default()`. Sie ist `pub(super)`.

M2 ist umgesetzt: `Connection::new` ruft die Methode mit
`application_policy_connection` auf. Das Code-Diff besteht ausschließlich aus
dieser Ersetzung und der neuen internen Methode mit ihrem Dokumentationssatz.
Die drei privaten Zählerdeklarationen und der abgeleitete `Default` bleiben
wortgleich. Das Policymodul hat nach der Änderung 445 Zeilen und 22.455 Bytes.

M3 ist durch denselben Fixautor nach der Quelländerung statisch durchgeführt:

| Prüffrage | Eigener Beleg und Ergebnis |
|---|---|
| Ist der Konstruktor nach Rusts Privacy-Regeln legal? | Der einzige Struct-Update-Ausdruck befindet sich jetzt im definierenden Modul; dort sind alle zehn Felder sichtbar. Der Elternaufruf benötigt nur die bereits sichtbare Struktur und die `pub(super)`-Methode. Dies entspricht den drei oben gesicherten Rust-Primärquellen. Der Compilerbeleg steht noch aus. |
| Bleiben sämtliche Startwerte erhalten? | Vorher und nachher wird derselbe abgeleitete `Default` verwendet und ausschließlich `connection` überschrieben. Die vollständige Feldtabelle ist abgeglichen. Keine Freigabe, zusätzliche Revision, Validierungsmenge oder Nutzdatenstatistik entsteht. Die `u64`-Kennung wird unverändert übernommen, einschließlich des Werts `0`; es gibt keine neue Nichtnullbedingung. |
| Bleibt der aktivierte Start gesperrt? | `admitted` bleibt `None`, `generation` und `revision` bleiben `0`. `application_data_allowed_on_path` benötigt weiter eine explizite passende Freigabe und den etablierten, validierten Pfad. Der neue Konstruktor kann kein Allow auslösen. |
| Bleibt der deaktivierte Pfad unverändert? | Der bestehende Ausdruck liefert weiterhin `0` ohne RNG-Ziehung. Das Flag bleibt im `TransportConfig`, nicht im neuen Konstruktor. Die vorhandenen Methoden liefern unverändert unbeschränkte Gatezulassung, `Disabled` für Policyupdates und keinen Transmitstamp; die Payload-Zählmethoden bleiben wirkungslos. |
| Bleiben RNG-Zahl und RNG-Zeitpunkt gleich? | Die Zeile mit dem bedingten `rng.random()` und ihre Position hinter den Paketräumen/CIDs sowie vor `CryptoState::new` sind im Diff unverändert. Die neue Methode erhält nur eine bereits berechnete `u64`, besitzt keinen RNG-Zugriff und verwendet ausschließlich denselben Default. Initial-/Handshake-/Data-Sonderpfade und Spinbit-Bedingung bleiben unverändert. |
| Bleiben Token, Grants, Revisionen und private Zähler gekapselt? | Keine bestehende Feldsichtbarkeit und keine öffentliche Typsignatur wird verändert. `check_application_path_token`, sämtliche vier Updatevarianten, Netzinvalidierung, Stampbildung und beide Zählmethoden bleiben wortgleich. |
| Bleiben Client, Server, Validierung, Abbruch und Streams erhalten? | Beide in `Endpoint::add_connection` zusammenlaufenden Konstruktorpfade verwenden dieselbe Initialisierung. Keine Änderung betrifft Treibermutex, Pending-I/O-Invalidierung, Pfad-/Netzwechsel, Revalidierungsfristen, Retirement-Schutz, Streamidentität, Queues, Datagramme oder Abbruchpfade. Die oben kartierten Verbraucher erhalten denselben vollständig abgeglichenen Anfangszustand. Das ist statische Erhaltungsbegründung, kein behaupteter Laufzeiterfolg. |
| Ist die Änderung vollständig auf ihren Scope begrenzt? | Eigenes `git diff` zeigt genau die beiden geplanten Quellhunks. `git diff --check` für die beiden veränderten Vendorquellen ist ohne Befund. Native Quellen, Suite, Workflows, Cargo-Dateien, Graph, Versionen und Releaseartefakte wurden durch diesen Fixautor nicht geändert. |

Im eigenen Review wurden zwei reine Berichtsnamen präzisiert:
`poll_transmit_path_space` ist die tatsächliche Sendefunktion;
`close_path_inner` ruft die unmittelbar invalidierende Funktion `abandon_path`
auf. Es wurde kein zusätzlicher Quelldefekt im Fix festgestellt.

Die Entscheidung für M1/M2 bleibt bestehen. Ein zusätzlicher lokaler Test oder
ein neuer, die zwei Zuweisungen nachbildender Test ist nicht angelegt worden.
Die vorhandene gemeinsame Remote-Suite prüft bereits die betroffene
Transportintegration. Dieser Subagent hat keinen Build, Compiler, Formatter,
Test oder Releaseprozess ausgeführt.

Offener Nachweis: Der Hauptagent muss den korrigierten Kandidaten committen und
pushen und ausschließlich dieselbe gemeinsame Remote-Suite auswerten. Erst deren
Linux-, Windows- und Android-Ausgaben können die Beseitigung des E0451 praktisch
bestätigen; der vorhandene B5-Pfad liefert anschließend den tatsächlichen
Funktionsnachweis. Die fehlgeschlagene Suite ist dafür kein Pass. Eine weitere
Scopeabhängigkeit, Primärquellenfrage oder lokale Quellfrage ist für diesen Fix
nicht offen. Release und Veröffentlichung bleiben Teil des Hauptauftrags.

## Gelesene und geänderte Dateien

Gelesene Projektdateien; bei großen Integrationsdateien wurden die im Mapping
benannten Abschnitte und ihre benötigten Aufrufkontexte gelesen:

- `AGENTS.md`
- `docs/ARCHITEKTUR.md`
- `vendor/SMART_EXPLORER_ROUTE_POLICY.md`
- `vendor/noq-proto-1.3.0/src/connection/mod.rs`
- `vendor/noq-proto-1.3.0/src/connection/application_path_policy.rs`
- `vendor/noq-proto-1.3.0/src/connection/application_path_revalidation.rs`
- `vendor/noq-proto-1.3.0/src/config/transport.rs`
- `vendor/noq-proto-1.3.0/src/lib.rs`
- `vendor/noq-proto-1.3.0/src/endpoint.rs`
- `vendor/noq-1.3.0/src/connection.rs`
- `vendor/iroh-1.3.0/src/endpoint/connection.rs`
- `vendor/iroh-1.3.0/src/endpoint/quic.rs`
- `native/src/share/os/shared/route_adapter.rs`
- `native/src/share/core/batches_route_task_tests.rs`
- `native/review-task-manifest.json`
- `docs/plaene/2026-10-24-batches/fix-path-policy-construction.md`

Zusätzlich gelesen: globale `/root/.codex/AGENTS.md`,
`/root/.codex/skills/arbeitsweise/SKILL.md`,
`/root/.agents/skills/arbeitsweise/SKILL.md`, die über MCP gelieferten
Arbeitsweise-Anleitungen, die bereitgestellten Evidenzdateien `failure-summary.json`,
`smartexplorer-final-review-user-contract.txt` und
`smartexplorer-fix-path-policy-scope.json` sowie die drei oben verlinkten
Rust-Primärquellen. Der Hauptagent hatte Architektur und Rootgraph vorab geprüft;
der Rootgraph erfasst `native/src`, nicht diese Vendor-Konstruktorgrenze. Es gab
keine eigene Graphänderung.

Der Hauptagent hatte das Dokumentationskontextgate für den sauberen
Ausgangskandidaten durchgeführt. Vor Anlegen dieses Berichts bestätigte die
eigene gezielte Statusabfrage unveränderte drei Schreibpfade;
`git log -1 --oneline` zeigte `7fb96ef0`. Die gezielten `rg`- und Quellbefunde
stehen in der Fehlerbeschreibung und im Mapping. Dieser neue Bericht enthält
keine neue Versions-, Installer- oder Veröffentlichungsbehauptung.

Geänderte Dateien:

- `vendor/noq-proto-1.3.0/src/connection/application_path_policy.rs`
- `vendor/noq-proto-1.3.0/src/connection/mod.rs`

Erzeugte Datei:

- `docs/plaene/2026-10-24-batches/fix-path-policy-construction.md`

Keine Commits oder Pushes durch diesen Subagenten. Der abgegrenzte Quell- und
Berichtsauftrag ist zur Integration an den Hauptagenten übergeben; der praktische
Compiler-/Funktionsnachweis bleibt ausdrücklich dessen nächster Schritt.
