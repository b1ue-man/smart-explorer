# M07/F57: Frische P27-Netzprofile

Stand: 2026-10-26. Eigener Fixautor: `fix_p27_shutdown`.
Arbeitskopie: `/tmp/smartexplorer-review-fixes-ee75e91b`, Branch
`codex/review-fixes-ee75e91b`. Fehlerkandidat:
`1ffbf4b0bc90b2479947680abcd0a5941cce00f8`. Beauftragter Arbeitsstand:
`33257f82`; beim Dokumentationskontextgate ist der gemeinsame Branch bereits
bei Mains unabhängigem Meilensteincommit `515e8286`. Die beiden eigenen
Schreibpfade sind dabei unverändert.

## A: Erstplan aus dem Ergebnisauftrag

Quelle: vollständiger M07/F57-Auftrag an diesen Autor,
`/tmp/smartexplorer-f-network_profiles-scope.json`, vollständiger wörtlicher
Nutzervertrag `/tmp/smartexplorer-final-review-user-contract.txt` und M07 im
abgeschlossenen `rv1-funktionsplan.md`.

Ziel: Alle drei unveränderten P27-Netzprofile erhalten nacheinander passende
frische Qdiscs und Portfilter im selben eigenen Netzwerk-Namespace.
Beauftragt sind die eigene Fehlerfunktionsliste, vollständige direkte und
indirekte Verknüpfungskarte der betroffenen Grenze, konkreter vorheriger
Fixplan, Umsetzung und eigene anschließende Anti-Regressionsreview.

Schreibrecht besteht ausschließlich für `native/review-task-network.sh` und
diesen Bericht. Messkörper, Grenzwerte und sicherer Aufgabenabschluss werden
nicht geändert. Main besitzt Manifest, Remote-Ausführung, Graphrefresh,
Integration, Commits, Push und Release. Kein lokaler Build, Compiler,
Formatter, Test, Server, Paketinstallation, langer Prozess oder Unteragent
ist erlaubt. Die früheren Scopefreigaben sind für diesen Auftrag abgelöst.

Promptwissen: Die erste Referenz und Bildszene wurden abgeschlossen;
danach scheitert der Profilaufbau mit einem `tc`-Exklusivitätsfehler.
Noch zu klären sind der genaue Mutationspfad, der Erhalt alter Filter und
der kleinste vollständige Reset. Diese Angaben sind durch den freigegebenen
Bestand und die genannten Primärquellen klärbar. Eine zusätzliche
Nutzerangabe ist nicht nötig. Planungstiefe zwei folgt aus der gekoppelten
Qdisc-/Filter-/Messlebensdauergrenze. Der erste eigene Arbeitspunkt ist die
gemeinsame Auswertung von Helper, Profilaufrufern und Fehlerlog; anschließend
werden die Kernel-/CLI-Verträge vor dem Edit geprüft.

## B: Belegte Fehlerfunktion, Quelle und Stufe-eins-Plan

Die fehlerhafte Stelle ist ausschließlich der Zweig `profile` in
`native/review-task-network.sh`: Er verwendet erneut Root-Handle `1:` mit
demselben Typ `prio` und danach exklusive `add`-Aufrufe für `10:` und `20:`.
Bei gleichem Handle bleibt der Root im Änderungsweg; seine bestehenden
Netem-Kinder und seine Portfilter sind damit nicht frisch aufgebaut.
Das erste nachfolgende `qdisc add` trifft auf das vorhandene Kind `10:`.

Das abgeschlossene Log
`/tmp/smartexplorer-rv1-40406246032/linux/p27-network.log` meldet exakt
`Exclusivity flag on, cannot modify.` aus `transport::network`.
Der Kernel gibt diese Diagnose im vorhandenen-Qdisc-/Exklusivpfad aus.
Die Bild- und Referenzdatei für `10mbit-one-window` existieren und enthalten
jeweils fünf Sekunden Warm-up sowie zwölf Sekunden stationäre Beobachtung.
Der Fehler ist damit ein belegter Fixture-Profilwechselfehler. Ein
Produktleistungsfehler oder ein bestandenes V7-Leistungsresultat folgt aus
diesen beiden Rohdateien nicht; der Gesamtbericht wurde nicht abgeschlossen.

`transport::network` selbst behandelt den fehlgeschlagenen Prozess korrekt
als Fehler. Die Reihenfolge in `p27_network_reference_acceptance` ist ebenfalls
korrekt: je Profil neuer Relayport, Konfiguration, Referenz, Snapshot,
Bildszene, Snapshot, Auswertung und Relayabschluss. Der Namespace bleibt
dabei absichtlich für die gesamte Fixture bestehen. Der erste notwendige
Plan ist deshalb ein vollständiger Austausch des eigenen Profilroots
einschließlich aller Kinder und Portfilter vor dem nächsten Profil.

## Eigene direkte und indirekte Verknüpfungskarte

Die folgende Karte umfasst die durch F57 betroffene Definitions-, Aufrufer-,
Plattform-, Prozess- und Fixturegrenze. Sie bezeichnet unmodifizierte
Produktiv-APIs als angeschlossene Grenzen, ohne deren außerhalb dieses
Scopes liegende Implementierungen neu zu bewerten.

1. `.github/workflows/review-task.yml` ruft den vorhandenen gemeinsamen
   Einstieg `native/test-review-task.sh` auf. Dieser bindet Checkout,
   Kandidaten-SHA, Stage und Artefaktverzeichnis und delegiert an
   `review-task-native.py`. Workflow und Einstieg bleiben Main vorbehalten.
2. `review-task-native.py::p27_network_tests` liest
   `native.p27_network` aus dem Manifest, verlangt genau die vorhandene
   ignorierte Fixture im Developmentbinary und startet ausschließlich den
   Helpermodus `exec` mit deren exaktem Namen. Die Linux-Netzstufe hat
   weiterhin 1.800 Sekunden Zeitbudget; Windows führt sie nicht aus.
3. Helper `exec` prüft die Werkzeugverfügbarkeit und nichtinteraktiven
   Rootzugang. Die einzige Schreibseite seiner anonymen Lebensdauerpipe
   gehört dem Runnerprozess. EOF und Signalbehandlung erreichen den
   Root-Supervisor; daran ändert der Profilfix nichts.
4. Helper `supervise` bindet den Unshareprozess über Parent-Death-Signal
   und Pipe an den Besitzer und erzeugt eigene Netz-, PID- und
   Mount-Namespaces. Der Tod von PID 1 beendet deren Prozessbaum.
   `inside` verlangt PID 1, eine vom Parent verschiedene Netznamespace und
   ausschließlich Loopback; er setzt unverändert MTU, Offloads und
   Umgebungsidentitäten. Der vorhandene EXIT-Trap räumt den eigenen Root auf.
5. Das darin laufende `p27_network_reference_acceptance` erstellt in seiner
   unveränderten Dreierfolge den privaten Relay und entdeckt dessen
   tatsächlichen HTTPS-Port. `transport::network("profile", [port, bps])`
   startet eine neue Helperinstanz innerhalb derselben Namespace und
   übernimmt ausschließlich gültiges JSON aus einem erfolgreichen Prozess.
6. Der gemeinsame Helperzweig `profile|snapshot` prüft zunächst tatsächliche
   Namespaceidentität, abweichenden Parent, ausschließlich Loopback,
   Argumentanzahl, TCP-Portbereich und die beiden erlaubten Bitraten.
   Ausschließlich `profile` darf die Qdiscstruktur verändern. Der neue
   vorbereitete Reset liest dessen aktuellen Rootzustand direkt aus dem
   Kernel; es gibt keine Profilmarkierungsdatei und keinen alten Relayport
   als implizite Voraussetzung.
7. Der eigene Root `1:` besitzt die beiden Netem-Kinder: `10:` auf Band
   `1:1` begrenzt ausschließlich Verkehr zum Relay; `20:` auf Band `1:2`
   verzögert den Rückweg ohne zweite Ratenbegrenzung. Beide Legs behalten
   jeweils 5 ms Delay und die bestehende Queuegrenze. Zwei TCP-Portfilter
   ordnen Zielport beziehungsweise Quellport des aktuellen Relays zu.
   Vollständiges Root-Delete entfernt diese Kinder und Filter gemeinsam;
   anschließendes Root-/Kind-/Filter-Add erzeugt den aktuellen Zustand neu.
8. Die gemeinsame JSON-Ausgabe liest Qdiscs, Filter, Offloads, Links,
   PID-/Netzidentitäten, Routen und tatsächliche TC-Version. Die bisherigen
   Assertions für Rate, zwei Netem-Legs, Verzögerung, Queuegrenze und
   Flowzuordnung bleiben erhalten. `snapshot` verwendet genau diesen
   Leseweg, ohne den neuen Reset zu betreten.
9. `transport::reference` verwendet echte private Relayverbindungen,
   Kontrollproben und QUIC-Nutzbytes; `screen` verwendet den bestehenden
   PipelineActor-/HostStage-/Wire-/ACK-Pfad. Beide verwenden weiterhin
   `Tasks::finish` vor `Pair::close`. Diese selbst gelesenen
   Aufgaben-/Verbindungsgrenzen werden nicht verändert. Der Profilreset
   liegt zwischen den Profilen, niemals zwischen Referenz und Bildszene
   desselben Profils.
10. `transport::assert_shaped` verlangt für beide Handles echte zusätzliche
    Pakete gegenüber dem vorherigen Snapshot. Nach frischem Profilaufbau
    gehören die vergleichbaren Zähler zu genau diesem Profil. Die Messung
    übernimmt unverändert reale Beobachtungen in
    `batches_rate_reference_metrics.rs::evaluate` und schreibt die sechs
    Rohdateien sowie nach der vollständigen Dreierfolge `acceptance.json`.
11. `p27_network_tests` bindet das Ergebnis an Kandidat und Fixture, vergleicht
    alle drei Profile mit dem Manifest, verlangt deren unveränderten Erfolg
    und jede Rohdatei und erzeugt den vorhandenen P27-Meilensteinbeleg.
    Es gibt keinen neuen Auswahlfilter und keinen zusätzlichen Suiteeinstieg.

Der vorhandene AST-Graph wurde nur lesend abgefragt. Die bestätigten
Graphvokabeln waren `network`, `reference`, `acceptance`, `transport`,
`profile` und `shaped`. `graphify query` ordnet `network()`, `reference()`,
`assert_shaped()`, `Profile`, die Aufgabenhilfe und Transport-/Metrikdateien
zu; die Bash-Mutationssemantik ist damit nicht bewiesen und wurde direkt
im Source und in Primärquellen geprüft. Die CLI meldet Skill 0.9.11 gegenüber
Paket 0.9.17. `.graphify_python` fehlt, aber die vorhandene CLI-Abfrage
funktioniert. Wegen des ausdrücklichen Schreib-/Installationsverbots wurde
kein Interpretermarker, Vokabularfile, Reflection, Save-result oder Update
erzeugt. Der Graph ist kein Laufzeitnachweis.

## C: Quellenrecherche und Entscheidung

Primärquellen tatsächlich am 2026-10-26 abgerufen:

- [Linux 6.17, `sch_api.c`, Qdisc-Modifikation](https://github.com/torvalds/linux/blob/v6.17/net/sched/sch_api.c#L1501-L1624),
  nach einem unbrauchbaren GitHub-Rawlink über die
  [versionsgleiche Rohquelle](https://raw.githubusercontent.com/torvalds/linux/v6.17/net/sched/sch_api.c)
  gelesen: gleicher Handle bleibt im Änderungszweig; Exklusiv-Add auf den
  vorhandenen Qdisc liefert `EEXIST` und die beobachtete Diagnose. Ein
  eingebauter Default mit Handle null wird für Neuanlage nicht als
  konfigurierter Qdisc behandelt.
- [`tc(8)`, Syntax und TC COMMANDS](https://man7.org/linux/man-pages/man8/tc.8.html):
  `show ... root` liest den Root, `delete ... root` entfernt dessen
  Unterstruktur einschließlich angehängter Filter, und `add` erzeugt den
  anschließend gewünschten Zustand. Die allgemeinen Kurzangaben zu
  `replace` ersetzen den konkreten gleichnamigen Kernelpfad nicht.

Gewählt: Den tatsächlichen aktuellen Rootzustand lesen; ausschließlich den
bekannten eigenen `prio`-Root mit Handle `1:` gecheckt löschen; Root,
Netem-Kinder und TCP-Portfilter anschließend frisch per `add` installieren.
Der erste Lauf auf der leeren eigenen Namespace benötigt kein Delete eines
eingebauten Defaultroots. Ein unbekannter Root wird nicht ausdrücklich
gelöscht; das anschließende exklusive Add darf daran scheitern. Lese-,
Lösch- und Addfehler bleiben Fehler.

Verworfen: nur `add` durch `replace` für beide Netem-Kinder ersetzen, weil
alte Portfilter und Zustände dabei erhalten bleiben können; gesondertes
Filternachführen, weil es die zusammengehörige Resetgrenze vervielfacht;
eine neue Namespace je Profil, weil dies den bestehenden Prozessbesitz
unnötig umbaut; unbedingtes Delete mit pauschalem `|| true`, weil es echte
Fehler verschweigt. Messfenster, V7-Schwellenwerte und Messkörper sind keine
Reparaturfläche dieses Befunds.

## D: Finaler Detailplan vor dem Helperedit

| Meilenstein | Datei und Abhängigkeit | Erwartetes Signal im bestehenden Remote-Einstieg |
|---|---|---|
| M07-1: Eigenen alten Profilbaum erkennen und vollständig entfernen | `native/review-task-network.sh`, nach den vorhandenen Namespace-/Argumentprüfungen ausschließlich in `profile` | Der erste Defaultroot braucht kein Delete; nach einem bisherigen Profil verschwinden dessen eigene Kinder und Portfilter gemeinsam. Unerwartete Lese-/Löschfehler beenden das Setup. |
| M07-2: Frische vollständige Konfiguration erzeugen | Derselbe Zweig; vorhandene Addparameter, Raten, Delays und Filter | `10mbit-one-window` → `1mbit-one-window` → `10mbit-four-windows` erhalten jeweils den aktuellen Relayport mit passender Rate und zwei verzögerten Legs. Kein alter Portfilter und kein alter Zählerzustand werden weiterverwendet. |
| M07-3: Eigene anschließende Anti-Regressionsreview | Dieser Bericht und vollständiger eigener Helperdiff; alle gelesenen Fixture-/Runnergrenzen | `snapshot` ist unverändert lesend, Namespace-/PID-/Signalbesitz bleibt erhalten, Aufgabenabschluss und alle V7-Orakel bleiben bytegleich. Exakte vorhandene Netzfallkennung genügt; keine zusätzliche Registrierung. |

Die zweite Lückenprüfung vor dem Edit klärt insbesondere: Der Zustand wird
bei jedem Profil aus `tc -j qdisc show dev lo root` ermittelt; eine
Markierungsdatei oder Kenntnis des vorherigen Ports ist unnötig. Die
bereits im Helper verwendeten JSON-Felder `handle` und `kind` genügen.
Delete wird ausschließlich für den gefundenen eigenen Typ/Handle ausgeführt
und erhält `check=True` sowie ein begrenztes Unterprozesszeitbudget.
Es gibt kein Fehlerignorieren im normalen Profilaufbau. Die vorhandenen
Shapingprüfungen und Paketzählervergleiche beginnen erst nach vollständiger
Neuinstallation. Vor der eigenen späteren Review sind deren Fragen
festgelegt: Reset nur vor Profilen, vollständiger Baum statt einzelner
Kinder, richtiger dynamischer Port, unveränderte Messwerte und Fehlerwege,
unveränderte sichere Prozess-/Namespace-Aufräumgrenze.

## Umsetzung und eigene anschließende Anti-Regressionsreview

M07-1 bis M07-3 sind als Quelländerung und statische Autorprüfung vorbereitet.
`profile` liest nun direkt vor der Neuinstallation den Rootzustand mit
`tc -j qdisc show dev lo root`. Ausschließlich der bekannte eigene
`prio`-Root `1:` wird über ein gechecktes, zeitlich begrenztes Delete
entfernt. Anschließend verwendet der bisherige Rootaufbau `add`;
Kind-Qdiscs und die beiden Portfilter werden mit exakt denselben bisherigen
Parametern erzeugt. Der Reset emittiert kein zusätzliches JSON und
verändert damit den bisherigen Stdoutvertrag des Helpers nicht.

Die nachfolgende Review hat derselbe Fixautor nach dem Helperedit
durchgeführt; sie ersetzt weder Kernel-Ausführung noch Leistungsmessung.

| Geprüfte Grenze | Eigener anschließender Befund | Beleg und verbleibende Grenze |
|---|---|---|
| Erster Profilaufbau | Ein Defaultroot wird nicht ausdrücklich gelöscht. Exklusives Add erzeugt den bisherigen `prio`-Root neu. | Direkter Helper-/Kernelvertragsabgleich; tatsächlicher Runneraufbau bleibt remote. |
| Zweiter und dritter Profilaufbau | Der bekannte alte Root wird samt Netem-Kindern und angehängten Portfiltern entfernt. Die Neuinstallation verwendet aktuelle Rate und aktuellen Relayport; keine alte Filterregel wird nachgeführt oder weiterverwendet. | `delete`-Vertrag und vollständiger eigener Diff. Derselbe vorhandene Netzfall übt beide Übergänge aus; lokaler `tc`-Aufruf wurde nicht vorgenommen. |
| Fehlerwahrheit | Kernelzustandslesen nutzt `check_output`; Delete nutzt `check=True` und 20 Sekunden Unterprozessbudget; Bash-`set -Eeuo pipefail` bleibt aktiv. Kein normaler Setupfehler wird ignoriert. Ein unbekannter konfigurierter Root wird nicht ausdrücklich entfernt. | Eigene Quellreview; der bisherige Rust-Subprozessfehlerweg bleibt unverändert. |
| Nur Profilmutationen | Der Reset liegt ausschließlich im vorhandenen `if [[ $mode == profile ]]`. `snapshot` liest weiterhin denselben aktuellen Zustand ohne Reset. Referenz und Bildszene eines Profils behalten ihre gemeinsamen Qdiscs und monoton vergleichbaren Zähler. | Byteidentität sämtlicher verbleibenden Helperteile nach Abzug des isolierten Resets und des einen Rootverbs. |
| Shaping und dynamische Ports | Ratenbegrenzung nur auf dem Relay-Zielport, Rückweg ohne zusätzliche Rate, zwei Verzögerungslegs, Queuegrenze, Priomap und beide TCP-Portmasken bleiben wortgleich. | Vollständiger eigener Diff; keine neue Rate, kein statischer Port, keine zusätzliche Shapingschicht. |
| Isolation, Signale und Aufräumen | Die Namespace-/Loopback-/Argumentgrenze liegt weiterhin vor jeder Mutation. `exec`, Lebensdauerpipe, Root-Supervisor, Parent-Death-Signal, Unshare, PID 1, Offload-/MTU-Setup und EXIT-Trap bleiben unverändert. | Byteidentitätsabgleich mit dem Fehlerkandidaten. Keine zusätzliche lokale Namespace oder Rootmutation wurde ausgeführt. |
| Sicherer Messabschluss | Die vorhandene zweiphasige Aufgabenhilfe, beide Finishaufrufe und `Pair::close` bleiben bytegleich. Echte frühe Aufgabenfehler werden weiter von der vorhandenen Hilfe gemeldet. | Byteabgleich der vier P27-Rustdateien; M07 verändert keine ACK-/Pipeline-/Tasklogik. |
| Messung und V7-Orakel | Dreierfolge, feste Fenster, reale Kontroll-/Wire-/ACK-/Nutzbytewege, Auswertung und Schwellenwerte bleiben unverändert. | `batches_rate_reference_task_tests.rs`, `batches_rate_reference_metrics.rs`, `batches_rate_reference_transport.rs` und `batches_rate_reference_tasks.rs` sind jeweils bytegleich mit Kandidat `1ffbf4b0`. Kein vollständiges Leistungsergebnis ist damit bewiesen. |
| Registrierung und Ausgabe | Exakte ignorierte Linuxfixture, Kandidatbindung, Manifestprofile und Pflichtrohdaten behalten ihre vorhandenen Namen und Prüfungen. | Runner-/Manifestquellen gezielt gelesen; keine zusätzliche Fallregistrierung und kein zusätzlicher Suiteeinstieg erforderlich. |

Tatsächlich ausgeführte zulässige statische Nachweise:

- `bash -n native/review-task-network.sh` ohne Syntaxbefund; der Helper
  wurde nicht ausgeführt.
- `ast.parse` für die zwei eingebetteten `python3 -c`-Programme sowie die
  beiden Python-Heredocs ohne Syntaxbefund; keines dieser Programme wurde
  ausgeführt.
- Statischer Textvergleich bestätigt: Nach Abzug des neuen Resetblocks und
  der Änderung des einen Rootverbs entspricht der gesamte Helper dem
  Fehlerkandidaten. Der Vergleich umfasst ausdrücklich Snapshot,
  Konfigurationsparameter und vollständige sichere Aufräumgrenze.
- Statischer Bytevergleich bestätigt unveränderte vier P27-Rustdateien;
  `git diff --check` meldet keinen Whitespacebefund.

Die eigene Review hat keinen weiteren belegten Defekt der zugewiesenen
Änderung gefunden. Ein erfolgreicher statischer Parse ist kein Kernel-,
Fixture- oder V7-Leistungsnachweis. Es gab keinen lokalen Build, Compiler,
Formatter, Test, Server, Paketinstallationslauf, Namespace-/`tc`-Versuch,
Unteragenten, Commit, Push, Graphupdate oder Release dieses Autors.

## Vorhandener Remote-Nachweis und offene Laufzeitfrage

Es ist keine neue Fallkennung erforderlich. Main verwendet ausschließlich
den vorhandenen gemeinsamen Remote-Einstieg mit der bereits registrierten
ignorierten Linuxfixture:

`screen::core::rate_control::p27_acceptance::p27_network_reference_acceptance`.

Ihre unveränderte Folge lautet vollständig:

1. `10mbit-one-window`: 10.000.000 bit/s, ein Fenster;
2. `1mbit-one-window`: 1.000.000 bit/s, ein Fenster;
3. `10mbit-four-windows`: 10.000.000 bit/s, vier Fenster.

Erwartetes Signal: Alle drei neuen Profilkonfigurationen gelingen in
derselben eigenen Namespace; jeder aktuelle Port erreicht seine beiden
Netem-Legs, und anschließend liefern dieselben V7-Orakel ihr unverändertes
Ergebnis. Erfolg verlangt weiterhin alle vollständigen Rohdateien
`10mbit-one-window-reference.json`, `10mbit-one-window-screen.json`,
`1mbit-one-window-reference.json`, `1mbit-one-window-screen.json`,
`10mbit-four-windows-reference.json` und `10mbit-four-windows-screen.json`
sowie den kandidatengebundenen Gesamtbericht `acceptance.json`.

Noch unbestätigt sind der tatsächliche Root-/Filteraustausch auf dem
Remote-Kernel, der vollständige Durchlauf der beiden bisher nicht
abgeschlossenen Profile und das vollständige V7-Leistungsresultat.
Die Rohdateien des ersten Profils aus dem Fehlerlauf belegen dessen
abgeschlossene Messkörper, keinen vollständigen P27-Pass. Main besitzt
weiterhin Auslösung und Auswertung derselben Suite sowie die anschließende
einmalige vollständige Remote-Veröffentlichung des Gesamtbatches.

## Vollständig gelesene und bearbeitete Dateien

Die Liste umfasst die eigenen vollständigen Lektüren und gezielten
Quellen-/Signaturauszüge dieses neuen M07/F57-Zyklus; frühere Lesefreigaben
wurden nicht als zusätzliche aktuelle Leseerlaubnis verwendet.

- Auftrag und Anleitungen: `/tmp/smartexplorer-f-network_profiles-scope.json`,
  `/tmp/smartexplorer-final-review-user-contract.txt`, die vollständig über
  MCP gelieferte aktuelle Arbeitsweise sowie
  `/root/.codex/skills/graphify/SKILL.md` und
  `/root/.codex/skills/graphify/references/query.md`. Die geltenden globalen
  und Repositoryanweisungen lagen außerdem vollständig in der
  Nutzernachricht vor.
- Projekt-/Fehlerbefunde: `docs/ARCHITEKTUR.md`,
  `docs/plaene/2026-10-24-batches/rv1-funktionsplan.md`,
  `docs/plaene/2026-10-24-batches/rv1-funktionsbefunde.json`,
  `docs/plaene/2026-10-24-batches/fix-review-p27-abnahme.md`, dieser eigene
  Bericht und der bestehende `graphify-out/graph.json` über Vokabular und
  Graphabfrage. Der ausdrücklich erlaubte Zugriff auf
  `graphify-out/.graphify_python` lieferte „Datei fehlt“; es wurde kein
  Ersatzmarker angelegt.
- Helper/Integration: `native/review-task-network.sh`,
  `native/review-task-native.py`, `native/review-task-manifest.json`,
  `native/test-review-task.sh` und `.github/workflows/review-task.yml`.
- Mess-/Abschlussgrenze unter `native/src/screen/core/`:
  `batches_rate_reference_task_tests.rs`,
  `batches_rate_reference_transport.rs`,
  `batches_rate_reference_tasks.rs` und
  `batches_rate_reference_metrics.rs` einschließlich Bytevergleich.
- Abgeschlossene Remote-Evidenz unter
  `/tmp/smartexplorer-rv1-40406246032/`: `native-failures.json`,
  `linux/p27-network.log`,
  `linux/p27-network/10mbit-one-window-reference.json` und
  `linux/p27-network/10mbit-one-window-screen.json`. Die Rohdatendateien
  wurden ausschließlich statisch geparst; kein Leistungsorakel wurde lokal
  ausgeführt.
- Externe Primärquellen: die oben verlinkte versionsgebundene
  `sch_api.c`-Quelle von Linux 6.17 sowie `tc(8)` auf man7.org, Abruf
  2026-10-26. Ein GitHub-Rawlinkfehler wurde durch die versionsgleiche
  Roh-URL ersetzt, ohne neue Projektdatei oder Downloadablage.

Geändert wurde ausschließlich `native/review-task-network.sh`.
Erstellt wurde ausschließlich dieser Bericht
`docs/plaene/2026-10-24-batches/fix-f-network-profiles.md`.
Keine Schreibfläche der gleichzeitig arbeitenden Windows-, Transfer-,
MCP-, Registry-, Capture- und Netzwechselautoren wurde berührt.

## Entscheidungen und Übergabe

Der eigene Fünfteilzyklus aus Fehlerfunktionsliste, Kettenkarte, vorherigem
Erhaltungsplan, Fix und anschließender statischer Anti-Regressionsreview
ist innerhalb der freigegebenen Fläche geliefert. Die Änderung bleibt eine
Reparatur des Messprogramms; Produktleistung und Veröffentlichung werden
nicht als bestätigt ausgegeben. Es bestehen keine zusätzlich benötigten
Scopeabhängigkeiten. Die benannten Laufzeitfragen bleiben bei Mains
unverändertem Remote-Nachweis. Dieser Autor stoppt nach der Übergabe.
