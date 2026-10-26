# M13 / F42: bestätigte Netzänderung und Anwendungspfadfreigabe

Stand: 2026-10-26. Arbeitskandidat `33257f82`; die Fehlerevidenz der gemeinsamen
Remote-Suite `40406246032` gehört zum quellidentischen Kandidaten
`1ffbf4b0bc90b2479947680abcd0a5941cce00f8`. Dieser Bericht ist der eigene
vollständige Fixzyklus desselben Autors. Er behauptet keinen Laufzeitpass und
keinen neuen Release.

## A — Auftrag und Erstplan

Quelle ist der delegierte Ergebnisauftrag M13/F42 mit dem wörtlichen
Nutzervertrag in `smartexplorer-final-review-user-contract.txt` sowie dem
abgeschlossenen `rv1-funktionsplan.md`. Ergebnis: Eine bestätigte tatsächliche
Netzänderung erreicht die Noq-Epoche und entzieht alten Anwendungspfadfreigaben
vor neuem Nutzverkehr ihre Wirkung. Ein potenzieller Iroh-Hinweis allein ist
kein Nachweis einer Änderung. Derselbe Autor liefert Fehlerfunktionsliste,
vollständige Verknüpfungskarte der betroffenen Grenze, Erhaltungsplan, Fix und
anschließende eigene Anti-Regressionsreview.

Der neue Scope `smartexplorer-f-route_epoch-scope.json` ersetzt den früheren
Konstruktorscope. Native Builds, Compiler, Tests, Formatter, Server,
Installationen, Unteragenten, Commits, Pushes, Graphupdates und Releaseprozesse
führt dieser Autor nicht aus. Main besitzt Integration, Manifestregistrierung
und denselben Remote-Suiteeinstieg. Vor Quelllektüre fehlten die tatsächliche
Meldungskette, die Stelle des ersten wirksamen Freigabeentzugs sowie die Ursache
der Fixtureannahme. Diese Fragen bestimmen die gezielte Lektüre; eine neue
Nutzerfreigabe ist für die beauftragte Änderung nicht erforderlich.

## B — Belegte Fehlerfunktionen und Bestand

1. **Fixturefehler, direkt durch beide Desktoplogs belegt:**
   `batches_task_b5_actual_stream_and_datagram_gate_survives_network_epoch`
   erwartet nach `left.network_change().await` einen gestiegenen
   `network_change_count`. Die isolierte Loopback-Fixture erzeugt dabei keine
   Netzänderung. Die Logs enden deshalb an `network invalidation reaches Noq`
   mit `Elapsed(())`. Der alte Erfolgskommentar der Fixture ist unbelegt.
2. **Produktlücke, aus der zusammenhängenden Quellfolge belegt:**
   `iroh::socket::Actor::handle_network_change` führt bei einer erkannten großen
   Änderung zuerst `transports_network_change.rebind()` aus. Erst anschließend
   erreicht `notify_quic_network_change` die Noq-Verbindungen. Ohne brauchbare
   Default-Route wird diese Meldung mit vorhandenem Backoff bis zu fünf Sekunden
   verschoben. Die bisherige Noq-Methode `Endpoint::handle_network_change`
   stellt ausschließlich Treiberereignisse in eine Queue. Damit fehlt vor dem
   Iroh-Rebind eine synchrone Grenze zum parallel sendenden QUIC-Treiber.
3. **Kein belegter Defekt:** `iroh::Endpoint::network_change` ist ausdrücklich
   ein potenzieller Hinweis an Netwatch. Der unveränderte Aufruf darf weiterhin
   ohne echte Änderung folgenlos bleiben. `Noq-proto::Connection::handle_network_change`
   entzieht die Freigabe bereits korrekt, sobald diese Meldung tatsächlich
   bearbeitet wird. Fehler ist ihre späte Erreichbarkeit vor dem Iroh-Rebind.
4. **Belegte Integrationsfolge des ersten Fixstands, im selben Zyklus korrigiert:**
   `BeginGeneration`, `complete_application_network_change` und
   `route_monitor::run` ließen ohne abgestimmte Wartephase das Sammel-/Direktfenster
   vor Netzbereitschaft ablaufen. Das konnte einen vorzeitigen Offline-Abbruch
   oder die spätere Auswahl mit bereits verbrauchtem Fenster auslösen. Der
   präzisierte Wartephasenplan dokumentiert die Quelle und die Nachkorrektur.
5. **Belegtes Beobachtungsrennen, im selben Zyklus korrigiert:**
   `route_monitor::run` kann aus einem alten Snapshot Offline veröffentlichen,
   nachdem der Actor bereits einen Netzwechsel begonnen hat.
   `wait_application_route` brauchte am Offline-Ausgang den aktuellen
   Proto-Epochenvergleich. Der präzisierte Offline-Plan grenzt diese Folge
   ausdrücklich von unverändert terminalen Deadline-/Cancelled-/Closed-Enden ab.

Wiederverwendet werden die vorhandenen Policyrevisionen, die verbindungs- und
pfadgebundenen Tokens, `requires_new_generation`, der Treibermutex,
`WeakConnectionHandle`, die Invalidierung eines retained Pending-Transmits und
die bestehenden endlichen Pfadvalidierungen. Transportkontrolle, CID-/Pfadgrenzen,
Streamidentität, Retransmit-Queues und Datagrammsemantik bleiben bestehen.

Graphify wurde read-only benutzt. Die Vokabeln `network`, `wake`, `route`,
`epoch`, `notify`, `batches`, `adapter` sind im Rootgraph vorhanden. Der breite
1200-Token-Query war für diese Vendorgrenze unpräzise; die konkreten Graphknoten
nennen `NodeWake`/`.notify_network_change()` in `share/core/node_wake.rs`,
`RouteAdapter` in `share/os/shared/route_adapter.rs` sowie die F42-Fixture.
Die Vendorgrenze ist im aus `native/src` aufgebauten Graph nicht enthalten.
Die Interpreterdatei fehlt; das vorhandene Graphify-CLI funktionierte. Es wurde
keine Graphdatei erzeugt, aktualisiert oder als Erfolgsbeleg verwendet.

## C — Ansatz, Alternativen und erste Quellenrunde

Aus dem Bestand folgt eine Trennung zwischen sofortigem Freigabeentzug und
späterer Transporterholung. Ein bestätigtes Actor-Ereignis muss alle aktivierten
Policies unter deren vorhandenem Sendemutex sperren, bevor Iroh Transporte
neu bindet. Die Wartezeit auf eine Default-Route darf danach für die bestehende
Liveness-/CID-/Pfaderholung erhalten bleiben. Während dieser Wartephase darf
`BeginGeneration` noch keine auf dem alten Netz gewonnene Freigabe erzeugen.

Noq soll dafür ausschließlich schwache Referenzen auf aktivierte Verbindungen
halten. Die Endpoint-Sperre wird vor jeder Verbindungssperre freigegeben;
inaktive Policies behalten den vorhandenen ereignisbasierten Ablauf. Neue
aktivierte Verbindungen während der Wartephase müssen ebenfalls gesperrt
starten. Eine gespeicherte PTO-Referenz erhält die vorherige Validierungsbasis.

Verworfen sind ein künstliches Hochsetzen des Diagnosezählers, ein künstlicher
`RouteStatus`, ein Weglassen der F42-Abnahme und ein pauschales Invalidieren nach
jedem potenziellen Hinweis. Auch eine bloß früher eingestellte Queuenachricht
belegt keine Synchronisierung mit einem bereits sendenden Treiber. Dauerhafte
starke Connectionreferenzen würden die Verbindungslebensdauer ändern.

Iroh-`test-utils` ist in `native/Cargo.toml` nur als Linux-Devfeature aktiv.
Deshalb trägt es keinen gemeinsamen Windows-Fixtureeinstieg. Geplant ist ein
expliziter bestätigter Hostmeldungs-Einstieg in Iroh, der denselben tatsächlichen
Actor-/Rebind-/Noq-Pfad wie der Monitor benutzt. Die Fixture injiziert dort eine
bestätigte Meldung; sie verändert weder Policyzustand noch Zähler direkt. Dieser
Nachweis ersetzt keinen physischen Adapterwechsel auf einem Endgerät.

Primärquellen, abgerufen am 2026-10-26:

- [Gepinnte Iroh-Endpointquelle](https://raw.githubusercontent.com/n0-computer/iroh/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/endpoint.rs): `network_change` ist ein potenzieller Hinweis.
- [Gepinnte Iroh-Socketquelle](https://raw.githubusercontent.com/n0-computer/iroh/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/socket.rs): erkannte Änderungen, Rebind und bestehende Netzbereitschaftsphase.
- [Gepinnte Noq-Endpointquelle](https://raw.githubusercontent.com/n0-computer/noq/c1f411562e6078852749b8bcf1190523096a107f/noq/src/endpoint.rs): vorhandene ereignisbasierte Weitergabe und Socketlebensdauer.
- [RFC 9000 §8.2.3/§8.2.4](https://www.rfc-editor.org/rfc/rfc9000.html#section-8.2.3): Pfadvalidierung braucht die passende Antwort; die bestehende endliche PTO-Basis bleibt erhalten. Der zusätzliche Anwendungsgate ist Projektpolicy.
- [Rust `Weak`](https://doc.rust-lang.org/std/sync/struct.Weak.html): Schwache Referenzen halten den inneren Wert nicht allein am Leben.

Die Iroh-1.3.0-Docs.rs-Seite war über das Webwerkzeug nicht abrufbar. Stattdessen
sind die offizielle exakt gepinnte Quelle und die aktuelle Vendorquelle
maßgeblich. Es werden keine neuen Abhängigkeiten, Konten oder Dienste benötigt.

## Direkte und indirekte Verknüpfungskarte

| Grenze | Konkrete Stellen und Erhaltungsvertrag |
|---|---|
| Hosthinweis → Monitor | `node_wake.rs::notify_network_change` debounced den bestehenden potenziellen `iroh::Endpoint::network_change`-Aufruf. `EndpointInner::network_change` stellt `ActorMessage::NetworkChange` ein; der Actor fordert Netwatch zum Prüfen auf. Dieser Weg wird nicht pauschal zum bestätigten Ereignis erklärt. |
| Bestätigtes Ereignis → Actor | `Actor::run` erhält `local_interfaces_watcher.updated`, bestimmt `is_major` und ruft `handle_network_change`. Der neue explizite Hostmeldungs-Einstieg führt auf genau dieselbe Funktion. Ein Reply bestätigt dessen Bearbeitung, nicht einen physischen Adapterwechsel. |
| Actor → frühe Sperre → Rebind | `handle_network_change` sperrt aktivierte Policies synchron über Noq, bevor `transports_network_change.rebind`, Relayprüfung, DNS-Reset und Re-STUN beginnen. Kleine Änderungen durchlaufen dieselbe frühe Sperre ohne erzwungenen großen Rebind. |
| Netzbereitschaft → Erholung | `has_usable_network`, `PendingNetworkChangeNotify` und der vorhandene Backoff bleiben erhalten. `notify_quic_network_change` bildet den aktuellen Hint für IP/Relay/Custom und ruft weiterhin die vorhandene Noq-Netzwechselbehandlung sowie `remote_map.on_network_change` auf. |
| Noq-Endpoint → Connection-Lebensdauer | `ConnectionSet::insert` erfasst ausschließlich aktivierte Policies über einen internen `Connecting`-Weak-Handle. Ein Neuaufbau während der Wartephase startet ebenfalls gesperrt. Beim bestehenden Drained-Ereignis wird der Weak-Eintrag entfernt. Keine dauerhafte starke Referenz wird ergänzt. |
| Endpoint → Sendemutex | Die Endpoint-Sperre dient nur Flag und Weak-Snapshot und wird vor jedem `ConnectionInner::lock_and_wake` freigegeben. Vorbereitung und spätere Behandlung aktivierter Verbindungen benutzen denselben Mutex wie `ConnectionDriver::poll`/`State::drive_transmit`. Disabled-Verbindungen behalten die bisherige Queuenachricht. |
| Policy → Token und Pfadproof | Die frühe Vorbereitung entzieht `admitted`, erhöht Revision und Diagnosezähler und verlangt eine neue Generation. Ein interner gespeicherter PTO hält die Wartephase fest. Neue Generationen bleiben erfolgreich; Beobachtungen, `Allow`, Retirement und Datenzulassung bleiben während der Wartephase gesperrt. Bei Erholung erhöht sich die Revision erneut und die vorhandene frische Pfadprüfung beginnt, ohne denselben Netzwechsel erneut zu zählen. |
| Core → bestehende Erholung | `Connection::handle_network_change` behält Handshake-, Recoverable-/Nonrecoverable-, CID-, Ping-, Multipath- und Serverzweige; nur der Policy-Integrationsaufruf schließt eine eventuell vorbereitete Änderung ab. Passive lokale IP-Wechsel in `process_payload` behalten ihren vorhandenen Invalidierungsweg. |
| Policy → gehaltene I/O | `application_transmit_stamp` enthält die Revision. `State::drive_transmit` verwirft einen gehaltenen verschlüsselten Transmit bei geändertem Stamp vor erneutem I/O. Keine STREAM-/DATAGRAM-Queue und kein Streamoffset wird entfernt. Bereits vom Sender akzeptiertes I/O bleibt bereits unterwegs. |
| Noq → Iroh → Share | Die bestehenden Fassaden in beiden `connection.rs` reichen Snapshot/Update/Gate weiter. Der additive Snapshotwert `network_change_in_progress` kommt aus dem einzigen tatsächlichen Literal in `Connection::application_path_policy`; er ist beim Disabled-/Initialdefault `false`. Mains globale Suche bestätigt keine zusätzliche Konstruktorfläche. |
| Share-Wartephase → aktuelle Offlineentscheidung | `route_monitor::run` überspringt während der expliziten Vorbereitung jede RoutePolicy-Entscheidung. Das private Kindmodul `route_monitor_network_wait` veröffentlicht Paused und pflegt die Pausenuhr unter den vorhandenen Locks. Nach Bereitschaft erzwingt die neue Generation einen frischen Ursprung, Adapter und Kandidatensnapshot. Die nächste beobachtete Fremdgeneration ersetzt ebenfalls eine alte Policy. `wait_application_route` liest vor einem Offline-Ende zusätzlich den aktuellen Protozustand; Cancelled, Closed und die absolute Deadline bleiben terminal. |
| Share → Operationsgrenzen | `node_routes::{monitor_route,await_route,connect_exec_cancellable}` behalten Identität, aktuelle Autorisierung, Cancellation und absolute Deadline; `route_monitor::wait_application_route` verlangt weiter Statusfreigabe plus wirklichen Transportgate. `RoutePolicy`/`RouteStatus` bleiben unverändert. |
| Quelle → gemeinsame Abnahme | Die bestehende F42-Fixture behält sämtliche Payload-, Token-, Netzepoch-, Stream- und Gegenrichtungsassertions. Sie meldet jetzt ein bestätigtes Hostereignis. Ein zusätzlicher Disabled-Fall erhält die bestehende ungegatete Übertragung und `Disabled`-Fehler. Main registriert die Fälle im vorhandenen Manifest und verwendet denselben Remote-Einstieg. |

## D — Endgültiger Plan vor Quelländerung

Die zweite Lückenrecherche hat die internen Sperr- und Weak-Schnittstellen sowie
den tatsächlichen Share-Monitor geklärt. `route_monitor::run` beendet sich bei
jedem fehlgeschlagenen `BeginGeneration`. Deshalb wird diese Operation während
der Netzbereitschaft **nicht** neu abgewiesen. Die interne Wartephase sperrt
stattdessen Beobachtungsvalidität, `Allow`, Daten und Pfadretirement. Nach der
Erholung wird die frische Proofrunde auch dann benötigt, wenn die App ihre neue
Generation bereits während der Wartephase begonnen hat. Eine zusätzliche
Revisionsänderung entwertet sämtliche Zwischenbeobachtungen. Der zuvor
gespeicherte PTO wird nicht durch eine inzwischen verlorene Grantreferenz
verkürzt. Die unten präzisierte explizite Wartebehandlung im Share-Monitor
erhält außerdem das nach Bereitschaft vollständig neue Entscheidungsfenster.

Die Prüfung von Noq-Sendeloop und Endpointsperre bestätigt die notwendige
Lockreihenfolge: Endpointzustand freigeben, eine Connection upgraden, deren
Sendemutex nehmen, Zustand ändern, Mutex freigeben, temporären starken Handle
fallen lassen. Es wird kein Connectionmutex unter dem Endpointmutex erworben.
Neue Policyverbindungen übernehmen das Warteflag bereits vor Treiberstart.
Die Iroh-Actorfolge serialisiert Vorbereitung und Erholung derselben Änderung.

| Meilenstein | Dateien | Abhängigkeit | Konkretes erwartetes Ergebnis |
|---|---|---|---|
| M13.1 Policyphase | Noq-proto `application_path_policy.rs`, `application_path_revalidation.rs`, ausschließlich `Connection::handle_network_change` in `connection/mod.rs` | Gesicherte Gate-/PTO-/Monitorverträge | Sofortiger Entzug mit einmaligem Netzwechselzähler; kein Daten-Allow während Vorbereitung; erfolgreiche neue Generationen bleiben möglich; Erholung entwertet Zwischenbeobachtungen und startet endliche frische Validierung mit erhaltener PTO-Basis. |
| M13.2 Synchrone Noq-Grenze | Noq `endpoint.rs` und interner Weak-Getter in `connection.rs` | M13.1 | Aktivierte Verbindungen werden unter ihrem Sendemutex vor Iroh-Rebind gesperrt; Neuverbindungen während der Wartephase ebenfalls. Weak-Besitz und Drained-Aufräumen erhalten Lebensdauer; Disabled behält den vorhandenen Eventpfad. |
| M13.3 Bestätigte Iroh-Meldung | Iroh `endpoint.rs`, `socket.rs` | M13.2 | Monitor und bestätigter Hostmeldungs-Einstieg nutzen denselben Actor/Rebind/Noq-Pfad. Die frühe Sperre liegt vor dem Rebind. Potenzieller Hinweis, Relay-/Custom-/IP-Hints, Wartebackoff, Shutdown und private Upstream-Fixtures bleiben erhalten. |
| M13.4 Share-Wartephase | `route_monitor.rs`, privates Kindmodul `route_monitor_network_wait.rs` und betroffene Fixture | M13.1–M13.3 | Während Vorbereitung Paused ohne verbrauchtes Entscheidungsfenster; nach Bereitschaft vollständige neue Generation. Veraltetes Offline beendet keine aktuelle Netzepochwartezeit. Deadline, Cancelled und Closed bleiben terminal; F42 bleibt unverkleinert erhalten. |
| M13.5 Autorreview und Vertrag | Vendor-Policyvertrag und dieser Bericht | Alle Quelländerungen | Derselbe Autor prüft Snapshot/Allow/Revision, Sendemutex, Weak-Drop-Reihenfolge, vorhandene Aufrufer, Disabled, Ressourcenabschluss und alle unveränderten F42-Orakel. Main erhält exakte Fallnamen und verbleibende Laufzeitfragen. |

Vor Beginn der Quelländerung festgelegte Reviewfragen: Kann ein alter oder
während der Wartephase beobachteter Token wieder Daten erlauben? Kann ein neuer
BeginGeneration-Erfolg den Wartegate öffnen? Bleiben alte Pending-I/O-Pläne
verworfen, Providerberechtigungen unverändert, Server-/Handshakefälle, IPv4/IPv6,
Relay/Custom, Cancellation und Streamlebensdauer erhalten? Gibt es einen
Sperrzyklus oder dauerhaften starken Connectionbesitz? Stimmen Quelländerungen
und Nachweisumfang mit dem Scope überein? Das Ergebnis wird erst nach tatsächlicher
Quelländerung dokumentiert. Bis zu diesem endgültigen Planstand wurde keine
Produkt- oder Fixturequelle geändert.

### Präzisierter Wartephasenplan vor der Nachkorrektur

Die Integrationslektüre hat nach der ersten Umsetzung eine weitere konkrete
Folge offengelegt: `route_monitor::run` setzt bei der früh verlangten Generation
seinen `origin` zurück, während `RoutePolicy::observe` das Sammelfenster nach
dieser Uhr einfriert. Ohne erneute Generation bei Netzbereitschaft würde die
spätere Auswahl ein bereits vorher verbrauchtes Fenster übernehmen. Bei kleiner
PTO-Basis könnte die Policy während der Bereitschaft sogar `Offline` melden;
`wait_application_route` würde dann vorzeitig enden. Das ist ein belegter
Quellpfad, keine hier nachgestellte Hardwarebeobachtung.

Der davon betroffene Teilplan wird vor der Nachkorrektur ersetzt: Ein additiver
Snapshotwert `network_change_in_progress` kennzeichnet die tatsächliche
Vorbereitungsphase. Der Monitor hält diese Phase explizit als `Paused`, führt
dabei keine Sammel-/Direktentscheidung aus und bewahrt Pausenuhr, Cancellation
und absolute Operationsdeadline. `BeginGeneration` bleibt erfolgreich, startet
während der Wartephase aber keine neue Proofrunde. Completion setzt
`requires_new_generation` erneut; die nächste Monitoriteration erzeugt erst
nach Bereitschaft die maßgebliche frische Generation mit neuem `origin`, neuer
Policy und neuem Adapter. Eine vor der Bereitschaft eingefrorene Policy wird
damit vollständig verworfen. Die rohe F42-Fixture muss denselben notwendigen
Generationserneuerungsschritt durchführen, ohne den öffentlichen `allow`-Helfer
zu verändern.

Die reine Timerlektüre bestätigt: Ein bestehender Validierungstimeout kann
`prev` zurückholen; er entfernt den Pfad nicht pauschal. Diese unveränderte
Timeoutbehandlung ersetzt weder die Bereitschaftsmeldung noch ein frisches
Anwendungsentscheidungsfenster. Die konkret benötigte zusätzliche Schreibgrenze
für den Monitor ist inzwischen von Main für `run` und unmittelbar zugehörige
Status-/Pausenuhrbehandlung bestätigt. Die Statusveröffentlichung wird unter
der bestehenden Watch-Schreibsperre bedingt geändert, damit ein gleichzeitig
gesetztes `Cancelled` nicht durch eine ältere Pausen- oder Policykopie
überschrieben wird. Derselbe Commitpunkt bewahrt die neueste
`remote_guarantee`. Unveränderte Zustände lösen keine zusätzlichen Watch-Wakes
aus. Der [offizielle Tokio-1.52.3-Vertrag von `send_if_modified`](https://raw.githubusercontent.com/tokio-rs/tokio/tokio-1.52.3/tokio/src/sync/watch.rs)
ist dafür gelesen; die große Docs.rs-Seite war nicht abrufbar.

### Präzisierter Plan für eine veraltete Offline-Publikation

Die weitere eigene Schlussprüfung bestätigt folgende mögliche Reihenfolge:
Der Monitor liest einen alten Proto-Snapshot; anschließend sperrt der Actor
die neue Netzeventphase; der Monitor berechnet aus seiner alten Kopie Offline.
Ein Recheck vor der Publikation verkleinert dieses Fenster, macht Publikation
und Protozustand aber nicht zu einer gemeinsamen Transaktion. Entscheidend
ist deshalb ein zusätzlicher frischer Proto-Snapshot an der Offline-Rückgabe
von `wait_application_route`: Eine laufende Netzvorbereitung, eine erforderliche
neue Generation oder eine bereits abweichende Generation macht den alten
Offline-Status als Operationsende ungeeignet. Nach diesem aktuellen
Epochencheck später eintretende Netzereignisse bilden einen späteren Zustand;
eine schon zuvor gültig beendete Operation wird nicht wieder geöffnet.
`Cancelled`, `close_reason` und die absolute Deadline bleiben unmittelbar
terminal. Keine Wiederholung einer FS-/Exec-/Screenoperation wird ergänzt.

Der endgültig ergänzte Arbeitsplan extrahiert die neu hinzugefügte
Wartephasenpublikation/Pausenuhr und diese gemeinsame Offline-Entscheidung
kohärent in das private Kindmodul `route_monitor_network_wait.rs`. Main hat
diesen exakten Lese-/Schreib-/Anlegepfad und `wait_application_route` bestätigt.
Der große Monitor bleibt dadurch unter der nativen Dateigrenze. Mains
Gesamtsuche über `native/` und `vendor/` bestätigt zudem genau das bereits
bearbeitete tatsächliche Snapshot-Literal; die Fassadentreffer sind nur
Rückgabetypen. Es fehlt keine zusätzliche Konstruktorfläche.

## Umsetzung, eigene Anti-Regressionsreview und Dateiverzeichnis

M13.1–M13.4 sind gemäß dem vor der Quelländerung gespeicherten Plan umgesetzt.
Die zusätzliche Policyphase speichert nur `Option<Duration>`; der Default ist
`None`. Der additive Snapshotwert stammt aus dieser Phase und ist initial
sowie bei Disabled `false`. `BeginGeneration` behält seine bisherige
Erfolgssemantik, startet vor Bereitschaft aber keine neue Proofrunde. Vorbereitung
und Abschluss verwenden die vorhandene Revisions-/Challengegrenze; die private
Zählersichtbarkeit und die RNG-Konstruktion bleiben unverändert.

Noq registriert ausschließlich aktivierte Policies über schwache Referenzen.
Die frühe Vorbereitung und die spätere Erholung arbeiten unter dem vorhandenen
Connection-Sendemutex. Die normale Endpoint-Sperre ist davor jeweils bereits
freigegeben. Für deaktivierte Policies werden unverändert
`LocalAddressChanged`-Ereignisse eingestellt. Neu angelegte aktivierte
Verbindungen übernehmen eine noch ausstehende Vorbereitungsphase vor dem
Treiberstart. Der bestehende Drained-Zweig räumt die Weak-Registrierung auf.

Iroh bietet `network_change_detected(is_major)` für bestätigte lokale Meldungen.
Der bestehende potenzielle Hinweis ist unverändert. Der neue Einstieg wartet
auf eine Actorantwort nach dessen Verarbeitung und verwendet denselben
`handle_network_change`-Pfad wie Netwatch. Die private Upstream-Funktion
`force_network_change` delegiert ebenfalls dorthin. Der Actor entzieht zuerst
die Policyfreigaben und führt danach den bestehenden Rebind-/Recoverypfad aus.
Die neue Bestätigung ist keine Behauptung einer gelungenen Socketneubindung:
deren vorhandene Fehlerbehandlung bleibt erhalten.

Der Monitor hält die explizite Vorbereitung jetzt als Paused, ohne
`RoutePolicy::observe` aufzurufen. Completion verlangt erneut eine Generation;
damit setzt der bestehende Generationseinstieg Ursprung, Policy und Adapter
nach Bereitschaft vollständig zurück. Ein zwischenzeitlich extern begonnener
neuer Proto-Generationswert wird ebenfalls übernommen, statt eine ältere
Policy dauerhaft weiterzuverwenden. Ein Recheck vermindert veraltete
Offline-Publikationen. Vor einer fatalen Offline-Rückgabe prüft der tatsächliche
Wartepfad zusätzlich einen frischen Proto-Snapshot. Die neuen Statusupdates
bewahren unter der Watch-Schreibsperre gleichzeitiges Cancelled und die neueste
Remote-Garantie; unveränderte Statuswerte erzeugen keine zusätzlichen Wakes.

Die F42-Fixture behält den ursprünglichen Namen und sämtliche bisherigen
Nutzdaten-, Stream-, Token-, Epoch- und Gegenrichtungsassertions. Sie prüft
zusätzlich den folgenlosen potenziellen Hinweis im unveränderten Loopbacknetz
und verwendet für den positiven Wechsel ein injiziertes bestätigtes
Hostereignis mit begrenzter Actorwartezeit. Eine parametrisierte private
Paarfixture erhält `pair()` unverändert als aktivierten Einstieg. Der neue
Disabled-Fall überträgt auf demselben Stream vor und nach dem Ereignis und
prüft weiterhin Datagramme, ungegatete Zulassung, null neue Zähler sowie
`Disabled` bei einem expliziten Policyupdate.

Ein zusätzlicher gezielter Wartefall hält eine frühere, von `RoutePolicy`
tatsächlich berechnete Offline-Entscheidung als bewusst verzögerten Watchstand
fest. Die aktuelle Noq-Epoche entsteht durch den echten bestätigten Iroh-
Ereigniseinstieg. Dieser Fall verlangt Weiterwarten sowohl bei erforderlicher
Generation als auch nach realem `BeginGeneration` mit abweichender Generation;
eine abgelaufene absolute Deadline, Cancelled und Noq-Schließung bleiben
nachweisbare Endzustände. Die historische Offline-Entscheidung ist eine
kontrollierte Eingabe für den Observer-Racefall, kein künstlicher Erfolg eines
Transportgates. Zähler und Grants werden dafür nicht direkt verändert.

Die abschließende eigene Nachreview umfasst den finalen Quellenstand nach
beiden präzisierten Planergänzungen. Der Begin-/Completion-Übergang ist im
Noq-Sendemutex atomar gegenüber Snapshotlesern: Nach dem Phasenabschluss ist
die neue Generation erneut erforderlich. Der Monitor überspringt vorher
`RoutePolicy::observe` vollständig und ersetzt anschließend Policy und Adapter.
Die Offline-Prüfung bewertet den aktuellen Epochenstand erst am möglichen
Fehlerausgang; sie verändert weder die absolute Deadline noch Cancelled/Closed.
Später eintretende Ereignisse machen eine zuvor passend abgeschlossene
Operation nicht nachträglich wieder offen. Die Watch-Schreibsperre schützt
die sticky Cancellation ohne eine darunter genommene Noq-Sperre; alle Borrows
enden vor dem nächsten Await. Die nachfolgende Schlusslektüre von Main meldete
keinen zusätzlichen Integrationsbefund. Das ersetzt den ausstehenden
Remote-Nachweis ausdrücklich nicht.

### Eigene nachfolgende Anti-Regressionsreview

Diese Prüfung wurde vom selben Autor nach sämtlichen Quelländerungen anhand des
Diffs und der konkreten Aufrufer durchgeführt. Sie ist keine Remoteabnahme.

| Prüffrage | Statischer Beleg und Ergebnis |
|---|---|
| Entzug vor neuem Iroh-I/O | `Actor::handle_network_change` ruft vor `rebind` den synchronen Noq-Haken auf. Er wartet auf jeden betroffenen Sendemutex. Ein bereits laufender Sendepoll endet vor der Änderung; nach der Sperre ist der Gate geschlossen. Bereits akzeptiertes I/O bleibt unterwegs. |
| Alte und Zwischen-Tokens | Die Vorbereitung entzieht `admitted` und verändert Revision. Die Wartephase blockiert `validated`, `Allow`, Daten und Retirement unabhängig von `requires_new_generation`. Der Abschluss verändert Revision nochmals und startet frische Challenges. Alte Tokens und während der Wartephase erzeugte Beobachtungen können keine spätere Freigabe erben. |
| Monitor und neue Generation | BeginGeneration bleibt erfolgreich, startet während Vorbereitung aber keinen Proof. Der explizite Wartezweig führt weder Sammeln noch Direkt-/Relayauswahl aus. Completion setzt atomar mit dem Phasenabschluss erneut `requires_new_generation`; erst danach werden Ursprung, Policy und Adapter neu angelegt. Ein vor Bereitschaft verbrauchtes Fenster wird nicht übernommen. Der globale absolute Operationsdeadlinewert wird nicht geändert. |
| Veraltetes Offline und terminale Enden | Die Quellen lassen Snapshot → Actoränderung → alte Offlinepublikation zu. run verwirft bereits erkannte alte Offlineentscheidungen; der zusätzliche frische Epochvergleich vor der Offline-Rückgabe verhindert den falschen Operationsabbruch. Nur ein dabei noch passender Offlinezustand bleibt terminal. Cancelled und tatsächliches Closed umgehen diese Ausnahme; Deadlineprüfung und Deadline-Select sind unverändert. Alle Watch-Borrows und Noq-Locks enden vor Await. |
| Frische, endliche Validierung | Der Abschluss benutzt den gespeicherten bisherigen PTO plus die aktuelle Referenz. Die bestehende Drei-PTO-/Previous-Path-Berechnung und Challenge-Resetfunktion bleiben erhalten. `paths.rs::on_path_response_received` erkennt weiterhin ausschließlich vorhandene passende Challenge-Tokens. Der Passive-IP-Zweig in `process_payload` bleibt unverändert. |
| Pending-I/O, Streams und Datagramme | Noq vergleicht vor Retry weiterhin denselben revidierten `ApplicationTransmitStamp`. Der Fix löscht keine Stream-/Datagrammqueue und verändert weder Streamoffsets noch Retransmitbuchhaltung. Die F42-Fixture verlangt weiterhin die Fortsetzung desselben offenen Streams und getrennte Senderfreigaben. |
| Disabled und Legacy | Der Default bleibt `false`. Disabled-Verbindungen werden nicht in die Weak-Policymap aufgenommen, verwenden den alten Queuepfad und verändern keine neuen Policyzähler. Receiververhalten und Altpeer-Einschränkungen bleiben erhalten. Der neue Disabled-Fall erhält deren explizites Laufzeitorakel. |
| Lebensdauer und Sperrfolge | Gespeichert werden nur `WeakConnectionHandle`s. Die temporäre starke Connection lebt ausschließlich während eines synchronen Aufrufs; der WakeGuard fällt vor ihr. Kein Connectionmutex wird unter dem Endpointmutex genommen. Drained entfernt den Weak-Eintrag. Der Rebind hält keinen Connectionmutex über ein Await hinweg. |
| Neue Verbindung während Vorbereitung | `ConnectionSet::insert` übernimmt die Phase auf dem noch lokalen Protoobjekt vor `Connecting::new`. Die normale Handshake-/Close-/Cancelfolge und die Verbindungszähler bleiben erhalten; kein neuer Grant wird erzeugt. |
| Plattform-/Transportbestand | Der neue Einstieg ist nicht von Linux-`test-utils` abhängig. Bestehende IP-, IPv4-/IPv6-, Relay- und Custom-Hintzweige, Server-/Handshakeerholung, CID-Grenzen, Congestion-/Anti-Amplification-Regeln, DNS-/Re-STUN-/Relayhandling und Netzbereitschaftsbackoff bleiben unverändert. Es gibt keine Änderung von Peerrechten, Identität, gespeicherten Endpunkten oder Nutzdaten-Drahtformaten. |
| Kapselung, Grenzen und Scope | Die drei privaten Zähler bleiben privat; der kapselnde Konstruktor bleibt erhalten. Keine RNG-Ziehung wird verschoben. Das einzige Snapshot-Literal setzt den additiven Wert für jeden Konfigurationspfad. Neue Arbeit bleibt in den erlaubten Vendorhaken, Monitor/Wartekindmodul, Fixture und beiden Dokumenten. Alle drei geänderten beziehungsweise neuen nativen Dateien bleiben unter 500 Zeilen und 50 KiB; es wurde keine große Vendor-Aufteilung begonnen. |

Der eigene `git diff --check` für sämtliche geänderten bestehenden Dateien
lieferte keinen Befund. Die finale native Dateigrenzenprüfung ergibt 494 Zeilen
für den Monitor, 152 für sein neues privates Kindmodul und 444 für die Fixture;
jede dieser Dateien bleibt unter 50 KiB. Eine zusätzliche optionale Tree-sitter-Lektüre war im
System-Python wegen des fehlenden Moduls `tree_sitter` nicht verfügbar. Es wurde
dafür nichts installiert und kein Parser-, Compiler- oder Funktionspass
behauptet. Die Quellprüfung von Sichtbarkeit, Typanschlüssen, Branches,
Lockreihenfolge und Ressourcenbesitz erfolgte direkt am Source/Diff.

Main meldet separat für sämtliche geänderten nativen Dateien eine statische
Tree-sitter-Prüfung ohne Syntaxfehler. Bei Iroh `socket.rs` ist der
Grammatikbefund für `#[cfg(not(wasm_browser))]` im Options-Destructuring bereits
unverändert an HEAD vorhanden: Zeile 878 dort, Zeile 879 im Arbeitsstand, mit
identischem ERROR-Text. Dieser gemeldete Vorher-/Nachhervergleich belegt keinen
neuen Parserbefund aus dem Diff, aber auch keinen vollständigen Vendor-Parser-
oder Compilerpass. Der unveränderte Upstreamcode wird deshalb nicht angepasst.

Im eigenen Review wurden die neu geschriebenen Fixtureketten auf die bestehende
mehrzeilige Form gebracht und die beiden neuen Actorwarteaufrufe explizit mit
dem vorhandenen `WAIT` begrenzt. Kein Prüffall wurde entfernt, kein bestehendes
Orakel abgeschwächt und kein direkter Policy-/Zähler-Erfolg injiziert.

### Übergabe und noch unbestätigte Laufzeitfragen

Für denselben Remote-Einstieg bleiben verbindlich:

- `share::route_policy::batches_route_task_tests::batches_task_b5_actual_stream_and_datagram_gate_survives_network_epoch` — bestehender F42-Fall mit vollständiger bisheriger Abnahme und bestätigtem Actor-Ereignis.
- `share::route_policy::batches_route_task_tests::batches_task_b5_confirmed_network_change_preserves_disabled_transport` — neuer Disabled-Erhaltungsfall; Main ergänzt seine erforderliche explizite Manifestbindung.
- `share::route_monitor::network_wait::tests::batches_task_b5_stale_offline_keeps_network_wait_deadline_cancel_and_close` — neuer Fall für eine echte Noq-Netzepochänderung bei altem Watch-Offline, abweichende Generation und unverändert terminale Deadline/Cancelled/Closed; Main ergänzt die Manifestbindung.
- Der vorhandene Abbruchfall `batches_task_b5_route_cancellation_closes_blocked_transport_without_payload` bleibt unverändert in derselben B5-Auswahl.

Das vorhandene Manifest wählt `batches_` und verlangt `batches_task_b5_`; P28
bleibt daran gebunden. Main besitzt Manifeständerung, Integration, Commit, Push
und denselben Remote-Suitelauf. Weil die native Fixture geändert wurde, besitzt
Main außerdem für Monitor, Kindmodul und Fixture die vorgeschriebene Aktualisierung
des Rootgraphs; dieser Autor verändert keine Graphdatei.

Offen sind der Compiler- und tatsächliche Laufzeitnachweis auf Linux und Windows
sowie die Android-Kompilation des geänderten Vendorpfads. Insbesondere ist die
zusätzliche Default-Route-Wartephase im Quellvertrag geprüft, aber hier nicht
gezielt mit einer physisch fehlenden Route ausgeführt worden. Der positive
Fixtureeinstieg injiziert eine bestätigte lokale Meldung und durchläuft reale
Actor-/Rebind-/Noq-Logik; er verändert keine physische Netzschnittstelle.
Echte Hardwareumschaltungen bleiben damit unbeobachtet. Diese fehlende
Beobachtung ist kein behaupteter Pass und kein neuer Releasehaltepunkt.
Keine Scopeabhängigkeit oder Implementierungsfrage dieses abgegrenzten Fixes
ist offen. Dieser Autor hat keine lokale Suite, keinen Compiler oder Formatter
und keinen Build-, Installations-, Server- oder Releaseprozess ausgeführt.

### Vollständiges Dateiverzeichnis

Gelesene Projektdateien; bei großen Dateien wurden die im Mapping benannten
Abschnitte einschließlich der benötigten Aufrufkontexte gelesen:

- `AGENTS.md` — bestehende Sitzungsfassung mit unverändertem Diff seit dem vorherigen Fix geprüft.
- `docs/ARCHITEKTUR.md`
- `docs/plaene/2026-10-24-batches/fix-path-policy-construction.md`
- `docs/plaene/2026-10-24-batches/fix-f-route-epoch.md`
- `docs/plaene/2026-10-24-batches/rv1-funktionsbefunde.json`
- `docs/plaene/2026-10-24-batches/rv1-funktionsplan.md`
- `graphify-out/graph.json`
- `native/Cargo.toml`
- `native/Cargo.lock`
- `native/review-task-manifest.json`
- `native/src/share/core/batches_route_task_tests.rs`
- `native/src/share/core/node_wake.rs`
- `native/src/share/core/node_routes.rs`
- `native/src/share/core/route_policy.rs`
- `native/src/share/core/route_state.rs`
- `native/src/share/os/shared/route_adapter.rs`
- `native/src/share/os/shared/route_monitor.rs`
- `native/src/share/os/shared/route_monitor_network_wait.rs`
- `vendor/SMART_EXPLORER_ROUTE_POLICY.md`
- `vendor/iroh-1.3.0/src/endpoint.rs`
- `vendor/iroh-1.3.0/src/endpoint/connection.rs`
- `vendor/iroh-1.3.0/src/endpoint/quic.rs`
- `vendor/iroh-1.3.0/src/socket.rs`
- `vendor/noq-1.3.0/src/connection.rs`
- `vendor/noq-1.3.0/src/endpoint.rs`
- `vendor/noq-proto-1.3.0/src/config/transport.rs`
- `vendor/noq-proto-1.3.0/src/connection/application_path_policy.rs`
- `vendor/noq-proto-1.3.0/src/connection/application_path_revalidation.rs`
- `vendor/noq-proto-1.3.0/src/connection/mod.rs`
- `vendor/noq-proto-1.3.0/src/connection/paths.rs`
- `vendor/noq-proto-1.3.0/src/connection/timer.rs`
- `vendor/noq-proto-1.3.0/src/lib.rs`

Zusätzlich gelesen beziehungsweise aus der laufenden Sitzung übernommen wurden:

- `/root/.codex/AGENTS.md`
- `/root/.codex/skills/arbeitsweise/SKILL.md`
- `/root/.agents/skills/arbeitsweise/SKILL.md`
- Die gelieferten MCP-Anweisungen und ihre ausgelagerten Dateien:
  `/root/.local/share/creativehub/arbeitsweise-codex/spill/323a3d474d6c11349d7a30380c37cf12f12e74e609ca96bd4c8bc983a6262b07.md`,
  `/root/.local/share/creativehub/arbeitsweise-codex/spill/53be4080b90085bbf00a6c538c0492ebe3861f8dbfd60119c3d776db27630856.md`,
  `/root/.local/share/creativehub/arbeitsweise-codex/spill/a2e98f923baf3dd4d0b2c2f7da00882470658876e31b3cde51126808911d7730.md`,
  `/root/.local/share/creativehub/arbeitsweise-codex/spill/a55fa39811fa40902cc2088998ff716f0afcacb860d3dda2c3788f06e9df6d32.md`,
  `/root/.local/share/creativehub/arbeitsweise-codex/spill/0514d4096b3057f8f5daa75b0becd061028fc238c2a9d8b4a4dceb7d2777508a.md`
- `/root/.codex/skills/graphify/SKILL.md`
- `/root/.codex/skills/graphify/references/query.md`

Gelieferte Evidenzdateien:
`smartexplorer-f-route_epoch-scope.json`,
`smartexplorer-final-review-user-contract.txt`,
`smartexplorer-rv1-40406246032/native-failures.json` sowie die oben verlinkten
Primärquellen. Der Abruf von `graphify-out/.graphify_python` scheiterte, weil
diese Datei fehlt. Der funktionierende Graphify-Query meldete eine Abweichung
zwischen Skill 0.9.11 und installiertem CLI 0.9.17; es wurde kein Update und
keine Graphmutation ausgeführt.

Geänderte Dateien, vollständig:

- `vendor/iroh-1.3.0/src/endpoint.rs`
- `vendor/iroh-1.3.0/src/socket.rs`
- `vendor/noq-1.3.0/src/connection.rs`
- `vendor/noq-1.3.0/src/endpoint.rs`
- `vendor/noq-proto-1.3.0/src/connection/application_path_policy.rs`
- `vendor/noq-proto-1.3.0/src/connection/application_path_revalidation.rs`
- `vendor/noq-proto-1.3.0/src/connection/mod.rs` — ausschließlich der Integrationsaufruf in `handle_network_change`.
- `native/src/share/core/batches_route_task_tests.rs`
- `native/src/share/os/shared/route_monitor.rs`
- `vendor/SMART_EXPLORER_ROUTE_POLICY.md`

Erzeugte Dateien, vollständig:

- `native/src/share/os/shared/route_monitor_network_wait.rs`
- `docs/plaene/2026-10-24-batches/fix-f-route-epoch.md`

Keine Commits, Pushes,
Versions-, Cargo-, Workflow-, Manifest- oder Releaseartefaktänderungen durch
diesen Autor. Der lokale Quell- und Berichtsauftrag ist zur Integration bereit;
die ausdrücklich benannten Remote-Nachweise folgen ausschließlich bei Main.
