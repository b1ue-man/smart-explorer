# M06: F36/F37 – Workerpayload und Windows-Capturebudget

Stand: 2026-10-26. Eigener vollständiger Fixzyklus des für M06 beauftragten
Capturebudgetautors. Fehlerevidenz: abgeschlossener Lauf `40406246032`,
Quellkandidat `1ffbf4b0bc90b2479947680abcd0a5941cce00f8`; isolierter
Arbeitsstand vor eigener Änderung `33257f82`. Keine Laufzeitfreigabe.

## A – Erstplan aus dem Ergebnisauftrag

Der Auftrag umfasst F36 und F37 vollständig: Workerpakete müssen vor ihrem
Payloadlesen am tatsächlichen Ticket-/Speichervertrag geprüft werden;
Windows-Captureresize muss vor nativer Allokation die neue Generation und alle
noch lebenden Altbesitzer vollständig finanzieren. Ein passender Ersatz darf
nach tatsächlichem alten Abschluss nicht durch eine veraltete Vollplanannahme
verhindert werden. Produktfehler, fehlerhafte Fixtureannahmen und ungeprüfte
Hypothesen werden getrennt belegt. Kein Prüffall und keine Schutzanforderung
darf entfernt oder durch bloße schwächere Assertions ersetzt werden.

Dieser selbe Autor verantwortet Fehlerfunktionsliste, vollständige tatsächliche
Kette, vorherigen konkreten Erhaltungsplan, Änderung und nachfolgende eigene
Anti-Regressionsreview. Die neue
`/tmp/smartexplorer-f-capture_budget-scope.json` ersetzt alle vorherigen
Scopefreigaben. Nur die dort jeweils ausdrücklich genannten Dateien sind schreibbar. Dieser
Bericht ist die einzige neue Berichtsdatei für M06; Main besitzt das gemeinsame
Manifest, Git, Graphaktualisierung, Remoteausführung und Release.

Vor der Codelektüre konkret zu klären sind die heute zuerst wirksame
Workerprüfung, das tatsächliche Paketformat mit Captureticket, die reservierten
Bytes einer lebenden alten Allocation nach Retirement, die neue
Generationszulassung und die Aufrufreihenfolge vor WGC-/DXGI-Allokation.
Die Remoteassertions belegen bisher keinen separaten Produktdefekt. Noch ist
offen, ob beide Änderungen ausschließlich Fixtures betreffen oder eine
Produktkorrektur nötig ist. Planungstiefe ist Stufe 2 mit verknüpfter
Besitz-/Parsergrenze. Es gibt keine durch den Benutzer fehlende Angabe.

Vor dieser Berichtanlage wurden der eigene Dateistatus, der jüngste Commit,
F36/F37 in den vollständigen Diagnoseeinträgen und die zugehörigen Abschnitte
des abgeschlossenen `rv1-funktionsplan.md` gelesen. Kein eigener Quellpatch
bestand zu diesem Zeitpunkt. Lokale Builds, Compiler, Tests, Formatter,
Server, Installationen, Unteragenten, Gitmutationen und Graphupdates sind
ausdrücklich ausgeschlossen. Die Annahmesignale werden vorbereitet und durch
Main ausschließlich über denselben bestehenden Remote-Einstieg bestätigt.

## B – Bestandsbefund und Funktionskarte

### Belegte fehlerhafte Funktionen und klare Abgrenzung

1. `batches_task_b6_worker_rejects_negotiated_packet_before_payload_read`
   konstruiert unbesessene `RAW`-/`ENCODED`-Pakete und erwartet die spätere
   PRODUCE-Budgetdiagnose beziehungsweise beim privaten Legacyleser ein
   `UnexpectedEof`. `worker_wire::read_message_with_limits` weist diese Tags
   bereits nach dem 5-Byte-Kopf mit der fehlenden Owned-Ticketvoraussetzung ab,
   sowohl mit als auch ohne versionierte Limits. Die beiden beobachteten
   Fehlassertions auf Linux/Windows betreffen diese veraltete Fixtureannahme.
   Die heutige tatsächliche Bildgrenze liegt in
   `capture_transport::{read_header, read_owned, decode_owned}` mit vorher
   zugelassenem Capturegrant und fester 58-Byte-Identitätspräambel. Der Fall
   muss diesen Weg und die unverändert sichere Abweisung unbesessener Bilder
   zusammen prüfen; eine geänderte erwartete Zeichenfolge allein genügt nicht.
2. `batches_capture_windows_resize_rejects_before_native_allocation_with_old_owner`
   hält eine `SourcePixels`-Slice von genau einem Byte und behauptet im Kommentar,
   diese halte nach `CaptureBudget::suspend` den gesamten alten Plan reserviert.
   `CaptureGrant::retire` reduziert dagegen nachweislich `usage.reserved` auf
   `usage.total`; `SessionInner::reduce_plan` gibt nur die unbenutzte Plankapazität
   zurück. Ein altes Byte plus ein allein passender Ersatzplan passt hier in die
   vorhandenen 512 MiB. Die beobachtete erfolgreiche Ersatzgenehmigung ist daher
   kein belegter Überbelegungsfehler. Der Fall muss einen tatsächlichen lebenden
   alten Ressourcen-/Bildbesitz finanzieren und eine daraus hergeleitete knappe
   Prozessgrenze verwenden.

3. Die vollständige tatsächliche Aufruferkette belegt zusätzlich eine
   Produktlücke: `WorkerReader::start` verwendet `capture_transport::read_header`
   und liest bei jedem Tag außer `OWNED_REFERENCE` pauschal
   `capture_transport::read_control(length)`. Erst danach folgt
   `WorkerReply::decode`. Damit werden unbesessene `RAW`-/`ENCODED`-Bytes
   unterhalb des Kontrolllimits bereits gelesen und allokiert, obwohl der
   geprüfte allgemeine Workerleser diese Bildtypen vorher abweist. Ebenso
   durchläuft ein `REFERENCE`-Umschlag die eigentliche innere Bildtypprüfung
   erst nach seinem vollständigen Payloadlesen. Die gegenläufige
   `capture_dispatch::start`-Schleife verwendet denselben untypisierten
   Kontrollleser. Diese direkte Produktlücke ist von den beiden veralteten
   Fixtureannahmen getrennt; Main hat sie im aktuellen Quellstand bestätigt.
4. `worker_frame_wire::read_reference_payload` schließt verschachtelte
   `REFERENCE`, unbesessene `RAW` und `ENCODED` bereits nach der festen
   Präambel aus. Der neue Bildtag `OWNED_REFERENCE` fehlt in dieser
   Ausschlussmenge. Dieser innere Bildtag darf keinen nicht finanzierten
   Kontrollumschlag erzeugen. Seine Ablehnung muss vor dem inneren Body gelten.

An `CaptureBudget::{suspend, replace, claim_native}`,
`CaptureGrant::{retire, claim_if_active}` und `SharedAuthority::fund` ist kein
zu F37 gehöriger Produktfehler belegt. Ihr tatsächlicher Retirementvertrag
bleibt unverändert. Produktkorrekturen sind für die unter 3 und 4 belegte
Workerlesekette erforderlich. Die konkret zusätzlich benötigten Schreibpfade
`capture_transport.rs`, `worker_frame_wire.rs`, `host_worker_reader.rs` und
die drei `read_control`-Aufrufstellen in `capture_dispatch.rs` wurden vor
der Umsetzung ausdrücklich von Main freigegeben.

Der vorhandene Graph wurde ausschließlich mit einer Leseabfrage genutzt. Er
enthält die tatsächlichen Symbole `read_message_with_limits`, `CaptureBudget`,
`CaptureAuthority`, `replace` und `retire`; die daraus benannten Quellgrenzen
werden gegen die ausdrücklich freigegebenen tatsächlichen Dateien geprüft.
Eine Graphkante ersetzt keinen Nachweis der Allokations-/Abschlussreihenfolge.

### Vollständige direkte und indirekte Karte der betroffenen Grenze

| Grenze | Gelesene tatsächliche Funktions-/Besitzkette und Erhaltungsvertrag |
|---|---|
| Allgemeiner Workerrahmen | `worker_wire::read_message` → `read_message_with_limits` → Taglimit → Owned-Ticketvoraussetzung → bei `REFERENCE` feste 42-Byte-Präambel → vorherige innere Tag-/Längenprüfung → erst danach Bodyallocation/Read. Diese Reihenfolge wird als gemeinsamer Payloadleser wiederverwendet. `write_message` behält sämtliche Taglimits und das bestehende 5-Byte-Format. |
| Echter Parentleser | `host_worker_client::WorkerLink` → `WorkerReader::start` → `capture_transport::read_header` → für `OWNED_REFERENCE` der vorhandene `read_owned`-Zweig, für Kontrolltypen der neue gemeinsame Payloadschutz unter dem bisherigen Kontrollgrößenlimit. Erst ein akzeptierter vollständiger Rahmen geht an `WorkerReply::decode` und die bestehende Antwortqueue; `reader_ended` bleibt erhalten. |
| Tatsächlicher Owned-Empfang | `read_owned` → feste 58 Byte → Ticket-ID-Lookup `CaptureBroker::grant` → gleiche Capturegeneration → aus angenommenem Vertrag beziehungsweise privatem Legacyquelllimit abgeleitete Bodygrenze → `AdmittedBytes::new(Transport)` → `CaptureGrant::claim` → erst danach Vecallocation und Bodyread → `decode_owned`. Eine große Präambelbehauptung ersetzt weder den angenommenen v2-Vertrag noch den tatsächlichen Grant. |
| Tatsächliche Bilddecoder | RAW: `decode_owned` → Geometrie-/Längenprüfung → zugelassene `CompositionPixels` → `CaptureImageBuilder::from_bytes`. ENCODED: `decode_records` → Recordanzahl, Einzelgröße, Summe und vollständiger Body → `CapturedRecords::from_allocator` mit Output- und Metadataclaim. Beide Ergebnisse behalten ihre ursprüngliche Quellen-/Capture-/Referenzgeneration. |
| Tatsächlicher Sender | `Worker::prepare_reply` → erfasste Referenz vor Produktion → `picture_grant` → `RemoteAuthority::id_for` → `prepare_owned` mit `claim_if_active(Transport)` → `PreparedReply` → `send_prepared`/`write_message(OWNED_REFERENCE)`. Bild und Transportbesitz überleben den Outputlock. Ein inzwischen retirierter Producer erhält weiterhin die vorhandene nicht allokierende Warteantwort. |
| Workeranträge und Rückkanal | `RemoteAuthority` bindet Session, Quelle und konkrete Auswahl; `capture_dispatch_requests::request` führt Quelle/Antrags-ID und Pendingbesitz bis zur echten Antwort. `GrantRequest::FramePlan` → `CaptureBroker::resource`/`approve` → disjunkte Parent-/Workerpläne → dieselbe `SessionAdmission`. `RemoteAuthority::import` prüft Quelle/Generation/Credit-ID und bindet das echte lokale Grantbuch. |
| Dauerhafter Childdispatcher | `capture_dispatch::start` → `read_header` → derselbe typisierte Kontrollleser → `GRANT_REPLY` an `RemoteAuthority::deliver` oder unveränderter `WorkerRequest::decode`. Nur die drei bestehenden Kontrollleseraufrufe dieser Datei werden angepasst; Pending-, Abbruch-, Supplement-, Import- und Retirementlogik bleibt erhalten. |
| Ressourcenabschluss | `CaptureGrant::retire` und `CaptureSlice::drop` → exakte Owner-/Livebytebilanz → `SessionInner::reduce_plan`; delegierter Abschluss publiziert monotone `REDUCED`-Nachrichten, finaler `Retirement::drop` publiziert `RETIRED`. Der Broker hält Worker-, Parent- und noch lebende Besitzer getrennt. Fehlender Readerabschluss ist kein Prozessende; `host_worker_lifetime::retire` hält den Child und Kredit bis zu echtem `try_wait`-Exit. |
| Gemeinsame Prozessgrenze | `MemoryPool::admit_session` → begrenzte W-Lease und `maximum_total` → `SessionAdmission`/`SharedAuthority::fund` → `max(W, Summe der noch reservierten Pläne)` gegen dieselbe Prozessgrenze. Retirement reduziert ausschließlich unbenutzte Plankapazität; lebende Slices und sämtliche Mitglieder behalten dieselbe gemeinsame Bilanz. |
| Windowsvorbereitung | `PreparedWindows::capture_plan` und `activate` → konkrete Quelle, Generation, Geometrie und `HostOutputPlan` → `CaptureGeometry::plan` → `CaptureBudget::activate` vergleicht Quelle, Generation, Anforderungen, Phasen und Ownerlimits. `HostOutputPlan`/`host_codec_phase_plan` enthalten bestehende Tiles-/Codec-, Snapshot-, Parent-/Worker-, Transport- und Konvertierungsbesitzer einschließlich möglichem Fallback. |
| WGC-Resize/-Resume | `WgcCapture::restart` → `stop` → tatsächliches `CaptureResources::close` → Readback-/Cachefreigabe → `budget.suspend` → neue Metadaten → `budget.replace` → `claim_native` → `CaptureResources::create`/`CreateFreeThreaded`. Ein fehlgeschlagener nativer Close hält Objekte und Slice im bestehenden Retirementbesitzer; es wird kein unbewiesener Abschluss in freie Bytes umgedeutet. |
| DXGI-Resize/-Resume | `DxgiCapture::reopen` → alter Live-/Readback-/Cacheabbau → `budget.suspend` → erneut geprüfte Geometrie → `budget.replace` → `duplicate`/`claim_native` → `DuplicateOutput`. Rotation und tatsächliche Source-/Referenzgeneration bleiben erhalten. Der bestehende `CaptureRecovery`-Weg behandelt Retry, Pause und Wiederkehr; diese Produktkörper werden nicht geändert. |
| Retirierte Ergebnisbilder | `CaptureImageBuilder::freeze` → `CapturedImage`/`Arc<PixelStorage>`. Cache und Resultat sind Aliasse derselben Allocation. Die letzte Aliasfreigabe zerstört Pixel vor Slice/Grant; `AdmittedResource` zerstört seine tatsächliche Ressource vor dem zugehörigen Nativeclaim. Ein bloßes Grantclone hält die Session, reserviert nach Retirement aber keinen alten Vollplan. |
| Betroffene Fixtures | F36 und sein bestehender Referenzbegleitfall werden auf allgemeinen und tatsächlichen Kontrollleser sowie echten Owned-Transport gebunden. F37 verwendet tatsächliche kleine Bild-/Ressourcenallocations, Cachealias, begrenzten Prozesspool und aus demselben produktiven Plan berechnete Grenzwerte. Die vorhandenen Integration-, Ownership-, Rotation-, WGC-/DXGI- und Wiederaufnahmefälle behalten Namen und Aussagen. |

## C – Quellenabgleich, Alternativen und endgültige Entscheidung

Primärquellen wurden am 2026-10-26 tatsächlich abgerufen:

- [Rust `Read::read_exact`](https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact):
  Ein fehlender Body erzeugt beim tatsächlichen Zugriff `UnexpectedEof`;
  `Interrupted` wird von `read_exact` wiederholt. Eine eigene Read-Fixture
  zählt zusätzlich jeden Zugriff nach der zugelassenen Präambel, damit eine
  späte Dekodierablehnung keine vorgezogene Speicherprüfung vortäuschen kann.
- [Rust `Cursor`](https://doc.rust-lang.org/std/io/struct.Cursor.html):
  Ein Cursor um vorhandene Kopfbytes liefert dieselbe Read-Schnittstelle und
  macht die tatsächlich verbrauchte Kopf-/Präambellänge beobachtbar.
- [Rust `Arc`](https://doc.rust-lang.org/std/sync/struct.Arc.html) und
  [Rust-Referenz zu Destruktoren](https://doc.rust-lang.org/reference/destructors.html):
  Ein Clone teilt die Allocation bis zum letzten starken Besitzer; Structfelder
  werden in Deklarationsreihenfolge zerstört. Das passt zum gelesenen
  `CapturedImage`, `AdmittedResource` und den darin gehaltenen Slices.
- [Microsoft `CreateFreeThreaded`](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.direct3d11captureframepool.createfreethreaded?view=winrt-26100)
  beschreibt die Zahl und Geometrie der angeforderten Capturepuffer.
  [Microsoft `DuplicateOutput`](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgioutput1-duplicateoutput)
  bindet die Desktopduplikation an den Adapter des übergebenen Geräts.
  Die vorhandenen WGC-/DXGI-Aufrufe bleiben an ihren tatsächlichen, vorher
  zugelassenen Nativeclaims; es wird keine neue Plattform-API eingeführt.

Gewählt wird ein gemeinsamer privater `worker_wire::read_payload_with_limits`.
Der allgemeine Workerrahmenleser und der echte typisierte Kontrollleser rufen
diesen vor jeder Bodyallocation auf. `capture_transport::read_control` erhält
Tag und optionale angenommene Limits; seine bisherige Kontrollgrößengrenze
bleibt als zusätzliche Grenze erhalten. Nur der bestehende explizite
`OWNED_REFERENCE`-Zweig des Parentlesers darf Bilddaten über `read_owned`
finanzieren. Eine verzögerte `WorkerReply::decode`-Ablehnung oder eine separate
duplizierte Tagliste im Parentleser würde den Schutzweg weiterhin aufspalten.
Kontroll-/Referenzmetadaten können schon bei Vorbereitung und Ablehnung vor
der endgültigen Bildvertragsbindung eintreffen. Beide permanenten Kontrollleser
übergeben deshalb `None`; ein neuer erzwungener `broker.contract()`-Aufruf für
solche Metadaten würde bestehende Vorbereitungsfehler überschreiben. Der
explizite Owned-Zweig liest seinen tatsächlichen gebundenen Vertrag unverändert.
Eine neue zweite Ressourcenbank würde die gemeinsame Prozessbilanz aufheben.
Diese Alternativen werden verworfen.

Die feste Referenzpräambel behält sämtliche Felder und bisherigen Grenzen;
`OWNED_REFERENCE` wird zusätzlich zu den bereits verbotenen inneren Bildtags
vor dem Body abgewiesen. Der vorhandene Legacyvertrag bleibt ein Legacyvertrag.
Es ändern sich keine Tags, JSON-Felder, Quellenbedeutungen, Wireversionen,
Eingabeoperationen oder Codec-/Fallbackangebote.

Für F37 wird der produktive Plan unverändert genutzt. `P` ist der ursprüngliche
Plan, `Q` der Plan nach einer echten Geometrieänderung, `I` die Bytes des alten
Bildes und `N` die Bytes seiner WGC-Nativeressource. Die Fixture prüft beide
Seiten derselben geforderten Grenze mit `B = Q + I - 1` und `B = Q + I`.
Vorher wird ausdrücklich nachgewiesen, dass P und Q jeweils allein passen.
Ebenso wird P+Q>B ausdrücklich als Fixturevoraussetzung geprüft.
Solange N+I leben, muss Ersatz vor dem neuen Nativeallocator scheitern.
Nach echtem Nativeabschluss bleibt das alte Cachebild I unverändert bilanziert:
Bei einem Byte zu wenig scheitert Ersatz bis zum letzten Aliasende; bei genau
passendem B gelingt er trotz noch lebendem altem Bild. Der neue Nativeallocator
darf erst nach erfolgreicher Q-Zulassung laufen. Ein erneutes Reservieren von
P+Q oder ein Verbot jedes noch lebenden Altbesitzers würde gültige Fortsetzung
brechen und wird ausdrücklich nicht implementiert.

Der zweite Planlückenabgleich hat die zuerst nur als Fixturefehler erkannten
Befunde über die echten permanenten Reader verfolgt und dabei die unter B.3
belegte Produktlücke geschlossen. Er umfasst außerdem die direkte
`capture_dispatch`-Signaturfolge, tatsächliche Broker-IDauflösung, delegierte
Grantanträge, alle alten Native-/Bild-/Aliasenden, den tatsächlichen
Windowsantrag vor der Nativeallocation und die bestehende
Remote-Manifestzuordnung. Die dafür konkret fehlenden Dateien wurden vor
dem Lesen/Ändern durch Main ergänzt. Eine ungeklärte Quellfreigabe besteht
für den folgenden Plan nicht mehr.

## D – Konkreter Plan vor der Umsetzung

| Schritt | Konkrete Änderung / erhaltene Abhängigkeit | Vorher festgelegtes Abnahmesignal |
|---|---|---|
| 1 | `worker_wire.rs`: vorhandene Payloadprüfung in einen gemeinsamen privaten Leser extrahieren; `read_message_with_limits` delegiert nach demselben 5-Byte-Kopf. | Absolute Grenzen, EOF-/Interrupted-Behandlung und vorgezogene Abweisung nicht besessener Bilder gelten unverändert. |
| 2 | `capture_transport.rs`: Tag und optionale Limits an `read_control` binden, Kontrollgrößengrenze erhalten und gemeinsamen Leser aufrufen. `host_worker_reader.rs` übergibt für reine Metadaten `None`, damit bereits vor Vertragsbindung eine Vorbereitungsantwort möglich bleibt; der echte Vertragslookup im Owned-Zweig bleibt unverändert. | Der echte Parentleser fordert kein unbesessenes Bild oder inneres Bildpayload an. `GRANT_REQUEST`, `REDUCED`, `RETIRED`, Kontrollantworten und die permanente Rückkanalfunktion bleiben erhalten. |
| 3 | Ausschließlich die drei genehmigten `read_control`-Aufrufe in `capture_dispatch.rs` und den von Mains vollständiger Aufrufsuche zusätzlich belegten Aufruf in `batches_capture_worker_fix_task_tests.rs` an Tag/`None` anpassen; `worker_frame_wire.rs` verbietet auch den inneren Owned-Bildtag vor dem Body. | Gegenrichtung und vorhandene Supplement-/Retirementfixtures behalten sämtliche Aussagen und Operationen. Die 42-Byte-Prüfung erfolgt vor Bodyallokation. |
| 4 | Bestehenden F36-Fall in `batches_frame_budget_task_tests.rs` über allgemeinen und tatsächlichen Kontrollleser sowie echte Owned-Grants führen; bestehende Referenzfixture erhält den inneren Owned-Tag als gleichartige verbotene Bildvariante und eine gültige frühe Ablehnungsmetadatenantwort. | Unbesessene Bilder und überbudgetierte Owned-Bodys rufen keinen Payloadread auf, verändern keine Grantlivebytes und geben keine Allocation frei. Gültige kleine RAW-/ENCODED-Bilder passieren im versionierten und Legacyvertrag mit derselben tatsächlichen Transport-/Decoderbesitzkette; reine Referenzmetadaten benötigen keinen bereits gebundenen Bildvertrag. Kein Fallname entfällt. |
| 5 | Bestehenden F37-Fall in `batches_capture_task_tests.rs` mit kleinen realen Ressourcen-/Pixelallocations, tatsächlichem Cachealias, hergeleitetem B und echter Geometrieänderung ersetzen. | P und Q passen jeweils allein. N+I+Q>B verhindert Genehmigung und neuen Nativeallocator. Nach echtem Ende darf I+Q=B gelingen; I+Q>B bleibt bis zum letzten Aliasende verweigert. Quelle, neue Generation, endgültiges Ressourcenende und gemeinsamer Pool bleiben geprüft. |
| 6 | Derselbe Autor prüft alle geänderten Funktionen und die vollständige obige Kette nach Umsetzung; statische Rustgrammatik, Größen, Fallnamen und unveränderte verknüpfte Produktkörper werden verglichen. | Normale Formatierung unter 500 Zeilen/50 KiB je wesentlich geänderter Rustdatei; vollständige Datei-/Fallliste, begründete Ergebnisse und klare Laufzeitgrenzen im Bericht. |

Der Plan steht vor der ersten eigenen Produkt-/Fixtureänderung. Die einzige
praktische Bestätigung bleibt Mains derselbe Remote-Suiteeinstieg. Dieser Autor
führt weder Builds noch Fixtures, Compiler, Formatter oder eine neue Suite aus.

## E – Umsetzung und eigene anschließende Anti-Regressionsreview

Alle sechs geplanten Schritte sind im zugewiesenen Autorumfang umgesetzt.
Die eigene Nachreview erfolgte nach der vollständigen Änderung durch denselben
Autor gegen die obige Quellkette und die genannten Primärreferenzen.

Die Produktkorrektur besteht aus dem gemeinsamen Payloadleser, der typisierten
Anbindung des echten Kontrolllesers und der zusätzlichen Abweisung eines
inneren `OWNED_REFERENCE`-Bildes. Der Parentreader ruft für reine Metadaten
weiterhin keinen gebundenen Bildvertrag ab. Sein vorhandener Owned-Zweig
verwendet unverändert den tatsächlichen Brokervertrag und Ticketlookup.
Der Childdispatcher und seine beiden bestehenden Supplement-/Retirement-
Fixtureaufrufe erhalten ausschließlich Tag und `None`. Mains vollständige
gezielte Aufrufsuche über `native` und `vendor` fand außerdem den einen
Aufruf in `batches_capture_worker_fix_task_tests.rs`; auch dort wurde allein
die Signatur angeschlossen. Kein vorhandener Konsument blieb beim alten API.

Die anschließende eigene Prüfung bestätigt folgende konkrete Erhaltungsfälle:

- **Unbesessene Bilder:** Allgemeiner und permanenter Kontrollleser weisen
  RAW/ENCODED vor dem Body ab. Der tatsächliche Kontrollweg wird mit einer
  einzigen angekündigten Bildbyte geprüft, damit keine globale Größenablehnung
  die fehlende Ticketprüfung verdecken kann. Der allgemeine Weg behält den
  ursprünglich über dem angenommenen Bildlimit liegenden Längenfall.
- **Owned-Budget vor dem Body:** Die F36-Fixture erzeugt Präambel und gültigen
  Vergleichstransport mit dem produktiven `prepare_owned` und echten zugelassenen
  Worker-/Parentgrants. Der kontrollierte Ticketlookup erhält die konkrete ID;
  bei `58 + Bodylimit + 1` sind genau 5 Kopfbytes und 58 Präambelbytes verbraucht,
  kein Bodyread erfolgt und keine zusätzliche Parent-Livebytebelegung bleibt.
  Die absichtlich größere Legacybeschreibung in der Präambel kann den kleineren
  angenommenen v2-Vertrag nicht erhöhen.
- **Gültige Bildpfade:** Kleine RAW- und ENCODED-Ergebnisse durchlaufen den
  echten Owned-Sender, Headerleser, Ticket-/Generationscheck und Decoder sowohl
  mit v2-Vertrag als auch als Legacy. Der empfangene Transport ist exakt mit
  seiner Länge bilanziert; das decodierte Ergebnis besitzt zusätzliche echte
  Pixel- beziehungsweise Record-/Metadatabytes. Ergebnisende gibt nur diese
  Besitzer frei, Transportende anschließend dessen restliche Bytes.
- **Referenzgrenze und frühe Antworten:** Nach genau 5+42 Bytes werden RAW,
  ENCODED und OWNED_REFERENCE im nicht besessenen Referenzumschlag abgewiesen.
  Die feste Präambel wird nicht als große Payloadallocation behandelt.
  Eine gültige frühe `Refused`-Referenzantwort wird ohne bereits gebundenen
  Bildvertrag gelesen und unverändert dekodiert. Die bestehenden JSON- und
  Kontrollgrößenlimits bleiben erhalten.
- **Windowsüberbelegung:** F37 berechnet P und Q aus derselben tatsächlichen
  WGC-Geometrie-/Outputplanung wie das Produkt und prüft deren alleinige
  Finanzierbarkeit. Die Nativeersatzressource besitzt einen echten kleinen
  Bytevektor unter `claim_native().allocate`; das Ergebnisbild besitzt echte
  Pixel unter `CaptureImageBuilder`. N+I bleiben nach Suspend vollständig live.
  Ein abgewiesener Ersatz bietet keinen Grant; der nachfolgende Nativeallocator
  wird nachweislich nicht aufgerufen.
- **Tatsächliche alte Enden und gültiger Resize:** Das Ende der alten
  Nativeersatzressource reduziert die Bilanz von N+I auf I. Das Ende nur eines
  Bildaliases reduziert sie nicht. Bei B=Q+I−1 scheitert Ersatz weiter, bis der
  letzte Bildalias endet. Bei B=Q+I gelingt Ersatz bereits mit lebendem altem
  Bild. Dieses behält Quelle, alte Generation und alte Geometrie; der neue
  Grant besitzt dieselbe konkrete Quelle, eine neuere Generation und die neue
  Geometrie. Ein noch lebendes altes Grantdescriptor hält keinen Vollplan P.
- **Neuer Abschluss und gemeinsamer Pool:** Der neue Nativeallocator läuft
  erst nach erfolgreicher Genehmigung. Captureende retirert den neuen Grant,
  während seine tatsächliche Ressource weiter ihre Bytes hält. Auch nach Ende
  der übrigen Session-/Grantdeskriptoren bleibt der Prozesspool dadurch belegt;
  erst der letzte tatsächliche Ressourcenabschluss erlaubt dessen neue Nutzung.
- **Unveränderte Ressourcensteuerung:** Der Owned-58-Byte-Pfad, RAW-/Record-
  Decoder, Sender, Broker, delegierte Claims, monotone Reduktionen, finales
  Retirement und bestätigter Childexit werden nicht verändert. Die einzige
  Änderung in Parentreader und Childdispatcher ist die Anbindung des Lesers;
  es kommen weder ein neuer Lock noch eine zusätzliche Vertragsabfrage hinzu.
- **Unveränderte Windowsfunktionen:** `CaptureBudget`, Grant-/Slicebuchhaltung,
  WGC-/DXGI-Resize, Metadaten-/Quellenprüfung, Readback, Rotation, Pause,
  Secure-Desktop-Rückkehr und Retry behalten ihre Produktkörper. Gelesen wurde
  auch die Freigabe erworbener Frames vor WGC-Poolersatz sowie die
  Readback-Texturfreigabe vor deren Slice. Die vorgezogene Nativezulassung
  bleibt an denselben tatsächlichen Allokationsaufrufen.

Die frühere private Legacy-`UnexpectedEof`-Erwartung wurde ausdrücklich
korrigiert: Sie setzte einen Zugriff auf einen unbesessenen Bildbody voraus,
den bereits der heutige allgemeine Leser verbietet. An ihre Stelle treten
konkrete Ticketablehnung vor jedem Bodyread und erfolgreiche gültige
Legacytransporte. Die frühere Ein-Byte-Vollplanannahme wurde durch beide Seiten
der tatsächlichen N/I/Q-Bilanz ersetzt. Keine echte Größen-, Sicherheits-,
Besitz-, Wiederaufnahme- oder Leistungsanforderung wurde abgeschwächt.

Die statische Rustgrammatikprüfung mit vorhandenem Tree-sitter findet in allen
geänderten Rustquellen weder `ERROR`-Knoten noch fehlende Syntaxknoten.
Der Funktionsvergleich gegen den eigenen unveränderten Ausgang belegt die
geplanten Grenzen: Nur `read_message_with_limits`, `read_reference_payload`,
`read_control`, der Parent-/Childreaderaufruf und die benannten bestehenden
Fixtures ändern sich; `read_payload_with_limits` und lokale Fixturehilfen
kommen hinzu. Kein bestehender Funktions- oder Fallname entfällt. Alle
`read_control`-Aufrufe in der geprüften Fläche besitzen vier Argumente.

Für `capture_dispatch.rs`, `host_worker_reader.rs` und die zusätzliche
Workerfixture wurde außerdem der gesamte Dateitext gegen ausschließlich die
genehmigten Aufrufsubstitutionen verglichen. Damit bleiben insbesondere ihre
Supplement-/Retirementassertions und ihre Dispatchersemantik erhalten.
Die weiteren bestehenden Fallkörper beider geänderter Hauptfixturedateien
sind identisch. `git diff --check` ist ohne Befund. Diese statischen Prüfungen
führen weder Repositorycode noch Fixtures aus und ersetzen keine Rust-
Typauflösung oder funktionale Remote-Bestätigung.

## F – Exakte vorhandene Fälle und verbleibender Remote-Nachweis

Main behält den einen bestehenden Remote-Einstieg und die Manifestregistrierung.
Die folgenden vorhandenen Fallnamen sind für M06 und seine direkten
Vertragsanschlüsse vorbereitet; kein Fall wurde umbenannt oder entfernt:

- `batches_task_b6_worker_rejects_negotiated_packet_before_payload_read`
- `batches_task_b6_worker_reference_cannot_raise_negotiated_budget`
- `batches_capture_windows_resize_rejects_before_native_allocation_with_old_owner`
- `batches_task_capture_integration_remote_member_supplement_uses_the_image_book`
- `batches_task_capture_integration_worker_prepared_picture_keeps_its_captured_reference`
- `batches_task_capture_integration_prepared_transport_finishes_under_its_original_reference`
- `batches_task_capture_integration_retirement_before_transport_is_a_nonallocating_stale_reply`
- `batches_capture_windows_native_slots_match_wgc_dxgi_rotation`
- `batches_capture_windows_cache_alias_and_last_result_retain_session`
- `batches_capture_windows_exhausted_slots_do_not_call_allocator`
- `batches_capture_windows_rotation_preserves_pixels_and_source_generation`

Die B6-Fälle sind über `batches_task_b6_` ausgewählt; die Windows- und
Captureintegrationfälle stehen bereits im gelesenen Manifest. Main ordnet
dieselben bestehenden Fälle den F36/F37-Befunden zu; es entsteht keine zweite
Suite und keine neue Fallquelle.

Noch praktisch unbestätigt sind Rust-Typauflösung und tatsächlicher Ablauf
dieser aktualisierten Fälle auf Linux/Windows sowie die neue Ablehnung im
laufenden permanenten Readerthread. Die kontrollierte F37-Nativeressource
ist ein kleiner Bytevektor und keine echte D3D-/WGC-Surface. Physischer
GPUtreiber-Speicher, ein real fehlschlagender WGC-Close und praktische
Display-/Secure-Desktopwechsel werden hier nicht als ausgeführt oder
freigegeben ausgegeben. Ihre unveränderten Allokations-/Abschlussgrenzen
wurden statisch verfolgt. Die lokale Abgabe ist eine statische Autorprüfung,
kein Laufzeitpass und kein zusätzlicher Freigabehalt für Main.

## G – Vollständige Dateiübersicht und Entscheidungen

Erstellt wurde ausschließlich dieser Bericht. Geändert wurden die folgenden
acht vorhandenen Rustdateien in normaler Formatierung; jede bleibt unter
500 Zeilen und 50 KiB:

| Datei | Zeilen | Bytes |
|---|---:|---:|
| `native/src/screen/core/worker_wire.rs` | 287 | 10 060 |
| `native/src/screen/core/worker_frame_wire.rs` | 200 | 7 286 |
| `native/src/screen/core/capture_transport.rs` | 240 | 8 460 |
| `native/src/screen/os/shared/host_worker_reader.rs` | 115 | 5 306 |
| `native/src/screen/os/shared/capture_dispatch.rs` | 450 | 16 903 |
| `native/src/screen/os/shared/batches_capture_worker_fix_task_tests.rs` | 444 | 16 257 |
| `native/src/screen/core/batches_frame_budget_task_tests.rs` | 387 | 15 761 |
| `native/src/screen/os/windows/batches_capture_task_tests.rs` | 418 | 15 579 |

Zusätzlich zu diesen selbst gelesenen/geschriebenen Dateien und
`docs/plaene/2026-10-24-batches/fix-f-capture-budget.md` wurden folgende Dateien
gezielt gelesen beziehungsweise nach konkret benannten Symbolen durchsucht.
Die vollständigen Projektpfade sind relativ ausschließlich zur isolierten
Arbeitskopie `/tmp/smartexplorer-review-fixes-ee75e91b` angegeben:

- `AGENTS.md`
- `docs/ARCHITEKTUR.md`
- `docs/plaene/2026-10-24-batches/fix-regression-screenbudget.md`
- `docs/plaene/2026-10-24-batches/rv1-funktionsplan.md`
- `docs/plaene/2026-10-24-batches/rv1-funktionsbefunde.json`
- `graphify-out/graph.json`
- `native/Cargo.toml`
- `native/review-task-manifest.json`
- `native/src/screen/core/capture_admission.rs`
- `native/src/screen/core/capture_authority.rs`
- `native/src/screen/core/capture_bytes.rs`
- `native/src/screen/core/capture_grant.rs`
- `native/src/screen/core/capture_grant_claims.rs`
- `native/src/screen/core/capture_grant_slices.rs`
- `native/src/screen/core/capture_image.rs`
- `native/src/screen/core/capture_plan.rs`
- `native/src/screen/core/capture_plan_cases.rs`
- `native/src/screen/core/capture_records.rs`
- `native/src/screen/core/capture_session_context.rs`
- `native/src/screen/core/capture_source_contract.rs`
- `native/src/screen/core/capture_test_support.rs`
- `native/src/screen/core/capture_transport_wire.rs`
- `native/src/screen/core/frame_budget.rs`
- `native/src/screen/core/frame_memory.rs`
- `native/src/screen/core/frame_size_caps.rs`
- `native/src/screen/core/frame_size_packet.rs`
- `native/src/screen/core/host_codec_adapters.rs`
- `native/src/screen/core/host_codec_phase_plan.rs`
- `native/src/screen/core/session_admission.rs`
- `native/src/screen/core/worker_capture_wire.rs`
- `native/src/screen/core/worker_reply_wire.rs`
- `native/src/screen/core/batches_capture_integration_fix_task_tests.rs`
- `native/src/screen/os/shared/capture_broker.rs`
- `native/src/screen/os/shared/capture_broker_imports.rs`
- `native/src/screen/os/shared/capture_dispatch_requests.rs`
- `native/src/screen/os/shared/host_worker_client.rs`
- `native/src/screen/os/shared/host_worker_lifetime.rs`
- `native/src/screen/os/shared/host_worker_main.rs`
- `native/src/screen/os/shared/host_worker_main_tests.rs`
- `native/src/screen/os/shared/host_worker_prepared.rs`
- `native/src/screen/os/shared/host_worker_run.rs`
- `native/src/screen/os/shared/host_worker_tests.rs`
- `native/src/screen/os/windows/capture_dxgi.rs`
- `native/src/screen/os/windows/capture_prepared.rs`
- `native/src/screen/os/windows/capture_readback.rs`
- `native/src/screen/os/windows/capture_recovery.rs`
- `native/src/screen/os/windows/capture_retirement.rs`
- `native/src/screen/os/windows/capture_slots.rs`
- `native/src/screen/os/windows/capture_source.rs`
- `native/src/screen/os/windows/capture_wgc.rs`

Außerhalb der Arbeitskopie wurden die aktuellen Scope-/Auftragsquellen
`/tmp/smartexplorer-f-capture_budget-scope.json`,
`/tmp/smartexplorer-final-review-user-contract.txt`,
`/tmp/smartexplorer-rv1-40406246032/native-failures.json` mit den vollständigen
F36/F37-Einträgen und `/root/.codex/AGENTS.md` gelesen. Die verwendete
Graphify-Anleitung und ihre reine Queryreferenz liegen unter
`/root/.codex/skills/graphify/SKILL.md` und `references/query.md` desselben
Skillverzeichnisses. Die tatsächlichen externen Primärquellen sind in C verlinkt.

Die entscheidende Änderung ist die gemeinsame vorgezogene Prüfung im echten
IOpfad. Der aktuelle Grant-/Retirementvertrag wird erhalten; die zwei veralteten
Fixtureannahmen werden durch die tatsächliche Eigentumskette ersetzt. Es gibt
keine bekannte offene Scopeabhängigkeit. Mains Integration, Graphaktualisierung,
Fallzuordnung und Bestätigung über denselben Remote-Einstieg stehen aus.
Keine lokale Kompilierung, Testausführung, Formatierung, Serverausführung,
Installation, Gitmutation, Graphänderung oder Releasearbeit fand statt; kein
Unteragent wurde gestartet. Der Main-Arbeitsbaum wurde von diesem Autor nicht
verändert. Nach dieser vollständigen lokalen Autorabgabe stoppt dieser Auftrag.
