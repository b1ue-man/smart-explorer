# Fixbericht: Screen-Metadaten und Tokio

Stand: 2026-10-27. Ausgangsquellstand: `b439d90e` (v0.5.185).
Dieser Bericht enthält keine betrieblichen Nutzer-, Geräte- oder Dienstpfade.

## Auftrag und erste Planung

Der Fixautor bearbeitet die belegte Runtime-Regression, ihre verketteten
Funktionen, den konkreten Fixplan, den Fix und die anschließende eigene
Anti-Regressionsprüfung. Die später ausgeführte eine Remote-Suite, Graphpflege,
Commits, Push und der vollständige terminale Release gehören dem Hauptagenten.
Lokal werden weder Tests noch Builds ausgeführt.

Promptbefund: Eine Linux-Fenster-/Iconabfrage panikt im Tokio-Share-Task;
danach ist der gemeinsame Share-Autorisierungszustand vergiftet. Der vorhandene
Monitorzugang ist ein getrennter, bereits betrieblich korrigierter Befund.
Die vorhandenen Fenster, Icons, Familien, Monitore, Codecs, Eingabe- und
Screenrechte bleiben erhalten. Noch offen: genaue Joblebensdauer, Abbruch,
Publikation unter Widerruf und die weiterhin laufende Viewer-Verarbeitung.

## Bestand und verkettete Funktionen

1. `icon_host::serve_metadata` ruft `icon_host::{list,icon}` und
   `family_listing::list` synchron aus seinem Async-Handler auf.
2. `host_session_control::Session::on_frame` nutzt dieselben drei Funktionen
   in laufenden Viewern. Ein bloßes Awaiten eines Hintergrundjobs würde zwar
   den Executor freigeben, aber dieselbe Capture-/Eingabeschleife anhalten.
3. `icon_host::{list,icon}` und `family_listing::list` führen die Plattformarbeit
   innerhalb von `ScreenAuthority::admit` aus. `NodeAuthority::admit`
   (`share/core/node_screen.rs`) hält dabei den gemeinsamen Auth-Mutex.
4. `WorkerBackend::{windows,icon}` delegieren an den Plattformadapter;
   `WorkerBackend::family_windows` verwendet ebenfalls dessen Fensterliste.
   Linux: `LinuxBackend::windows` → `app_icons::attach` → `current_theme` →
   `zbus::blocking`; `load`, `catalog_icon` und `attach_apps` teilen die
   Themeauflösung. Die synchronen Katalog-/Startadapter bleiben erhalten.
5. Linux-Fensterlisten behalten X11/EWMH, Wayland-Toplevels, KDE und
   systemd-Appquellen. Die Listenherkunft, installierte Appidentität,
   aktuelle Iconreferenz, PNG-/SVG-Grenzen und NET_WM-Fallback bleiben erhalten.
6. `host_server::Start::hello_ok`, primäre Monitorauswahl und
   `host_server_start::prepare` fragen Plattform-/Codec-/Monitor-/Limit- und
   Speichermetadaten ebenfalls synchron im Async-Pfad ab. Linux-`limits`
   kann eine Wayland-Fensterliste abfragen und so D-Bus erreichen.
7. `ScreenRegistry::{restrict_authorization,stop}` signalisiert Widerruf,
   Ablauf und Sitzungsende. `SessionHandle` besitzt die Registrierung;
   Kontrollfreigabe, gehaltene Eingaben, Capture-Actor und Schlussmeldungen
   bleiben Teil der laufenden Session.
8. `control_sender::send` kann beim Budget oder Transport warten. Eine
   Rechteprüfung nur nach der Abfrage deckt diesen späteren Wartepunkt nicht ab.

## Herstellerbefunde und Entscheidung

Die lokal vorhandenen, durch `native/Cargo.lock` gebundenen Quellen sind
`zbus-5.16.0/src/blocking/mod.rs` (Caveats: eigene `block_on`-Runtime, kein
Aufruf aus Async-Kontext) und `tokio-1.52.3/src/task/blocking.rs`
(`spawn_blocking`: endliche Arbeit, begonnene Arbeit nicht abbrechbar,
Semaphore zur Begrenzung). Recherche am 2026-10-27. Online bestätigt die
[Tokio-Primärdokumentation](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)
die Grenze; die zbus-Latest-URL war bei diesem Abruf nicht verfügbar, deshalb
ist die gepinnte Quelle hier maßgeblich.

Gewählt: blockierende Plattformabfragen in besessene Hintergrundjobs verlegen;
vor und nach der Arbeit kurz autorisieren; Plattformcode niemals im
Auth-Callback ausführen. Eine Lane pro tatsächlichem Backend bewahrt die
bisherige Serialisierung durch den Auth-Mutex, ohne diesen Mutex zu halten.
Ein bereits begonnener Job hält seine Lane bis zum tatsächlichen Ende.
Abgebrochene wartende Jobs führen keine Plattformarbeit mehr aus.

Laufende Viewer besitzen eine begrenzte FIFO für Metadaten und pollen deren
Abschluss unabhängig von Capture, Kontrollframes und Abbruch. Die Queuegrenze
verwendet die vorhandene Kontrollframe-Queue des Viewers. Jede tatsächliche
Metadaten-Schreiboperation prüft Rechte und Sitzungsstopp erneut, einschließlich
eines nach Backpressure erneut gepollten Schreibens.

Verworfen: Icons entfernen (Fähigkeitsverlust), Panic ignorieren oder
vergifteten Auth-Zustand übernehmen (Ursache bleibt), `block_in_place`
(ungeeignet für current-thread-Runtimes und dieselbe Viewer-Schleife), nur
`spawn_blocking` awaiten (Capture-/Kontrollschleife wartet weiterhin),
unbegrenzte abgelöste Jobs (Abbruch lässt unbegrenzte Restarbeit zu).

## Detaillierter Arbeitsplan vor Quelländerung

1. `host_metadata_jobs.rs`: Lane, besessener Job, Auftragsdeadline,
   kurzzeitige Vor-/Nachautorisierung und Drop-Abbruch. Erwartung: Nested-Runtime
   ist erlaubt, Auth bleibt frei; abgelöste Arbeit kann weder neu starten noch
   Ergebnisse veröffentlichen und vervielfacht keine Backendarbeit.
2. `icon_host.rs`, `family_listing.rs`, `host_metadata.rs`: synchrone Verträge
   erhalten, Plattformaufrufe aus Auth-Callbacks entfernen; ein gemeinsamer
   Metadatenrequest, Capability-Snapshot und autorisierter Writer. Erwartung:
   identische Listen/Iconinhalte, keine Bytes nach Widerruf oder Sitzungsstopp.
3. `host_server.rs`, `host_server_start.rs`: Snapshot asynchron holen,
   Handshake und Startup verwenden denselben aktuellen Plattformbestand.
   Erwartung: Probe, Metadatenkanal und Bildschirmstart erreichen die vorhandenen
   Quellen und Codecs ohne synchrone D-Bus-Aufrufe im Tokio-Task.
4. `host_session.rs`, `host_session_control.rs`: Metadaten getrennt pollen,
   bestehende Eingabe-/Kontroll-/Capturepfade erhalten. Erwartung: langsame
   Metadaten blockieren keine Capture-Ticks, Pings oder Kontrollfreigabe.
5. Regressionsquellen `screen_metadata_runtime_task_tests.rs` und
   `screen_metadata_runtime_fixtures.rs` für die gemeinsame Remote-Suite:
   Runtimevarianten, Ergebnisidentität, Backend-Panic ohne Authvergiftung,
   Widerruf während Arbeit/Schreiben, Abbruch und laufender Viewer.
6. Eigene statische Anti-Regressionsprüfung gegen alle obigen Verträge;
   Laufzeitbelege bleiben bis zum Remote-Ergebnis ausdrücklich offen.

## Umsetzung und eigene Anti-Regressionsprüfung

Stand 2026-10-27, nur statisch geprüft (kein Build, kein Test, kein
`cargo`). Formatierung je Datei mit `rustfmt --edition 2021` über stdin.

### Umgesetzter Quellstand

- `screen/core/host_metadata_jobs.rs`: `Job` besitzt einen Tokio-Task mit
  Frist von 15 s; Drop setzt ihn inaktiv und bricht ihn ab. Plattformarbeit
  läuft in `spawn_blocking`, nie unter `ScreenAuthority::admit`; Rechte und
  Aktivität werden vor und nach der Arbeit kurz geprüft. `spawn` (Fenster,
  Icon, Familie eines Viewers) teilt sich eine Lane pro Backend wie zuvor den
  Auth-Mutex; ein abgebrochener, schon laufender Job hält sie bis zum Ende.
  `spawn_start` (Hello-Metadaten, Quellwahl, Speicherfakten) wartet nicht
  auf diese Lane, wie diese Abfragen auch vorher nie auf Fensterlisten
  warteten.
- `screen/core/host_metadata.rs`: Abfragen, `Request`/`Requests` (FIFO der
  laufenden Sitzung, begrenzt auf die Kontrollframe-Queue), `Fallback`,
  `until` (Stopp → `Interrupted`, Viewer weg → `ConnectionAborted`) und
  `AuthorizedWriter`, der jeden einzelnen Transport-Schreibaufruf unter der
  Rechteprüfung des Modus ausführt. Metadatenframes prüfen zusätzlich den
  Sitzungsstopp; `HELLO_OK` prüft nur die Rechte und überlässt einen Stopp
  wie bisher der Sitzungsschleife.
- `icon_host.rs`, `family_listing.rs`: dieselben Listen-/Iconverträge ohne
  Plattformaufruf im Auth-Callback.
- `host_server.rs`, `host_server_start.rs`: Hello-Metadaten, Quellwahl und
  Speicherfakten kommen aus Jobs. Fehlschläge enden wie vorhandene Wege:
  Stopp oder Viewerende still (wie `Failure::Cancelled`), Rechteverlust als
  `permission_denied` bzw. `source_permission_revoked`, Frist oder
  Plattformpanik als `capture_failed` mit Protokolleintrag.
- `host_session.rs`, `host_session_control.rs`: laufende Viewer reihen
  Metadaten ein und pollen sie neben Capture, Ping und Kontrollfreigabe.

### Korrekturen gegenüber dem unterbrochenen Zwischenstand

1. Fehlerabbildung beim Start: Der Zwischenstand gab Metadatenfehler als
   nacktes `Err` zurück; Viewer erhielten weder Antwort noch Ablehnung, ein
   Stopp vor `HELLO_OK` endete als Fehler statt still.
2. `HELLO_OK` brach bei einem Sitzungsstopp ab, sodass der Viewer statt
   „beendet: Grund“ einen abgerissenen Handshake sah. Jetzt nur rechtegeprüft.
3. Metadatenkanal: Ein Stopp während Abfrage oder Versand lieferte `Err`
   statt des bisherigen `Ok`. Wiederhergestellt.
4. Icons: Frist oder Plattformpanik beendeten den Kanal bzw. die Sitzung.
   Wie jeder andere Plattform-Iconfehler wird das Icon jetzt ausgelassen
   (`content: None`); Rechteverlust und Validierungsfehler bleiben Fehler.
5. Fensterliste in laufender Sitzung: Rechteverlust beendete die Sitzung,
   bisher kam `ERROR capture_failed`. Wiederhergestellt; jeder Fehler der
   Liste ist wieder ein `ERROR`, die Sitzung läuft weiter.
6. Start-Abfragen hingen an der Viewer-Lane: Eine hängende Fensterliste
   hätte jeden neuen Sitzungsstart nach 15 s abgelehnt. Getrennt.
7. Tests: die Prüfung „Auth-Mutex frei“ per `try_lock` kollidierte im
   Mehrthread-Test mit der laufenden Capture-Rechteprüfung (unzuverlässig).
   Ersetzt durch eine Thread-Markierung „läuft innerhalb von `admit`“ und
   eine befristete Rechteänderung. Unbenutzte Importe entfernt, Dateien
   formatiert und unter 500 Zeilen gehalten; der Publikationstest liegt
   als Testmodul in `host_metadata.rs`.

### Statische Anti-Regressionsprüfung

- Autorisierung: Jede Plattformabfrage verlangt vor und nach der Arbeit
  dieselben Rechte wie zuvor (`ScreenView` für Listen, Icons und Familien,
  die Modusrechte für Start-Abfragen); `Ask` bleibt wie bisher zugelassen
  (`is_on`), Proben unter „Fragen“ erhalten weiter `HELLO_OK`. Jeder
  Schreibaufruf von Metadaten und `HELLO_OK` ist zusätzlich rechtegeprüft;
  das ist strenger als zuvor. Ein vergifteter Auth-Mutex wird weiterhin
  abgelehnt (`NodeAuthority::admit` → `false`).
- Ausführungsumgebung: kein `zbus::blocking` und kein Plattformaufruf mehr
  im Tokio-Task oder im Auth-Callback der geänderten Wege;
  `app_units::off_runtime` und `app_icons::portal_theme` bleiben unverändert.
- Funktionen: Fensterlisten (`bounded`), Icon-Autorisierung und
  Hash-/Größenprüfung, Familienlisten, Monitore, primäre Monitorwahl, Codecs,
  Eingabe, Limits, Speicheraufnahme, Kontrollübernahme und -freigabe,
  Indikator und Capture-Pfade sind inhaltlich unverändert. Die Reihenfolge
  der Metadatenantworten bleibt FIFO; Pong, Kontrollzustand und Bilder können
  ihnen jetzt vorausgehen.
- Plattformen: keine neuen `cfg`-Zweige in `screen/core`; der einzige
  Linux-Test steht in `screen/os/linux_os/app_icons.rs`.

### Nur durch Kompilierung und Remote-Suite zu belegen

Kompilierbarkeit (Linux, Windows, Android) und Clippy-Freiheit, das
Bestehen der Fälle `screen_metadata_runtime_task_*` (Testdatei
`screen_metadata_runtime_task_tests.rs`, Fixtures
`screen_metadata_runtime_fixtures.rs`, Publikationsfall in
`host_metadata.rs`, Linux-Fall in `app_icons.rs`) sowie das reale
Verhalten auf dem betroffenen Linux-Host sind offen.
