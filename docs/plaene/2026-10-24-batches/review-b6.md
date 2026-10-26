# B6 – statischer Code- und Primärquellenreview

Stand: 2026-10-26 UTC. Vergleichsbasis `bea312ea`; gemeinsamer Arbeitsbaum, zuletzt beobachteter HEAD `d3c6a994`. Dieser Bericht bewertet den gelesenen Quellstand. Er bestätigt weder Buildfähigkeit noch Laufzeitabnahme oder Veröffentlichung.

Aktueller Befundstand: **B6-R9 und B6-R10 bleiben offen.** Die neue native v2-Klasse bis 864 MiB beseitigt die ursprüngliche pauschale 4K-Sperre rechnerisch. R9 verbleibt wegen fünf konkreter Lücken: Lebensdauer verzögerter CPU-Uploads, cgroup-Namespacewurzel, terminale Drop-Reihenfolge, dauerhaft gebundene YUV-Sitzungszusage und unversionierter Snapshot in einer v2-Sitzung. R10 betrifft ein altes asynchrones Skalierergebnis, das nach Sitzungsersatz die neue Sitzung bestätigen kann. B6-R3, B6-R4 und B6-R8 sind statisch geschlossen; R1, R2, R5 und R6 wurden bereits im Erstreview nachgelesen. Die R7-Schließung gilt für die dort nachverfolgten Transport-/Android-/Scalerbesitzer; sie ist kein Nachweis der jetzt unter R9 geprüften prozessweiten Desktopzulassung und verzögerten Rendererübergaben. Parallel bereits gestartete Autorenkorrekturen sind in diesem Durchgang nicht erneut abgenommen.

Die folgenden R9-/R10-Befunde beziehen sich auf den oben bezeichneten historischen
Reviewkandidaten. Die anschließende Capture-, Renderer-, Login- und Androidarbeit
ist in `umsetzung.md` nachgeführt. Neuere konkrete Batchbefunde stehen in
`review-capture-integration.md`, `review-login-bildspeicher.md` und
`review-android-bildfixtures.md`; dieser Bericht behauptet keine erneute Prüfung
des inzwischen weiterentwickelten Gesamtstands.

## Auftrag, Abgrenzung und Entscheidungsgrundlage

Der Auftrag umfasst P2, P3, P6, P7, P9, P27, P30, P31-Bildregionen und P32. Nutzerwortlaut vom 2026-10-24/25: „aufgrund des umfangs der aufgaben, ohne dass das weiteres vorgehen blockt, will ich dass du danach n astra xhigh die sachen kritisch reviewn lässt. per Batch“ und „also rein codeseitig und gegen quellen reviewen lässt“. Für P32 gilt ausdrücklich „nach einstellungen des nutzers im system“, präzisiert als „also nicht den einstellungen in der app, in androud“.

A — Ausgangsplan: Die 204 freigegebenen Dateien aus `/tmp/smartexplorer-review-b6-scope.json` begrenzen den Review. Der Hauptagent gab anschließend zusätzlich `native/android-bridge/src/screen_jni.rs` für die direkte Buffergrenze frei. Untersucht werden Referenzidentität, Größen-/Speicherverträge, Codecaufrufe, Worker-/Wireübergänge, tatsächliche Darstellung, Eingabefreigabe, Sichtbarkeit, Widgetdrag, Iconbindung und Medienzustand. P1-Broker, P4-OSfensteroperationen und P5-Appstart werden nicht als eigene Features bewertet.

B — Bestand: Die Umsetzung besitzt einen versionierten Bildvertrag, Hybridrekonstruktion, Quellen- und Darstellungsreferenzen sowie getrennte Android-/Windows-/Linuxadapter. Die zuvor angenommene Einbildzuordnung der Decoder und die OS-Verdeckungs-/Rotationsgrenzen benötigen Herstellerbelege. Planungstiefe 2: mehrere bestehende gekoppelte Verträge; keine Produktimplementierung durch den Reviewer.

C — Entscheidungen: Befunde werden aus konkreten Aufrufpfaden und aktuellen Primärquellen abgeleitet. Bedingte Defekte nennen ihren Auslöser; ein unbekanntes betroffenes Gerätemodell wird nicht als beobachteter Geräteausfall ausgegeben. Historische Plan-/Releaseaussagen gelten nicht als Abnahme. Eine Reparatur gehört ausschließlich dem Hauptagenten.

D — Abschlussgegenstand: priorisierte Defekte mit Ort, Auslöser, Wirkung und Korrekturziel; separat während des Reviews korrigierte Stellen, statisch erhaltene Fähigkeiten und ungeprüfte Grenzen. Keine Builds, Compiler, Tests, Server, Installationen, Graphneubauten, Commits, Pushes oder Releaseaufrufe durch diesen Reviewer.

### Nachreview vom 2026-10-26

Der Nachreview begann mit 90 Dateien in `/tmp/smartexplorer-review-b6-final-scope.json` sowie `docs/oberflaechen/bildschirm.md`. Der ausdrücklich erweiterte R7-Folgeauftrag umfasst die abschließend 99 inventarisierten Dateien und aktuelle Primärquellen. Auftrag: die integrierten R3-/R4-Korrekturen einschließlich Codecidentität, Android-Einzelinputausgabe, tatsächlicher Encoder-Ready-Bindung, vollständiger GOP-ID im Pullvertrag, Qualitätspräferenzen, Referenz-/Darstellungsepochen und sichtbarer Fallbackdiagnose beurteilen. Produktdateien bleiben außerhalb der Schreibfläche.

Erstplan aus dem Auftrag: Diese acht Übergänge gegen den vorhandenen Bericht und die aktuellen Quelldateien abgleichen; den gemeldeten Queuebudgetverdacht bis zu Allokation und JNI-Pull verfolgen. Bestandsplan nach der ersten Lektüre: Die neue `DecodedFrame`-API bis zur Renderbestätigung nachverfolgen; Androids Latenzhinweise vom tatsächlichen Einzelinputnachweis trennen; die Host-/Viewerfallbacks und gehaltenen Präferenzen prüfen; die Ressourcengrenzen anhand eines konkret zulässigen v2-Angebots berechnen. Zweite Quellenprüfung: MFT-Zeitstempel und `ProcessInput`, Android `KEY_LATENCY`/`KEY_LOW_LATENCY` sowie der gebundene OpenH264-Rustwrapper 0.9.9 und Ciscos Ausgabezeitstempelfelder. Akzeptanzsignal ist die belegte Schließung oder ein konkreter verbleibender Fehler je Übergang; kein Laufzeitversprechen.

R7-Folgeauftrag: Die Speicherrechnung umfasst zusätzlich QUIC-Empfang, Daemon-Relay, Workerantworten und den Lebenszyklus der Kotlin-Pullbuffer. Erstplan ist die Zuordnung jeder gleichzeitig gehaltenen Kopie zu einem begrenzten Besitzer. Nach Lektüre der freigegebenen IPC-Verbraucher steht die Bindung vor der Allokation fest; der abschließende Planpunkt war die gemeinsame Rechnung mit Paket-/Queuegrenzen und wiederverwendeten Androidpuffern einschließlich gültiger verlustfreier Frames; die anschließende Scalerlektüre erweiterte diesen Punkt auf dessen tatsächliche Besitzer. Das frühere 8-MiB-/512×512-Angebot muss nicht weiterhin zulässig sein; rechtzeitige Ablehnung oder ein kleineres belegtes Angebot erfüllen die Grenze. Direktbuffer- und MediaCodec-Lebensdauer werden gegen die aktuelle Android-Referenz geprüft.

4K-Folgeauftrag: Zusätzlich freigegeben sind genau `native/src/app/core/screen_render_caps.rs` und `native/src/app/core/screen_pool.rs`. Erstplan: V7/P30 gegen die rechnerische Zulassung, das tatsächliche Desktopangebot und den Full-/Previewübergang abgleichen. Bestandsplan: Die feste 512-MiB-Obergrenze, den Faktor 24 und die erneute Quelllimit-Schnittmenge getrennt bewerten; eine größere ressourcengebundene Angebotsklasse mit unveränderter Besitzerbilanz berechnen. Zweite Recherche: Hello-Reihenfolge und HMAC-Eingabe im Quelltext, JSON-Erweiterbarkeit bei Serde sowie dokumentierte Windows-/Linux-/Android-Speicherinformationen. Ergebnis ist B6-R9 mit konkreten Anschlussstellen und einer Korrekturoption; keine Produktänderung.

R9-/R10-Nachreview: Verbindliche Grundlage sind `/tmp/smartexplorer-r9-review-brief.txt` und die exakte Vereinigung der `read`-/`write`-Pfade in `/tmp/smartexplorer-r9-scope.json`; diese Arrays erweitern ausschließlich die Lesefläche dieses Reviewers. Erstplan: größere v2-Klasse, tatsächliche Ressourcenfakten, gemeinsame Sitzungszulassung und Vorschau-Presented gegen ihre Aufrufer prüfen. Bestandsplan: Jede Zusage bis zum letzten Datenbesitzer verfolgen, einschließlich Blocking-Ergebnis, Decoderende, CPU-Uploaddelta und globaler YUV-Ressource; bei der Vorschau alte Skalierergebnisse über Sitzungsersatz betrachten. Zweite Recherche: Rust-Dropreihenfolge, Linux-cgroup-Namespace und tatsächliche Root-Dateierzeugung sowie epaint/eframe 0.29.1. Der Hauptagent hat nach Meldung der Befunde getrennte Korrekturaufträge begonnen; dieser Bericht dokumentiert den zuvor vollständig gelesenen Stand und schließt diese Änderungen nicht vorzeitig.

## Priorisierte Befunde

### B6-R9 — Hoch, offen: 4K-Klasse angebunden, Zulassungs- und Besitzergrenzen noch unvollständig

**Nachgelesene Korrektur:** `frame_budget.rs` trennt jetzt die 864-MiB-Sitzungsklasse von RAW/Legacy mit weiterhin 512 MiB. `FrameCapabilities::admitted/for_server`, die optionale ServerHello-Klasse und Accepted-Prüfung erhalten alte Gegenstellen. `ScreenSource::frame_limits_for_contract` trennt Quellcapturegrenzen vom Sitzungsbudget; Androids wirklich kleineres Heapangebot bleibt erhalten. `GopHeader::decode_prefix` prüft die Kopfstruktur, bevor die echte Erweiterung den vollständigen Vertrag liefert. Die produktiven Desktopöffner verwenden die gemeinsame Zulassung auch für Preview; Preview→Full erweitert den bereits angenommenen Vertrag nicht. Die nachstehende 768-MiB-Rechnung finanziert weiterhin 3840×2160, 24 Bildäquivalente und die unveränderte Record-/Scalerformel. Das ursprüngliche feste Größenhindernis ist damit statisch beseitigt.

**Gemeinsame Zusage:** `frame_memory.rs` zählt `MemoryLease` pro Prozess in einem synchronisierten Pool. Ein aktiver Besitzerbestand hält seine Zulassungshülle stabil; eine neue Zulassung zieht bestehende Zusagen sowohl von der Hülle als auch von frisch beobachtetem Ressourcenangebot ab. Arc-Klone derselben Zusage werden nicht als neue Sitzung gezählt. Windows liefert physisch verfügbaren Speicher, verbleibenden Commit und virtuellen Adressraum; Linux ergänzt MemAvailable, Commit-/Adressraumgrenzen und sichtbare cgroup-Vorfahren. Android bleibt beim begrenzten Heapangebot. Die Quote `min(physisch, Commit, Adressraum)/4` ist ausdrückliche Produktpolitik. Diese interne Zusage reserviert keinen Betriebssystemspeicher gegen fremde Prozesse; sie garantiert weder Gesamt-RSS noch Codec-/Treiber-/Kernelbelegung. Die folgenden fünf Lücken verhindern den vollständigen Abschluss.

#### R9-A — Hoch: CPU-Uploaddaten überleben ihre letzte Speicherzusage

**Ort und Auslöser:** `native/src/app/core/screen_texture_cpu.rs:104`, besonders Zeile 108, ersetzt `uploaded_memory` vor `TextureHandle::set/load_texture`. Beim Sitzungswechsel kann dies die letzte Zusage der bisherigen Ausgabe lösen, bevor deren Bild ersetzt wurde. Zusätzlich kann nach einem Upload das `CpuTexture` bei Hidden, Sitzungsende oder Widgetabbau entfallen, während das hochgeladene `ColorImage` noch in einem ausstehenden `TexturesDelta` liegt.

**Wirkung und Beleg:** epaint 0.29.1 stellt bei `alloc/set` ein `ImageDelta` in `delta.set`; `TextureHandle::drop` gibt den Handle frei, aber `TextureManager::free` entfernt dessen bereits eingereihtes Bild nicht aus `delta.set`. `take_delta` verschiebt die Daten zum nächsten Besitzer. Deshalb darf eine neue Sitzung das bereits freigegebene Poolbudget wieder belegen, während die alte Pixelallokation noch lebt. Die reine Feldreihenfolge von `CpuTexture` und ein Verschieben der Zuweisung hinter `handle.set` schließen den verzögerten Übergang nicht. [epaint 0.29.1 TextureManager](https://raw.githubusercontent.com/emilk/egui/0.29.1/crates/epaint/src/textures.rs), [TextureHandle](https://raw.githubusercontent.com/emilk/egui/0.29.1/crates/epaint/src/texture_handle.rs)

**Nötige Korrektur:** Eine begrenzte Lebensdauerbindung muss die tatsächlichen ausstehenden Bilddaten bis zu deren Abfluss und die ersetzte Ausgabe bis zu ihrem festgelegten Renderer-Ende abdecken. Ein Zeitlimit oder pauschal die nächste GUI-Runde belegt diesen Abfluss nicht. Der Anschluss betrifft `CpuTexture`, Full-/Previewwidget, Logintextur, Sitzungsende und beide eframe-Renderer WGPU/Glow. Eine unbegrenzt in Rendererressourcen geparkte ganze Sitzungszusage wäre ebenfalls keine Schließung; siehe R9-D. Die konkrete Retirementimplementierung gehört dem gesonderten Autorenauftrag.

#### R9-B — Hoch: sichtbares cgroup-Root `/` beweist keine vollständige Hierarchie

**Ort und Auslöser:** `native/src/screen/os/linux_os/session_memory.rs:73` setzt die Grenze für unsichtbare Vorfahren auf `u64::MAX`, wenn der aus mountinfo gelesene Root `/` ist. Ein cgroup-Namespace kann aber sowohl diesen Mountroot als auch `/proc/self/cgroup` relativ zu seiner eigenen Wurzel darstellen. Eine sichtbare Gruppe mit `memory.max=max` kann unter einem unsichtbaren begrenzten Vorfahren liegen. Großes systemweites MemAvailable ist dann kein Prozessbudget. Der gelesene Pfad kann beispielsweise eine 864-MiB-Klasse anbieten, obwohl ein verborgener Vorfahr nur 512 MiB erlaubt. [Linux cgroup-Namespace](https://man7.org/linux/man-pages/man7/cgroup_namespaces.7.html), [hierarchische cgroup-v2-Grenzen](https://docs.kernel.org/admin-guide/cgroup-v2.html)

**Tragfähige Korrekturoption:** Auf einem nachweislich lebenden cgroup2-Mount kann die Existenz controllerunabhängiger Dateien zwischen echter Hierarchiewurzel und sichtbar gemachter Untergruppe unterscheiden. Im Kernel tragen `cgroup.events` und `cgroup.type` `CFTYPE_NOT_ON_ROOT`; `cgroup_addrm_files` wendet das Flag anhand des tatsächlichen `cgroup_parent(cgrp)` an. Ihr Vorhandensein am sichtbaren Mountroot widerlegt daher die behauptete globale Wurzel. Ein Rootbeleg über ihr Fehlen benötigt ausdrücklich ENOENT bei stabiler, zugänglicher cgroup2-Zuordnung und lesbarem `cgroup.controllers`; Zugriff verweigert oder ein verschwundenes Verzeichnis sind kein Beleg. Diese Option gilt für v2, nicht unverändert für v1. Ein pauschaler Linux-Legacyzwang würde das native 4K-Ziel erneut verhindern. [Linux-Kernel: cgroup_base_files und cgroup_addrm_files](https://raw.githubusercontent.com/torvalds/linux/master/kernel/cgroup/cgroup.c)

#### R9-C — Mittel: terminale Drop-Reihenfolge löst die Zusage vor den Restdaten

**Decoder:** `native/src/app/core/screen_decode_thread.rs:150` deklariert `DecodeState.shared` vor `gop`. `StreamEvent::End` und `connection_lost` beenden die Darstellung, löschen aber den Decoder-GOP nicht. Sind die GUI-/Senderreferenzen bereits weg, kann beim abschließenden Drop die letzte `Shared`-Referenz samt Zusage vor den Decoderpuffern entfallen. Die inzwischen korrigierte Feldreihenfolge innerhalb von `Shared` reicht dafür nicht.

**Blocking-Ergebnisse:** `native/src/screen/core/host_pipeline.rs:427` liefert `(stage, produced, request)`, `host_capture.rs:62` liefert `(stage, shot)`. Wird ein fertiges Blocking-Ergebnis nach tatsächlichem Abbruch seines Asyncjobs verworfen und ist die Pipelinezusage bereits weg, fällt `Stage` vor den erzeugten Frame-/Snapshotdaten. Der reguläre `PipelineActor::drop` wartet den Job hingegen ab und entsorgt die Pipeline danach; dieser zusätzliche Auslöser ist deshalb kein behaupteter Fehler bei jedem normalen Disconnect.

**Korrekturziel und Beleg:** Daten müssen in allen terminalen und verworfenen Ergebnisbesitzern vor der letzten Zusage enden, etwa durch einen entsprechend angeordneten Ergebnistyp beziehungsweise explizites Abräumen. Die bereits korrekt angeordneten `Stage`, `Picture`, `Work`, `Finished`, `Planes` und Workerfelder bleiben erhalten. Rust droppt Strukturfelder in Deklarationsreihenfolge und Tuplelemente von vorn nach hinten. [Rust Reference: Destructors](https://doc.rust-lang.org/reference/destructors.html)

#### R9-D — Mittel: globale YUV-Ressource hält ein beendetes Sitzungsbudget unbegrenzt

**Ort und Auslöser:** `native/src/app/core/screen_wgpu_yuv.rs:79/93/253` hält `Planes.memory` in den langlebigen `YuvResources` der Callbackressourcen. `screen_viewer_app.rs::new` installiert diesen Besitzer einmal; `ViewState::end_session` räumt ihn nicht ab. Ein Wechsel auf CPU-Ausgabe, Rückkehr zur Kachelansicht oder Schließen der letzten YUV-Sitzung liefert keinen eigenen Endpfad. Die Zusage endet erst bei späterem YUV-Ersatz, ungültigem Upload oder Rendererabbau.

**Wirkung:** Hat die beendete Sitzung die stabile Poolhülle ausgeschöpft, bleibt die nächste Zulassung gesperrt. Beispielsweise liefert ein ausreichend großer Anfangsfakt eine 768-MiB-Hülle, deren vollständige Zusage nach Sitzungsende allein in `Planes` hängt. Eine neue Sitzung erhält dann kein Budget und erreicht folglich auch den ersetzenden Upload nicht. Die Feldreihenfolge schützt zwar vor vorzeitiger Freigabe, erzeugt hier aber eine Wiederverbindungssperre während des weiter geöffneten Viewerfensters.

**Nötige Korrektur:** Ein begrenzter Renderer-Endpfad muss die wirklich noch benötigte Ausgabe erhalten und nach deren Abfluss sowohl Planes als auch Zusage freigeben, auch ohne weiteren YUV-Upload. Er muss mit R9-A zusammenpassen. Nur die Zusage zu entfernen, während die zugehörigen Produktressourcen weiterleben, verschiebt den Defekt zur Unterzählung. Primärbeleg sind die genannten lokalen Besitzer- und Endpfade; eine tatsächliche GPU-Freigabefrist wird nicht behauptet.

#### R9-E — Mittel: Snapshot einer v2-Sitzung bleibt unversioniert und ungebunden

**Ort und Auslöser:** `native/src/screen/core/host_session.rs:333` nimmt `ViewerFrame::Snapshot` auch für v2 an. `host_capture.rs::snapshot_epoch` verwendet die angeforderte Größe beziehungsweise `FrameSize::MAX`, `still_tiles` und einen normalen Tilesencoder; der Snapshotkopf um Zeile 73 setzt immer `frame_contract:None`.

**Wirkung:** `gop_stream::read_picture_stream` und `ScreenStream::next_event` verlangen auch für Snapshot die Paarung zum angenommenen Sitzungsvertrag. Die Antwort einer legitimen v2-Anfrage wird daher als gemischtes v1-/v2-Paar verworfen. Außerdem bindet die Snapshotproduktion ihre Größen-/Encodekopien nicht an das möglicherweise kleine angenommene W. Das ist ein konkreter vorhandener Protokollpfad; eine sichtbare Snapshot-Schaltfläche wird damit nicht behauptet.

**Nötige Korrektur:** Snapshotcapture, verlustfreie Paketierung und Kopf müssen denselben angenommenen Vertrag tragen und vor ihren Allokationen begrenzt werden; echte Legacy-Snapshots bleiben erhalten. Die strikte Paarprüfung aus R1 darf nicht als Umgehung gelockert werden. Primärbeleg sind die drei genannten Produzenten-/Empfängergrenzen.

#### R9-Fixnachweis und Quellenfixtures

Die gelesenen Fixtures decken die allgemeine Poolarithmetik, Arc-Zusage, kleinere Angebote, Helloerweiterung und GOP-Präfixreihenfolge ab. `batches_4k_task_tests.rs` verbindet die 2-Mbit/s-Qualitätspolitik mit der unverkleinerten Quellgeometrie und rekonstruiert separat eine 4K-Referenz durch direkt an `HybridDecoder` gelieferte Lossless-Tiles bytegleich. Das ist kein ausgeführter HostPipeline-/Netzwerkdurchsatznachweis. Die cgroup-Fixtures modellieren sichtbare Vorfahren und unlesbare Daten, aber keinen realen Namespace-Rootbeleg. Das CPU-Fixture hält Quell-/Work-Arcs, nicht einen verzögerten Upload nach Handleabbau. Keine dieser Dateien wurde kompiliert oder ausgeführt. Die fünf offenen Auslöser benötigen konkrete Korrekturen und passende Grenzen in der bereits vorgesehenen gemeinsamen Suite; dieser Review startet keine Suite.

#### Ursprünglicher R9-Befund und hergeleitete Korrekturoption — Historie vor dem R9-Fix

Die folgenden Absätze beschreiben den vor der 864-MiB-Implementierung gelesenen Zustand. Ihre damaligen Gegenwartsformen und offenen Anschlussstellen sind keine Aussage über den oben nachgelesenen Fix; die Rechnung bleibt dessen Grundlage.

**Anforderung und Fehlerstelle:** `vertraege.md:367` verlangt für P30 nach vollständiger verlustfreier Korrektur 3840×2160 bei 2 Mbit/s mit exakt passender Auflösung und pixelgleicher Schriftreferenz. `frame_budget.rs:11/14` kombiniert dagegen `MAX_WORKING_BYTES=512 MiB` mit `VERSIONED_WORKING_IMAGES=24`. Die Konstruktoren `resources` und `legacy_resources` begrenzen das Angebot auf `floor(W/24/4)`; `FrameLimits::with_working_budget` kann es nur weiter verkleinern. `FrameContract::negotiate/validate` erzwingt denselben Quotienten. Mehr tatsächlich verfügbarer Speicher kann diese feste Obergrenze nicht erweitern.

**Auslöser und Wirkung:** Eine 3840×2160-Quelle, hinreichende Capture-/Codec-/Texturgrenzen und zwei v2-fähige Gegenstellen genügen. Ein BGRA-Bild benötigt 33.177.600 Byte; 24 Bildäquivalente benötigen 796.262.400 Byte = 759,375 MiB. Zulässig sind heute höchstens 5.592.405 statt 8.294.400 Pixel. Bei ansonsten unbeschränkter 4K-Quelle ergibt `FrameLimits::fit` im Tilespfad 3153×1773. `host_pipeline.rs:286` bildet die Grenze vor der Bildkodierung; auch `target_size` für Tiles verwendet das bereits verkleinerte Bild. Vollständige Lossless-Korrektur kann deshalb nur dieses verkleinerte Bild wiederherstellen. Mehr Aufbauzeit, weniger fps oder ein höheres Bitratenziel bringen die verlorenen Quellpixel nicht zurück. Das ist ein statischer Vertragswiderspruch, kein ausgeführter Qualitätsvergleich.

**Tatsächlicher Desktopanschluss:** `screen_pool.rs:90` verwendet trotz des Namens `legacy_resources` einen expliziten v2-Vertrag mit dieser Grenze für Preview und Full. `screen_render_caps.rs::capabilities/check/checked_target` leitet seine Grenze ebenfalls aus dem festen Konstruktor ab. `screen_pool::to_view` übernimmt eine zulässige Previewsitzung und sendet lediglich SET_MODE; der bereits angenommene Bildvertrag wird dadurch nicht erweitert. Ein Fix nur am Full-Aufruf oder am Losslessschalter reicht deshalb nicht.

**Damals vorgeschlagene Korrekturoption:** Die bestehende Besitzerrechnung erhalten und eine ausdrücklich erkennbare größere v2-Budgetklasse anbieten, soweit die konkrete Sitzung deren Speicher tragen kann. Eine aus vorhandener Bildfähigkeit abgeleitete obere Klasse wäre `24 × 4 × (4096×2304) = 905.969.664 Byte = 864 MiB`; sie finanziert das bereits bestehende Legacy-Pixelmaximum mit der neuen Besitzerbilanz. Dieser Wert ist eine Implementierungsobergrenze, kein pauschales Geräteangebot. Host und Viewer müssen ihr wirklich verfügbares Sitzungsbudget darunter begrenzen. Kleinere Android-Heaps behalten ihr kleineres Angebot; ihre Grenze darf nicht auf die Desktopklasse angehoben werden.

Als rein rechnerisches Zulassungsbeispiel genügt ein durch Ressourcen belegtes `W=768 MiB`. Angenommen sind ein Seitenrahmen bis 16384, `max_pixels=floor(W/24/4)` und unveränderte R7-Besitzer. Dann ergibt die aktuelle Formel:

| Größe | Wert |
|---|---:|
| W | 805.306.368 Byte |
| max_pixels / I | 8.388.608 Pixel / 33.554.432 Byte |
| N / F / H | 2.561 / 1.443.842 Byte / 1.099 Byte |
| R / E | 4.022.385 / 39.020.659 Byte |
| Pullbuffer | 33.554.464 Byte |
| `20I+32(R+H)+(E−I)` | 805.306.355 Byte ≤ W |
| Gemeinsame Skalierreserve nach 13 festen Bildplätzen | 234.881.037 Byte |

4K liegt innerhalb dieses Pixelangebots. Die Recordgrenze bleibt unter 16 MiB, und die Rechnung behält die beschriebenen 32 Recordbesitzer sowie expliziten Scalerleases. Die Zahlen belegen die Verträglichkeit der größeren Klasse mit der vorhandenen Formel; sie sind weder eine Speichermessung noch ein Nachweis, dass ein konkretes Gerät 768 MiB zusätzlich tragen kann. Ein bloßes Senken von 24 auf 16 ohne veränderte und erneut belegte Besitzer wäre keine Korrektur.

**Vollständige betroffene Schnittstelle der vorgeschlagenen Korrektur:**

1. **Ressourcenfakt → Angebot:** Desktop-Host und Desktop-Viewer benötigen denselben typisierten Budgeteingang bei der Konstruktion von `FrameCapabilities`; Texture-Sidelimits allein belegen kein Arbeitsbudget. Der Hauptagent benennt die bestehende Funktion `crate::transfer::available_memory()` als möglichen Lieferanten. Deren Implementierung gehörte nicht zur freigegebenen Lesefläche und wurde hier nicht unabhängig bewertet. Grundlast, gleichzeitig zugesagte Sitzungsbudgets und konkrete Prozessgrenzen müssen bei der Zulassung berücksichtigt werden. Das Angebot bleibt ein festes Sitzungsbudget; es darf nicht durch bereits gehaltene eigene Bildpuffer nachträglich gegen sich selbst verkleinert werden.
2. **ServerHello → ClientHello → Accepted:** Ein alter `ScreenClientHello::validate` lehnt `working_bytes>512 MiB` vor `FrameContract::negotiate` ab. Der Host muss daher vor dem Clientangebot ein optionales Kennzeichen für den erweiterten Budgetrahmen liefern. Fehlt dieses Kennzeichen, sendet der neue Viewer höchstens das bisherige Angebot. Ein alter Viewer ignoriert das zusätzliche JSON-Feld und bietet weiterhin seinen bisherigen Rahmen an; der neue Host darf den angenommenen Vertrag nie darüber hinaus erweitern. Dafür sind `wire_hello.rs`, die Auswahl beim Helloaufbau und `viewer_client::handshake_for` sowie beide Vertragsvalidatoren betroffen. Die vorhandene GOP-/Accepted-Bindung und Recordableitung bleiben erhalten. Ein vergrößertes unbedingtes Clientangebot ist nicht abwärtskompatibel. Serde dokumentiert das Ignorieren unbekannter JSON-Felder ohne `deny_unknown_fields`; die gelesenen Hellotypen verwenden dieses Attribut nicht. [Serde: Container attributes](https://serde.rs/container-attrs.html#deny_unknown_fields)
3. **HMAC-Grenze:** Im aktuell gelesenen `wire_hello::screen_transcript` gehen nur `server.relation()` und `client.relation()` ein. Deren Konstruktoren enthalten weder `frame_capabilities` noch `display_size`, `max_fps`, `icon_version` oder `family_version`. Allein das Begrenzen von `frame_capabilities` ändert diese bestehende HMAC-Eingabe deshalb nicht. Die Budgetauswahl sollte dennoch vor dem Aufbau des versendeten Client-Hellos erfolgen, damit Angebot und späterer Accepted-Vergleich dieselbe Größe verwenden. Ein neuer Fähigkeitsmarker ist ohne explizite Transkriptänderung nicht zusätzlich durch dieses HMAC gebunden; eine solche Bindung darf nicht behauptet werden.
4. **Quellcapture → Sitzungsvertrag:** `host_pipeline::fitted_size` schneidet `working_bytes` mit `ScreenSource::frame_limits_for_contract` und wendet danach erneut `/24` an. Ein auf 768 MiB vergrößerter Sitzungsvertrag würde durch einen unverändert als 512 MiB gemeldeten RAW-Quellvertrag erneut verkleinert. Die getrennte Zulassung des Einzelcaptures muss dessen Seiten-, Pixel- und Capturepuffergrenzen erhalten; die 24 sitzungsweiten Besitzer müssen das eigens zugelassene Sitzungsbudget verwenden. `ScreenSource::frame_limits_for_contract`, Worker-Quellfakten und der PRODUCE-Vertrag müssen diese Unterscheidung ausdrücklich tragen. Androids `BridgeSource` enthält hingegen ein wirklich kleineres Heapangebot und darf dieses bei der Trennung nicht verlieren.
5. **Renderer und Sitzungswechsel:** `screen_pool::renderer_capabilities` und `screen_render_caps` müssen dieselbe angenommene Ressourcenklasse verwenden. Ein Previewvertrag muss den für spätere Fullnutzung zugesagten Rahmen bereits tragen, oder der Übergang benötigt eine explizite neue Zulassung, bevor größere Frames übertragen werden. Getrennte Full-/Previewgrößen und -raten sowie der einmalige Losslessfallback aus R4/R8 bleiben erhalten. Kein SET_MODE darf den bestehenden Speichervertrag still erweitern.
6. **Parser, Pull und Legacy:** Die höhere unterstützte Obergrenze muss in `FrameLimits` und bei teilnehmenden Kotlin-Verbrauchern in `ScreenBufferBudget` konsistent geprüft werden; QUIC-/IPC-/Worker-/JNI-Limits werden weiterhin aus dem tatsächlich angenommenen Vertrag berechnet. Zusätzlich erzeugt `wire_binary::GopHeader::decode` heute einen vorläufigen Vertrag mit `working_bytes=MAX_WORKING_BYTES` und prüft ihn bereits, bevor `read_gop_header` beziehungsweise `ipc_protocol_screen::decode_relay_record` die echte Vertragserweiterung einsetzen. Bleibt diese Konstante für RAW bei 512 MiB, würde der Zwischenvertrag einen neuen zulässigen 4K-GOP weiterhin ablehnen. Kopfstrukturprüfung und vollständige Vertragsprüfung müssen diese Reihenfolge ausdrücklich berücksichtigen; erst die echte Erweiterung darf die Speicherfreigabe liefern, weiterhin an Accepted gebunden. RAW-/Sourcegrenzen, `FrameLimits::LEGACY`, Kotlin-LEGACY und die D3D-Capturebilanz bleiben bei ihren bisherigen Grenzen. Ein globales Ersetzen der gemeinsam verwendeten 512-MiB-Konstante würde diese Verträge mitverändern. Kleine Androidangebote bleiben mindestens durch den beim Öffnen beobachteten freien Java-Heapanteil begrenzt. Ohne unabhängigen größeren Plattformbeleg ist für Android kein größeres Angebot begründet.

**Primärquellen und fehlende Plattformfakten, geprüft am 2026-10-26:** Windows bietet mit `GlobalMemoryStatusEx` momentane physische und virtuelle Speicherinformationen; `ullAvailPageFile` bezeichnet den noch möglichen Prozess-Commit. Microsoft kennzeichnet die Werte ausdrücklich als veränderlich. Eine Abfrage ist deshalb keine Reservierung. [GlobalMemoryStatusEx](https://learn.microsoft.com/en-us/windows/win32/api/sysinfoapi/nf-sysinfoapi-globalmemorystatusex), [MEMORYSTATUSEX](https://learn.microsoft.com/en-us/windows/win32/api/sysinfoapi/ns-sysinfoapi-memorystatusex). Unter Linux ist `MemAvailable` eine Schätzung des ohne Swapping verfügbaren Speichers; cgroup-v2-Grenzen können den Prozess zusätzlich beschränken und gelten hierarchisch. `memory.current` und `memory.max` müssen bei zutreffender cgroup-Zuordnung berücksichtigt werden. [Linux /proc](https://docs.kernel.org/filesystems/proc.html), [Control Group v2](https://docs.kernel.org/admin-guide/cgroup-v2.html). Androids `Runtime.maxMemory` bezeichnet das Maximum, das die VM zu verwenden versucht; `totalMemory/freeMemory` beschreiben den Javaheap. Daraus folgt kein freier Native-/GPU-Speicher. [Android Runtime](https://developer.android.com/reference/java/lang/Runtime#maxMemory())

Unbekannt bleiben die tatsächlich verfügbaren Budgets der Zielgeräte, die genaue Semantik des vom Hauptagenten benannten Speicherhelpers, zusätzliche Prozess-/Containergrenzen und die erforderliche Reserve für Codec, Treiber, Grafik und Anwendungsgrundlast. Die Korrekturoption ist deshalb an einen belegten Ressourcenlieferanten und dessen gemeinsame Zulassung gebunden. Annahmen über einen pauschal ausreichenden Desktop oder Androidheap sind nicht Teil des Befunds.

**Schließungssignal:** Der konkrete Code muss 3840×2160 bei ausreichendem Ressourcen-, Codec- und Texturangebot zulassen, die vollständige verlustfreie Rekonstruktion auf diesen Quellpixeln halten und die gemeinsame Pufferformel weiter erzwingen. Kleineres Ressourcenangebot, alte Gegenstellen, unversioniertes Legacy und Preview→Full benötigen weiterhin ihre ausdrücklich begrenzten Pfade. Die spätere gemeinsame Task-Suite muss das V7-Ergebnis bei 2 Mbit/s auswerten; dieser Reviewer hat weder Code repariert noch Tests ausgeführt. Bis zum nachgelesenen vollständigen Anschluss bleibt R9 offen.

### B6-R10 — Hoch, offen: altes Skalierergebnis kann eine neue Previewsitzung bestätigen

**Korrigierter Ausgangsfehler:** `accept_preview_picture` verwendet jetzt `can_present` statt der bereits freigegebenen Eingabe als Voraussetzung für das neue Bild; `update_frame` erhält dessen Darstellungsidentität. `draw_tile` erzeugt den Presented-Callback erst nach Vorbereitung und Einfügen der sichtbaren Textur in den Paintpfad. Innerhalb derselben Sitzung durchbrechen diese Änderungen die frühere Hidden→Visible-Abhängigkeit; Ack, Snapshotfloor, Epoche, Hidden und geschlossene Sitzung werden beim Callback erneut geprüft. Das ist ein statischer Nachweis des Aufrufpfads, kein physischer Scanoutbeleg.

**Verbleibender Ort:** `native/src/app/core/screen_tile_grid.rs::step/open_preview` behält beim Ende beziehungsweise Ersatz einer Sitzung die `CpuTexture`; `screen_tile_face.rs:151` setzt im Retryzweig Session und Picture zurück, nicht den Texturworker. `screen_texture_cpu.rs:129` akzeptiert ein fertiges Skalierergebnis anhand gleicher Quell-/Zielmaße und `done.seq >= self.presented`. Eine Sitzungsidentität fehlt. `screen_presented::PresentedFrame` enthält nur Epoche, GOP-ID und Frame-ID; diese Zähler können in einer neuen Sitzung dieselben Werte erreichen. `screen_tile_face.rs:163` bindet den so erhaltenen alten Beleg an den Callback der aktuell in der Kachel gespeicherten Sitzung.

**Konkreter Auslöser:** Sitzung A beendet sich mit einem noch laufenden oder fertigen CPU-Skalierauftrag. Sitzung B ersetzt A in derselben Kachel und liefert ein Bild mit gleichen Quell-/Zielmaßen. Das Ergebnis von A besitzt eine Sequenz, die noch nicht dargestellt wurde; daher passiert es den genannten Vergleich trotz des inzwischen neuen Source-Bilds. Stimmen seine lokalen Epoch-/GOP-/Framewerte mit einem zulässigen B-Beleg überein, kann `preview_presented` den A-Pixelinhalt über B bestätigen und B-Eingabe freigeben. Das ist keine notwendige Folge jeder Wiederverbindung, sondern ein belegbarer zulässiger Ablauf ohne eindeutige Sitzungskopplung.

**Nötige Korrektur:** Quelle, Work, Finished, Upload und Presented müssen dieselbe eindeutige Sitzungsbesitzeridentität tragen; alternativ muss jeder Ersatz-/Retry-/Offline-/Endpfad den alten Worker samt Ergebnis sicher invalidieren. Der tatsächliche Bufferabbau muss gleichzeitig R9-A erfüllen. Ein Vergleich nur mit einer wiederverwendbaren GOP-/Frame-/Epochkombination genügt nicht. Full→Preview, Preview→Full, Legacy und die unabhängige Logintextur dürfen dabei keine fremde Bestätigung erhalten.

**Fixturegrenze und Primärbeleg:** `batches_preview_task_tests.rs` prüft an einer einzelnen Shared-Sitzung fehlendes Ack, alte Epoche, nicht hochgeladenes beziehungsweise nicht gezeichnetes Bild und Hidden. Die verwendete kleine unvergrößerte Textur durchläuft keinen konkurrierenden Skaliererabschluss über einen Sitzungsersatz; genau dieser Auslöser fehlt. Primärbeleg sind die genannten lokalen Zähler-, Worker- und Callbackpfade. Keine GUI-/GPUausführung. R10 bleibt bis zur eindeutigen Sitzungsbindung offen.

### B6-R1 — Hoch, inzwischen statisch korrigiert: v1-GOP umgeht ausgehandelten v2-Speichervertrag

**Nachtrag:** `read_picture_stream` prüft jetzt die vollständige Paarung aus GOP- und Sitzungsvertrag. `(None, None)` erhält alte Sitzungen; `(Some, Some)` verlangt gleiche Version und Limits sowie die ausgehandelte Hybridberechtigung. Beide gemischten Paarungen werden vor dem Lesen der Records verworfen. Die konkrete Vertragsumgehung ist damit im erneut gelesenen Pfad geschlossen. Keine Netzwerk- oder Speicherlaufzeitprüfung.

**Ort:** `native/src/screen/core/gop_stream.rs::read_picture_stream`, insbesondere die Prüfung nach `read_gop_header` um Zeile 188; `wire_binary.rs::GopHeader::decode`; `screen/os/android/viewer_sink.rs::ViewerSink::record`.

**Ursprünglicher Auslöser:** Eine Sitzung akzeptiert v2 mit beispielsweise 32 MiB `working_bytes`. Der Peer sendet anschließend einen v1-GOP mit 4096×2304 Pixeln. Vor der Korrektur lief die Vertragsprüfung nur bei `header.frame_contract.is_some_and(...)`; ein fehlender Bildvertrag passierte sie.

**Wirkung:** Paketbudget und Decoder wählen für diesen GOP `FrameLimits::LEGACY` mit 512 MiB Arbeitsbudget statt der vorher vereinbarten Grenze. Die auf acht BGRA-Arbeitsbilder bezogene Rechnung beträgt bei diesem Beispiel 288 MiB. Das neue Android-Heapangebot wird dadurch nicht durchgängig erzwungen. Ein fehlerhafter oder absichtlich abweichender bereits verbundener Host genügt; aktueller ehrlicher Hostcode ist für den Auslöser nicht erforderlich.

**Korrekturziel:** Die GOP-Vertragsform an den Sitzungsvertrag binden: v1 nur in einer ohne v2 ausgehandelten Sitzung; v2 nur mit passenden ausgehandelten Grenzen. Legacykompatibilität für tatsächlich alte Sitzungen erhalten.

**Primärbeleg:** Der eigene Protokollcode: `viewer_client.rs` prüft den akzeptierten Vertrag gegen das Angebot; `gop_stream.rs` muss dieselbe Bindung vor der Decoderallokation erhalten. Der Fehler folgt aus dieser internen Grenze, nicht aus einer externen Protokollannahme.

### B6-R2 — Hoch, inzwischen statisch korrigiert: fehlender Enumimport im extrahierten Kachelrenderer

**Nachtrag:** Der Hauptagent ergänzte `use super::screen_tile_grid::{Probe, Tile};`. Die neue Importzeile wurde erneut gelesen; die konkret fehlende Namensauflösung ist korrigiert. Kein Compilerlauf.

**Ort:** `native/src/app/core/screen_tile_face.rs:138`, `draw_tile`; Import um Zeile 6; Definition `screen_tile_grid.rs:42`; Moduldeklaration `app/screen_modules.rs:53`.

**Ursprünglicher Auslöser:** Kompilierung des Desktopmoduls vor der Importkorrektur. `draw_tile` verwendete `Probe::Idle`, importierte aus `screen_tile_grid` aber nur `Tile`. Im Modul war `Probe` weder definiert noch importiert.

**Wirkung:** Statisch belegte unaufgelöste Namensreferenz; kein Compileraufruf wurde ausgeführt. Der Extraktionsdiff seit `bea312ea` verschiebt den Retryzweig, ohne seinen Enumimport mitzunehmen.

**Korrekturziel:** Den bestehenden `Probe`-Typ im Modul auflösen; Retryverhalten erhalten. [Rust Reference: Item scopes](https://doc.rust-lang.org/reference/names/scopes.html#item-scopes) beschreibt, dass Modulnamen keine implizite Sichtbarkeit in einem benachbarten Modul erhalten.

### B6-R3 — Hoch, inzwischen statisch korrigiert: MFT-Ausgabe wird mit der Frame-ID des aktuellen Eingangs bestätigt

**Nachtrag 2026-10-26:** `MfDecoder::drain` liest jetzt `GetSampleTime`; `decode` registriert die aktuelle Eingabe erst nach erfolgreichem `ProcessInput`. Vorher abgeholte Ausgabe behält deshalb ihren eigenen Token. `FrameIdentities` ordnet den Token dem tatsächlich eingereichten `DecodedFrame` zu. Der OpenH264-Adapter setzt `uiInBsTimeStamp` und liest ausdrücklich `uiOutYuvTimeStamp`; dessen vorhandener automatischer Flushpfad bleibt erhalten. `ReferenceDecoder` vergleicht die Ausgabe mit einer vollständig eingereichten Referenz. `DecodeState::record` holt den vor dem betreffenden Decode gespeicherten Darstellungsbeleg einschließlich Epoche ab; ein späterer Eingang erzeugt keinen neuen Beleg für das alte Bild. `HybridDecoder` fordert unmittelbare Identitätsgleichheit und liefert bei benötigtem Folgeinput oder fehlender Identität einen typisierten Lossless-Rückfall. Der Desktop fordert diesen einmalig an und zeigt den Grund. Damit ist die falsche Bestätigung des aktuellen Eingangs geschlossen; Treiberverfügbarkeit und tatsächliche Pixelzuordnung sind nicht ausgeführt.

Der versionsgebundene [OpenH264-Rustwrapper 0.9.9](https://raw.githubusercontent.com/ralfbiedert/openh264-rs/v0.9.9/openh264/src/decoder.rs) setzt im bisherigen `decode_with_options` keinen Eingabetoken und bildet `DecodedYUV` aus dem Eingabezeitstempelfeld. Die eigene Raw-API-Nutzung vermeidet diese Grenze. [Ciscos `SBufferInfo`](https://raw.githubusercontent.com/cisco/openh264/master/codec/api/wels/codec_def.h) unterscheidet ausdrücklich Eingabe- und Ausgabezeitstempel. Beide Quellen am 2026-10-26 gelesen; der Cisco-Link ist aktueller Upstream, keine Bestätigung der konkret geladenen Binärdatei.

**Ort:** `native/src/screen/os/windows/mf_decoder.rs::MfDecoder::decode` und `drain` (Zeilen 150–187); `screen/core/frame_reference.rs::ReferenceDecoder::decode`; `app/core/screen_decode_thread.rs::DecodeState::record` um Zeilen 375–398.

**Ursprünglicher Auslöser:** Ein MFT puffert ein vollständiges Access Unit und liefert dessen Bild erst bei einem späteren Eingabeaufruf. Dieser Fall wird von `ReferenceDecoder` nach einem IDR ausdrücklich zugelassen. Der damalige `MfDecoder` setzte Eingabezeiten, las die Ausgabezeit jedoch nicht. Schon vor Annahme des aktuellen Samples abgeholte Ausgabe konnte über `shown` als Erfolg des aktuellen Records gelten.

**Wirkung:** Das Bild aus AU N−1 erhält den Darstellungsbeleg des Records N. Die Bestätigung beweist damit nicht die behauptete Frame-ID. Im Hybridpfad darf außerdem kein verzögertes Videobild mit Regionen eines neueren Captureframes zusammengesetzt werden. Ob ein konkret installiertes MFT dieses Verhalten zeigt, wurde nicht ausgeführt. Der optionale `MF_LOW_LATENCY`-Hinweis ersetzt keinen Identitätsbeleg.

**Korrekturziel:** Ausgabezeit/zugehörige Eingabereferenz erhalten und nur das tatsächlich ausgegebene Bild bestätigen; Hybridrekonstruktion benötigt dieselbe Captureidentität oder einen expliziten verlustfreien Rückfall.

Microsoft erlaubt das Halten von Eingaben und beschreibt Ausgabezeitstempel als Zuordnung zum tatsächlich verwendeten Eingang: [MFT time stamps](https://learn.microsoft.com/en-us/windows/win32/medfound/time-stamps-and-durations). `MF_E_NOTACCEPTING` fordert zunächst das Abholen bereits ausstehender Ausgabe: [IMFTransform::ProcessInput](https://learn.microsoft.com/en-us/windows/win32/api/mftransform/nf-mftransform-imftransform-processinput).

### B6-R4 — Hoch, inzwischen statisch korrigiert: Android-Einzelinputfallback einschließlich GOP-Wechsel

**Korrektur des zusätzlichen Integrationsbefunds:** `hybridUnavailable` akzeptiert jetzt einen bekannten vollständigen GOP aus dem zur Sitzung gehörenden Streamcache auch nach einem folgenden `NewStream`. Die Oberfläche ruft die Methode nur aus der noch aktiven Decoderinstanz auf. Geschlossene/beendete Sitzungen, unbekannte GOPs und bereits ausgelöster `forceLossless` bleiben ausgeschlossen. Der Rückfall sperrt den aktuellsten GOP, erhöht die Darstellungsepoche und verweigert weitere H.264-Darstellungsbelege bis zum verlustfreien Strom. Quelle, Full-/Previewgrößen und -raten werden weiterhin nicht erneut gesendet. Der konkrete Zeitablauffehler ist damit statisch geschlossen; kein Scheduler-/Gerätenachweis.

**Zusätzlicher ursprünglicher Integrationsbefund aus dem R7-Nachreview:** `ScreenCodecLatency.TRANSACTION_NS` erlaubt zwei Sekunden für die Einzelinputausgabe; `PullQueue::push` verwirft Rückstau nach einer Sekunde und fordert einen Reset an. `host_mode::request_keyframe` invalidiert die Referenz; `screen_view_run` kann den folgenden GOP als `NewStream` melden, während der vorherige Decoderaufruf noch wartet. `ScreenSessionState` übernimmt diesen Stream unabhängig vom Decoderthread. Liefert der vorherige Aufruf anschließend `Unavailable`, verwirft `ScreenSession.hybridUnavailable` das Feedback bei `current.stream.gop != gopId`. Der Decoder sperrt nur den bisherigen GOP und kann denselben ungeeigneten Codec im folgenden GOP erneut versuchen. Die verlustfreie Anforderung und deren Diagnose können dadurch ausbleiben. Voraussetzung ist ein aktiver Bildstrom mit genügend weiteren Records und einem neuen GOP vor Ablauf der Decoderfrist; kein konkretes Gerätevorkommen wurde ausgeführt. Korrekturziel: Codec-Unfähigkeit an die noch aktive Sitzung/Decoderinstanz binden, ohne aus einem abgelösten Decoder oder einer beendeten Sitzung fremde Präferenzen zu ändern. Eine reine Verlängerung der Queuefrist schließt die Ereignisrace nicht grundsätzlich.

**Vorheriger Nachtrag 2026-10-26:** `ScreenCodecLatency` fordert Encoderlatenz 1 und keine B-Frames an, prüft vorhandene bestätigte Latenzwerte und bindet jede vollständige AU an den tatsächlichen ausstehenden Captureframe. Fehlende passende Ausgabe innerhalb der bestehenden Referenzfrist beendet die Annahmefähigkeit; ein bloß bestätigtes Format gilt nicht als Ausgabe. Der Hybriddecoder versucht passende pixelzugängliche Decoder innerhalb einer gemeinsamen Frist und prüft Ausgabe-PTS und Bildgeometrie gegen seinen einzigen Eingang. `Unavailable` schließt die ungültige Referenz, sperrt den betroffenen GOP und führt über `ScreenSession.hybridUnavailable` zu einer expliziten verlustfreien Anforderung. Hostfehler laufen über `encoder_failed` zum dauerhaften Tiles-Rückfall der Pipeline; beide Viewer zeigen dessen Grund. Der Rückfall verwendet weiterhin die bestehende MediaProjection-Auswahl; fehlende Appzuordnung wird nicht durch einen Vollbildscreenshot ersetzt.

`ScreenHostControl.reportProjectionReady` hält den konkreten Encoder und dessen Token; Capturefreigabe erfolgt erst nach erfolgreichem `ScreenApi.ready` und erneutem Identitätsvergleich. `BridgeSource::h264_frame` liest nach dem Eingang die tatsächlich gemeldeten Encoderangaben. Ein früheres Fähigkeitsangebot oder Ready einer abgelösten Instanz kann so den ersten Hybridcapture nicht freigeben. Der ursprüngliche unerkannte Einbildlatenzvertrag ist damit geschlossen; die Ressourcen- und Präferenzgrenzen werden separat unter R7 und R8 bewertet. Keine Codec-/Geräteausführung und keine garantierte Framezeit.

**Ort:** Android `screen/ScreenEncoder.kt::queueRgba` um Zeile 196 und `open` um Zeile 310; `ui/screen/ScreenHybridDecoder.kt::openVideo/decodeVideo` um Zeilen 135–182; Rust `screen/os/android/bridge_source.rs::h264_frame`.

**Ursprünglicher Auslöser:** Ein angebotener MediaCodec benötigt mindestens zwei Eingabebilder, bevor er eine Ausgabe liefert. Der Encoder lehnt jeden weiteren Eingang ab, solange `pending` nicht leer ist; der Host wartet auf genau dessen Ergebnis. Der damalige Hybriddecoder führte einen Eingang zu und wartete bis zu zwei Sekunden auf dessen Ausgabe, ohne daraus eine unpassende Codec-Betriebsart mit dauerhaftem Rückfall abzuleiten.

**Wirkung:** Ein solcher Codec erreicht die nötige zweite Eingabe nicht. Der Host läuft beim Schlüsselbild in Timeout/Rückfall; der Viewer verwirft die Referenz und fordert erneut ein Schlüsselbild an. Flexible YUV-Pixelzugänglichkeit belegt diese zeitliche Eigenschaft nicht. Keine betroffene Geräteliste und keine Ausfallhäufigkeit sind nachgewiesen.

**Korrekturziel:** Entweder eine tatsächlich bestätigte geeignete Codec-Betriebsart mit explizitem Rückfall oder eine begrenzte Pipeline, die mehrere Eingaben mit eindeutig zugeordneten Capture-/Ausgabereferenzen verarbeitet. Nur längeres Warten behebt den fehlenden zweiten Eingang nicht.

Android dokumentiert die Encoder-Standardlatenz als implementationsabhängig; `KEY_LATENCY` ist optional und sein Ergebnis muss bestätigt werden. Decoder-Low-Latency ist standardmäßig ausgeschaltet und beseitigt keine durch den Codecstandard erforderliche Pufferung: [MediaFormat: KEY_LATENCY und KEY_LOW_LATENCY](https://developer.android.com/reference/android/media/MediaFormat#KEY_LATENCY), erneut gelesen am 2026-10-26. Der korrigierte Pfad fragt diese Hinweise an und verlangt zusätzlich passende echte Ausgabe.

### B6-R5 — Mittel, inzwischen statisch korrigiert: X11-Verdeckung berücksichtigt Bounding-Shapes nicht

**Nachtrag:** `shape_get_rectangles(..., SK::BOUNDING)` liefert jetzt die lokale Bounding-Shape. Der Abzug verwendet den Schnitt aus Opaque-Hint, Shape-Rechteck und Clientgeometrie; unbekannte Shapes führen zum Überspringen des betreffenden Covers. Die abschließende erneute Shapeabfrage verwirft ein inzwischen abweichendes Ergebnis. Das `shape`-Feature ist in der x11rb-Abhängigkeit aktiviert. Diese konkrete Lücke ist statisch geschlossen; keine X11-/Compositorausführung.

**Ort:** `native/src/app/os/linux_os/screen_visibility.rs::sample_x11`, Abzug der Opaque-Rechtecke um Zeilen 123–136.

**Ursprünglicher Auslöser:** Ein darüberliegendes ARGB-Fenster hat per X Shape Aussparungen, meldet aber ein größeres `_NET_WM_OPAQUE_REGION`. Vor der Korrektur schnitt der Code die Rechtecke nur mit der rechteckigen Fenstergeometrie; eine Shapeabfrage fehlte.

**Wirkung:** Durch die Aussparungen tatsächlich sichtbare Viewerflächen werden als verdeckt gemeldet. Bei vollständigem rechnerischem Abzug pausiert der Host den weiterhin sichtbaren Bildstrom. Der Dateikommentar behauptet konservatives Verhalten bei geformten Fenstern, der Aufrufpfad setzt es nicht um.

**Korrekturziel:** Opaque-Hint mit nachgewiesener Bounding-Shape schneiden oder bei fehlendem Shapebeleg konservativ sichtbar/unknown bleiben.

Die EWMH-Spezifikation definiert für ARGB-Fenster die wirksame deckende Fläche als Schnitt von Opaque-Hint und Shape-Bounding-Region: [EWMH §5.18](https://specifications.freedesktop.org/wm/latest/ar01s05.html).

### B6-R6 — Mittel, inzwischen statisch korrigiert: DXGI-Fallback rotiert Monitorbilder nicht

**Nachtrag:** Der neue Stand speichert DXGI-Rotation, nimmt die anfängliche Desktopgröße aus `DesktopCoordinates`, rotiert BGRA über `pixel_convert.rs::rotate_bgra` und verwirft native Regionen bei Rotation zugunsten des vollständigen Vergleichs. Die 90°-/180°-/270°-Koordinaten wurden gegen [Microsofts DisplayManager-Beispiel](https://raw.githubusercontent.com/microsoft/Windows-classic-samples/main/Samples/DXGIDesktopDuplication/cpp/DisplayManager.cpp) abgeglichen. Ein Rotationswechsel erhöht die Referenzgeneration. Kein Monitor-/Treibernachweis.

**Ort:** `native/src/screen/os/windows/capture_dxgi.rs::duplicate/read` (Zeilen 63–76, 142–152); `d3d_device.rs::Readback::read`; Auswahl in `capture_source.rs::WindowsSource::open_monitor`.

**Ursprünglicher Auslöser:** Ein um 90° oder 270° gedrehter Monitor wird über DXGI erfasst, weil WGC nicht verfügbar ist oder das Öffnen scheitert. Vor der Korrektur wurde `GetDesc().Rotation` nicht ausgewertet und die gelesene Textur unverändert weitergegeben.

**Wirkung:** Seitwärts dargestellter Desktop und zur Monitororientierung unpassende Bildgeometrie. Auch die neuen Dirty-/Move-Metadaten werden nicht in eine gedrehte Referenz transformiert. Der fehlende Rotationsschritt bestand bereits vor B6, liegt aber direkt in der nun geforderten Portrait-/Rotationsabnahme von P30/P31.

**Korrekturziel:** Capturepixel, gelieferte Größe und optionale Regionen konsistent auf die Desktoporientierung beziehen; bei fehlender Regionstransformation vollständigen Vergleich erhalten.

Microsoft nennt ausdrücklich die unrotierte Acquired-Surface mit darin gedrehtem Desktop und verlangt Rotation im Client: [Desktop Duplication API, Rotating the desktop image](https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/desktop-dup-api).

### B6-R7 — Hoch, inzwischen statisch korrigiert: Arbeitsbudget über Transport, Pull und CPU-Skalierung

**Abschließendes R7-Urteil vom 2026-10-26:** Der ursprüngliche 16-MiB-Record in einer 8-MiB-v2-Sitzung passiert die Aufnahmegrenze nicht mehr. Der bisherige Vertrag 512×512 / `max_pixels=262144` / 8 MiB wird bereits als überhöht abgelehnt. Kleinere belegte Angebote sind zulässig. Record-, Frame-, Queue-, Direktbuffer- und Android-CPU-Skalierungsbesitzer sind im dort freigegebenen Stand gemeinsam begrenzt. R7 ist für diese konkret nachverfolgten lebenden Produktpuffer statisch geschlossen; keine Gesamt-RSS-, Grafik-/Treiber-/Kernel-, Allocator- oder GC-Garantie und keine Laufzeitabnahme. Die anschließend eingeführte prozessweite Desktopzulassung und deren verzögerte Rendererübergaben besitzen die unter R9 dokumentierten offenen Grenzen. Die ursprüngliche pauschale 4K-Sperre wurde durch die größere v2-Klasse beseitigt; R9 bleibt wegen seiner konkret genannten Übergänge offen.

**Vertragsformen:** `FrameContract::validate` und `PacketBudget::for_contract(Some(...))` erzwingen die v2-Rechnung. Bloße Quelllimits, `RESOURCE`, ausdrücklich unversionierte Sitzungen und Workeranfragen ohne v2-Vertrag behalten ihre bisherigen acht Bildäquivalente beziehungsweise Codec-/RAW-Grenzen. Die numerische Gleichheit von Limits wird nicht als Legacykennzeichen verwendet. Der native Quellcapturevertrag bleibt vom engeren übertragenen Bildvertrag getrennt; die v2-Viewergrenze ist keine nachträgliche Verkleinerung eines separat zugelassenen RAW-Quellcaptures.

**Gemeinsame Rechnung:** Mit `W=working_bytes`, `I=maximale Bildbytes`, `N=konservative Tilezahl`, `H=1099`, `F=floor(I/255)+512N+1024` und `E=I+F+R` verlangt der Code `I≤W/24` sowie `20I+32(R+H)+(E−I)≤W`. `R` wird aus dem Restbudget bestimmt, auf 16 MiB begrenzt und muss einen vollständigen atomaren Lossless-Tile tragen; andernfalls scheitert die Zulassung. Die maximalen Record- und Framezahlen werden ebenfalls abgeleitet. Das statisch nachgerechnete Beispiel mit 512×512 als Seitenrahmen und verkleinertem Pixelangebot lautet:

| Größe | Bytezahl beziehungsweise Anzahl |
|---|---:|
| W | 8.388.608 Byte |
| max_pixels / I | 87.381 Pixel / 349.524 Byte |
| N / H | 39 Tiles / 1.099 Byte |
| R / E | 40.624 / 412.510 Byte |
| Pullbuffer | 349.556 Byte |
| Zulassungsformel | 8.388.602 Byte |
| Verbleibende gemeinsame Skalierreserve | 2.446.674 Byte |

Das ist eine Rechnung über den Quelltext, keine Speichermessung. Dreizehn Bildplätze decken native Tile-/Queuebesitzer (vier), Pullbuffer (einen), Decoderbilder (drei), Full-/Previewsnapshots (zwei), einen geposteten Snapshot sowie laufenden/nächsten Skalierquellframe (zwei) ab. Die 32 Recordplätze decken vier QUIC-Reader, vier PictureEventplätze, acht Relayplätze und die begrenzten Encode-/Writer-/Client-/Parser-/Kotlin-Zwischenkopien ab. Der Scaler reserviert seine tatsächlichen zusätzlichen Arrays, Floatzeilen und gehaltenen Ausgaben aus `W−13I−32(R+H)−(E−I)`; die zusätzlichen sieben Bildäquivalente sind eine Zulassungsreserve, keine Annahme, dass jedes Viewportziel in I passt.

**Nachgelesene Durchsetzung:**

- **QUIC:** Höchstens vier aktive Reader und vier PictureEventplätze. `read_frame_record_budgeted` prüft Einzelrecord, Anzahl und Framelängensumme vor der Payloadallokation. Die v1-/v2-Sitzungsbindung aus R1 bleibt erhalten.
- **Hostworker:** `WorkerReader` erhält vor jedem serialisierten `PRODUCE` sein Budgetticket. `try_send` beendet einen widersprüchlichen oder getrennten Ticketzustand über den vorhandenen Fehlerpfad, statt zu blockieren. `read_reference_payload` prüft nach dem festen 42-Byte-Präfix die innere Längengrenze vor der großen Allokation. `validate_encoded_budget` meldet zu große ehrliche Encoderausgaben als `EncoderFailed`, solange der Worker lebt; der vorhandene Tiles-Rückfall kann denselben Quellworker weiterverwenden. Framingfehler bleiben terminal.
- **Daemon-IPC:** Acht Relayplätze; zusätzlich sind aktiver Writer und wartender Producer begrenzt. `encode_relay_record` reserviert die vollständige Header-/v2-Erweiterungs-/Payloadlänge einmalig. `ScreenStream::next_event` bindet die Recordgrenze vor dem Read an `Accepted`; vor Accepted gibt es keine Recordfreigabe, `Some(None)` bezeichnet ausdrücklich Legacy. Nach dem Decode wird die unveränderte Vertragspaarung geprüft. `screen_view_run` verarbeitet Records synchron und fügt keine Recordqueue hinzu.
- **Pull und verlustfreie Frames:** Die v2-Pullqueue hat Byte- und Eintragsgrenzen ohne 64-MiB-Mindestanhebung; ihre Backpressure erlaubt dem Decoder, eine paketierte Lossless-Transaktion zu konsumieren. Nicht-hybride Tiles werden an ganzen Tilegrenzen passend paketiert; hybride Dirtyrecords, Begin, optionale Video-/Moveoperation und Commit passen in das abgeleitete Framebudget. Die vollständigen GOP-/Frame-IDs bleiben im JNI-Payload. JNI schreibt direkt in den begrenzten Zielbuffer. Nur Legacy behält seine vorherige begrenzte Wachstumsregel.
- **Android-Lebensdauer:** Native `accepted_fields` liefert einen typisierten `ScreenBufferBudget`; Kotlin prüft dieselbe Formel. `ScreenSession.decoderBuffers` hält den einmal zugelassenen v2-Direktbuffer über Surfacepausen und Full-/Previewwechsel. Der neue Decoder wartet auf das Ende des vorherigen Threads; ein gemeinsamer Buffer wird nicht auf sich selbst übertragen. Alle vier AVC-Empfangspfade prüfen sichtbare und codierte SPS-Maße gegen dieses Budget sowie höchstens 15 Pixel Codingpadding vor `configure` beziehungsweise `queueInputBuffer`.
- **CPU-Skalierung:** Ein `ScreenScaleWorker` je Sitzung besitzt höchstens eine laufende Arbeit und eine zusammengefasste nächste Anfrage. `ScreenResample.plan` berücksichtigt Quellarray, Zielarray plus Zielbitmap und eine feste Anzahl Floatzeilen. `reserve` berücksichtigt zusätzlich bestehende Ausgaben und beim ersten/größeren Ziel die nächste gleich große Ersetzung. Ausgabeleases enden erst nach Compose-Ersetzung oder Dispose; eine abgebrochene CPU-Arbeit gibt ihre Reserve erst nach `cancelAndJoin` frei. Native-Pixelzoom verwendet Alias beziehungsweise exakten Crop ohne Quellarray und Floatzeilen. Area- und Catmull-Rom-Gewichte sowie Abbruchprüfungen bleiben vorhanden. Nicht finanzierbare Ziele erzeugen vor ihren großen Allokationen den sichtbaren Ressourcenhinweis und keinen Darstellungsbeleg.

**Während dieses Nachreviews zusätzlich geschlossene Übergänge:** Die erste Transportkorrektur hatte noch drei Lücken: Die Kotlin-SPS-Prüfung kannte nur die globale 64-MiB-Grenze; der Pullbuffer wurde nach vollständig beendetem Decoder während einer Surfacepause nicht wiederverwendet; die CPU-Skalierung konnte Viewportpuffer außerhalb der Rechnung anlegen. Für den letzten Fall genügten 512×256 Quellpixel und eine normale Vergrößerung auf 1920×960: Zielarray und Zielbitmap benötigten zusammen 14.745.600 Byte. Die oben beschriebenen Budget-, Sitzungsbesitz- und Scaleränderungen schließen diese konkreten Auslöser. Das dabei separat gefundene GOP-/Fallbackrennen ist unter R4 dokumentiert und ebenfalls korrigiert.

**Primärquellen und praktische Grenze:** Android dokumentiert mögliche Direktbufferbelegung außerhalb des normalen Javaheaps; Wiederverwendung ersetzt deshalb eine unbelegte Annahme sofortiger GC-Freigabe. Die MediaCodec-API verlangt explizite Ressourcenfreigabe; Bitmap-`allocationByteCount` beschreibt die tatsächliche Pixelallokation genauer als bloße aktuelle Abmessungen. Diese Grenzen wurden gegen [ByteBuffer](https://developer.android.com/reference/java/nio/ByteBuffer), [MediaCodec.release](https://developer.android.com/reference/android/media/MediaCodec#release()) und [Bitmap.getAllocationByteCount](https://developer.android.com/reference/android/graphics/Bitmap#getAllocationByteCount()) am 2026-10-26 geprüft. Nicht belegte Speicherfreigabe durch GC, Compositor oder Treiber wird nicht in die Produktpufferrechnung eingerechnet. Die Summe mehrerer gleichzeitiger Sitzungen ist kein einzelnes W-Budget. Ein v2-Angebot von 512 MiB trägt bei 24 Bildäquivalenten höchstens 5.592.405 Pixel; Android darf bei begrenztem Heap weiterhin ein solches oder kleineres Angebot machen. Die früher globale 512-MiB-Obergrenze verletzte dagegen V7/P30 und wurde im R9-Fix für ausreichend zugelassene native Sitzungen erweitert. Unversionierte Übertragung und separat zugelassener RAW-Quellcapture behalten ihre beschriebenen Grenzen.

**Ursprünglicher Transportbefund:**

**Ort im ursprünglichen, inzwischen korrigierten Stand:** `frame_size_caps.rs::PacketBudget::for_limits` um Zeile 131; `wire_binary.rs::GopHeader::record_limit/read_frame_record` um Zeilen 142/294; `viewer_sink.rs::PullQueue::set_frame_limits/push` um Zeilen 88/106 und `ViewerSink::record` um Zeilen 359/382; Android `ScreenDecoder.kt::pullLarger` um Zeile 146 und `MAX_PULL_BYTES` um Zeile 435.

**Konkreter Auslöser:** Eine korrekt ausgehandelte v2-Sitzung erlaubt 512×512 Pixel, `max_pixels=262144`, `working_bytes=8 MiB` und H.264 ohne Hybridregionen. `FrameLimits::check` akzeptiert hierfür 1 MiB BGRA × acht Arbeitsbilder. Ein verbundener Host sendet einen 16-MiB-Record. Dies ist eine statische Eingabekonstruktion, kein beobachteter Geräteangriff. Die Allokation erfordert noch keine erfolgreiche H.264-Dekodierung.

**Nachgewiesener Pfad und gleichzeitig gehaltene Nutzpuffer:**

1. `GopHeader::record_limit` erlaubt weiterhin 16 MiB; `read_frame_record` allokiert sie vor der aggregierten Paketprüfung. `PacketBudget` addiert unabhängig vom 8-MiB-Angebot einen festen 16-MiB-Videoanteil plus Tile-/Headeranteile, sodass dieser Record auch die spätere Summenprüfung passiert.
2. Im nicht-hybriden Androidpfad leben `record.payload`, dessen `data`-Klon und das mit GOP-/Frame-ID versehene `stamped` im selben Aufruf: zusammen mindestens 48 MiB + 16 Byte Nutzdaten. Vorherige Queueelemente sind darin nicht enthalten. `PullQueue::push` hebt das errechnete kleinere Limit mit `.max(64*1024*1024)` auf mindestens 64 MiB an. Die Einsekundenregel begrenzt Alter, verhindert aber diesen sofortigen Einzelrecordpfad nicht.
3. Kotlin akzeptiert für `pullLarger` bis zur globalen Bildgrenze plus Header/Identitäten und kann für diesen Record einen Direktbuffer von 16 MiB + 32 Byte anlegen. Die vorherige Stagingallokation wird nur unerreichbar; eine sofortige Freigabe durch den Laufzeitallocator ist nicht belegt. Beim Konfigurieren folgt eine weitere `ByteArray(len)`-Kopie. Der JNI-Kapazitätscheck verhindert Speicherüberschreibung, erzwingt jedoch kein Sitzungsbudget.

**Wirkung:** Bereits ein Record kann die als v2-Ressourcenangebot verwendeten 8 MiB mehrfach überschreiten. Die vorherige R1-Vertragsprüfung bleibt korrekt, schließt diese nachgelagerte Allokationslücke aber nicht. Kein tatsächlicher OOM, keine Prozessspeichermessung und keine konkrete Gerätehäufigkeit nachgewiesen.

**Korrekturziel:** Recordgrenze vor der ersten Allokation, gesamte Framepakete, Queuebestand und Pull-/Codeczwischenkopien gemeinsam aus dem v2-Budget ableiten. Ein bloßes Entfernen von `.max(64 MiB)` reicht wegen des festen Videoanteils nicht. Die zulässige Video-/Tiles-Repräsentation und der tatsächliche gleichzeitige Pufferbesitz müssen zusammenpassen; der ausdrücklich unversionierte LEGACY-Vertrag bleibt erhalten. Die oben beschriebenen Korrekturen wurden anschließend nach Freigabe erneut gelesen.

### B6-R8 — Mittel, inzwischen statisch korrigiert: Android und Host halten ausgelassene Qualitätspräferenzen unterschiedlich

**Nachtrag 2026-10-26:** `QualityMode` hält jetzt Größe und Rate getrennt für Preview und Full. Ausgelassene Werte lassen die jeweilige Wahl erhalten; `host_mode` liest die zum aktiven Modus gehörende Größe. Der Wirekommentar dokumentiert dieselbe Bedeutung. Android hält für den Rückfall nur Modus, Schärfe und Losslesswunsch und sendet Quelle, Größe und fps ausdrücklich nicht erneut. Damit kann der Rückfall keine lokal aus Preview übernommene Rate oder alte Größe auf Full zurückspielen. Der aktuelle `ScreenSession`- und Hoststand wurde nach Freigabe erneut gelesen; kein Laufzeitnachweis.

**Ort:** `ScreenSession.kt::ScreenModeRequest`, `commands.setMode` und `hybridUnavailable` um Zeilen 220/247/405; `ScreenApi.kt::viewSetMode` um Zeile 245; `host_quality.rs::QualityMode::update` um Zeile 17; `host_mode.rs::set_quality_mode` um Zeile 21.

**Ursprünglicher Auslöser und Wirkung:** Android hielt nur einen `wantedMode.fps`, der Host dagegen `full_fps` und `preview_fps`. Die zulässige Vertragsfolge Full mit 30 fps → Preview mit 1 fps → Full ohne fps ließ beim Host 30 fps erhalten, speicherte in Android aber 1 fps als aktuellen Wunsch. Ein anschließender `hybridUnavailable` sendete ausdrücklich Full mit 1 fps und überschrieb dadurch die erhaltene Vollbildpräferenz. Die vorhandene Vorschaukachel öffnet selbst nur Tiles mit maximal 1/2 fps; diese konkrete Oberfläche ist deshalb kein belegter H.264-Auslöser der genannten Folge. Der Fehler ist an der gemeinsamen öffentlichen Sitzungs-/SET_MODE-Grenze belegt, ohne Geräteausführung.

Zusätzlich dokumentierte `ScreenApi.viewSetMode` ausgelassene Felder als Behalten und `ScreenSession` behielt ausgelassene Breite/Höhe. Der Host setzte mit `QualityMode::update` hingegen `size=None` und fiel auf Anzeige-/Previewstandard zurück. Ein Qualitätswechsel ohne neue Größe verwarf so eine zuvor ausdrücklich gewählte Größe; ein späterer Android-Lossless-Rückfall konnte die lokal noch gespeicherte alte Größe wieder einsetzen.

**Korrekturziel:** Die vollständige gewünschte Rate und Größe pro Modus eindeutig halten und an der API-/Hostgrenze dieselbe Bedeutung ausgelassener Felder verwenden. Ein Decoderfallback ändert nur den erforderlichen Codecmodus und darf keine Previewpräferenz in den Vollbildmodus übertragen. `sharpness`, gewolltes `lossless`, Quelle und tatsächlich ausgehandelter Codecrahmen bleiben erhalten.

## Während des Reviews korrigiert

- **P32, Prepared → pause:** `PlayerPage.ON_STOP` konnte nach erfolgreicher Vorbereitung ohne Start `MediaPlayer.pause()` auslösen. Beispiele sind gespeicherte Pausenabsicht und verweigerter Audiofokus. Android erklärt diesen Aufruf im Prepared-Zustand für ungültig. Der Hauptagent ergänzte in `MediaPlaybackState.kt` einen `started`-Marker, der nur nach erfolgreichem `start()` gesetzt und bei Pause/Release gelöscht wird. Der korrigierte Quellstand wurde erneut gelesen; die konkrete ungültige Transition wird verhindert. Kein Laufzeitnachweis. [MediaPlayer, Valid and invalid states](https://developer.android.com/reference/android/media/MediaPlayer).
- **Zwei CodecChoice-Testliterale:** In `native/src/screen/os/shared/codec_inventory.rs` fehlte zunächst `max_macroblocks_per_second`. Beide Literale enthalten im erneut gelesenen Stand `None`. Das bestätigt die Textkorrektur, keine Kompilierung der Tests.
- **Android-Arbeitsbudget:** Die zuletzt ergänzte `ScreenFrameLimits.workingBytes()` verwendet den freien Anteil bis zur maximalen Java-Heapgröße, begrenzt auf 512 MiB. `ScreenApi.kt`, `screen_view.rs`, `screen_host.rs` und `BridgeSource` reichen dieses Angebot weiter beziehungsweise begrenzen die FrameLimits damit. Das beseitigt die pauschale 512-MiB-Angebotsannahme bei kleinerem Java-Heap; die zugehörige Vertragsumgehung an der GOP-Grenze ist gemäß B6-R1 inzwischen statisch korrigiert. Eine vollständige reale Native-/Grafikspeicherbilanz ist damit nicht nachgewiesen.

## Statisch erhaltene Fähigkeiten

Diese Aussagen beschreiben gelesene Implementierungspfade, keine erfolgreiche Ausführung:

- **Widgetdrag:** Das vorhandene AppWidgetHostView bleibt während der Geste komponiert; erst eine vorhandene Bitmapvorschau blendet das Original aus. Snapshotgröße ist an die tatsächliche Rootfläche gebunden. Abbruch, Detach und Größenwechsel beenden den Drag. Der `ItemContext` wird im HomeScreen erinnert; die vermutete Abbruchschleife durch eine neue Kontextinstanz bei jedem Dragframe ist deshalb kein Befund.
- **Icons:** Wire-/Cachetypen binden PNG-Daten an Hash und App-/Peeridentität. Decoder prüfen Dimensionen und Größen. Linux rastert begrenzte SVGs lokal und verweigert externe Bildreferenzen. Windows-/Linux-/Androidadapter sowie die generischen Fallbackicons bleiben vorhanden; kein Remote-SVG wird als frei ladbarer Viewerinhalt übernommen.
- **Hybridreferenz:** Begin/Video/Moves/Dirty/Commit, Generation, Frame-ID und Basisframe bilden einen gemeinsamen Vertrag. Moves lesen aus der vorherigen Referenz; verlustfreie Regionen werden vor dem atomaren Commit angewandt. Fehler vergiften die Referenz und verlangen einen neuen vollständigen Schlüsselbildaufbau.
- **Größen-/Ratenverträge:** `FrameLimits`, `PacketBudget`, SPS-Prüfungen und Workergrenzen ersetzen mehrere pauschale Annahmen durch geprüfte Werte. Encoder-/Decoder-Makroblockraten werden für dasselbe ausgewählte Codecpaar begrenzt. FrameClock und StageRequest verwenden denselben tatsächlich gewählten fps-Wert. Reine fps-Begrenzung verkleinert die Quellpixel nicht.
- **Darstellung:** CPU-Skalierung läuft im begrenzten Worker; Pixelmaßstab umgeht Rekonstruktion. CPU/GPU verwenden Flächenintegration beim Verkleinern und kubische Rekonstruktion beim Vergrößern. Darstellungsbestätigungen kommen aus Rendercallbacks beziehungsweise Android-Frame-/Bitmapdarstellung, nicht allein aus dem Empfang. Die MFT-/OpenH264-Ausgabeidentität ist nach B6-R3 nun ebenfalls angebunden.
- **Sichtbarkeit und Eingabe:** Fokusverlust allein wird nicht als Verdeckung ausgegeben. Wayland ohne globale Verdeckungsdaten bleibt unknown. Captureaktivität wird serialisiert; Wiedereintritt verlangt eine neue Referenz und zugehörige Bestätigung. Innerhalb derselben Sitzung gelten die geprüften Epoch-/Referenzgrenzen; der Ersatz einer Previewsitzung besitzt die gesonderte Identitätslücke R10.
- **Vollbild:** Desktop bietet den dauerhaft sichtbaren lokalen Ausstieg, F11, Strg+Alt+Pause und Strg+Alt+Umschalt+F; der Umschaltpfad löst gehaltene Remoteeingaben. Android fängt die lokalen Hardwarekombinationen vor Remoteweitergabe ab und bietet den Ausstieg über die Bildschirmleiste. Systemreservierte Tastenkombinationen und tatsächliche Gestenzustellung sind nicht praktisch bestätigt.
- **P32:** MainActivity setzt keine `screenOrientation`; in den geprüften Medien-/Activitydateien existiert kein `requestedOrientation`-Schreibzugriff. Die Orientierung bleibt Systementscheidung. Bildzoom/Fokus sind pfadgebunden gespeichert; der Player hält Position und Wiedergabeabsicht getrennt von seiner austauschbaren Surface. Hintergrundwechsel bleibt pausiert bis zur ausdrücklichen Fortsetzung. Swipe, Tastaturnavigation, externe Appöffnung und formatbezogene Fehlermeldungen bleiben vorhanden. [Android Activity-Konfiguration](https://developer.android.com/guide/topics/manifest/activity-element#screen) beschreibt die Systementscheidung bei nicht vorgegebener Orientierung.

Die vermutete doppelte Hybridumhüllung wurde verworfen: Die konkret geprüften Windows-/Linux-/Worker-Backendmethoden öffnen den Rohencoder über `open_encoder_with_contract`; die Umhüllung erfolgt anschließend an der jeweiligen Produktionsgrenze.

### Integrationsnachweis des Nachreviews

| Übergang | Statischer Befund und Grenze |
|---|---|
| MFT/OpenH264 → `DecodedFrame` → Desktopdarstellung | Tatsächlicher Ausgabetoken, eingereichte Frame-/Captureidentität und vor dem Decode gespeicherte Epoche werden verglichen. Fehlende Identität wird nicht durch den aktuellen Eingang ersetzt. R3 geschlossen; keine Treiberausführung. |
| Android-Einzelinput → Rückfall | Optionaler Formathinweis und echte passende Ausgabe sind getrennt. Nicht passende zeitliche Betriebsart erzeugt typisiertes Feedback. Ein bekannter GOP bleibt auch nach Queue-Reset gültiger Anlass für den einmaligen sitzungsweiten Losslessfallback. R4 geschlossen; unterstützte Gerätemodelle nicht praktisch bestätigt. |
| Konkreter Hostencoder → Ready → Capture | Instanztoken, Projectionidentität und aktiver Epoch müssen nach dem Ready-Aufruf noch stimmen. Erster Hybridcapture bleibt bis dahin zurückgestellt. Kein beobachteter Coroutine-/MediaCodec-Lauf. |
| Native Queue → JNI → Kotlin-GOP | Jede neue native Ausgabe trägt `gop_id u64` und `frame_id u64` im Payload, auch bei v1-Wireeingang. Kotlin liest den ganzen GOP, vergleicht ihn ohne Low16-Verengung und verlangt das genau passende Streamereignis. Codecbytes beginnen bei gesetzten beiden Flags bei Offset 32. Alte Low16-Bits bleiben nur Hinweis; alte Testfixtures sind noch kein Produktionsfehler. |
| SET_MODE → PipelineActor → Qualität | Coalescing hält den vollständigen Modus; Full-/Previewgröße und -rate sind getrennt. Sharpnesskappung überschreibt den ursprünglichen Fullwunsch nicht; ein echter Encoderfehler bleibt trotz UIwechsel im Tilesmodus. Android-Fallback lässt Größe/Rate beim Host. R8 geschlossen. |
| Referenz-/Epochenverlust → Darstellung → Eingabe | Desktop hält den ursprünglichen Darstellungsbeleg über Decoderlatenz. Android prüft Sitzung, GOP, Epoche, Sichtbarkeitsfloor und Familienlayout; MediaCodec-Surfacebelege kommen über PTS-gebundene Rendercallbacks, Bitmapbelege über den Darstellungsweg. Der native Host verlangt einen in der aktuellen Referenz tatsächlich gesendeten Frame. Keine Aussage über physisches Display-Scanout. |
| Rückfallgrund → Oberfläche | Native `notice` erscheint in Viewerstatus beziehungsweise Kachelhinweis. Android `codecFallback` wird in der Bildschirmleiste dargestellt. Decoder- und Hostencoderfallback bleiben vom terminalen Fehlerzustand getrennt. |
| v2-Arbeitsbudget → Record → Queue → Pull → CPU-Skalierung | R7 statisch geschlossen: gemeinsame Vorallokationsgrenzen, erhaltene Legacyform, einmaliger Sitzungsbuffer und geleaste Skalierausgaben. Die Aussage betrifft lebende Produktpuffer, keine Gesamt-RSS oder Geräteabnahme. |
| Ressourcenangebot → volle 4K-Quelle → verlustfreie Korrektur | Größere v2-Klasse, getrennte Capturegrenze und alte Hello-Gegenstellen sind angebunden; 768 MiB tragen rechnerisch 4K mit unveränderter Besitzerformel. R9 bleibt wegen der fünf konkret dokumentierten Zulassungs-, Besitzer- und Snapshotlücken offen. |
| Preview → tatsächliche Darstellung → Eingabe nach Wiederverbindung | Der zyklische Hidden→Visible-Ausgangsfehler ist innerhalb derselben Sitzung korrigiert. R10 bleibt offen, weil ein altes Skalierergebnis nach Sitzungsersatz mit wiederholten lokalen Identitätswerten bestätigt werden kann. |

Der nach Freigabe erneut gelesene Windows-/Workerübergang erhält die Bildbindung: DXGI-Rotation und Reopen erhöhen die Quellgeneration; ein Generationswechsel während `capture` verwirft das erhaltene Ergebnis und den Encoder. `WorkerReply::Reference` transportiert die Generation zurück, der Client verweigert einen Rückschritt. `send_bound_input` erhält die beim Aufruf bestätigte Generation bis zur Prüfung im ursprünglichen Worker. `HostSession::produce` sendet eine Fallbackdiagnose auch dann, wenn kein neuer Streamingstatus vorliegt. Die konkrete Windows-Eingabegeometrieprüfung außerhalb des Manifests bleibt Gegenstand des gesonderten B7-Reviews.

## Ungeprüfte Grenzen und offene Punkte

1. **Keine Ausführung:** Keine Aussage über erfolgreichen Rust-/Kotlin-/WGSL-Build, Hardwarecodecverfügbarkeit, echte GUIgesten, tatsächlich dargestellte Pixel, Netzleistung, Remote-CI-Erfolg oder Releasezustand. Keine Test-/Buildfreigabe wird erteilt.
2. **Codec-/Grafikgeräte:** MFT-/MediaCodec-Latenz, konkrete Profile/Level, Treiberpitches, Shaderausführung, Surface-Neuerzeugung und reales Speichermaximum benötigen den vorgesehenen gemeinsamen Nachweis außerhalb dieses Reviews. Das Android-Heapangebot ist keine Messung des freien Grafik- oder Nativeheaps.
3. **P32-Gerätefälle:** Systemrotation ein/aus, Sensorrotation, Fenstergrößenwechsel, Prozessneustart, langsame Vorbereitung, Audiofokuswechsel und Zurücknavigation wurden statisch betrachtet, nicht am Gerät ausgeführt.
4. **JNI-Direktgrenze nach Freigabe geschlossen:** `native/android-bridge/src/screen_jni.rs` wurde nach ausdrücklicher Scopeergänzung gelesen. Negative Offsets/Längen werden durch geprüfte Umwandlung verworfen; Addition, DirectBufferkapazität, Nulladresse und `isize::MAX` begrenzen die Slices. Die Slicelebensdauer bleibt innerhalb des JNI-Aufrufs. Die geprüften Aufrufer halten MediaCodec-/DirectBuffer bis zur Rückkehr. `smart_explorer_android` bindet laut Cargo.lock JNI 0.22.4. Die aktuelle [Env DirectBuffer-API](https://docs.rs/jni/latest/jni/struct.Env.html#method.get_direct_buffer_address) dokumentiert Adresse und Kapazität passend zu diesen Aufrufen; der direkte Abruf der versionsgebundenen Dokumentations-URL war nicht verfügbar. Keine Geräteausführung oder allgemeine JNI-Auditaussage.
5. **P4-Integration:** In den freigegebenen Dateien wurden Capture-Layoutrevision und Decoderlayoutgate als B6-Integrationsgrenze betrachtet. P4-OSfensteroperationen und P5-Appstart sind nicht abgenommen. Änderungen, die nach dem letzten Read im gemeinsamen Arbeitsbaum eintreffen, benötigen einen gezielten Nachtrag.
6. **Graph:** Kein Graphinhalt gehörte zum delegierten Dateimanifest; kein Graphread oder Graphneubau wurde ausgeführt. Der Review benutzt direkte Quellpfade und Primärquellen.
7. **Quellenstand:** Die Herstellerseiten des Erstreviews wurden am 2026-10-25 abgerufen; die Quellen zu R3, R4, R7 und R9 wurden am 2026-10-26 erneut beziehungsweise zusätzlich gelesen. Der Arbeitsbaum bindet Windows-Rust 0.58, eframe/egui-wgpu 0.29.1, x11rb 0.13.2, OpenH264 0.9.9 und resvg 0.49.0. Die Android-Gradlekonfiguration gehört nicht zum freigegebenen Manifest; die genaue gebundene Compose-/SDKversion ist in diesem Review deshalb nicht unabhängig bestätigt. Ein erster Abruf der Android-Runtime-Referenz scheiterte; der spätere Abruf im R9-Nachtrag gelang und belegt die dort beschriebenen Javaheap-Semantiken.
8. **Offene Anforderung V7/P30:** Die größere native Klasse lässt 4K rechnerisch zu und die Quellenfixture erhält die 4K-Geometrie bei der 2-Mbit/s-Qualitätspolitik. Noch nicht geschlossen sind die fünf R9-Übergänge und die R10-Sitzungsidentität. Reale Ressourcenfakten der Zielgeräte, Codec-/Grafikgrundlast und tatsächlicher 2-Mbit/s-Netzaufbau wurden nicht gemessen. Die 25%-Quote ist Produktpolitik, kein Plattformgesetz; vorhandene Zusagen schützen nicht vor externem Speicherdruck.

## Datei- und Aktionsnachweis

### Erstreview vom 2026-10-25

**Angelegt/geändert durch diesen Reviewer:** ausschließlich `docs/plaene/2026-10-24-batches/review-b6.md`. Kein Produktcode geändert.

**Zusätzlich gelesen:** `/tmp/smartexplorer-review-b6-scope.json`, `/root/.agents/skills/arbeitsweise/SKILL.md`, `/root/.codex/skills/arbeitsweise/SKILL.md`; zentrale Arbeitsweise Revision 337 über MCP geladen.

Die folgende Liste nennt die inhaltlich gelesenen Dateien; bei umfangreichen Dateien waren es gezielte Abschnitte, Diffstellen oder Suchtreffer. Sie behauptet keine vollständige Zeilenabnahme aller 204 Manifestdateien. Die übrigen Manifestdateien gingen nur in den begrenzten Diffmetadatenvergleich ein.

- `AGENTS.md`
- `android/app/src/main/AndroidManifest.xml`
- `android/app/src/main/java/app/smartexplorer/android/MainActivity.kt`
- `android/app/src/main/java/app/smartexplorer/android/api/ScreenApi.kt`
- `android/app/src/main/java/app/smartexplorer/android/launcher/ui/CellGrid.kt`
- `android/app/src/main/java/app/smartexplorer/android/launcher/ui/DragLayer.kt`
- `android/app/src/main/java/app/smartexplorer/android/launcher/ui/HomeScreen.kt`
- `android/app/src/main/java/app/smartexplorer/android/launcher/widgets/WidgetPreview.kt`
- `android/app/src/main/java/app/smartexplorer/android/launcher/widgets/WidgetView.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenEncoder.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenFrameLimits.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenHostControl.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenHostNotifier.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenProjectionFrames.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenProjectionIdentity.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenDecoder.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenHybridDecoder.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenScaling.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenSession.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenViewScreen.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenVisibility.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/viewer/ImagePage.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/viewer/MediaGeometry.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/viewer/MediaPlaybackState.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/viewer/MediaViewer.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/viewer/PlayerPage.kt`
- `docs/ARCHITEKTUR.md`
- `docs/plaene/2026-10-24-batches/batches.md`
- `docs/plaene/2026-10-24-batches/recherche.md`
- `docs/plaene/2026-10-24-batches/umsetzung.md`
- `docs/plaene/2026-10-24-batches/vertraege.md`
- `native/Cargo.lock`
- `native/Cargo.toml`
- `native/android-bridge/src/screen_jni.rs`
- `native/src/app/core/desktop_surface_tile_ui.rs`
- `native/src/app/core/screen_decode_thread.rs`
- `native/src/app/core/screen_pool.rs`
- `native/src/app/core/screen_presented.rs`
- `native/src/app/core/screen_render_caps.rs`
- `native/src/app/core/screen_scaling.rs`
- `native/src/app/core/screen_texture_cpu.rs`
- `native/src/app/core/screen_tile_face.rs`
- `native/src/app/core/screen_tile_grid.rs`
- `native/src/app/core/screen_view_widget.rs`
- `native/src/app/core/screen_viewer_app.rs`
- `native/src/app/core/screen_viewer_view.rs`
- `native/src/app/core/screen_visibility.rs`
- `native/src/app/core/screen_wgpu_yuv.rs`
- `native/src/app/core/screen_wgpu_yuv.wgsl`
- `native/src/app/mod.rs`
- `native/src/app/os/linux_os/screen_visibility.rs`
- `native/src/app/os/shared/screen_visibility.rs`
- `native/src/app/os/windows/screen_visibility.rs`
- `native/src/app/screen_modules.rs`
- `native/src/mobile/os/shared/domains/screen_host.rs`
- `native/src/mobile/os/shared/domains/screen_view.rs`
- `native/src/mobile/os/shared/domains/screen_view_json.rs`
- `native/src/mobile/os/shared/domains/screen_view_presented.rs`
- `native/src/mobile/os/shared/domains/screen_view_run.rs`
- `native/src/screen/core/app_icon.rs`
- `native/src/screen/core/codec_registry.rs`
- `native/src/screen/core/control_sender.rs`
- `native/src/screen/core/frame_budget.rs`
- `native/src/screen/core/frame_protocol.rs`
- `native/src/screen/core/frame_reference.rs`
- `native/src/screen/core/frame_size_caps.rs`
- `native/src/screen/core/gop_stream.rs`
- `native/src/screen/core/host_backend.rs`
- `native/src/screen/core/host_capture.rs`
- `native/src/screen/core/host_capture_pause.rs`
- `native/src/screen/core/host_frame_send.rs`
- `native/src/screen/core/host_hybrid.rs`
- `native/src/screen/core/host_mode.rs`
- `native/src/screen/core/host_pipeline.rs`
- `native/src/screen/core/host_reference.rs`
- `native/src/screen/core/host_stage.rs`
- `native/src/screen/core/host_visibility.rs`
- `native/src/screen/core/icon_cache.rs`
- `native/src/screen/core/icon_host.rs`
- `native/src/screen/core/negotiate.rs`
- `native/src/screen/core/pacing.rs`
- `native/src/screen/core/pixel_convert.rs`
- `native/src/screen/core/rate_control.rs`
- `native/src/screen/core/sender_budget.rs`
- `native/src/screen/core/viewer_client.rs`
- `native/src/screen/core/viewer_reconstruct.rs`
- `native/src/screen/core/visibility.rs`
- `native/src/screen/core/wire_binary.rs`
- `native/src/screen/core/worker_frame_wire.rs`
- `native/src/screen/core/worker_wire.rs`
- `native/src/screen/os/android/bridge_backend.rs`
- `native/src/screen/os/android/bridge_channels.rs`
- `native/src/screen/os/android/bridge_source.rs`
- `native/src/screen/os/android/viewer_sink.rs`
- `native/src/screen/os/linux_os/app_icons.rs`
- `native/src/screen/os/linux_os/host_backend.rs`
- `native/src/screen/os/linux_os/pipewire_identity.rs`
- `native/src/screen/os/linux_os/pipewire_pause.rs`
- `native/src/screen/os/linux_os/pipewire_stream.rs`
- `native/src/screen/os/linux_os/portal_session.rs`
- `native/src/screen/os/linux_os/portal_source_binding.rs`
- `native/src/screen/os/native_ffi/openh264_codec.rs`
- `native/src/screen/os/shared/app_icons.rs`
- `native/src/screen/os/shared/codec_inventory.rs`
- `native/src/screen/os/shared/host_worker_backend.rs`
- `native/src/screen/os/shared/host_worker_client.rs`
- `native/src/screen/os/shared/host_worker_main.rs`
- `native/src/screen/os/windows/app_icons.rs`
- `native/src/screen/os/windows/capture_dxgi.rs`
- `native/src/screen/os/windows/capture_source.rs`
- `native/src/screen/os/windows/d3d_device.rs`
- `native/src/screen/os/windows/host_backend.rs`
- `native/src/screen/os/windows/mf_common.rs`
- `native/src/screen/os/windows/mf_decoder.rs`
- `native/src/screen/os/windows/mf_encoder_sync.rs`
- `native/src/screen/os/windows/mf_media.rs`

Nachträgliche gezielte Reads zur Icon- und Vollbildgrenze:

- `android/app/src/main/java/app/smartexplorer/android/screen/AppIcons.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenIcons.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenViewBars.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenTouch.kt`
- `native/src/app/core/screen_icons.rs`
- `native/src/app/core/screen_icon_requests.rs`
- `native/src/app/core/screen_input_keys.rs`
- `native/src/app/core/screen_input_map.rs`

### Nachreview vom 2026-10-26

**Geändert:** ausschließlich dieser Bericht. Keine Produktdatei, Testdatei, Build-/Releasekonfiguration oder Graphdatei geändert; kein Commit, Push, Build, Compiler, Test, Server, Paketinstallations- oder Releaseaufruf. `git status --short` und `git log -1 --oneline` dienten dem Dokumentationskontext; aus den Statuspfaden wurde keine Leseerlaubnis abgeleitet.

**Zusätzliche Grundlagen:** das exakte Manifest `/tmp/smartexplorer-review-b6-final-scope.json`; die Arbeitsweise-Skilldateien unter `/root/.agents/skills/arbeitsweise/SKILL.md` und `/root/.codex/skills/arbeitsweise/SKILL.md`; der Graphify-Skill unter `/root/.codex/skills/graphify/SKILL.md`; zentrale Arbeitsweise Revision 337. Kein Graphinhalt gehört zum freigegebenen Manifest. Der Nachreview benutzt deshalb direkte Quellpfade. Hersteller-/Bibliothekslinks in R3, R4 und R7 wurden am 2026-10-26 abgerufen. Ein erster docs.rs-Abruf des OpenH264-Quelltexts war nicht verfügbar; der versionsgebundene Herstellerrepositorypfad lieferte den benötigten Quelltext.

**Inhaltlich gelesene Repositorydateien:** vollständige Dateien oder gezielte Abschnitte/Suchtreffer, keine Behauptung einer vollständigen Zeilenabnahme aller Manifestdateien:

- `docs/ARCHITEKTUR.md`
- `docs/oberflaechen/bildschirm.md`
- `docs/plaene/2026-10-24-batches/batches.md`
- `docs/plaene/2026-10-24-batches/recherche.md`
- `docs/plaene/2026-10-24-batches/review-b6.md`
- `docs/plaene/2026-10-24-batches/umsetzung.md`
- `docs/plaene/2026-10-24-batches/vertraege.md`
- `native/Cargo.toml`
- `native/Cargo.lock`
- `native/android-bridge/src/screen_jni.rs`
- `native/src/app/core/screen_decode_thread.rs`
- `native/src/app/core/screen_presented.rs`
- `native/src/app/core/screen_session.rs`
- `native/src/app/core/screen_tile_face.rs`
- `native/src/app/core/screen_view_widget.rs`
- `native/src/app/core/screen_viewer_view.rs`
- `native/src/mobile/os/shared/domains/screen_view.rs`
- `native/src/mobile/os/shared/domains/screen_view_presented.rs`
- `native/src/mobile/os/shared/domains/screen_view_run.rs`
- `native/src/screen/core/codec_registry.rs`
- `native/src/screen/core/decoded_frame.rs`
- `native/src/screen/core/frame_budget.rs`
- `native/src/screen/core/frame_reference.rs`
- `native/src/screen/core/frame_size_caps.rs`
- `native/src/screen/core/gop_stream.rs`
- `native/src/screen/core/host_mode.rs`
- `native/src/screen/core/host_pipeline.rs`
- `native/src/screen/core/host_pipeline_actor.rs`
- `native/src/screen/core/host_quality.rs`
- `native/src/screen/core/host_reference.rs`
- `native/src/screen/core/host_work_control.rs`
- `native/src/screen/core/viewer_reconstruct.rs`
- `native/src/screen/core/wire_binary.rs`
- `native/src/screen/core/wire_control.rs`
- `native/src/screen/os/android/bridge_channels.rs`
- `native/src/screen/os/android/bridge_source.rs`
- `native/src/screen/os/android/viewer_sink.rs`
- `native/src/screen/os/native_ffi/openh264_codec.rs`
- `native/src/screen/os/windows/mf_decoder.rs`
- `android/app/src/main/java/app/smartexplorer/android/api/ScreenApi.kt`
- `android/app/src/main/java/app/smartexplorer/android/core/NativeBridge.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenCodecLatency.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenEncoder.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenFrameLimits.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenHostControl.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenHostNotifier.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenProjectionFrames.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenDecoder.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenHybridDecoder.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenPreviewTile.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenSession.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenViewBars.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenViewScreen.kt`

Nach Freigabe ergänzend gelesene Quellpfade:

- `native/src/screen/core/host_backend.rs`
- `native/src/screen/core/host_frame_send.rs`
- `native/src/screen/core/host_session.rs`
- `native/src/screen/core/worker_frame_wire.rs`
- `native/src/screen/os/shared/host_worker_backend.rs`
- `native/src/screen/os/shared/host_worker_client.rs`
- `native/src/screen/os/shared/host_worker_main.rs`
- `native/src/screen/os/windows/capture_dxgi.rs`
- `native/src/screen/os/windows/capture_source.rs`
- `native/src/screen/os/windows/mf_media.rs`

Im R7-Folgeauftrag zusätzlich gelesen:

- `native/src/daemon/os/shared/ipc_protocol_screen.rs`
- `native/src/daemon/os/shared/screen_ipc.rs`
- `native/src/daemon/os/shared/screen_ipc_client.rs`
- `native/src/screen/core/viewer_client.rs`
- `native/src/screen/os/shared/host_worker_reader.rs`

Weitere im abschließenden R7-Read gelesene Dateien:

- `native/src/mobile/os/shared/domains/screen_view_json.rs`
- `native/src/screen/core/frame_protocol.rs`
- `native/src/screen/core/host_hybrid.rs`
- `native/src/screen/core/tiles_codec.rs`
- `native/src/screen/core/worker_wire.rs`
- `native/src/screen/os/android/bridge_backend.rs`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenPullBuffers.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenScaling.kt`

Für den abschließenden Scalerstand zusätzlich gelesen:

- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenScaleWorker.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenResample.kt`

Für den V7-/4K-Folgeauftrag zusätzlich beziehungsweise erneut außerhalb der vorstehenden Nachreviewlisten gelesen:

- `native/src/app/core/screen_render_caps.rs` — ausdrücklich zusätzlich freigegeben.
- `native/src/app/core/screen_pool.rs` — ausdrücklich zusätzlich freigegeben.
- `native/src/mobile/os/shared/domains/screen_host.rs`
- `native/src/screen/core/negotiate.rs`
- `native/src/screen/core/wire.rs`
- `native/src/screen/core/wire_hello.rs`
- `native/src/screen/os/windows/d3d_device.rs`

Die bereits aufgeführten Budget-, Pipeline-, Worker-, IPC-, Android- und Vertragsdateien wurden für R9 gezielt erneut gelesen. Die Rechnung zu 512/768/864 MiB wurde als reine Ganzzahl-/Geometrieauswertung nachvollzogen; kein Produktcode, Compiler oder Test wurde ausgeführt. `crate::transfer::available_memory()` ist eine Nachrichtengrundlage des Hauptagenten, kein eigener Dateiread. Änderungen dieses Folgeauftrags betreffen ausschließlich diesen Bericht.

### Abschließender R9-/R10-Durchgang vom 2026-10-26

**Grundlage und Änderungen:** `/tmp/smartexplorer-r9-review-brief.txt`, `/tmp/smartexplorer-r9-scope.json` und die genannten öffentlichen Primärquellen wurden gelesen. Ausschließlich `docs/plaene/2026-10-24-batches/review-b6.md` wurde durch diesen Reviewer geändert. Die zuvor nur vom Hauptagenten benannten Transfer-Speicherhelper wurden in diesem Durchgang gezielt über ihre Deklarations-/Suchtreffer betrachtet; der neue Produktpfad verwendet die eigenständigen Screen-OSadapter. Kein Produktcode, keine Fixture, kein Graph und keine Releasekonfiguration wurden geändert. Keine Build-, Compiler-, Formatter-, Test-, Server-, Installations-, Commit-, Push- oder Releaseausführung. Dokumentationskontrolle: gezielte Textsuche, `git status --short`, `git log -1 --oneline` und `git diff --check` für diesen Bericht.

**Gelesene Repositorydateien in diesem Durchgang:** vollständige Dateien oder gezielte Abschnitte/Suchtreffer; keine Behauptung einer vollständigen Zeilenprüfung jeder Manifestdatei.

```text
android/app/src/main/java/app/smartexplorer/android/api/ScreenApi.kt
android/app/src/main/java/app/smartexplorer/android/screen/ScreenFrameLimits.kt
docs/plaene/2026-10-24-batches/review-b6.md
docs/plaene/2026-10-24-batches/vertraege.md
native/review-task-manifest.json
native/src/app/core/batches_preview_task_tests.rs
native/src/app/core/login_viewer_image.rs
native/src/app/core/screen_decode_thread.rs
native/src/app/core/screen_pool.rs
native/src/app/core/screen_presented.rs
native/src/app/core/screen_render_caps.rs
native/src/app/core/screen_scaling.rs
native/src/app/core/screen_session.rs
native/src/app/core/screen_texture_cpu.rs
native/src/app/core/screen_tile_face.rs
native/src/app/core/screen_tile_grid.rs
native/src/app/core/screen_view_widget.rs
native/src/app/core/screen_viewer_app.rs
native/src/app/core/screen_viewer_view.rs
native/src/app/core/screen_visibility.rs
native/src/app/core/screen_wgpu_yuv.rs
native/src/daemon/os/shared/ipc_protocol_screen.rs
native/src/daemon/os/shared/screen_ipc.rs
native/src/daemon/os/shared/screen_ipc_client.rs
native/src/screen/core/batches_4k_task_tests.rs
native/src/screen/core/batches_frame_budget_task_tests.rs
native/src/screen/core/frame_budget.rs
native/src/screen/core/frame_memory.rs
native/src/screen/core/frame_memory_tests.rs
native/src/screen/core/frame_size_caps.rs
native/src/screen/core/gop_stream.rs
native/src/screen/core/host_backend.rs
native/src/screen/core/host_capture.rs
native/src/screen/core/host_mode.rs
native/src/screen/core/host_pipeline.rs
native/src/screen/core/host_pipeline_actor.rs
native/src/screen/core/host_server.rs
native/src/screen/core/host_session.rs
native/src/screen/core/viewer_client.rs
native/src/screen/core/visibility.rs
native/src/screen/core/wire_binary.rs
native/src/screen/core/wire_control.rs
native/src/screen/core/wire_hello.rs
native/src/screen/core/wire_hello_tests.rs
native/src/screen/core/wire_tests.rs
native/src/screen/os/android/bridge_backend.rs
native/src/screen/os/android/bridge_source.rs
native/src/screen/os/android/session_memory.rs
native/src/screen/os/linux_os/host_backend.rs
native/src/screen/os/linux_os/session_memory.rs
native/src/screen/os/mod.rs
native/src/screen/os/shared/batches_memory_task_tests.rs
native/src/screen/os/shared/host_worker_backend.rs
native/src/screen/os/shared/host_worker_client.rs
native/src/screen/os/shared/host_worker_main.rs
native/src/screen/os/shared/session_memory.rs
native/src/screen/os/windows/host_backend.rs
native/src/screen/os/windows/session_memory.rs
native/src/share/core/node_screen_link.rs
native/src/transfer/mod.rs
native/src/transfer/os/shared/memory.rs
native/src/transfer/os/unix.rs
native/src/transfer/os/windows.rs
```

Zusätzlich wurde nach ausdrücklicher Freigabe `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/epaint-0.29.1/src/textures.rs` an `alloc`, `set`, `free` und `take_delta` gelesen. Ergänzend wurden die öffentlichen 0.29.1-Quellen von [egui-WGPU Renderer](https://raw.githubusercontent.com/emilk/egui/0.29.1/crates/egui-wgpu/src/renderer.rs) und [egui-Glow Painter](https://raw.githubusercontent.com/emilk/egui/0.29.1/crates/egui_glow/src/painter.rs) am 2026-10-26 an ihren Texturupdate-/Freigabepfaden gelesen. Daraus wird keine feste GPU-Freigabefrist abgeleitet. Die zuletzt zusätzlich freigegebenen `family_viewer.rs`, `frame_update.rs` und `app/core/mod.rs` wurden nicht als Anlass einer zusätzlichen breiten Prüfung benutzt.

**Entscheidung und offene Übergabe:** R9 bleibt mit R9-A bis R9-E offen; R10 bleibt wegen der fehlenden Sitzungsidentität offen. Der 4K-Geometrieanschluss sowie der ursprüngliche Hidden→Visible-Kreislauf innerhalb einer Sitzung sind statisch korrigiert. Die inzwischen durch den Hauptagenten begonnenen Nicht-Rendererkorrekturen und der getrennte Renderer-Retiremententwurf sind keine vorweggenommene Schließung. Unbekannt bleiben die tatsächlichen Zielgerätebudgets, reale Renderer-/Codecfreigabe und der ausgeführte Ende-zu-Ende-Nachweis bei 2 Mbit/s. Der beauftragte statische Durchgang ist mit dieser Befundübergabe abgeschlossen.
