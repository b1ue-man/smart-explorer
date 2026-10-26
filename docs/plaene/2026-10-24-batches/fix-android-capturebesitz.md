# Android-Capturebesitz: Behebung A1–A3

Stand: 2026-10-26. Ergebnisauftrag des Main ist die vollständige Behebung der drei
belegten Befunde aus [review-android-bildfixtures.md](review-android-bildfixtures.md).
Auflistung, Plan, Umsetzung und anschließende Anti-Regressionsreview stammen von
demselben Fixautor. Dieser Bericht behauptet keinen ausgeführten Laufzeitnachweis.

## Ziel und Erstplan aus dem Auftrag

Beauftragt sind: native Claimübernahme und Aliasübernahme bei Konstruktorfehlern
zurückrollen; die vollständige Encoderkonstruktion bis zur Drainübernahme absichern;
Synthetic-Abschluss ohne Listenwachstum und Listenkopie ermöglichen. Jeder reale
Besitzer muss bis zum tatsächlichen Ende gebunden bleiben. Synthetic-Testbild,
Projection, Accessibility-Stills, Codecverhandlung, Bildausgabe und Fehlercallbacks
bleiben erhalten.

Die zunächst sechs zulässigen Schreibpfade stehen im Manifest
`/tmp/smartexplorer-android-capture-ownership-fix-scope.json`. Suite-/Manifestintegration,
Git, Graphpflege und Remoteausführung gehören Main. Der Fixautor führt keine lokalen
Builds, Compiler, Formatter, Tests, Server, Installationen oder Releasebefehle aus.
Main hat anschließend exakt `ScreenProjectionFrames.kt`, `ScreenControlService.kt`
und `ScreenCodecLatency.kt` als zusätzliche Produktdateien freigegeben, um die bei
der eigenen Kettenprüfung belegten vier Besitzlücken im selben Fixzyklus zu beheben.

Promptwissen: A1–A3 betreffen drei Übergaben einer zusammenhängenden Ressourcenbesitzkette.
Vor der Bestandslektüre fehlen die konkreten Konstruktorgrenzen, Aufrufer und
Veröffentlichungszeitpunkte. Diese Angaben sind aus den zugewiesenen Quellen klärbar;
eine zusätzliche Nutzerangabe ist für diesen Fix nicht erforderlich.

Erstplan: die Produktbesitzer und direkten Aufrufer bis zur nativen Arenagrenze
lesen; aus diesen Verträgen den Umbau planen; Thread-/Allokationsverträge anhand
aktueller Primärquellen prüfen; drei Korrekturen gemeinsam implementieren und danach
als Autor die verketteten Pfade erneut prüfen. Fehlerfälle werden für die eine
Remote-Suite vorbereitet.

## Fehlerhafte Endnutzerfunktionen und vollständige betroffene Kette

Die Auflistung umfasst die von A1–A3 betroffenen Bedienfähigkeiten und die im
zugewiesenen Scope tatsächlich gelesenen direkten sowie indirekten Besitzgrenzen.
Sie ist kein Anspruch einer vollständigen Inventur sämtlicher Android-Funktionen.

| Befund | Endnutzerfunktion und Fehlverhalten | Direkte und indirekte Programmstellen |
| --- | --- | --- |
| A1 | Android-Bildschirmfreigabe, Synthetic-Testbild und Accessibility-Standbilder können nach einer fehlgeschlagenen Besitzererzeugung innerhalb desselben Prozesses Bildspeicherzulassungen verlieren. | `ScreenCaptureBudget.claim` → `NativeBridge.screenCaptureClaim` → JNI `screenCaptureClaim` → `mobile::screen_capture_claim` → `CaptureSession.grant` → `capture_arena::claim` → `CaptureGrant::claim` / `CaptureSlice::drop`; Rückweg `ScreenCaptureLease` / `retain` / `close` → JNI-/Mobile-/Arena-`release`. Consumer: `ScreenEncoder`, `ScreenProjectionFrames` und `ScreenControlService.still`; Bufferconsumer: `ScreenCaptureBuffer`, `ScreenCaptureBitmap`, `ScreenStillSource`, Encoder-AU-/Header-/RGBA-Pfade und `ScreenCaptureQueue`. |
| A2 | Der Start eines Android-H.264-Bildschirmhosts oder Synthetic-Testbilds kann nach Codec-/Surfaceerwerb scheitern und die bereits erworbenen Ressourcen unerreichbar lassen. | `ScreenHostControl.applyConfig` und beide `ScreenTaskTest`-Encoderfälle → `ScreenEncoder`-Feldinitialisierung / `init` → `open` / Codec-Konfiguration / Ergebnis-Triple → Surfaceerzeugung / Codecstart → Drainthread-Konstruktion / Start → `drainLoop` / `deliver` / Fehlerhandler → `finishResources`; beendender Rückweg `release`, `retireEncoder`, `ScreenSyntheticRetirement` und `ScreenCaptureFixture.retireSynthetic`. |
| A3 | Das Beenden oder Ersetzen eines Synthetic-Testbilds sowie ein spontaner Encoderfehler können nach fehlgeschlagener Abschlussallokation hängen; ein bloßes Aliasende darf dabei nicht als Threadende gelten. | `ScreenEncoder.retainSurface` → `ScreenHostControl.applyConfig` beziehungsweise `ScreenCaptureFixture.source` → `SyntheticSurfaceSource.worker` / `loop` / `finally` → Abschlusslease → Surfacebesitzerregistrierung → `release` / `finishResources` → tatsächlicher Workerjoin; Teardown über `stop`, `awaitStopped`, `ScreenSyntheticRetirement`, Host-Pause / Konfigurationswechsel / Sitzungsende und Fixtureabschluss. |

Die native Generation bleibt über Resize und Kanalende durch lebende Arenaeinträge
gebunden (`CaptureSession.begin`, `retire`, `bridge_channels::set_phase`). Dieser Fix
ändert keine native Freigaberegel und führt keinen Arenasweep ein. Gleiche
Pixelpuffer behalten Aliasbesitz ohne Kopie. Fehlercallbacks und der vorhandene
Hybrid-/Projection-Tiles-Rückfall bleiben dieselben Aufruferverträge.

## Bestandsbefund und Planstufe 1

Der bisherige `claim` erzeugt nach nativer Registrierung einen Kotlinbesitzer ohne
Catch. `retain` erhöht vor der Aliaskonstruktion den Zähler. `tryBuffer` besitzt
bereits einen Catch über JNI-Puffer, Lease und Bufferwrapper; dieser Schutz bleibt.

Im Encoder liegen `surfaceOwners`, `surfaceClosers` und `pending` nach dem ersten
Claim, aber vor dem bisherigen `init`-Catch. Nach `open` liegen außerdem Name,
Admission und AU-Verwalter außerhalb einer durchgehenden Freigabegrenze. Die
Surface-/Start-Catches behandeln nur `RuntimeException`, der Threadstart keinen
Rollback. Auch das Ergebnis-`Triple` in `open` entsteht erst nach Codecerwerb.

Die bisherige Abschlusslease erweitert `ArrayList<Thread>` erst im Synthetic-
`finally`. Anschließend kopiert `finishResources` die Liste mit `toList`. Beide
Operationen können Speicher benötigen. Der anschließende tatsächliche Join ist
hingegen notwendig und darf nicht durch ein Zählernull ersetzt werden.

Planstufe 1: ein enges, Android-unabhängiges Ownershipmodul bündelt die fehlbaren
Übergaben, den einmaligen Konstruktor-/Draineintritt und vorab allokierte
Produzentenknoten. Der Encoder behält alle Ressourcen bis zur tatsächlichen
Drainübernahme in lokalen Rollbackreferenzen. Die Produzentenknoten bleiben bis zum
vollständigen Join erreichbar; `release` darf ihren echten Threadstatus weiterhin
prüfen.

## Primärquellen und erste Recherche

Abrufdatum aller folgenden Quellen: 2026-10-26.

- [Kotlin `thread`](https://kotlinlang.org/api/core/kotlin-stdlib/kotlin.concurrent/thread.html):
  `start` steuert den sofortigen Start. Der Fix trennt Threadkonstruktion und Start
  ausdrücklich, damit der Drainverweis vor Ausführung gesetzt ist.
- [Java 17 `Thread`](https://docs.oracle.com/en/java/javase/17/docs/api/java.base/java/lang/Thread.html):
  `join` wartet auf das tatsächliche Threadende. Eine vorzeitig geschlossene Lease
  belegt dieses Ende nicht.
- Die im Ausgangsreview gelesenen ART-/ArrayList-Quellen bleiben Bestandsbelege für
  fehlbare Threaderzeugung und Listenwachstum. Sie ersetzen keinen ausgeführten
  Android-Fehlerfall.

## Planstufe 2: konkrete Meilensteine und Akzeptanzsignale

| Meilenstein | Dateien / Abhängigkeit | Konkretes erwartetes Ergebnis für die gemeinsame Remote-Suite |
| --- | --- | --- |
| M1 Claim-/Aliasübernahme | `ScreenCaptureBudget.kt`, `ScreenCaptureOwnership.kt`, `ScreenCaptureBudgetTest.kt`; bestehende JNI-/Arenaverträge | Eine gescheiterte Besitzerfactory gibt den übernommenen Token genau einmal zurück. Eine gescheiterte Aliasfactory verändert den Referenzstand nicht; echte Callback-/Workeraliase behalten den Claim bis zum letzten Close. |
| M2 vollständiger Encoderstart | `ScreenEncoder.kt`, `ScreenCaptureOwnership.kt`, `ScreenEncoderOwnershipTest.kt`; M1 | Fehler vor Drainübernahme rollen alle erworbenen Ressourcen zurück. Gewinnt der echte Drain den Eintritt, kann der Konstruktor diese Ressourcen nicht freigeben. Ein später eintretender abgebrochener Drain benutzt keine bereits zurückgerollten Ressourcen. |
| M3 allokationsfreier Produzentenabschluss | `ScreenCaptureOwnership.kt`, `ScreenEncoder.kt`, `ScreenEncoderOwnershipTest.kt`; M1/M2 | Registrierung und Callbackwrapper entstehen vor Besitzübergabe. Close setzt nur vorhandene Referenzen/Zähler und meldet den Abschluss. Leaseende bei weiter lebendem Worker blockiert Freigabe; Join wartet auf seine tatsächliche Rückkehr. Konstruktorabbruch auf dem Erzeugerthread benötigt keinen Join dieses Erzeugers. |
| M4 Autorprüfung der Integrationen | dieselben sechs Schreibpfade; gelesene Host-/Source-/Callback-/JNI-Grenzen | Synthetic, Projection mit Surface, Hybrid-RGBA, Projection-Tiles und Accessibility-Stills behalten ihre vorhandenen Aufrufer und Besitzer. Bestehende Gerätfälle bleiben unverändert. Der Bericht trennt Quellprüfung, vorbereitete Fälle und tatsächlich ausgeführte Nachweise. |

### Zweite Recherche und endgültiger Plan

Der Vertrag benötigt keine Annahme darüber, welchen `Thread.State` ART nach einem
fehlgeschlagenen Start meldet. Ein vor dem ersten Claim erzeugter
Konstruktorbesitzer entscheidet unter einem Monitor genau einmal zwischen
Drainübernahme und Konstruktorabbruch. Ein nach dem Abbruch eintretender Thread
kehrt vor jedem Ressourcenzugriff zurück. Hat der Drain bereits übernommen,
führt ausschließlich sein bestehendes `finally` die Freigabe aus. Der Verweis
`drain` wird vor `start()` gesetzt.

Der Konstruktorbesitzer hält rohe Codec-/Surfacereferenzen und den vorhandenen
Claim, damit Admission-, AU-, Threadobjekt- und Threadstartfehler dieselbe
Rollbackgrenze besitzen. `open` gibt auch beim fehlgeschlagenen Ergebniswrapper
den Kandidatcodec zurück; nur `RuntimeException` behält den bestehenden
Codec-Fallback. Ein fehlgeschlagener Plattform-`release` erlaubt keine Freigabe
des zugehörigen Claims; dieser bleibt in diesem Fehlerpfad gebunden.

Die zweite Quellenprüfung der [Java-17-Thread-API](https://docs.oracle.com/en/java/javase/17/docs/api/java.base/java/lang/Thread.html)
bestätigt die getrennte Bedeutung von `isAlive` und Leaseabschluss. Ein
intrusiver, bei Registrierung erzeugter Knoten kann den schließenden Thread
später ohne Containeränderung aufnehmen. Der unveränderte Knotenkopf bleibt
während sämtlicher Joins erreichbar; `release` sieht dadurch weiterhin auch
bereits geschlossene, aber noch lebende Produzenten. Keine Listenkopie ist nötig.

[JLS 17 §15.9.4](https://docs.oracle.com/javase/specs/jls/se17/html/jls-15.html#jls-15.9.4)
und [Kotlin-Inlinefunktionen](https://kotlinlang.org/docs/inline-functions.html)
wurden am 2026-10-26 ergänzend für die Besitzerfactorygrenze gelesen. Der
Claimrollback umschließt die gesamte Besitzererzeugung. Der Aliaszähler wird erst
nach erfolgreicher Factory erhöht. Die inline Rollbackhülle führt im produktiven
Claimaufruf keinen zusätzlich zu erzeugenden Callbackbesitzer ein.

Damit sind M1–M4 in dieser Form der endgültige Implementierungsplan. Es werden
keine nativen Grenzen, Hostabläufe, Sourcezeichenschleifen, Codecangebote oder
Gerätassertionen geändert.

## Umsetzung und anschließende Autorprüfung

Die Umsetzung ist für A1–A7 einschließlich der im erweiterten Bereich belegten
Anfragezählungs-, Repeatreferenz- und Projection-Abschlussgrenzen abgeschlossen.
Die folgenden Aussagen sind Ergebnisse der anschließenden eigenen Quellprüfung.
Sie sind kein Compiler-, Geräte- oder Laufzeitpass.

| Meilenstein | Tatsächlich umgesetzte Entscheidung |
| --- | --- |
| M1 | `adoptCaptureOwner` umschließt die vollständige Leaseerzeugung nach nativer Registrierung. `ScreenCaptureReferences.retain` erzeugt zuerst den vollständigen Alias und publiziert danach den neuen Referenzstand. Der bestehende `tryBuffer`-Catch und die geschlossenen Einzelholder bleiben erhalten. |
| M2 | `ScreenCaptureConstruction` entsteht vor dem ersten Claim. Codecobjekte werden unmittelbar nach `createByCodecName` registriert, nicht erst nach dem Ergebnis-`Triple`. Die gesamte Initialisierung und beide Threadphasen liegen im `Throwable`-Catch. Der Drainverweis existiert vor `start`. Der Besitzentscheid verhindert Konstruktorfreigabe nach echter Drainübernahme und verhindert Ressourcenzugriffe eines verspäteten abgebrochenen Drains. Normale Codecablehnungen behalten ihren Fallback; Fehler der Kandidatfreigabe behalten die ursprüngliche Ursache. |
| M3 | `ScreenSurfaceOwners` allokiert Produzentenknoten und Abschlusslease vor der Übergabe. Der Workerabschluss schreibt nur den vorhandenen Threadverweis, dekrementiert und benachrichtigt. Der unveränderte Knotenkopf bleibt während aller tatsächlichen Joins sichtbar. `ArrayList.add`, `toList` und Iteratorerzeugung sind aus dieser Abschlussgrenze entfernt. |
| M4 | Die bestehenden Host-, Synthetic-, Projection-, Still-, Codec-, JNI- und Arenaaufrufer wurden gegen die geänderten Grenzen erneut gelesen. Die tatsächlichen H.264-/Barcode-/Still-Gerätfälle bleiben unverändert und für Mains gemeinsame Suite vorgesehen. |
| M5 | Der Capture-Konstruktorbesitzer hält Readerclaim, Reader und HandlerThread. Ein vor der Runtimeübernahme beendeter Worker bleibt beim Konstruktorrollback. Nach erfolgreicher Übergabe beendet das `finally` des bestehenden HandlerThreads Reader, Quellenpuffer und Claim nach der Callbackschleife. `close` erzeugt keinen Abschlusspost, meldet Erfolg erst nach Threadende und Ressourcenabschluss und erlaubt die Wiederholung eines fehlgeschlagenen Abschlusses nach wirklichem Threadende. Der Surfacegetter verweigert eine bereits beendete Quelle. |
| M6 | Die `AtomicBoolean`-Abschlussmarke entsteht vor `shotRequests++`. Im Screenshotcallback entsteht `ScreenCaptureBitmap` vor dem Löschen des lokalen Bitmapbesitzers und vor `delivered=true`. Ein Konstruktorfehler erreicht damit wieder die bestehenden Recycle-/Claim-/Request-`finally`-Pfade. |
| M7 | `ScreenCaptureSlot.allocate` hält den tatsächlichen Partialowner vor dem Viewaufbau und schließt ihn bei Initialisierungsfehlern. `transfer` gibt den Owner erst nach vollständiger `AccessUnit`-Erzeugung ab. Der nicht fragmentierte Codec-Ausgabepfad sowie die PTS-/Flag-/Maximalgrößenprüfung bleiben erhalten. |
| M8 | `ScreenCaptureInput` entsteht vor seinem RGBA-Retain und schließt einen konkreten Input idempotent. `ScreenCaptureInputs` publiziert, übernimmt und storniert ohne neue Container-/Boxingallokation. Ein Queueabbruch schließt keinen bereits von `push` übernommenen Input. `EncoderAdmission` wird bei Fehlern zurückgesetzt. Die Repeatreferenz `lastPixels` wird erst nach erfolgreichem `queueInputBuffer` ersetzt. |

Scheitert die Plattformzerstörung des Konstruktor-Produzenten, bleiben dessen
Surface und Claim konservativ gebunden. Ein solcher Fehler wird nicht als
erfolgreiches Rollback bezeichnet. Die ursprüngliche Konstruktionsausnahme bleibt
die Ursache; soweit eine weitere Ausnahmenreferenz noch gespeichert werden kann,
wird die Cleanupausnahme angehängt. Der Fix führt keinen unbestätigten Sweep und
keine pauschale Freigabe beim Kanalende ein.

### Eigene Anti-Regressionsreview nach der Umsetzung

| Geprüfter Vertrag | Quellbeleg und Ergebnis |
| --- | --- |
| Exakter letzter Claimholder | `ScreenCaptureLease.close` bleibt pro Holder atomar/idempotent; `retain` und der geteilte Referenzstand sind unter demselben Monitor serialisiert. Callback- und Workeraliase verzögern weiterhin dieselbe native Freigabe. Fehlgeschlagene Aliasfactorys erhöhen den Stand nicht. |
| Gescheiterter versus übernommener Start | Alle Ressourcenregistrierungen geschehen vor dem Drainstart. `enterRuntime` und `abort` haben einen einzigen synchronisierten Gewinner. Ein gestarteter, aber noch nicht eingetretener abgebrochener Drain kehrt ohne Codeczugriff zurück. Hat er übernommen, bleibt seine bestehende `finally`-Freigabe zuständig. |
| Echter Synthetic-Abschluss | Das Schließen der Abschlusslease setzt den konkreten schließenden Thread im vorhandenen Knoten. `beginRelease` prüft weiterhin `isAlive`, `awaitEnded` joined bis zum echten Ende und stellt einen Interrupt erst danach wieder her. Konstruktorabbruch auf dem Erzeugerthread joint diesen Erzeuger nicht. |
| Projectionstart und Callbackende | Handler-/Listenerfehler bleiben vor Runtimeübernahme beim Konstruktor. Der Runtime-Handler schließt seine Ressourcen erst nach Rückkehr der Callbackschleife. `close` kann einen Join-Timeout oder fehlgeschlagenes Readerende nicht als Erfolg melden. Die Quellenpuffer behalten ihre eigenen tatsächlichen Encoder-/Callbackaliase. |
| Screenshotcallback | Die native Hardwarezusage entsteht weiterhin vor dem Screenshot, die Softwarezusage vor der Kopie. Hardwarewrapper und Hardwarebuffer werden weiterhin vor ihrem Claim abgeschlossen. Eine erfolgreiche Bitmapübergabe bleibt bis zum bisherigen Consumer-`finally` gebunden; eine gescheiterte Wrappererzeugung bleibt beim Service. |
| Fragmentierte und vollständige AUs | Partial-Puffer, PTS und Flags behalten ihre bisherige Bedeutung; der Owner bleibt über View-/BufferInfo-/Ausgabedeskriptorfehler erreichbar. Der Ausgabecode hält ihn weiterhin bis `deliverComplete` tatsächlich zurückkehrt. |
| Hybrid-Reihenfolge und Wiederholung | Der Einzelinputvertrag ist unverändert: eine zweite ausstehende Hybrid-Eingabe wird nicht angenommen. PTS und Frame-ID bleiben gemeinsam gebunden. `take` überträgt den Besitzer ausschließlich an den Outputconsumer; ein konkurrierender Queuefehler kann ihn nicht schließen. Ein fehlgeschlagener Queueaufruf ändert `lastPixels` nicht. Der bestehende Repeatpfad, Keyframestart, Nativeflags und Packetaufbau bleiben erhalten. |
| Hostablösung und Quellenidentität | `ScreenHostControl.applyConfig`, `captureCurrent`, `retireEncoder`, `teardown`, Pausen- und Projection-Ready-Tokens bleiben unverändert. Alte Generationen und Encoderaliase bleiben durch ihre wirklichen Besitzer gebunden; der neue Slot ersetzt keine Source-/Session-/Epochprüfung. |
| Native Grenzen | JNI, `CaptureSession`, `capture_arena`, `CaptureGrant`, `CaptureSlice`, Kanalretirement und native Allokation wurden nicht geändert. Claims überleben weiterhin Resize/Kanalende bis zur tatsächlichen Freigabe. |
| Gemeinsame Codec-API | Main hat alle direkten `ScreenCodecLatency`-/`AccessUnits`-Verweise unter Android main/test/androidTest gezielt gesucht; diese Suchvollständigkeit ist Mains Evidenz. Die vier Trefferdateien `ScreenCodecLatency.kt`, `ScreenEncoder.kt`, `ScreenDecoder.kt` und `ScreenHybridDecoder.kt` wurden gelesen. Nur `ScreenEncoder` konstruiert `AccessUnits`; beide Decoder nutzen die unveränderten Konstanten, Hints und `Unavailable`. Beide Decoderdateien bleiben unverändert. |

Nach den genannten Korrekturen hat die eigene abschließende Quellprüfung keinen
weiteren konkreten Fehler in den geänderten Besitzübergaben belegt. Diese Aussage
ist auf die dokumentierte Lesefläche und Quellverträge begrenzt.

## Erweiterung aus der eigenen Kettenprüfung

Die Erweiterung ist am 2026-10-26 von Main mit drei zusätzlichen Schreibpfaden
autorisiert. Fehleraufnahme, Plan, Fix und anschließende Prüfung bleiben bei
demselben Fixautor.

| Befund | Endnutzerfunktion, Defekt und vollständige zusätzliche Kette |
| --- | --- |
| A4 | Projection-Tiles und Hybrid-Bildschirmfreigabe: `ScreenHostControl.applyConfig` / bestehender Projection-Gerätfall → `ScreenProjectionFrames` → Readerclaim / `ImageReader` → `HandlerThread` / `Handler` / Listenerkonstruktion → Callbackende / `close`. Bei einem Fehler nach Readererwerb fehlte das Konstruktionsrollback. Nach einem spontan beendeten Handler konnte der bisherige `handler.post`-Abschluss abgelehnt werden; dessen Rückgabe wurde ignoriert, und `close` konnte danach trotz lebendem Reader/Claim Erfolg melden. |
| A5 | Accessibility-Standbilder und Securestill: `ScreenStillSource.capture` / beide Still-Gerätfälle → `ScreenControlService.still` → Screenshotcallback → Hardwarebuffer / Hardwarebitmap / Softwarebitmap → `ScreenCaptureBitmap` → Callback / `finally`. `bitmap=null` und `delivered=true` standen vor der fehlbaren Wrapperkonstruktion. Zusätzlich wurde `shotRequests` vor der fehlbaren `AtomicBoolean`-Erzeugung erhöht; ein Erzeugungsfehler hinterließ eine Anfrage ohne Callback. |
| A6 | Fragmentierte H.264-Ausgaben: `ScreenEncoder.drainLoop` / `deliver` → `ScreenCodecLatency.AccessUnits.complete` → budgetierter Partialbuffer / ByteBuffer-View → BufferInfo / `AccessUnit` → `deliverComplete` / Consumer-`finally` / `clear`. Der Partialowner wurde erst nach der fehlbaren Viewkonstruktion gespeichert und vor der fehlbaren Ergebnisdeskriptorkonstruktion gelöscht. |
| A7 | Hybrid-Bildschirmfreigabe: `ScreenProjectionFrames.sendRequested` → `ScreenHostControl`-Submit → `ScreenEncoder.queueRgba` → RGBA-Alias / Pair / ConcurrentHashMap / `EncoderAdmission.expect` → Codecqueue → AU-/Capture-Paarung / `push` → `lastPixels`. Pair-/Mapfehler konnten eine bereits erzeugte Referenz verlieren. Ein Fehler von `expect` lag außerhalb des bisherigen Rollbacks. Außerdem wurde `lastPixels` bereits vor bestätigtem Codecqueueerfolg ersetzt; ein wiederholtes Bild konnte dadurch fälschlich als erfolgreich eingereichte Referenz gelten. |

### Ergänzender konkreter Fixplan vor der Erweiterungsimplementierung

| Meilenstein | Änderung und betroffene Dateien | Erwartetes Ergebnis |
| --- | --- | --- |
| M5 Projectionstart und -ende | Den durchgehenden Konstruktorbesitzer als Capture-Konstruktorbesitzer verwenden; die vorhandenen Reader-/Threadreferenzen in `ScreenProjectionFrames.kt` darin halten. Bei Abbruch zuerst tatsächliches HandlerThread-Ende, dann Readerende, dann Claimende. Nach erfolgreicher Übergabe besitzt das `finally` des vorhandenen HandlerThreads den Ressourcenabschluss; dazu wird kein neuer Handlerpost angelegt. Ein vor der Übergabe beendeter Worker bleibt beim Konstruktorrollback. | Jeder bereits erworbene Besitzer bleibt erreichbar; ein späterer Handler-/Listenerfehler erzeugt weder verlorene Claims noch Leserzugriffe auf einen bereits zerstörten Reader. Auch ein spontan beendeter Handler schließt seine Ressourcen; ein Join-Timeout meldet keinen Erfolg. |
| M6 Accessibilityübergabe | In `ScreenControlService.kt` die Abschlussmarke vor Anfrageregistrierung erzeugen. Softwarebitmap erst nach erfolgreicher `ScreenCaptureBitmap`-Erzeugung an den Callback übertragen. | Fehlgeschlagene Wrappererzeugung recycelt die Bitmap und schließt Software-/Hardwareclaims; die Anfragezählung bleibt auch vor dem Screenshotaufruf konsistent. |
| M7 AU-Übergabe | `ScreenCodecLatency.kt` verwendet einen vorab vorhandenen Ownershipslot aus `ScreenCaptureOwnership.kt`: Buffererzeugung und Viewaufbau sind gemeinsam geschützt, der Ausgabedeskriptor übernimmt den Owner erst nach erfolgreicher Erzeugung. | Fehlgeschlagener Viewaufbau gibt den tatsächlichen Owner zurück; fehlgeschlagener Deskriptoraufbau lässt ihn für `clear` erreichbar. Erfolgreiche AU-Ausgabe bleibt bis zum Consumer-`finally` gebunden. |
| M8 Hybridinput | Den bereits vorhandenen Einzelinputvertrag in `ScreenEncoder.kt` durch einen kleinen synchronisierten, allokationsfreien Pending-Slot ausdrücken. Jeder Input besitzt einen vor dem Retain erzeugten Abschlusswrapper; Fehler nach Publikation schließen denselben Wrapper idempotent. `lastPixels` erst nach erfolgreicher Codecqueue übernehmen. | Eingabefehler räumen Pending-/Admissionzustand und Aliase auf. Gleiches Bild nach einer gescheiterten Eingabe wird erneut vollständig eingereicht. Gleiches Bild nach einer erfolgreichen Eingabe behält den bisherigen Repeatpfad. |

Die Entscheidung gegen weitere Pair-/Map-Rollbackflags folgt aus dem tatsächlichen
Einzelinputvertrag (`pending.isNotEmpty`, `EncoderAdmission`): Ein fester Slot
benötigt für Publikation, Abbruch und Übernahme keine Boxing-/Containernachallokation.
Die Auswahl entfernt keine zuvor erlaubte parallele Hybrid-Eingabe; eine zweite
ausstehende Eingabe war schon vorher ausgeschlossen.

Primärgrundlagen bleiben JLS 17 §15.9.4, Kotlin-Inline-/Threaddokumentation und die
Java-17-Thread-API. Die verbleibende Plattformfrage für M5 ist die geordnete
`HandlerThread.quitSafely`-/`getLooper`-Grenze. Die versionierte
[AOSP-Android-16-Quelle](https://raw.githubusercontent.com/aosp-mirror/platform_frameworks_base/android-16.0.0_r1/core/java/android/os/HandlerThread.java)
wurde am 2026-10-26 gelesen: `getLooper` wartet bei einem gestarteten Thread auf
seine Loopervorbereitung; `quitSafely` fordert dessen geordnetes Ende an. Ein
anschließender tatsächlicher Join bleibt deshalb notwendig. Die bereits gelesenen Quellen belegen die
Allokationsgrenzen von M6–M8; keine neue Bibliothek oder externe API wird eingeführt.

## Vorbereitete gezielte Suitefälle und deren Beleggrenzen

Die folgenden Methoden sind im Quellstand vorbereitet. Main registriert die
Klassen und endgültigen Methodennamen in seiner einzigen Remote-Suite. Der
Fixautor hat keine dieser Methoden lokal oder remote ausgeführt.

| JVM-Klasse / Methode | Erwartete Beobachtung |
| --- | --- |
| `ScreenCaptureBudgetTest.callbackAndWorkerRetainTheSamePromiseUntilBothEnd` | Bestehende Callback-/Workeraliase schließen denselben Token erst nach dem letzten tatsächlichen Holder. |
| `ScreenCaptureBudgetTest.aClosedHolderCannotCreateAReplacementOwner` | Ein geschlossener Holder kann keinen Ersatzbesitzer erzeugen. |
| `ScreenCaptureBudgetTest.failedTokenAdoptionReturnsTheRegisteredNativeTokenExactlyOnce` | Injizierter Ownerfactoryfehler gibt genau den bereits registrierten Token zurück. |
| `ScreenCaptureBudgetTest.failedAliasFactoryLeavesOnlyTheTwoActualOwners` | Injizierter Aliasfactoryfehler erzeugt keinen zusätzlichen Zählerbesitzer. |
| `ScreenEncoderOwnershipTest.failedConstructionReturnsEveryAcquiredResourceInOrder` | Ein Abbruch vor Runtimeübernahme beendet jeden tatsächlich erworbenen Produzenten, dessen Surface und anschließend den Claim; ein erneuter Abbruch gibt nichts doppelt frei. |
| `ScreenEncoderOwnershipTest.rejectedCodecCanBeReplacedWithoutReleasingTheSurfaceClaim` | Codecablehnung beendet den Kandidaten und erhält den gemeinsamen Surfaceclaim für den folgenden Kandidaten. |
| `ScreenEncoderOwnershipTest.failedPlatformDestructionKeepsTheClaimAndTheOriginalFailure` | Eine fehlgeschlagene Produzentenzerstörung gibt weder Surface noch Claim vorzeitig frei und erhält die ursprüngliche Ausnahme. |
| `ScreenEncoderOwnershipTest.abortedConstructionRejectsALateDrainBeforeAnyResourceAccess` | Ein nach dem Abbruch tatsächlich gestarteter JVM-Thread darf die bereits zurückgerollten Ressourcen nicht benutzen. |
| `ScreenEncoderOwnershipTest.enteredDrainKeepsItsResourcesUntilItsActualEnd` | Ein bereits eingetretener echter JVM-Worker verhindert Konstruktorrollback bis zu seiner eigenen Bereinigung. |
| `ScreenEncoderOwnershipTest.failedProducerWrapperDoesNotPublishAUserOrLoseAnAlias` | Fehlgeschlagene Abschlusswrappererzeugung erhöht keinen Produzentenstand und verliert keinen Alias. |
| `ScreenEncoderOwnershipTest.creatorRollbackNeedsNoJoinOfTheStillRunningCreator` | Ein Konstruktionsabbruch auf dem Erzeugerthread beendet nur den erzeugten Besitzer; ein noch tatsächlicher zweiter Besitzer verhindert Freigabe. |
| `ScreenEncoderOwnershipTest.completedLeaseStillWaitsForTheClosingWorkerToReturn` | Geschlossene Lease bei weiter lebendem JVM-Worker bleibt von wirklichem Threadende getrennt; auch während des Joins bleibt die Freigabe gesperrt. |
| `ScreenEncoderOwnershipTest.interruptedDrainStillWaitsForTheProducerAndRestoresItsFlag` | Ein Interrupt ersetzt das Threadende nicht; das Flag wird erst nach dessen bestätigtem Ende wiederhergestellt. |
| `ScreenEncoderOwnershipTest.projectionRollbackEndsItsStartedWorkerBeforeTheReaderAndClaim` | Nach einem injizierten Handleraufbaufehler endet zuerst der tatsächlich gestartete JVM-Worker, anschließend der Readerersatz und der Claim. |
| `ScreenEncoderOwnershipTest.producerEndBeforeStartupTransferLeavesTheConstructorResponsible` | Ein vor Startupübernahme beendeter Produzent bleibt in genau einem Konstruktorrollback. |
| `ScreenEncoderOwnershipTest.failedPixelViewConstructionReturnsItsActualAllocationOwner` | Ein injizierter Viewfactoryfehler schließt den bereits übernommenen Partialowner einmal. |
| `ScreenEncoderOwnershipTest.failedAccessUnitDescriptorLeavesItsOwnerForTeardown` | Ein injizierter Deskriptorfactoryfehler lässt den Partialowner bis zum `clear` erreichbar. |
| `ScreenEncoderOwnershipTest.completedAccessUnitTransfersItsExistingOwnerUntilConsumerEnd` | Ein vollständiger Ausgabewrapper übernimmt denselben Besitzer, der erst beim Consumerende geschlossen wird. |
| `ScreenEncoderOwnershipTest.hybridQueueFailureClearsTheExactPendingInputAndAllowsRetry` | Ein abgebrochener Input hinterlässt keinen wartenden Slot; eine neue PTS-/Frame-ID kann anschließend denselben Einzelinputvertrag nutzen. |
| `ScreenEncoderOwnershipTest.queueAbortCannotCloseAnInputAlreadyOwnedByTheOutputConsumer` | Ein konkurrierender Queueabbruch kann einen übernommenen Outputbesitzer nicht vor dessen `finally` schließen. |
| `ScreenEncoderOwnershipTest.hybridInputRetainFailureNeverPublishesAReplacement` | Ein fehlgeschlagener Input-Retain erzeugt keinen publizierten Pendingbesitzer. |

Die Unitfälle verwenden die produktiven Ownershipzustände mit injizierten
Factory-/Grenzfehlern und tatsächlichen JVM-Threads. Sie erzwingen keinen realen
Speichermangel und keine ART-/MediaCodec-/ImageReader-Ausnahme. Die Endnutzerpfade
bleiben über die bereits bestehenden Devicefälle vorgesehen:
`ScreenTaskTest.screen_task_android_host_bench_synthetic`,
`screen_task_android_viewer_decodes_barcode`,
`screen_task_android_still_interval_respected`,
`screen_task_android_still_secure_window_black` und
`screen_task_android_host_snapshot_nonblack` sowie die bestehende
`BatchesCaptureTaskTest`-Klasse. Deren Assertions wurden in diesem Fix nicht
verändert. Die tatsächliche Android-Fehlerauslösung für Bitmapwrapper,
HandlerThreadstart, Codecqueue und AU-Deskriptorallokation bleibt ungemessen.

### Genaue Zuordnung der zusätzlichen Meilensteine M5–M8

| Meilenstein | Zugeordnete vorbereitete Fixturemethoden | Ausdrückliche statische Beleggrenze |
| --- | --- | --- |
| M5 | `ScreenEncoderOwnershipTest.projectionRollbackEndsItsStartedWorkerBeforeTheReaderAndClaim`; `producerEndBeforeStartupTransferLeavesTheConstructorResponsible` | Der Produktionsbesitzentscheid und die Reihenfolge werden mit tatsächlichem JVM-Worker und Readerersatz geprüft. Ein echter Android-Handler-/Listener-Konstruktorfehler, das tatsächliche HandlerThread-`finally` und ein wirklicher Projection-Close-Timeout werden nicht injiziert. Deren Anschluss ist statisch gelesen; der normale Projectionabschluss bleibt im bestehenden `ScreenTaskTest.screen_task_android_host_snapshot_nonblack` vorgesehen. |
| M6 | Bestehende `ScreenTaskTest.screen_task_android_still_interval_respected` und `screen_task_android_still_secure_window_black` | Beide Devicefälle betreffen den normalen tatsächlichen Screenshotpfad. Keine Fixture injiziert einen Fehler der `AtomicBoolean`- oder `ScreenCaptureBitmap`-Erzeugung. Für diese beiden Fehlerzweige ist die konkrete Beleggrenze die Reihenfolge im aktuellen `ScreenControlService.still` und dessen vorhandene `finally`-Pfade. |
| M7 | `ScreenEncoderOwnershipTest.failedPixelViewConstructionReturnsItsActualAllocationOwner`; `failedAccessUnitDescriptorLeavesItsOwnerForTeardown`; `completedAccessUnitTransfersItsExistingOwnerUntilConsumerEnd` | Die Factoryfehler werden am produktiven Ownershipslot mit Ersatzbesitzern injiziert. Reale ART-View-/BufferInfo-/AccessUnit-Allokationsfehler und echte fragmentierte MediaCodec-AUs werden nicht erzwungen. Der Anschluss `AccessUnits.complete` → Slot → Outputconsumer ist statisch geprüft. |
| M8 | `ScreenEncoderOwnershipTest.hybridQueueFailureClearsTheExactPendingInputAndAllowsRetry`; `queueAbortCannotCloseAnInputAlreadyOwnedByTheOutputConsumer`; `hybridInputRetainFailureNeverPublishesAReplacement` | Die produktiven Inputslot-/Übergabezustände werden gezielt vorbereitet. Kein Fall injiziert einen echten `queueInputBuffer`-Fehler oder einen Fehler der `EncoderAdmission.Pending`-Allokation. Die Rücksetzung beider Zustände und `lastPixels` erst nach erfolgreicher Codecqueue sind statisch geprüft; ein tatsächlicher Pixel-Repeat nach injiziertem Codecfehler bleibt unbelegt. |

Die kurzen Methodennamen einer Tabellenzelle bezeichnen dieselbe dort zuerst
vollständig genannte JVM-Klasse. Diese Zuordnung benennt vorgesehene Nachweise;
keine Tabellenzelle ist ein ausgeführter Pass.

## Gelesene und geänderte Dateien

Die folgende Liste bezeichnet vollständig gelesene Produkt-, Fixture- und
native Besitzerdateien. Der Architektur-/Dokumentationsbezug und die größeren
Fassadendateien folgen getrennt mit ihrem tatsächlich gelesenen Umfang.

```text
/tmp/smartexplorer-android-capture-ownership-fix-scope.json
android/app/src/main/java/app/smartexplorer/android/screen/ScreenCaptureBudget.kt
android/app/src/main/java/app/smartexplorer/android/screen/ScreenCaptureOwnership.kt
android/app/src/main/java/app/smartexplorer/android/screen/ScreenCaptureBuffer.kt
android/app/src/main/java/app/smartexplorer/android/screen/ScreenEncoder.kt
android/app/src/main/java/app/smartexplorer/android/screen/ScreenCodecLatency.kt
android/app/src/main/java/app/smartexplorer/android/screen/ScreenControlService.kt
android/app/src/main/java/app/smartexplorer/android/screen/ScreenProjectionFrames.kt
android/app/src/main/java/app/smartexplorer/android/screen/ScreenStillSource.kt
android/app/src/main/java/app/smartexplorer/android/screen/SyntheticSurfaceSource.kt
android/app/src/main/java/app/smartexplorer/android/screen/ScreenSyntheticRetirement.kt
android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenDecoder.kt
android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenHybridDecoder.kt
android/app/src/test/java/app/smartexplorer/android/screen/ScreenCaptureBudgetTest.kt
android/app/src/test/java/app/smartexplorer/android/screen/ScreenEncoderOwnershipTest.kt
android/app/src/androidTest/java/app/smartexplorer/android/task/ScreenCaptureFixture.kt
android/app/src/androidTest/java/app/smartexplorer/android/task/ScreenCaptureQueue.kt
native/android-bridge/src/screen_capture_jni.rs
native/src/screen/os/android/capture_arena.rs
native/src/screen/os/android/capture_admission.rs
native/src/screen/core/capture_grant_claims.rs
native/src/screen/core/capture_grant_slices.rs
```

| Abschnittsweise gelesene oder gezielt durchsuchte Datei | Umfang |
| --- | --- |
| `/root/.codex/AGENTS.md`, `AGENTS.md` | gelieferte globale und lokale Projektregeln; Scope-/Build-/Suite-/Releasegrenzen |
| `/root/.codex/skills/graphify/SKILL.md` | Navigation mit vorhandener Query; keine Extraktion, Installation oder Graphmutation |
| `docs/ARCHITEKTUR.md` | Android-/Screen-/Bridge-/Task-Suite-Einstiege und aktuelle Batchgrenzen |
| `docs/oberflaechen/bildschirm.md` | Android als Host, Capture-/Codecgrenzen und bestehende Oberflächenzuordnung |
| `docs/plaene/2026-10-24-batches/android-bildfixtures.md` | F2–F6-Besitzketten, Synthetic-Endevertrag und bestehende Deviceassertionen |
| `docs/plaene/2026-10-24-batches/review-android-bildfixtures.md` | A1–A3, Korrekturgrenzen und deren Primärquellen |
| `docs/plaene/2026-10-24-batches/fix-android-capturebesitz.md` | eigener inkrementell geführter Plan und Ergebnisbericht |
| `graphify-out/graph.json` | nur die Capture-/Claim-/Thread-bezogene `graphify query`-Ausgabe; kein vollständiger Graphread |
| `android/app/src/main/java/app/smartexplorer/android/core/NativeBridge.kt` | Capture-Begin-/Claim-/Buffer-/Release-/Push-Verträge |
| `android/app/src/main/java/app/smartexplorer/android/api/ScreenApi.kt` | Encoderangebote und Hostvoraussetzungsbezug |
| `android/app/src/main/java/app/smartexplorer/android/screen/ScreenHostControl.kt` | Konfiguration, Pausen, Capture/Still, Sourceanlage, Sinkidentität, Projection-Ready-Tokens, Ablösung und Teardown |
| `android/app/src/androidTest/java/app/smartexplorer/android/task/ScreenTaskTest.kt` | Imports, beide Encoderfälle und die direkt betroffenen Still-/Projection-/Barcodeaufrufe |
| `native/src/mobile/os/shared/domains/screen_host.rs` | Capturebegin, Grantauflösung, Arenaclaim, Bufferadresse und Release |
| `native/src/screen/os/android/bridge_channels.rs` | Generation-/Capturebezug und Kanalretirement |
| `native/src/screen/os/android/bridge_source.rs` | Projection-/Accessibility-/Synthetic-Grenze und Tilesframebezug |
| `native/src/screen/core/capture_grant.rs` | Grant-/Credit-/Retirementbezug |
| `native/review-task-manifest.json` | bestehende Android-Unit-/Deviceklassen und Android-Meilensteinbindung; keine Änderung durch den Fixautor |

Erstellt wurden exakt `ScreenCaptureOwnership.kt`,
`ScreenEncoderOwnershipTest.kt` und dieses Dokument. Geändert wurden exakt
`ScreenCaptureBudget.kt`, `ScreenEncoder.kt`, `ScreenProjectionFrames.kt`,
`ScreenControlService.kt`, `ScreenCodecLatency.kt` und `ScreenCaptureBudgetTest.kt`.
Alle Pfade liegen im freigegebenen Schreibmanifest. Der Fixautor hat keine
native Rust-Datei, Decoderdatei, Hostkontrolldatei, Gerätfixture, Suite-/Manifestdatei,
Release-/Versionsdatei oder Graphdatei verändert.

## Tatsächlich ausgeführte statische Prüfung und offene Laufzeitbelege

Der Dokumentationskontext wurde vor der ersten Dokumentänderung mit
`git status --short`, `git log -1 --oneline`, einer gezielten Themensuche und den
aktuellen Capturequellen geprüft. Ausgangscommit war `2fb02432`; die geteilte
Arbeitskopie enthielt bereits Mains zusammenhängende Batchänderungen.

`git diff --check` wurde nur für die freigegebenen geänderten Trackingdateien
ausgeführt. Die zusätzlich erstellten Kotlin-/Dokumentdateien wurden per
Textsuche auf Zeilenend-Leerraum geprüft. Diese Prüfungen fanden keinen
Whitespacefehler. Die Abschlussprüfung der Methodendeklarationen ist eine
Quelltextsuche, kein JUnitlauf. Die Graphquery meldete den Skill-/Paketstand
0.9.11/0.9.17; es wurde nichts aktualisiert oder neu aufgebaut, und ihre Ausgabe
wurde nicht als Kotlin-Lebensdauerbeweis verwendet.

Es fehlen weiterhin ein ausgeführter Kotlin-Build, die ausgeführten JVM-Fixtures,
die tatsächlichen Android-Gerätfälle und real ausgelöste Allokations-/Threadstart-
Fehler. Verhalten konkreter Hersteller-Codecs und ein tatsächlich blockierter
HardwareCanvas sind durch diese Quellenprüfung nicht beobachtet. Main besitzt
die gemeinsame Remote-Suite, Integration, Commit/Push und den terminalen Remote-
Release. Diese fehlenden Laufzeitbelege sind keine als bestanden gezählten Fälle
und keine zusätzlich eingeführten Releasevoraussetzungen.
