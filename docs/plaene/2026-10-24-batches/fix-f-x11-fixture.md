# M12 / F39–F40: Xvfb-Lebensdauer der Capturefixture

## A: Erstplan aus dem vollständigen Fixauftrag

Quelle ist Mains neuer vollständiger Fixauftrag M12/F39/F40 vom 2026-10-26,
gebunden durch `/tmp/smartexplorer-f-x11_fixture-scope.json` und den
unveränderten vollständigen Nutzervertrag
`/tmp/smartexplorer-final-review-user-contract.txt`. Der aktuelle Arbeitsstand
ist `515e8286 fix(recovery): preserve absent registry ownership fields [task
candidate]`; die Rohbefunde stammen vom Kandidaten
`1ffbf4b0bc90b2479947680abcd0a5941cce00f8`.

Beauftragtes Ergebnis: Der isolierte X-Server bleibt zwischen Clients für
dieselbe zusammenhängende Capture-/Resizeauswahl verfügbar. Die tatsächlichen
Capturebilder, Resizewirkungen, Speicherbesitzer und Frameerhaltung bleiben
ungekürzte Orakel. Derselbe Autor liefert Fehlerfunktionsliste, vollständige
direkte/indirekte Karte, vorherigen Erhaltungsplan, Fix und nachfolgende eigene
statische Anti-Regressionsreview.

Schreibbar sind ausschließlich `native/review-task-native.py` für die
Xvfb-Lebensdauer, die genannte Capture-Recoveryfixture und dieser Bericht.
Auswahl, Build-/Cachelogik, Messparameter, gemeinsame Manifestregistrierung,
Remote-Einstieg, Monitoring, Integration, Commit/Push, Graph und Release bleiben
bei Main. M03 liest denselben Runner für `SE_SCREEN_TASK_WORKER`, schreibt ihn
aber nicht. Keine produktive X11-Quelländerung ohne konkret belegte weitere
Freigabe. Keine lokalen Builds, Compiler, Tests, Formatter, Server,
Installationen, Unteragenten, Commits, Pushes, Graphupdates oder Releases.

Promptwissen: Das vorhandene Log belegt einen Verbindungsreset beim Aufbau,
keinen nachgewiesenen Serverabsturz. Ein Reset nach dem letzten Client ist
eine ausdrücklich noch ungeprüfte Hypothese. Konkrete Serverargumente,
Client-/Cleanup-Reihenfolge und unveränderte Fallorakel sind aus dem
freigegebenen Bestand und der Xserver-Primärquelle zu klären. Eine nur vom
Nutzer lieferbare Angabe ist bisher nicht erkennbar.

Vorläufige Planungstiefe A bis D: Rohbefunde vollständig auswerten;
Graph-/Runner-/Fixture-/Captureketten bis zu ihrer betroffenen Besitzgrenze
verfolgen; Xserver-Lebensdauer gegen Primärquelle abgleichen; daraus den
schmalen Fix mit Regressionserhalt festhalten; umsetzen und dieselbe Kette
anschließend selbst statisch prüfen. Die lokale Übergabe ist kein Laufzeitpass.

Dokumentationskontextgate vor Erstellung: gezieltes `git status --short` für
die drei Schreibpfade ohne Ausgabe; letzter Commit wie oben; gezielte
Themensuche in `rv1-funktionsplan.md` belegt die M12-Zuordnung und die
unbewiesene Reset-Hypothese. Weitere Bestandsbefunde folgen inkremental.

## B: Fehlerfunktionen, Belege und vollständige Verknüpfungskarte

Die vollständigen F39-/F40-Rohbefunde aus
`/tmp/smartexplorer-rv1-40406246032/native-failures.json` sind selbst gelesen.
Beide scheitern an `fixture_window()` beim `RustConnection::connect(None)`
mit OS-Fehler 104 / `ConnectionReset`, bevor Fenster, Bild, Encoder oder
Resize angelegt werden. Die protokollierten Compilerartefakte desselben
Kandidaten enthalten für diese beiden Befunde keinen Compilerfehler.

| Befund | Fehlerfunktion und Einordnung |
|---|---|
| F39 | `batches_task_b6_x11_hybrid_resize_releases_old_reference_before_retry → resize_with_encoder → fixture_window`: isolierter Displayaufbau scheitert. Kein nachgewiesener Fehler der Hybrid-Capture-/Speicherlogik; deren Orakel sind in diesem Aufruf noch nicht erreicht. |
| F40 | `batches_task_b6_x11_tiles_resize_releases_old_reference_before_retry → resize_with_encoder → fixture_window`: derselbe Setupfehler für Tile. Auch hier belegt das Log keinen Fehler des eigentlichen Resizepfads. |
| M12, Runnergrenze | `native_tests` startet einen isolierten Xvfb für den gesamten Libraryaufruf, sichert dessen Lebensdauer zwischen letzten Clientverbindungen aber bisher nicht gegen Serverreset. Die benötigte Option fehlt in `--server-args`; dies ist die begrenzte Konfigurationskorrektur. Welches konkrete Serverereignis die beobachteten Verbindungsresets ausgelöst hat, bleibt ohne Serverereignisaufzeichnung unbewiesen. Ein Serverabsturz wird nicht behauptet. |

Eigene Funktionsauflistung: Die Anpassung betrifft ausschließlich
`native/review-task-native.py::native_tests`. Die beobachtete Fehlerstelle
ist `fixture_window`, die beiden oben vollständig benannten Fälle verbrauchen
`resize_with_encoder`. Dessen `request`, `capture_image` und `WindowOwner::drop`
bilden die unveränderte Fixturekette. Keine produktive X11-Funktion ist durch
F39/F40 als defekt nachgewiesen.

Die vollständige direkt und indirekt verkettete Karte innerhalb der
freigegebenen Grenze:

1. `.github/workflows/review-task.yml`, Linuxjob auf `ubuntu-24.04`, stellt
   `xvfb`/`xauth` bereit und ruft denselben nativen Suiteeinstieg auf.
   `native/test-review-task.sh --stage native` bindet Checkout und
   Kandidaten-SHA, Logwurzel und begrenzte Cargoentwicklungseinstellungen;
   danach startet es denselben Pythonrunner. Diese Pfade werden nur gelesen.
2. Runner `main → artifact/fingerprint` erhält exakt dieselben geprüften
   Entwicklungsbinaries und temporären Profile. Die erfolgreiche Library-
   und CLI-Bereitstellung bindet weiterhin `SE_SCREEN_TASK_WORKER` vor
   `native_tests`. Der geänderte Xvfb-Parameter verändert keinen dieser Wege.
3. `native_tests → manifest → run(binary --list)` entdeckt echte Fallnamen,
   ergänzt genau die bisherigen Integrationen, prüft Pflichtmarker und
   schreibt dieselbe Auswahl. Der Libraryaufruf behält `--include-ignored`,
   `--show-output`, `--test-threads=1`, Filter und Ausschlüsse.
4. Nur Linux umschließt diesen einen Aufruf mit `xvfb-run --auto-servernum`.
   `fixture_env` bleibt eine Kopie mit `WAYLAND_DISPLAY` entfernt und
   `SE_SCREEN_TASK_X11=1`, `XDG_SESSION_TYPE=x11`; die Workerbindung bleibt
   in dieser Kopie. Der Server behält Bildschirm `1280x720x24`, TCP-Sperre
   und eigenes Fehlerlog. `run` erhält denselben Rückgabecode, dieselbe
   7200-Sekunden-Frist und die vorhandene Beendigung des eigenen Prozessbaums
   bei Unterbrechung oder Fristablauf. Der Windowszweig bleibt unverändert.
5. Beide bestehenden `#[ignore]`-Fälle werden durch diesen Suiteaufruf
   eingeschlossen. Jeder durchläuft weiter sowohl GetImage als auch SHM.
   `fixture_window` verlangt die Suiteumgebung, verbindet einen echten
   Client, prüft die tatsächliche Rootgröße, erzeugt/mappt ein eigenes
   Override-Redirect-Fenster mit bekannter Hintergrundfarbe und synchronisiert
   den Request. `WindowOwner` hält diese Verbindung bis zur Fensterzerstörung.
6. `resize_with_encoder → capture_memory::plan → MemoryPool::admit_session`
   begrenzt die Sitzung weiterhin auf genau den berechneten neuen Peak.
   `HostOutputPlan`, Legacy-/Versioned-Vertrag und `request` behalten
   Arbeitsbudget, Zielrate, FPS, Ausgangsgröße und Codec. Keine Buchung wird
   ersetzt, vergrößert oder synthetisch reduziert.
7. `X11Source::prepare → AdmissionStream::connect → aim/current_area/
   note_geometry` bindet denselben realen Quellbereich. `activate → plan/verify`
   prüft Identität, Generation und Kapazität. Die Fixture verlangt weiterhin
   tatsächlich verfügbares MIT-SHM im SHM-Durchlauf, deaktiviert nur den
   Cursor und den für diese unbedeckte Rohfensterfixture unnötigen Compositor.
   Produktive Familienfenster behalten ihre eigene Pixmapanforderung.
8. `capture_image → ScreenSource::capture → pass → pass_window →
   admit_geometry → read` liefert ein wirklich erfasstes Bild. `read` verwendet
   unverändert entweder `ensure_segment → Segment::create → shm_get_image`
   mit getrenntem NativeSurface-/SourcePixelsbesitz oder den sequenzgebundenen
   `Gate::image → GetImage → Gate::finish`-Weg mit Raw-/Parserzulassung.
   `CaptureImageBuilder::freeze`, Hash und Pixelvergleich bleiben echt.
9. `AdmittedTilesEncoder::new → host_hybrid::wrap_captured → encode_captured`
   erzeugt weiterhin wirkliche Encoderrecords und persistenten Besitz.
   Die Fixture verwirft nur bereits überprüfte erste Records und den ersten
   Bildgriff; der Encoder bleibt absichtlich Besitzer des alten Grants.
10. Reales `configure_window` und ein bestätigter Synchronisationsrequest
    ändern danach die X11-Geometrie. `pass_window → admit_geometry` entfernt
    alte Source-/SHM-/Cachebesitzer, retiert den alten Nichtmitgliedsgrant,
    publiziert Referenz und Generation vor der fehlbaren neuen Zulassung.
    Das unverändert enge Budget muss mit dem noch lebenden Encoder
    `WouldBlock` liefern. Weder alter noch neuer Eingabereferenzwert ohne
    zugehörige Pixel darf Eingaben freigeben.
11. Der zweite Captureversuch behält dieselbe wartende Referenz und Generation;
    Keyframe hebt lebende Speicherbesitzer nicht auf. Erst der echte
    `drop(encoder)` darf alte `live_bytes` auf null bringen. Danach müssen
    frische Pixel in neuer Größe, derselbe SourceKey und die wartende Generation
    erscheinen. Der letzte unveränderte Capture muss `SourceFrame::Unchanged`
    liefern. Alle diese Assertions bleiben vollständig erhalten.
12. Die nachfreigegebenen produktiven Verbraucher bestätigen den Sinn dieses
    Orakels: `host_capture::produce/capture_and_encode` verwirft den alten
    Stageencoder bei geänderter Sourcereferenz vor der Ergebnisverarbeitung.
    `Worker::produce` setzt bei geändertem Vorher-/Nachherwert `encoder=None`
    auch nach `WouldBlock`; `Worker::prepare_reply` entfernt alten Encoderbesitz
    erneut, bindet echte Bilder an ihre ursprüngliche Referenz und behält
    konkrete Pausen-/Ablehnungsgründe. `Pipeline::sync_source_reference →
    invalidate_reference → drop_encoder` hält Präsentationsboden, Stream,
    Encoder und Eingabegeometrie konsistent. Keiner dieser Verbraucher wird
    verändert oder stellvertretend durch die Fixture implementiert.
13. `X11Source::drop`, `Segment::{release,drop}` und `WindowOwner::drop` behalten
    Fenster-, Verbindungs-, Mapping- und Grantabschluss. Bei unsicherem nativen
    Abschluss bleibt der bestehende `Gate::retain`-Besitz wirksam. Zwischen
    den vollständig beendeten Fixtureclients muss nur der einmal gestartete
    Suite-X-Server verfügbar bleiben; ein zusätzlicher globaler Clientbesitzer
    wird nicht eingeführt.
14. Runnerauswertung bleibt `run → native-suite.log → Erfolgszeile je Auswahl
    → milestone_evidence/quality_evidence → native-acceptance.json`.
    Fehlende oder fehlgeschlagene Fälle werden weiter abgewiesen;
    environmentabhängige Fälle bleiben separat ausgewiesen. Beide konkreten
    Resizefälle stehen im Ressourcenmanifest und im Linuxpflichtmarker.
    Die gemeinsame Registrierung und Ergebnisbewertung bleiben bei Main.

Graphify wurde vor der Quellkarte nur lesend verwendet: aus dem Graphvokabular
gewählt wurden `capture resize recovery reference fixture memory`.
Die breite BFS-Abfrage war zu allgemein; `graphify explain resize_with_encoder`
belegt anschließend acht extrahierte Kanten bei der Fixturefunktion, darunter
`fixture_window`, `capture_image`, `request` und beide genauen Testaufrufer.
Der Pythonrunner ist damit nicht graphseitig bewiesen; seine Kanten stammen
aus der vollständigen Quellenlektüre. Kein Graph wurde geschrieben oder
aktualisiert. Die Abfrageausgabe enthält keinen gemessenen Tokenverbrauch;
das gesetzte Ergebnisbudget betrug 1600 Tokens.

## C: Primärquelle und begründete Auswahl

Das [X.Org-Xserver-Handbuch](https://www.x.org/archive/current/doc/man/man1/Xserver.1.xhtml)
wurde am 2026-10-26 nach dem XHTML-Fehler des Webabrufs direkt per HTTPS
gelesen. Es dokumentiert `-noreset` gegen den Reset nach Schließen der letzten
Clientverbindung; ein explizites SIGTERM beendet den Server weiterhin sauber.
Das ist der für die gemeinsame Fixture benötigte Vertrag. Die Protokollfehler
belegen weder einen SIGTERM noch einen Serverabsturz oder den exakten
Resetzeitpunkt.

Gewählt ist die Serveroption am bestehenden gemeinsamen `xvfb-run`-Aufruf.
Ein zusätzlich gehaltener Kontrollclient könnte ebenfalls letzte
Clientverbindungen vermeiden, hätte aber eigenen Verbindungs-/Fehler- und
Cleanupbesitz und wäre für den vorhandenen Wrapper unnötig. Ein Produktretry
oder ein geschwächtes Fixtureorakel ist aus diesen Setupbefunden nicht
begründet. Es wird weder ein eigener X-Serverwrapper noch eine neue Suite
eingeführt.

## D: Vorheriger konkreter Fixplan mit Erhaltungsvertrag

1. In `native_tests` ausschließlich `-noreset` an die vorhandenen
   Xvfb-Serverargumente anhängen und die Lebensdauer kurz am bestehenden
   Kommentar erklären. Erwartetes Ergebnis: derselbe isolierte Server bleibt
   zwischen Fixtureclients verfügbar; Rootgröße, Farbtiefe und TCP-Sperre
   sind unverändert. Keine Auswahl-/Frist-/Cache-/Build-/Workeränderung.
2. Die vollständige Capturefixture und die produktiven X11-/Host-/Workerquellen
   unverändert lassen. Erwartetes Ergebnis: reale SHM-/GetImagepfade,
   Hybrid-/Tileencoder, echte Resizes, genaue Budgets, geänderte Referenzen,
   verweigerte Eingabe und echte Pixel-/Unchangedorakel bleiben erhalten.
3. Nach dem Edit eigene statische Anti-Regressionsreview: exakten Runnerdiff
   und Python-AST prüfen, alle unberührten Runnerfunktionen vergleichen,
   Fixture und gelesene Verbraucher gegen `515e8286` bytevergleichen sowie
   Fallregistrierung und Fehlerweitergabe prüfen. Kein Runnerimport, kein
   lokaler Compiler, Test oder Server. Diese Prüfung ersetzt den Remotebeleg
   nicht.
4. Main erhält dieselben beiden vorhandenen Fallnamen. Die verbleibende
   Laufzeitfrage ist, ob beide unter derselben bereits bestehenden Remote-Suite
   die Setupgrenze passieren und ihre vollständigen Resize-/Capture-/
   Speicherorakel erfüllen. Kein zusätzlicher Suiteeinstieg oder lokaler
   Vorabtest; Registrierung und Auswertung bleiben bei Main.

## E: Umsetzung und eigene anschließende Anti-Regressionsreview

Umgesetzt ist ausschließlich `-noreset` als zusätzlicher Serverparameter in
`native_tests`, mit einem Kommentar zur Lebensdauer zwischen Fixtureclients.
Die Capturefixture wurde nicht bearbeitet. Es gibt keine neue native Quelle,
keinen produktiven X11-Umbau und keine Änderung am gemeinsamen Manifest.

Nach der Änderung hat derselbe Autor den kompletten Runnerdiff und die
vorstehende Kette selbst statisch gegen `515e8286` geprüft:

- Der Bytevergleich des Runners entspricht exakt dem bisherigen Inhalt mit
  ausschließlich der Kommentarzeile und dem zusätzlichen Serverparameter.
  Die Python-AST ist erfolgreich geparst. Nach Normalisierung genau dieses
  einen Stringliterals sind beide vollständigen ASTs gleich. Der Runner wurde
  nicht importiert oder ausgeführt; damit wurden weder Tests noch Builds
  gestartet. Auswahl, Builds, Cache, Workerbindung, Messwerte, Fristen,
  Auswertungen und Windowszweig sind strukturell unverändert.
- Bildschirm `1280x720x24`, `-nolisten tcp`, Auto-Displaywahl, Authority-/
  Serverbesitz des bestehenden `xvfb-run`, Fehlerlog und Prozessbaumbeendigung
  bleiben erhalten. Es gibt keine neue Retryschleife, keinen zusätzlichen
  Serverprozess und keine als Erfolg umgedeutete Verbindungsablehnung.
- Die komplette Capturefixture ist bytegleich: reale Fensteranlage,
  `configure_window`, Synchronisation, MIT-SHM-Pflicht, beide Producer,
  tatsächlicher Encoderbesitz, genau berechnete Kapazität, alle `WouldBlock`-
  Erwartungen, Referenz-/Generationsprüfungen, verweigerte Eingabe,
  `drop(encoder)`, `live_bytes==0`, Pixelwert und `Unchanged` bleiben erhalten.
  Fallnamen, `#[test]`, bestehendes `#[ignore]`, Schleifen und sämtliche
  Assertions wurden weder verändert noch entfernt.
- Die sechs gelesenen X11-Capturedateien, `host_pipeline`, `host_reference`,
  `host_capture`, `host_worker_produce` und `host_worker_run` sind vollständig
  bytegleich zur Basis. Der nachgeprüfte tatsächliche Encoderabschluss bleibt
  bei geänderter Referenz vor dem nächsten Zulassungsversuch wirksam; kein
  Speicherzähler oder Bildbesitzer wird durch diese Runneränderung ersetzt.
- Workflow, Shellsuiteeinstieg und gemeinsames Manifest sind ebenfalls
  bytegleich zur Basis. Beide bestehenden Fallnamen bleiben über den
  bisherigen Auswahlfilter und im Ressourcenmeilenstein registriert.
  Nicht erreichte oder fehlgeschlagene Fälle müssen weiterhin fehlschlagen.
- Statischer Diff-/Whitespacecheck ohne Befund. Die unveränderte Rustfixture
  umfasst 250 Zeilen und 8.685 Bytes; es entsteht kein neues oder wesentlich
  bearbeitetes Rustmodul. Die Größenregel erfordert keine Extraktion.

Diese eigene Review bestätigt den schmalen Quellen-/Vertragserhalt. Sie
behauptet keinen ausgeführten Xvfb-/Resize-/Capturepass und keine nachträglich
bewiesene konkrete Resetursache des alten Logs.

## F: Vorhandene Abnahmefälle und offene Laufzeitfrage

Die unverändert beauftragten Fälle heißen vollständig:

- F39: `screen::os::x11_capture::recovery_tests::batches_task_b6_x11_hybrid_resize_releases_old_reference_before_retry`
- F40: `screen::os::x11_capture::recovery_tests::batches_task_b6_x11_tiles_resize_releases_old_reference_before_retry`

Beide verbleiben im bestehenden Libraryaufruf unter dem vorhandenen
`batches_`-Filter; `batches_task_b6_x11_` bleibt Linuxpflichtmarker.
Das gemeinsame Manifest registriert beide wörtlich im bestehenden
Ressourcenmeilenstein `R13-RSOL11-X11-Capture`. Es ist kein neuer Fall und kein zusätzlicher
Suiteeinstieg erforderlich. Main besitzt Registrierung, Kandidatenbindung,
Ausführung und Auswertung derselben Remote-Suite.

Noch unbestätigte Laufzeitfrage: Beide Fälle müssen mit derselben neuen
Serveroption die Clientwechsel passieren und anschließend ihre unveränderten
Tile-/Hybrid-, GetImage-/SHM-, Resize-, Referenz-, Speicher-, Eingabe- und
Pixelprüfungen erfüllen. Ein weiterhin fehlgeschlagener Aufbau wäre erneut
ein tatsächlicher Fehler, kein erlaubter Skip. Keine fehlende Quellfreigabe
bleibt offen; die drei nachgeforderten Host-/Workerdateien wurden von Main
lesend ergänzt und vom selben Autor vollständig untersucht.

## G: Vollständige Lese-/Änderungsliste und Übergabe

In dieser Aufgabe tatsächlich gelesen:

- `.github/workflows/review-task.yml`, `native/test-review-task.sh`,
  `native/review-task-native.py` vollständig; gemeinsames
  `native/review-task-manifest.json` an Auswahl-, Pflichtmarker- und
  Ressourcenregistrierung.
- `docs/ARCHITEKTUR.md` an der Bildschirm-/Bildreferenzgrenze,
  `docs/plaene/2026-10-24-batches/rv1-funktionsplan.md` an M12 und der
  Xserver-Quellenbegründung,
  `docs/plaene/2026-10-24-batches/rv1-funktionsbefunde.json` an F39/F40,
  dieser Bericht.
- `native/src/screen/os/linux_os/batches_capture_recovery_task_tests.rs`,
  `native/src/screen/os/linux_os/x11_capture.rs`,
  `native/src/screen/os/linux_os/x11_capture_memory.rs`,
  `native/src/screen/os/linux_os/x11_capture_read.rs`,
  `native/src/screen/os/linux_os/x11_capture_shm.rs`,
  `native/src/screen/os/linux_os/x11_capture_source.rs`,
  `native/src/screen/os/linux_os/x11_capture_target.rs` vollständig.
- `native/src/screen/core/host_pipeline.rs`,
  `native/src/screen/core/host_reference.rs`,
  `native/src/screen/core/host_capture.rs`,
  `native/src/screen/os/shared/host_worker_produce.rs`,
  `native/src/screen/os/shared/host_worker_run.rs` vollständig.
- `graphify-out/graph.json` für Vokabular und die beiden lesenden Abfragen;
  `/root/.codex/skills/graphify/SKILL.md` und
  `/root/.codex/skills/graphify/references/query.md` am vorhandenen
  Read-only-Query-/Explainablauf. Der bereitgestellte CLI war verwendbar;
  die fehlende `graphify-out/.graphify_python` wurde gemäß Schreibverbot
  nicht angelegt. Kein Update, Reflect, Save-result oder Graphaufbau.
- `/tmp/smartexplorer-f-x11_fixture-scope.json`,
  `/tmp/smartexplorer-rv1-40406246032/native-failures.json` an den vollständigen
  F39-/F40-Diagnosen und ihrer Kandidaten-/Compilerzuordnung. Die unveränderten
  bereits geladenen Projektregeln und der vollständige Nutzervertrag bleiben
  verbindlich; die Arbeitsweise wurde über den lokalen MCP für diesen Auftrag
  geladen. X.Org-Primärquelle und Abrufweg stehen in C.

Geändert: ausschließlich `native/review-task-native.py` im freigegebenen
Xvfb-Lebensdauerargument und dieser neue Bericht
`docs/plaene/2026-10-24-batches/fix-f-x11-fixture.md`. Keine Rustfixture,
Produktquelle, Manifest-, Build-/Cache-, Mess- oder Freigabelogik geändert.

Entscheidung: Lebensdauer des bestehenden isolierten Servers sichern und alle
tatsächlichen Produkt-/Speicherorakel erhalten. Keine offene Scopeabhängigkeit;
die konkrete historische Resetursache und der neue Funktionspass sind weiterhin
nur im Remote-Ergebnis bestätigbar. Keine lokalen Builds, Compiler, Tests,
Formatter, Server, Installationen, Unteragenten, Commits, Pushes, Graph- oder
Releasearbeit. Die eigene statische Abgabe endet mit der Übergabe an Main.
