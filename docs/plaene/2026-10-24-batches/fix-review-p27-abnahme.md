# G3: P27-Abnahmeprogramm

Stand: 2026-10-26. Fixautor: delegierter Agent `fix_p27_acceptance`.
Arbeitskopie: `/tmp/smartexplorer-review-fixes-ee75e91b`, Ausgang
`ee75e91bad6b28ef9206a5b2b2857cbea75c4e5b`.

## Erstplan aus dem Ergebnisauftrag

Quelle: vollständiger delegierter Ergebnisauftrag; Nutzervorgaben in
`/tmp/smartexplorer-final-review-user-contract.txt`; Schreib-/Lesefläche in
`/tmp/smartexplorer-fix-p27-acceptance-scope.json`.

Ziel ist die Behebung des belegten Suitefehlers G3. Das Abnahmeprogramm muss
V7/P27 tatsächlich über einen isolierten Netzwerkpfad messen: 10 Mbit/s,
20 ms RTT, Relayreferenz ohne Encoder, hinreichendes synthetisches Frameangebot,
Zielfrist, stationärer Nutzdurchsatz, P95-Steuerlatenz, Queuealter sowie
Niedrigrate und mehrere Fenster mit gemeinsamem Senderbudget.

Beauftragt sind die Funktionsauflistung, vollständige direkte und indirekte
Verknüpfungskarte, konkreter Fixplan, Fixtureimplementierung und eigene
Anti-Regressionsreview durch denselben Autor. Die Produktleistung ist bisher
unbekannt; G3 behauptet keinen gemessenen Produktfehler.

Grenzen: ausschließlich die sieben zugewiesenen Dateien; an `rate_control.rs`
nur Fixtureanschlüsse. Kein Build, Compiler, Formatter, Testlauf, Serverstart,
Installation, Commit, Push, Graphupdate oder Release. Gemeinsames Manifest,
Orchestrierung und CI gehören Main. Bestehende Rate-/Pacing-/Senderbudgetfälle
und Bildschirmbedienwege bleiben erhalten. Der neue Code wird erst in Mains
einer gemeinsamer Remote-Suite ausgeführt.

Bekannt aus dem Auftrag: fehlende echte P27-Messung; verbindliche V7-Grenzen;
gemeinsamer Zielhaushalt; keine öffentliche Relayvoraussetzung. Durch gezielte
Bestandslektüre zu klären: verfügbare lokale Relay-API, produktiver Wire-/ACK-
und Budgetpfad, Runneranschluss, Messfenster und unverfälschtes Orakel. Vom
Nutzer zusätzlich benötigte Angaben sind bisher nicht erkennbar.

Planungstiefe: detailliert, weil reale Transportmessung und Budgetteilung
zusammenpassen müssen. Benötigte Wissensgebiete: Iroh/QUIC-Relay, Linux-
Netzwerkisolation und Traffic Control, Bildschirm-Wireprotokoll, Ratenregelung,
Messstatistik sowie Remote-Suiteauswahl.

Nächster Arbeitsschritt: die zugewiesenen Produkt- und Suitegrenzen erfassen,
daraus den Erstentwurf ableiten und Netz-/Relayfragen mit Primärquellen klären.

## Bestand und Stufe-eins-Plan

G3 betrifft drei gekoppelte Abnahmefunktionen: `native_tests` wählt die alten
Regler-/Budgetfälle, `milestone_evidence` akzeptiert deren Namen als P27-Beleg,
und der P27-Manifesteintrag nennt keinen Transportvergleich. Die vorhandenen
`batches_sender_task_tests.rs`-Fälle sind korrekte Besitz-/Fairnessfixtures;
ihre Existenz ersetzt die Leistungsabnahme nicht. Der lokale Encoderbench
misst ebenfalls keinen Relayvergleich. Quelle: G3 in `review-gesamt.md` und
`vertraege.md`, V7; Runner und Manifest am Ausgangskandidaten.

Stufe-eins-Plan: einen echten privaten Relayvergleich ergänzen, das bestehende
Abnahmeprogramm um dessen verpflichtenden Ergebnisanschluss erweitern lassen
und die bereits vorhandenen Logikfälle behalten. Keine Änderung des
Produktreglers aus einer bisher ungemessenen Leistungsannahme.

## Direkte und indirekte Verknüpfungskarte

Der vollständige vom G3-Messprogramm berührte Abhängigkeitsumfang ist:

1. `.github/workflows/review-task.yml` → `native/test-review-task.sh` →
   `native/review-task-native.py::{artifact,native_tests,milestone_evidence}` →
   `native/review-task-manifest.json`: Kandidatbindung, Developmentbinary,
   Auswahl, Ergebnisbeleg. Diese vier Dateien bleiben in Mains Schreibfläche.
2. Neues Linux-Fixture → isolierter Netzwerkhelfer → Kernel-Netznamespace,
   Loopback-MTU, Offloadabschaltung, `tc`-Filter und Netem. Die Namespace
   besitzt ausschließlich Loopback und keine externe Route. Ein fehlendes
   Werkzeug oder ein wirkungsloses Profil ist ein Fehler, kein Messpass.
3. Gepinntes Iroh 1.3.0 `test_utils::run_relay_server_with(false)` → privater
   TLS-/WebSocketrelay → zwei `Endpoint`-Instanzen ohne IP-Transporte und
   Address Lookup → echte QUIC-Verbindungen. Mehrere Fenster verwenden
   getrennte Verbindungen derselben beiden Endpointidentitäten.
4. Referenz: vorallokierte Nutzbytes → QUIC-Uni-Stream → Relay → echter
   Empfänger. Kein Encoder und keine Reglerarithmetik liefert Referenzwerte.
5. Kandidat: synthetischer `ScreenSource`/`HostStage` → echte
   `Pipeline`/`PipelineActor` → `host_pipeline_produce` mit `FrameClock`,
   `Pacer`, `RateController`, gemeinsamem `SenderBudget` →
   `host_capture::produce` und `CapturedRecords` mit vorheriger Admission →
   `host_frame_send` → `GopWriter::write_frame_admitted` → Relay.
6. Empfänger: `accept_picture_streams_with_contract` → budgetierter
   Wire-Reader/`RecordSequence` → tatsächliche vollständige Records und
   `ViewerFrame::FrameAck` → Kontrollstream → `PipelineActor::on_ack` →
   `Pipeline::on_ack` → Senderdebtfreigabe, Pacer und Ratecontroller. Es gibt
   keine direkt in den Regler eingespeisten fiktiven Empfangszeiten.
7. Steuerlatenz: `send_viewer_frame(Ping)` → derselbe belastete Relaypfad →
   `recv_viewer_frame` → `control_sender::send(Pong)` mit peergeteilter
   Steuerreserve → `recv_host_frame`. Referenz und Kandidat verwenden diesen
   identischen Weg; ACKs teilen mit Pings den Kontrollstream.
8. Orakel: echte Empfangsbytes/-zeitstempel, angebotene Pipelineziele,
   tatsächliche Frameintervalle und echte Pingantworten → feste stationäre
   Fenster → Zielrampenfrist, Nutzrate, P95, Queuealtersfolge und
   Fensterfairness → kandidatengebundene Rohdaten und Ergebnisartefakte.
9. Unveränderte indirekte Produktverbraucher sind Desktop-/Android-Viewer,
   Worker-Stage und Hostsession, deren `PipelineActor`-/Wire-/Budgetverträge
   wiederverwendet werden. Decoderqualität, echte Eingabeinjektion,
   Bildschirmcapture und öffentlicher Relaybetrieb sind keine Aussagen dieser
   isolierten P27-Messung. Die bestehenden Fälle bleiben ihre separaten Belege
   innerhalb derselben Task-Suite.

Der AST-Graph wurde mit `graphify query` befragt. Er liefert Ratecontroller-,
Pipeline-, Wire- und Share-Knoten, aber keinen Messnachweis. Die installierte
Graphify-CLI meldet Skill 0.9.11 gegenüber Paket 0.9.17; deshalb wurden die
entscheidenden Kanten zusätzlich im aktuellen Quelltext nachvollzogen.

## Recherche, Alternativen und zweite Lückenprüfung

Geprüft am 2026-10-26: gepinnte Iroh-Quelle `src/test_utils.rs:34–87` und
`src/endpoint.rs::endpoint_two_relay_only_no_ip`; [Iroh-Quelldokumentation](https://docs.rs/crate/iroh/1.3.0/source/src/test_utils.rs),
[Netem](https://man7.org/linux/man-pages/man8/tc-netem.8.html),
[HTB](https://man7.org/linux/man-pages/man8/tc-htb.8.html) und
[Netznamespacevertrag](https://man7.org/linux/man-pages/man7/network_namespaces.7.html).

Gewählt: private Iroh-Testrelay-API und Linux-Netem in eigener Namespace.
`clear_ip_transports` verhindert eine Direktabkürzung; `Minimal` und
`clear_address_lookup` verhindern externe Discovery. Ein Zielportfilter
begrenzt nur den Verkehr zum Relay auf 10 Mbit/s beziehungsweise 1 Mbit/s;
der Rückweg wird nicht nochmals ratenbegrenzt. Je Relay-TCP-Leg kommen 5 ms
hinzu: Host→Relay→Viewer→Relay→Host ergeben 20 ms Grund-RTT. Das vermeidet
die fälschliche Halbierung des Nutzdurchsatzes durch zweifache Formung beider
Relaylegs. Kalibrierte unbelastete Ping-RTT und tatsächliche Filterzähler
werden zusätzlich geprüft und gespeichert.

Verworfen: reine virtuelle Reglerzeit (kein Transport), nur ein Encoderbench
(kein Referenz-/Netzvergleich), öffentlicher Relay (nicht isoliert), globales
Host-Qdisc (berührt fremden Verkehr), eigener Relaynachbau (dupliziert das
Protokoll). Ein privater Share-Server wäre für Auth-/Servermatrixfälle nötig,
aber für die unveränderte Iroh-Relaystrecke dieses G3-Vergleichs größer als
die bereits gepinnte Test-API. Eine Alt-/Neuserververbesserung wird nicht
behauptet; der Server wird in diesem Fix nicht geändert.

Die zweite Lückenprüfung löste: Admission muss auch synthetische Recordbytes
vor ihrer Allokation decken; deshalb eigener kleiner CapturePlan mit echten
CodecOutput-/Metadata-Claims. Mehrfenster benötigen dieselbe Peeridentität,
aber getrennte Screenverbindungen, damit GOP-/ACK-Identitäten nicht
kollidieren. Steuerproben müssen tatsächlich versendet und vollständig
zurückgekehrt sein; fehlende Antworten werden nicht aus P95 entfernt.

## Finaler Detailplan vor der Fixtureimplementierung

| Meilenstein | Dateien | Erhaltene Grenze / konkretes erwartetes Ergebnis |
|---|---|---|
| M1: Isolierter realer Pfad | `review-task-network.sh`, Dev-Iroh-Feature in `Cargo.toml` | Nur eigene Namespace; dynamischer Relayport; 10/1 Mbit/s; 20-ms-Grund-RTT; echte Relayauswahl und Profilzähler. |
| M2: Referenz und Steuerkanal | `batches_rate_reference_transport.rs` | Encoderfreie Sättigung und identischer produktiver Ping/Pong-Weg; Messung zählt empfangene Nutzbytes. |
| M3: Produktive Bildpipeline | `batches_rate_reference_task_tests.rs` | Synthetische admitted Frames durch PipelineActor, Pacing, Rate, gemeinsamen Peerhaushalt, GOP und echte ACKs; ein und vier Fenster. |
| M4: Leistungsorakel | `batches_rate_reference_metrics.rs` | Tatsächliche Rampenfrist, mindestens 90 % Nutzrate, höchstens zwei Frameintervalle zusätzliche P95, kein signifikanter stetiger Queuealtersanstieg und kein verhungertes Fenster; Rohdaten bleiben auswertbar. |
| M5: Anschluss und eigene Review | Fixtureanschluss in `rate_control.rs`, dieser Bericht; Main registriert Runner/Manifest | Genau ein dedizierter Messfall im gemeinsamen Linux-Suiteeinstieg; kein zusätzlicher allgemeiner Filterlauf; vollständige statische Eigenprüfung derselben Kette. |

Messdefinitionen vorab: je Lauf fünf Sekunden Warm-up und zwölf Sekunden
stationäres Fenster. Referenz erhält kontinuierlich vorallokierte Bytes;
Kandidat erhält mit jedem produktiven StageRequest das darin verlangte
Byteangebot. 30 fps ist die gewählte Fixtureszene. Die 1-Mbit/s-Szene liegt
ausdrücklich unter dem bisherigen Relaystartwert. Vier Fenster sind die
gewählte konkrete Mehrfensterszene. Queuealter ist die wirkliche Dauer von
Frameproduktion bis vollständigem Recordempfang; Einsekunden-P95-Buckets
werden auf streng monotones Wachstum und einen positiven Mann-Kendall-Trend
mit einseitig 95 % ausgewertet. Dies ist die vorab festgelegte endliche
Operationalisierung von „nicht fortlaufend steigen“, kein Produktionslimit.
Ausbleibender Durchsatz, fehlende Samples, ungesättigte Referenz, verlorene
Steuerproben und wirkungsloses Shaping sind Fehler. Ein bereits über der
Referenz liegendes Startziel benötigt null Wachstumsintervalle; die V7-Zugabe
von zwei Intervallen bleibt bestehen.

Zusätzliche Schlussprüfung vor dem Helferabschluss: Main weist auf den
Benutzerwechsel durch `sudo` und den vom normalen Runnerbenutzer ausgehenden
Prozessgruppenabbruch hin. Der Aufräumvertrag wird deshalb explizit an eine
eigene PID-Namespace mit `unshare --fork --kill-child=KILL` gebunden. Unshare
ignoriert beim Warten auf sein Kind SIGTERM/SIGINT; dessen bloße Verwendung
würde den Runnerabbruch daher nicht vollständig abdecken. Gewählt ist eine
anonyme Pipe mit genau einer Schreibseite im Prozess des Runnerbenutzers.
Deren EOF veranlasst den Root-Supervisor, seinen Unshareprozess zu töten.
`PR_SET_PDEATHSIG` bindet Unshare zusätzlich an das Leben dieses Supervisors.
Das Ende von Unshare tötet deren PID 1; der Kernel beendet anschließend
sämtliche Prozesse der PID-Namespace. Die private Netznamespace und ihre
Qdisc werden damit ebenfalls freigegeben. Der Helfer benötigt keine
temporäre Profilsteuerdatei mehr: beide Snapshotaufrufe erhalten den bereits
entdeckten Relayport und die Szenenrate als überprüfte Argumente. Dies ist
eine Ergänzung des eigenen Messprogramms, kein behaupteter Produktdefekt.

## Umsetzung und getroffene Entscheidungen

M1–M5 sind innerhalb der Autorfläche vorbereitet. Die neue Linux-Devabhängigkeit
aktiviert `iroh = =1.3.0` mit `test-utils`; die produktiven Iroh-Features
bleiben unverändert. Der private Relay ist die gepinnte Herstellerimplementierung,
kein Nachbau und kein öffentlich erreichbarer Dienst.

`SaturatingSource` erzeugt rateabhängige synthetische Recordbytes im
produktiven HostStage-Vertrag. Der verwendete H.264-Drahtmodus ist
ConstrainedBaseline. Die Recordbytes dienen ausschließlich der isolierten
Transportmessung und sind kein Nachweis eines decodierbaren H.264-Bildes.
Der produktive PipelineActor übernimmt Capturetakt, Ratenregelung,
Senderbudget, Pacing, Streamöffnung und Schreiben. Ein echter CapturePlan
verspricht vor der Allokation bis zu 1.000.000 CodecOutput-Bytes und einen
Recordheadercontainer; `CapturedRecords::from_allocator` hält diese Claims.
Das Fixture verwendet vorhandene isolierte Capture-Testadmission mit
kontrollierten Speicherfakten. Die 64-MiB-Framecapability ist Fixturekonfiguration,
keine Änderung einer Produktionsspeichergrenze. Frame-/Sessionmemory-Code
und dessen bestehende Fixtures wurden nicht bearbeitet.

Die drei Szenen sind exakt:

| Kennung | Profilrate | Sichtbare synthetische Fenster |
|---|---:|---:|
| `10mbit-one-window` | 10.000.000 bit/s | 1 |
| `1mbit-one-window` | 1.000.000 bit/s | 1 |
| `10mbit-four-windows` | 10.000.000 bit/s | 4 |

Jede Szene misst zuerst die encoderfreie Referenz und anschließend dieselbe
Pipeline mit synthetischem Angebot, jeweils fünf Sekunden Warm-up plus zwölf
Sekunden stationäre Beobachtung und zwei Sekunden Antwortnachlauf. Die
unbelastete tatsächliche RTT wird vor der Last gemessen. 18–35 ms sind die
vorab gewählte Kalibriertoleranz des 20-ms-Profils. Eine Referenz unter 80 %
oder über 105 % der profilierten Linkrate wird als ungültige Sättigungs-/
Profilgrundlage verworfen. Diese zusätzlichen Plausibilitätsgrenzen sind
Fixtureentscheidungen; die verbindliche Kandidatgrenze bleibt 90 % des
tatsächlich gemessenen Referenznutzdurchsatzes.

Die Rampenfrist verwendet die V7-Formel mit tatsächlich gemessenem `R_ref`
und dem beobachteten Relaystartziel. Mehrere Fenster verwenden deren
gemeinsame Zielsumme. Die verbrauchten Regelintervalle werden aus realer Zeit
und `max(beobachteter RTT, beobachtetem Frameintervall)` integriert; die
Rohangebote enthalten beide Timingwerte. Bei mehreren Fenstern bestimmt der
jeweils größte beobachtete Wert den gemeinsamen Takt. Es gibt keinen
vorgegebenen erfolgreichen Ziel-/ACK-Verlauf. Der erste angebotene Zielwert
jedes Fensters muss den tatsächlichen Relaystart bestätigen.

Jeder tatsächlich gesendete Ping muss genau eine passende Antwort erhalten.
P95 enthält die wirklichen Hin-/Rücklaufzeiten. Bei konstanter Kadenz gilt
direkt `P95_Kandidat <= P95_Referenz + 2 × Frameintervall`. Bei einer vom
Produktpfad tatsächlich veränderten Kadenz wird je Ping dessen zuletzt
beobachtetes Frameintervall abgezogen und der P95 des verbleibenden
Latenzanteils gegen den Referenz-P95 geprüft. Ein einzelner langsamer Frame
vergrößert somit nicht nachträglich das Latenzbudget aller Steuerproben.
Vorbereitungskapazität entsteht aus tatsächlichen Bytes und gemessener
Vorbereitungszeit über den Lauf; ein einzelner Schedulerjitter wird nicht
als dauerhafte Encoderkapazität ausgegeben.

Alle vollständigen empfangenen Recordbytes zählen zum Nutzdurchsatz;
Wireheader zählen dort nicht. Das Queuealter wird erst nach vollständigem
Recordempfang bestimmt. Gebrochene Streams werden mit Fenster, GOP und
Zeitpunkt in den Rohdaten protokolliert; verlorene Bytes werden nicht als
geliefert gezählt. Die Mehrfensterprüfung verlangt zusätzlich pro Fenster
mindestens die Hälfte seines gleichen Referenzanteils. Diese konkrete
Fairnessschwelle ist eine vorab gewählte Operationalisierung des Erhalts
sichtbarer Fenster; sie ersetzt keine zugesagte Produktionsrate je Fenster.

Das Queueorakel verwendet die tie-korrigierte Mann-Kendall-Varianz und
Kontinuitätskorrektur gemäß
[USGS, The Seasonal Kendall Test for Trend, Gleichungen 1–4](https://pubs.usgs.gov/of/1981/0488/report.pdf).
Die zwölf Einsekunden-P95-Buckets bleiben vollständig im Bericht. Der
einseitige Normalapproximation-Schwellenwert ist 1,6448536269514722. Die
endliche Bucketprüfung ist keine Garantie für unbegrenzt lange Laufzeiten
oder für statistische Unabhängigkeit serieller Queuewerte.

## Eigene anschließende Anti-Regressionsreview

Diese Prüfung hat derselbe Fixautor nach der Implementierung durchgeführt.
Prüfgegenstand waren sämtliche Stationen der oben erfassten Kette,
insbesondere unverfälschte Messwerte, vollständige Fehlererfassung,
gemeinsame Peeridentität, Credit-/Taskbesitz und die Runneranbindung.

| Bereich | Eigener Befund / Korrektur | Beleg und praktische Grenze |
|---|---|---|
| Regler und bestehende Logikfälle | Produkttext in `rate_control.rs` bytegleich mit Ausgang; nur vier Zeilen Linux-Testanschluss ergänzt. `batches_sender_task_tests.rs` unverändert. | Statischer `git show`-/Dateivergleich. Kein neuer gemessener Produktpass. |
| Cargo-/Plattformgrenze | TOML-Parse bestätigt ausschließlich den Linux-Dev-Iroh-Eintrag; vollständige verbleibende Konfiguration entspricht dem Ausgang. | Kein Featurewechsel des Produktbaus. Remote-Resolver-/Compilerbeleg steht aus. |
| Produktive Pipeline | Quelle erreicht den vorhandenen HostStage-Zweig; Admission liegt vor dem Byteallocator; produktive `write_frame_admitted`-/ACK-Funktionen bleiben zuständig. | Signaturen und Eigentumspfade im aktuellen Quelltext gelesen. Kein physischer Encoder-/Decoderlauf. |
| Referenz und Bytezählung | Referenzbytes werden erst nach echtem Streamread gezählt; Kandidatbytes erst nach vollständigem Wire-Record. Payloadidentität wird überprüft. | Keine simulierte Arrival-/Deliveryfolge und kein hartkodierter Referenzwert. |
| Start, Timing und Regelintervall | Startziele werden aus tatsächlichen StageRequests erfasst. RTT und Frameintervalle kommen aus Verbindung/Request. | Rampenorakel an gemessenes Timing angepasst; kein künstlicher 20-ms-ACK-Eingang. |
| Steuerlatenz | Sent-/Reply-Zuordnung prüft Fenster, Sequenz und Sendestempel. Fehlende und doppelte Antworten scheitern. | Zwei tatsächliche Frameintervalle werden pro Probe berücksichtigt; ein Ausreißer erweitert kein gemeinsames Maximalbudget. |
| Queuealter und Fairness | Fehlende Einsekunden-Buckets scheitern; streng monotone oder signifikant positive Folgen scheitern; Fensteridentitäten bleiben getrennt. | Ein-/Vierfensterfall nutzt dieselben Peerendpoints und getrennte stabile Verbindungen. |
| Privates Netzprofil | Tatsächliche Qdisc-Rate, Delay, Queuegrenze, Filter, MTU, Offloads und Paketdifferenzen werden gelesen und überprüft. | Beide Richtungen müssen im jeweiligen Messlauf zusätzliche Pakete durch Netem führen. Die Namespace besitzt nur Loopback. |
| Taskfehler und Cleanup | Ein vorzeitiger Taskabschluss wird beim Join als Fehler ausgegeben. Während dieser Review wurde der Joinloop auf `pop` geändert, damit eine Panic verbleibende Handles nicht durch `drain` abtrennt. | Verbleibende Tasks bleiben bis zum Abbruch im Besitzer. PipelineActor behält seinen produktiven Interrupt-/Disposeweg. |
| Runnerabbruch und Root-Kinder | Anonyme Lebensdauerpipe, Root-Supervisor, Parent-Death-Signal und eigene PID-/Mount-/Netznamespace binden den vollständigen Kindbaum. Keine Profilsteuerdatei bleibt bei SIGKILL zurück. | [Unshare-Vertrag](https://man7.org/linux/man-pages/man1/unshare.1.html), [PID-1-Vertrag](https://man7.org/linux/man-pages/man7/pid_namespaces.7.html), [PR_SET_PDEATHSIG](https://man7.org/linux/man-pages/man2/PR_SET_PDEATHSIG.2const.html). Statisch geprüft; Abbruch noch nicht ausgeführt. |
| Bestehende Oberflächen und Daten | Keine produktive Screen-/Share-/Viewer-, CLI-, IPC-, MCP- oder Androidfunktion geändert. Keine gespeicherten Profile oder echten Captures geöffnet. | Die einzige produktive Quelldateiänderung ist unter `cfg(all(test,target_os="linux"))`; Quellen-/Backend-, Berechtigungs-, Abbruch-, Retry-, Backup- und Konfliktimplementierungen bleiben unverändert. |
| Gemeinsame Suiteanbindung | Mains aktueller Runner startet genau den ignorierten Linux-Fall aus demselben Developmentbinary, prüft Kandidat, drei Szenen und Rohdateien; allgemeiner Filter schließt den Messfall aus. | `p27_network_tests` und Manifesteintrag gezielt gegengelesen. Keine eigene Pipeline und keine zusätzliche Ausführung. |

Der H.264-Profilname wurde während der statischen Autorprüfung gegen
`negotiate.rs` zu `ConstrainedBaseline` berichtigt. Der Netzwerkhelfer liest
die tatsächlich installierte TC-Version; dessen JSON-Rate ist Bytes/s,
die Delayangabe Sekunden. Die Prüfungen sind gegen die versionierte
[iproute2-Netemquelle 6.1.0](https://github.com/iproute2/iproute2/blob/v6.1.0/tc/q_netem.c)
und die aktuelle Herstellerquelle abgeglichen.

Durchgeführte zulässige statische Checks: Bash-Parsing des Helfers,
Python-AST-Parsing seiner drei eingebetteten Programme, TOML-Parsing,
Rust-AST-Parsing mit dem bereits installierten Graphify-Tree-sitter und
`git diff --check` für die vorhandenen geänderten Dateien. Die drei neuen
Rustdateien haben 245, 248 beziehungsweise 240 Zeilen und jeweils weniger
als 14 KiB; `rate_control.rs` hat 403 Zeilen und weniger als 15 KiB. Kein
Compiler, Formatter, Test, Server oder Netzprofil wurde lokal gestartet.

## Gelesene Dateien und Quellenfläche

Die folgende Liste nennt gezielt gelesene Dateien einschließlich der
auftragsbezogenen Such-/Signaturauszüge; sie behauptet keine vollständige
Lektüre der gesamten freigegebenen Verzeichnisse.

- Auftrag/Regeln: `/tmp/smartexplorer-fix-p27-acceptance-scope.json`,
  `/tmp/smartexplorer-final-review-user-contract.txt`, `AGENTS.md`,
  `/root/.codex/skills/graphify/SKILL.md` und über MCP gelieferte Arbeitsweise.
- Dokumentation: `docs/ARCHITEKTUR.md`, `docs/refs/rv1-remote-suite.md`,
  `docs/oberflaechen/bildschirm.md`,
  `docs/plaene/2026-10-24-batches/batches.md`, `vertraege.md`, `umsetzung.md`,
  `regressionsreview.md` und `review-gesamt.md` im festen Main-Arbeitsbaum.
- Suite/Konfiguration: `native/Cargo.toml`, `native/Cargo.lock`,
  `native/review-task-manifest.json`, `native/review-task-native.py`,
  `native/test-review-task.sh`, `.github/workflows/review-task.yml` und
  `graphify-out/graph.json` über die Graphify-Abfrage.
- Screen-Core: `rate_control.rs`, `rate_control_tests.rs`, `sender_budget.rs`,
  `batches_sender_task_tests.rs`, `pacing.rs`, `gop_stream.rs`,
  `control_sender.rs`, `host_pipeline.rs`, `host_pipeline_actor.rs`,
  `host_pipeline_produce.rs`, `host_pipeline_init.rs`, `host_frame_send.rs`,
  `host_mode.rs`, `host_quality.rs`, `host_reference.rs`, `host_capture.rs`,
  `host_backend.rs`, `host_stage.rs`, `capture_records.rs`,
  `capture_admission.rs`, `capture_source_contract.rs`, `capture_authority.rs`,
  `capture_session_context.rs`, `capture_plan.rs`, `capture_grant.rs`,
  `capture_test_support.rs`, `frame_size_caps.rs`, `negotiate.rs`,
  `wire_control.rs`, `wire_binary.rs`, `batches_frame_budget_task_tests.rs`
  und `batches_snapshot_task_tests.rs`, jeweils unter
  `native/src/screen/core/`.
- Screen-OS/Share: `native/src/screen/os/shared/synthetic_backend.rs`,
  `native/src/share/core/node_screen_task_fixture.rs`,
  `native/src/share/core/node_relay_tests.rs`,
  `native/src/share/os/shared/lan_link_transport_task_fixture.rs` und
  `native/src/share/os/shared/transport_options.rs`.
- Gepinnte Herstellerquellen: `vendor/iroh-1.3.0/Cargo.toml`,
  `vendor/iroh-1.3.0/src/lib.rs`, `src/test_utils.rs`, `src/endpoint.rs`,
  `src/endpoint/presets.rs`, `src/protocol.rs`, `tests/patchbay/util.rs`;
  `vendor/noq-1.3.0/src/send_stream.rs` und `src/recv_stream.rs`.
- Eigene erzeugte Dateien: dieser Bericht, `native/review-task-network.sh`
  sowie die drei nachfolgend genannten neuen Rustdateien wurden zur eigenen
  Schlussprüfung erneut gelesen beziehungsweise geparst.

## Erstellte und geänderte Dateien

Erstellt:

1. `native/review-task-network.sh`.
2. `native/src/screen/core/batches_rate_reference_metrics.rs`.
3. `native/src/screen/core/batches_rate_reference_transport.rs`.
4. `native/src/screen/core/batches_rate_reference_task_tests.rs`.
5. `docs/plaene/2026-10-24-batches/fix-review-p27-abnahme.md`.

Geändert:

1. `native/src/screen/core/rate_control.rs`: ausschließlich Fixtureanschluss.
2. `native/Cargo.toml`: ausschließlich Linux-Dev-Iroh-Feature.

`batches_sender_task_tests.rs` ist unverändert. Gemeinsames Manifest,
zentraler Runner, Workflow und deren Änderungen gehören Main und sind keine
Änderungen dieses Fixautors. Der feste Main-Arbeitsbaum wurde durch diesen
Fixautor nicht verändert.

## Exakter Anschluss für Main und tatsächlich offene Grenzen

Fallkennung:
`screen::core::rate_control::p27_acceptance::p27_network_reference_acceptance`.
Das Linux-Fixture ist ignoriert und hat absichtlich keinen allgemeinen
`batches_`-Präfix. Mains gemeinsamer Runner verwendet das bereits vorhandene
Library-Testbinary mit folgendem Helferinterface:

```text
bash native/review-task-network.sh exec <Library-Testbinary> --exact screen::core::rate_control::p27_acceptance::p27_network_reference_acceptance --ignored --nocapture --test-threads=1
```

Benötigte Umgebungswerte: `CANDIDATE_SHA`, `SMART_EXPLORER_TASK_LOG_ROOT`
und `GITHUB_ACTIONS=true` oder `SMART_EXPLORER_REMOTE_RUNNER=1`. Benötigte
Runnerwerkzeuge: Bash, Python 3, `sudo -n` oder Root, `unshare`, `ip`, `tc`
und `ethtool`; die zusätzlichen Linuxpakete sind `util-linux`, `iproute2`
und `ethtool`. Der Kernel muss eigene Netz-/PID-/Mount-Namespaces zulassen.
Dieser Aufbau wurde lokal nicht aktiviert.

Ausgaben unter `$SMART_EXPLORER_TASK_LOG_ROOT/p27-network/`:
`acceptance.json`, `10mbit-one-window-reference.json`,
`10mbit-one-window-screen.json`, `1mbit-one-window-reference.json`,
`1mbit-one-window-screen.json`, `10mbit-four-windows-reference.json`
und `10mbit-four-windows-screen.json`. Der Hauptbericht enthält die
tatsächlich gelesenen TC-/Namespace-/Offload-Snapshots. Stdout enthält
zusätzlich `SE_P27_ACCEPTANCE_JSON=...`. Main bindet Bericht, Manifesteintrag
und vorhandenes Binary an denselben Kandidaten und nimmt den Messfall nicht
erneut in den allgemeinen Filter auf.

Die bestehende Lockdatei enthält `iroh-relay 1.3.0` und `simple-dns 0.12.0`,
aber bisher keine Axum- oder Rcgen-Pakete. Die konkreten zusätzlichen
Resolverversionen sind deshalb noch unbekannt. Main hat die erforderliche
kandidatengebundene Remote-Lockauflösung übernommen. Kein lokaler
Cargoaufruf wurde vorgenommen und die Lockdatei wurde durch diesen Autor
nicht geändert. Den erforderlichen Root-Graphrefresh übernimmt Main nach
der gesamten Integration; dieser Autor hat keinen Graph neu erzeugt.

Der vorbereitete Code ist kein gemessenes Bestehen der Leistungsziele.
Offen sind der Remote-Compiler-/Formatnachweis, die tatsächliche Ausführung
der drei Netzszenen und eine tatsächlich ausgelöste Abbruchbestätigung.
Physische Codec-/Displayqualität, echte Eingabeinjektion, öffentliche
Relayleistung, Live-Serverbetrieb und die Alt-/Neuservermatrix werden durch
diese isolierte P27-Messung nicht behauptet. Es wurde kein zusätzlicher
Produkt-Performancefehler festgestellt. Mains gemeinsame Remote-Suite und
der anschließende gesamte Releaseweg bleiben dessen Zuständigkeit.

Der eigene Fünfteilzyklus aus Funktionsauflistung, Kettenkarte, Plan,
Implementierung und anschließender statischer Anti-Regressionsreview ist
abgeschlossen. Der Fixautor stoppt nach dieser Übergabe.

## L02: Neuer eigener Fixzyklus für den Messabschluss

Stand 2026-10-26. Dieser Abschnitt gehört ausschließlich zum neuen Fixautor
`fix_p27_shutdown`. Der frühere G3-Autor ist nicht mehr im verfügbaren
Agentbaum; dessen vorstehender Bericht wird dadurch nicht als neue eigene
Prüfung ausgegeben. Seine Aussagen zu damals noch ausstehenden Compilerläufen
sind historisch; den aktuellen Compilerstand führt `rv1-compilerplan.md`.
Ausgang der Fehlerbelege ist Kandidat
`0425aa4608d7e27eb95ee0a37e3bc13dca2ce8bf`, Remote-Lauf `40381452096`.
Arbeitskopie: `/tmp/smartexplorer-review-fixes-ee75e91b`. Mains anschließender
Dokumentationscommit ist beim Beginn der Bearbeitung `2f67ee8d`.

### A: Erstplan aus dem delegierten Auftrag

Quelle: vollständiger L02-Auftrag an diesen Autor, unveränderte Nutzervorgaben
in `/tmp/smartexplorer-final-review-user-contract.txt`, genaue Freigabe in
`/tmp/smartexplorer-fix-p27-shutdown-scope.json` und L02 in
`rv1-compilerplan.md`. Ziel ist ein zuverlässiger Abschluss derselben realen
P27-Messung bei weiterhin vollständiger Meldung früher Aufgabenfehler.
Beauftragt sind die eigene Fehlerfunktionsliste, direkte und indirekte
Verknüpfungskarte, konkreter Fixplan, Umsetzung und nachfolgende eigene
Anti-Regressionsprüfung. Die drei Netzprofile, tatsächliche Pipeline-/Wire-/
ACK-Wege, Zeitfenster, Rohdaten und sämtliche V7-Leistungsorakel bleiben
erhalten. Ein Produktleistungsfehler ist durch den bisherigen Abbruch nicht
belegt.

Nur die drei freigegebenen Fixture-Rustdateien und dieser Berichtsabschnitt
sind schreibbar. Main besitzt Manifest, Suiteeinstieg, Graphrefresh, Commits,
Pushes und Release. Kein lokaler Build, Compiler, Formatter, Test, Server,
Installation, langer Prozess oder Unteragent ist erlaubt. Die eigene
Bestätigung bleibt statisch; die Funktionsbestätigung gehört in dieselbe
Remote-Suite. Aus dem Prompt bekannt sind die vorhandene 10-Mbit/s-Referenz
und der Fehler beim Join. Unbekannt sind zunächst dessen genaue Entstehung
und die zuverlässige Abschlussabgrenzung. Diese Angaben sind durch die
zugewiesenen Quellen klärbar; eine zusätzliche Nutzerangabe ist nicht nötig.
Die Planungstiefe ist zwei: gekoppelte bestehende Aufgaben- und
Kanalbesitzverträge ohne neue Produktfähigkeit.

### B: Bestand, fehlerhafte Funktionen und Stufe-eins-Plan

`Tasks::finish` fordert nacheinander den Abbruch aller gespeicherten
JoinHandles an und akzeptiert danach ausschließlich `JoinError::is_cancelled`.
Die Spawnfolge des ersten Screenflows lautet Host-Control-Leser,
Viewer-Control-Schreiber, Viewer-Control-Leser, Bildstreamakzeptor,
Bildkonsument, Pipelineakteur. Der Host-Control-Leser besitzt `ack_tx`;
der Aktor besitzt dessen Empfänger. Wird der Leser zuerst abgebrochen, kann
der noch laufende Aktor `recv(None)` beobachten und selbst
`wire ACK stream closed` zurückgeben. Der anschließende `abort` muss dessen
bereits fertiges Ergebnis nicht in `Cancelled` ändern. Ein bloßes Ignorieren
von `Ok(Err(...))` würde dagegen echte frühe Fehler verbergen.

Fehlerhafte Funktionen in dieser Grenze sind `Tasks::finish` und die
fehlende gemeinsame Abschlusskoordination von `Tasks::spawn`. Die direkt
betroffenen Integrationen sind `screen`, `consume_picture`,
`transport::control_link` sowie `transport::reference`; ihre Mess- und
Fehlerlogik selbst wird erhalten. `Tasks::drop` muss den Besitz aller noch
nicht gejointen Handles auch bei abgebrochenem Abschluss behalten.

Beobachtung aus `linux/p27-network.log`: `finish` erhält
`Ok(Err(..."wire ACK stream closed"...))`; der 10-Mbit/s-Referenzdatensatz
enthält fünf Sekunden Warm-up, zwölf Sekunden stationäres Fenster und
20.764 µs unbelastete RTT. Im Log fehlt ein Zeitpunkt des ursprünglichen
ACK-Fehlers. Daher bleibt ausdrücklich unbekannt, ob genau dieser Remote-
Fehler erst durch den Abschluss entstand oder vorher vorlag. Belegt ist die
oben beschriebene fehlerhafte Abschlussmöglichkeit, kein Produktleistungsfehler.

Stufe-eins-Plan: den vorhandenen Aufgabenbesitzer auf einen ausdrücklich
eingeleiteten gemeinsamen Abschluss umstellen und sämtliche tatsächlichen
Rückgaben und Panics vollständig sammeln. Referenz und Bildszene verwenden
denselben Besitzer; alle Messwerte und Leistungsentscheidungen bleiben an
ihren vorhandenen Quellen.

### Vollständige direkte und indirekte Verknüpfungskarte dieses L02-Fixes

1. `rate_control.rs` bindet die gesamte Fixture nur unter
   `cfg(all(test, target_os = "linux"))` ein. Die neue Aufgabenhilfe liegt
   innerhalb dieser Grenze. Produktregler, Desktop-, Android-, CLI-, IPC-
   und MCP-Zugänge bekommen dadurch keine neue Laufzeitlogik.
2. `p27_network_reference_acceptance` erstellt je Profil den privaten Relay,
   formt den dynamischen Port über `review-task-network.sh` und ruft zuerst
   `transport::reference`, danach `screen` auf. Beider `Pair::new` erzeugt
   dieselben privaten Endpointidentitäten mit getrennten Fensterverbindungen;
   `Pair::close` bleibt nach dem Aufgabenabschluss zuständig für Verbindung
   und Endpoints.
3. `control_link` eröffnet den bidirektionalen Kontrollstream und erzeugt
   ACK- und Viewerframekanal. Sein Host-Leser ruft `recv_viewer_frame` auf:
   echte Frame-/Preview-ACKs gehen über `ack_tx` zum Aktor; Pings gehen über
   `control_sender::send(Pong)` durch die vorhandene Peersteuerreserve zurück.
   Sein Viewer-Schreiber multiplexiert echte Pings und tatsächliche
   Viewerframes in `send_viewer_frame`. Sein Viewer-Leser protokolliert
   `recv_host_frame(Pong)`. Die Kodierung bleibt in `wire_control_codec.rs`.
4. `reference` hält die Kontrolllinks, startet echten QUIC-Writer und
   Byteprüfer/-zähler, wartet bis `end_us + 2_000_000`, beendet seine Aufgaben,
   übernimmt die Beobachtungen und schließt danach `Pair`. Kein Encoder und
   keine fiktiven Empfangsdaten treten in diesen Pfad ein.
5. `screen` bindet `contract` und `source`: `FrameCapabilities`/
   `FrameContract` → `CapturePlan`/`SessionAdmission` → `CaptureGrant` →
   `SaturatingSource::produce` → `CapturedRecords::from_allocator`. Claims
   bestehen vor der Byteallokation; die Quelle protokolliert reale
   StageRequest-Ziele und Vorbereitungszeiten.
6. `PipelineActor::begin_tick` → `start` → `Pipeline::tick_epoch` in
   `host_pipeline_produce.rs` → FrameClock, Pacer, RateController und
   gemeinsames Senderbudget → bestehender HostStage-Producer →
   `host_frame_send::send` → `GopWriter::open/write_frame_admitted` → Relay.
   Der Aktor behält seine echte Hintergrundoperation, Fehlerausgabe,
   Fallbackprüfung, Sekundenstatistik und `on_ack`-Behandlung.
7. `accept_picture_streams_with_contract` →
   `accept_picture_streams_with_memory` → `accept_picture_streams_admitted`
   → produktive Readeraufgaben → `read_picture_stream`: Contract-/Budget- und
   Sequenzprüfung, echte FrameAck-/PreviewAck-Erzeugung und `PictureEvent`.
   `consume_picture` prüft Recordidentität, Nutzbytes und Capturezeit und
   protokolliert echte Broken-Ereignisse. ACKs laufen durch Punkt 3 zurück
   in `PipelineActor::on_ack`, dessen bestehende Pipeline-/Pacer-/Rate-/
   Budgetverträge erhalten bleiben. Die selbst gelesene Definition
   `host_mode.rs::Pipeline::on_ack` gibt zunächst ausschließlich die eigene
   Senderdebt frei, reicht eine bekannte Pacerprobe mit echtem
   `received_us` an `RateController::on_frame_acked` weiter und erhält
   Previewfreigabe sowie die Ablehnung vergessener Pacerproben.
8. Jeder in Punkten 3, 4, 6 und 7 ausdrücklich von der Fixture gestartete
   Future gehört zu `Tasks`. Der neue Abschluss hält diese Futures zunächst
   alle an, bevor deren Capturebesitzer, QUIC-Streams, ACK-Sender,
   Bildereignissender oder Aktoren freigegeben werden. Nach der Freigabe
   werden alle eigenen JoinHandles abgewartet; Fehler werden erst danach
   gemeinsam zurückgegeben. Produktintern erzeugte GOP-Reader und
   Pipelinejobs behalten ihren vorhandenen Verbindungs-/Interrupt-/Dispose-
   Lebensweg; L02 verändert deren Implementierung nicht.
9. `span`, `calibrate`, `until` und `Observed` liefern unverändert die realen
   Zeit-/Byte-/Angebots-/Kontrollbeobachtungen an
   `batches_rate_reference_metrics.rs::evaluate`. Fünf Sekunden Warm-up,
   zwölf Sekunden stationäre Beobachtung und zwei Sekunden Antwortnachlauf,
   alle drei Profile sowie jedes V7-Orakel bleiben erhalten.
10. `.github/workflows/review-task.yml` → `native/test-review-task.sh` →
    `review-task-native.py::p27_network_tests` → vorhandene exakte ignorierte
    Fixture → kandidatengebundene sechs Rohdateien und `acceptance.json` →
    vorhandener verpflichtender Leistungsbeleg. Main registriert die neuen
    kleinen Abschlussfälle in der allgemeinen Auswahl desselben Einstiegs;
    der echte Netzfall bekommt keinen zusätzlichen Aufruf.

### C: Recherche und Entscheidung

Geprüft am 2026-10-26 gegen den Lockpin Tokio 1.52.3: lokale Herstellerquelle
`src/runtime/task/join.rs` (`abort`, Cancel-Sicherheit und Handle-Drop),
`src/macros/select.rs` (`biased`, gepinnte Referenzen und Cancel-Sicherheit),
`src/sync/watch.rs` (`Sender::subscribe/send_replace`,
`Receiver::changed`) sowie `src/sync/oneshot.rs` (Bereitschaftssignal und
Sender-Drop). Die beiden Syncquellen wurden erst nach Mains ausdrücklicher
Erweiterung der Lesefläche gelesen. Versionsgebundene Webeinstiege:
[JoinHandle](https://github.com/tokio-rs/tokio/blob/tokio-1.52.3/tokio/src/runtime/task/join.rs),
[select](https://github.com/tokio-rs/tokio/blob/tokio-1.52.3/tokio/src/macros/select.rs),
[watch](https://github.com/tokio-rs/tokio/blob/tokio-1.52.3/tokio/src/sync/watch.rs),
[oneshot](https://github.com/tokio-rs/tokio/blob/tokio-1.52.3/tokio/src/sync/oneshot.rs).
Diese URLs benennen die lokal gelesene Herstellerrevision; der neue Autor
hat keinen zusätzlichen Netzabruf durchgeführt.

Gewählt ist ein zweiphasiger Abschluss mit vorhandenen Tokio-Primitiven:
ein Watchsignal fordert das Anhalten an, pro Aufgabe bestätigt ein Oneshot
das Verlassen ihres Messfutures, ein zweites Watchsignal erlaubt erst nach
sämtlichen Bestätigungen dessen Drop. Der gepinnte Future bleibt zwischen
Bestätigung und Freigabe ausdrücklich erhalten. `select!` prüft sein Ergebnis
vor dem Stoppsignal; ein bereits bereitliegender echter Fehler bleibt so
sichtbar. Der Messfuture kann höchstens einmal `Ready` liefern; andernfalls
liefert er `Pending` und der Stopzweig wird im selben Poll geprüft. Es ist
kein ständig bereiter einzelner Streamzweig vor dem Stoppsignal.

Verworfen sind pauschales Ignorieren von Kanal-/Transportfehlern (verliert
echte Fehler), unmittelbarer Stop mit sofortigem Future-Drop (erhält das
Kanalrennen), umgekehrte Abbruchreihenfolge (löst nur eine konkrete Kante),
ein Mutex um sämtliche Future-Polls (serialisiert die Messarbeit) sowie
Änderungen an Produktregler, Netzprofil, Leistungsschwellen oder Zeitfenstern
(durch diesen Befund nicht gerechtfertigt).

### D: Finaler Detailplan vor dem Rustedit und zweite Lückenprüfung

| Meilenstein | Dateien und Abhängigkeit | Erwartetes, später remote prüfbares Ergebnis |
|---|---|---|
| L02-M1: Gemeinsamer Aufgabenabschluss | Neue `batches_rate_reference_tasks.rs`; bestehende Tokio-Sync-/Joinverträge | Noch laufende Messfutures verlassen erst gemeinsam die Verarbeitung und werden danach gedroppt; kein dadurch erzeugter ACK-/Bildkanalfehler. |
| L02-M2: Ehrliche Fehler und vollständiger Besitz | Dieselbe Aufgabenhilfe; alle eigenen Handles bleiben bis zum Join im Besitzer | Bereitliegende Fehler, mehrere frühe Fehler, frühes `Ok(())`, Panics und ein abgebrochener Finishfuture bleiben erkennbar beziehungsweise hinterlassen keine eigene abgetrennte Aufgabe. Fehlerausgabe erst nach sämtlichen Joins. |
| L02-M3: Integration beider realer Messungen | `batches_rate_reference_transport.rs`, `batches_rate_reference_task_tests.rs` | Referenz und Screen verwenden dieselbe Hilfe; unveränderte reale Pipeline/Wire/ACK-Pfade und V7-Ergebnisdateien. |
| L02-M4: Eigene Anti-Regressionsprüfung und Übergabe | Dieser Abschnitt, drei Rustdateien; Main besitzt Auswahl und Remoteausführung | Eigener vollständiger Diff-/Kettenabgleich erhält Profile, Zeitfenster, Rohdaten, Nutzbyteprüfung und sämtliche Orakel. Main erhält exakte Abschlussfallnamen ohne neuen Suiteeinstieg. |

Die zweite Lückenprüfung klärt vor dem Edit: Watch-Subscriber werden vor
dem Spawn erstellt, damit kein Stop verloren geht; `send_replace` erhöht
auch für `()` die Version. Ein zu früh beendeter Future behält sein Ergebnis
bis zum Join; ein Panic schließt das Oneshot, sodass die Abschlussbarriere
weiterlaufen und anschließend den echten Joinfehler sammeln kann. Jeder
noch gejointe Handle bleibt per veränderlicher Referenz im Besitzer, sodass
ein Drop des Finishfutures alle verbleibenden Aufgaben abbrechen kann.
Der normale Messabschluss benutzt keine `abort`-Reihenfolge. Die neue Datei
bleibt unter 500 Zeilen und 50 KiB. Alle vorbereiteten Abschlussfälle laufen
ausschließlich durch Main innerhalb der vorhandenen Remote-Suite.

Eigene spätere Reviewfragen sind vorab festgelegt: keine Freigabe eines
Kanalbesitzers vor vollständiger Barriere; Fehlerpriorität bei gleichzeitig
bereitem Stop; vollständiger Join auch nach erstem Fehler; Drop bei
abgebrochenem Abschluss; echte Mess- und Orakelpfade unverändert; keine
lokale Ausführung und keine Änderung außerhalb der freigegebenen Fläche.

### Umsetzung und eigene anschließende Anti-Regressionsreview

L02-M1 bis L02-M4 sind innerhalb der ausdrücklich freigegebenen Schreibfläche
vorbereitet. Die eigene Review nach dem Rustedit hat derselbe Autor an der
gesamten oben beschriebenen Kette durchgeführt. `Tasks` liegt jetzt in der
eigenen kleinen Aufgabenhilfe; `transport` exportiert denselben Namen an
seine bisherigen Konsumenten. `finish` liefert nach allen Joins ein
`io::Result<()>`; `reference` und `screen` verlangen dessen Erfolg mit
eigenem Kontext. Kein zurückgegebenes Fehlerkind und kein Fehlertext wird
als harmloser Abschluss ausgefiltert. Nur der ausdrücklich gewählte
Stopzweig zählt als regulärer Abschluss; selbst ein unerwartetes `Ok(())`
des Messfutures bleibt ein Fehler.

| Geprüfte Grenze | Eigener anschließender Befund | Beleg und praktische Grenze |
|---|---|---|
| Kanalbesitz beim regulären Abschluss | Gepinnte Messfutures bleiben bis nach jeder Bereitschaftsbestätigung erhalten. Erst die zweite Watchversion erlaubt ihren Drop. Kein eigener ACK-/Bildkanalbesitzer wird durch diesen Abschluss vor einem noch verarbeitenden direkten Messfuture freigegeben. | Vollständige eigene Aufgabenhilfe und alle acht Spawnstellen beider Messpfade gelesen; Laufzeitbestätigung bleibt remote. |
| Frühe Fehler und echter ACK-Verlust | Die Ergebnisalternative steht vor dem Stopzweig. Mehrere echte Rückgaben werden vollständig gesammelt; derselbe Text `wire ACK stream closed` bleibt bei einem vorher geschlossenen Kanal ein Fehler. Ein Panic schließt seine Bereitschaftsseite und erreicht anschließend den Joinfehler. | Eigene Quellprüfung gegen Tokio 1.52.3 und vorbereitete gezielte Abschlussfälle. Keine pauschale Behandlung von `BrokenPipe`, `Cancelled` oder Kanaltexten als Erfolg. |
| Vollständiger Join und Abbruchbesitz | Jeder Handle bleibt während des gesamten Await im Besitzer. Ein erster Fehler beendet den Joinloop nicht. Drop des Besitzers oder eines noch wartenden Finishfutures fordert den Abbruch aller verbliebenen Handles an. | Eigene Quellprüfung und Drop-Wächterfälle; ein erfolgter `abort` allein wird nicht als bereits gejoint behauptet. |
| Pipelinearbeit und Admission | `host_pipeline_init` bindet Peerbudget, Quelle und Contract; `host_capture::produce` verwendet den bestehenden HostStage-Zweig. `WorkControl` erhält seine Epoch-/Interruptsignale; `PipelineActor::drop` behält den vorhandenen Job-/Disposeweg. | Die explizit nachfreigegebenen Definitionsdateien wurden vollständig selbst gelesen. L02 ergänzt keine neue Produktionssperre, keinen Mutex um Messpolls und keinen produktiven Cancelpfad. |
| ACK, Rate, Pacing und Steuerreserve | `host_mode::on_ack/second`, `SenderBudget::{admit,ack,reset}` und die Permit-Drops, `Pacer::on_ack`, Ratecontroller und `control_sender` bleiben bytegleich. Derselbe echte ACK-Zeitstempel erreicht denselben Regler; Geschwisterdebt und peergeteilte Steuerreserve bleiben erhalten. | Statischer Byteabgleich mit Kandidat `0425aa46`; bestehende Senderbudgetfälle bleiben unverändert in Mains Auswahl. Ein neuer Leistungs- oder Fairnesspass wird nicht behauptet. |
| Reale Pfade und Zeitfenster | Der vollständige Screenfixturekörper entspricht dem Kandidaten nach Abzug von Modulanschluss und Finish-Ergebniskontext. Sämtliche Transportfunktionen ab `network` entsprechen ihm nach Abzug desselben Ergebniskontexts. | Statischer Textabgleich: beide Gleichheitsprüfungen wahr. Die drei Profile, Kalibrierung, fünf/zwölf/zwei Sekunden, echte Wire-/Payloadprüfungen und Beobachtungsquellen sind damit erhalten. |
| Leistungsorakel und Rohdaten | `batches_rate_reference_metrics.rs` ist bytegleich mit dem Kandidaten. Rohdateinamen, Berichtsstruktur, Auswertungsaufruf und Netzfixturekennung sind unverändert. | Statischer Byte-/Textabgleich. Das tatsächliche P27-Leistungsergebnis bleibt bis zum Remote-Lauf unbekannt. |
| Oberflächen und Veröffentlichung | Die Aufgabenhilfe wird ausschließlich über den vorhandenen Linux-Testanschluss eingebunden. Keine produktive Desktop-, Android-, CLI-, IPC-, MCP-, Netz-, Rechte-, Retry-, Backup- oder Konfliktfunktion wurde geändert. Main behält die Veröffentlichung des vollständigen Gesamtbatches. | Vollständiger eigener Diff und unveränderte gelesene Produktquellen. Keine Behauptung einer neuen Benutzerfunktion oder einer abgeschlossenen Veröffentlichung. |
| Dokumentation und Schreibfläche | Der frühere G3-Bericht bleibt als unveränderter Dateipräfix erhalten; nur dieser klar bezeichnete L02-Abschnitt wird ergänzt. | Statischer Präfixvergleich wahr; eigener Rustcode bleibt in genau drei freigegebenen Pfaden. |

Die eigene Review hat den Fehlerkontext auf die genaue Aussage
`measurement task … returned` begrenzt: Ein im letzten Poll vor der
Bereitschaft beobachtetes Ergebnis bekommt keinen erfundenen Zeitstempel
„vor dem Stoppsignal“. Mains Zwischenhinweis zur üblichen Formatierung des
kompakten `failures.push(format!(...))` wurde manuell übernommen; kein
Formatter wurde gestartet. Die Review fand danach keinen weiteren belegten
Fehler in der zugewiesenen Änderung.

Zulässige statische Nachweise dieses Autors: vollständiger eigener Diff,
gezielte Quelllektüre, Byte-/Textabgleiche mit dem Fehlerkandidaten,
Whitespaceprüfung einschließlich der neuen Datei, Dokumentationskontextgate
und `git diff --check`. Die drei Rustdateien haben 261/352/342 Zeilen und
9.032/15.206/12.529 Bytes in der Reihenfolge Aufgabenhilfe/Screenfixture/
Transport. Damit bleiben alle unter 500 Zeilen und 50 KiB. Es gab keinen
lokalen Compiler-, Formatter-, Build-, Test-, Server-, Installations- oder
Netzprofillauf, keinen Commit/Push und keinen Graphrefresh dieses Autors.

### Exakte vorbereitete Fallnamen und Registrierung durch Main

Voller gemeinsamer Namenspräfix der neuen nicht ignorierten Linux-Fälle:
`screen::core::rate_control::p27_acceptance::tasks::tests::`.

1. `p27_task_shutdown_preserves_ack_and_picture_dependencies_until_quiesced`:
   verbundene Sender und Empfänger schließen ohne selbst erzeugten
   ACK-/Bildkanalfehler; sämtliche eigenen Kinder geben ihren Besitz frei.
2. `p27_task_shutdown_preserves_ready_transport_and_pipeline_failures`:
   beide bereits bereiten echten Fehler bleiben bei gleichzeitigem
   Stoppsignal erhalten; verbliebene Kinder sind vor Rückgabe gejoint.
3. `p27_task_shutdown_reports_ack_loss_before_explicit_stop`:
   ein tatsächlich vorher geschlossener ACK-Kanal bleibt trotz identischer
   Fehlerzeichenfolge ein Fehler.
4. `p27_task_shutdown_rejects_early_success`:
   ein vorzeitig beendeter Messfuture mit `Ok(())` wird nicht als
   regulärer angeforderter Abschluss ausgegeben.
5. `p27_task_shutdown_preserves_panic_and_joins_remaining_children`:
   eine tatsächliche Panic bleibt sichtbar, und die übrigen eigenen
   Kinder sind vor der Fehlerrückgabe gejoint.
6. `p27_task_shutdown_drop_aborts_owned_children`:
   Drop des Aufgabenbesitzers bricht seine noch gehaltenen Kinder ab.
7. `p27_task_shutdown_cancelled_finish_keeps_child_ownership`:
   Drop des bereits angefangenen, noch wartenden Finishfutures trennt
   dessen Kindbesitz nicht vom Aufgabenbesitzer ab.

Main registriert diese sieben Suffixe ausschließlich über die vorhandenen
Linuxintegrationen der gemeinsamen Task-Suite. Die allgemeine Präfixauswahl
enthält sie bisher nicht. Es gibt keinen zusätzlichen Suiteeinstieg und
keinen zusätzlichen Aufruf des weiterhin exakt gleich benannten ignorierten
Netzfalls
`screen::core::rate_control::p27_acceptance::p27_network_reference_acceptance`.
Die Windowsauswahl erhält keine Linux-Testnamen. Alle vorbereiteten Fälle
und das tatsächliche Leistungsresultat sind bislang nicht ausgeführt.

### Tatsächlich gelesene und bearbeitete Dateien dieses Autors

Die Liste nennt eigene vollständige Lektüre oder gezielte Quellen-/
Signaturauszüge; sie übernimmt keine Lesebehauptung des G3-Autors.

- Regeln/Auftrag: `/root/.codex/AGENTS.md`,
  `/root/.codex/skills/arbeitsweise/SKILL.md`,
  `/root/.agents/skills/arbeitsweise/SKILL.md`, die vollständig über MCP
  gelieferte Arbeitsweise, `/tmp/smartexplorer-final-review-user-contract.txt`
  und `/tmp/smartexplorer-fix-p27-shutdown-scope.json`. Die geltenden
  Repositoryanweisungen lagen außerdem vollständig in der Nutzernachricht vor.
- Dokumentation: `docs/ARCHITEKTUR.md`,
  `docs/oberflaechen/bildschirm.md`, dieser Bericht,
  `docs/plaene/2026-10-24-batches/rv1-compilerplan.md` und
  `docs/plaene/2026-10-24-batches/vertraege.md`.
- Konfiguration/Suite: `native/Cargo.toml`, `native/Cargo.lock`,
  `native/review-task-manifest.json`, `native/review-task-native.py`,
  `native/review-task-network.sh`, `native/test-review-task.sh` und
  `.github/workflows/review-task.yml`.
- Unter `native/src/screen/core/`: `batches_rate_reference_task_tests.rs`,
  `batches_rate_reference_transport.rs`, eigene neue
  `batches_rate_reference_tasks.rs`, `batches_rate_reference_metrics.rs`,
  `batches_sender_task_tests.rs`, `capture_admission.rs`, `capture_plan.rs`,
  `capture_records.rs`, `control_sender.rs`, `frame_budget.rs`,
  `gop_stream.rs`, `host_backend.rs`, `host_frame_send.rs`,
  `host_pipeline.rs`, `host_pipeline_actor.rs`, `host_pipeline_produce.rs`,
  `host_stage.rs`, `rate_control.rs`, `wire_control.rs` und
  `wire_control_codec.rs`; nach expliziter zusätzlicher Lesefreigabe
  `host_pipeline_init.rs`, `host_work_control.rs`, `host_capture.rs`,
  `sender_budget.rs`, `pacing.rs`, `host_quality.rs`, `host_reference.rs`
  und `host_mode.rs`.
- Unter der freigegebenen lokalen Tokio-1.52.3-Registryquelle:
  `src/macros/select.rs`, `src/runtime/task/join.rs`,
  `src/sync/watch.rs` und `src/sync/oneshot.rs`.
- Remote-Evidenz unter `/tmp/smartexplorer-rv1-40381452096/`:
  `linux/p27-network.log`, `linux/summary.json` und
  `linux/p27-network/10mbit-one-window-reference.json`.

Erstellt wurde ausschließlich
`native/src/screen/core/batches_rate_reference_tasks.rs`.
Geändert wurden ausschließlich
`native/src/screen/core/batches_rate_reference_task_tests.rs`,
`native/src/screen/core/batches_rate_reference_transport.rs` und der eigene
L02-Abschnitt in
`docs/plaene/2026-10-24-batches/fix-review-p27-abnahme.md`.
Keine Schreibfläche der parallelen Android-Autoren wurde berührt.

### Entscheidungen und verbleibende Grenzen

Der neue eigene Fünfteilzyklus ist als Quelländerung und statische eigene
Anti-Regressionsreview vorbereitet. Der genaue Entstehungszeitpunkt des
alten ACK-Fehlers bleibt im vorhandenen Log unbekannt; der neue Abschluss
beseitigt das belegte eigene Abbruchrennen und verschweigt keinen echten
Messfuturerücklauf. Die fehlende Laufzeitbestätigung ist keine bestandene
Prüfung. Main übernimmt dieselbe Remote-Suite, die Auswertung ihrer
unveränderten V7-Orakel, Graphrefresh, Commit/Push und den einen terminalen
vollständigen Remote-Release des Gesamtbatches. Produktinterne asynchrone
GOP-Reader und Pipeline-Disposejobs behalten ihren bisherigen Lebensweg;
die neue Gruppe joint ausschließlich die von ihr selbst gestarteten
Messaufgaben. Außerhalb der Fläche ist kein zusätzlicher belegter
Reparaturbedarf offen. Dieser Autor stoppt nach der Übergabe.
