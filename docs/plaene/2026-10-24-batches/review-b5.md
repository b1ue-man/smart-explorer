# B5 — Statischer Quellenreview

## Abschließender IPC-Nachreview — R01 statisch geschlossen

Stand: 2026-10-25, `7442afc0` plus integrierte Arbeitskopie. **R01 bis R06 sind statisch geschlossen.** Die abschließende IPC-Prüfung hat keinen weiteren belegten Defekt ergeben. R02–R06 wurden in diesem letzten Lauf nicht neu vollständig geprüft; die nachfolgend dokumentierte erste Nachprüfung bleibt ihre Grundlage. Eine Laufzeit-, Compiler- oder Releasefreigabe ist damit nicht verbunden.

A — Auftrag vom 2026-10-25: Die integrierte R01-IPC-Korrektur auf unabhängigen Cancel-/Besitzer-Drop, Bindung an Daemonauthentifizierung, ExecId und frische Capability sowie Handoff, Fehler, Timeout, Terminalabschluss und Legacy-IPC prüfen. R02–R06 nur bei einer direkt belegten Regression erneut öffnen. Ausschließlich diesen Bericht ändern; keine Ausführung von Produktfunktionen, Tests oder Compilern.

B — Nachgelesener Bestand: `ExecIpcInput` besitzt einen gesonderten Kontrollsocket. `exec_lifetime.rs` registriert eine frische 32-Byte-Capability und wartet vor dem Setup-Handoff auf deren einmalige Bindung. `ipc.rs` authentifiziert beide neuen Requests vor ihrer Dispatchverzweigung; Listener und Admission erlauben den zweiten Socket in einem unabhängigen Thread. Die bisherigen Stdin-/EOF-/Resize-Frames und der Legacy-Request `ExecStream` bleiben vorhanden.

C — Entscheidung: Die bereits belegte innere Watch-Cancellation und die geschlossenen R02–R06 wiederverwenden. Die neue IPC-Kette vom öffentlichen Cancel/Drop über Kontrollsocket, Besitzerregistrierung und Watch bis zum Pausenabbruch verfolgen. Socket-/Condvar-Verträge aus Rust-Primärquellen, die Zufallsquelle am Lockfile-Pin und die unveränderte Setup-Probe an den OS-Primärquellen abgleichen. Keine Schließung allein aus der Existenz eines zweiten Sockets ableiten.

D — Erreichtes Abschlusskriterium: Der bestehende öffentliche Cancel-/Drop-Weg erreicht den inneren Watch unabhängig von den vollen Stdinqueues; falsche, veraltete und doppelte Kontrollbindungen erhalten keinen Besitzerzugriff. Setup- und Terminalpfade räumen die konkrete Besitzerregistrierung auf. Die folgenden Kontrollflussbelege schließen den zuletzt offenen R01-Rest.

### Kontrollflussbelege der R01-Schließung

- **Abbruch bei Rückstau:** `exec_ipc.rs:122` ruft unmittelbar `shutdown(Both)` auf dem gesonderten `Arc<TcpStream>` auf. Die Stdinmutex bei `:136` wird dabei nicht erworben. `exec_lifetime.rs:133` wartet in einem eigenen IPC-Thread ausschließlich auf diesen Kontrollsocket; EOF, Reset oder ein unerwartetes Byte führen zu `owner.cancel()`. Dieser signalisiert nach Freigabe seiner Zustandsmutex den vorhandenen `ShareExecInput::send(Cancel)`-Watch. Die normale MPSC-Reservierung in `exec_client_input.rs` beobachtet diesen Watch und beendet damit auch das in `serve_input` blockierte Stdin-Senden. Der weiter gepollte Cancelwatcher in `exec_client.rs::run_connected` schließt eine pausierte Verbindung, ohne auf einen Platz in `operation_rx` zu warten. Auf einer zugelassenen Route bleibt der vorhandene authentifizierte Cancel-/Terminalaustausch bestehen.
- **Besitzer-Drop trotz Eingabeklones:** `ExecIpcSession::drop` bei `exec_ipc.rs:90` signalisiert den Kontrollsocket ausdrücklich und beendet anschließend über den unabhängigen Ereignishandle den Datensocket. Eingabeklones eines blockierten Produzenten verhindern beides nicht. Ein bereits vollständig dekodiertes Terminal-/Failed-Ereignis setzt `finished`; der Drop führt dann keinen ausdrücklichen Cancel oder Shutdown des Datensockets aus. Das gewöhnliche Schließen des letzten Kontrollhandles bleibt vom serverseitigen `finished`-Zustand abhängig. `eof()` bleibt geordnete Stdinbeendigung und wird nicht zu Besitzerabbruch umgedeutet.
- **Authentifizierung und Sitzungsbindung:** Die Admission prüft den Loopbackpeer und Daemontoken; `ipc.rs::handle_client` prüft die vollständige Anfrage vor dem Dispatch und bei `RuntimeCall` zusätzlich die innere Anfrage. Beide neuen Varianten gehören ausdrücklich zu `daemon_token()`. `ExecLifetime::new` bei `exec_lifetime.rs:35` erzeugt je Besitzer 32 frische Zufallsbytes; ein Zufallsfehler bricht ab. `ExecState::owner` verlangt gleichzeitig Capability und ExecId. `claim` bei `:78` akzeptiert unter Mutex genau eine Kontrollbindung und verweigert bereits beanspruchte, abgebrochene oder beendete Besitzer. Falsche ExecId, fremde Capability und doppeltes Claim scheitern vor `owner.cancel()`. `remove_owner` prüft zusätzlich `Arc::ptr_eq`, bevor es den registrierten Eintrag entfernt.
- **Handoff, Fehler und Frist:** `serve_controlled` bei `exec_lifetime.rs:113` registriert den Besitzer vor Ausgabe von `ExecControlledReady`. Der Setupwächter bleibt bis `wait_ready` und dem anschließenden erfolgreichen `handoff` aktiv. Der Kontrollserver schreibt seine Bestätigung, beseitigt seine temporären Socketfristen und setzt erst dann `ready`. Bereits eingetroffener Abbruch wird unter derselben Zustandsmutex vor Ready geprüft; ein späterer Abbruch erreicht den unabhängigen Watch. `wait_ready` verwendet die bestehende Zehnsekundenfrist als absolute Deadline, prüft Zustandsprädikate nach jedem Condvar-Aufwachen neu und beobachtet weiterhin den Setupabbruch. Fehler vor Registrierung lassen die Remote-Session und den Setupwächter fallen; Fehler nach Registrierung führen zusätzlich durch `Registration::drop`. Kontrollverbindungs-, Antwort- und Klonfehler auf der Clientseite lassen die bereits erworbenen Sockets fallen. Die bestehenden Start-/Handshakeprüfungen werden dabei nicht übersprungen.
- **Terminalabschluss und Lebensdauer:** Nach `serve` ruft `Registration::drop` zuerst `finish` auf: Der Besitzer wird beendet, sein serverseitiger Kontrollhandle wird heruntergefahren und anschließend sein Registryeintrag entfernt. Ein dadurch geweckter Kontrollreader löst wegen `finished` keinen neuen Cancel aus. Im bestehenden `serve` beendet Transportfehler die Remote-Session; Terminal/Failed wartet weiterhin auf deren `finish`, schließt den Datensocket und wartet auf den Eingabethread. Der Retainer liegt unverändert im Execworker bis zu dessen tatsächlichem Ende; kein neuer `abort`-Pfad wurde eingeführt.
- **Legacy- und Nachrichtenvertrag:** `ExecStream` erhält weiterhin `ExecReady` und benutzt dieselben Eingabe-/Ereignisframes. Stdin, StdinEof, Cancel und Resize behalten ihre Tags und Größenbegrenzungen; die zwei angepassten `ExecIpcInput`-Testliterale versorgen lediglich das zusätzliche Kontrollfeld. Der neue Client verlangt ausdrücklich `ExecStreamControlled`/`ExecControlledReady` und die Kontrollbestätigung. Ein alter Worker führt zu einem Inkompatibilitäts-/Verbindungsfehler; es gibt keinen stillen Rückfall auf den queueabhängigen Legacyabbruch. Die stärkere Cancelgarantie gilt daher für den neuen kontrollierten Clientpfad. Die Legacy-Frames wurden erhalten, ihre alte queueabhängige Cancelzustellung wurde nicht rückwirkend aufgewertet.

### Primärquellen, Entscheidungen und Grenzen des IPC-Nachreviews

Der Rust-[`TcpStream`-Vertrag](https://doc.rust-lang.org/std/net/struct.TcpStream.html#method.shutdown) bestätigt die Wirkung von `shutdown` auf ausstehendes I/O; `try_clone` teilt den zugrunde liegenden Socket. Der [`Condvar`-Vertrag](https://doc.rust-lang.org/std/sync/struct.Condvar.html#method.wait_timeout) verlangt wiederholte Prädikatsprüfung und ein angepasstes Restbudget. Beide wurden zusätzlich an den versionsgebundenen Rust-Quellen [1.99.1 `tcp.rs`](https://github.com/rust-lang/rust/blob/1.99.1/library/std/src/net/tcp.rs) und [1.99.1 `condvar.rs`](https://github.com/rust-lang/rust/blob/1.99.1/library/std/src/sync/poison/condvar.rs) abgeglichen. Diese Versionsangabe benennt die gelesene Herstellerquelle, nicht eine hier ermittelte Compilerinstallation.

Die Zufallsquelle wurde gegen [`getrandom` 0.2.17](https://github.com/rust-random/getrandom/blob/v0.2.17/src/lib.rs) geprüft, entsprechend `native/Cargo.lock`: Fehler werden nicht als gültige Capability übernommen. Für die vorhandene Setup-Probe wurden [Linux `poll(2)`](https://man7.org/linux/man-pages/man2/poll.2.html), Microsofts [`SIO_TCP_INFO`](https://learn.microsoft.com/en-us/windows/win32/winsock/sio-tcp-info) und [`TCP_INFO_v0`](https://learn.microsoft.com/en-us/windows/win32/api/mstcpip/ns-mstcpip-tcp_info_v0) gelesen. Die Linuxprobe beobachtet `POLLRDHUP` auch vor Konsum vorhandener Eingabe; die Windowsprobe verwendet TCP-Status und nichtverbrauchende Lesebereitschaft. Die Microsoftquelle nennt Windows 10 Version 1703 als Mindestclient für `TCP_INFO_v0`. Eine Probe-API-Störung wird im vorhandenen Setup als Fehler behandelt. Abrufdatum sämtlicher ergänzender Quellen: 2026-10-25.

Entscheidungen: R01 geschlossen, keine neue Defektquote eingeführt, R02–R06 ohne neue Regressionsgrundlage nicht erneut aufgerollt. Die vollständige Remote-Update-/Runtime-Implementierung hinter `runtime_hello` ist außerhalb dieser Restprüfung; belegt sind die bestehende Generation-/Vertragsprüfung an dieser Grenze und die Weitergabe des Retainers. Die unveränderte untere Zeilenlesefunktion hinter `read_response` wurde nicht zusätzlich geöffnet. Tatsächliche Schedulerlatenz, Socketfehler je Betriebssystem, Abbruch unter realem Rückstau und Kompilierbarkeit sind nicht praktisch bestätigt. Das Lesen vorhandener Testquellen ist kein Testlauf. Es wurden keine Builds, Compiler, Tests, Server, Installationen, Graphläufe, Commits, Pushes oder Releaseaktionen ausgeführt.

Die geprüften Kerndateien wurden am Abschluss per Inhaltsvergleich fixiert: SHA-256 `exec_lifetime.rs` = `33aaa5734fb3ce8485773c7b21b9716374f4fa15a77bba756bb711b274a6d428`, `exec_ipc.rs` = `0be5508dbb53d750c5af21c6f001dd957234c100173691b2cf273647cd3755e2`. Spätere Änderungen sind durch diesen Bericht nicht vorweggenommen. Das kumulative Inventar umfasst jetzt 131 Projektdateien; ausschließlich dieser Bericht wurde geändert.

## Erster Nachreview — historischer Zwischenstand mit offenem R01-IPC-Rest

Der Hauptagent hat am 2026-10-25 die statische Nachprüfung der integrierten Korrekturen B5-R01 bis B5-R06 beauftragt. Der folgende Abschnitt bewahrt den Zwischenstand vor der abschließenden IPC-Korrektur. Der aktuelle R01-Status steht ausschließlich im abschließenden IPC-Nachreview am Anfang dieses Berichts; die R02–R06-Begründungen dieses Abschnitts bleiben gültig.

| Befund | Statischer Status dieses historischen Zwischenstands |
| --- | --- |
| R01, P1 | Offen am bestehenden Daemon-IPC-Cancel-/EOF-Weg; innere Execqueue korrigiert |
| R02, P2 | Geschlossen |
| R03, P2 | Geschlossen |
| R04, P1 | Geschlossen |
| R05, P2 | Geschlossen |
| R06, P2 | Geschlossen |

A — Ziel und Grenzen: Jeden der sechs Befunde gegen seine konkrete Zustandsfolge schließen oder offen halten; bestehende Eingabe-, Authentifizierungs-, QUIC-Migrations-, Stream-/Datagramm- und Deadlineverträge erhalten. Nur dieser Bericht wird geändert. Die erneuerte Lesefläche umfasst die 115 Manifestdateien; zusätzliche unmittelbar erforderliche Module werden ausschließlich nach ausdrücklicher Freigabe gelesen.

B — Belegter Bestand: Die Nachlektüre zeigt eine unabhängige Watch-Cancellation im Execclient, Handshake-Permits vor dem Routemonitor, frische Challengezustände mit pfadbezogenen PTOs, begrenzte atomar abgesicherte Pfadaufgabe sowie aktive Screen-HELLO-/Livenessbudgets. Die synchron auf Stdinannahme wartende Daemon-IPC-Weiterleitung wurde als offener R01-Rest bestätigt. Die Task-Scope bis HostSession ist durch den zusätzlich freigegebenen `host_server.rs` belegt.

C — Entscheidung: Den gültigen versionsgebundenen Erstabgleich wiederverwenden; die veränderten Zweige gegen dieselben Iroh-/Noq-Pins und Tokio 1.52.3 prüfen. Eine korrigierte innere Funktion genügt nicht, wenn der vorhandene Aufrufer den Abbruch davor festhält. Keine neuen praktischen Nachweise, Produktänderungen oder Freigabegates ergänzen.

D — Erreichtes Abnahmesignal: Für R01–R06 ist die ursprüngliche Zustandsfolge im korrigierten Kontrollfluss mit direkten Aufrufern bewertet und ein begründeter Status festgehalten. Das Lese-/Schreibinventar und die verbleibenden praktischen Grenzen sind aktualisiert. Die Behebung des offen gemeldeten R01-Rests gehört zum Produktionsauftrag des Hauptagenten.

### Nachreviewbefunde

**R01 — noch offen an der Daemon-IPC-Grenze; innerer Execclient korrigiert.** `exec_client_input.rs::ExecInputSender` signalisiert Cancel über einen eigenen `watch<bool>`, während Stdin/EOF/Resize begrenzt über MPSC laufen. Die normale Reservierung und die Weiterleitung beobachten Cancel unabhängig vom Füllstand. `exec_client.rs::run_connected` pollt den Cancelwatcher auch nach Ende der Weiterleitung weiter; `ShareExecSession::drop` benutzt denselben Besitzerpfad. Damit ist die zuerst beschriebene blockierte Weiterleitungsqueue geschlossen. Im zusätzlich gelesenen `exec_ipc.rs:312` wartet `serve_input` jedoch nach einem Stdinframe synchron auf `ShareExecInput::send`; ein dahinter im TCP-Stream liegendes `INPUT_CANCEL` oder EOF wird nicht gelesen. `ExecIpcInput::cancel` schreibt bei `:100` weiterhin in genau diesen Stream, unter derselben Schreibmutex wie Stdin. Der neue Watch erhält deshalb kein Signal. Der separate Statusaufruf `CancelExec` umgeht diesen Engpass, erfüllt aber nicht automatisch den bestehenden `ExecIpcInput::cancel`-Weg. Der Hauptagent hat diesen Restbefund zur Korrektur übernommen; ein nachfolgender Quellstand wird vor Schließung erneut gelesen.

**R02 — statisch geschlossen.** `application_path_revalidation.rs::start_application_path_validation_on` entfernt beim Generationen-/Versuchsstart die alten Challengeeinträge sowie den alten Retry-/Fehlertimer und setzt erst anschließend eine neue Challenge mit endlicher Frist. Die alte Antwort C ist nun in `paths.rs::on_path_response_received` unbekannt; sie erreicht den `OnPath`-Zweig und die Entfernung von `pending_validation` nicht. Auch ein Timeoutrollback auf `prev` leert dessen alte Challengeeinträge durch `application_path_validation_failed`. `mod.rs::on_path_validated` entfernt die zusätzliche Policy-Sperre nicht mehr durch bloße Handshake-Adressvalidierung. Identität, Revision, PathId und Pfadinkarnation werden weiterhin geprüft. Die ursprüngliche Zustandsfolge kann damit keine neue Policyzulassung erzeugen.

**R03 — statisch geschlossen für die belegte Kapazitätskonstellation.** `application_path_retirement.rs` wählt bei tatsächlich blockierter Pfadanlage genau einen nicht ausgewählten IPkandidaten aus; offene NAT-Pfadanlagen erreichen denselben Ersatzweg durch `check_connections`. `RetireRedundantPath(token)` prüft unter Noqs Connection-Mutex den aktuellen Token und erneut Kapazität, aktives Grant, laufende Validierung, bereits ausstehende Aufgabe und mindestens einen unabhängigen validierten überlebenden Pfad. Es ruft den vorhandenen `close_path`-/`PATH_ABANDON`-Weg auf. Dessen bestehende `MAX_PATH_ID`-/CID-Freigabe ermöglicht anschließend die wartende Neuanlage. Der gewählte Grant bleibt erhalten; ein stale Snapshot kann keinen inzwischen zugelassenen Pfad schließen. Die ursprüngliche Acht-Pfade-Konstellation besitzt jetzt einen begrenzten Fortschrittsweg. Ein nicht kooperierender Peer wird dadurch nicht zu erreichbaren neuen Pfaden verpflichtet.

**R04 — statisch geschlossen, einschließlich LAN-Link-Kompatibilitätsgrenze.** `node_accept.rs` bestimmt nach TLS die Remoteidentität und hält einen tatsächlich erworbenen `ApplicationHandshakePermit`, bevor `await_route` die globale Registrierung erreicht. `HandshakeAdmission` trennt weiterhin 64 bekannte und acht unbekannte Zulassungen. Der zusätzlich gelesene `fair_admission.rs::Ticket::try_acquire` prüft `used < capacity` unter Mutex und gibt den Platz erst beim Permit-Drop frei. Weitere unbekannte Verbindungen warten somit außerhalb der Routenregistrierung oder werden abgewiesen. Unbekannter LAN-Link wird vor dieser Registrierung abgelehnt. Die Grenze entfernt die konkrete Folge „256 unbekannte Routenregistrierungen eines Acceptpfads“; sie behauptet keine unbegrenzte Zahl legitimer Verbindungen. `lan_link_transport.rs::accept_lan_link` verlangte bereits einen akzeptierten Direct-Pin aus `contact_pin` oder `grant_pin`; deren Zustands-/Schlüsselbedingungen in `lan_link_facts.rs` implizieren den neuen `known_endpoint`-Vorfilter. Die spätere genaue Pinprüfung bleibt erhalten. Ein zusätzlicher äußerer 20-Sekunden-Sessiontimeout ist im nachgeprüften Pfad nicht vorhanden; die vorhandenen LAN-Link-Runden-/Sessionfristen bleiben maßgeblich.

**R05 — statisch geschlossen.** Die Frist verwendet nun den maximalen PTO aus Referenzpfad, Initialwert, konkret geprüftem Pfad und vorhandener vorheriger Migrationsinkarnation. Ein Fehlschlag beendet die Challengefolge ohne Grant. Nach dem Timeout kann `open_path_ensure` für einen vorhandenen Pfad oder dessen bestehender Keepalive eine neue endliche Revalidierung starten. Ein noch aktiver Challenge-/Fehlertimer verhindert dabei Fristverlängerung durch wiederholte Trigger. Die Iroh-Frühabkürzung für schon bekannte Tupel bleibt nur bei deaktivierter Policy bestehen. Die Appentscheidung behält ihre eingefrorene anfängliche Deadline; neue Versuche ermöglichen erst danach eine spätere Routeverbesserung.

**R06 — statisch geschlossen.** Die beiden HELLO-Operationen in `viewer_client.rs` laufen jetzt jeweils mit `run_with_route` und demselben einmal erzeugten Future; eine Pause startet einen angefangenen Read nicht neu. Der Host setzt und vergleicht `heard` über `active_now`. Die zusätzliche Lektüre von `host_server.rs` bestätigt die direkte Await-Kette `node_accept::scope → dispatch_connection → node_screen::handle_screen_connection → host_server::serve → Session::run`; die Livenessprüfung bleibt in derselben Task-Scope. Das gesondert gespawnte Eingabelesen verschiebt diese Prüfung nicht. Die bisherige Benutzer-/Rechtebeendigung über `stop.changed()` bleibt unabhängig. Die Änderung betrifft die zwei benannten Protokollbudgets, nicht ausdrücklich gesetzte Gesamtfristen.

**Versions- und Quellenabgleich des Nachreviews:** Die lokalen Manifest-/Lockeinträge verwenden weiterhin die drei Vendorcrates 1.3.0 und Tokio 1.52.3. Der erneute In-Memory-Vergleich von Noqs `connection/mod.rs` und Irohs `remote_state.rs` mit den unveränderten Primärpins zeigt die beschriebenen Integrationshooks; `connection/paths.rs` ist mit dem Noq-Pin identisch. Ergänzend wurden die [Watch-Semantik in Tokio 1.52.3](https://github.com/tokio-rs/tokio/blob/tokio-1.52.3/tokio/src/sync/watch.rs), dessen [Task-Scope](https://github.com/tokio-rs/tokio/blob/tokio-1.52.3/tokio/src/task/task_local.rs) und die [Timerentnahme am Noq-Pin](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq-proto/src/connection/timer.rs) gelesen. Der letzte Watchwert bleibt unabhängig von MPSC-Kapazität beobachtbar; der abgelaufene Noq-Timer wird vor dem Folgeversuch entfernt. Diese Quellen ergänzen die Primärbelege des Erstberichts; Abrufdatum 2026-10-25.

**Parallele Integrationsgrenzen:** Während des Nachreviews änderten sich innerhalb der gelesenen Produktionsfläche `native/src/screen/core/host_pipeline.rs`, `native/src/screen/core/host_server.rs` und `native/src/share/core/node_apps.rs`. Erneut gelesen wurden die Routen-/Arbeitsepochenprüfungen vor und nach der Screenaufnahme, die direkte `Session::run`-Await-Kette sowie der Apps-Routeneinstieg mit Task-Scope, aktiver Handshakefrist, Capabilitybestätigung und Permitfreigabe. Diese Einbindungen sind im zuletzt gelesenen Stand vorhanden. Die vollständigen parallel entwickelten P4-Screen- und Apps-Funktionen sind kein Ergebnis dieses B5-Nachreviews. Die Vendortransportdateien änderten sich zwischen den Inhaltsabgleichen nicht.

**Abschlussgrenze:** Die oben bewerteten Korrekturen sind ausschließlich statisch belegt. Keine Laufzeit-, Compiler- oder Releasefreigabe wird erteilt. R01 bleibt für den gelesenen Daemon-IPC-Stand offen; eine angekündigte, danach eintreffende Korrektur ist durch diesen Abschluss nicht vorweggenommen. Produktionskorrektur, Integration, gemeinsame Remote-Suite und Release bleiben beim Hauptagenten. Die vollständige kumulative Lesefläche umfasst 115 Manifestdateien und vier ausdrücklich freigegebene Zusatzdateien; das vollständige Inventar steht am Ende dieses Berichts.

## Erstreview — Stand vor den Korrekturen

Stand: 2026-10-25. Ausgangsbasis: `7442afc0` plus integrierte Arbeitskopie; die Vendorverzeichnisse waren beim Reviewbeginn untracked. Gegenstand sind P28 und P19c gemäß D5 in `batches.md` und V3 in `vertraege.md`.

Der Nutzer beauftragte am 2026-10-25 die kritische Prüfung „per Batch“ und präzisierte „also rein codeseitig und gegen quellen reviewen lässt“. Dieser Bericht enthält keine Compiler-, Laufzeit- oder Releasefreigabe. Builds, Tests, Formatter, Server, Installationen, Commits, Pushes und Graphneubauten wurden nicht ausgeführt. Die belegten Startparameter wurden vom Hauptagenten gesetzt; tatsächliche Antwortmodellmetadaten sind diesem Reviewlauf nicht zugänglich.

## Prüfplan und belegte Grundlage

A — Auftrag: Den gemeinsamen Nutzdatenvertrag für FS/Mount/Sync, Exec/Terminal, Screen, TCP, Apps, LAN-Link und Login prüfen; die Diagnose muss reale Zustände und Messbedeutung wiedergeben. Ausschließlich dieser Bericht darf verändert werden.

B — Bestand: Die Anwendung aktiviert die Policy vor Endpoint-Verbindungsaufbau. Noq-proto sperrt STREAM/DATAGRAM im PacketBuilder und vor Entnahme aus den Queues. Noq hält einen generationsgebundenen Stempel für ein bei I/O wartendes Paket. Der Appmonitor verbindet die opaque Tokens mit LAN-/Direkt-/Relayklassifikation und aktiven Fristen. Der Rootgraph enthält die neuen Routenmodule noch nicht; seine vorhandenen `endpoint_routes`-Einträge dienen nur als Navigation, nicht als Beweis für die neue Implementierung.

C — Reviewentscheidung: Versionsgebundener Vergleich der zugewiesenen Vendorquellen mit Iroh-Commit `0072d7d84b233f9e7185eb676f049beaf557ac03` und Noq-Commit `c1f411562e6078852749b8bcf1190523096a107f`; anschließend konkrete Zustandsfolgen für Generationswechsel, Rückstau, spätere Pfade und Zulassungsgrenzen. App-Prechecks werden nicht als Beleg für die Paketsperre verwendet. Vorhandene Vorgaben werden nicht durch neue Test- oder Releasegates ergänzt.

D — Abnahmesignal dieses Reviews: Jede Defektaussage nennt Auslöser, betroffenen Kontrollfluss, Vertragsverletzung, Quellenstelle und notwendige Korrektur. Widerlegte Verdachte, nicht nachgelesene Integrationsgrenzen und nicht ausgeführte praktische Nachweise werden getrennt ausgewiesen. Der zusätzliche Abgleich der freigegebenen Screenintegration und Oberflächenaufrufe ist abgeschlossen.

## Belegte Befunde

### B5-R01 — P1: Exec-Abbruch bleibt bei Rückstau hinter einer pausierten Eingabe hängen

**Fundorte:** `native/src/share/core/route_protocol.rs:52`, `native/src/share/core/exec_client.rs:108`, `native/src/share/core/exec_client_active.rs:172`, `native/src/share/core/exec_client_spawn.rs:18`.

**Auslösende Zustandsfolge:** Eine aktive Execsitzung erhält Stdin. Während `write_frame_with_timeout` auf die erneute Routezulassung wartet, liest die Execschleife `operation_rx` nicht. Die neue Weiterleitungsaufgabe füllt dessen 16 Plätze und wartet anschließend innerhalb `output.send(command)` auf den nächsten Platz. Ein nachfolgender `ExecClientInput::Cancel` liegt noch in `input`; der Weiterleiter erreicht `input.recv()` und seine Cancelprüfung nicht mehr. Der Ereignisempfänger bleibt geöffnet, weshalb `close_if_abandoned_during_pause` nicht auslöst. Der normale `spawn_connected`-/`start_exec_retained`-Weg besitzt keinen zusätzlichen `RouteCancellation`-Besitzer.

**Auswirkung und Vertrag:** Ein bereits eingereihter Benutzerabbruch wirkt erst bei Routewiederkehr oder Transportende. V3 verlangt einen sofort wirksamen Abbruch auch während der Pfadpause. Das Pausieren des aktiven Schreibtimeouts beseitigt hier außerdem die bisherige zeitliche Schranke des blockierten Schreibens. Die Nutzdaten bleiben transportseitig gesperrt; der Befund behauptet keinen ungeprüften Datenabfluss.

**Notwendige Korrektur:** Abbruch muss einen unabhängigen, während voller Nutzdatenwarteschlangen gepollten Besitzerpfad haben; normale Stdin-/Resize-Reihenfolge und begrenzte Puffer bleiben erhalten. Nur eine zusätzliche Prüfung vor `output.send` schließt die Konstellation nicht.

**Primärquelle:** Tokio **1.52.3**, entsprechend `native/Cargo.lock`, [`bounded.rs::Sender::send` und `reserve_inner`](https://github.com/tokio-rs/tokio/blob/tokio-1.52.3/tokio/src/sync/mpsc/bounded.rs): Senden wartet auf freie Kapazität. Geprüft am 2026-10-25. Die konkrete nicht mehr gepollte Cancelverzweigung ist im genannten Projektcode belegt.

### B5-R02 — P2: Eine alte Challenge-Antwort erfüllt die Validierung einer neuen Netzgeneration

**Fundorte:** `vendor/noq-proto-1.3.0/src/connection/application_path_policy.rs:250` und `:308`, `vendor/noq-proto-1.3.0/src/connection/paths.rs:503`, `vendor/noq-proto-1.3.0/src/connection/mod.rs:5675`.

**Auslösende Zustandsfolge:** Für einen bestehenden Pfad wurde Challenge C in Generation g gesendet; ihre Antwort ist noch unterwegs. `BeginGeneration(g+1)` startet die zugesagte frische Validierung, setzt dafür aber nur `pending_validation`, `pending_challenge` und den Timer. Der vorhandene Eintrag für C in `unconfirmed_challenges` bleibt erhalten. Vor Versand der neuen Challenge trifft die Antwort auf C ein. Stimmt das gespeicherte Netzwerk-Tupel weiterhin mit dem aktuellen Tupel überein, liefert `on_path_response_received` den Wert `OnPath`. `reset_on_path_challenges` löscht dabei auch die neu gesetzte Sendeabsicht. `handle_path_response_on_path` entfernt danach die aktuelle Policy-Validierungssperre. Ein **neu** gelesener Token für g+1 kann jetzt zugelassen werden, obwohl in g+1 noch keine Challenge gesendet wurde. Das betrifft insbesondere einen Netzwechsel, bei dem das interne Relay-Tupel unverändert bleibt; der explizite Generationenwechsel allein genügt ebenfalls für die Zustandsfolge.

**Auswirkung und Vertrag:** Die neue Generation erhält keinen von der alten Generation getrennten Erreichbarkeitsnachweis. `SMART_EXPLORER_ROUTE_POLICY.md:113` verspricht frische On-Path-Validierung bei `BeginGeneration`; V3 verlangt neue endliche Netzgenerationen und erneute Pfadvalidierung. Die bestehende QUIC-Tupelprüfung ist hierfür zu schwach. Der Befund behauptet weder die Annahme eines alten `Allow`-Tokens noch einen Authentifizierungsbruch. Diese Tokens werden korrekt durch Generation und Revision abgewiesen.

**Notwendige Korrektur:** Die für die Policy erforderliche Challenge/Antwort muss an die neue Generation gebunden sein. Eine alte gültige QUIC-Antwort darf die neue Policy-Sperre nicht entfernen und deren noch ausstehende Probe nicht stornieren. Allgemeine QUIC-Validierung und zusätzliche Policy-Validierung müssen dabei konsistent bleiben.

**Primärquellen:** [`paths.rs` am Noq-Pin](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq-proto/src/connection/paths.rs) speichert Challengezeit und Netzwerk-Tupel, keine Anwendungsgeneration. [RFC 9000 §8.2.3](https://www.rfc-editor.org/rfc/rfc9000.html#section-8.2.3) definiert die Validierung des Pfads, auf dem die Challenge gesendet wurde; er verspricht keine Bindung an die zusätzliche Anwendungsgeneration. Die verletzte zusätzliche Frischegarantie stammt aus V3 und der lokalen Vendorbeschreibung. Geprüft am 2026-10-25.

### B5-R03 — P2: Dauerhaft erhaltene Kandidaten können die spätere Rückkehr zu LAN verhindern

**Fundorte:** `vendor/iroh-1.3.0/src/socket/remote_map/remote_state.rs:708` und `:1056`, `vendor/iroh-1.3.0/src/endpoint/quic.rs:158`, `vendor/noq-proto-1.3.0/src/config/transport.rs:451`, `vendor/noq-proto-1.3.0/src/connection/mod.rs:574` und `:5404`, `native/src/share/core/keepalive.rs:139`.

**Auslösende Zustandsfolge:** Eine Verbindung belegt das unverändert begrenzte Budget von acht gleichzeitig offenen Pfaden, beispielsweise durch einen Relaypfad und sieben weiter erreichbare direkte Adresstupel. Die neue Bedingung `!conn.application_path_policy_enabled()` schaltet das bisherige Schließen redundanter IP-Pfade für diese Verbindung vollständig ab. Die fünfsekündigen Pfad-Keepalives erhalten die erreichbaren Kandidaten. Ein später bekannter, höher priorisierter LANpfad benötigt einen weiteren Pfadplatz. Noq verweigert dessen Öffnung mit `MaxPathIdReached`; Iroh reiht denselben Versuch nach 333 ms wieder ein. Ein Pfadplatz wird dadurch nicht frei. Die Apppolicy kann ausschließlich tatsächlich geöffnete und validierte Pfade zulassen; sie verdrängt keinen der redundanten Kandidaten.

**Auswirkung und Vertrag:** Solange die acht bisherigen Pfade erreichbar bleiben, kann die Verbindung trotz verfügbarer LANadresse dauerhaft auf der bisherigen Route bleiben. Die Änderung verhindert damit in einer gültigen Mehrpfadkonstellation die in V3 zugesagte spätere Rückkehr zu Direkt/LAN. Der Befund setzt ausdrücklich acht belegte, weiter lebende Pfade voraus; eine gewöhnliche Verbindung mit Relay und genau einem Direktpfad löst ihn nicht aus.

**Notwendige Korrektur:** Die anfängliche Kandidatenerhaltung benötigt eine begrenzte Ersatz-/Freigaberegel für neue höher priorisierte Pfade. Das aktuelle Grant, der letzte nutzbare Pfad und laufende Validierungen müssen geschützt bleiben; unbegrenztes Anheben der Pfadgrenze ersetzt diese Regel nicht.

**Primärquellen:** [`socket.rs` am Iroh-Pin](https://github.com/n0-computer/iroh/blob/0072d7d84b233f9e7185eb676f049beaf557ac03/iroh/src/socket.rs) definiert `MAX_MULTIPATH_PATHS = 8`; [`TransportConfig` am Noq-Pin](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq-proto/src/config/transport.rs) rechnet dies in die anfänglich höchste PathId 7 um. [`Connection` am Noq-Pin](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq-proto/src/connection/mod.rs) prüft diese Grenze beim Öffnen und erhöht die Kapazität nach Pfadaufgabe. Geprüft am 2026-10-25.

### B5-R04 — P1: Unbekannte TLS-Peers können vor der getrennten Zulassung die globale Routenregistrierung füllen

**Fundorte:** `native/src/share/core/node_accept.rs:33`, `:45` und `:82`, `native/src/share/core/node_routes.rs:82`, `native/src/share/os/shared/route_monitor.rs:15` und `:40`, `native/src/share/core/handshake_limits.rs:13`.

**Auslösende Zustandsfolge:** Ein unbekannter Peer kann den Iroh-TLS-Handshake unter seiner eigenen Transportidentität abschließen; dafür besitzt er noch keine Appberechtigung. Der Acceptpfad gibt danach den TLS-Semaphorplatz frei und startet `await_route`. Jeder solche noch nicht appauthentifizierte Kanal belegt einen Eintrag in der globalen, auf 256 Verbindungen begrenzten Registrierung. Der Peer hält die Verbindungen während der Routenprüfung offen und beantwortet die zur Freigabe nötigen Challenges nicht rechtzeitig. Wiederholte Handshakes können die 256 Plätze belegen. Eine danach eintreffende Verbindung eines gepinnten Geräts scheitert bereits in `route_monitor::start` mit `WouldBlock`. Die Trennung in bekannte und unbekannte Geräte sowie die App-Authentifizierung werden erst nach dieser Hürde erreicht.

**Auswirkung und Vertrag:** Die bisherige Trennung der Kapazität für bekannte und unbekannte Geräte schützt diese neu vorgeschaltete Ressource nicht. Ein unbekannter Peer kann dadurch bekannte FS-, Exec-, Screen-, TCP- und Appsverbindungen sowie ausgehende Routenstarts desselben Prozesses zurückweisen lassen. Die Registrierung ist begrenzt; der Befund ist Kapazitätsverdrängung, kein unbegrenztes Speicherwachstum und keine Erteilung fremder Rechte. Er setzt gleichzeitig lebende, TLS-abgeschlossene Verbindungen voraus und behauptet keine praktisch gemessene Angriffsrate. Der separate Login-Gatewaypfad prüft seine Transportidentität vor dem Monitor und ist nicht die hier beschriebene Eintrittsstelle.

**Notwendige Korrektur:** Schon die Routen-Vorzulassung muss die bekannte TLS-Remoteidentität und die bestehende Trennung/Fairness berücksichtigen. Ein unbekannter Peer darf die für gepinnte Geräte benötigte Kapazität nicht vollständig belegen. Appnutzdaten bleiben bis zur Routezulassung gesperrt; ein Vorziehen ihres Handshakes ist keine Korrektur.

**Primärbeleg:** Der vorhandene `HandshakeAdmission` in `handshake_limits.rs` besitzt getrennte Pools für bekannte Geräte (`capacity`) und unbekannte Geräte (`capacity / 8`, mindestens eins). Der aktuelle `node_accept.rs` verschiebt die neue gemeinsame Ressourcenbelegung vor diese Schutzgrenze. Der Vergleich mit `7442afc0` belegt, dass die zusätzliche globale Vorzulassung aus diesem Batch stammt. Die konkrete Zustandsfolge folgt aus diesen Projektquellen und benötigt keine angenommene externe APIgarantie.

### B5-R05 — P2: Fehlgeschlagene Policy-Revalidierung lässt einen lebenden Pfad ohne neuen Probeweg gesperrt

**Fundorte:** `vendor/noq-proto-1.3.0/src/connection/application_path_policy.rs:308`, `vendor/noq-proto-1.3.0/src/connection/mod.rs:553`, `:2588` und `:5740`, `vendor/noq-proto-1.3.0/src/connection/paths.rs:568`.

**Auslösende Zustandsfolge:** Ein bereits QUIC-validierter, nicht migrierender Direktpfad wird bei Beginn einer neuen Policygeneration erneut geprüft. Seine Challenge-Antworten fehlen bis zum gesetzten Timer, beispielsweise wegen vorübergehenden Paketverlusts; PING/ACK hält den Pfad weiter offen. Der bestehende `PathValidationFailed`-Zweig stoppt `PathChallengeLost`, leert die Challenges und löscht `pending_challenge`. Die neu eingeführte Menge `pending_validation` bleibt für diesen Pfad gesetzt. Ein später erfolgreicher NAT-/Upgraderundenversuch findet denselben nicht aufgegebenen Pfad in `open_path_ensure` als `already_existed`; dieser Zweig stellt keine neue Challenge ein. Normale Keepalive-ACKs entfernen die zusätzliche Policy-Sperre ebenfalls nicht. Ohne Netzgenerationenwechsel, Migration oder Aufgabe/Neuanlage des Pfads entsteht kein neuer Freigabenachweis.

**Zusätzliche Fristabweichung:** `start_application_path_validation` setzt für sämtliche Pfade `3 × max(PTO_initial, PTO_best)`. Ein vorhandener langsamer Direktpfad erhält dadurch bei schnellerem Relaypfad eine kürzere Validierungsfrist als `3 × max(PTO_bestand, PTO_des_geprüften_Pfads)`. Das höhere Appbudget in `RoutePolicy::include_validation_pto` korrigiert den bereits gestarteten Transporttimer nicht.

**Auswirkung und Vertrag:** Ein vorübergehender Probeausfall kann einen weiter lebenden Direkt-/LANpfad für die restliche Netzgeneration dauerhaft aus der Auswahl entfernen. Die endliche erste Runde darf Relay freigeben, muss aber die in V3 vorgesehene spätere Rückkehr ermöglichen. Der tatsächliche Transporttimer muss außerdem das zugesagte pfadbezogene PTO-Budget einhalten.

**Notwendige Korrektur:** Fehlgeschlagene Policy-Revalidierung braucht einen endlichen, erneut anstoßbaren Zustand oder eine sichere Aufgabe/Neuanlage. Dabei müssen lebende QUIC-Pfade und die zusätzliche Policy-Sperre konsistent bleiben. Die Frist muss den PTO des jeweils geprüften Pfads berücksichtigen; sie darf die endliche initiale Appentscheidung nicht unbegrenzt verlängern.

**Primärquellen:** [`Connection` am Noq-Pin](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq-proto/src/connection/mod.rs) enthält den übernommenen Migrationstimer und den wiederverwendenden `open_path_ensure`-Zweig. Die zusätzliche dauerhafte Sperre entsteht durch ihre neue Kombination mit `pending_validation`. [RFC 9000 §8.2.4](https://www.rfc-editor.org/rfc/rfc9000.html#section-8.2.4) nennt den größeren PTO des aktuellen und des geprüften Pfads und begrenzt die Aussage eines Fehlversuchs auf diesen Validierungsversuch. Geprüft am 2026-10-25.

### B5-R06 — P2: Screen-HELLO und Viewer-Liveness verbrauchen weiterhin Routepausen als Fehlerzeit

**Fundorte:** `native/src/screen/core/viewer_client.rs:174` und `:178`, `native/src/screen/core/host_session.rs:40`, `:137`, `:150` und `:162`; zum bereits angebundenen GOP-Versand `native/src/screen/core/gop_stream.rs:69`.

**Auslösende Zustandsfolge:** Nach der ersten erfolgreichen Routezulassung beginnt der Screenhandshake. Während `accept_bi` oder `recv_server_hello` aussteht, sperrt ein Netz-/Pfadwechsel die Anwendungsframes erneut. Die beiden rohen Tokio-Timeouts laufen dennoch gegen 20 Sekunden Gesamtzeit; eine längere gültige Routenprüfung endet deshalb mit `screen host sent no hello`. In einer bestehenden Sitzung misst der Host zusätzlich `heard.elapsed()` gegen `VIEWER_SILENCE = 15 s`, obwohl die App-PINGs während der Routepause transportseitig gesperrt sind. Der erste nach Überschreiten dieses Intervalls ausgewertete Sekundentick kann die Sitzung als verloren beenden. QUIC-Keepalives können die Verbindung währenddessen erhalten; fehlender App-PING ist dann kein Beleg für einen verstummten Viewer.

**Auswirkung und Vertrag:** Die gemeinsame aktive Frist gilt bisher für GOP-Schreiben, nicht vollständig für Screenaufbau und Sitzungs-Liveness. Ein erlaubter Pfadwechsel kann daher einen neuen oder laufenden Screenkanal abbrechen. Diese eingebauten Protokollbudgets sind keine ausdrücklich vom Aufrufer gesetzten Gesamtfristen. V3 verlangt die Pause aktiver Anwendungsfristen und eine Wiederaufnahme mit neuer Referenz.

**Notwendige Korrektur:** Die beiden HELLO-Budgets und der Livenesszähler müssen die gemeinsame aktive Routezeit verwenden beziehungsweise nur zulässige aktive Intervalle verbrauchen. Benutzerabbruch, Rechteablauf und ausdrücklich gesetzte Gesamtfristen bleiben unabhängig wirksam. Der noch laufende HELLO-Read darf nicht nach einem Teil-Read neu gestartet werden.

**Primärquelle:** Tokio **1.52.3**, [`timeout.rs`](https://github.com/tokio-rs/tokio/blob/tokio-1.52.3/tokio/src/time/timeout.rs), bildet die Frist aus `Instant::now() + duration`; die lokale Task-Scope ändert diese Funktion nicht. Die 15-Sekunden-Prüfung nutzt ausdrücklich `std::time::Instant`. Geprüft am 2026-10-25.

## Widerlegte Verdachte und nachgewiesene Einbindung

- **Bloße Clientpräferenz:** Widerlegt für den untersuchten Sendepfad. Die Policy wird vor dem Endpointaufbau eingeschaltet. Der gemeinsame PacketBuilder klassifiziert STREAM und DATAGRAM vor Verschlüsselung; die Stream- und Datagrammqueues prüfen die Sperre zusätzlich vor Entnahme. Der Allow-Token benennt die konkrete Verbindung, Netzgeneration, Revision und Pfadinkarnation. Irohs Available/Backup-Auswahl allein erteilt keinen Grant.
- **Bereits verschlüsseltes Pending-Transmit umgeht spätere Sperre:** Im untersuchten Noq-Treiber widerlegt. Policyänderung und Sendepfad teilen die Connection-Mutex; ein zurückgehaltenes Paket trägt den Transportstempel und wird bei dessen Änderung verworfen. Zuverlässige Streamdaten bleiben in der bestehenden Verlust-/Wiederholungsverwaltung. DATAGRAM bleibt Best Effort.
- **Ungeprüfter 0-RTT-/Handshake-Durchlass oder Anwendungsfehlertext als Steuerdaten:** In den verglichenen Serialisierungszweigen kein solcher Durchlass gefunden. Die Frametypen sind für diesen Pin vollständig klassifiziert; gesperrte App-Close-Texte werden vor Ausgabe entfernt. Das ist ein statischer Quellbefund, keine Paketmessung.
- **Operation wird bei Routewiederkehr neu erzeugt:** `route_protocol::run_with_route` hält genau ein gepinntes Future und pausiert dessen aktives Budget. Die gelesenen FS-/Exec-/TCP-/Apps-/LAN-Link-/Login-Einstiege behalten Identitätsprüfung, HMAC/Beziehungsnachweis und Rechtebindung. B5-R01 und B5-R06 bleiben konkrete Ausnahmen beim Abbruch-/Fristvertrag.
- **Private IPadresse gilt pauschal als LAN:** In den drei zugewiesenen OS-Adaptern widerlegt. Die Einstufung verlangt Interface- und On-Link-Fakten; unbekannte Typen bleiben Direkt. Windows verwendet `GetBestRoute2`, Linux/Android verwenden die Kernel-Routeninformation. Die tatsächliche Verfügbarkeit dieser Informationen unter Android-SELinux und realen VPNs ist nicht praktisch geprüft.
- **Diagnose behauptet bestätigten Nutzdurchsatz:** Widerlegt. `service_routes.rs` nennt ausdrücklich serialisierte TX-Frame-Nutzlast einschließlich Wiederholungen und verworfener Sendepläne sowie authentifiziert verarbeitete RX-Frame-Nutzlast. Unbekannte RTT/Zeitstempel werden nicht erfunden. Die Historie gehört dem gepinnten Gerätepaar und wird keinem ungesicherten ALPN zugeordnet.
- **Noch unentschiedene Verbindung heißt Relay:** Die neue Projektion führt Sammeln, LANprüfung, Direktprüfung, Pause, Offline und begründeten Relayrückfall getrennt. Bei älteren Peers wird die nur lokale Sendegarantie benannt; eine Capabilitybehauptung ersetzt keine Berechtigung.
- **Diagnoseansichten sind nicht eingebunden:** Die freigegebenen Desktop-Geräteeinstiege rufen `share_routes_ui::device` für Kontakte, direkte Grants und Geräteschlüssel auf. Androids `DeviceSettingsPage.kt:116` bindet `RouteDetails` ein. CLI-/MCP-/Daemon-/Mobileprojektionen sind im zugewiesenen Quellumfang angeschlossen. Die Aktualisierung des Desktopfeldes `app.share_routes` außerhalb dieser Einhängepunkte gehört nicht zur freigegebenen Lesefläche und wurde hier nicht behauptet.
- **Screen besitzt keinerlei Referenzinvalidierung bei Routewechsel:** Widerlegt. `PipelineActor::poll` vergleicht Generation/Pauseepoch/Klasse/Zulassung, unterbricht laufende Arbeit und verwirft die Referenz; `host_frame_send` prüft die Arbeitsepoche und setzt den GOP zurück. `host_pipeline` verlangt danach einen Keyframe. Die asynchrone Reihenfolge zwischen Monitorfreigabe, bereits gepufferten Screenbytes, Actorinterrupt und Viewerpräsentation wurde nicht praktisch beobachtet und erhält durch diesen Befund keine Racefreiheit-Zusage.

## Quellenstand und praktische Grenzen

Die Herstellerquellen wurden am 2026-10-25 abgerufen. Iroh und Noq wurden gegen die oben genannten unveränderlichen Commitstände verglichen, Tokio gegen den Lockfile-Pin 1.52.3. Für die Windowsstruktur wurde zusätzlich die [generierte IpHelper-Bindung von windows-sys 0.59.0](https://github.com/microsoft/windows-rs/blob/0.59.0/crates/libs/sys/src/Windows/Win32/NetworkManagement/IpHelper/mod.rs) und die [Microsoft-Dokumentation zu GetBestRoute2](https://learn.microsoft.com/en-us/windows/win32/api/netioapi/nf-netioapi-getbestroute2) gelesen. Der Verdacht eines nicht vorhandenen `_bitfield`-Feldes wurde am passenden Pin widerlegt.

Der lokale Graph wurde nur als vorhandene AST-Navigation gelesen; seine fehlenden neuen Routenmodule begrenzen seinen Beweiswert. Kein Graphneubau wurde gestartet. Die Prüfung nutzte gezielte Textlektüre, statische Suchläufe, JSON-/Textverarbeitung und Quellvergleiche; sie führte keine Produktionsfunktion und keine Tests aus. Versionsgebundene Fremdquellen wurden für den Vergleich nur im Speicher gehalten.

Offen bleiben die praktische Bestätigung von Nutzbytes pro Richtung bei 0-RTT, Retransmits, Pending-I/O und Netzwechsel; die reale Win/Linux/Android-Interfaceerkennung; tatsächliche zeitliche Obergrenzen und Wiederkehr unter Verlust; die Screenfreigabe-/Präsentationsreihenfolge; die tatsächliche Desktop-Snapshotaktualisierung außerhalb des genehmigten Einstiegs; Kompilierbarkeit der drei gepatchten Crates und aller Appverbraucher. Diese Punkte sind weder als Defekt noch als bestanden ausgewiesen.

Zum Abschluss des Erstreviews hatte der Hauptagent die Korrektur von B5-R01 bis B5-R05 übernommen; B5-R06 war ihm mit der Screenzustandsfolge gemeldet. Die vorstehenden Erstbefunde bewahren diese ursprünglichen Fehlerstände. Ihre inzwischen geprüften Korrekturen und der verbleibende R01-Rest werden ausschließlich im Nachreview am Anfang dieses Berichts bewertet. Produktionsänderungen, Integration, gemeinsame Remote-Suite und Release bleiben beim Hauptagenten.

## Entscheidungen und Inventar

Entscheidungen: Keine Produktionsdatei geändert; keine pauschale Sicherheitsfreigabe aus App-Prechecks abgeleitet; Timeoutbeweis und Probeerschöpfung gemäß V3 als zulässige endliche Entscheidungsformen unterschieden; Modellstartparameter nicht mit nicht verfügbaren Antwortmodellmetadaten gleichgesetzt; keine Befundquote verwendet. P1 bezeichnet hier unmittelbar zu behebende Abbruch-/Verfügbarkeitsregressionen, P2 belegte bedingte Korrektheits-/Wiederkehrfehler.

**Schreibinventar, vollständig:** ausschließlich neu angelegt und fortgeschrieben: `docs/plaene/2026-10-24-batches/review-b5.md`. Kein Commit und kein Push durch diesen Subagenten.

**Zentrale Quellen und Metadaten, vollständig:** die vom Nutzer gelieferten Projektregeln; `/tmp/smartexplorer-review-b5-scope.json`; `/root/.codex/skills/arbeitsweise/SKILL.md`; `/root/.agents/skills/arbeitsweise/SKILL.md`; `/root/.codex/skills/graphify/SKILL.md`; die über das lokale Arbeitsweise-MCP gelieferten Anleitungen; Gitstatus und HEAD-Metadaten sowie gezielte HEAD-/Arbeitskopievergleiche der freigegebenen Dateien. Die folgende Dateiliste dokumentiert sämtliche lokalen Projektlesezugriffe einschließlich gezielter Suchen und Textinventarisierung; sie behauptet keine vollständige semantische Prüfung jeder Zeile jeder Datei.

**Kumulative Manifestlesefläche des Erst- und Nachreviews, vollständig (115 Dateien):**

- `AGENTS.md`
- `android/app/src/main/java/app/smartexplorer/android/api/RouteApi.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/share/DeviceSettingsPage.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/share/RouteDetails.kt`
- `docs/ARCHITEKTUR.md`
- `docs/plaene/2026-10-24-batches/batches.md`
- `docs/plaene/2026-10-24-batches/recherche.md`
- `docs/plaene/2026-10-24-batches/review-b5.md`
- `docs/plaene/2026-10-24-batches/umsetzung.md`
- `docs/plaene/2026-10-24-batches/vertraege.md`
- `graphify-out/graph.json`
- `native/Cargo.lock`
- `native/Cargo.toml`
- `native/src/app/core/share_device_page_ui.rs`
- `native/src/app/core/share_routes_ui.rs`
- `native/src/cli/mcp/status_tools.rs`
- `native/src/cli/share/status.rs`
- `native/src/daemon/os/shared/exec_connect.rs`
- `native/src/daemon/os/shared/exec_ipc.rs`
- `native/src/daemon/os/shared/exec_state.rs`
- `native/src/daemon/os/shared/ipc_host_commands.rs`
- `native/src/daemon/os/shared/ipc_protocol.rs`
- `native/src/daemon/os/shared/ipc_protocol_bounds.rs`
- `native/src/login/core/gateway_types.rs`
- `native/src/login/os/shared/gateway.rs`
- `native/src/login/os/shared/gateway_client.rs`
- `native/src/mobile/os/shared/domains/route_status.rs`
- `native/src/screen/core/gop_stream.rs`
- `native/src/screen/core/host_frame_send.rs`
- `native/src/screen/core/host_pipeline.rs`
- `native/src/screen/core/host_pipeline_actor.rs`
- `native/src/screen/core/host_reference.rs`
- `native/src/screen/core/host_session.rs`
- `native/src/screen/core/host_work_control.rs`
- `native/src/screen/core/viewer_client.rs`
- `native/src/share/core/direct_protocol_tests.rs`
- `native/src/share/core/endpoint_routes.rs`
- `native/src/share/core/exec_admission.rs`
- `native/src/share/core/exec_client.rs`
- `native/src/share/core/exec_client_active.rs`
- `native/src/share/core/exec_client_input.rs`
- `native/src/share/core/exec_client_spawn.rs`
- `native/src/share/core/exec_server.rs`
- `native/src/share/core/exec_session.rs`
- `native/src/share/core/handshake_limits.rs`
- `native/src/share/core/io_deadline.rs`
- `native/src/share/core/keepalive.rs`
- `native/src/share/core/login_transport.rs`
- `native/src/share/core/node.rs`
- `native/src/share/core/node_accept.rs`
- `native/src/share/core/node_apps.rs`
- `native/src/share/core/node_apps_link.rs`
- `native/src/share/core/node_routes.rs`
- `native/src/share/core/node_screen.rs`
- `native/src/share/core/node_screen_link.rs`
- `native/src/share/core/node_sessions.rs`
- `native/src/share/core/node_tcp.rs`
- `native/src/share/core/node_tcp_link.rs`
- `native/src/share/core/node_wake.rs`
- `native/src/share/core/peer_storage_snapshot.rs`
- `native/src/share/core/peer_stream.rs`
- `native/src/share/core/peer_walk.rs`
- `native/src/share/core/route_cancellation.rs`
- `native/src/share/core/route_diagnostics.rs`
- `native/src/share/core/route_policy.rs`
- `native/src/share/core/route_protocol.rs`
- `native/src/share/core/route_state.rs`
- `native/src/share/core/server.rs`
- `native/src/share/core/server_batch_put.rs`
- `native/src/share/core/service.rs`
- `native/src/share/core/service_routes.rs`
- `native/src/share/core/session.rs`
- `native/src/share/core/signal_connected.rs`
- `native/src/share/core/signal_presence.rs`
- `native/src/share/core/signal_session.rs`
- `native/src/share/core/signal_worker_tests.rs`
- `native/src/share/core/tracked_signal_dispatch.rs`
- `native/src/share/core/tracked_signal_tests.rs`
- `native/src/share/core/wire.rs`
- `native/src/share/mod.rs`
- `native/src/share/os/android/route_interfaces.rs`
- `native/src/share/os/linux_os/route_interfaces.rs`
- `native/src/share/os/shared/lan_link_dial.rs`
- `native/src/share/os/shared/lan_link_exchange.rs`
- `native/src/share/os/shared/route_adapter.rs`
- `native/src/share/os/shared/route_history.rs`
- `native/src/share/os/shared/route_monitor.rs`
- `native/src/share/os/shared/tcp_pump.rs`
- `native/src/share/os/shared/transport_options.rs`
- `native/src/share/os/windows/route_interfaces.rs`
- `vendor/SMART_EXPLORER_ROUTE_POLICY.md`
- `vendor/iroh-1.3.0/Cargo.toml`
- `vendor/iroh-1.3.0/LICENSE-APACHE`
- `vendor/iroh-1.3.0/LICENSE-BSD3`
- `vendor/iroh-1.3.0/LICENSE-MIT`
- `vendor/iroh-1.3.0/src/endpoint.rs`
- `vendor/iroh-1.3.0/src/endpoint/connection.rs`
- `vendor/iroh-1.3.0/src/endpoint/quic.rs`
- `vendor/iroh-1.3.0/src/socket/remote_map/application_path_retirement.rs`
- `vendor/iroh-1.3.0/src/socket/remote_map/remote_state.rs`
- `vendor/noq-1.3.0/Cargo.toml`
- `vendor/noq-1.3.0/src/connection.rs`
- `vendor/noq-1.3.0/src/lib.rs`
- `vendor/noq-proto-1.3.0/Cargo.toml`
- `vendor/noq-proto-1.3.0/src/config/transport.rs`
- `vendor/noq-proto-1.3.0/src/connection/application_path_policy.rs`
- `vendor/noq-proto-1.3.0/src/connection/application_path_revalidation.rs`
- `vendor/noq-proto-1.3.0/src/connection/datagrams.rs`
- `vendor/noq-proto-1.3.0/src/connection/mod.rs`
- `vendor/noq-proto-1.3.0/src/connection/packet_builder.rs`
- `vendor/noq-proto-1.3.0/src/connection/paths.rs`
- `vendor/noq-proto-1.3.0/src/connection/streams/state.rs`
- `vendor/noq-proto-1.3.0/src/frame.rs`
- `vendor/noq-proto-1.3.0/src/lib.rs`
- `vendor/noq-proto-1.3.0/src/n0_nat_traversal.rs`

**Zusätzlich zum aktualisierten Manifest ausdrücklich freigegeben und gelesen, vollständig (4 Dateien):**

- `native/src/screen/core/host_server.rs`
- `native/src/share/core/fair_admission.rs`
- `native/src/share/os/shared/lan_link_transport.rs`
- `native/src/share/core/lan_link_facts.rs`

**Im abschließenden IPC-Nachreview zusätzlich freigegeben und gelesen, vollständig (12 Dateien):**

- `native/src/daemon/mod.rs`
- `native/src/daemon/os/linux_os/exec_connect.rs`
- `native/src/daemon/os/shared/exec_ipc_tests.rs`
- `native/src/daemon/os/shared/exec_lifetime.rs`
- `native/src/daemon/os/shared/ipc.rs`
- `native/src/daemon/os/shared/ipc_admission.rs`
- `native/src/daemon/os/shared/ipc_client.rs`
- `native/src/daemon/os/shared/ipc_host.rs`
- `native/src/daemon/os/shared/ipc_listener.rs`
- `native/src/daemon/os/shared/ipc_protocol_io.rs`
- `native/src/daemon/os/shared/runtime_hello.rs`
- `native/src/daemon/os/windows/exec_connect.rs`

Die erneut gelesenen `exec_connect.rs` unter `os/shared`, `exec_ipc.rs`, `exec_state.rs`, `ipc_protocol.rs`, `exec_session.rs`, `exec_client_input.rs`, `exec_client.rs`, `exec_client_spawn.rs`, `route_protocol.rs`, `node.rs`, Cargoquellen und Plan-/Berichtsdateien sind bereits in der Manifestliste enthalten. Die neuen Readfreigaben verdoppeln diese Einträge nicht.

**Erfolgreich abgerufene externe Primärquellen, vollständiger Datei-/Dokumentumfang:**

- Iroh-Commit `0072d7d84b233f9e7185eb676f049beaf557ac03`: `iroh/src/endpoint.rs`, `iroh/src/endpoint/connection.rs`, `iroh/src/endpoint/quic.rs`, `iroh/src/socket/remote_map/remote_state.rs`, `iroh/src/socket.rs`.
- Noq-Commit `c1f411562e6078852749b8bcf1190523096a107f`: `noq/src/connection.rs`, `noq/src/lib.rs`, `noq-proto/src/config/transport.rs`, `noq-proto/src/connection/mod.rs`, `noq-proto/src/connection/packet_builder.rs`, `noq-proto/src/connection/streams/state.rs`, `noq-proto/src/connection/datagrams.rs`, `noq-proto/src/connection/paths.rs`, `noq-proto/src/connection/timer.rs`, `noq-proto/src/frame.rs`, `noq-proto/src/lib.rs`, `noq-proto/src/n0_nat_traversal.rs`.
- Tokio-Tag `tokio-1.52.3`: `tokio/src/sync/mpsc/bounded.rs`, `tokio/src/time/timeout.rs`, `tokio/src/sync/watch.rs`, `tokio/src/task/task_local.rs`.
- windows-rs-Tag `0.59.0`: `crates/libs/sys/src/Windows/Win32/NetworkManagement/IpHelper/mod.rs`.
- RFC 9000 mit den einschlägigen Abschnitten 8.2.3 und 8.2.4; Microsoft-Learn-Dokument `GetBestRoute2 function (netioapi.h)`.
- Rust-Tag `1.99.1`: `library/std/src/net/tcp.rs` und `library/std/src/sync/poison/condvar.rs`; zusätzlich die aktuellen Rust-APIseiten `std::net::TcpStream` und `std::sync::Condvar`.
- getrandom-Tag `v0.2.17`: `src/lib.rs`.
- Linux-man-pages: `poll(2)`; Microsoft Learn: `SIO_TCP_INFO Control Code` und `TCP_INFO_v0 structure (mstcpip.h)`.
