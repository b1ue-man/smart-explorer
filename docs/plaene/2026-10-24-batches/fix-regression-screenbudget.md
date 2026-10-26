# Fixzyklus RA-05, RSOL-03, RSOL-04 und RSOL-10

Stand: 2026-10-26. Autor dieses Dokuments ist zugleich Autor der Funktionsaufnahme,
des Fixplans, der Umsetzung und der anschließenden eigenen Anti-Regressionsreview.
Arbeitsbasis: `ee75e91bad6b28ef9206a5b2b2857cbea75c4e5b`; Releasevergleich:
`a814f263096fe69d3746e840b5f3fad3fd7e88a0` (v0.5.184).
Bearbeitet wird ausschließlich `/tmp/smartexplorer-review-fixes-ee75e91b`.
Der Main-Arbeitsbaum bleibt unveränderter Reviewkandidat.

## Ergebnisauftrag und Grenzen

Der delegierte Ergebnisauftrag und der wörtliche Nutzervertrag in
`/tmp/smartexplorer-final-review-user-contract.txt` verlangen die vollständige
Behebung der Befunde RA-05, RSOL-03 und RSOL-04; die spätere delegierte
Präzisierung erweitert denselben Autorzyklus um RSOL-10. Android muss vorhandene
Screenverbindungen zum Releasepeer in beiden Richtungen innerhalb tatsächlicher
Ressourcenlimits weiter benutzen können. Mehrere Gruppenvorschauen dürfen das
gemeinsame Prozessbudget bedarfsgerecht teilen. Native Quellen-/Sessionidentität,
Allokationsbesitz bis zum tatsächlichen Abschluss, Protokollgrenzen und die
Screen-/Login-Verbraucher bleiben erhalten.

Dieser Fixautor führt keine Builds, Compiler, Formatter, Tests, Server,
Installationen, langen Prozesse, Commits, Pushes, Graphupdates, Verpackung oder
Releases aus und startet keine Agenten. Laufzeitnachweise und die Registrierung
in der gemeinsamen Remote-Suite gehören Main. Die unten vorbereiteten Fälle
sind keine bestandenen Prüfungen und keine Gerätefreigabe.

## Erstplan aus dem Auftrag

1. Fehlerfunktionen samt direkten und indirekten Verbrauchern selbst aufnehmen;
   Legacy-Wireformat, Besitz- und Prozessgrenzen sowie Vorschau/Vollansicht als
   bestehende Kompatibilitätsverträge festhalten.
2. Nach dem Quellenbefund eine konkrete Abhilfe mit testbaren Erwartungen planen;
   offene Plattform-/Protokollfragen anhand aktueller Primärquellen klären.
3. Alle notwendigen Änderungen innerhalb der exklusiven Schreibfläche umsetzen;
   fehlende zusammenhängende Schreibabhängigkeiten vor einer Änderung Main nennen.
4. Die gesamte ermittelte Kette selbst statisch auf Regressionen prüfen und Fälle
   für Mains gemeinsame Remote-Suite vorbereiten.

Promptwissen: Die ursprünglich drei Befund-IDs und ihre geforderten Nutzerergebnisse sind
bekannt. Anfangs unbekannt waren die tatsächlichen zusätzlichen Legacy-Prüfer,
die Allokationsgrenzen hinter JNI und der Übergang von Vorschau zu Vollansicht.
Keine nur durch den Nutzer zu klärende Produktentscheidung wurde festgestellt.

## Erster Quellenbefund und Stufe-eins-Plan

`viewer_client.rs::handshake_for` und `host_server_start.rs::prepare` lehnen
Legacyverbindungen bei einer Lease unter 512 MiB ab. Zusätzlich verlangt
`capture_session_context.rs::check_contract` beim lokalen und delegierten
Legacy-Binding exakt 512 MiB. Ein bloßes Entfernen der ersten zwei Prüfungen
behebt den Hostpfad deshalb nicht.

`os/shared/session_memory.rs::handshake` reserviert bei vorhandenem Angebot dessen
gesamte `working_bytes`. Android hat keine Desktop-MemoryFacts; `ScreenApi.kt`
meldet zugleich die verbleibende Heapgröße als Prozessgrenze. Die erste Vorschau
verbraucht damit die ganze Zusage, obwohl `ScreenPreviewTile.kt` 320×320 anfordert.

Der v2-Vertrag ist unveränderlich. Eine kleinere Vorschauzusage muss durch
Paketlesen, Queues, JNI-Pull, Java-Decodierung und Skalierung getragen werden.
Eine bloße kleinere Poolbuchung bei unverändert großem Vertrag wäre eine
ungedeckte Zusage. Legacy behält auf dem Netzwerk `frame_contract=None`;
lokale kleinere Grenzen dürfen keinen ausgehandelten v2-Vertrag vortäuschen.

`ScreenViewScreen.kt` übernimmt eine Vorschau und hebt sie per `SET_MODE` zur
Vollansicht an. Eine kleine unveränderliche Vorschauvereinbarung darf diesen
Bedienweg nicht dauerhaft auf Vorschauauflösung beschränken. Der bestehende
Wechsel zur Steueransicht öffnet bereits eine eigene Vollansichtssitzung.

Planungstiefe: zusammenhängende Arbeit (Stufe 2), weil vorhandene Verträge von
Host, Viewer, Prozesspool und Android-Darstellung betroffen sind; keine neue
externe API oder neue Nutzfunktion.

## Recherche vor dem Detailplan

Primärquellen, abgerufen am 2026-10-26:

- [Android Runtime](https://developer.android.com/reference/java/lang/Runtime):
  `maxMemory`, `totalMemory` und `freeMemory` beschreiben VM-Speicher. Die
  verbleibende Java-Heapgröße ist kein Nachweis einer zusätzlichen freien
  nativen/GPU-Allokationsklasse. Die vorhandene konservative Prozessgrenze
  bleibt eine Grenze, keine Behauptung physisch reservierter Bytes.
- [Serde-Feldattribute](https://serde.rs/field-attrs.html): optionale/default-
  Felder können bei fehlendem Wirefeld mit dem Default eingelesen werden.
  Kleinere lokale Legacygrenzen benötigen deshalb kein neues Pflichtfeld.
- [Tokio JoinHandle](https://docs.rs/tokio/latest/tokio/task/struct.JoinHandle.html):
  Drop eines Handles beendet die laufende Aufgabe nicht. Die bereits vorhandenen
  Abschlussbesitzer und Grants müssen bis zum tatsächlichen Abschluss erhalten
  bleiben; die Budgetkorrektur darf keinen davon verkürzen.
- [Rust `Arc::get_mut`](https://doc.rust-lang.org/std/sync/struct.Arc.html#method.get_mut):
  Ein bereits aliasierter Besitz wird nicht exklusiv veränderbar. Die Verkleinerung
  einer angenommenen Viewerlease findet deshalb nur vor ihrer Weitergabe und bei
  tatsächlich ungeteilter `Arc` statt; ein vorhandener Alias behält seine Zusage.

Bewertete Wege: (1) 512-MiB-Prüfungen nur löschen: verworfen, weil Hostplanung und
Legacy-Paket-/Java-Grenzen dann weiter ungedeckt groß wären; (2) Prozessbudget je
Vorschau vervielfachen: verworfen, weil gemeinsame Grenzen verloren gingen;
(3) lokale Legacygrenzen aus tatsächlicher Lease und Geometrie durchtragen und
v2-Vorschauen bedarfsgerecht zulassen: gewählter Ansatz, Detailplan folgt nach
vollständiger Aufnahme der verketteten Verbraucher.

## Offene Integrationsabhängigkeiten

Vor Produktänderungen wurde Main die erforderliche zusätzliche Schreibfläche für
die Legacy-Vertragsprüfung, Host-Ausgabeplanung, Empfangsgrenzen, mobile
Budgetweitergabe und Android-Vollansichtsübergabe genannt. Keine Datei außerhalb
der ausdrücklich freigegebenen Schreibfläche wird geändert.

## Stufe-zwei-Detailplan vor der Umsetzung

Die zweite Lückenrecherche verfolgt zusätzlich die reale IPC-Grenze
`ipc_protocol_screen.rs` → `screen_ipc.rs` → `screen_ipc_client.rs` und die private
Parent/Worker-Grenze `worker_capture_wire.rs::OutputWire`. Ein aus dem ursprünglichen
Android-Angebot neu berechnetes Budget wäre kein Beleg der tatsächlich erhaltenen
Lease. Deshalb wird die konkrete lokale Grenze privat weitergegeben; das Remote-
HELLO_OK und die v1-GOP-Header bleiben unverändert. `StreamEvent` behält seine Form.

Die kleinen Legacygrenzen werden aus derselben vorhandenen Produktbilanz wie v2
abgeleitet: Bildbesitzer, 32 begrenzte Recordkopien, Wire-/JNI-Köpfe und ein
vollständiges Paket. Legacy kann seine Records nicht nach einem v2-Budget
umpacken; seine Grenze berücksichtigt daher einen vollständigen alten Tile-Record.
Es gibt keinen frei erfundenen festen Kachelbetrag und keine zweite Prozesskasse.
Die bestehende vollständige 512-MiB-Legacyklasse bleibt als Bestandsvertrag erhalten.

| Meilenstein | Dateien / Grenze | Erwartetes Ergebnis und zu erhaltender Vertrag |
|---|---|---|
| M1 Bedarf und lokale Legacygeometrie | `frame_budget.rs`, neues `session_admission.rs`, `frame_size_caps.rs` | Eine 320×320-Vorschau erhält einen aus realen Besitzergrenzen berechneten Teilbetrag. Kleinere Legacy-Leases ergeben konkrete darstellbare Grenzen; v1 bleibt v1. Null, Überlauf und unmögliche Paketbilanz werden abgelehnt. |
| M2 Host und tatsächliche Allokation | `host_server_start.rs`, `capture_session_context.rs`, `host_codec_adapters.rs`, `host_codec_phase_plan.rs`, `host_pipeline.rs`, `host_pipeline_actor.rs`, `host_capture.rs`, `worker_capture_wire.rs`, Android `capture_admission.rs` | Kein pauschales 512-MiB-Erfordernis. Start, Quellenwechsel, Preview/Full, Codec-Fallback und Snapshot benutzen dieselbe lokale Ausgabegrenze. Capture-W/X und tatsächliche Native-/Java-Besitzer bleiben vollständig gebucht, auch nach Abbruch. Login-Quellpläne ohne Ausgabe behalten ihren bisherigen Vertrag. |
| M3 Viewer, Pakete und private IPC | `os/shared/session_memory.rs`, `viewer_client.rs`, `gop_stream.rs`, `wire_binary.rs`, `ipc_protocol_screen.rs`, `screen_ipc.rs`, `screen_ipc_client.rs` | Die angeforderte Vorschaugeometrie begrenzt die Lease, nicht die Prozessgrenze. Legacy braucht keine volle Klasse; lokale Grenzen werden vor Record-Payload-Allokation geprüft. Ein ausgehandelter v2-Vertrag darf nicht erhöht werden; die private IPC trägt nur die tatsächlich zugelassene Grenze weiter. |
| M4 Android-Verbraucher und Vollansicht | `viewer_sink.rs`, `screen_view_run.rs`, `screen_view_json.rs`, `ScreenApi.kt`, `ScreenSession.kt`, `ScreenViewScreen.kt` | JNI-Queue, Decoder, Pull und Skalierung verwenden die konkrete Grenze. Eine explizite kleine Legacygrenze kann nicht auf das pauschale Legacybudget zurückfallen. Das Antippen einer Vorschau öffnet weiter die Vollansicht; eine eigene Vollansichtssitzung verhindert, dass deren unveränderlicher Vertrag auf Vorschaugröße bleibt. Schließen stellt die Gruppenkachel weiter bereit. |
| M5 Eigene Anti-Regressionsreview und vorbereitete Fälle | Zwei exklusive `batches_session_budget_compatibility_task_tests.rs`, bei Bedarf `frame_memory_tests.rs`, dieser Bericht | Native/Java-Grenzen, Legacy-Drahtformat, mehrere Vorschauen, Session-/Quellenbindung, Quellenwechsel, Fallback, Snapshot, Abschlussbesitz und Login-Konkurrenz sind im Quelltext selbst geprüft und in exakt benannten Remote-Fällen abgebildet. Kein lokaler Lauf wird behauptet. |

M2 und M3 hängen von M1 ab; M4 hängt von M3 ab; M5 erfolgt nach dem vollständigen
Fix. Die zusätzliche Freigabe für alle genannten privaten Grenzen wurde bei Main
eingeholt. Der Plan verlangt Rustdateien unter 500 Zeilen und 50 KiB nach
üblicher Formatierung; die nachträgliche Strukturkorrektur ist unten dokumentiert.

Korrektur des Plans aus der anschließenden eigenen Verbraucheraufnahme, vor der
darauf gestützten Anpassung: Die geometrische Vorschaupolitik gilt ausschließlich
für Android-Viewer. Desktop hebt eine Vorschau weiterhin per `SET_MODE` in
derselben Sitzung an und benötigt dafür sein unverändertes vollständiges
Vertragsangebot. Die Plattformadapter liefern diese Auswahl als getypten
`PreviewAdmission`-Wert; die gemeinsame Zulassung setzt sie vor der Lease um.
Der Host verkleinert Legacy-/Desktopangebote nicht pauschal anhand des
Startmodus. Androids bereits kleines explizites Angebot begrenzt moderne Hosts
durch die normale Schnittmenge. Diese Korrektur bewahrt auch den bestehenden
Desktopweg gegen einen Androidhost.

Die anschließende Aufnahme des Kachel-Einstiegs ergänzt M3: Vor jeder Vorschau
liegt ein `Probe`. Dieser überträgt definitionsgemäß keine Bilder, darf daher
keine volle Bildlease gegen die parallel startenden Kacheln reservieren. Probe
erhält keine Bildzusage und der Core startet dafür keinen Bildreader; die
bestehende Rechte-/Fähigkeitsantwort bleibt unverändert. Ein vorbereiteter Fall
prüft, dass der produktive Zulassungshelfer dafür keine Reservierung aufruft.

Der Bedienweg bleibt ein Antippen der Kachel und derselbe Vollansichtsdialog mit
dem vorhandenen Schließen. Aufruf, Zeigerweg, Suche, Scrollen, Kontextwechsel,
Wiederholung, Lernen und Fehlerkorrektur erhalten keine zusätzliche Nutzeraktion.
Der bestehende Verbindungs-/Rückfrage-/Fehlerzustand begleitet die neue eigene
Vollansichtsverbindung; es wird kein fertiges Bild vor dem tatsächlichen Empfang
angezeigt. Das Verhalten ist geplant und quellenbasiert, noch nicht auf einem
Gerät gemessen.

## Umsetzung, eigene Anti-Regressionsreview und Suitefälle

### Zusatzplan RSOL-10 vor seiner Umsetzung

Der vollständige Befund `review-regression-sol.md` §RSOL-10 und die anschließende
eigene Quellenlektüre bestätigen die Kette: Kotlin `capture_active(false)` trennt
die Displayoberfläche und beendet Frame-/Encoderbesitzer; die Brücke bestätigt die
Aktivitätsanfrage. `BridgeSource::set_capture_active` verwirft den alten nativen
Encoderzustand. Das nächste `configure` ruft über `ScreenCaptureBudget.open` und
JNI erneut `CaptureSession::begin`. Dort wird gegenwärtig vor `retire_grant` der
vollständige neue Plan angefordert. Damit bleibt der unbenutzte alte Maximalplan
zusätzlich zur neuen Aufnahme gebucht.

Entscheidungsrecherche: `capture_grant_claims.rs::retire` reduziert die alte
Reservierung atomar auf `usage.total`, die tatsächlich noch lebenden Slices.
Es beendet die alte Allokationsbefugnis, gibt aber keinen lebenden Besitzer frei.
`CaptureSlice`-Abschluss reduziert auch einen bereits retirten Plan weiter.
Dieser bereits bestehende Vertrag genügt; eine zusätzliche Kotlinmeldung oder
ein pauschales Freigeben des Plans wären unnötig beziehungsweise unsicher.

M6: In `os/android/capture_admission.rs::begin` nach der unveränderten möglichen
Erstübernahme die alte Rootbefugnis vor der neuen Genehmigung retirieren. Die
konkrete alte Grant bleibt bis zur erfolgreichen Übergabe gespeichert. Ein
fehlgeschlagener Kandidat darf die sichtbare Generation nicht verbrauchen;
ein Wiederholungsversuch nach dem tatsächlichen Abschluss alter Slices muss
dieselbe nächste Generation genehmigen können. Familien-Mitgliedsgrants dürfen
keine fremde Rootbefugnis retirieren. Erwartung: Bei Q ≤ B < P+Q und tatsächlich
abgeschlossenen alten Besitzern wird Q zugelassen. Bei L+Q > B mit noch lebendem
altem Besitzer L bleibt der Versuch abgewiesen; nach dessen tatsächlichem Ende
gelingt derselbe Versuch. Beide Erwartungen werden in der bereits exklusiven
gemeinsamen Fixturedatei vorbereitet und anschließend selbst statisch geprüft.

Die zweite Lückenrecherche zu M6 zeigt zwei notwendige Integrationen: Bei einem
Konfigurationswechsel schließt `ScreenHostControl.applyConfig` die alten Java-
Frame-/Encoderbesitzer bislang erst nach der neuen Allokation. Diese Reihenfolge
wird für den ausdrücklich ersetzten Epoch auf tatsächliches altes Ende vor Q
geändert. Synthetische Producer behalten ihren bestehenden echten
Retirementcallback; ein wartender Neustart benutzt erst nach dessen Erfolg die
dann aktuelle Konfiguration. Pause/Ende verhindern einen verspäteten Neustart.
Zusätzlich beendet `BridgeSource` seine nicht mehr konsumierbaren alten Tiles-/
Hybridreferenzen vor `configure`. `Channel::send(configure)` entfernt die alte
Queue unter derselben Sperre wie den Epochwechsel; ein noch außerhalb der Queue
gehaltener Besitzer wird nicht freigegeben. Die exakte zusätzliche Schreibfläche
wurde vor diesen Änderungen von Main freigegeben. Erwartung: Weder eine alte
Maximalreservierung noch ein künstlicher Abhängigkeitsskreis zwischen altem
Referenzbild und erstem neuen Bild verhindert den erlaubten Neustart.

Weitere eigene Gegenprüfung von M6: Nach einer zunächst wegen real lebender
Besitzer abgewiesenen neuen Aufnahme ist die alte Bildbefugnis bereits retirert.
Die bestehende neun Byte große Aktivitätsquittung darf dann nicht versuchen,
über diese Bildbefugnis einen neuen Transportclaim anzulegen. Sie erhält bei
diesem Zustand einen eigenen exakt neun Byte großen Plan derselben vorhandenen
Session-/Quellen-/Generationsautorität. Der aktive Bildpfad benutzt weiterhin
seinen bestehenden Claim. Es entsteht keine zweite Kasse, keine neue
Bildallokationsbefugnis und keine Freigabe alter Slices. Erwartung: Pause,
Wiederaufnahme und Schließen bleiben auch während echter Retirement-Last
quittierbar; die Quittung selbst ist vollständig im gemeinsamen Budget gebucht.

Die abschließende M6-Gegenprüfung ergänzt den synthetischen Tiles-/Snapshotpfad:
Eine nach fehlgeschlagenem `begin` retirierte Befugnis darf der Snapshot nicht
erneut als aktive Befugnis zurückgeben. Beim Ersetzen eines ausschließlich
nativen Snapshotplans wird dessen unbenutztes Maximum vor Q retirert; echte
Slices bleiben L. Ein noch aktiver Surfaceplan bleibt für parallele Snapshots
dagegen vollständig erhalten. Auch nichtprojizierte Tiles-Epochen beenden ihre
überholten Encoderreferenzen vor der nächsten Bildanforderung. Erwartung:
Retrierbare Native-/Snapshotaufnahmen brauchen L+Q, während parallele Aufnahmen
eines tatsächlich aktiven Surfaceencoders weiterhin P+Q finanzieren müssen.

### Fehlerfunktionen und vollständige betroffene Verknüpfungskarte

Die Aufnahme und die folgende Review stammen von diesem Fixautor. „Vollständig“
bezeichnet die durch die vier Befunde tatsächlich betroffene Vertragskette,
einschließlich der unveränderten Verbraucher und ihrer öffentlichen Grenzen.
Es bezeichnet keine Prüfung sämtlicher Projektfunktionen.

| Fehlerfunktion | Beleg des ursprünglichen Fehlers | Auswirkung / Zuordnung |
|---|---|---|
| `host_server_start::prepare` | Legacy ohne Fähigkeiten verlangte exakt `MAX_WORKING_BYTES` | Kleine Androidhost-Lease wurde vor dem ersten Bild abgewiesen; RA-05. |
| `viewer_client::handshake_for` | Legacy-Accepted wurde bei Lease unter 512 MiB abgewiesen | Android konnte v0.5.184 nicht ansehen; RA-05/RSOL-03. |
| `capture_session_context::check_contract` | Auch lokales und delegiertes Legacy-Binding verlangten exakt 512 MiB | Zusätzlicher verketteter Host-/Helper-Abbruch derselben Ursache. |
| `os/shared/session_memory::handshake` | Vorschauen und die vorgelagerten bildlosen Probes reservierten das ganze Fähigkeitenangebot | Die erste Androidkachel oder ihre vorgelagerte Abfrage belegte die gemeinsame Zusage; RSOL-04. `MemoryPool` selbst setzte die angebotene Grenze korrekt durch. |
| `CaptureSession::begin` | Neues Q wurde vor Retirierung von P beantragt | P+Q statt tatsächlichem L+Q; RSOL-10. |
| `CaptureSession::synthetic_snapshot` | Im erweiterten Retirementvertrag konnte der Snapshot eine bereits retirierte Befugnis wiederverwenden oder einen ersetzbaren nativen Vorgänger vollständig behalten | In der eigenen Nachreview geschlossener Folgezustand von RSOL-10; eine aktive Surfacebefugnis bleibt für parallele Aufnahmen erhalten. |
| `ScreenHostControl::applyConfig` / `BridgeSource`-Epochwechsel | Alte Java-Producer und native Referenzbilder blieben bis nach dem ersten neuen Bild erhalten | Der neue Plan konnte auf seinen eigenen vorausgesetzten Vorgänger warten; verketteter RSOL-10-Konfigurationspfad. |

| Vollständige betroffene Kette | Geprüfte Grenze und Erhalt |
|---|---|
| Android `ScreenPreviewTile` → `ScreenSession` → `CoreScreenBackend.open` → `ScreenApi.viewOpen` → `mobile::screen_view::view_open` | 320×320, ursprünglicher `relation`-/`relationId`-/`deviceId`-/`expectedNodeId`-/Quellenbezug, echte Renderergrenze und Java-Heapangebot bleiben erhalten. |
| Desktop `screen_pool` / `screen_session_backend` → `StreamRequest` → Daemon | Desktop behält sein vollständiges Angebot und die bestehende Preview→Full-Umschaltung innerhalb einer Verbindung. `StreamRequest` und `StreamEvent` bleiben unverändert. |
| Daemon `screen_ipc` → `ScreenLink::client_hello` → `viewer_client` | Gepinnte Geräte-/Nodeidentität, Relation-HMAC, Screen-ALPN, Routefreigabe, Pending, Refused und Metadata-only bleiben vor den bestehenden Grenzen. Kein Remote-Hello-Feld wurde ergänzt. |
| Android-/Windows-/Linux-Sessionmemoryadapter → `admit_hello` → `MemoryPool::reserve_with_ceiling` | Nur Android wählt `PreviewAdmission::Geometric`; Windows/Linux wählen `FullContract`. Physische/Commit-/Adressraumfakten, Java-Grenze, Kohortenobergrenze und aggregierte Zusagen bleiben dieselben. |
| `host_server_start` → `family_listing::prepare` → `PreparedSourceInfo` → `host_admission` | W liegt vor Vorbereitung; Nativegröße und SourceKey/Generation werden weiterhin vor Aktivierung geprüft. Die Legacy-Ausgabegrenze ist lokal, nicht ein vorgetäuschter v2-Vertrag. |
| `SessionAdmission` → `SharedAuthority::fund` → `CaptureGrant` → `CapturedImage` / `AdmittedBytes` / `CaptureSlice` | W/X bleiben gemeinsame Finanzierung. Planung, Sliceclaim und tatsächlicher Abschluss bleiben getrennt. Ein Clone bleibt ein Alias; die Bildbytes enden vor ihrer Lease. |
| `HostOutputPlan` → `host_codec_phase_plan` → `OutputWire` → `host_worker_prepared` | Kleine lokale Legacygrenzen überleben den privaten Parent-/Workerübergang. Vertrag, SourceKey, Generation und konkrete Credits behalten ihren bestehenden Bezug. Login-Quellpläne ohne Ausgabe behalten den Source-only-Pfad. |
| `Pipeline::fitted_size` / `PipelineActor::capture_context` / `host_capture::still_tiles_admitted` | Normales Bild, Quellwechsel, Full/Preview, H.264/Tiles-Fallback und Snapshot verwenden dieselbe lokale Ausgabegrenze. Tatsächliche große Nativequellen werden weiter separat und vollständig geplant. |
| `GopHeader` → GOP-Reader → `read_frame_record_admitted` | v1 bleibt `frame_contract=None` / `GOP_VERSION=1`; v2 behält den ausgehandelten Vertrag. Bild-, Einzelrecord- und Transaktionsgrenzen werden vor Payloadallokation geprüft. Legacy-H.264-Anfragen berücksichtigen die kodierten 16×16-Blöcke; unter einem solchen Block bleibt Tiles verfügbar. |
| `RelayHello::Accepted` → `ScreenStream::next_event` → `StreamEvent` | Nur die private Relayantwort trägt optionale tatsächliche `local_limits`. Die Relay-Recordlänge wird vor Allokation begrenzt; v1 erhält keinen unnötigen v2-Extensionpuffer. Die öffentliche Ereignisform bleibt gleich. |
| `screen_view_run` / `screen_view_json` → `ViewerSink` / `PullQueue` → `screen_jni` | Die konkrete Zulassung begrenzt Tiledecoder, Queue, JNI-Pull und Recordgrößen. Vollständige GOP-/Frameidentitäten und verlorene Referenzen behalten ihre bestehenden Regeln. |
| `ScreenSessionState.apply` → `ScreenBufferBudget` → `ScreenPullBuffers` / `ScreenDecoder` / `ScreenScaleWorker` / `ScreenResample` | Explizit kleine Legacybudgets haben denselben begrenzten Besitzerhaushalt; ein fehlender Zusatzmarker vergrößert sie nicht. Ein ungültiges explizites Budget fällt nicht auf 512 MiB zurück. Altereignisse ohne Budget behalten ihren bisherigen Legacy-Fallback. |
| `ScreenViewScreen.openOwn` / Rückweg / Quellen- und Qualitätswahl | Vollansicht und Kontrolle erhalten eine eigene tatsächliche Zulassung. Gewählte Qualität wird nach Accepted angewandt. Rückkehr übergibt die gewählte Quelle an die bestehende Vorschaukachel; Familienansichten behalten ihren eigenen Pfad. |
| `BridgeSource::set_capture_active` / `tiles_frame` / `h264_frame` → `Channel::send` → `ScreenHostControl` | Eine neue Epoch beendet die nicht mehr verwendeten nativen Referenzbesitzer und alte Queuebilder. Der Queue-/Epochwechsel ist atomar; außerhalb der Queue gehaltene Slices bleiben unberührt. |
| `ScreenHostControl.applyConfig` → `ScreenProjectionFrames.close` / `ScreenEncoder.release` / `ScreenSyntheticRetirement` → `ScreenCaptureBudget.open` → JNI → `CaptureSession::begin` | Nichtsynthetische Producer müssen tatsächlich enden; synthetische Producer behalten ihren Abschlusscallback. Ein wartender Start nimmt die aktuelle Konfiguration, keinen veralteten Epoch. Pause, Ende, Projektion und Identität werden weiter geprüft. |
| `CaptureSession::begin` / `activity_ack` → `CaptureGrant::retire` / `CaptureSlice::drop` | Alte unbenutzte Maxima werden vor Q zu L reduziert. Echte alte Besitzer bleiben gebucht. Ein abgewiesener Kandidat verbraucht keine Generation; die neun Byte große Aktivitätsquittung bleibt auch bei retirter Bildbefugnis exakt finanziert. |
| Login `ParentCapture` / `helper_capture` / `client` / `gateway_client` / `viewer_images` / Android `login_images` | Gleicher Pool, gleiche beobachtete Grenze und bestehende Rollen. Source-only-Anmeldung, Bild-/Helperidentität und tatsächliche Retirementbesitzer bleiben erhalten. Der neue Konkurrenzfall verwendet genau diese `ImageAdmission`-Grenze. |
| CLI `screen::{input,snapshot,windows,bench}` / MCP `screen_tool` → `screen_ipc_still` | View-, Control-, Probe-, Snapshot-, Fenster-/Icon- und Pendingpfade behalten ihre Eingaben, Quellnamen, Fristen, PNG-Ausgabe und Berechtigungen. Die neue lokale Grenze läuft unter ihrer unveränderten Streamform. |

### Umgesetzte Änderungen und Entscheidungen

M1–M6 sind im Quelltext umgesetzt. Die neuen Helfer liegen im kleinen
`core/session_admission.rs`; Plattformwahl bleibt unter `os/`. Die globale
Poolimplementierung wurde nicht vereinfacht und nicht durch unabhängige
Sitzungskassen ersetzt. Eine Vorschauzusage berücksichtigt den alten
vollständigen Tile-Record und den tatsächlichen Besitzerhaushalt statt einer
beliebigen MiB-Zahl. Die vollständige veröffentlichte Legacyklasse bleibt erhalten.

Ein kleiner Legacy-Vertrag wird nicht nach außen erfunden. Host, Viewer, private
Worker-/Relaygrenzen und Android-Verbraucher erhalten lediglich konkrete lokale
Allokationsgrenzen. Neue v2-Verträge dürfen weder das Angebot noch die reale
Lease übersteigen. Ein ungeteilter, noch nicht an Puffer weitergereichter
Viewerlease kann auf den tatsächlich angenommenen kleineren v2-Vertrag sinken;
bereits geteilte Leases bleiben bis zum letzten tatsächlichen Besitzer bestehen.

Bei RSOL-10 wurde kein „frei“-Flag anstelle des tatsächlichen Abschlusses
eingeführt. `retire` gibt ausschließlich unbenutzte Plananteile frei; Java-
Producer, native Referenzbilder, Queuebilder und externe Slices enden über ihre
realen bisherigen Besitzer. Pause/Ende verhindern einen verspäteten synthetischen
Neustart. Bei noch ausreichendem L+Q ist ein Neustart möglich; bei echter
Überschreitung bleibt er abgewiesen und derselbe nächste Kandidat wiederholbar.

### Eigene anschließende Anti-Regressionsreview

Geprüft wurden die vollständigen oben aufgeführten Ketten nach der Umsetzung,
einschließlich unveränderter Verbraucher. Konkrete eigene Befunde und Korrekturen:

- Eine globale geometrische Vorschaupolitik hätte die Desktop-Vollansicht
  verkleinert. Die finale Politik ist ausschließlich für Android geometrisch;
  Windows/Linux und der Host behalten vollständige Desktop-/Legacyangebote.
- Parallel vorgelagerte Kachel-Probes dürfen die Bildbudgets nicht bereits vor
  der eigentlichen Vorschau belegen. Probe hat keinen Bildreader und benötigt
  keine Bildlease; seine bisherige Rechte-/Fähigkeitsantwort bleibt erhalten.
- Die neue Android-Vollverbindung durfte die bestehende Quellenwahl auf der
  zurückkehrenden Kachel und bereits gewählte Qualität nicht verlieren. Beide
  Übergaben sind im tatsächlichen UI-Lebenszyklus erhalten.
- Eine geänderte interne Snapshot-Signatur hätte bestehende Fixtures gebrochen.
  Der vorhandene Vierargument-Anschluss bleibt als Testadapter erhalten; der
  Produktionspfad benutzt die tatsächlichen lokalen Grenzen.
- Ein zu großzügiger v1-Relaykopf hätte bis zu 1026 nicht vorhandene
  Extensionbytes als Payloadspielraum zugelassen. Die finale v1-Grenze enthält
  ausschließlich seine echten Köpfe. Der Sink prüft zusätzlich die konkrete
  Recordgrenze vor seiner Verarbeitung.
- Ein Legacy-H.264-Bild nahe dem Pixelmaximum kann kodiertes Padding benötigen.
  Die Anfrage wird gegen ganze 16×16-Blöcke angepasst; ein nicht finanzierbarer
  AVC-Block entfernt den weiterhin möglichen Tiles-Weg nicht.
- Ein explizites ungültiges Legacybudget durfte nicht zu einem 512-MiB-
  Skalierer werden. Der Fallback gilt nur bei tatsächlich fehlendem Budget;
  jeder kleinere Betrag bindet den Besitzerhaushalt auch ohne Zusatzmarker.
- Das reine Vorziehen von `retire` hätte nach echter Retirement-Last die
  Aktivitätsquittung über eine tote Bildbefugnis versucht. Der eigene exakt
  neun Byte große Kontrollplan schließt diese Pause-/Abbruchlücke.
- Reiner Planabbau genügte für RSOL-10-Konfigurationswechsel nicht: Die alten
  Java-Producer und nicht mehr benutzbaren nativen Referenzen müssen tatsächlich
  enden. Die finale Reihenfolge und der echte synthetische Callback erhalten
  diesen Abschluss; ein altes Queuebild kann nicht zwischen Leeren und
  Epochwechsel neu eingereiht werden.
- Ein synthetischer Snapshot hätte nach abgewiesener Neukonfiguration eine
  retirierte Bildbefugnis wiederverwenden können. Er übernimmt eine neue gültige
  Befugnis; native Vorgängerpläne werden vor Q auf ihre tatsächliche Restlast
  reduziert. Ein noch aktiver Surfaceencoder behält dagegen seinen vollständigen
  Plan für parallele Snapshots. Auch nichtprojizierte Tiles-Epochen beenden die
  überholte Encoderreferenz vor der neuen Bildanforderung.

Statisch geprüft sind Source-/Session-/Generationserhalt, Slot- und Bytegrenzen,
Root-/Membergrants, Abbruch, Wiederholung, Metadaten, H.264/Tiles, Quellwechsel,
Snapshot, Desktop-Promotion, Android-Rückweg und Login-Konkurrenz. Die Änderung
berührt keine Dateisystem-/Sync-Backup-, Konflikt- oder Löschoperation.
Unveränderte Rechte-, Zustimmungs-, Projektions- und Inputbelege bleiben vor der
ersten Aufnahme beziehungsweise Eingabe wirksam.

Integrationsbeleg nach Mains Hinweis auf den parallel geänderten Capturepfad:
Der aktuelle Windows-`CaptureRecovery` wiederholt einen ausstehenden Neustart
über `CaptureBudget::replace`; X11 veröffentlicht seinen Referenzverlust vor
einem gegebenenfalls abgewiesenen neuen Antrag. `host_capture` beendet bei
veränderter Quellreferenz den alten Encoder vor dem Wiederholungsaufruf. Diese
Pfade benutzen denselben `SessionAdmission`-/`CaptureAuthority`-Vertrag und die
gespeicherten `HostOutputPlan`-Klone; deren lokale Legacygrenzen bleiben erhalten.
Androids `CaptureSession::begin` ist kein direkter Windows-/X11-Aufrufer. Die
Capturedateien des parallelen Autors wurden ausschließlich gelesen.

### Vorbereitete Fälle und Registrierung durch Main

Keiner dieser Fälle wurde lokal ausgeführt. Die Registrierung gehört in Mains
eine gemeinsame Remote-Suite; keine zweite Pipeline und kein eigener lokaler Lauf.

`core/batches_session_budget_compatibility_task_tests.rs`, eingebunden aus
`frame_budget.rs` → `session_admission.rs`:

- `batches_task_b6_session_budget_two_android_previews_share_the_real_process_ceiling`
- `batches_task_b6_session_budget_preview_contract_is_small_and_full_request_is_independent`
- `batches_task_b6_session_budget_legacy_host_plan_worker_resize_and_image_retirement_stay_bound`
- `batches_task_b6_session_budget_login_image_claims_compete_with_preview_leases`
- `batches_task_b6_session_budget_legacy_record_length_is_rejected_before_payload_read`
- `batches_task_b6_session_budget_v184_whole_tile_records_fit_the_small_legacy_receiver`

`os/shared/batches_session_budget_compatibility_task_tests.rs`, eingebunden aus
`os/shared/session_memory.rs`:

- `batches_task_b6_session_budget_android_policy_shares_previews_desktop_keeps_full_contract`
- `batches_task_b6_session_budget_v184_hello_and_picture_work_below_512_mib`
- `batches_task_b6_session_budget_android_resume_reuses_capacity_after_actual_owner_end`
- `batches_task_b6_session_budget_android_resume_keeps_live_owner_and_retries_same_generation`
- `batches_task_b6_session_budget_android_snapshot_replaces_only_retired_or_native_authority`
- `batches_task_b6_session_budget_android_epoch_drops_old_queue_but_keeps_external_owner`

Direkt im bestehenden privaten `daemon/os/shared/ipc_protocol_screen.rs`-Testmodul:

- `batches_task_b6_session_budget_private_relay_keeps_v1_and_rejects_oversize_before_read`

Androidklasse
`app.smartexplorer.android.screen.ScreenSessionBudgetRegressionTest`:

- `batches_android_session_budget_small_legacy_receiver_keeps_its_scaler_bound`
- `batches_android_session_budget_invalid_explicit_legacy_offer_has_no_512_mib_fallback`
- `batches_android_session_budget_old_acceptance_without_budget_keeps_legacy_contract`

M1/M3/RSOL-04 werden durch die gemeinsamen Prozess-/Policyfälle abgebildet;
M2/RA-05 durch Legacybinding, Hostplan, Workerweitergabe und v1-Bilddaten;
M3/RSOL-03 zusätzlich durch den v1-Handshake-/Bildstromfall;
M4 durch Kotlin-Budgetübernahme und den unveränderten bestehenden Android-
Darstellungs-/Sitzungsanschluss der Gesamtsuite; M6/RSOL-10 durch tatsächliches
Sliceende, echte Restlast, dieselbe nächste Generation und den Queue-/Epochfall.
Die neun Byte große Kontrollquittung wird im Restlastfall mitgeprüft. Der
Snapshotfall unterscheidet aktive Surfacebefugnis, retirierte Surfacebefugnis
und den mit tatsächlichem altem Slice belasteten nativen Ersatzplan.

### Tatsächlicher Nachweis und verbleibende Grenzen

Ausgeführt wurden ausschließlich Quelllektüre, Graphabfragen des vorhandenen
Kandidatengraphen, statische Text-/AST-Prüfung und Dateiedits. Für die AST-Prüfung
wurden die bereits installierten Rust-/Kotlin-Tree-sitter-Grammatiken des
Graphify-Werkzeuginterpreters benutzt; es gab keine Installation und keinen
Graphumbau. `git diff --check` meldet für die eigene Schreibfläche keinen Fehler.
Die damalige Größenprüfung zählte den noch dichten Quellstand. Ihr unvollständiger
Strukturnachweis wird durch die unten dokumentierte Fortsetzung nach der
tatsächlichen Remoteformatierung ersetzt.
Main übernimmt den vorgeschriebenen vollständigen Root-Graphrefresh nach der
Integration; dieser Fixautor verändert die generierten Graphdateien nicht.

Eine AST-Prüfung ist kein Typ-/Compiler- oder Laufzeitnachweis. Insbesondere
wurden weder ein vollständiger v0.5.184-Prozess als Gegenstelle noch ein echtes
Telefon, MediaProjection, MediaCodec, JNI-Pause/Resume, GL-/GPU-Abschluss oder
physischer Speicherpeak ausgeführt. Der vorbereitete v1-Fall simuliert das alte
Wireformat über isolierte QUIC-Endpunkte; er ist kein behaupteter Altprozesslauf.
Codec-/Treiberpuffer, RSS und GC liegen weiterhin außerhalb der Produktpufferbilanz.
Die Remote-Suite und der terminale Release gehören Main und bleiben offen.

Es verbleibt keine bekannte unbehobene Quellabhängigkeit der vier zugewiesenen
Befunde. Die tatsächliche Typ-/Laufzeitbestätigung ist noch unbekannt. Die
unverifizierten Gerätepfade sind keine bestandene Prüfung und keine
Releaseblockade. Dieser Fixautor hat keine Gitoperation mit Mutation und keine
Release-/Build-/Testorchestrierung ausgeführt.

## Gelesene und geänderte Dateien

Alle relativen Pfade beziehen sich auf die isolierte Arbeitskopie. Die folgende
Liste umfasst Volltext-, Abschnitts-, Suchtreffer- und AST-Lektüre; sie behauptet
keine Laufzeitprüfung. Jede erstellte oder geänderte Datei wurde ebenfalls selbst
gelesen. Reine Dateinamensfunde ohne inhaltliche Prüfung sind nicht als Lektüre
ausgewiesen. Die produktive Schreibfläche entspricht der von Main freigegebenen
Scope-Datei; fremde Änderungen im gemeinsamen Gitstatus wurden nicht übernommen.

### Erstellt und selbst gelesen

- `android/app/src/test/java/app/smartexplorer/android/screen/ScreenSessionBudgetRegressionTest.kt`
- `docs/plaene/2026-10-24-batches/fix-regression-screenbudget.md`
- `native/src/screen/core/batches_session_budget_compatibility_task_tests.rs`
- `native/src/screen/core/session_admission.rs`
- `native/src/screen/os/shared/batches_session_budget_compatibility_task_tests.rs`

### Geändert und selbst gelesen

- `android/app/src/main/java/app/smartexplorer/android/api/ScreenApi.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenHostControl.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenSession.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenViewScreen.kt`
- `docs/oberflaechen/bildschirm.md`
- `native/src/daemon/os/shared/ipc_protocol_screen.rs`
- `native/src/daemon/os/shared/screen_ipc.rs`
- `native/src/daemon/os/shared/screen_ipc_client.rs`
- `native/src/mobile/os/shared/domains/screen_view_json.rs`
- `native/src/mobile/os/shared/domains/screen_view_run.rs`
- `native/src/screen/core/capture_session_context.rs`
- `native/src/screen/core/frame_budget.rs`
- `native/src/screen/core/frame_size_caps.rs`
- `native/src/screen/core/gop_stream.rs`
- `native/src/screen/core/host_capture.rs`
- `native/src/screen/core/host_codec_adapters.rs`
- `native/src/screen/core/host_codec_phase_plan.rs`
- `native/src/screen/core/host_pipeline.rs`
- `native/src/screen/core/host_pipeline_actor.rs`
- `native/src/screen/core/host_server_start.rs`
- `native/src/screen/core/viewer_client.rs`
- `native/src/screen/core/wire_binary.rs`
- `native/src/screen/core/worker_capture_wire.rs`
- `native/src/screen/os/android/bridge_channels.rs`
- `native/src/screen/os/android/bridge_source.rs`
- `native/src/screen/os/android/capture_admission.rs`
- `native/src/screen/os/android/session_memory.rs`
- `native/src/screen/os/android/viewer_sink.rs`
- `native/src/screen/os/linux_os/session_memory.rs`
- `native/src/screen/os/shared/session_memory.rs`
- `native/src/screen/os/windows/session_memory.rs`

### Zusätzlich gezielt gelesene, von diesem Autor unveränderte Dateien

- `AGENTS.md`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenCaptureBudget.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenCaptureOwnership.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenEncoder.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenFrameLimits.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenProjectionFrames.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenSyntheticRetirement.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/SyntheticSurfaceSource.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenDecoder.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenPreviewTile.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenPullBuffers.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenResample.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenScaleWorker.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenTilesScreen.kt`
- `android/app/src/test/java/app/smartexplorer/android/screen/ScreenCaptureBudgetTest.kt`
- `android/app/src/test/java/app/smartexplorer/android/screen/ScreenEncoderOwnershipTest.kt`
- `android/app/src/test/java/app/smartexplorer/android/ui/screen/BatchesScreenBudgetTest.kt`
- `android/app/src/test/java/app/smartexplorer/android/ui/screen/ScreenSessionStateTest.kt`
- `docs/ARCHITEKTUR.md`
- `docs/plaene/2026-10-24-batches/batches.md`
- `docs/plaene/2026-10-24-batches/regressionsreview.md`
- `docs/plaene/2026-10-24-batches/umsetzung.md`
- `docs/plaene/2026-10-24-batches/vertraege.md`
- `graphify-out/graph.json`
- `native/Cargo.lock`
- `native/Cargo.toml`
- `native/android-bridge/src/screen_capture_jni.rs`
- `native/android-bridge/src/screen_jni.rs`
- `native/review-task-manifest.json`
- `native/src/app/core/screen_decode_run.rs`
- `native/src/app/core/screen_decode_thread.rs`
- `native/src/app/core/screen_pool.rs`
- `native/src/app/core/screen_session.rs`
- `native/src/app/core/screen_session_backend.rs`
- `native/src/app/core/screen_viewer_fake.rs`
- `native/src/cli/mcp/screen_tool.rs`
- `native/src/cli/screen.rs`
- `native/src/cli/screen/bench.rs`
- `native/src/cli/screen/input.rs`
- `native/src/cli/screen/snapshot.rs`
- `native/src/cli/screen/windows.rs`
- `native/src/daemon/mod.rs`
- `native/src/daemon/os/shared/ipc_protocol.rs`
- `native/src/daemon/os/shared/screen_ipc_still.rs`
- `native/src/login/core/batches_capture_task_tests.rs`
- `native/src/login/os/shared/capture_admission.rs`
- `native/src/login/os/shared/client.rs`
- `native/src/login/os/shared/gateway_client.rs`
- `native/src/login/os/shared/helper_capture.rs`
- `native/src/login/os/shared/viewer_images.rs`
- `native/src/mobile/os/shared/domains/login_images.rs`
- `native/src/mobile/os/shared/domains/login_view.rs`
- `native/src/mobile/os/shared/domains/screen_host.rs`
- `native/src/mobile/os/shared/domains/screen_view.rs`
- `native/src/mobile/os/shared/domains/screen_view_tests.rs`
- `native/src/screen/core/batches_capture_admission_task_tests.rs`
- `native/src/screen/core/batches_capture_pipeline_task_tests.rs`
- `native/src/screen/core/batches_frame_budget_task_tests.rs`
- `native/src/screen/core/batches_snapshot_task_tests.rs`
- `native/src/screen/core/capture_admission.rs`
- `native/src/screen/core/capture_authority.rs`
- `native/src/screen/core/capture_bytes.rs`
- `native/src/screen/core/capture_grant.rs`
- `native/src/screen/core/capture_grant_claims.rs`
- `native/src/screen/core/capture_grant_slices.rs`
- `native/src/screen/core/capture_image.rs`
- `native/src/screen/core/capture_plan.rs`
- `native/src/screen/core/capture_records.rs`
- `native/src/screen/core/capture_source_contract.rs`
- `native/src/screen/core/capture_test_support.rs`
- `native/src/screen/core/decoded_frame.rs`
- `native/src/screen/core/frame_memory.rs`
- `native/src/screen/core/frame_memory_tests.rs`
- `native/src/screen/core/frame_protocol.rs`
- `native/src/screen/core/frame_reference.rs`
- `native/src/screen/core/frame_regions.rs`
- `native/src/screen/core/host_admission.rs`
- `native/src/screen/core/host_backend.rs`
- `native/src/screen/core/host_capture_pause.rs`
- `native/src/screen/core/host_mode.rs`
- `native/src/screen/core/host_pipeline_init.rs`
- `native/src/screen/core/host_pipeline_produce.rs`
- `native/src/screen/core/host_quality.rs`
- `native/src/screen/core/host_reference.rs`
- `native/src/screen/core/host_server.rs`
- `native/src/screen/core/host_session.rs`
- `native/src/screen/core/host_stage.rs`
- `native/src/screen/core/image_admission.rs`
- `native/src/screen/core/negotiate.rs`
- `native/src/screen/core/pixel_convert.rs`
- `native/src/screen/core/registry.rs`
- `native/src/screen/core/session_state.rs`
- `native/src/screen/core/snapshot.rs`
- `native/src/screen/core/tiles_codec.rs`
- `native/src/screen/core/viewer_reconstruct.rs`
- `native/src/screen/core/wire_hello.rs`
- `native/src/screen/os/android/batches_capture_task_tests.rs`
- `native/src/screen/os/android/bridge_backend.rs`
- `native/src/screen/os/android/capture_arena.rs`
- `native/src/screen/os/android/capture_prepared.rs`
- `native/src/screen/os/linux_os/capture_prepared.rs`
- `native/src/screen/os/linux_os/host_backend.rs`
- `native/src/screen/os/linux_os/login_source.rs`
- `native/src/screen/os/linux_os/pipewire_capture_memory.rs`
- `native/src/screen/os/linux_os/x11_capture.rs`
- `native/src/screen/os/linux_os/x11_capture_memory.rs`
- `native/src/screen/os/mod.rs`
- `native/src/screen/os/shared/host_worker_prepared.rs`
- `native/src/screen/os/shared/image_memory.rs`
- `native/src/screen/os/windows/batches_capture_task_tests.rs`
- `native/src/screen/os/windows/capture_dxgi.rs`
- `native/src/screen/os/windows/capture_prepared.rs`
- `native/src/screen/os/windows/capture_recovery.rs`
- `native/src/screen/os/windows/capture_slots.rs`
- `native/src/screen/os/windows/capture_source.rs`
- `native/src/screen/os/windows/capture_wgc.rs`
- `native/src/screen/os/windows/host_backend.rs`
- `native/src/screen/os/windows/login_source.rs`
- `native/src/share/core/node_screen.rs`
- `native/src/share/core/node_screen_link.rs`

### Außerhalb der Arbeitskopie gelesene Auftrags- und Fehlerbelege

- `/home/CodeProjects/smartexplorer/smart-explorer/docs/plaene/2026-10-24-batches/review-gesamt.md`
- `/home/CodeProjects/smartexplorer/smart-explorer/docs/plaene/2026-10-24-batches/review-regression-astra.md`
- `/home/CodeProjects/smartexplorer/smart-explorer/docs/plaene/2026-10-24-batches/review-regression-sol.md`
- `/root/.codex/AGENTS.md`
- `/root/.codex/skills/graphify/SKILL.md`
- `/tmp/smartexplorer-final-review-user-contract.txt`
- `/tmp/smartexplorer-fix-screen-budget-scope.json`

Die beiden ursprünglichen Befundberichte und `review-gesamt.md` im Main-Baum
wurden ausschließlich gelesen beziehungsweise gezielt nach den zugewiesenen IDs
durchsucht. Kein Produktcode und kein Bericht im Main-Baum wurde verändert.

## Fortsetzung nach Remote-Vorbereitung 40332971455: fachliche Modulgrößen

Diese Fortsetzung gehört zu demselben Autorzyklus. Ausgang ist der bereits
übernommene, an den Kandidaten gebundene Patch `10988167`. Die Remotevorbereitung
hat die Größen nach normaler Formatierung festgestellt; vor einer Kompilierung
scheiterte der Vorbereitungsschritt. Compiler- und Gerätejobs wurden übersprungen.
Die frühere Größenangabe dieses Berichts bezog sich auf den damaligen dichten
Quelltext und belegte die geforderte Größe nach üblicher Formatierung nicht.
Diese Lücke wird durch fachliche Aufteilung geschlossen, ohne Zeilenverdichtung.

### Erstplan und konkret belegter Bestand

Ergebnisauftrag: Die sechs benannten Verantwortlichkeiten und ihre neuen Module
bleiben jeweils unter 500 Zeilen und 50 KiB. Verhalten, Besitzer, Berechtigungen,
Wireformate und sämtliche exakten Fallnamen bleiben erhalten. Main besitzt Git,
Manifest, Rootgraph und dieselbe spätere Remote-Suite. Dieser Autor startet keine
Compiler, Formatter, Tests, Server oder App-/Hostprozesse und keine Unteragenten.

Die Dateien haben am formatierten Ausgang 524 (`screen_decode_thread.rs`),
598 (`host_session.rs`), 580 (`bridge_channels.rs`), 648 (`bridge_source.rs`),
597 (`viewer_sink.rs`) und 573 Zeilen (gemeinsame Budgetfixture). Die bytegrößten
beiden dieser Quellen haben 25 886 und 23 232 Bytes; das Problem ist hier die
Zeilenzahl. Konkrete Quellen: die sechs Dateien sowie die gezielt auf diese
Dateien eingegrenzte Größeninventur der Vorbereitung. Keine Quellfunktion wird
als neuer funktionaler Fehler bewertet; fehlerhaft ist ihre Modulabgrenzung nach
dem durchgesetzten Größenvertrag.

Die Stufe-eins-Entscheidung trennt Ereignissteuerung von Bildverarbeitung,
Sitzungslauf von Steuerframeverarbeitung, Kanalregistrierung vom geschützten
Kanalzustand, rohe Androidbilder von ihrer Codierung und Pulltransport von
Rekonstruktion. Ressourcenfelder und ihre Deklarations-/Dropreihenfolge bleiben
in ihren bisherigen Typen. Die Budgetfälle werden nach Capture-Neustart und
Transport geteilt; ihre bisherigen registrierten Testeinstiege bleiben erhalten.

### Recherche und finaler Strukturplan vor der Umsetzung

Die bestehenden Untermodulmuster `host_input_release.rs` und
`host_visibility.rs` benutzen einen privaten `#[path]`-Anschluss und
`use super::*`. Die [Rust-Modulreferenz](https://doc.rust-lang.org/reference/items/modules.html)
und [Rust-Sichtbarkeitsreferenz](https://doc.rust-lang.org/reference/visibility-and-privacy.html),
abgerufen am 2026-10-26, bestätigen die relative Quelldateiauflösung, Zugriff der
Kindmodule auf private Elternfelder und `pub(super)` als Grenze für zurückgerufene
Methoden. Es wird keine Abhängigkeit und kein öffentlicher Modulpfad ergänzt.

Die zweite Lückenrecherche prüft die zurücklaufenden Methodenaufrufe, vorhandene
Testmodule, `super`-Pfade und unveränderte Zugriffsstufen. Reexporte erhalten die
vorhandenen Namen. Die konkrete Umsetzung nutzt vollständige Methodenblöcke;
es werden keine Besitzfelder in Hilfsobjekte umgehängt und keine Sperren geteilt.

| Strukturschritt | Zielmodule und konkrete Grenze | Erwarteter statischer Beleg |
|---|---|---|
| S1 Desktopdecode | `screen_decode_thread.rs` behält Decoderzustand, Record-/Framezuordnung und Ausgabe. `screen_decode_events.rs` übernimmt `on_event`, `connection_lost` und Hostereignisse. | Familien-/Accepted-/Kontrollereignisse erreichen dieselben Methoden. Decoder und ausstehende Bilder enden weiterhin vor dem Sitzungslease; Darstellung und Verlustfreiheit behalten ihre Identitäten. |
| S2 Hoststeuerung | `host_session.rs` behält Lauf, Produktion, Berichte und Abschluss. `host_session_control.rs` übernimmt Steuerframes, Eingabe und Kontrollfreigabe. | Unveränderte Rechte-, Sichtbarkeits-, Referenz-, Snapshot-, Pending-, Rückfrage- und Releasebedingungen; die abschließende Freigabe erreicht dieselbe Methode. |
| S3 Androidkanal | `bridge_channels.rs` behält Typen und Prozessregister. `bridge_channel_state.rs` übernimmt die Methoden des bestehenden `Channel`. | Genau dieselbe Mutex-/Condvargrenze schützt Epochwechsel, Queuebilder, Grants, Aktivitätsquittung, Ende und Capturebeginn. |
| S4 Androidquelle | `bridge_source.rs` behält Source-/Stage-Vertrag, Aktivität und Drop. `bridge_source_input.rs` übernimmt Still-Anforderung und RAW-Decodierung; `bridge_source_capture.rs` übernimmt Tiles/H.264. | Gleiche Epoch-/Generationswahl, native Referenzfreigabe, Grants, Paaridentität, Fristen, Fallback und Zustimmung; der öffentliche `decode_still`-Name bleibt erreichbar. |
| S5 Androidviewer | `viewer_sink.rs` behält GOP-Rekonstruktion und Typen. `viewer_sink_queue.rs` übernimmt den bestehenden `PullQueue`-Algorithmus. | Lokale Legacygrenzen, vollständige GOP-/Frameidentität, Stau, Backpressure, Pullpuffer und Abschluss bleiben unverändert. Ein zusätzliches Decode-Modul ist für diese fachliche Trennung nicht erforderlich. |
| S6 Budgetfälle | Die gemeinsame Fixture behält Transport-/Policyfälle und ihre registrierten Einstiege. `batches_android_capture_restart_task_tests.rs` übernimmt Capture-Neustart-, Snapshot- und Queue-/Epochlogik. | Die bisherigen vollständigen Testpfade und Fallnamen bleiben durch schlanke Aufrufe erhalten; alle bisherigen Fallkörper werden genau einmal aufgerufen. |

S1–S6 sind unabhängig schreibbar und werden sequenziell durch diesen Autor
umgesetzt. Danach folgen die eigene vollständige statische Nachreview, ein
AST-Vergleich der verschobenen Körper, Prüfung der Sichtbarkeits-/Modulanschlüsse,
der unveränderten Testeinstiege sowie normale Zeilen-/Bytegrößen. Diese Erwartungen
autorisieren keinen lokalen Compiler-, Formatter- oder Testlauf.

### Umsetzung und eigene anschließende Anti-Regressionsreview dieser Fortsetzung

S1–S6 sind umgesetzt. Der gleiche Fixautor hat anschließend die Anschlüsse und
die vollständigen betroffenen Vertragsketten erneut geprüft. `DecodeState`,
`Session`, `Channel`, `BridgeSource`, `PullQueue`, `ViewerSink`, ihre Grant-/Lease-
Felder und sämtliche Queuetypen bleiben dieselben Typen. Die Definitionen ihrer
Felder, Konstanten und Zustandsvarianten sind gegenüber `10988167` unverändert.
Insbesondere bleibt die Deklarationsreihenfolge von Decoder, ausstehenden Bildern,
gemeinsamem Zustand und Sitzungslease erhalten.

Der AST-Vergleich findet jeden ursprünglichen Funktionskörper genau als
unveränderten Körper in seinem Ursprungsmodul oder seinem neuen Fachmodul wieder.
Ausgenommen sind ausschließlich die notwendigen zusätzlichen `super`-Stufen in
den qualifizierten Modulpfaden. Es wurde keine Bedingung, Frist, Claimgröße,
Queueoperation, Ausnahmebehandlung oder Abschlussoperation verändert. Methoden,
die aus dem Elternmodul zurückgerufen werden, erhalten `pub(super)`; bereits
crateweite Einstiege behalten ihre bisherige Sichtbarkeit. Die Scopeerweiterung
endet damit an derselben bisherigen Elternmodulgrenze. Alle neuen `#[path]`-
Anschlüsse zeigen auf vorhandene Dateien innerhalb der erlaubten Schreibfläche.

| Fortschreibung der vollständigen betroffenen Kette | Eigener statischer Nachbeleg |
|---|---|
| Desktop `screen_decode_run` → `DecodeState::on_event` in `screen_decode_events` → `record`/`show` in `screen_decode_thread` | Accepted-/Familienvertrag, Kontrollanforderung, Fehler-/Endereignis, Schlüsselbildanforderung und GOP-/Framezuordnung besitzen unveränderte Körper. Decoderdrain, gehaltene Bilder und Lease bleiben in derselben Zustandsinstanz. |
| `host_server::session` → `Session::run` → `on_frame` in `host_session_control` → vorhandene Family-/Visibility-/Input-Release-Module | View/Control, Preview/Full, Rechte, Fenster-/Iconanfragen, Snapshot, Referenzboden, gedrückte Eingaben, Freigabe, Stop und Sitzungsabschluss benutzen dieselben Aufrufe und Bedingungen. `finish` ruft dieselbe `release_control`-Methode auf. |
| JNI/Bridge → `bridge_channels` → `Channel`-Methoden in `bridge_channel_state` → vorhandene `CaptureSession`/`CaptureAuthority` | Register und Kanalidentität bleiben im Elternmodul. Queueabbau und Epochbindung bleiben unter derselben Sperre; externe Slices, Aktivitätsquittung, Rückfrage, Ende und Capturebeginn behalten ihren bisherigen tatsächlichen Besitz. |
| `BridgeSource::produce`/`capture` → `bridge_source_input`/`bridge_source_capture` → derselbe Kanal und dieselben Grants | RAW-/H.264-Paaridentität, Frames, Generationen, Encoderreferenzen, Epochwechsel, Deadline, Zustimmung und Tiles-Fallback bleiben erhalten. `decode_still` wird am bisherigen Modulpfad crateweit reexportiert. `set_capture_active` und `Drop` bleiben im Ursprungsmodul; der Kotlin-Drain-/Retirementanschluss bleibt unverändert. |
| Mobile Stream → `ViewerSink::record` → `PullQueue`-Methoden in `viewer_sink_queue` → JNI-Pull/Kotlin | Tatsächliche lokale Legacygrenzen, GOP-/Framepräfixe, Queuebudgets, Backpressure, Schlüsselbild-/Stauzustand, zu kleiner Pullpuffer und tatsächliches Schließen bleiben unverändert. Neue Queue-, Mutex-, Condvar- oder Prozessbudgetinstanzen entstehen nicht. |
| Bestehende `session_memory::compatibility_tests`-Einstiege → `android_capture_restart` | Reihenfolge und vollständige registrierte Testnamen sind identisch zum formatierten Ausgang. Jeder der vier bisherigen Android-Capturefälle ruft seinen unveränderten Körper genau einmal auf. Der neue Fachmodulname erzeugt keinen zusätzlichen registrierten Test und erfordert keinen neuen Manifestfilter. |

Die ursprünglichen statischen Funktions- und Besitzbelege dieses Berichts gelten
damit weiter. Legacy-Wireformat, private IPC, Android-Vorschau-/Vollansichtsübergang,
Loginverbraucher, Source-/Sessionidentität und der gemeinsame Prozesspool erhalten
keine neue Schnittstelle. Es gibt keine neue Benutzeraktion, Wartebedienung oder
geänderte Fehlerkorrektur. Der Großteil dieser unveränderten Verbraucher musste
nicht erneut implementiert werden; die neue Modulgrenze endet unter ihren
vorhandenen öffentlichen Einstiegen.

### Dateiübersicht und konkrete Größen nach der Aufteilung

Alle folgenden Dateien wurden in dieser Fortsetzung durch denselben Autor
gelesen und geändert beziehungsweise erstellt. Vollständige Körper wurden aus
dem bereits remoteformatierten Ausgang übernommen; neue Anschlüsse und Methoden-
köpfe verwenden normale Rustformatierung. Es fand kein lokaler Formatterlauf statt.

| Datei | Status in dieser Fortsetzung | Zeilen | Bytes |
|---|---|---:|---:|
| `native/src/app/core/screen_decode_thread.rs` | geändert | 318 | 11 909 |
| `native/src/app/core/screen_decode_events.rs` | erstellt | 214 | 8 580 |
| `native/src/screen/core/host_session.rs` | geändert | 375 | 14 138 |
| `native/src/screen/core/host_session_control.rs` | erstellt | 231 | 9 308 |
| `native/src/screen/os/android/bridge_channels.rs` | geändert | 378 | 13 059 |
| `native/src/screen/os/android/bridge_channel_state.rs` | erstellt | 207 | 7 490 |
| `native/src/screen/os/android/bridge_source.rs` | geändert | 260 | 9 504 |
| `native/src/screen/os/android/bridge_source_input.rs` | erstellt | 84 | 3 133 |
| `native/src/screen/os/android/bridge_source_capture.rs` | erstellt | 321 | 13 651 |
| `native/src/screen/os/android/viewer_sink.rs` | geändert | 401 | 15 326 |
| `native/src/screen/os/android/viewer_sink_queue.rs` | erstellt | 201 | 7 419 |
| `native/src/screen/os/shared/batches_session_budget_compatibility_task_tests.rs` | geändert | 266 | 10 487 |
| `native/src/screen/os/shared/batches_android_capture_restart_task_tests.rs` | erstellt | 328 | 11 398 |

Der einzige geänderte Bericht ist weiterhin diese Datei. Die erlaubte optionale
Extraktion `viewer_sink_decode.rs` wurde nicht benötigt und nicht erstellt.
Zusätzlich gezielt gelesen wurden die aktuelle Scope-Datei, der eigene bisherige
Bericht, die sechs passenden Einträge in `/tmp/smartexplorer-rv1-size-inventory.json`,
die Metadaten von `/tmp/smartexplorer-rv1-40332971455/format.json`,
`native/src/app/mod.rs`, `native/src/screen/os/mod.rs`,
`native/src/screen/core/host_server.rs`, `host_input_release.rs`,
`host_visibility.rs`, `batches_snapshot_task_tests.rs`,
`native/src/app/core/batches_decode_lifetime_task_tests.rs`,
`native/src/screen/os/android/batches_capture_task_tests.rs` und
`native/src/screen/os/shared/session_memory.rs`. Bloße nicht existente
Dateinamensannahmen wurden nicht als gelesene Dateien erfasst.

### Tatsächliche Prüfungen und verbleibende Grenzen dieser Fortsetzung

Durchgeführt wurden ausschließlich Quellen-/Dokumentlektüre, Edits und statische
Text-/AST-Prüfungen mit der vorhandenen Tree-sitter-Rustgrammatik. Der Vergleich
mit `10988167` meldet keine geänderten oder fehlenden ursprünglichen Körper,
keine geänderte Daten-/Konstantendefinition, keinen Syntaxfehler, keinen fehlenden
Modulpfad, keine veränderten registrierten Testnamen und keine Größenverletzung
in den zugewiesenen Dateien. Die Kontroll- und Besitzketten wurden darüber hinaus
vom selben Autor selbst gegen die neuen Sichtbarkeitsgrenzen geprüft.

Keine lokale Build-, Compiler-, Formatter-, Test-, Server-, Geräte- oder
Hostausführung fand statt. Es gibt keine neue funktionale Fallregistrierung:
Die oben aufgeführten exakten Fälle und die Kotlin-Testklasse bleiben gültig.
Die erneute Bestätigung durch Mains denselben Remote-Suiteeinstieg steht aus;
insbesondere sind Typprüfung, eine erneute Remoteformatierung und Laufzeitverhalten
nach der Aufteilung noch unbekannt. Main behält Commit/Push, Manifest, Rootgraph
und die weitere Suite-/Releaseorchestrierung. Innerhalb der zugewiesenen
Strukturkorrektur bleibt keine bekannte ungeklärte Quellabhängigkeit.

Der abschließende Vergleich umfasst zusätzlich die ursprünglichen Signaturen;
abgesehen von den dokumentierten internen Sichtbarkeiten sind auch diese
unverändert. Die registrierten Testaufrufe besitzen jeweils genau einen Aufruf
des verschobenen Körpers. `git diff --check` sowie die ergänzende Prüfung der
neuen Dateien auf nachgestellten Whitespace und zusätzliche Leerzeilen am
Dateiende melden keinen Befund. Die normalen Dateigrößen der zugewiesenen
Verantwortlichkeiten liegen abschließend zwischen 84 und 401 Zeilen und
zwischen 3 133 und 15 326 Bytes. Keine Gateausnahme wurde eingeführt.

## Fortsetzung nach Remote-Lauf 40354605251: Retry, ViewerHidden und Snapshotgrant

Ausgang: `d95dc404e834edfe7be0e1c96be162b36865efa9`. Die vollständigen Diagnosen
`/tmp/smartexplorer-rv1-40354605251/screen-budget-errors.json` wurden vor der
Ursachenentscheidung gelesen. Sie belegen C27 (lokaler Retry-Namenskonflikt),
C46 (fehlender Grant im Snapshotquellinitialisierer), C55 (fehlender Statustext)
und C58/A19 (derselbe fehlende ViewerHidden-Übergangsarm unter Windows/Android).
Alle Jobs dieses Laufs sind laut delegiertem Auftrag beendet. Dieser Autor
startet keine lokale Kompilierung und keinen lokalen Testlauf.

### Erstplan, Fehlerfunktionen und vollständige betroffene Ketten

Ergebnisauftrag dieser Fortsetzung: Die vorhandene Wiederholung bleibt per
Kachelklick bedienbar. Eine nicht sichtbare Ansicht behält ihren eigenen
Pausenzustand, gültige Eintritts-/Wiederaufnahmeübergänge und einen verständlichen
Status. Die vorhandene Snapshotfixture erhält eine echte Capture-Zulassung;
ihre Budget-, Bildidentitäts- und Besitzerlebensaussagen bleiben vollständig
erhalten. Varianten, Ressourcenprüfungen und Fixtureaussagen werden nicht entfernt.

| Fehlerfunktion | Vollständige tatsächlich betroffene Kette und Erhalt |
|---|---|
| `screen_tile_face::draw_tile` | `ScreenTileGrid::show`/`step` → `draw_tile` → bestehendes `retry(Tile)` → Probe `Idle`, Beenden der bisherigen Sitzung, Bild-/Texturabbau und Sichtbarkeitsinvalidation → nächster Probe-/Previewstart. Eine lokale Boolbindung `retry` verdeckt den Funktionsnamen. Cursor, Klickbedingung und Wiederholungsretirement bleiben gleich. |
| `SessionState::can_move_to`/`is_paused` | `ViewerVisibility::hidden` → Host-Visibilitygeneration und Capture-Aktivitätswechsel → echte Aktivitätsquittung → `Session::produce` meldet `ViewerHidden` → gemeinsame Wire-/IPC-/CLI-Zustandsform und `SessionTracker`. Wiederanzeigen führt weiter zu Starting/Streaming beziehungsweise einer vorhandenen Hostpause. Rechte, Verbindung, Eingabereferenzboden und tatsächlicher Captureabschluss bleiben eigenständige vorhandene Grenzen. |
| `screen_texts::state_text` | Host-State → Desktop-`Shared`/`Info` → Statuszeile und `Tile::face` → vorhandene Darstellung. Nur der neue bereits bestehende Zustand hat keinen Matcharm. `Streaming` behält als einziger Zustand seinen leeren Text; vorhandene Fehler- und Beendigungsursachen bleiben erhalten. |
| Snapshotfixture `batches_task_b6_snapshot_returns_to_new_live_keyframe_without_stale_presentation` | `capture_test_support::session` → isolierter echter MemoryPool/SessionAdmission → vertragsgebundener CapturePlan → `CaptureAuthority::approve` → `Source.grant`/`capture_grant` → `CaptureImageBuilder`/CapturedImage → Pipeline, eigener Snapshotstrom und neues Live-Schlüsselbild. Der zweite `Source`-Initialisierer lässt das bereits erforderliche Feld aus. Der vorhandene Quellhelfer erzeugt echte Claims statt einer weiteren Dummy-/Bypassbefugnis. |

Die Quellenaufnahme bestätigt: Der Host begrenzt die Hidden-Meldung nicht auf
Streaming. Eine angenommene Sitzung kann auch während Start, erneuter Rückfrage,
Portalauswahl, nativer Pause oder NoDisplay unsichtbar werden. Die Pause betrifft
die Bildaufnahme dieser Ansicht; die Steuersitzung bleibt bestehen. Connecting
ist noch keine angenommene laufende Sitzung und bekommt dadurch keinen direkten
Hidden-Einstieg. Ended bleibt endgültig, Unknown bleibt der vorhandene
Vorwärtskompatibilitätsfall.

### Recherche, Entscheidungen und konkreter Fixplan vor der Umsetzung

Die [Rust-Referenz zu lokalen Namensbindungen](https://doc.rust-lang.org/reference/names/scopes.html),
abgerufen am 2026-10-26, bestätigt die Reichweite und das Verdecken eines
Funktionsnamens durch eine lokale Bindung. Für C27 genügt deshalb ein eindeutiger
Boolname; ein Umbau der Wiederholungsfunktion ist nicht erforderlich.

Die Zustandsentscheidung folgt der gelesenen Hostemission und der vorhandenen
Tabelle. ViewerHidden wird eine echte Pause; die bisherigen Quellenpausen bleiben
erhalten. Ein eigener expliziter Eintrittsarm erlaubt die vorhandenen angenommenen
Lauf-/Wartezustände und NoDisplay. Eine Catch-all-Freigabe, Löschen der Variante
oder Umdeuten zu Streaming würden den Vertrag verletzen und werden verworfen.
Die zweite Lückenrecherche hat zudem zwei bestehende Vollständigkeitsfälle
gefunden, deren Listen noch zwölf Zustände enthalten; diese werden unter
Erhalt ihrer Namen und bisherigen Aussagen ergänzt.

Der Snapshotquellhelfer verwendet bereits einen isolierten MemoryPool, den
gebundenen Framevertrag, SourceKey/Generation und den gemeinsamen
CaptureAuthority-Pfad. Dessen synthetische Kapazitätsfakten sind Fixturebedingungen,
kein behaupteter physischer Speicherbeleg. Der fehlende Initialisierer soll
denselben vorhandenen Helfer mit derselben Bildgröße und demselben Vertrag nutzen.
Ein optionaler Grant oder das Entfernen der Snapshot-/Präsentationsaussagen ist
keine zulässige Alternative.

| Fixpunkt | Konkrete Änderung | Erwarteter Nachweis |
|---|---|---|
| F1 C27 | Lokale Bedingung in `screen_tile_face.rs` wird `can_retry`; der tatsächliche Aufruf `retry(tile)` bleibt erhalten. | Derselbe Klick und dieselben drei Fehlergesichter erreichen dieselbe Retirementfunktion ohne Namensverdeckung. |
| F2 C58/A19 | `ViewerHidden` in `is_paused`; eigener Eintrittsarm in `can_move_to` für angenommene Lauf-/Wartezustände und NoDisplay. | Bestehende Übergänge zwischen bisherigen Zuständen bleiben unverändert. Hidden kann wieder anlaufen, in eine Hostpause wechseln und enden; Connecting und Ended erhalten keinen neuen Ausweg. |
| F3 C55 | Eigener verständlicher Statustext in `screen_texts.rs`; keine Wildcard und keine verkürzte Zustandsliste. | Vollständigkeitsfall enthält alle dreizehn vorhandenen Varianten, eindeutigen Hidden-Text und weiterhin ausschließlich leeres Streaming. |
| F4 C46 | Der zweite Snapshot-`Source` erhält `capture_test_support::grant(size, contract)`. | Derselbe reale Plan finanziert Source-/Ausgabebesitzer; Snapshot bleibt auf eigener GOP und die Rückkehr erfordert ein neues Live-Schlüsselbild mit gültiger Präsentation. |
| F5 Eigene Nachreview | Zustands-/Text-Vollständigkeitsfälle ergänzen und die gesamte obige Kette selbst statisch prüfen. | Bestehende Fallnamen und Aussagen bleiben erhalten; AST, normale Datei-/Bytegrößen und Whitespace ohne Befund. Praktische Bestätigung ausschließlich in Mains derselben Remote-Suite. |

Vor Produktedits ist die Funktionskarte damit aus dem aktuellen Bestand
abgeleitet. Für die beiden zusätzlichen bestehenden Vollständigkeitsdateien
wurde Main eine konkrete Schreibfreigabe gemeldet; bis dahin werden sie nicht
verändert. `app/screen_modules.rs` bleibt ausschließlich lesbar und beim
Captureautor. Keine Anpassung seiner Registrierungen ist für F1–F4 erforderlich.

Erweiterung der F5-Karte vor der zugehörigen Fallergänzung: Mains gezielte
Symbolsuche nennt zusätzlich zu Tabelle und Zustandsfall genau einen produktiven
`is_paused`-Aufrufer, `screen_indicator_ui::watching`. Dieser Autor hat die
inzwischen freigegebene Datei vollständig gelesen: Pausensitzungen müssen im
Freigabeindikator bleiben. Andernfalls würde er bei ausschließlich versteckten
Viewern nach seiner Leerfrist selbst schließen, obwohl die Steuersitzung noch
besteht. Die ViewerHidden-Pausenklassifikation erhält diesen C7-Vertrag. Der
bestehende Fall `screen_task_gui_indicator_lines` erhält eine zusätzliche
Erhaltungsassertion; Produktionslogik und bisherige Indikatoraussagen bleiben
unverändert. Main hat die genaue Datei vor dieser Änderung schreibbar freigegeben.

### Umsetzung und eigene anschließende Anti-Regressionsreview zu C27/C46/C55/C58/A19

F1–F5 sind umgesetzt; derselbe Autor hat danach die gesamte ermittelte Kette
selbst nachgeprüft. Die zusätzlichen Schreibfreigaben für beide vorhandenen
Vollständigkeitsfälle und die Indikatorassertion wurden jeweils vor deren
Änderung von Main erteilt und in die exakte Scope-Datei aufgenommen.

- C27: Nur die lokale Boolbindung und ihre zwei Verwendungen heißen jetzt
  `can_retry`. Der Quellvergleich mit `d95dc404` bestätigt, dass der gesamte
  übrige Kachelcode unverändert ist. Cursor, Klick, die drei wiederholbaren
  Fehlergesichter und der tatsächliche Aufruf `retry(tile)` bleiben erhalten.
  `retry` besitzt weiterhin seinen ursprünglichen Körper. `tile.session = None`
  führt über den vorhandenen `ViewerSession::drop`-/`close`-Weg; gemeinsame
  Decoder-/Bildbesitzer und ihre Leases werden dadurch nicht vorzeitig beendet.
- C58/A19: `ViewerHidden` ist jetzt eine Pause. Der explizite Eintrittsarm
  berücksichtigt die vorhandenen Lauf- und Wartezustände sowie NoDisplay.
  Alle Übergänge zwischen bisherigen Zuständen bleiben unverändert: Ihre
  Pausenklassifikation und bisherigen Matcharme wurden nicht verändert.
  Hidden kann zu Starting/Streaming, vorhandenen Hostpausen, Rückfrage/Portal
  und NoDisplay wechseln. Der Ended-Schutz, das Verbot des direkten
  Connecting→Hidden-Wechsels, wiederholte Zustände und Unknown bleiben erhalten.
  Der bestehende Zustandsfall enthält alle dreizehn Varianten und die zusätzlichen
  Hidden-Schritte. Sein bisheriger Pfad bleibt als Reihenfolge erhalten; die
  Endzeitmarke wurde entsprechend den zusätzlichen Schritten fortgeschrieben.
  Bestehende End-, NoDisplay-, Unknown-, Serialisierungs- und Schlüsselbildregeln
  sind weiter Bestandteil desselben Falls.
- C55: `state_text` besitzt einen eigenen Hidden-Arm mit dem Text „Diese Ansicht
  ist nicht sichtbar; die Bildübertragung pausiert“. Er erklärt die angehaltene
  Bildübertragung ohne einen Verbindungsabbruch zu behaupten. Der vorhandene
  Vollständigkeitsfall prüft durch die erweiterte Zustandsliste weiterhin
  eindeutige Texte und ausschließlich leeres Streaming. Kein Fehlertext oder
  Beendigungsgrund wurde ersetzt oder entfernt.
- Indirekter Indikatorvertrag: `screen_indicator_ui::watching` behält eine
  versteckte Pausensitzung nun wie die vorhandenen Quellenpausen im aktiven
  Freigabeindikator. Sichtbarkeit, Beenden einer Sitzung und Alle-beenden bleiben
  verfügbar; der Indikator schließt bei einer solchen offenen Sitzung nicht
  aufgrund seiner Leerfrist. Der bestehende Fall enthält die neue Erhaltungs-
  assertion. Der Quellvergleich bestätigt unveränderte Produktionslogik und
  unveränderte bisherige Indikatoraussagen.
- C46: Der zweite Snapshotquellinitialisierer verwendet jetzt den bestehenden
  echten `grant(size, contract)`-Helfer. Dessen Session, SourceKey, Generation,
  Planbindung, SourcePixels-Slots und Ausgabeplan werden vor dem ersten Bild
  durch dieselbe CaptureAuthority genehmigt. Der Quellbesitzer hält den Grant;
  erzeugte CapturedImages und Records behalten ihre eigenen tatsächlichen
  Slice-/Grantlebensdauern. Es wird keine zusätzliche Kasse eingeführt. Der
  Quellvergleich bestätigt, dass der gesamte übrige Snapshotfixturecode samt
  jeder bisherigen Aussage unverändert ist: Größen-/Familienvertrag, gesonderte
  Snapshot-GOP, Ablehnung alter Präsentationen, erforderliches neues Live-
  Schlüsselbild und anschließende neue Präsentationsfreigabe bleiben erhalten.

Eine Pause ist weiterhin kein Ressourcenabschluss und keine neue Eingabefreigabe.
Host-Visibilitygeneration, echte Capture-Aktivitätsquittung, Referenzboden und
Draw-/Präsentationsbeleg bleiben vor der Eingabe wirksam. Die Zustandskorrektur
verändert keine Quelle, Relation, Geräteidentität, Berechtigung oder bestehende
Screen-/Login-Allokationsgrenze. Insbesondere wird der vorhandene Wirezustand
`viewer_hidden` nicht umbenannt und keine Protokollvariante hinzugefügt.

### Genaue Fälle für denselben Remote-Suiteeinstieg

Kein Fall wurde lokal ausgeführt. Main hat die Aufnahme der drei vorhandenen
Fälle `screen_task_session_state_moves_and_keyframe_requests`,
`screen_task_viewer_state_texts_complete` und `screen_task_gui_indicator_lines`
in denselben Suiteeintrag vorgemerkt; dieser Autor bearbeitet das Manifest nicht.
Die vorhandene Snapshotgruppe ist bereits über `batches_task_b6_` ausgewählt.

| Bestehender unveränderter Fallname | Konkreter erwarteter Erhalt |
|---|---|
| `screen_task_session_state_moves_and_keyframe_requests` | Hidden-Eintritt und Wiederaufnahme bei angenommenen Lauf-/Warte-/NoDisplay-Sitzungen, vorhandene Quellenpausen, terminales Ended, Connecting-/Unknown-Grenzen und bestehende Schlüsselbildregel. |
| `screen_task_viewer_state_texts_complete` | Alle dreizehn Zustände haben ihre beabsichtigte Darstellung; nur Streaming bleibt leer. Bestehende Fehler-, End- und Eingabeblocktexte bleiben abgedeckt. |
| `screen_task_gui_indicator_lines` | Eine offene versteckte Freigabe bleibt im Indikator sichtbar und beendbar; alle bisherigen Modus-/Quell-/Raum- und Pauseaussagen bleiben erhalten. |
| `batches_task_b6_snapshot_returns_to_new_live_keyframe_without_stale_presentation` | Reale Capture-Zulassung des vorhandenen Quellplans, eigene Snapshot-GOP und neue Live-Präsentation nach echtem Schlüsselbild. |
| `batches_task_b6_snapshot_preserves_accepted_limits_family_and_legacy_pixels` | Unveränderter Begleitfall: angenommene Bildgrenzen, Familie, Legacy-Pixel und tatsächliche Recordbesitzer. |
| `batches_task_b6_abandoned_capture_output_retires_records_before_lease` | Unveränderter Begleitfall: aufgegebene Ausgabe beendet Records und Quelle vor Freigabe ihres Leases. |

Für die reine lokale Namenskorrektur wurde keine neue Implementierungs-Spiegel-
fixture angelegt. Sie führt denselben vorhandenen Klickpfad und dieselbe bereits
vorhandene Wiederholungsfunktion wieder zusammen. Kein Fallname wurde geändert,
keine Assertion entfernt und kein Erwartungswert zur Umgehung eines Befunds
abgeschwächt.

### Dateiübersicht, tatsächlicher Nachweis und offene Grenzen dieser Fortsetzung

Geändert und selbst gelesen wurden ausschließlich die folgenden Dateien:

| Datei | Zeilen | Bytes |
|---|---:|---:|
| `native/src/app/core/screen_tile_face.rs` | 308 | 10 768 |
| `native/src/app/core/screen_texts.rs` | 144 | 7 132 |
| `native/src/app/core/screen_texts_tests.rs` | 128 | 4 323 |
| `native/src/app/core/screen_indicator_ui.rs` | 288 | 9 963 |
| `native/src/screen/core/session_state.rs` | 216 | 7 610 |
| `native/src/screen/core/session_state_tests.rs` | 119 | 3 961 |
| `native/src/screen/core/batches_snapshot_task_tests.rs` | 281 | 10 825 |
| `docs/plaene/2026-10-24-batches/fix-regression-screenbudget.md` | Bericht | Bericht |

Es wurde keine neue Datei erstellt. Die zusätzliche gezielte Lektüre umfasst:

- `/tmp/smartexplorer-rv1-40354605251/screen-budget-errors.json` vollständig;
- `/tmp/smartexplorer-fix-screen-budget-scope.json` einschließlich seiner konkret
  von Main erteilten Erweiterungen;
- `native/src/app/core/screen_tile_grid.rs` und `screen_session.rs` für Probe,
  Preview, Wiederholungsretirement und gemeinsame Viewerbesitzer;
- `native/src/app/screen_modules.rs` für die unveränderten Registrierungsnamen;
- `native/src/screen/core/visibility.rs`, `host_visibility.rs` und `host_session.rs`
  für Hidden-Meldung, echte Capturequittung, Referenzboden und Wiederaufnahme;
- `native/src/screen/core/capture_test_support.rs`, `host_backend.rs`,
  `host_capture.rs` sowie die gezielt untersuchten kanonischen Zulassungs- und
  Pipelinegrenzen `capture_admission.rs`, `capture_grant.rs`, `host_pipeline.rs`;
- `native/review-task-manifest.json`, `docs/oberflaechen/bildschirm.md` und
  `docs/plaene/2026-10-24-batches/rv1-modulplan.md` gezielt zum betroffenen
  Suite-/Status-/Snapshotanschluss.

Die eigene anschließende AST-Prüfung der geänderten Rustquellen meldet keinen
Syntaxfehler. Normale Zeilen-/Bytegrößen bleiben unter 500 Zeilen und 50 KiB.
`git diff --check` und die Whitespaceprüfung melden keinen Befund. Die statische
Fallnamensprüfung bestätigt unveränderte Namen und Reihenfolge; die zusätzlichen
Quellvergleiche bestätigen die oben genannten unveränderten Retry-, Indikator-
und Snapshotanteile. Die Prüfung verwendet ausschließlich die bereits installierte
Tree-sitter-Grammatik und führt keinen Produkt- oder Fixturecode aus.

Compiler-/Typbestätigung, UI-Klicklauf, physischer Speicherpeak, Geräteabschluss
und praktische Hidden-/Resume-/Snapshotbestätigung bleiben nach diesen Änderungen
unbekannt. Sie gehören ausschließlich in Mains dieselbe Remote-Suite. Es fand
kein lokaler Build-, Compiler-, Formatter-, Test-, Server-, Installations-,
App-/Host- oder Geräteaufruf statt. Main behält Git, Manifest, Rootgraph und
Suite-/Releaseorchestrierung. Es bleibt keine bekannte ungeklärte Quellabhängigkeit
der hier zugewiesenen fünf Diagnose-IDs; die vorgemerkte Fallregistrierung und
die praktische Bestätigung sind noch offene Integrationsschritte von Main.

## Fortsetzung D17 nach Remote-Lauf 40368746815: vollständiger Android-Pausentext

Ausgang: `b233e5336a0ee177e19382a7adf592d297dcf202`. Die vollständige Diagnose in
`/tmp/smartexplorer-rv1-40368746815/screen-budget-errors.json` wurde vor der
Ursachenentscheidung gelesen. D17 belegt den fehlenden ausdrücklichen
ViewerHidden-Arm in der mobilen Vollständigkeitsfixture. Die eigene anschließende
Aufnahme findet denselben fehlenden Zustand in ihrer Keydatei, der produktiven
Kotlin-Zuordnung und der Android-Ressource. Die vorherige Desktop-/Zustandskorrektur
hat diese verbundene mobile Textkette noch nicht vollständig nachgezogen.

### Erstplan, Fehlerfunktionen und vollständige betroffene Kette

Der begrenzte Ergebnisauftrag verlangt Androids Hidden-Pausentext wortgleich zum
Desktop und ausdrückliche Vollständigkeit der vorhandenen Rust-/Kotlin-Kette.
Alle bisherigen Zustands-, Fehler-, End-, Block- und Detailtexte sowie ihre
Aussagen bleiben erhalten. Die Diagnoseempfehlung eines Wildcardarms wird nicht
übernommen; der vorhandene Vollständigkeitsvertrag muss weitere Auslassungen
weiter sichtbar machen. Geschrieben werden ausschließlich die vier freigegebenen
Quell-/Ressourcen-/Fixturedateien und dieser bestehende Bericht. Main besitzt
Bestätigung, Git, Manifest, Rootgraph und Release; `Cargo.toml` bleibt unangetastet.

| Fehlerfunktion oder Grenze | Direkte und indirekte Kette samt Erhalt |
|---|---|
| Mobile `screen_view_tests::all_keys` | Bestehende `SessionState`-Variante/`key()` → ausdrückliche mobile Zustandsliste und exhaustiver Match → Zustands-, Fehler-, End- und Blockschlüssel. Liste und Match lassen ViewerHidden aus; sämtliche bestehenden Varianten und Kategorien bleiben erhalten. |
| `screen_keys.txt` | `all_keys` → `screen_task_android_screen_keys_file_current` → dieselbe Datei als Kotlin-Testressource. Der verbindende Zustandsschlüssel `state:viewer_hidden` fehlt. Reihenfolge und vollständiger bisheriger Dateiinhalt bleiben erhalten. |
| `ScreenTexts.stateRes`/`stateText` | Bestehender `viewer_hidden`-Wirekey → mobiler `HostFrame::State`-Handler → `emit`/`event_value` → Android-Sitzungsstatus → `stateRes` → `Context.getString`. Der bekannte Zustand fällt bislang in die vorhandene Unknown-Behandlung. Eine explizite Ressourcenreferenz behebt das; die Vorwärtskompatibilität für tatsächlich unbekannte Schlüssel bleibt erhalten. |
| `strings.xml` | `screen_state_viewer_hidden` → generierte `R.string`-Referenz → beide Android-Statusverbraucher. Die Ressource fehlt; der Desktopliteral ist bereits eindeutig vorhanden. Es gibt laut Mains gezielter Ressourceninventur nur diese eine `main/res/values/strings.xml`, keine zusätzliche Sprachdatei. |
| Android-Vollansicht und Gruppenkachel | Die beiden von Main konkret benannten produktiven Aufrufer `ScreenViewScreen.kt` und `ScreenPreviewTile.kt` wurden an ihren Statuspfaden selbst gelesen. Sie verwenden denselben `ScreenTexts.stateText`-Helfer. Ihre bisherigen vorrangigen End-/Fehler-/Eingabe-/Bildzustände bleiben erhalten; Aufruf, Statuswahl, Steuerfreigabe und Präsentationsgrenzen werden nicht geändert. |
| Bestehende Vollständigkeitsprüfungen | Rust-Keydateigleichheit und Rust-Desktop-/XML-Wortgleichheit → Kotlin `ScreenTextsTest` mit exakter `R.string`-Zuordnung für jeden Schlüssel → bestehende Detail-/Unknown-Aussagen. Keine Aussage und kein Fallname wird entfernt oder ersetzt. |

Der native Sender benutzt weiterhin `report.state.key()` und übergibt den
vorhandenen Unterstrich-Schlüssel unverändert im `screenView`-Ereignis. Eine neue
Protokollvariante oder Umbenennung ist nicht erforderlich. Die vorhandenen
Android-Paused-/Resume-/Präsentationsregeln aus `BatchesScreenStateTest` werden
dabei nicht geändert.

### Recherche, Entscheidungen und konkreter Fixplan vor der Umsetzung

Die vorhandenen Prüfungen beschreiben den gewünschten Anschluss bereits genau:
`all_keys` und `screen_keys.txt` sind gleich; der Rust-Ressourcenfall vergleicht
den Literaltext mit Desktop, und Kotlin prüft den Ressourcenbezug jedes Keys.
Die [Android-Referenz für Stringressourcen](https://developer.android.com/guide/topics/resources/string-resource),
abgerufen am 2026-10-26, bestätigt den vorhandenen Weg über den XML-Namen,
`R.string` und `Context.getString`. Deshalb wird derselbe bestehende Ressourcenweg
ergänzt. Ein hart codierter Kotlintext oder das Weiterreichen des bekannten
Zustands an Unknown würde den bestehenden Vertrag nicht erfüllen.

Die zweite Lückenrecherche prüft ausdrücklich die Arrayliste zusätzlich zum
Matcharm, die Keydateireihenfolge, den tatsächlichen Desktopliteral, beide
Statusaufrufer und die bestehenden Ressourcen-/Detailprüfungen. Alle Grundlagen
liegen vor; zusätzliche Schreibabhängigkeiten sind nicht erforderlich.

| Fixpunkt | Konkrete Änderung | Erwarteter statischer und späterer Remote-Beleg |
|---|---|---|
| A1 Rust-Vollständigkeit | `ViewerHidden` in Zustandsliste und ausdrücklichem `all_keys`-Match ergänzen. | Alle dreizehn vorhandenen Zustände bleiben benannt; kein neuer Wildcardarm, unveränderte Fehler-/End-/Blocklisten und Prüfungen. |
| A2 Sprachgrenze | `state:viewer_hidden` an derselben Stelle der Keydatei ergänzen. | Vollständige Schlüsseldatei bleibt mit dem bestehenden Rusthelfer und dem Kotlin-Ressourcenfall verbunden. |
| A3 Kotlin | Expliziter `viewer_hidden`-Zweig auf `R.string.screen_state_viewer_hidden`. | Der bekannte Zustand erhält seine eigene Ressource; alle bisherigen Zuordnungen, Detailregeln und Unknown-Fälle bleiben unverändert. |
| A4 Androidressource | `screen_state_viewer_hidden` mit „Diese Ansicht ist nicht sichtbar; die Bildübertragung pausiert“ ergänzen. | Derselbe Literaltext wie Desktop; XML bleibt gültig, bisherige Ressourcen bleiben wortgleich. |
| A5 Eigene Nachreview | Geänderte Quellen und beide Verbraucher selbst gegen die gesamte obige Kette prüfen. | Statischer Erhaltungs-, Syntax-/AST-, XML- und Größenbeleg; dieselben bestehenden Fälle werden ausschließlich durch Main im selben Remote-Einstieg bestätigt. |

Dieser Plan wurde vor den darauf gestützten Quelländerungen durch denselben
Fixautor festgehalten. Die bestehenden Kotlin-Testdateien werden vollständig
erhalten und müssen für die vorhandene datengetriebene Abdeckung nicht geändert
werden. Main hat die aktuelle Dokument- und Graphgrundlage bereits bereitgestellt;
dieser Autor verändert weder fremde Berichte noch generierte Graphdateien.

### Umsetzung und eigene anschließende Anti-Regressionsreview zu D17

A1–A5 sind umgesetzt und durch denselben Autor anschließend selbst nachgeprüft.
Die Rustliste und ihr ausdrücklich exhaustiver Match nennen jetzt dieselben
dreizehn Varianten wie die aktuelle gemeinsame `SessionState`-Definition.
`state:viewer_hidden` steht in der Keydatei an derselben Position. Der Kotlinzweig
zeigt ausdrücklich auf `R.string.screen_state_viewer_hidden`; die entsprechende
XML-Ressource enthält wortgleich den aktuellen Desktopliteral „Diese Ansicht
ist nicht sichtbar; die Bildübertragung pausiert“.

Der additive Quellvergleich mit `b233e533` bestätigt für jede der vier Dateien:
Alle bisherigen Quellteile bleiben erhalten. Das schließt jede Rustfixture-
aussage, ihre Fallnamen, die Zustands-/Fehler-/End-/Blocklisten, sämtliche
vorhandenen Kotlinzuordnungen und Detail-/Unknown-Regeln sowie alle bisherigen
XML-Ressourcen ein. Es wurde kein Wildcardarm ergänzt und kein Platzhalter
eingesetzt. Die bereits bestehenden allgemeinen Antworten für tatsächlich
unbekannte Schlüssel bleiben unverändert.

Die beiden Androidverbraucher benutzen weiter ihren vorhandenen Statuspfad.
Vorrangige Beendigungen, Fehler, fehlende Eingabereferenz, Decoderfehler und die
Warteanzeige auf ein vollständiges Bild behalten ihre bisherigen Bedingungen.
Der neue bekannte Pausenschlüssel erreicht jetzt den bestehenden Ressourcenweg.
Keine Bild-/Codec-/Queueallokation, Visibilitygeneration, Präsentationsquittung,
Sitzungs-/Quellenidentität, Berechtigung, Retry- oder Retirementoperation wurde
verändert. Die bestehende Vollansicht und Gruppenkachel behalten ihre Bedienung.

Statisch ausgeführt wurden Quellen- und AST-Lektüre der Rust-/Kotlinänderung,
XML-Syntaxprüfung und Quellvergleiche. Der Ressourcenname ist einmalig vorhanden;
sein Literal stimmt mit der aktuellen Desktopquelle überein. Keine Syntax-,
Whitespace- oder Größenverletzung wurde festgestellt; `git diff --check` meldet
keinen Befund. Die Rustquelle hat in normaler Formatierung 393 Zeilen und
15 149 Bytes. Keine Fixture und kein Produktcode wurde dabei ausgeführt.

### Bestehende Fälle und Registrierung für denselben Remote-Einstieg

Main hat die folgenden beiden bereits vorhandenen nativen Fälle und die
unveränderte JVM-Klasse für die abschließende gemeinsame Manifestregistrierung
vorgemerkt. Dieser Autor verändert das Manifest nicht:

- `screen_task_android_screen_keys_file_current`: Rust-Keyliste und
  `screen_keys.txt` einschließlich `viewer_hidden` bleiben vollständig gleich.
- `screen_task_android_strings_match_desktop_sources`: Jede vorhandene Kategorie
  behält ihren Desktop-/Android-Wortlaut; auch Hidden erhält denselben Text und
  ausschließlich Streaming bleibt leer.
- `app.smartexplorer.android.ui.screen.ScreenTextsTest#screen_task_android_texts_match_desktop`:
  Jeder Schlüssel hat seine konkrete Kotlin-/`R.string`-/XML-Zuordnung;
  die vorhandene Aussage zu einem unbekannten Zukunftscode bleibt erhalten.
- `app.smartexplorer.android.ui.screen.ScreenTextsTest#a_detail_follows_only_where_the_desktop_shows_one`:
  Bestehende Detail-, Leertext- und Nullbehandlung bleiben erhalten. Die
  Klassenregistrierung nimmt diesen vorhandenen Begleitfall mit.

Die bestehende Klasse `BatchesScreenStateTest` wurde für ihre unabhängige
Präsentations-/Eingabereferenzgrenze gelesen und nicht geändert. Ihre beiden
bestehenden Fälle `batches_android_only_current_presented_identity_unlocks_input`
und `batches_android_stream_cache_does_not_alias_low_gop_bits` behalten ihre
Aussagen. Es gibt keine neue Fallquelle, keinen umbenannten Fall und keine
zusätzliche Suite oder Pipeline.

### Dateiübersicht, Entscheidungen und offene Grenzen zu D17

Geändert und selbst gelesen wurden ausschließlich:

| Datei | Zeilen | Bytes |
|---|---:|---:|
| `native/src/mobile/os/shared/domains/screen_view_tests.rs` | 393 | 15 149 |
| `android/app/src/test/resources/screen_keys.txt` | 42 | 848 |
| `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenTexts.kt` | 120 | 6 224 |
| `android/app/src/main/res/values/strings.xml` | 150 | 11 614 |
| `docs/plaene/2026-10-24-batches/fix-regression-screenbudget.md` | Bericht | Bericht |

Es wurde keine Datei erstellt. Zusätzlich gezielt gelesen wurden die aktuelle
Scope-Datei, die vollständige D17-Diagnose,
`native/src/screen/core/session_state.rs`,
`native/src/app/core/screen_texts.rs` und `screen_texts_tests.rs`,
`native/src/mobile/os/shared/domains/mod.rs`, `screen_view.rs`,
`screen_view_json.rs` und `screen_view_run.rs`,
`android/app/src/test/java/app/smartexplorer/android/ui/screen/ScreenTextsTest.kt`
und `BatchesScreenStateTest.kt`, die konkret freigegebenen Statusabschnitte von
`android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenViewScreen.kt`
und `ScreenPreviewTile.kt` sowie gezielt zum bestehenden Anschluss
`native/review-task-manifest.json`, `docs/oberflaechen/bildschirm.md` und
`docs/plaene/2026-10-24-batches/rv1-compilerplan.md`. Die Daten- und Ressourcen-
entscheidungen sind oben mit den lokalen Quellen und der Android-Primärreferenz
belegt; eine neue Bibliothek oder Sprachdatei war nicht erforderlich.

Keine lokale Kompilierung, Android-Ressourcengenerierung, JVM-/Rust-Testausführung,
App-/Geräteausführung, Installation, Formatierung, Gitmutation oder
Graphaktualisierung fand statt; es wurden keine Unteragenten gestartet.
Android-`R`-Generierung, Typ-/Compilerbestätigung, tatsächliche Statusdarstellung
und praktische Bestätigung der genannten Fälle sind nach dieser Änderung noch
unbekannt und gehören ausschließlich in Mains denselben Remote-Einstieg.
Es bleibt keine bekannte ungeklärte Quellabhängigkeit von D17. Die vorgemerkte
gemeinsame Fallregistrierung und die Remote-Bestätigung bleiben bei Main.

## K02/K10/K11: Capturestart und Budgetfunktionsbindung nach 40381452096

### Auftrag und Erstplan vor der Produktänderung

Der fortgesetzte Auftrag dieses selben Autors bezieht sich auf den vollständig
beendeten Remote-Lauf `40381452096`, Kandidat
`0425aa4608d7e27eb95ee0a37e3bc13dca2ce8bf`. Die vollständige Diagnose liegt in
`/tmp/smartexplorer-rv1-40381452096/kotlin-errors.json`. Zugewiesen sind K02,
K10 und K11 sowie die beiden konkret verbundenen Konstruktoraufrufe in
`BatchesScreenBudgetTest`. K01 und K03 bis K09 gehören dem Geräteautor.
Die aktuelle Scope-Datei ersetzt die vorherige Schreibfläche: Dieser Zyklus
ändert ausschließlich `ScreenHostControl.kt`, `ScreenSession.kt`,
`BatchesScreenBudgetTest.kt` und diesen bestehenden Bericht. Main besitzt
Manifest, Graph, Git, Remote-Suite und Release.

Der Erstplan aus dem Ergebnisauftrag ist: einen fehlgeschlagenen ersten
Displaystart und eine fehlgeschlagene Rekonfiguration in den vorhandenen
Capture-Fehlerkanal führen, bereits erworbene temporäre Besitzer tatsächlich
abschließen lassen und alle drei fehlerhaften Budgetaufrufe an die bestehende
Budgetfunktion binden. Zu erhalten sind Pause und Fortsetzung, dieselbe
Projektionsidentität, Wiederverwendung eines vorhandenen Displays, konkrete
Epochzuordnung, echte Ressourcenbesitzer, Annahme des Viewerbudgets erst nach
`Accepted`, Allocationübernahme bei Decoderwechsel und feste JNI-Puffer.
Vor der Bestandslektüre waren die genauen Fehleraufrufer, deren Abschlusswege
und die Kotlin-Konstruktorbindung zu klären; ein zusätzlicher Benutzerentscheid
war für dieses ausdrücklich beauftragte Ergebnis nicht erforderlich.

### Bestandsbefund und Fehlerfunktionsliste

1. `ScreenHostControl.attachSurface`: Der Rückgabetyp ist `VirtualDisplay`,
   `MediaProjection.createVirtualDisplay` liefert dagegen `VirtualDisplay?`.
   K02 belegt den Typfehler. Ein Nullresultat darf keine erfolgreich installierte
   Capturequelle darstellen. Der vorhandene Zweig `display.resize` und
   `display.surface = surface` gehört zur Wiederverwendung und bleibt erhalten.
2. `ScreenHostControl.applyConfig`: Tiles besitzt temporäre
   `ScreenProjectionFrames`; H.264 besitzt einen temporären `ScreenEncoder`
   und bei Hybrid zusätzlich temporäre Frames. Beide Attach-Fehlerpfade müssen
   diese Besitzer abschließen lassen. Im vorhandenen H.264-Catch verhindert eine
   Ausnahme von `newFrames.close()` den folgenden `encoder.release()` und
   verdeckt die ursprüngliche Attach-Ursache. Das ist ein direkt verbundener
   Abschlussfehler, den dieser Auftrag mitbehebt.
3. `ScreenHostControl.attachProjection`: Der erste direkte `applyConfig(host)`
   steht nach Registrierung des Projektionscallbacks ohne Fehlerbehandlung.
   `ScreenHostService.start` fängt diesen Aufruf nicht ab. Eine spätere
   Konfiguration besitzt bereits den `encoderFailed`-Meldeweg; der Erststart
   benötigt denselben Weg. Die vorhandene Metadatenmeldung
   `reportProjectionReady` ersetzt keinen tatsächlich installierten Encoder.
4. Verbundene Meldestelle: Der `configure`-Zweig von
   `ScreenHostControl.controlLoop` fängt den Fehler außerhalb des Sessionlocks.
   Er serialisiert seine eigenen Konfigurationsnachrichten bereits. Die neue
   Erststartbehandlung benötigt dagegen den Lock auch während der Meldung,
   weil ihr parallel der Kontrollthread eine neue Konfiguration setzen kann.
   Beide Meldestellen werden deshalb unter denselben Lock wie ihr Antrag
   gebunden; ein bereits nachgewiesener fremder Epochenwert im bisherigen
   `configure`-Catch wird hier nicht behauptet.
5. `ScreenSession.decoderBuffers`: K10 meldet den fehlenden Parameter `offered`,
   K11 die Zuordnung des nachgestellten Lambdas zum letzten Parameter
   `fixedBuffer: ByteBuffer?`. Die vorhandene Budgetfunktion muss weiterhin
   beim tatsächlichen `prepare()` den aktuellen Sessionzustand lesen.
6. `BatchesScreenBudgetTest.batches_android_pull_waits_for_acceptance_and_transfers_the_same_allocation`:
   Sowohl `previous` als auch `next` verwenden dasselbe fehlerhafte nachgestellte
   Lambda. Diese beiden Quellbefunde sind statisch belegt; der Remotebericht
   enthält für sie noch keine ausgeführte JVM-Diagnose.

### Direkte und indirekte Verknüpfungskarte dieses Zyklus

| Grenze | Tatsächliche Aufruf-/Besitzkette und erhaltener Vertrag |
|---|---|
| Erster Capturestart | Zustimmung → `ScreenHostService.start` → `ScreenHostControl.attachProjection` → an genau diese `MediaProjection` gebundene `ScreenProjectionIdentity` und registrierter Callback → `reportProjectionReady` für Quellmetadaten und `applyConfig` für den tatsächlich zugelassenen Producer. Ein schon beendeter Host, gesperrter Host, unbewiesener Paketbezug oder doppelter Projektionsgrant behält seine vorhandene Ablehnung. |
| Spätere Konfiguration | Native `screenControl`-Nachricht → `controlLoop`/`EncoderConfig` → Sessionlock → `applyConfig` → Abbau der tatsächlichen alten Frames/Encoder → `ScreenCaptureBudget.open` → neue Frames/Encoder → `attachSurface`. Eine ausstehende synthetische Retirement hält den Ersatz bis zu ihrem echten Callback zurück. |
| Display und Pause | `attachSurface` verwendet ein vorhandenes Display durch `resize` und `surface` erneut. `capture_active=false` trennt seine Surface, beendet Frames und Encoder beziehungsweise startet das vorhandene synthetische Retirement. Die 9-Byte-Quittung, `pendingConfigStart`, `pendingCapture` und der spätere Konfigurationsweg bleiben erhalten. |
| Tiles-Besitz | `ScreenCaptureBudget.open(PROJECTION_TILES)` → `ScreenProjectionFrames` → Native-Surface-Claim, `ImageReader`, tatsächlicher Handlerthread → temporäre Surface → Displayattach → erst danach `session.frames`. Bei Fehler muss `close()` den realen Thread beenden; sein `finally`/`finishResources` hält Reader und Claim bis zum tatsächlichen Ende. |
| H.264-/Hybridbesitz | Budget für `PROJECTION_SURFACE`, `PROJECTION_HYBRID` oder `SYNTHETIC_SURFACE` → `ScreenEncoder`/Konstruktionsbesitz/Drain → bei Hybrid zusätzliche `ScreenProjectionFrames`; bei Synthetic `retainSurface` und `SyntheticSurfaceSource` mit tatsächlichem Retirement. `ScreenEncoder.release` und `finishResources` warten auf Producer-/Drainende. Ein Attachfehler muss beide temporären Abschlussversuche erreichen, ohne native Claims vorzeitig zurückzugeben. |
| Epoch- und Eingabebindung | `captureCurrent` prüft Ende, Pause, konfigurierte Epoche, Sperrzustand und dieselbe Projektionsidentität. Encoder-Token und `encoderReadyToken` verhindern, dass eine neue Instanz die Freigabe ihrer Vorgängerin erbt. `reportProjectionReady` publiziert einen tatsächlichen Encoder nur bei unveränderter Identitäts-/Tokenbindung. Kein Input-, Quellen- oder Berechtigungsvertrag wird geändert. |
| Capturefehler | `encoderFailed` → höchstens 4096 UTF-8-Bytes, Epochflags und Flag `0x8` → `NativeBridge.screenPush` → JNI `screen_jni.rs` → `mobile::screen_push` → Android-Bridgekanal. Die gelesenen Reexports in `mobile/mod.rs` und `mobile/os/shared/domains/mod.rs` führen zur Definition in `domains/screen_host.rs`: Diese ruft direkt `channels::push` auf; `channels` ist der Alias von `crate::screen::os::bridge_channels`, dessen Androidregistrierung in `screen/os/mod.rs` gelesen wurde. `bridge_channels::push` lässt diesen begrenzten Lifecyclekanal mit aktuellem Capturegrant zu; `bridge_channel_state::push` verwirft fremde Generation/Epoche und setzt `encoder_failure`. `next_picture` liefert `encoder_failed: …`; `BridgeSource`/`bridge_source_capture` bilden dies auf `ScreenErrorCode::EncoderFailed` ab. |
| Viewerbudget | `ScreenSessionState.apply(Accepted)` → validiertes `ScreenBufferBudget` beziehungsweise bestehender Legacyvertrag → verzögertes `decoderBuffers.offered` → `ScreenPullBuffers.prepare` → passende direkte Little-Endian-Allocation. Ein fehlendes Budget lässt den 32-Byte-Anfangspuffer stehen; ein veränderter installierter Vertrag wird weiterhin abgewiesen. `ScreenScaleWorker` besitzt seinen eigenen gültigen Funktionsparameter. |
| Vollansicht und Decoderwechsel | `ScreenViewScreen` → `ScreenDecoder(..., pullBuffers = active.decoderBuffers)` → Ersatzdecoder wartet auf das Ende des Vorgängers → `ScreenPullBuffers.takeFrom` → dieselbe Allocation. Stop, Surfaceabbau, Sessionzustand, Stream-/Eingabereferenz und Snapshotpfad werden durch die Parameterkorrektur nicht verändert. |
| Fester JNI-Puffer | `ScreenCaptureFixture.pulls` übergibt Budgetfunktion und `Bridge.captureViewerPullBuffer(session)` bereits korrekt als zwei Argumente. `ScreenTaskTest.screen_task_android_viewer_decodes_barcode` verwendet diesen Puffer; erst nach Decoder- und Readerende folgen `relinquishFixedBuffer` und `releaseViewer`. Dieser zweite Konstruktorparameter bleibt vollständig erhalten. `ScreenDecoder` besitzt außerdem den gültigen einargumentigen Aufruf `ScreenPullBuffers(bufferBudget)`. |
| Vorhandene Budgetfixture | `BatchesScreenBudgetTest` → null vor `Accepted`, direkte Allocation im vereinbarten Limit, Payloadgrenzen, tatsächliche Übernahme und Ablehnung eines Vertragswechsels. Der zweite bestehende Budgetfall prüft weiterhin die finanzierten Imagebesitzer und Recordslots. Kein Fall und keine Aussage wird gestrichen. |

### Primärverträge, Entscheidungsplan und zweite Lückenprüfung

Am 2026-10-26 tatsächlich abgerufen wurden
[Android `MediaProjection.createVirtualDisplay`](https://developer.android.com/reference/android/media/projection/MediaProjection),
[Kotlin zu nachgestellten Lambdas](https://kotlinlang.org/docs/lambdas.html#passing-trailing-lambdas)
und [Kotlin zu benannten Argumenten](https://kotlinlang.org/docs/functions.html#named-arguments).
Die Androidreferenz erlaubt ein Nullresultat bei fehlgeschlagener
Displayerzeugung und verlangt bei den neueren Projektionsregeln den registrierten
Callback sowie Beachtung der Projektionslebensdauer. Deshalb bleibt die
vorhandene Displaywiederverwendung erhalten. Kotlin bindet ein nachgestelltes
Lambda an einen letzten Funktionsparameter; hier ist der letzte Parameter ein
optionaler `ByteBuffer`. Ein benanntes `offered = { … }` erhält beide Zugänge.

Gewählt wird `checkNotNull(createVirtualDisplay(...))` mit der konkreten Meldung
„Virtuelle Anzeige für die Bildschirmaufnahme konnte nicht erstellt werden“.
Ein weiterhin nichtnuller Rückgabevertrag erlaubt die vorhandene Installation
erst nach Erfolg. Eine nullable Rückgabe würde einen fehlgeschlagenen Start als
installierbaren Zustand weiterreichen; `!!` böte keinen verständlichen Grund.
Ein neues Display bei jeder Konfiguration würde die vorhandene Wiederverwendung
und Projektionslebensdauer ändern. Diese drei Alternativen werden verworfen.

Ein privater Abschlusshelfer versucht Framesabschluss und Encoderfreigabe
unabhängig. Er hängt Abschlussausnahmen an den ursprünglichen Fehler an und
wirft diesen erneut; die Besitzerklassen behalten ihre realen Claims bis zum
eigenen Abschluss. Eine manuelle Tokenfreigabe, ein Ersatzgrant oder bloßes
Verwerfen der temporären Objekte würde diesen Besitzvertrag nicht erfüllen.
Beim Fangen einer `InterruptedException` aus den tatsächlichen Threadjoins
muss der Helfer nach beiden Abschlussversuchen das Interruptflag wieder setzen;
die ursprüngliche Ursache und ein noch lebender Producer bleiben erhalten.
Die beiden Runtimefehlerfänger für Konfiguration und Erststart laufen jeweils
unter dem vorhandenen Sessionlock, damit Fehler und Epoche zusammengehören.
Die Metadatenmeldung bleibt erhalten: Ihre Maße werden vor dem tatsächlichen
Captureantrag benötigt; sie darf nicht auf einen noch nicht finanzierten
Encoder warten. Der vorhandene Tokenvergleich schützt seine spätere Freigabe.

Die zweite Lückenprüfung hat neben dem gemeldeten Produktionsaufruf die beiden
Budgetfixture-Aufrufe, den gültigen Decoder-Fallbackkonstruktor und den gültigen
zweiparametrigen JNI-Fixturezugang gelesen. Für Capture wurden beide
`attachSurface`-Aufrufer, alle drei `applyConfig`-Einstiege, tatsächliche
Frames-/Drainabschlüsse, Erststartservice, Callbackidentität, Pause, Synthetic-
Retirement und der native Fehlerempfänger verfolgt. Es entstehen keine neuen
Bedienhandlungen, Ansichten, Berechtigungen, Protokollfelder oder Abhängigkeiten.
Der Benutzer behält dieselben Start-, Pause-, Fortsetzungs- und Stopwege; der
fehlgeschlagene Start erhält eine konkrete Ursache im vorhandenen Fehlerweg.

### Konkreter Fixplan und vorgesehene eigene Nachreview

| Schritt | Änderung und Abhängigkeit | Vorher festgelegtes prüfbares Ergebnis |
|---|---|---|
| 1 | In `ScreenHostControl` Nullresultat von `createVirtualDisplay` zurückweisen; Wiederverwendungszweig erhalten. | Ein fehlgeschlagener neuer Displaystart erreicht den bestehenden Ausnahmeweg; nur ein tatsächlich vorhandenes Display wird installiert. |
| 2 | Temporäre Frames und Encoder über einen privaten Abschlusshelfer unabhängig schließen; beide Konfigurationsfehlerfänger innerhalb des jeweiligen Sessionlocks führen. | Der Erststart und die spätere Konfiguration melden denselben ursprünglichen Fehler für ihre Epoche. Eine Abschlussausnahme überspringt den zweiten Besitzer nicht und bewirkt keine vorzeitige Claimfreigabe. |
| 3 | In `ScreenSession` und den beiden vorhandenen Fixtureaufrufen die Lambdaausdrücke ausdrücklich an `offered` binden. | Die Budgetfunktion bleibt verzögert; Annahme, Bounds, echte Allocationübernahme, Vertragswechselablehnung und fester JNI-Puffer bleiben erhalten. |
| 4 | Eigene Nachreview der vollständigen obigen Kette nach Umsetzung, Vergleich aller bestehenden Fixtureaussagen und Fallnamen sowie statische Kotlin-Syntax-/Whitespaceprüfung. | Nur die geplanten Funktionskörper ändern sich. Die bestehenden Capture-, Budget-, Ownership- und Gerätefälle werden mit ihrem genauen Namen für denselben Remote-Einstieg gemeldet; keine lokale Laufzeit- oder Compilerbestätigung wird behauptet. |

Dieser Plan wurde vor der ersten Änderung der drei Kotlinquellen festgehalten.
Die spätere Typ-, JVM- und Gerätebestätigung gehört ausschließlich zu Mains
demselben Remote-Suiteeinstieg. Die vorhandenen Ownershipfälle prüfen die
verwendeten Besitzautomaten; sie sind keine bereits ausgeführte Injektion eines
`null` liefernden Android-Displayaufrufs.

### Umsetzung und eigene anschließende Anti-Regressionsreview

Die vier geplanten Schritte sind im eigenen zugewiesenen Umfang umgesetzt und
anschließend durch denselben Autor statisch geprüft. `attachSurface` weist ein
Nullresultat mit der geplanten konkreten Ursache zurück. `session.display`,
`session.frames` und `session.encoder` übernehmen weiterhin erst nach den
jeweiligen erfolgreichen Konstruktionen und dem Attach. Ein schon vorhandenes
Display wird unverändert mit `resize` und `surface` weiterverwendet.

Der neue private `closeFailedCapture` erhält den ursprünglichen Fehler,
versucht Framesabschluss und Encoderfreigabe unabhängig und hängt aufgetretene
Abschlussausnahmen als unterdrückte Ursachen an. Selbst eine fehlschlagende
Suppression verhindert den zweiten Abschlussversuch nicht. Der Autor hat beim
Nachreview die `join`-Unterbrechung als konkrete Folge des neuen Catchs geprüft
und das Wiederherstellen des Interruptflags nach beiden Versuchen ergänzt.
Ein fehlgeschlagener oder noch unvollständiger Abschluss wird dadurch nicht als
beendeter Producer oder freier Nativeclaim ausgegeben.

Die statische Nachreview hat folgende vollständige Fehler-/Erhaltsfälle der
geänderten Grenze gegen die gelesenen tatsächlichen Besitzer verfolgt:

- **Neues Tiles-Display liefert null:** `checkNotNull` wirft eine
  `IllegalStateException`; `applyConfig` fordert den tatsächlichen Abschluss der
  temporären Frames an und wirft dieselbe Ursache. Weder Display noch Frames
  werden in die Session übernommen. Erststart und Konfigurationsnachricht melden
  die Ursache anschließend unter ihrem Sessionlock über `encoderFailed`.
- **Neues H.264-Display liefert null:** Der temporäre Encoder wird über seinen
  bestehenden Release-/Drainweg abgeschlossen. Bei Hybrid wird davor zusätzlich
  der Framesabschluss versucht. Ein Framesfehler überspringt den Encoder nicht;
  beide temporären Besitzer bleiben bis zu ihrem jeweils realen Ende bilanziert.
- **Displayresize oder Surfacezuweisung schlägt fehl:** Derselbe neue
  Abschlusshelfer gilt auch für diesen bestehenden Ausnahmeweg. Das bisherige
  Display und seine Projektionsidentität werden nicht durch eine neue Projektion
  ersetzt; der vorhandene native Rekonfigurations-/Fallbackweg bleibt erreichbar.
- **Abschluss wirft oder läuft nach dem Joinlimit weiter:**
  `ScreenProjectionFrames.close` hält über Handler-`finally`, Reader und
  `readerLease` den noch lebenden Besitzer. `ScreenEncoder.release` lässt den
  tatsächlichen Drain mit `finishResources` und `ScreenSurfaceOwners.awaitEnded`
  seine Ressourcen freigeben. Der neue Helfer ruft weder JNI-Claimrelease noch
  Grantretirement direkt auf und behauptet bei `close()==false` kein Threadende.
  Eine `InterruptedException` wird als zusätzliche Ursache behalten und das
  Interruptflag vor dem erneuten Werfen wiederhergestellt.
- **Konfigurationswechsel während der Fehlerbehandlung:** Antrag und Meldung
  bleiben für Erststart und `configure` unter demselben vorhandenen Sessionlock.
  Ein konkurrierender Konfigurationsantrag kann die Epoche nicht zwischen dem
  Attachfehler im Erststart und dessen Meldung austauschen. Der vorhandene Retirementcallback
  arbeitet bereits unter diesem Lock und behält seine eigene Fehlerbehandlung.
- **Ready-Metadaten und echter Encoder:** Der vorlaufende Metadatenweg bleibt
  für Quellmaße und Capturefinanzierung erhalten. Er kann einen fehlenden Encoder
  nicht über `encoderReadyToken` freigeben, weil hierfür die tatsächliche gleiche
  Instanz erforderlich ist. Im gelesenen nativen Kanal löscht ein späteres
  `set_phase(Ready)` den gesetzten `encoder_failure` nicht; ein neuer
  `configure`-Auftrag bindet dagegen wie bisher eine neue Epoche.
- **Pause, Fortsetzung, Stop und konkrete Quelle:** Die bestehenden Körper von
  `retireEncoder`, `captureCurrent`, `reportProjectionReady` und `teardown`
  sind unverändert. Der `capture_active`-Zweig mit Surfaceablösung, tatsächlichem
  Framesabbau, Syntheticretirement, Quittung und späterem Ersatzantrag bleibt
  unverändert. Projektionscallback, Sperrprüfung, Paketnachweisgrenze,
  Encoder-Tokenbindung und Endbereinigung wurden nicht entfernt oder erweitert.
- **Accepted-Budget und Allocation:** In `ScreenSession` ändert sich exakt die
  Bindung des vorhandenen Lambdas an `offered`. Weder `Accepted`-Parsing noch
  Legacyzuordnung, Skalierungsbudget, Zustand, Sichtbarkeit, Modus, Input oder
  Backend ändern sich. Das Lambda liest weiterhin bei `prepare()` den aktuellen
  Zustand. In `BatchesScreenBudgetTest` ändern sich exakt die beiden Bindungen;
  sämtliche bisherigen Assertions und beide Fallnamen sind erhalten.
- **Decoderwechsel und fester JNI-Puffer:** Der Ersatzdecoder wartet weiterhin
  auf seinen Vorgänger, bevor er dessen echte Allocation übernimmt. Der gültige
  `ScreenPullBuffers(bufferBudget)`-Aufruf bleibt erhalten. Der zweiparametrige
  `ScreenCaptureFixture`-Aufruf behält seinen echten direkten Nativepuffer und
  dessen Freigabe erst nach tatsächlichem Decoder-/Readerende.

Ein Vergleich der Kotlin-Funktionsknoten gegen den unveränderten eigenen
Dateiausgang in Git belegt: Bei `ScreenHostControl` ändern sich ausschließlich
`controlLoop`, `applyConfig`, `attachSurface` und `attachProjection`; hinzu
kommen der private Abschlusshelfer und seine lokale Suppressionsfunktion.
Kein bestehender Funktionsknoten entfällt. Für `ScreenSession` sowie die
Budgetfixture wurde zusätzlich der gesamte Dateitext gegen ausschließlich die
drei geplanten `offered`-Substitutionen verglichen. Die Kotlin-Grammatikprüfung
mit vorhandenem Tree-sitter wurde ohne Compiler, Produktcode oder Fixturelauf
durchgeführt. Sie bestätigt Syntax, keine Kotlin-Typauflösung.
Nach der endgültigen Interruptkorrektur wurden diese statischen Vergleiche
erneut durchgeführt; es bestehen keine Kotlin-Syntax- oder Whitespacebefunde.
`git diff --check` ist ohne Befund. Die drei geänderten Kotlinquellen haben
676 Zeilen/32 842 Bytes, 471 Zeilen/23 299 Bytes und 54 Zeilen/2 547 Bytes in der
oben genannten Dateireihenfolge. In diesem Zyklus wurde keine Rustquelle
geändert oder erstellt.

### Vorhandene genaue Fälle für Mains denselben Remote-Einstieg

Die folgenden bereits existierenden Fälle bleiben für diese verbundenen
Verträge maßgeblich. Der Klassenbestand beziehungsweise die konkreten
Gerätefälle sind im gelesenen `native/review-task-manifest.json` bereits
ausgewählt. Main kontrolliert die Zuordnung zu K02/K10/K11; dieser Autor ändert
das Manifest nicht und erzeugt keine weitere Suite oder Fallquelle.

| Vollständige Klasse | Genaue bestehende Fallnamen und Beleggrenze |
|---|---|
| `app.smartexplorer.android.ui.screen.BatchesScreenBudgetTest` | `batches_android_budget_finances_twenty_image_owners_and_record_slots`; `batches_android_pull_waits_for_acceptance_and_transfers_the_same_allocation`: konkrete Budgets, spätes Accepted, Payloadbounds und dieselbe Allocation. |
| `app.smartexplorer.android.screen.ScreenSessionBudgetRegressionTest` | `batches_android_session_budget_small_legacy_receiver_keeps_its_scaler_bound`; `batches_android_session_budget_invalid_explicit_legacy_offer_has_no_512_mib_fallback`; `batches_android_session_budget_old_acceptance_without_budget_keeps_legacy_contract`: bestehende Session-/Legacyannahmen. |
| `app.smartexplorer.android.screen.ScreenCaptureBudgetTest` | `callbackAndWorkerRetainTheSamePromiseUntilBothEnd`; `aClosedHolderCannotCreateAReplacementOwner`; `failedTokenAdoptionReturnsTheRegisteredNativeTokenExactlyOnce`; `failedAliasFactoryLeavesOnlyTheTwoActualOwners`: konkrete Claims und Referenzenden. |
| `app.smartexplorer.android.screen.ScreenEncoderOwnershipTest` | `failedConstructionReturnsEveryAcquiredResourceInOrder`; `failedPlatformDestructionKeepsTheClaimAndTheOriginalFailure`; `enteredDrainKeepsItsResourcesUntilItsActualEnd`; `completedLeaseStillWaitsForTheClosingWorkerToReturn`; `interruptedDrainStillWaitsForTheProducerAndRestoresItsFlag`; `projectionRollbackEndsItsStartedWorkerBeforeTheReaderAndClaim`; `rejectedCodecCanBeReplacedWithoutReleasingTheSurfaceClaim`: die verwendeten produktiven Besitzautomaten mit Ersatzressourcen, keine Injektion in `attachProjection`. |
| `app.smartexplorer.android.task.ScreenTaskTest` | `screen_task_android_host_snapshot_nonblack`; `screen_task_android_viewer_decodes_barcode`; `screen_task_android_grid_switch`; `screen_task_android_host_bench_synthetic`: vorhandener Gerätecapture, echter JNI-Viewerpuffer, Wechsel und synthetischer Producer. Der Snapshotfall besitzt je nach verfügbarer Capturequelle einen Accessibility- oder Projektionsweg. |
| `app.smartexplorer.android.task.BatchesCaptureTaskTest` | `unknownSessionCannotObtainPixelsOrExternalResourceCredit`: unbekannte Session erhält weder Pixel noch externes Ressourcenbudget. |

Eine erfolgreiche Ausführung dieser vorhandenen Fälle ist kein Beleg dafür,
dass ein Android-Displayaufruf tatsächlich `null` zurückgegeben hat. Die genaue
Fehlerinjektion im ersten Projektionsstart, ein reales Displayresize-Versagen,
ein Handler-closetimeout und das Verhalten physischer OEMcodecs bleiben ohne
solchen Lauf praktisch unbestätigt. Ihre behandelten Quellpfade und tatsächlichen
Besitzer sind oben statisch belegt; es wird keine Gerätefreigabe behauptet.

### Dateiübersicht, Entscheidungen und offene Grenzen zu K02/K10/K11

Geändert und selbst gelesen wurden ausschließlich:

- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenHostControl.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenSession.kt`
- `android/app/src/test/java/app/smartexplorer/android/ui/screen/BatchesScreenBudgetTest.kt`
- `docs/plaene/2026-10-24-batches/fix-regression-screenbudget.md`

Es wurde keine Datei erstellt. Zusätzlich wurden in diesem Zyklus innerhalb
der jeweils ausdrücklich freigegebenen Lesefläche folgende Dateien gezielt
gelesen; jeder Pfad ist relativ zur isolierten Arbeitskopie angegeben:

- `AGENTS.md`
- `docs/ARCHITEKTUR.md`
- `docs/oberflaechen/bildschirm.md`
- `docs/plaene/2026-10-24-batches/rv1-compilerplan.md`
- `native/Cargo.toml`
- `native/Cargo.lock`
- `native/review-task-manifest.json`
- `android/app/src/main/java/app/smartexplorer/android/api/ScreenApi.kt`
- `android/app/src/main/java/app/smartexplorer/android/core/CoreEvents.kt`
- `android/app/src/main/java/app/smartexplorer/android/core/NativeBridge.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenCaptureBudget.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenCaptureBuffer.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenCaptureOwnership.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenEncoder.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenHostService.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenProjectionFrames.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenProjectionIdentity.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenSyntheticRetirement.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/SyntheticSurfaceSource.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenDecoder.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenPullBuffers.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenScaleWorker.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenViewScreen.kt`
- `android/app/src/test/java/app/smartexplorer/android/screen/ScreenCaptureBudgetTest.kt`
- `android/app/src/test/java/app/smartexplorer/android/screen/ScreenEncoderOwnershipTest.kt`
- `android/app/src/test/java/app/smartexplorer/android/screen/ScreenSessionBudgetRegressionTest.kt`
- `android/app/src/androidTest/java/app/smartexplorer/android/task/BatchesCaptureTaskTest.kt`
- `android/app/src/androidTest/java/app/smartexplorer/android/task/ScreenCaptureFixture.kt`
- `android/app/src/androidTest/java/app/smartexplorer/android/task/ScreenTaskTest.kt`
- `native/android-bridge/src/screen_jni.rs`
- `native/src/mobile/mod.rs`
- `native/src/mobile/os/shared/domains/mod.rs`
- `native/src/mobile/os/shared/domains/screen_host.rs`
- `native/src/screen/core/frame_budget.rs`
- `native/src/screen/os/mod.rs`
- `native/src/screen/os/android/bridge_channels.rs`
- `native/src/screen/os/android/bridge_channel_state.rs`
- `native/src/screen/os/android/bridge_source.rs`
- `native/src/screen/os/android/bridge_source_capture.rs`
- `native/src/screen/os/android/capture_admission.rs`
- `native/src/screen/os/android/viewer_sink.rs`

Außerhalb der Arbeitskopie wurden die aktuelle
`/tmp/smartexplorer-fix-screen-budget-scope.json`, die vollständige
`/tmp/smartexplorer-final-review-user-contract.txt`, die vollständige Diagnose
`/tmp/smartexplorer-rv1-40381452096/kotlin-errors.json` und die geltenden
`/root/.codex/AGENTS.md` gelesen. Die Android-/Kotlin-Primärquellen und ihre
Entscheidungsfolgen stehen im vorherigen Abschnitt. Vor der Berichtänderung
wurden der eigene Dateistatus, der jüngste Commit und die zugehörigen aktuellen
Code-/Themenstellen geprüft. Der jüngste gelesene Integrationscommit war
`00c97f88`; die fremden Iconänderungen wurden nicht bearbeitet.

Die Entscheidungen sind der nichtnullen Displayinstallation, den echten
Abschlussbesitzern, dem bestehenden Fehlerkanal, derselben Epoche und der
expliziten Budgetfunktion verpflichtet. Es verbleibt keine bekannte fehlende
Quellfreigabe dieses Zyklus. Noch erforderlich ist ausschließlich Mains
Bestätigung über denselben Remote-Suiteeinstieg und seine Integrations-/Release-
verantwortung. Keine lokale Kompilierung, JVM-/Geräte-/Rust-Testausführung,
Formatierung, Serverausführung, Installation, Gitmutation oder Graphänderung
wurde durchgeführt; es wurden keine Unteragenten gestartet. Der Main-
Reviewkandidat blieb durch diesen Autor unverändert.
