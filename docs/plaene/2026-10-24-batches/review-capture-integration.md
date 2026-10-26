# Capture-Integration: kritischer Batchreview

Stand: 2026-10-26. Gegenstand ist die zurückgegebene Capture-Core-/Worker-/Familien-/Linuxintegration im gemeinsamen Arbeitsbaum. Die Lesefläche und diese einzige Schreibfläche sind in `/tmp/smartexplorer-capture-integration-review-scope.json` festgelegt. Der beim Dokumentationsgate gelesene HEAD war `2fb02432`; die geprüften uncommitteten Dateien gehören ebenfalls zum Gegenstand. Die maßgeblichen Dateihashes stehen am Ende.

Ergebnis: **fünf statisch belegte Fehler**, unten als CAP-R1 bis CAP-R5 beschrieben. Es gab keine Build-, Compiler-, Formatter-, Test-, Server-, Installations-, Commit-, Push-, Release- oder Graphaktualisierung durch diesen Reviewagenten. Kein Laufzeitverhalten wird als praktisch reproduziert ausgegeben. Die P1-Einstufung bezeichnet einen vor der Auslieferung zu behebenden Funktions- beziehungsweise Linux-Buildfehler, keine gemessene Eintrittshäufigkeit.

## Auftrag, Planung und Beleggrenze

**A – Auftrag:** Der delegierte Nutzerauftrag verlangt einen kritischen Code- und Primärquellenreview, der echte funktionelle Fehler mit Trigger, Endnutzerfolge und Fundstelle liefert. Produktcode darf dieser Reviewagent nicht ändern. Die zusammenhängende Grenze umfasst gemeinsame Parent-W/X-Zulassung, Prepare/Activate, IPC-/RAW-/Recordbesitz, Quellenwechsel auf derselben Bank, Familienpläne, X11-Bild-/Cursor-/SHM-Besitz und PipeWire-Importpause bis zum Mapping-/Callbackende. Android-Fixtures und Glow werden getrennt geprüft.

**B – belegter Bestand:** `SessionAdmission`, `CaptureGrant` und die Bild-/Recordhalter bilden die gemeinsame Budgetgrenze; der Worker importiert Parent-Credits und meldet Retirements über dieselbe Ausgabe wie Bildantworten. Familienmitglieder verwenden eine delegierende `MemberAuthority`; X11 fordert für jeden Cursor einen zusätzlichen `CapturePlan` an. PipeWire hat einen asynchronen Formatcallback, separate Importtickets und eine getrennte Importsteuerung. Diese tatsächlich verbundenen Verträge begründen Planungstiefe 2. Die Architekturübersicht, die Capture-/PipeWire-Pläne, der vorhandene Graph und die im Dateiinventar genannten Quellen wurden herangezogen. Der Graphquery lieferte überwiegend breite Treffer; die Befunde beruhen auf den konkreten Quellpfaden.

**C – Bewertungsentscheidung:** Ein Fehler wird nur aufgenommen, wenn ein vollständiger Quellpfad bis zur sichtbaren Folge besteht und der benötigte Sprach-/Bibliotheksvertrag belegt ist. Bei Race-Bedingungen wird eine konkrete zulässige Reihenfolge angegeben. Fehlende Livebelege allein sind kein Fehler. Gegenprobe war jeweils, ob bestehende Grants, Scopegrenzen, Fehlerbehandlung oder Lifetimehalter den vermuteten Fehler bereits verhindern. Änderungen, zusätzliche Produktfunktionen und eine alternative Architektur sind nicht Ergebnis dieses Reviews.

**D – konkrete Reviewpunkte und Abschluss:** Quellenidentität und gemeinsame Bank; Prepare/Activate und Parentpartition; Bild-/Record-/Transportbesitz; Cursorzulassung in Einzelquelle und Familie; Worker-Nachrichten und asynchrones Retirement; PipeWire-Core-Import, Vollmapping, Callbackende und Abbau. Abschlussbeleg sind die vollständigen Fehlerketten unten sowie die ausdrücklich begrenzten Nichtbefunde und offenen Laufzeitbelege. Die zweite Quellenrunde klärte insbesondere x11rb-Paketallokation, Rust-Mutexverhalten, Modulnamensauflösung und die PipeWire-Mapping-/Callbackreihenfolge.

## CAP-R1 · P1 · Linux-SHM-Modul verwendet einen nicht importierten Typ

**Fundstelle:** `native/src/screen/os/linux_os/x11_capture_shm.rs:8`; zugehörige Importgrenze `native/src/screen/os/linux_os/x11_capture.rs:31` und `:37`.

**Trigger und Beleg:** Der Linux-Build bindet `capture_shm` als Untermodul ein. Dessen Feld `slice: Option<CaptureSlice>` verwendet `CaptureSlice`, aber das Modul importiert nur `super::*`. Das Elternmodul importiert aus `capture_admission` `CaptureGeneration`, `CaptureGrant`, `CaptureOwner`, `SessionAdmission` und `SourceKey`; `CaptureSlice` fehlt. Der explizite Import von `CaptureSlice` im Geschwistermodul `admission_stream` bringt den Namen nicht in den Scope von `capture_shm`.

**Endnutzerfolge:** Der geänderte Linux-Kandidat kann mit dieser Moduldefinition nicht kompiliert und deshalb nicht als funktionsfähiger Linux-Capture-Build ausgeliefert werden. Dies ist ein statischer Namensauflösungsbefund, kein behauptetes Compilerprotokoll.

**Vertragsabgleich:** Das Projekt verwendet Rust Edition 2021 (`native/Cargo.toml:4`). Die Rust-Sprachreferenz beschreibt die Modulgrenze für Itemnamen; Geschwisterimporte sind keine transitiven Importe. [Rust Reference, Item scopes](https://doc.rust-lang.org/reference/names/scopes.html#item-scopes), abgerufen 2026-10-26.

**Erforderliche Korrektur:** `CaptureSlice` im SHM-Modul oder in dessen explizit importierter Elternoberfläche einführen. Abschlusskriterium ist ein erfolgreich kompilierbarer Linux-Zweig im vorgesehenen entfernten Kandidatenlauf; hier wurde kein Compiler gestartet.

## CAP-R2 · P1 · X11-Cursorzulassung beendet die Parent-Berechtigung des aktiven Bildes

**Fundstellen:** `native/src/screen/os/linux_os/x11_admission_stream.rs:95`, `native/src/screen/os/shared/capture_broker.rs:67`, `native/src/screen/os/shared/capture_broker_imports.rs:77`, `native/src/screen/core/capture_transport.rs:58`.

**Trigger:** Eine isolierte X11-Monitor- oder Fensterquelle läuft mit erfolgreich aktivierter XFixes-Cursorabfrage. Schon ein normaler gültiger Cursor bei unveränderter Bildgeometrie reicht aus. Das Bild und der Cursor passen in das verfügbare Budget; eine Ressourcenknappheit ist für diesen Fehler nicht nötig.

**Belegte Kette:**

1. `x11_capture.rs:208` aktiviert den Cursor. Nach der Bildaufnahme ruft `capture()` bei `:433` `x11_cursor::fetch` mit dem Bildgrant und der Quellenautorität auf.
2. `x11_cursor.rs:45` armiert die Cursorantwort. Der Gate übernimmt Quelle und Generation des Bildgrants (`x11_admission_stream.rs:54`) und erstellt aus dem Antwortheader einen zusätzlichen Plan ausschließlich für Readback und Cursor-Metadaten (`:95–101`).
3. `RemoteAuthority::approve` sendet diesen Zusatzplan als gewöhnlichen `GrantRequest::FramePlan` (`capture_dispatch_requests.rs:69–71`). `CaptureBroker::resource` leitet ihn unverändert an `approve(plan, None)` weiter.
4. `CaptureBroker::approve` behandelt jeden solchen Plan als Ablösung des Bildplans: Es retired den bisherigen Parent-Bildgrant (`capture_broker.rs:67`) und setzt den Cursorcredit als `state.current` (`:76`). Der originale Worker-Bildgrant bleibt dagegen aktiv; das Bild trägt weiterhin dessen ID.
5. Beim Zerstören des Cursors nach dem Einblenden wird sein Credit freigegeben. `Retirement::drop` sendet `RETIRED` (`capture_dispatch.rs:133–143`); der Broker entfernt diesen Eintrag und setzt `current` auf `None` (`capture_broker.rs:112–116`). Das stellt den ursprünglichen Parentgrant nicht wieder her.
6. Beim nachfolgenden RAW- oder Recordempfang findet `read_owned` den ursprünglichen Parentgrant über die Bild-ID (`capture_transport.rs:52`), versucht damit aber einen neuen Transport-Claim (`:58`). `CaptureGrant::claim` lehnt den bereits retired Grant ab (`capture_grant_claims.rs:38–39`). `WorkerReader` beendet seine Leseschleife mit diesem Fehler (`host_worker_reader.rs:45`).

**Endnutzerfolge:** X11-Bildschirm-/Fensterfreigabe beziehungsweise Snapshot kann bereits am ersten regulären Bild mit Cursor scheitern. Die gewollte Cursorzugabe zerstört die Empfangsberechtigung für das eigentliche Bild.

**Vertragsabgleich:** `Cargo.lock:9241` und `:9256` pinnen `x11rb` und `x11rb-protocol` auf **0.13.2**. Deren gelesene Implementierung von `GetCursorImageReply::try_parse` liest `width × height` Cursorpixel aus der gültigen Antwort; `Cookie::reply` parst die Raw-Antwort und gibt den Raw-Puffer vor Rückkehr frei. Damit ist der separate Cursorbesitz sachlich begründet, seine Gleichsetzung mit einer Bildplanablösung dagegen eine eigene Projektentscheidung. Primärquellen: lokal gepinnte `x11rb-protocol-0.13.2/src/protocol/xfixes.rs:1138` und `x11rb-0.13.2/src/cookie.rs:247`, abgerufen aus dem vorhandenen Cratequelltext am 2026-10-26.

**Erforderliche Korrektur:** Cursorzusatzbesitz muss unter derselben Budgetbank zugelassen werden können, ohne den aktiven Parent-Bildgrant oder den aktuellen Bildcredit zu ersetzen. Abschlusskriterium: Eine Worker-Bildantwort nach Cursorzulassung behält einen empfangsberechtigten Parentgrant; Cursor- und Bildbytes bleiben während ihrer tatsächlichen Lebensdauer belastet.

## CAP-R3 · P1 · X11-Familienquelle startet wegen jeder Cursorantwort ihre Zulassung erneut

**Fundstellen:** `native/src/remote_window/os/shared/family_capture_members.rs:13`, `native/src/screen/os/linux_os/x11_admission_stream.rs:99`, `native/src/remote_window/os/shared/family_source.rs:126` und `:145`.

**Trigger:** Eine X11-Fensterfamilie enthält mindestens ein sichtbares, erfolgreich vorbereitetes Mitglied mit aktivierbarer XFixes-Erweiterung. Die Fenstergeometrie bleibt konstant; ein gültiger gewöhnlicher Cursor reicht aus.

**Belegte Kette:**

1. `family_capture_plan.rs:57–58` öffnet jedes Mitglied mit `member_admission`. Diese Admission verwendet `MemberAuthority` (`family_capture_members.rs:54–59`).
2. Die X11-Quelle aktiviert auch für Familienmitglieder den Cursor (`x11_capture.rs:208`) und fordert während jedes Capture einen Cursorplan an, wie in CAP-R2 beschrieben.
3. `MemberAuthority::approve` klassifiziert ausnahmslos jeden Plan als koordinierte Familienänderung: `changed = true`, danach `WouldBlock` (`family_capture_members.rs:13–16`). Es unterscheidet nicht zwischen neuem Bildbedarf und zusätzlichem Cursorbesitz.
4. Der X11-Gate speichert auch diesen `WouldBlock` dauerhaft als `failure` (`x11_admission_stream.rs:125–128`); folgende Reads liefern dadurch einen Fehler. `capture()` ignoriert zunächst die fehlgeschlagene Cursorabfrage, fragt anschließend jedoch die Geometrie über dieselbe Verbindung ab (`x11_capture.rs:433`, `:440`).
5. Die Familie gibt bei einem Mitgliedsfehler mit gesetztem `changed` nur `Paused(Starting)` zurück (`family_source.rs:145`). Selbst bei einer bereits vorhandenen Geometrieantwort verhindert die zusätzliche `changed`-Prüfung bei `:155–159` die Komposition.
6. Beim nächsten Capture erzwingt `changed` `readmit` (`:126–129`). Das öffnet neue Mitglieder und setzt das Flag zurück (`:59–74`); deren nächste Cursorantwort setzt es erneut. Keine Geometrieänderung ist erforderlich, um diese Schleife fortzusetzen.

**Endnutzerfolge:** Die X11-Fensterfamilie bleibt im Start-/Wartezustand und liefert kein zusammengesetztes Bild, obwohl ihre Fenster sichtbar, unverändert und erfolgreich vorbereitet sind. CAP-R2 beschreibt einen separaten Einzelquellenfehler; eine bloße Brokerkorrektur für CAP-R2 beseitigt die harte Ablehnung durch `MemberAuthority` nicht.

**Vertragsabgleich:** Der gültige XFixes-Antwortpfad ist durch dieselben gepinnten x11rb-0.13.2-Primärquellen wie CAP-R2 belegt. Die zusätzliche Fehlerpersistenz ist im Projektgate ausdrücklich implementiert; ein externer Serverfehler wird dafür nicht angenommen.

**Erforderliche Korrektur:** Der Familienvertrag muss den Cursorzusatzbesitz zulassen, ohne eine Bildgrößen-/Familienablösung zu signalisieren; die vorherige Belastung der tatsächlichen Raw-/ARGB-Allokationen muss erhalten bleiben. Abschlusskriterium: Ein unverändertes X11-Familienmitglied kann einen gültigen Cursor einblenden und ein Frame liefern, ohne `changed` und `readmit` auszulösen.

## CAP-R4 · P1 · Retirement und Bildversand können sich gegenseitig dauerhaft sperren

**Fundstellen:** `native/src/screen/os/shared/host_worker_run.rs:27`, `native/src/screen/core/capture_transport_wire.rs:26`, `native/src/screen/core/capture_grant_claims.rs:134`, `native/src/screen/os/shared/capture_dispatch.rs:57`.

**Trigger:** Ein laufender PipeWire-Stream meldet einen Formatwechsel, während der Worker eine Bildantwort mit Grant G serialisiert oder schreibt. Diese Callback-/Workerparallelität ist vorgesehen, nicht ein ungültiger Aufruf durch den Nutzer.

**Konkrete zulässige Reihenfolge:**

1. Worker A hält `output` in `host_worker_run.rs:27` und ruft `send_owned` auf. Darin entsteht der lokale `AdmittedBytes`-Transporthalter `out` für G (`capture_transport_wire.rs:26`).
2. PipeWire-Thread B führt `on_param_changed` aus (`pipewire_stream.rs:114`, `:144`). `CaptureMemory::format` retired G (`pipewire_capture_memory.rs:85`).
3. G setzt `usage.retired = true` und ruft synchron `notify_retired` auf (`capture_grant_claims.rs:104–106`). B hält nun Gs `notifications`-Mutex (`:134–137`), während der Observer auf `output` wartet (`capture_dispatch.rs:55–62`).
4. A beendet `write_message`. Vor der Rückkehr aus `send_owned` wird dessen lokaler Transporthalter zerstört. `CaptureSlice::drop` ruft `grant.notify_retired()` auf (`capture_grant_slices.rs:218–219`) und wartet auf Gs `notifications`-Mutex. A hält dabei weiterhin den äußeren `output`-Guard.
5. A wartet auf B und B wartet auf A. Weder Callback noch Worker können diesen Zyklus selbst auflösen.

**Endnutzerfolge:** Die laufende Freigabe friert ein; der Worker beantwortet weitere Anfragen nicht. Ein bestehender Parent-Timeout kann die Sitzung später beenden, löst aber den Sperrfehler im Worker nicht. Ein dauerhafter Stillstand des gesamten Daemons wird hier nicht behauptet.

**Vertragsabgleich:** PipeWire **0.3.65** beschreibt den eigenen Loopthread und die Ausführung der Callbacks unter dessen Lock; der Projektworker hält beim Bildversand diesen Looplock nicht. [PipeWire 0.3.65, thread-loop.h](https://raw.githubusercontent.com/PipeWire/pipewire/0.3.65/src/pipewire/thread-loop.h), Abschnitte Overview und Events and Callbacks. Rusts `Mutex` bleibt bis zum Guard-Drop gesperrt; rekursive Akquisition ist ebenfalls nicht als erfolgreiches Verhalten erlaubt. [Rust std, Mutex::lock](https://doc.rust-lang.org/std/sync/struct.Mutex.html#method.lock), Dokumentationsstand 1.99.1, abgerufen 2026-10-26. Der konkret installierte Compilerstand wurde nicht abgefragt und wird nicht behauptet.

**Erforderliche Korrektur:** Retirement-/Reduction-Meldungen dürfen nicht synchron aus dem Drop eines unter `output` gehaltenen Eigentümers erneut auf diese Ausgabe warten. Die Serialisierung der monotonen Reduction-Snapshots muss dabei erhalten bleiben. Abschlusskriterium: Formatwechsel und Ende eines Transporthalters können in der beschriebenen Reihenfolge vollständig abschließen; kein Bild-/Importcredit wird vor seinem tatsächlichen Besitzer freigegeben.

## CAP-R5 · P1 · Formatwechsel nach Bilderzeugung lässt den Worker beim Versand aussteigen

**Fundstellen:** `native/src/screen/os/shared/host_worker_run.rs:17–33`, `native/src/screen/os/shared/host_worker_produce.rs:35` und `:111`, `native/src/screen/os/linux_os/pipewire_capture_memory.rs:82–85`, `native/src/screen/core/capture_transport_wire.rs:26`.

**Trigger:** Der Worker hat bereits eine gültige kodierte Antwort für Generation g fertiggestellt. Bevor `send_owned` ihren Transporthalter beansprucht, meldet PipeWire einen Formatwechsel. Für diese Reihenfolge ist keine gleichzeitige Sperrhaltung aus CAP-R4 nötig.

**Belegte Kette:**

1. `produce` prüft die Generation nach `source.capture` (`host_worker_produce.rs:35–44`), führt danach aber noch Skalierung, Encoderauswahl und Kodierung aus und gibt schließlich Records zurück (`:111–117`). Nach dieser letzten Prüfung besteht kein Schutz gegen ein asynchrones Retirement.
2. `CaptureMemory::format` erhöht die Generation und retired den aktuellen Bildgrant G (`pipewire_capture_memory.rs:82–85`). Der schon erzeugte Reply besitzt weiterhin Records von G. Seine vorhandenen Bytes bleiben zulässig; neue Claims auf G sind dagegen ausdrücklich verboten.
3. `run` übernimmt nach `worker.handle` die nun aktuelle Quellenreferenzgeneration. Es setzt gegebenenfalls den Encoder zurück, behält aber den alten Reply und verpackt ihn mit dieser neu gelesenen Referenzgeneration (`host_worker_run.rs:17–24`).
4. `picture_grant` entnimmt G den Records (`capture_transport_wire.rs:7`). `send_owned` versucht einen neuen Transport-Claim auf G (`:26`). Dieser scheitert mit `retired capture grant cannot authorize new claims` (`capture_grant_claims.rs:38–39`).
5. `host_worker_run.rs:33` behandelt jeden Sendefehler terminal und beendet den Worker mit Status 1. Der vorgesehene vorübergehende Zustand beim Formatwechsel wird damit zum Sitzungsabbruch.

**Endnutzerfolge:** Eine Fenstergrößenänderung oder ein Monitorformatwechsel während einer bestehenden PipeWire-Freigabe kann die Freigabe abbrechen. Zusätzlich wird die fertige alte Antwort im Code mit einer neu gelesenen Referenzgeneration verknüpft; im beschriebenen Ablauf verhindert bereits der abgelehnte Transport-Claim ihre Übertragung. Eine tatsächlich ausgelieferte falsche Referenz wird deshalb hier nicht als separate beobachtete Folge behauptet.

**Vertragsabgleich:** Die Nebenläufigkeit folgt aus dem versionierten PipeWire-0.3.65-Loopvertrag aus CAP-R4. `PortalSource::reference_generation` gibt unmittelbar die asynchron geänderte Speicher-Generation zurück (`portal_session.rs:370`). Der Retirementvertrag ist im Projekt selbst eindeutig (`capture_grant_claims.rs:38–39`, `:89–106`).

**Erforderliche Korrektur:** Eine durch Generationenwechsel überholte Antwort muss vor dem Versand konsistent verworfen oder mit einem zuvor gesicherten, gültigen Transportbesitz abgeschlossen werden; sie darf weder als aktuelles Frame umetikettiert werden noch den Worker wegen erwartbaren Retirement beenden. Eine einzige zusätzliche ungeschützte Generationenabfrage beseitigt das Zeitfenster bis zum Claim nicht. Abschlusskriterium: Die gezeigte Reihenfolge führt zu einem wartenden/erneuerten Framezustand, einer korrekten Quellenreferenz und einem weiter antwortenden Worker.

## Geprüfte Grenzen ohne zusätzlichen belegten Fehler

- **Gemeinsame Bank und Quellenidentität:** `capture_session_context.rs` bindet neue Quellensitzungen an dieselbe `SessionInner`; `host_visibility.rs` verwendet `new_source_session` vor der Ablösung. In diesem Pfad wurde kein zweites unabhängiges W und keine Wiederverwendung der alten Quellenidentität nachgewiesen.
- **Prepare/Activate und Prozessende:** Der Broker trennt vorbereitete Quellenregistrierungen, Workercredit und Parentpartition. `process_exited` lässt Parent-Eigentum bis zum Ende des Readers bestehen; Readerende allein bestätigt keinen Workerprozessabgang. Der gelesene Pfad rechtfertigt keinen Befund einer vorzeitigen Freigabe allein durch EOF.
- **Bild-/Recordhalter:** `FrozenBytes` teilt dieselbe `Arc<AdmittedBytes>`-Allokation. Byte-/Recordcontainer halten ihre Slices über die tatsächliche Allokationslebensdauer. Die oben beschriebenen Retirement-Integrationsfehler bleiben davon unberührt; eine zusätzliche unberechtigte Kopie wurde in den gelesenen Pfaden nicht belegt.
- **Familienzahlen:** Der Familienplan trennt persistente Mitgliederressourcen und eine serialisierte Produzentenphase. Für die gelesenen Planfälle wurde kein zusätzlicher numerischer Unteransatz nachgewiesen. Der tatsächliche Cursorpfad ist dagegen durch CAP-R3 nicht funktionsfähig.
- **PipeWire-Importbesitz:** Der gelesene Pfad pausiert den Coreimport, fordert den tatsächlichen Vollmappingbedarf an, mappt vor Wiederaufnahme, invalidiert entfernte Handles im Callback und zerstört Freigabehandles nach dessen Rückkehr unter dem Looplock. Der Abbau zerstört Stream/Corebesitz vor der endgültigen Ticketfreigabe. Die upstream Implementierungen von `pw_memblock_map`, `pw_memmap_free` und `pw_memblock_free` wurden gegen **0.3.65** und **1.6.9** gelesen: Der `removed`-Callback läuft vor der finalen View-/Mappingzerstörung. Das begründet die bestehende Nach-Callback-Freigabe. [PipeWire 0.3.65 mem.c](https://raw.githubusercontent.com/PipeWire/pipewire/0.3.65/src/pipewire/mem.c), [PipeWire 1.6.9 mem.c](https://raw.githubusercontent.com/PipeWire/pipewire/1.6.9/src/pipewire/mem.c), geprüft 2026-10-26. Kein zusätzlicher vorzeitiger Mappingcredit-Drop wurde daraus belegt.
- **X11-Verbindungsaufbau:** Die Prüfung der gepinnten x11rb-0.13.2-Konstruktoren widerlegt die zunächst offene Vermutung eines doppelten Protokoll-Handshakes oder bereits verbrauchten Post-Setup-Sequenzzählers: `for_connected_stream` baut die äußere Verbindung aus vorhandenem Setup auf; der innere Aufbau sendet keinen zusätzlichen Post-Setup-Request. Dies wird nicht als Fehler berichtet.

Diese Aussagen sind begrenzte Ergebnisse der bezeichneten Quellpfade. Sie sind keine pauschale Kompatibilitäts- oder Laufzeitfreigabe.

## Entscheidungen und offene Belege

- Nur die Berichtdatei wurde erstellt. Die fünf Produktfehler sind an den Hauptagenten zur Integration gemeldet; dieser Review hat sie nicht behoben.
- CAP-R2 und CAP-R3 bleiben getrennt, weil Brokerablösung und Familienautorität jeweils einen eigenen Funktionsbruch verursachen. CAP-R4 und CAP-R5 bleiben getrennt, weil eine Korrektur der Ausgabesperren die fehlende Übergabe eines bei Retirement noch versendbaren Replies nicht automatisch korrigiert.
- x11rb ist durch `Cargo.lock` versionsgenau bekannt. Die im späteren Zielsystem geladene PipeWire-Version ist unbekannt; die Bibliothek wird dynamisch aufgelöst. Der Review hat die ausdrücklich benannten upstream Versionen geprüft, keinen konkreten Zielsystemlauf.
- Ein echter X11-Server mit XFixes/MIT-SHM, eine reale Fensterfamilie mit Compositor und ein realer Portal-/PipeWire-Stream wurden nicht gestartet. Damit fehlen praktische Belege zur Häufigkeit der Race-Bedingungen, zu konkreten Treiber-/Compositorvarianten und zur Darstellung der Endnutzerfehler. Diese fehlenden Livebelege ersetzen keine der nachgewiesenen Quellketten und sind nach Projektvorgabe keine eigenständige Releasevoraussetzung.
- Die vorhandenen Task-Testdateien wurden nur gelesen. Insbesondere beweisen die Metadaten-/Grant-Fixtures nicht den vollständigen Cursorpfad durch `MemberAuthority` und Parentbroker oder die beiden konkreten Worker-/Formatwechselreihenfolgen. Kein Test gilt aufgrund dieser Lektüre als bestanden.

## Gelesene Dateien

Die folgenden Listen nennen die tatsächlich vollständig oder abschnittsweise gelesenen Dateien; `rg`-Fundstellenlektüre wird separat bezeichnet. Die Lesefreigabe des Manifests ist größer als diese tatsächlich genutzte Fläche.

**Auftrag und Dokumente:**

```text
/tmp/smartexplorer-capture-integration-review-scope.json
/root/.codex/skills/arbeitsweise/SKILL.md
/root/.codex/skills/graphify/SKILL.md
docs/ARCHITEKTUR.md
docs/plaene/2026-10-24-batches/aufnahme-speicher.md
docs/plaene/2026-10-24-batches/pipewire-import.md
native/Cargo.toml
native/Cargo.lock
graphify-out/graph.json (über graphify query)
```

Die geltenden AGENTS-Anweisungen lagen zusätzlich im delegierten Gesprächskontext vor. `docs/plaene/2026-10-24-batches/review-b6.md` wurde über thematische `rg`-Fundstellen gelesen.

**Capture-Core:**

```text
native/src/screen/core/capture_admission.rs
native/src/screen/core/capture_authority.rs
native/src/screen/core/capture_session_context.rs
native/src/screen/core/capture_plan.rs
native/src/screen/core/capture_grant.rs
native/src/screen/core/capture_grant_claims.rs
native/src/screen/core/capture_grant_slices.rs
native/src/screen/core/capture_import.rs
native/src/screen/core/capture_transport.rs
native/src/screen/core/capture_transport_wire.rs
native/src/screen/core/host_codec_adapters.rs
native/src/screen/core/host_codec_phase_plan.rs
native/src/screen/core/host_admission.rs
native/src/screen/core/host_server_start.rs
native/src/screen/core/host_capture.rs
native/src/screen/core/host_pipeline_init.rs
native/src/screen/core/capture_prepared_lifetime.rs
native/src/screen/core/host_visibility.rs
native/src/screen/core/frame_memory.rs
native/src/screen/core/capture_source_contract.rs
native/src/screen/core/capture_records.rs
native/src/screen/core/capture_bytes.rs
native/src/screen/core/worker_capture_wire.rs
native/src/screen/core/batches_capture_pipeline_task_tests.rs
native/src/screen/core/batches_capture_worker_task_tests.rs
```

**Worker und Familie:**

```text
native/src/screen/os/shared/capture_broker.rs
native/src/screen/os/shared/capture_broker_imports.rs
native/src/screen/os/shared/capture_dispatch.rs
native/src/screen/os/shared/capture_dispatch_requests.rs
native/src/screen/os/shared/host_worker_main.rs
native/src/screen/os/shared/host_worker_prepare.rs
native/src/screen/os/shared/host_worker_prepared.rs
native/src/screen/os/shared/host_worker_run.rs
native/src/screen/os/shared/host_worker_produce.rs
native/src/screen/os/shared/host_worker_reader.rs
native/src/screen/os/shared/host_worker_source.rs
native/src/remote_window/os/shared/family_source.rs
native/src/remote_window/os/shared/family_capture_plan.rs
native/src/remote_window/os/shared/family_capture_members.rs
```

**Linux:**

```text
native/src/screen/os/linux_os/pipewire_import.rs
native/src/screen/os/linux_os/pipewire_import_control.rs
native/src/screen/os/linux_os/pipewire_import_events.rs
native/src/screen/os/linux_os/pipewire_memory_abi.rs
native/src/screen/os/linux_os/pipewire_buffers.rs
native/src/screen/os/linux_os/pipewire_capture_memory.rs
native/src/screen/os/linux_os/pipewire_stream.rs
native/src/screen/os/linux_os/pipewire_frame_copy.rs
native/src/screen/os/linux_os/pipewire_pause.rs
native/src/screen/os/linux_os/x11_capture.rs
native/src/screen/os/linux_os/x11_admission_stream.rs
native/src/screen/os/linux_os/x11_capture_shm.rs
native/src/screen/os/linux_os/x11_cursor.rs
native/src/screen/os/linux_os/x11_capture_memory.rs
native/src/screen/os/linux_os/capture_prepared.rs
native/src/screen/os/linux_os/batches_capture_x11_task_tests.rs
native/src/screen/os/linux_os/portal_session.rs
```

**Zusätzliche Fundstellenlektüre per `rg`:** `native/src/screen/core/host_pipeline.rs`, `host_pipeline_actor.rs`, `host_server.rs`, `host_family.rs`, `host_session.rs`, `host_mode.rs` und `family_listing.rs` jeweils in demselben Coreverzeichnis; außerdem `native/src/screen/os/linux_os/pipewire_abi.rs`. Die gezielte Suche nach `new_source_session` und `capture_context(` war ausschließlich auf die expliziten nativen Dateipfade des Lesemanifests begrenzt; trefferlose Dateien werden nicht als inhaltlich gelesen ausgegeben.

**Gepinnte lokale Bibliotheksprimärquellen:** Unter `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/` wurden `x11rb-0.13.2/src/cookie.rs`, `x11rb-0.13.2/src/rust_connection/mod.rs`, `x11rb-protocol-0.13.2/src/protocol/xfixes.rs` und `x11rb-protocol-0.13.2/src/packet_reader.rs` gelesen.

**Weitere gelesene externe Primärquellen:** [PipeWire 0.3.65 mem.h](https://raw.githubusercontent.com/PipeWire/pipewire/0.3.65/src/pipewire/mem.h), [PipeWire 0.3.65 core.c](https://raw.githubusercontent.com/PipeWire/pipewire/0.3.65/src/pipewire/core.c), [PipeWire 0.3.65 module-protocol-native.c](https://raw.githubusercontent.com/PipeWire/pipewire/0.3.65/src/modules/module-protocol-native.c), [PipeWire 0.3.65 thread-loop.c](https://raw.githubusercontent.com/PipeWire/pipewire/0.3.65/src/pipewire/thread-loop.c), [PipeWire 1.6.10 Thread Loop](https://docs.pipewire.org/page_thread_loop.html). Die für Befunde verwendeten weiteren Quellen stehen direkt beim jeweiligen Befund. Abrufdatum sämtlicher Webquellen: 2026-10-26.

## Erstellte Datei und Quellstand

Einzige erstellte/geänderte Datei dieses Agents: `docs/plaene/2026-10-24-batches/review-capture-integration.md`.

SHA-256 der zentralen Befunddateien, am 2026-10-26 vor dem Berichtschreiben gelesen:

```text
62192f2ec284c133cdd6108cf161c9b2a22f3de40ad7d79af46c279c8d73c99d native/src/screen/os/linux_os/x11_capture_shm.rs
1fb5c9ad9d292a113b515392603b4d159f33743e6b73a2c7d887dbdd74e67497 native/src/screen/os/linux_os/x11_capture.rs
9fa13c7ffff9f04130a9b096938a6de369fbe19850a6ef1eb53ba4f3a12003c3 native/src/screen/os/linux_os/x11_admission_stream.rs
f36327877d1119508def4f0ada8d3180b6b62cb18ecd676d6ab74ca8cb0a7af9 native/src/remote_window/os/shared/family_capture_members.rs
a1f4f395d8cf110755ac62e3b907b3cd34e47ddedf352b383c882dd225fb80ba native/src/remote_window/os/shared/family_source.rs
d3da9592ec48c20c49a0bbcab7dd00db4b03522f47999a1f2a3f9d3294b1c159 native/src/screen/os/shared/capture_broker.rs
25e511cfc298cd66828209b406797ea10506c07197befdf02cc7a8730cb74f25 native/src/screen/os/shared/host_worker_run.rs
9f6a7773a86a5d9a4831782107cf644403e692035cc93853f1472785dbc31ba5 native/src/screen/os/shared/capture_dispatch.rs
0b7f7f413cc52aa6a138aee4898dee3b9b6a8f7abeaeeb151a93c67989921214 native/src/screen/core/capture_grant_claims.rs
8bf7d12f8bdacf80418b43175bcddae7291b8ef80f514065d8910dab7e818f34 native/src/screen/core/capture_transport_wire.rs
630034980044b7062a1318a1b783021f3f95ef91bc3620dd24e15ff2df82d73a native/src/screen/os/linux_os/pipewire_capture_memory.rs
```
