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
