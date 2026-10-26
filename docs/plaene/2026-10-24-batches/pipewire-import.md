# C7-Nachplan: tatsächliche PipeWire-Zulassungsgrenze

Stand: 2026-10-26. Nur Planung, keine Produktänderung und kein Laufzeitnachweis.

## Auftrag und Erstplan

Der delegierte Nachtrag korrigiert ausschließlich die C7-Annahme im übernommenen Aufnahmeplan. Der Linuxautor hat an PipeWire 0.3.65 belegt: Ein inaktiver Inputnode verhindert Linkvorbereitung; Aktivierung mit späterem update_params beweist dagegen keinen asynchronen Allokationshalt. Der vorhandene Wayland-/Portalpfad muss auch bei zunächst unkonfiguriertem Producer funktionsfähig bleiben.

Erstplan: Die freigegebenen aktuellen Adapter, Kernverträge und den bisherigen C7-Plan lesen; die tatsächliche öffentliche PipeWire-/SPA-Reihenfolge von Format, Bufferverhandlung, add/remove und Mapping an passenden Upstreamquellen nachvollziehen; einen vor Allokation wirkenden Anschluss oder eine belegte Alternative innerhalb der Produktbesitzgarantie wählen; API-/Dateiänderungen, Besitzerkette und spätere Abnahmesignale konkret festlegen; offene Planfragen erneut gegen Primärquellen prüfen.

Erhalten bleiben Portalzustimmung, ausgewählte Node-/Serial-/Sitzungsidentität, Inputbindung, Pause, Abbruch, Preview/Full, Snapshot, Familienkomposition und der unveränderte Host-W-Vertrag. Es werden keine Bufferanzahl, kein Pitch und keine Größe als Gerätefakt erfunden. Producer-/Serverinternas werden von eigenen angeforderten, allokierten und gemappten Besitzern unterschieden. Lese-/Schreibfläche ist ausschließlich das Manifest /tmp/smartexplorer-pipewire-admission-plan-scope.json; geschrieben wird nur diese Datei. Builds, Tests, Formatter, Installation, Server, Graphupdates, Commits, Pushes, Release und Unteragenten sind ausgeschlossen.

## Endentscheidung und ersetzter Vertrag

**C7 verwendet einen aktiven `pw_stream` ohne `PW_STREAM_FLAG_MAP_BUFFERS` und ohne `PW_STREAM_FLAG_ALLOC_BUFFERS`. Die öffentliche asynchrone Zulassungsgrenze liegt bei `pw_core_events.add_mem` und `pw_core_set_paused(true)`, vor der Zustellung der nächsten PipeWire-Nachricht. Jedes tatsächlich importierte SHM-Objekt wird aus derselben Parent-Session exakt zugelassen und anschließend genau einmal vollständig über deren Core-Mempool gemappt.** Enthaltene Buffer-, Metadaten- und Pixelansichten teilen diese Abbildung. Erst ein zusätzlich zugelassener Bild-/Hostplan erlaubt die CPU-Kopie.

Die Zusage umfasst die vom Produkt übernommenen lokalen Abbildungen und seine eigenen Bilder. Sie behauptet keinen Stopp fremder Producer-/Serverallokationen. Die aktive Verhandlung darf den Compositor zur Herstellung seines Quellpools veranlassen, bevor dessen Speicherobjekte lokal importiert werden. Diese Grenze entspricht dem bereits akzeptierten Ausschluss fremder Prozesse und nicht vollständig beobachtbarer Compositor-/Treiberinternas. Sie erweitert weder das gemeinsame Budget noch dessen Anspruch zu einer physischen RAM-Reservation.

Damit ersetzt dieser Nachtrag im Aufnahmeplan vollständig:

- den Ablauf unter „PipeWire: Verhandlung und Lebensdauer“, soweit dort ein verspätetes `update_params` als asynchrone Allokationssperre gilt;
- das C7-Abnahmesignal „Formatbedarf geht der Bufferverhandlung voraus; alle k angebotenen Buffer … sind gedeckt“;
- den zweiten offenen Implementierungsbeleg, der einen bereits bewiesenen vor der Bufferallokation liegenden Verhandlungspunkt behauptet;
- den PipeWire-Teil der allgemeinen Prepare-Aussage, der jede native Verhandlung vor dem Bildgrant ausschließt.

Der neue Satz für C7 lautet: **Vor jedem lokalen Pixel-/SHM-Mapping und vor jeder eigenen CPU-Bildallokation lebt eine passende, gemeinsam gezählte Zusage. Native Bufferanzahl, Größe und Pitch werden nicht aus dem Formatcallback oder einer Bibliothekskonstante behauptet. Serverallokation, FD-Import, lokales Mapping und CPU-Kopie sind getrennte Ereignisse.**

Der Zweitplan aus der Bestandslektüre behält die vorhandene Portalöffnung, den Hintergrundkontrollthread, `SessionAdmission`, `CaptureGrant`/`CaptureSlice`, `CapturedImage` und den Parentdispatcher bei. Er ersetzt ausschließlich die unerfüllbare Verhandlungssperre durch eine konkrete Empfangssperre und ergänzt die dafür nötige typisierte Vorbereitungszulassung. Die Entscheidungsfrage dieser Planung ist damit beantwortet; offen bleibt ihre Umsetzung und der benannte spätere Nachweis.

## Nachgelesene Reihenfolge und verworfene Wege

Alle folgenden Primärquellen wurden am **2026-10-26** gelesen. Der vollständige Verhaltensbeleg bezieht sich auf **PipeWire 0.3.65**; Core-Import, Pausedispatch und Mappingwiederverwendung wurden zusätzlich im Tag **1.6.9** gegengeprüft. Aktuelle API-Seiten wurden ergänzend gelesen: Core-Seite 1.6.10, Memory-Seite 1.6.9. Diese unterschiedlichen Seitenstände werden nicht als eine getestete Binärversion ausgegeben.

| Beleg | Konsequenz |
|---|---|
| `impl-link.c::pw_impl_link_prepare` kehrt bei inaktivem Input- oder Outputnode zurück. `do_negotiate` setzt die Formate und behandelt echte SPA-Async-Ergebnisse; diese Eigenschaft ist nicht die Signatur eines `pw_stream`-Eventcallbacks. [0.3.65 impl-link.c](https://raw.githubusercontent.com/PipeWire/pipewire/0.3.65/src/pipewire/impl-link.c) | `INACTIVE → auf Format warten` ist für einen unkonfigurierten Producer kein vollständiger Startpfad. |
| `stream.c::impl_port_set_param` emittiert `param_changed` und kehrt synchron zurück. `pw_stream_update_params` aktualisiert Parameter; ein späterer Aufruf stellt keine zurückgehaltene Formatquittung dar. Bei `MAP_BUFFERS` erfolgt das Datenmapping vor `add_buffer`. `clear_buffers` meldet die Entfernung vor seinem Unmapping. [0.3.65 stream.c](https://raw.githubusercontent.com/PipeWire/pipewire/0.3.65/src/pipewire/stream.c), [öffentlicher stream.h](https://raw.githubusercontent.com/PipeWire/pipewire/0.3.65/src/pipewire/stream.h) | Weder ein späterer Grant im Formatcallback noch die Prüfung erst in `add_buffer` schließt die Lücke. |
| `buffers.c::param_filter` kann Parameter der verbleibenden Seite verwenden. `pw_buffers_negotiate` besitzt bei fehlendem Buffers-POD einen eigenen Fallback. `alloc_buffers` legt den gemeinsamen Speicher aus den tatsächlich gewählten Parametern an. [0.3.65 buffers.c](https://raw.githubusercontent.com/PipeWire/pipewire/0.3.65/src/pipewire/buffers.c) | Aus fehlenden Consumerparametern folgt kein Allokationshalt; die dortigen Fallbackzahlen sind keine Produktzusage. |
| `remote-node.c::client_node_port_use_buffers` mappt den gemeinsamen Speicher für Chunk-/Metadaten und eingebettetes `MemPtr`, bevor es den lokalen Port aufruft. Externe `MemId`-Datendeskriptoren werden zu FD plus leerem Datenpointer. [0.3.65 remote-node.c](https://raw.githubusercontent.com/PipeWire/pipewire/0.3.65/src/modules/module-client-node/remote-node.c) | `MAP_BUFFERS` nur zu entfernen genügt nicht. Die Sperre muss schon vor dem nächsten Remote-Node-Ereignis wirken. |
| `core_event_add_mem` importiert den FD. `pw_mempool_import` erzeugt dabei keinen Mappingbereich. `pw_core_set_paused` ist öffentlich. [0.3.65 core.c](https://raw.githubusercontent.com/PipeWire/pipewire/0.3.65/src/pipewire/core.c), [core.h](https://raw.githubusercontent.com/PipeWire/pipewire/0.3.65/src/pipewire/core.h), [aktuelle Core-API](https://docs.pipewire.org/group__pw__core.html) | Ein eigener `add_mem`-Listener kann nach dem Import die Nachrichtenzustellung anhalten, bevor Pixel oder Chunk-/Metadaten dieser neuen Speicherkennung gemappt werden. |
| `module-protocol-native.c::process_remote` prüft `paused` zwischen Nachrichten. `impl_set_paused(false)` ruft unmittelbar wieder `process_remote` auf. [0.3.65 Protokollimplementierung](https://raw.githubusercontent.com/PipeWire/pipewire/0.3.65/src/modules/module-protocol-native.c), [Gegenprüfung 1.6.9](https://raw.githubusercontent.com/PipeWire/pipewire/1.6.9/src/modules/module-protocol-native.c) | Die Sperre gilt auch für schon im Socket empfangene Folgenachrichten. Resume ist reentrant; Zustandsmutex und IPC dürfen dabei nicht gehalten werden. |
| `mem.c::pw_memblock_map` nutzt eine bestehende, den Teilbereich enthaltende Abbildung; die Referenzen dieser Abbildung bestimmen ihr Ende. Bei erzwungenem Blockdestroy wird `removed` vor dem restlichen Aufräumen emittiert. [0.3.65 mem.c](https://raw.githubusercontent.com/PipeWire/pipewire/0.3.65/src/pipewire/mem.c), [1.6.9 mem.c](https://raw.githubusercontent.com/PipeWire/pipewire/1.6.9/src/pipewire/mem.c), [öffentliche mem.h](https://raw.githubusercontent.com/PipeWire/pipewire/0.3.65/src/pipewire/mem.h) | Eine vorher zugelassene vollständige Abbildung deckt spätere enthaltene Teilansichten ohne weitere Pixelabbildung. Ein Removalcallback allein ist noch keine Freigabequittung. |

Nicht gewählt werden:

1. **Dauerhaft inaktiver Probe-Stream.** Die Initialverhandlung kann ausbleiben. `Format`/`EnumFormat`/`Buffers`/`Meta` des Producerports bleiben nützliche Hinweise, aber keine Startvoraussetzung.
2. **Aktivierung und spätere Parameteraktualisierung als Sperre.** Der synchrone Callback und serverseitige Fallback widerlegen diese Annahme.
3. **Nur eigene MemFd-Mappings nach `add_buffer`.** Dieser Weg übersieht vorgelagertes `MemPtr` und Metadatenmapping und würde bei pauschalem MemPtr-Verbot einen bestehenden Pfad verlieren.
4. **Ein fester Bootstrap-Pixelbetrag oder `64 × Formatgröße`.** Die Bibliotheksgrenze 64 beschreibt ihre Bufferverwaltung, weder die tatsächliche Anzahl noch FD-Größe, Metadatenumfang oder Zeilenpitch.
5. **Neuer eigener SPA-Node als C7-Lösung.** SPA bietet echte asynchrone Methoden und `sync`; eine exportierte lokale Node benötigt jedoch weiterhin den Remote-Node-Transport. Der öffentliche Exportanschluss beseitigt dessen vorgelagerte Abbildungen nicht von selbst und verlangt zusätzlich Node-/IO-/Prozessimplementierung. Für die gewählte lokale Produktbesitzgarantie ist der kleinere Core-Empfangsanschluss ausreichend. [SPA node.h](https://raw.githubusercontent.com/PipeWire/pipewire/0.3.65/spa/include/spa/node/node.h), [impl-node.h](https://raw.githubusercontent.com/PipeWire/pipewire/0.3.65/src/pipewire/impl-node.h), `pw_core_spa_node_export` in der oben genannten `remote-node.c`.

## Genaue gezählte Fläche

Für jedes tatsächlich lokal übernommene SHM-Objekt `i` werden folgende Fakten festgestellt:

- die aktuelle PipeWire-Verbindung, ihre nicht wiederverwendete Verbindungsgeneration, `mem_id` und eine eigene monotone Importkennung;
- der von `add_mem` gelieferte FD und seine Berechtigungen, ohne den fremden FD eigenmächtig zu schließen;
- `L_i = fstat(fd).st_size`, geprüft auf positive Länge, Überlauf und Darstellbarkeit durch die verwendete öffentliche PipeWire-Mapping-API;
- die echte Seitengröße `P = sysconf(_SC_PAGESIZE)` und `M_i = ceil(L_i / P) × P` mit geprüfter Arithmetik.

Der Adapter mappt nach Grant `offset=0, length=L_i` über `pw_memblock_map`; gezählt werden `M_i` Bytes. Die Seitengröße ist keine hartcodierte 4096-Konstante. `st_size`, `mapoffset`, `maxsize`, Chunkgröße und Stride bleiben unterschiedliche Werte. Die neuen Syscalls gehören in den Linuxadapter. [Linux man-pages 6.19: fstat/stat](https://man7.org/linux/man-pages/man2/stat.2.html), [mmap](https://man7.org/linux/man-pages/man2/mmap.2.html), [sysconf](https://man7.org/linux/man-pages/man3/sysconf.3.html), [PipeWire Mapping-API](https://docs.pipewire.org/group__pw__memblock.html).

Dies ist eine bewusste **Ganzobjektabbildung**, keine bloß vorsichtige Reservation eines nie gemappten FD-Rests. Sie verhindert unbekannte Teilmapping-Spitzen und mehrfaches Pixelmapping desselben Core-Memblocks. Ein FD, der mehrere Buffer enthält, erhält einen Importbesitzer. Mehrere getrennte Memblocks erhalten getrennte Besitzer; gleiche Zahlenwerte eines wiederverwendeten FD oder einer wiederverwendeten `mem_id` sind kein Aliasbeweis. Die genaue Bufferanzahl ist für diese Rechnung nicht erforderlich. Die tatsächlich eintreffenden `add_buffer`-Deskriptoren werden weiterhin geprüft und zu diesen Besitzern zugeordnet.

Die gewählte Ganzobjektabbildung kann größer sein als die Summe der aktuell sichtbaren Pixelrechtecke. Diese Differenz wird vollständig gebucht und als Folge des gewählten öffentlichen Anschlusses ausgewiesen. Ein ungewöhnlich großes gepacktes FD kann deshalb eine konkrete Ressourcenablehnung verursachen. Es gibt keine pauschale Ablehnung von Legacy, 4K, MemPtr oder zunächst unkonfigurierten Producern. Eine kleinere Teilbereichsstrategie dürfte später nur mit einem belegten vorgelagerten Deskriptor-/Mappinganschluss die Ganzobjektstrategie ersetzen; nachträgliche Teilbereichsrechnung ist hier nicht zulässig.

| Besitzerklasse | Gezählte Kapazität und Ende |
|---|---|
| Lokale importierte SHM-Abbildungen einschließlich eingebetteter Pixel, Chunk- und Cursor-/Buffer-Metadaten sowie importierter IO-Bereiche | `Σ M_i` der tatsächlich gleichzeitig gemappten Importobjekte einschließlich Retiring-Objekten; Ende nach dem letzten Mapping und abgeschlossener lokaler Aufräumoperation. |
| Tight-BGRA-Quelle | Drei Plätze mit jeweils `N = width × height × 4`, vor jedem Builderclaim zugelassen. Ein laufender Builder, das neueste Cachebild und ein noch gehaltener voriger Bildbesitzer erklären diese Obergrenze. Sind die Plätze belegt, wird keine vierte Kopie erzeugt. |
| Ein Cachealias, keyframebedingte Rückgabe und Borrow desselben `CapturedImage` | Kein zusätzlicher Pixelbetrag; die bestehende gemeinsame Storage-/Slice-Lebensdauer bleibt maßgeblich. |
| Skalierung, Komposition, Encode, Workertransport, Parent-Snapshot | Der vorhandene `HostOutputPlan` und die vorhandenen Familien-/Parentpläne bleiben zuständig. Native Importbytes werden dort nicht erneut hinzugefügt. |
| Produktseitige dynamische Bild-/Regionsmetadaten | Tatsächlich angeforderte Kapazität in `CaptureOwner::Metadata`, vor ihrer Allokation. Eine Importtabelle darf nicht durch ungeprüftes `Vec`-Wachstum zum neuen großen Speicherpfad werden. |

Der Controller benötigt einen festen Pending-Deskriptor und seine kleine Kontrollverwaltung bereits zum Stellen des Bedarfs. Streamobjekte, Callbackregistrierung, Protokollköpfe und Bibliotheksverwaltungsobjekte gehören wie bisher zur nicht als vollständige RSS-Bilanz behaupteten Basisprozess-/Bibliotheksreserve. Diese Abgrenzung gilt ausdrücklich **nicht** für eingebettete Cursorpixel, variable native Bildbereiche oder selbst angelegte vollständige Bildkopien; deren Bytes stehen in der Tabelle.

Der Host hält weiterhin sein vollständiges `W`. Die gleichzeitig gültigen Importgrants, Bildgrants, Hostphasen und auslaufenden Besitzer werden derselben Sessionautorität zugeordnet. `X = max(0, tatsächlich benötigte gleichzeitig finanzierte Plansumme − W)` bleibt der zusätzliche Betrag. Ein vorhandener W-Anteil wird nicht ein zweites Mal reserviert. W/24, PacketBudget, Legacyrahmen und die vorhandene Plattform-/Kohortenkapazität werden nicht geändert. Alle tatsächlichen gleichzeitig gehaltenen Slices müssen sowohl ihren Ownergrenzen als auch der Gesamtgrenze genügen; disjunkte Phasenmaxima erlauben keine unerfasste reale Überlappung.

## Minimale plattformfreie Vertragsergänzung

Der heutige `CaptureAuthority::approve(CapturePlan)` verlangt eine gebundene Session. Das bleibt für Bilder und Hostausgabe richtig. PipeWire kann jedoch einen echten IO-/SHM-Import vor dem ersten festen Format liefern. Dies ist eine eigene Vorbereitungsklasse, deren Größe bereits feststeht; sie darf nicht von einer noch unbekannten Pixelgeometrie abhängig sein.

Der Kernverantwortliche ergänzt diesen kleinen internen Vertrag. Die Namen sind die verbindlichen vorgeschlagenen Anschlussnamen; alle Felder bleiben private, Konstruktoren prüfen die Invarianten:

```rust
// Reine Identität und Bytezahlen; keine FDs, OS-Typen oder PipeWire-Pointer.
struct CaptureImportId { connection: u64, object: u64 }
struct CaptureImportNeed {
    source: SourceKey,
    import: CaptureImportId,
    mapped_bytes: u64,
    metadata_bytes: u64,
}
struct CaptureImportGrant { /* eng gekapselter bestehender Grant/Lifetime-Anschluss */ }

trait CaptureAuthority: Send + Sync {
    fn approve(&self, plan: CapturePlan) -> io::Result<CaptureGrant>;
    fn approve_import(&self, need: CaptureImportNeed) -> io::Result<CaptureImportGrant>;
}

impl CaptureImportGrant {
    fn check_identity(&self, source: &SourceKey, import: &CaptureImportId) -> io::Result<()>;
    fn claim_mapping(&self) -> io::Result<CaptureSlice>;
    // Nur explizit im Bedarf vorhandene produktseitige Metadatenkapazität.
    fn claim_metadata(&self, bytes: u64) -> io::Result<CaptureSlice>;
}
```

Verbindliche Regeln dieses Vertrags:

1. `approve_import` ist bei `Unbound` und bei `Bound` zulässig, aber nur für die registrierte Prepare-Quelle derselben Session und ihre aktive Verbindungsgeneration. `connection` stammt aus dem vom Parent bestätigten Preparekontext; `object` ist dessen checked-incrementierte Importfolge. Beide Werte sind ungleich null und dürfen in diesem Kontext nicht wiederverwendet werden. Eine neue Importkennung darf nicht beliebig einen neuen Quellzugriff legitimieren. Familienmitglieder müssen bereits als Mitglieder dieser Vorbereitung registriert sein.
2. Ein Importgrant finanziert ausschließlich `NativeSurface` und ausdrücklich benannte Import-/Bildmetadaten. Er kann weder `CapturedImage` bauen noch einen `FrameContract` binden noch `PreparedSource::activate` ersetzen. Es gibt keinen allgemeinen grantlosen `begin`, keinen zusätzlichen MemoryPool und keinen Workerpool aus eigenen OS-Fakten.
3. Der gesamte Betrag wird atomar in derselben bereits gehaltenen W-/X-Zusage gebucht. Eine Teilzulassung darf keinen Mappingaufruf erlauben. Der kleine Vorbereitungsbedarf darf weder zur neuen Kohortenkapazität werden noch W verkleinern.
4. Der unveränderte Bildgrant bleibt an Pixel-/Referenzgeneration gebunden. Importidentität ist davon getrennt, weil ein FD mehrere Bilder oder mehrere Formatphasen überleben kann. Bei Bedarf wird intern ein separater, fest an die Prepare-Quelle gebundener Ledger-Schlüssel verwendet; dieser ist kein vom Worker frei auswählbarer Alias.
5. Der geprüfte Konstruktor von `CaptureImportNeed` verlangt positive Mappingbytes und eine überlauffreie Summe mit den gegebenenfalls null großen Metadatenbytes. Er bietet nur lesende Zugriffsmethoden für die fünf Felder. Ein kontrolliertes Handle zum eng gekapselten Importledger hält die Parentkredit-Lebensdauer wie bestehende Capturegrants. `CaptureImportGrant` gibt weder einen allgemeinen `CaptureGrant` noch ein `Deref` darauf heraus. `claim_mapping` darf den gebuchten Mappingbetrag nur einmal gleichzeitig ausgeben. Geteilte Ansichten klonen dessen wirklichen Besitzer, nicht einen neu zählenden Scheinclaim.
6. Die private Workeranforderung unterscheidet **FramePlan** und **PreparedImport** ausdrücklich. Der Importdescriptor enthält keine vom Peer übernommene FD-Adresse, sondern Source-/Connection-/Objektkennung und genaue Bytes. Der Parent validiert den bereits verifizierten Preparekontext, Request-/Credit-ID und Beträge. Doppelte, verspätete oder fremde Anforderungen geben keine zusätzliche frei verwendbare Zusage.
7. `GRANT_REQUEST`/`GRANT_REPLY`, Retirement und bestätigtes Workerende benutzen den bestehenden privaten Kanal. Der Dispatcher muss Importanfragen auch während PREPARE und ohne laufendes PRODUCE beantworten. Eine ausschließlich an normale Requestantworten gebundene Pipe-Lektüre würde hier einen Sperrkreis erzeugen und ist kein zulässiger Anschluss.

Der Kern erhält keine PipeWire-Erkennung. `SessionAdmission`, `CapturePlan` und der bereits geplante Parent-/Worker-Endbesitz bleiben die Budgetinstanz. Die Implementierung dieser Signatur liegt beim Kern-/Workerautor; der Linuxautor konsumiert sie anschließend.

## Linux-API und Ablauf

### Öffnung und Empfangssperre

`pipewire_abi.rs` bindet die öffentlichen Funktionen `pw_core_set_paused`, `pw_core_get_mempool`, `pw_mempool_find_id`, `pw_mempool_find_fd`, `pw_mempool_find_ptr`, `pw_memblock_map`, `pw_memmap_free` und `pw_mempool_add_listener`. Dazu kommen die versionierten öffentlichen Layouts `pw_core_events`, `pw_mempool_events`, `pw_memblock`, `pw_memmap`. Listenerdaten und Hooks bleiben adressstabil. Es werden keine privaten `pw_proxy`-, `pw_node_activation`-, `struct mapping`- oder `struct memblock`-Innenlayouts nachgebaut.

Der Importcontroller wird unter dem vorhandenen Threadloop-Lock **vor `pw_stream_connect` und vor dem Start der Eventzustellung** am ausschließlich für diese Portalquelle verwendeten Core registriert. Der Core besitzt keinen von einer unabhängigen Sitzung unbemerkt mitgenutzten Stream. Ein erfolgreicher Connect stellt den Listener nicht nachträglich her.

Der Ablauf für einen neuen Import ist:

1. `CoreEvents::add_mem` erhält `id/type/fd/flags`. Der interne Coreimport hat noch keine Pixel gemappt. Der Callback setzt `pw_core_set_paused(core, true)` und sperrt neue Produktkopien, speichert einen festen Pendingdescriptor, meldet Arbeit an den vorhandenen Kontrollthread und kehrt zurück. Ein Fehler der Pause ist terminal; er wird nicht durch Weiterlaufen umgangen. Der Mempool-`added`-Hook ist nicht der alleinige Eintrittspunkt, weil `DONT_NOTIFY` ihn unterdrücken kann.
2. Nach Rückkehr aus diesem Nachrichtendispatch werden unter dem Loop-Lock Memblockidentität und FD-Fakten gelesen. Der Controller hält weder Loop-Lock noch `CaptureMemory`-/Importtabellenmutex, während er die exakte gemeinsame Zusage anfordert. Die Corepause hält weitere Import-/Buffer-/IO-Nachrichten zurück. Bereits vorhandene alte Besitzer bleiben gültig und gezählt.
3. Der Parent bewilligt den vollständigen Importbedarf oder liefert einen konkreten Ressourcenfehler. Es entsteht keine indefinite Warteschlange auf eine irgendwann frei werdende Speichermenge. Der Pendingrequest bleibt durch Close, Parentende und Quellwiderruf abbrechbar.
4. Nach der Antwort wird der Loop erneut gesperrt. Session, Quelle, Verbindungsgeneration, Objektkennung, `mem_id`, FD-Identität, Länge und Berechtigungen werden wieder geprüft. Eine abweichende Länge ist ein neuer Bedarf, keine Erlaubnis zur größeren Abbildung. Ein veralteter Grant wird ohne Mapping an die normale Retirementkette gegeben.
5. Noch vor `pw_memblock_map` wird der echte `CaptureSlice` geclaimt. Die Abbildung verwendet Offset null, tatsächliche Objektlänge und die nötigen, vom FD erlaubten Read-/Write-Rechte. Weder `MAP_PRIVATE` noch ein zusätzlicher Stream-Mappingpfad wird angelegt. `PW_MEMMAP_FLAG_TWICE` wird nicht benutzt. Bei Mappingfehler bleibt die Quelle gestoppt; nach abgeschlossenem Fehlerrücklauf wird der unbenutzte Claim freigegeben.
6. Der Controller registriert den adressstabilen `ImportBacking` aus Mapping, Kapazität, Identität und Slice. Erst danach entfernt er die Importwartebedingung und ruft `pw_core_set_paused(false)`. Dabei hält er keine Mutex, die ein nachfolgender Core-/Streamcallback benötigt. Ein nächstes `add_mem` darf noch innerhalb dieses Resumeaufrufs wieder pausieren. Der Controller darf diesen neuen Pausestatus nach der Rückkehr nicht überschreiben.
7. Erst wenn die Importwartebedingung frei ist, eine gültige Format-/Bildgeneration aktiviert wurde, der Nutzer die Aufnahme wünscht und die Quellenidentität gültig ist, wird der CPU-Kopierpfad geöffnet. Der bloß aktive PipeWire-Stream ist kein Bildgrant.

Native FD-Objekte werden damit nicht durch geschätzte `k/B` ersetzt. Der Controller kennt zu jedem Zeitpunkt die tatsächlich zugesagten Importobjekte, deren Mappings und die nächste noch unzugelassene Nachricht.

### Aktive Formatverhandlung ohne Pendingregression

`make_stream` benutzt weiterhin `AUTOCONNECT`, die bestehende Zielbindung und `DONT_RECONNECT`. `MAP_BUFFERS` und `ALLOC_BUFFERS` bleiben aus. `RT_PROCESS` bleibt aus; damit bleibt der öffentliche Prozesscallback am Main-/Threadloop serialisiert. Die initiale Nativeaktivierung wird durch den gewünschten Pausestatus gesteuert und wartet nicht auf Producer-Portparameter. Der CPU-Kopierschalter beginnt geschlossen.

`param_changed` nimmt das tatsächlich feste Format entgegen, prüft die vorhandenen Capturelimits und erzeugt eine neue Bildgeneration. Die vier vorhandenen gepackten Formate BGRx, RGBx, BGRA und RGBA bleiben erhalten. Portalgröße in Compositoreinheiten wird weiterhin nicht als Pixelgröße verwendet.

Der Consumer darf **synchron** seine funktionalen Buffers-Anforderungen aktualisieren: ein gepackter Datenblock, die aus der feststehenden Geometrie berechnete Mindestnutzfläche `4 × width × height`, bestehende SHM-Datentypen MemFd/MemPtr und gegebenenfalls belegte Producerbedingungen. Dabei werden `buffers` und ein unbelegter exakter `stride` weggelassen; eine Größenrange lässt größere gültige Producerflächen zu. Die untere Nutzfläche ist eine vom Consumer angeforderte Mindestgröße, keine gemessene native Pitch-/Kapazitätsangabe. Die tatsächliche Abbildung bleibt erst am FD-Import zulässig. Dies verhindert, dass ein Producer ohne Buffers-POD allein den kleinen Bibliotheksfallback erhält. Die optionalen Eigenschaften sind in `pw_buffers_negotiate` explizit optional geparst; deren serverseitiger Bufferdefault wird nicht zum Produktbudget erklärt.

`ProducerProbe` darf Format-/Buffer-/Metadatenhinweise weiterhin liefern, wenn sie verfügbar sind. Fehlende Leserechte, noch nicht konfigurierte Parameter, ein EnumFormat-Bereich oder mehrere sichtbare Ports dürfen den gültig gebundenen Stream jedoch nicht durch diesen optionalen Probeweg dauerhaft sperren. Entscheidend bleiben dessen tatsächlicher Formatcallback und die bestehende Node-/Serialbindung. Ein Fehler oder eine Entfernung der tatsächlich gebundenen Quelle bleibt terminal; eine fehlgeschlagene optionale Parameterauskunft ist davon getrennt.

`PreparedSource::description` kann nach dem festen Format die echte Pixelgröße melden. `capture_plan(output)` enthält dann die drei CPU-Plätze und den passenden Hostplan, **ohne** `BufferSpec::native_capacity()` nochmals zu addieren. Imports sind bereits eigenständige lebende Sessionbesitzer. Ein fehlender Producer-Pitch oder eine fehlende Produceranzahl ist kein Grund für `None`, sobald die für den CPU-/Hostplan benötigte Geometrie feststeht. `activate` prüft weiter den Bildgrant; bis dahin darf der Prozesscallback Buffer zurückgeben, aber kein neues Produktbild kopieren.

### Übernahme tatsächlicher Buffer

`BufferOwners` wird zur Zuordnung tatsächlicher `pw_buffer`-Identitäten zu ihren `ImportBacking`-Besitzern und zur aktiven Bildgeneration. Es erzeugt keinen eigenen pauschalen Nativepool mehr.

- Bei MemFd wird der Core-Memblock über den tatsächlichen FD identifiziert. Pixel werden über die bereits zugelassene Abbildung und `mapoffset` gelesen; `data == NULL` ist ohne `MAP_BUFFERS` erwartbar und kein automatischer Frameverlust.
- Bei MemPtr wird der vorhandene Pointer seinem zugelassenen Core-Memblock zugeordnet und gegen dessen Bereich geprüft. Er erhält kein neues Mapping. Ein nicht zuordenbarer Pointer darf nicht als grantloser MemPtr-Fallback gelesen werden.
- Chunkpointer, alle verwendeten Metadatapointer und Datenbereiche werden an den richtigen Importbesitzer gebunden. Gültige zusätzliche Datenblöcke sind vollständig durch ihre Importobjekte gedeckt, selbst wenn die vorhandene gepackte Konvertierung nur den ersten Bildblock verwendet.
- `mapoffset + maxsize`, Chunkoffset, tatsächlich benötigte letzte Zeile und `stride × (height−1) + row_bytes` werden mit geprüfter Arithmetik validiert. Der tatsächliche Chunkstride ersetzt den bisherigen Gleichheitszwang zu einem geratenen Offerstride. Der bestehende Tight-Fallback für einen nichtpositiven Chunkstride bleibt an die vollständige Bereichsprüfung gebunden.
- Ein leerer oder als beschädigt markierter Chunk bleibt der vorhandene KWin-Fall ohne neues Bild. BGR/RGB-Tausch und opakes Alpha bleiben unverändert.
- Der Prozesscallback wartet nicht auf Parent-IPC. Fehlender Bildgrant, belegte drei CPU-Plätze, offene Importzulassung oder Nutzerpause führen zu Rückgabe ohne zusätzliche Pixelallokation. Jedes tatsächlich entnommene Bufferhandle wird weiterhin genau einmal zurückgestellt.

Die Grenze 64 darf als belegte Verwaltungsgrenze der benutzten PipeWire-Version geprüft werden, aber nie als tatsächliche Poolzahl oder Multiplikator einer Pixelzusage. Der Metadatenbedarf eigener Tabellen wird vor deren dynamischer Vergrößerung bestimmt. Der feste Pendingdescriptor darf keine Kopie eines importierten Bildes enthalten.

## Generationen, letzte Besitzer und Fehler

### Format- und Bufferwechsel

Ein neues Format schließt zuerst die CPU-Publikation und erhöht die vorhandene Bild-/Referenzgeneration. Alte `CapturedImage`-Besitzer bleiben an ihrer alten Zusage. Danach werden neue Importobjekte einzeln vor dem Mapping und der neue CPU-/Hostplan vor dem Builder zugelassen. Ein weiterverwendetes, ausreichend großes Importobjekt wird als derselbe tatsächliche Speicherbesitzer weiter gehalten; es wird nicht allein wegen eines neuen Bildformats doppelt reserviert.

Die zum öffentlichen Mempool gehörenden automatisch gemappten Chunk-/MemPtr-/IO-Flächen werden in den gelesenen Pfaden als feste Speicherobjekte exportiert; Bufferersatz gibt die bisherigen Exportreferenzen frei. Die App ändert diese FDs nicht und setzt keine neuen Seals auf fremde FDs. [0.3.65 client-node.c: `do_port_use_buffers`, `clear_buffers`](https://raw.githubusercontent.com/PipeWire/pipewire/0.3.65/src/modules/module-client-node/client-node.c), [0.3.65 impl-node.c: Aktivierungsobjekt](https://raw.githubusercontent.com/PipeWire/pipewire/0.3.65/src/pipewire/impl-node.c).

Für separat gelieferte MemFd-Pixelflächen wird auch bei späterer Benutzung kontrolliert, ob FD und tatsächlicher beanspruchter Bereich noch zu der zugelassenen Abbildung gehören. Eine echte größere Fläche benötigt vor einem neuen Mapping einen weiteren exakten Grant; ein gleichzeitig noch lebendes altes Mapping bleibt mitgezählt. Es wird weder eine immutable FD-Größe allein aus `fstat` behauptet noch ein fehlendes Seal pauschal zum Ausschluss aller bisher gültigen SHM-Quellen gemacht. Ein inkonsistenter, geschrumpfter oder nicht zuordenbarer Deskriptor führt zu einem konkreten Quellfehler vor der Produktkopie.

Der Nachweis beruht auf gültigen PipeWire-Speicherobjekten und dem gelesenen öffentlichen Bibliothekspfad. Er ist keine Sandbox gegen einen bösartigen lokalen PipeWire-Daemon, der Protokoll-/Bibliotheksinvarianten verletzt. Insbesondere ist ein nachträglicher `add_buffer`-Check kein Schutz vor einem beliebigen bereits von einer fehlerhaften nativen Bibliothek ausgeführten Mapping. Diese native Vertrauensgrenze wird nicht als zusätzliche selbst gezählte Produktfläche ausgegeben.

### Normales Retirement

Die folgenden Ereignisse sind ausdrücklich verschieden:

1. `queue_buffer` beendet einen Erwerb. Der Nativeimport bleibt bestehen.
2. `remove_buffer` entfernt eine Streamansicht. Die zugehörige FD-Abbildung kann weiterhin Chunk-/IO-Ansichten, laufende Callbackzugriffe oder weitere Buffer tragen. Sie verliert ihren Slice dadurch nicht.
3. `CoreEvents::remove_mem` beendet die Protokollkennung. Der Adapter markiert die entsprechende Importgeneration als Retiring und lässt sie nicht auf eine später wiederverwendete `mem_id` übergehen. Seine eigene Whole-Map-Referenz wird am sicheren Kontrollpunkt freigegeben; danach können noch Bibliotheksansichten die tatsächliche Abbildung halten.
4. `pw_mempool_events.removed` signalisiert das Blockende, kann aber noch innerhalb einer Destroy-/Unmapoperation laufen. Der Callback verschiebt den echten Slice daher in eine Deferred-Retirementliste; er gibt ihn nicht unmittelbar frei.
5. Der Kontrollthread verarbeitet diese Liste erst, nachdem der betreffende API-/Callbackstack vollständig zurückgekehrt ist und er den Threadloop exklusiv übernommen hat. Bei reentrantem Resume geschieht dies erst nach dessen Rückkehr. Bei Stream-/Core-Destroy geschieht es erst nach dessen Rückkehr. Nur dieser Punkt darf den Slice und anschließend die Parentkreditreferenz fallen lassen.

Das Halten der eigenen Whole-Map-Referenz darf `removed` nicht dauerhaft verhindern: Nach `remove_mem` wird diese eigene Referenz explizit am sicheren Punkt abgegeben. Ein danach noch lebender fremder Teilmappinghalter hält denselben budgetierten `ImportBacking` über dessen Retirementticket, bis das tatsächliche Blockende beobachtet und abgearbeitet wurde. Ein Callback mit `DONT_NOTIFY` benötigt den gleichwertigen expliziten Destroyabschluss; er darf nicht als bereits entfernt interpretiert werden.

### Close, Abbruch und Fehler

Der terminale Ablauf ist festgelegt:

1. Produktkopie und Publikation schließen; die vorhandene `StreamIdentity` terminal invalidieren; neue Grantanfragen verbieten. Native Streamaktivität wird angehalten, soweit der Handle noch gültig ist.
2. Pending-Import-/Bildanfragen über den bestehenden Parentdispatcher abbrechen. Eine verspätete Bewilligung wird zurückgegeben und nicht mehr gemappt. Parent-IPC und Controlthread-Join erfolgen ohne gehaltenen Threadloop-Lock.
3. Den Kontrollthread beenden und seine tatsächliche Beendigung abwarten. Ein gesendetes Stopsignal ist kein Endbesitzbeleg. Der Dispatcher muss wartende Grants beim Close/EOF aufwecken, damit der Join keine Antwort abwartet, die derselbe Closepfad verhindert.
4. Unter exklusivem Threadloop-Lock Probe und Stream deaktivieren/abtrennen, `pw_stream_disconnect` und `pw_stream_destroy` abschließen. Die Import-/Mempoollistener und ihre Daten leben noch.
5. Eigene Whole-Map-Handles nach Ende aller Streamzugriffe freigeben; Core disconnect/destroy abschließen. Ein Core kann auch Aktivierungs-/IO-Mappings halten, daher genügt Streamdestroy allein nicht. Die Buchung lebt bis zum Ende des Core-Aufräumens. Listener werden nicht vor den Ereignissen entfernt, die für ihre Retirementtickets nötig sind.
6. Nach bestätigtem Core-/Mappingende Deferred-Retirement abarbeiten, Kontext und Threadloop vollständig beenden. Erst dann dürfen Callbackdaten und native Importtickets endgültig fallen. Jeder Handle wird genau einmal freigegeben; ein bereits vom Core zerstörter Mappingpointer wird nicht erneut dereferenziert.
7. Ein bereits ausgeliefertes CPU-`CapturedImage`, Parent-RAW-Bild oder Snapshot besitzt seinen eigenen Slice weiter. Dessen Ende ist sein letzter tatsächlicher Besitzer; es hängt nicht am Portal-/Core-Destroy. Die unveränderte Parent-/Workerregel gibt unquittierte Kindbytes erst nach bestätigtem Prozessende frei.

Bei Zulassungsablehnung, `set_paused`-/Mappingfehler, Parentverlust oder Quellenwiderruf bleibt die Produktkopie geschlossen. Es gibt keinen Fehlerpfad, der zur Wiederaufnahme `MAP_BUFFERS` setzt, die Corepause ohne Grant löst oder einen grantlosen Bildbuilder benutzt. Der Fehler wird als konkreter Ressourcen-/Capture-/Quellverlust gemeldet und die bestehende Closekette ausgelöst. Die Portalzustimmung wird nicht als Speicherablehnung umetikettiert.

### Nutzerpause

Es gibt drei getrennte Zustände: gewünschte Aufnahmeaktivität, native Streamaktivität und wegen Importzulassung gesperrte Corezustellung. `set_capture_active(false)` schließt sofort den Produktkopierschalter, wartet den laufenden eigenen Callbackzugriff am Loopübergang ab und fordert native Deaktivierung an. Bereits gehaltene Mappings und Bilder bleiben gebucht.

Ein bereits laufender genauer Importbedarf darf während der Pause abschließen, damit danach Deaktivierungs- und Lebensdauerereignisse verarbeitet werden können. Der anschließende Core-Resume setzt die gewünschte Aufnahme nicht auf aktiv zurück. Die bestehende native PAUSED-Bestätigung wird nur nach dem tatsächlichen Statecallback gemeldet; eine wegen Importpause noch ausstehende Bestätigung wird nicht vorgetäuscht. Nutzerpause darf weder Quellidentität invalidieren noch auf einen anderen Port wechseln. Close bleibt unabhängig von einer solchen Bestätigung ausführbar. Release-Eingaben und Portal-Close benutzen weiter ihren vorhandenen Kanal.

## Konkrete Änderungen und Abnahmesignale

Die folgende Reihenfolge beschreibt umsetzbare, zusammenhängende Ergebnisse. Sie ist keine Erlaubnis, während dieses Planauftrags Produktdateien zu ändern oder Prüfungen auszuführen. Neue Rustdateien bleiben nach Projektregel unter 500 Zeilen und 50 KiB; bestehende Sammeldateien erhalten keine weitere große Zustandsmaschine.

| Schritt | Dateien und konkrete Änderung | Abhängigkeit und späteres Abnahmesignal |
|---|---|---|
| C7.1 Typisierte Importzusage | `core/capture_admission.rs`, vorgeschlagenes neues `core/capture_import.rs`, Anschluss an `core/capture_plan.rs`; nötiger privater Modulanschluss in `core/mod.rs` durch den Kernautor. `CaptureImportNeed`/`CaptureImportGrant` und `approve_import` verwenden denselben W-/X-Ledger. `capture_source_contract.rs` dokumentiert zugelassene Importvorbereitung vor Bildaktivierung. | Kern-/Workervertrag. Ein echter Unbound-Import wird exakt finanziert, ein Bild-/Codecclaim aus demselben Ticket abgewiesen; Bilder benötigen weiterhin gebundenen Vertrag und Generation. |
| C7.2 Private Parent-/Workerübergabe | `core/worker_capture_wire.rs`, `core/worker_wire.rs`; die vorgesehenen `core/host_admission.rs`, `core/capture_transport.rs`, `os/shared/capture_broker.rs`, `capture_dispatch.rs`, `host_worker_prepare.rs`; Anschluss in `host_worker_reader.rs`, `host_worker_main.rs`, `core/host_server.rs`. Importanfrage mit Source-/Connection-/Objektkennung, exakten Bytes und normalem Kreditretirement. | C7.1 und bestehender C4-Dispatcher. Import vor Format/Accepted, während PREPARE und im ruhenden Stream bekommt eine Antwort ohne PRODUCE; Close beendet einen wartenden Request ohne vorzeitige Parentfreigabe. |
| C7.3 Öffentliche Core-Sperre und Abbildung | `pipewire_abi.rs`; neues `os/linux_os/pipewire_import.rs` für Hook-/Mappingbesitz und gegebenenfalls neues `pipewire_import_control.rs` für den abbrechbaren Controller. Öffentliche Core-/Memory-ABI, `fstat`/Seitengröße, geprüfte Whole-Map-Kapazität, reentrantes Resume und Deferred-Retirement. Einbindung als private Submodule von `pipewire_stream.rs`. | C7.1–C7.2. Ereignisfolge für jeden tatsächlichen Import lautet `add_mem → core_pause → exact_grant → claim → map → resume`; abgelehntes Ticket führt zu null Mappingbytes dieses Objekts. |
| C7.4 Verhandlung und native Ansichten | `pipewire_stream.rs`, `pipewire_buffers.rs`, `pipewire_capture_memory.rs`, `spa_pod.rs`. Aktiver Nicht-MAP-/Nicht-ALLOC-Stream, funktionale Mindestnutzfläche ohne erfundene Anzahl/Pitch, optionaler Probe, konkrete FD-/Pointerzuordnung, CPU-/Hostplan ohne doppelte Nativefläche. | C7.3. Producer ohne anfängliches festes Format und ohne initiales Buffers-POD liefert nach aktiver Verhandlung ein zugelassenes Bild. MemFd und MemPtr bleiben möglich; gepaddete Zeilen und gemeinsame FD-Pools werden korrekt gebucht. |
| C7.5 Lebensdauer, beide Portalvarianten und bestehende Verträge | `pipewire_pause.rs`, `capture_prepared.rs`, `portal_session.rs`, `portal_input.rs`; Identitätsanschlüsse in `pipewire_identity.rs` und `portal_source_binding.rs` semantisch erhalten. Close-/Join-/Core-Retirement und getrennte Aktivitätszustände anbinden. | C7.2–C7.4. Pause oder Fehler öffnet keinen grantlosen Pfad. Ein nach Close gehaltenes Bild behält seine Zusage; alte und neue FD-Generationen sowie abgebrochene Requests bleiben bis zum tatsächlichen Ende gezählt. |
| C7.6 Kanonischer Ersatz und gemeinsame Abnahme | Hauptagent ersetzt die oben genau benannten C7-Aussagen in `docs/plaene/2026-10-24-batches/aufnahme-speicher.md` und den C7-Status in `umsetzung.md`; Linux-/Kernfälle werden in die bereits geplante eine Remote-Task-Suite aufgenommen. | Alle C7-Schritte integriert. Die Dokumentation behauptet weder eine fremde Allokationssperre noch einen Laufzeitnachweis aus diesem Plan. Keine zusätzliche lokale Suite, kein eigener C7-Release. |

`core/host_admission.rs`, `core/capture_transport.rs`, `os/shared/capture_broker.rs`, `capture_dispatch.rs` und `host_worker_prepare.rs` waren bei den konkreten Leseversuchen noch nicht vorhanden. Main hat diese genauen Zieldateien freigegeben und als parallel entstehende Integration benannt. Ihre finalen Signaturen sind daher noch durch den Kernautor zusammenzuführen; dieser Plan behauptet dort keinen bereits existierenden produktiven Dispatcher. Es ist keine weitere Dateisuche erforderlich.

## Fälle für die eine spätere Remote-Task-Suite

Diese Fälle sind Abnahmespezifikation, **keine ausgeführten Tests**. Es werden die vorhandenen Entwicklungsartefakte und Adapterfakes der gemeinsamen Batchsuite verwendet.

1. **Unkonfigurierter Producer:** Der Fake liefert zunächst nur einen Formatbereich, feste Format-/Bufferfakten erst nach nativer Aktivierung. Der Stream bleibt nicht wegen `INACTIVE` oder fehlender Producerabfrage Pending. Kein CPUbild entsteht vor Bildgrant; jeder Importmap hat seinen eigenen vorherigen Grant.
2. **Schon empfangene Ereignisfolge:** In derselben Empfangslieferung folgen `add_mem`, `port_use_buffers` und Prozesssignal. Pause im ersten Ereignis verhindert das folgende Mapping bis zur Bewilligung. Resume liefert unmittelbar ein zweites `add_mem`; dessen neue Pause bleibt gesetzt und verursacht keinen reentranten Mutex-Sperrkreis.
3. **MemPtr vor `add_buffer`:** Der Remote-Node-Fake versucht eingebettete Pixel und große Metadaten vor dem Streamcallback zu mappen. Beide verwenden ausschließlich die bereits zugelassene Whole-Map. Der Fall scheitert bewusst bei der verworfenen Strategie „nur MAP_BUFFERS abschalten“.
4. **Konkrete Größen und Aliase:** Ein FD trägt mehrere Buffer, ein Buffer nutzt getrennte FD-Objekte, Zeilen sind gepaddet, Seitengröße unterscheidet sich vom üblichen Wert. Buchung entspricht exakt den tatsächlichen Ganzobjektabbildungen; weder 64 noch `k × width × height × 4` ersetzt die Fakten. Eine echte zweite Abbildung wird als zweiter Besitzer gezählt.
5. **Fehlende Producer-Bufferparameter:** Nach festem gepacktem Format liefert die Consumeranforderung ausreichende Mindestnutzfläche ohne Bufferanzahl. Der Nativefallback bleibt serverseitig; kleine und 4K-Bilder scheitern nicht an einem künstlichen 8192-Byte-Bildpuffer. Abweichende valide Strides werden aus dem wirklichen Chunk verwendet.
6. **Eine Parentautorität:** Zwei Workerimportbedarfe, der CPU-/Hostplan und ein noch auslaufender Import konkurrieren in derselben Parentkapazität. Die Summe wird atomar eingehalten; Kind-Fakten erzeugen keinen zweiten Pool. Ein voller angenommener W-Vertrag bleibt unverändert, X entsteht nur für die ungedeckte Gleichzeitigkeit.
7. **Unbound ist kein Bildrecht:** Ein Kontrollimport vor Format gelingt bei vorhandener Kapazität; `SourcePixels`, Encode, falsche Quelle, falsche Verbindungsgeneration und ein wiederverwendeter Credit werden über diesen Tickettyp abgewiesen. Die spätere Legacy-/v2-Vertragsbindung bleibt einmalig.
8. **Grow/Resize und noch gehaltene Bilder:** Neue FD-Länge beziehungsweise neue Pixelgeneration wird vor neuem Mapping/Builder zugelassen. Alte Mappings und ausgelieferte Bilder bleiben gleichzeitig gezählt. Ablehnung erzeugt weder Teilbild noch stilles Clipping; FD-/mem_id-Wiederverwendung übernimmt keinen alten Besitzer.
9. **Vollständiges Retirement:** `queue_buffer`, `remove_buffer`, `remove_mem`, `mempool.removed`, Rückkehr aus Unmap, Streamdestroy und Coredestroy werden einzeln angehalten. Kredit wird erst nach der tatsächlichen letzten Mappingfreigabe und Callbackrückkehr frei. Ein ausgeliefertes `CapturedImage` hält seinen Bildslice danach weiter. Eigene Whole-Map-Referenz verhindert das Retirement nicht dauerhaft.
10. **Abbruch und verzögerte Antwort:** Close bei offener Parentanfrage; Bewilligung erst nach Close; Parent-EOF; Workercrash ohne RETIRED; Mapping- und Pausefehler; ausstehende native PAUSED-Bestätigung. Es entsteht kein neues Mapping nach terminalem Zustand. Pendinganfrage, Controllerjoin und Pipe-Reader bleiben abbrechbar. Unbestätigter Prozess-/Mappingabschluss bleibt Retiring.
11. **Verhaltenserhalt:** Beide Portalvarianten, Auswahl mit Node/Serial und Sitzung, Embedded-Cursor/Leerchunk, MemFd, MemPtr, kleine v2-Verträge bei größerer nativer Quelle, Legacy, native 4K, Preview/Full, Snapshot und Familienmitglied bleiben an demselben Admissionkontext. Keine neue Rootaufnahme, kein `PIPEWIRE_NODE`-Override, keine Wiederbelebung abgelaufener Quellidentität; Release-Eingaben bleiben möglich.

Ein fakebasierter Ereignisbeleg ersetzt keine Aussage über die intern gewählte Bufferzahl eines realen Compositors. Eine verfügbare reale PipeWire-Ausführung ergänzt den Nachweis. Fehlende Liveausführung wird als nicht ausgeführt dokumentiert; sie ist nach Projektregel kein zusätzlicher Releasehaltepunkt.

## Verbleibende Grenzen und Abschlussbefund

**Die öffentliche API-Frage ist für die beschriebene Produktbesitzgrenze beantwortet:** Corepause, FD-Import ohne Mapping, Folgenachrichtendispatch und Wiederverwendung einer bereits bestehenden vollständigen Abbildung sind an den benannten Versionen nachgelesen. Die alte Verhandlungsgarantie wird ausdrücklich zurückgenommen. Die oben benannte native Vertrauensgrenze bleibt ausdrücklich erhalten.

Noch herzustellende Implementierungsbelege sind die ABI-/Hookanbindung, die Unbound-Importklasse im parallel entstehenden Dispatcher, die reentranzsichere Controllerzustandsmaschine, die wirkliche letzte Mappingreferenz und die genaue einmalige Verrechnung der nativen Imports mit dem Bild-/Hostplan. Die fünf zum Lesezeitpunkt fehlenden Workerdateien sind oben vollständig benannt. Die vorgeschlagenen neuen Module sind Planänderungen und existieren durch diesen Nachtrag noch nicht.

Nicht ermittelt und nicht behauptet werden die Bufferzahl, Poolgröße oder Paddingwahl des auf dem Nutzergerät laufenden Compositors, dessen Producer-/Serverinternas, gesamte Treiberbelegung, physisch residenter RAM, zukünftiger Speicherdruck oder eine systemweite Autorität über unabhängige Dienste. Ein tatsächlicher großer gepackter FD ist dagegen beobachtbar und wird nach diesem Plan vollständig vor seiner lokalen Abbildung gezählt. Diese Unterscheidung ist Bestandteil des Ergebnisanspruchs.

## Lese- und Änderungsbericht

Gelesen wurden die folgenden lokalen Dateien innerhalb des freigegebenen Manifests. Bei großen Dateien wurden relevante Symbole, Treffer und Abschnitte gelesen; dies ist keine Behauptung einer vollständigen Prüfung jeder Zeile. `AGENTS.md` lag als vollständiger Nutzer-Anweisungstext vor.

- `AGENTS.md` (bereitgestellter Anweisungstext)
- `docs/ARCHITEKTUR.md`
- `docs/plaene/2026-10-24-batches/aufnahme-speicher.md`
- `docs/plaene/2026-10-24-batches/umsetzung.md`
- `native/src/screen/os/linux_os/pipewire_abi.rs`
- `native/src/screen/os/linux_os/pipewire_stream.rs`
- `native/src/screen/os/linux_os/pipewire_pause.rs`
- `native/src/screen/os/linux_os/pipewire_buffers.rs`
- `native/src/screen/os/linux_os/pipewire_capture_memory.rs`
- `native/src/screen/os/linux_os/spa_pod.rs`
- `native/src/screen/os/linux_os/portal_session.rs`
- `native/src/screen/os/linux_os/portal_input.rs`
- `native/src/screen/os/linux_os/capture_prepared.rs`
- `native/src/screen/os/linux_os/pipewire_identity.rs`
- `native/src/screen/os/linux_os/portal_source_binding.rs`
- `native/src/screen/core/capture_source_contract.rs`
- `native/src/screen/core/capture_admission.rs`
- `native/src/screen/core/capture_plan.rs`
- `native/src/screen/core/host_codec_adapters.rs`
- `native/src/screen/core/worker_capture_wire.rs`
- `native/src/screen/core/worker_wire.rs`
- `native/src/screen/core/host_server.rs`
- `native/src/screen/os/shared/host_worker_reader.rs`
- `native/src/screen/os/shared/host_worker_main.rs`
- `/tmp/smartexplorer-pipewire-admission-plan-scope.json`
- `/tmp/smartexplorer-pipewire-admission-plan.md` (eigener Ergebnisstand)

Die Leseversuche für `native/src/screen/core/host_admission.rs`, `native/src/screen/core/capture_transport.rs`, `native/src/screen/os/shared/capture_broker.rs`, `native/src/screen/os/shared/capture_dispatch.rs` und `native/src/screen/os/shared/host_worker_prepare.rs` ergaben jeweils „No such file or directory“. Diese Dateien wurden nicht gelesen. Arbeitsweise-Instruktionen und deren Recherche-/Planungs-/kreative Referenzen wurden über das bereitgestellte MCP geladen; kein Graph oder Repositorybereich außerhalb des Manifests wurde erkundet.

Erstellt und geändert wurde ausschließlich **`/tmp/smartexplorer-pipewire-admission-plan.md`**. Primärquellen, Versionen, Befunde, Entscheidung, Änderungsdateien, Abnahmesignale und verbleibende Integrationsbelege stehen oben. Es gab keine Produktänderung, keinen Build, Compiler, Testlauf, Formatter, Server, Paketinstallation, Graphupdate, Commit, Push, Release oder Unteragentenstart.

Main übernahm die vollständige temporäre Übergabe in diese kanonische Datei und entfernte danach die Arbeitskopie. Der Aufnahmeplan verweist auf diesen Ersatzvertrag. Die dort aufgeführten Implementierungsbelege bleiben offen, bis der Produktanschluss integriert ist.
