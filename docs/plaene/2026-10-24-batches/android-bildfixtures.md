# Nachplan: bestehende Android-Aufnahmefixtures nach C9

Stand: 2026-10-26. Die Abschnitte „Erstplan“ bis „Projektbericht dieser Planungsarbeit“ dokumentieren den abgeschlossenen historischen Planungsauftrag vor Umsetzung. Der aktuelle Quellkandidat und seine weiterhin fehlenden Laufzeitbelege stehen im abschließenden Abschnitt „Quellkandidat F2–F6“. Ergänzung des Hauptagents um 07:22 UTC: derselbe abgeschottete Fixtureanschluss soll zusätzlich den echten Login-ViewerFrame→JNI→NDK-Fill→GlobalRef-/Recycleweg ermöglichen; vorhandene HWUI-Fakes gelten nicht als JNI-Nachweis.

## Erstplan aus dem delegierten Auftrag

Ergebnis: Jeder bestehende, durch C9 betroffene Aufrufer in `ScreenTaskTest.kt` erhält einen konkret benannten budgetierten Anschluss mit Ressourcenbesitz, Ressourcenende, Buildgrenze und Remote-Nachweis. Die bestehenden Codec-Rundläufe und Accessibility-Prüfungen bleiben erhalten. Ein Produktzugang ohne vorherige echte W/X-/Slicezusage ist ausgeschlossen.

Auftragsquelle: delegierter Auftrag `/root` an `/root/android_fixture_plan`. Übermittelter Nutzerwortlaut: „die umsetzung läuft komplett über codex, über credits“; außerdem ausdrücklich maximale Planungstiefe. Datum dieser Nutzernachricht im übermittelten Verlauf unbekannt.

Grenze: Lesen ausschließlich gemäß `/tmp/smartexplorer-android-capture-fixture-plan-scope.json`; Schreiben ausschließlich dieser Datei. Keine Produkt-, Fixture-, CI-, Cargo-, Dokumentations-, Graph- oder Gitänderungen. Keine Builds, Compiler, Tests, Formatter, Server, Installationen, langen Prozesse, Commits, Pushes, Graphupdates oder Unteragenten.

Promptwissen: `ScreenEncoder` benötigt `(config, budget, sink)`, `SyntheticSurfaceSource` einen gehaltenen `surfaceLease`, `ScreenControlService.still` benötigt `(budget, width, height, callback)` und `ScreenStillSource` ist an eine budgetierte native Sitzung gebunden. Die einzelnen Aufrufstellen, Zulassungswege und Besitzerenden sind noch durch Quelllektüre zu bestimmen.

Bestandsfragen: Welche alten Aufrufer existieren vollständig? Welche regulären Sessionstarts sind ohne Netzwerk-/Peerzustand isolierbar? Welcher native Zulassungsweg besitzt echte W/X-Grants und Slicegrenzen? Welche Feature-/Exportgrenze isoliert nötigenfalls ausschließlich Instrumentationszugänge? Welche Remote-Konfiguration und realen Ergebnisbelege fehlen?

Planungstiefe: maximal für den engen Android-Aufnahmefixture-Anschluss, weil JNI-, native Grant-, Surface-/Codec- und Accessibility-Lebensdauern zusammenpassen müssen. Nächste Handlung: freigegebene Architektur-, Vertrag- und Aufruferdateien lesen, daraus belegten Zweitplan und anschließend den vollständigen Meilensteinplan ableiten.

## B — Belegter Bestand und Zweitplan

Quellstand: freigegebene Arbeitsdateien, gelesen am 2026-10-26; kein Commitstand wurde abgefragt. Die tatsächliche Ausführung sämtlicher unten beschriebener Pfade ist noch ungeprüft.

| Bestehender Aufrufer in `android/app/src/androidTest/java/app/smartexplorer/android/task/ScreenTaskTest.kt` | Fundstelle | Bisheriges Ergebnis, das erhalten bleibt |
|---|---|---|
| `screen_task_android_host_bench_synthetic` | `ScreenEncoder(config)` Zeile 218; `SyntheticSurfaceSource(...)` Zeile 231 | Reale MediaCodec-Ausgabe, mindestens 45 Frames im bestehenden Messfenster, erstes Bild Schlüsselbild, Annex-B-Präfix und SPS-NAL, bestehende Prereq-/leere-Hostsession-Prüfung. |
| `screen_task_android_viewer_decodes_barcode` | `ScreenEncoder(config)` Zeile 478; `SyntheticSurfaceSource(...)` Zeile 489 | Reale Surfaceproduktion, reale Decoderbilder, steigender aus YUV gelesener Barcode, volle GOP-/Frame-IDs und tatsächlich gemeldete Surface-Präsentationsidentität. |
| `screen_task_android_still_interval_respected` | `ScreenStillSource { _, length -> ... }` Zeile 316; `capture(service)` Zeile 320 | Vier echte Accessibility-Screenshotantworten; erfolgreiche Nutzdatenverarbeitung; Abstände der Anfragen mindestens `MIN_INTERVAL_MS=333`. |
| gemeinsamer Helfer `still(service)` und `screen_task_android_still_secure_window_black` | alter `service.still { bitmap -> ... }` Zeile 191; Verwendung Zeile 343 | Tatsächliches Screenshotresultat mit geschwärzter `FLAG_SECURE`-Fläche; `dark > 0.9`; Windowflag und Bildressourcen werden auch bei Fehler beendet. |
| `screen_task_android_host_snapshot_nonblack` | `PROBE_SESSION` Zeile 282; Reader/Display Zeilen 288–289; Bitmap Zeile 298 | Systemzustimmung und laufender Foregrounddienst, reale MediaProjection-Pixel, `dark < 0.95`. Dieser Aufrufer hat keine C9-Signaturabweichung, besitzt aber unmittelbar betroffene, bislang unbudgetierte Reader-/Bitmapressourcen. |

Die vollständige `ScreenTaskTest.kt` wurde gelesen. Ihre Gesten-, Text-, Zustimmungs-, Genehmigungs- und Gridprüfungen enthalten keine Aufrufe der vier geänderten Capturetypen und bleiben erhalten.

Wiederverwendbarer Produktvertrag:

- `ScreenCaptureBudget.open(session,width,height,kind)` öffnet ausschließlich die bereits zugelassene native Sessiongeneration (`ScreenCaptureBudget.kt:37`; `screen_host.rs:49`; `CaptureSession::begin`, `capture_admission.rs:50`). Ohne attach, plan, approve und activate gibt es keine Generation.
- `ScreenEncoder(config,budget,sink)` beansprucht die Eingangssurface vor Codecöffnung; `retainSurface()` aliasiert denselben Besitz. `SyntheticSurfaceSource(...,surfaceLease)` schließt seinen Alias ausschließlich im `finally` des Zeichenworkers. `stop()` hat einen begrenzten Join und beweist allein kein Workerende (`ScreenEncoder.kt:40–86,100–125`; `SyntheticSurfaceSource.kt:30–37`).
- `ScreenControlService.still(budget,expectedWidth,expectedHeight,done)` hält vor `takeScreenshot` Hardware- und Softwareclaims. Der Callback liefert `ScreenCaptureBitmap?`, keine nackte `Bitmap`; `ScreenCaptureBitmap.close()` recycelt zuerst die Bitmap und schließt danach die Lease (`ScreenControlService.kt:108–164`; `ScreenCaptureBuffer.kt:35–38`).
- `ScreenStillSource(session,geometry,sink)` erwartet `sink(ByteBuffer,Int,Long):Int`; das dritte Argument ist die Capturegeneration. `capture` öffnet ACCESSIBILITY, benutzt native Transportablage und schließt das Resultat nach dem Sink. `close()` deaktiviert neue Produktion; ein ausstehender Callback bleibt verantwortlich (`ScreenStillSource.kt:12–71`).
- `frame_memory::admit_session` verwendet den gemeinsamen Prozesspool; `SessionAdmission::bind_contract` bindet den unveränderten Vertrag, `authority().approve(plan)` hält W und den exakt benötigten Zusatz X. Android liefert keine Desktop-Speicherfakten. `ScreenFrameLimits.workingBytes()` misst den verfügbaren Javaheap und begrenzt auf 512 MiB (`frame_memory.rs:91–117,163–178`; `capture_admission.rs:137–180,251–293`; `session_memory.rs`; `ScreenFrameLimits.kt:13–17`).
- `capture_arena` hält echte Slices beziehungsweise native Byteablagen. Channelende entfernt keine lebenden Arenatokens. Die vorhandenen Rustfälle belegen die vorgesehenen Quellinvarianten, sind hier nicht ausgeführt (`capture_arena.rs:15–38`; `batches_capture_task_tests.rs:46–81`).

Der reguläre Produktstart ist als unmittelbarer Fixtureanschluss nicht vorhanden: `screen.viewOpen` verlangt `direct` oder `room`, gültige Profile und eine Peer-Verbindung (`screen_view.rs:115–152,286–319`; `screen_view_run.rs:149–166`). `BridgeBackend.prepare_session_source` öffnet einen Channel mit Ereignis; dessen `screenHostStart` startet in Kotlin einen eigenen Steuerthread (`bridge_backend.rs:146–171`; `bridge_channels.rs:345–365`; `ScreenHostControl.kt:155–179`). Die Fixture darf diese automatische Quelle nicht parallel zum selbst gehaltenen Encoder starten. `open_source` lehnt mit `capture_grant_required` ab (`bridge_backend.rs:173–175`).

Der vorhandene `PROBE_SESSION=-1`-Zugang dient nur dem System-Projectiontoken; JNI akzeptiert keine negative Capture-Session. Er erzeugt keine native SessionAdmission. Er kann als bestehender Permission-/Servicebeleg erhalten werden, finanziert aber keine Pixel (`ScreenHostControl.kt:82–92,374–388`; `ScreenHostService.kt:62–82`; `screen_capture_jni.rs:5–10`).

Notwendige neue Arbeit für den Nachplan: eng begrenzter, standardmäßig ausgeschalteter Fixturezugang; echte gemeinsame W/X-Zulassung; stille Fixturechannelregistrierung ohne `screenHostStart`; genaue Ressourcenhalter für vorhandene Encoder-, Still-, Projection- und Transportbesitzer; Remote-Buildgrenze ohne Fixtureexport im Release. Keine neue Produktfunktion und kein grantloser Ersatzkonstruktor.

## C — Anschlussentscheidung und Recherche

Endgültige Entscheidung nach Bestandslektüre und zweiter Recherche: Für die selbst besessenen Codec-/Stillfixtures ist ein ausschließlich im Instrumentationskandidaten aktivierbarer nativer Fixturezugang erforderlich. Er verwendet die realen Produktionsbudgets, Claim-/Buffer-/Releasefunktionen und den gemeinsamen Prozesspool. Ein separater `MemoryPool::default()` wie in isolierten Rust-Unittests wäre hier falsch, weil Instrumentation im echten Appprozess läuft.

Der ergänzte Loginauftrag ändert die Quellfläche und die gemeinsame Buildgrenze, nicht den Erhalt sämtlicher C9-Aufrufer.

### Ergänzter Loginbestand

`LoginImageTaskTest.kt` besitzt zwei echte HWUI-Fälle: `close_without_followup_draw_waits_for_public_destroy_off_ui` und `texture_detach_transfers_release_responsibility_until_renderer_ends`. Ihr `Calls : LoginBitmapCalls` erzeugt jedoch selbst eine rote Java-Bitmap und simuliert Borrow-, Retirement- und Renderzielzählung. Beide Fälle bleiben erhalten; keiner belegt `NativeBridge.loginBitmapTake` oder NDK-Fill.

Die produktive Kette ist bereits vorhanden: `LoginImagePane.take(session)` → `NativeBridge.loginBitmapTake(session,owner)` → `login_images::take` → `login_view::take_image_frame` → `LoginBitmapTicket` → `login_bitmap_owner::create` → `login_bitmap_pixels::fill`. Der Nativeowner besitzt `Global<JObject>` und `LoginBitmapLease`; Cleanup recycelt, entfernt den GlobalRef und beendet anschließend den Slice. Das Ticket akzeptiert nur ein echtes `ViewerFrame.image` mit Rolle `ViewerInput` und derselben `Arc<ImageAdmission>` wie der Pane. Format, Token, Bindung und Generation werden geprüft. Damit genügt kein beliebiger Bytepuffer und kein nachträglich angehängter Scheinbesitz (`login_images.rs:74–153`; `login_bitmap_owner.rs:16–81`; `login_bitmap_pixels.rs:30–53`).

Die vorhandene `daemon/os/shared/batches_login_image_task_tests.rs::login_image_fixture` betreibt realen PNG-/IPC-/ViewerSession-Verkehr. Sie erstellt jedoch einen eigenen Testpool und ihre Einträge `ViewerSession::from_fixture_viewer`/`wait_fixture_frame` sind `cfg(test)`. Ihre Wiederverwendung im Appprozess würde mehr Transportfläche öffnen und die gemeinsame Android-Poolgrenze verändern. Der Nachplan verbindet deshalb einen echten, vor Allokation zugelassenen `ViewerFrame` direkt mit der bestehenden produktiven Bitmapabholung. Der neue Androidbeleg beginnt ausdrücklich an diesem ViewerFrame. Authentifizierter Netzwerkempfang, Maschinenbindung und PNG-Verkehr bleiben Gegenstand der vorhandenen Rustfixturen; der Androidfall behauptet keinen zusätzlichen Netzwerkbeleg.

### Bewertete Möglichkeiten

| Möglichkeit | Prüfung an den Anforderungen | Entscheidung |
|---|---|---|
| Regulärer isolierter `screen.viewOpen`/`login.viewOpen` | Benötigt Peer-/Profil- beziehungsweise Gateway-/Benutzer-/Pinbindung; startet laufende Produktcontroller. Kein vorhandener isolierter Standalone-Einstieg für die manuell besessenen Codecfixtures. | Für diese Fixtures ausgeschlossen. Die Produktwege bleiben unverändert. |
| Bestehender `PROBE_SESSION` oder beliebige positive Session-ID | Probe liefert Systemzustimmung; eine willkürliche ID hat weder native SessionAdmission noch aktivierten Grant. | Als Budget ausgeschlossen. Probe bleibt nur beim vorhandenen MediaProjection-Zustimmungspfad. |
| Testfake, `MemoryPool::default()` oder grantloser Konstruktor | Umgeht gemeinsamen Prozesspool, Vorallokation oder reale Bildbesitzer. | Ausgeschlossen. Vorhandene gezielte HWUI-Fakes bleiben mit ihrem bisherigen Belegumfang bestehen. |
| Vollständiger lokaler Login-IPC-Gegenpart im Androidtest | Erweitert Transport-/Daemon-Testfläche und verlangt das Herauslösen mehrerer `cfg(test)`-Hilfen. Die ergänzte konkrete Lücke beginnt bereits am ViewerFrame. | Für den engen JNI-Nachplan nicht erforderlich. Bestehende native IPC-Fixture wird nicht entfernt. |
| Separates, standardmäßig ausgeschaltetes Instrumentationsfeature mit eigener Debug-JNI-Klasse | Ermöglicht echte gemeinsame Admission und echte produktive Consumer; Setup/Injektion und Diagnostik fehlen vollständig im Release. | Gewählt; Ausgestaltung folgt unten. |

Schleifenziel aus dem Auftrag: ein prüfbar abgeschotteter Anschluss für sämtliche bestehenden Aufnahmefälle und den echten Login-JNI-Pfad. Kriterien sind vollständiger Erhalt der Fälle, vorherige reale Zusage, identischer Produktconsumer, Lebensdauer bis tatsächliches Ressourcenende und fehlende Releaseexporte. Die Quelllektüre erfüllt die Auswahlkriterien; D konkretisiert die API-, Cleanup- und Remotebelege. Status dieser Planungsrunde: durch die vollständige D-Ausarbeitung erreicht. Laufzeitwirkung bleibt ausdrücklich ungeprüft.

### Zweite Recherche: verbindliche Grenzen

Geprüft am 2026-10-26: Cargo kann das Feature ausschließlich auf ausdrückliche Anforderung einschalten und zum abhängigen Paket weiterreichen. Features desselben Pakets vereinigen sich; deshalb darf weder `default` noch ein Releaseaufruf mit `--all-features` das Fixturefeature mitführen. Keine neue Bibliothek ist nötig. [Cargo-Featurevertrag](https://doc.rust-lang.org/stable/cargo/reference/features.html).

Android bündelt Debugquellen und Debug-JNI-Bibliotheken über eigene Source-Sets; `src/main` bleibt dabei gemeinsam. Eine zweite gleichnamige Kotlin-Klasse in `debug` überschreibt keine Mainklasse, sondern erzeugt einen Konflikt. Gewählt wird deshalb eine eigenständige Debugklasse. [Android-Buildvarianten](https://developer.android.com/build/build-variants).

`NewDirectByteBuffer` besitzt die native Ablage nicht. Ihre tatsächliche Gültigkeit muss bis zum letzten Zugriff bestehen; die Javaansicht kann nicht als Freigabesignal dienen. Die Fixture benutzt deshalb `ScreenCaptureBuffer.access` synchron und transportiert den Besitzer, keine entkommene `ByteBuffer`-Ansicht. [JNI-Buffervertrag](https://docs.oracle.com/en/java/javase/21/docs/specs/jni/functions.html#newdirectbytebuffer).

`ImageReader.maxImages` begrenzt erworbene Images und beziffert nicht sämtliche BufferQueue-Speicher. Nach Reader-/Imageende werden vorherige Planeansichten nicht weiter gelesen. Die Readernutzlast wird vor Readeranlage gebucht; unbekannte System-/Treiberbytes bleiben ausdrücklich außerhalb der bekannten Produktpixel. [ImageReader](https://developer.android.com/reference/android/media/ImageReader).

Der Screenshotweg bleibt `takeScreenshot(DEFAULT_DISPLAY, executor, callback)` ab API 30 mit deklarierter Screenshotfähigkeit. Die vorhandenen 333 ms sind der Projektvertrag; die Androiddokumentation liefert dafür keine hier behauptete universelle OEM-Zeitgarantie. Ein `wrapHardwareBuffer`-Bitmap hält eine eigene HardwareBuffer-Referenz, weshalb Bitmaprecycling und HardwareBuffer-Schließen beide nötig bleiben. [AccessibilityService](https://developer.android.com/reference/android/accessibilityservice/AccessibilityService), [Bitmap](https://developer.android.com/reference/android/graphics/Bitmap).

Bei MediaProjection auf API 34 bleibt der Foregrounddienst aktiv; jede neue Capture-Sitzung braucht die vorhandene Systemzustimmung und verwendet ihr Token nur für ein `createVirtualDisplay`. Ein Fixturegrant ersetzt diese Freigabe nicht. [MediaProjection-Vertrag](https://developer.android.com/media/grow/media-projection).

Der NDK-Vertrag bestätigt: `getInfo` liefert Format/Stride, ein erfolgreicher `lockPixels` stabilisiert die Adresse, `unlockPixels` beendet deren Nutzung. JNI-GlobalRefs bleiben bis zum expliziten Löschen gültig. Die Debug-JNI-Klasse wird im Ziel-APK und dessen vorhandenem Classloader geladen; es gibt keine zweite native Bibliotheksinstanz im Test-APK und keinen neuen nativen Worker mit einem fremden `JNIEnv`. [NDK-Bitmap-API](https://developer.android.com/ndk/reference/group/bitmap), [JNI-Referenzen und Classloader](https://developer.android.com/ndk/guides/jni-tips).

Nachfreigegebener Kernbefund: `HostOutputPlan::new` und `CaptureSession::begin` erzeugen dieselbe ausgewählte Codec-/Tiles-Ausgaberechnung. Die Fixture muss diesen vollständigen Pfad verwenden: Ein anfängliches `source_only()` würde bei `begin` die zusätzliche Produktrechnung erst später erzwingen und wäre kein verlässlicher positiver Anschluss. `FrameContract::validate` prüft unverändert W/24 und `PacketBudget`. `CaptureImageBuilder::new_local` beansprucht die `ViewerInput`-Slice vor der BGRA-Allokation. `host_codec_phase_plan.rs` war beim gezielten Abruf noch nicht vorhanden; die in `host_codec_adapters.rs` gelesene Rechnung ist daher als momentaner Arbeitsstand gekennzeichnet. Main meldete parallel den produktiven Anschluss der echten Heapbeobachtung vor `screen.viewOpen` und `screen.hostPrereqs`; diese Nachricht ist Integrationshinweis, kein von diesem Plan neu erzeugter Codebeleg.

## D — Endgültiger Anschluss- und Meilensteinplan

Historischer Planungsstand vor Umsetzung: Die nachfolgend genannten Dateien, Funktionen und Signaturen waren beim Abschluss dieses Planungsauftrags noch vorgesehen. Die später autorisierte Quellumsetzung ist im abschließenden Abschnitt „Quellkandidat F2–F6“ erfasst. Main koordiniert Suite, Cargo-/CI-Integration und den gemeinsamen Release.

### D1 — Abschottung und genaue Einbaufläche

Ein einziges Feature `android-instrumentation-fixtures`, standardmäßig aus, verbindet Capture und Login. `native/Cargo.toml` definiert es ohne Defaultaktivierung; `native/android-bridge/Cargo.toml` leitet es ausdrücklich als `smart_explorer/android-instrumentation-fixtures` weiter. Native Module, Mobile-Reexports und JNI-Exports stehen unter `cfg(all(target_os = "android", feature = "android-instrumentation-fixtures"))`. Zusätzlich verhindert eine explizite Compile-Fehlergrenze `feature && !debug_assertions` versehentlich optimierte Releasekandidaten mit Fixtureexporten; die Quell- und Pakettrennung bleibt trotzdem verpflichtend. Ein Debugflag alleine ersetzt die Featuregrenze nicht.

Die einzige neue Java/JNI-Einstiegsklasse heißt `app.smartexplorer.android.core.AndroidImageFixtureBridge` in `android/app/src/debug/java/app/smartexplorer/android/core/AndroidImageFixtureBridge.kt`. Sie ist unabhängig von `NativeBridge` und besitzt keine Produkt-JSON-Route, keinen Intent, keinen Dienst, keine öffentliche Netzwerkmethode und keinen dauerhaft registrierten Betriebsschalter. Ihre Aufrufe kommen aus `androidTest`. Das APK lädt weiterhin genau die vorhandene Appbibliothek. Ungültige Fixturehandles und fehlgeschlagene native Zusagen ergeben einen Fehler mit Ursache; fehlende Exporte sind ein Build-/Konfigurationsfehler und werden nicht übersprungen.

Vorgesehene neue, jeweils eng begrenzte Dateien:

- `native/src/screen/os/android/capture_fixture.rs`: stille Capturefixture-Registrierung, reale Zulassung, Grantbeobachtung und geordnetes Ende.
- `native/src/mobile/os/shared/domains/android_image_fixtures.rs`: schmale featuregebundene Weitergabe für die JNI-Crate; keine reguläre API-Dispatchroute.
- `native/src/mobile/os/shared/domains/login_image_fixture.rs`: gebundener, echter ViewerFrame-Kandidat und Beobachtung seiner lokalen ImageAdmission.
- `native/android-bridge/src/android_image_fixture_jni.rs`: separate JNI-Exports sowie kleine JSON-Deskriptoren und Statusdaten; keine Bildkopie.
- `android/app/src/debug/java/app/smartexplorer/android/core/AndroidImageFixtureBridge.kt`: Debugdeklarationen der unten festgelegten externen Funktionen.
- `android/app/src/androidTest/java/app/smartexplorer/android/task/ScreenCaptureFixture.kt`: Kotlinbesitzer, Stillhelfer und Cleanupaggregation für die vorhandenen Screenfälle.
- `android/app/src/androidTest/java/app/smartexplorer/android/task/ScreenCaptureQueue.kt`: begrenzte, leasebesessene AU-Übergabe zum bestehenden Decoder.
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenSyntheticRetirement.kt`: eng besessener, nachgelagerter Abschluss eines Synthetic-Source-/Encoderpaars nach tatsächlichem Workerende.

Vorgesehene bestehende Integrationsdateien: `native/src/screen/os/mod.rs` für das neue Androidmodul; `bridge_channels.rs` für die stille registrierte Fixturevariante; `screen_host.rs::screen_capture_begin` für eine featuregebundene Beobachtung der tatsächlich aktivierten Generation; `login_view.rs::{take_image_frame,login_close}` für den featuregebundenen Kandidatenzweig mit gemeinsamem ID-Zähler; `login_images.rs` für auf registrierte Fixtureowner begrenzte, lesende Admissionzustände; `mobile/os/shared/domains/mod.rs`, `mobile/mod.rs` und `android-bridge/src/lib.rs` für die bedingten Module/Reexports; `login_bitmap_owner.rs` für lesende, nach Fixtureowner gefilterte tatsächliche Bitmap-/Borrow-/Targetzustände; `ScreenTaskTest.kt` und `LoginImageTaskTest.kt` für die erhaltenen beziehungsweise ergänzten realen Aufrufer. Hinzu kommen `android/app/src/main/java/app/smartexplorer/android/screen/SyntheticSurfaceSource.kt`, `ScreenHostControl.kt` im selben Verzeichnis und `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenPullBuffers.kt` für die in D4 benannten notwendigen Lebensdaueranschlüsse. `login_bitmap_pixels.rs` bleibt für den Positivnachweis unverändert. Kein Kotlin-Produktkonstruktor erhält einen Fakebudget-Default.

Neue und substanziell geänderte Rustmodule bleiben unter den projektweiten 500 Zeilen/50 KiB. Die bereits große `bridge_channels.rs` erhält nur den kleinen gemeinsamen Registrierungsanschluss und den bedingten stillen Aufrufer; Registry- und Zulassungslogik der Fixture wird nicht dort angehäuft. Die konkrete Abspaltung der gleichzeitig bearbeiteten Codecphasen bleibt beim Kernautor/Main.

### D2 — Echte Capturezulassung und Datenvertrag

Geplante Kotlin-JNI-Signaturen in `AndroidImageFixtureBridge`:

```kotlin
external fun captureOpen(width: Int, height: Int, outputWidth: Int, outputHeight: Int, kind: Int,
    heapOffer: Long, decoderReader: Boolean): String
external fun captureState(session: Long): String
external fun captureConsumeStill(session: Long, generation: Long): String
external fun captureViewerPullBuffer(session: Long): ByteBuffer
external fun captureReleaseDecoderReader(session: Long)
external fun captureReleaseViewer(session: Long)
external fun captureRetire(session: Long)
external fun captureFinish(session: Long)
external fun loginOpen(owner: Long, heapOffer: Long, width: Int, height: Int): Long
external fun loginEnqueue(session: Long, frameId: Long, pattern: Int)
external fun loginState(session: Long): String
external fun loginFinish(session: Long)
```

Die Stringrückgaben sind kleine typisierte, strikt geparste Deskriptoren. `captureOpen` liefert `session`, `generation`, `width`, `height`, `output_width`, `output_height`, `kind`, `heap_offer`, `working_bytes`, `additional_bytes`, `plan_peak_bytes`, `decoder_reader_bytes`, `viewer_working_bytes` sowie beim Decoderfall `viewer_budget`; `captureState` liefert Phase, aktuellen Generationswert, je beobachteter Generation `live_bytes`, Host-W/X, Viewer-W und gehaltene Reader-/Pullbytes. `viewer_budget` besitzt die bestehenden `ScreenBufferBudget`-Felder `workingBytes`, `imageBytes`, `recordBytes`, `packetBytes`, `pullBytes`, `legacy=false` und `recordOverheadBytes`, abgeleitet aus dem unveränderten nativen PacketBudget. Die Stringrückgaben enthalten weder Produktpixel noch ein pauschales `passed=true`. Die zusätzliche Bufferfunktion liefert ausschließlich die schon zugelassene, feste Viewer-Pullablage dieses Fixturehandles. Native Fehler werden mit der betroffenen Operation und ihrem realen Fehlergrund weitergegeben.

Erlaubte Capturearten dieses Anschlusses sind ausschließlich ACCESSIBILITY=0, PROJECTION_TILES=1 und SYNTHETIC_SURFACE=4. `decoderReader=true` ist nur für den vorhandenen Synthetic-Decoderfall zulässig. Der Fixtureeintrag reserviert eine positive ID aus demselben Channel-ID-Zähler wie Produktchannels; er setzt keine fremde Session zurück und verwendet keine globale `reset_for_tests`-Funktion. Die stille Registrierung erzeugt keinen `screenHostStart` und keinen zweiten Kotlinproduzenten. Der Fixtureeintrag darf nur seine eigene ID beenden. Stille Löschung ist ebenfalls notwendig, damit kein Produktcontroller einen nicht von ihm gestarteten Host beendet.

Vor jeder Plattformressource erfolgt diese Reihenfolge:

1. `heapOffer=ScreenFrameLimits.workingBytes()` stammt aus dem Zielprozess. Native validiert den positiven Betrag, beobachtet ihn über den vorhandenen gemeinsamen Poolanschluss und verwendet `facts=None`, `CapacityCeiling::bounded(heapOffer)`. Das Plattformangebot bleibt getrennt von W; keine `MemoryPool::default()`-Instanz, keine Desktopfakten, kein angehobenes Mindestheapangebot.
2. Native prüft Aufnahme- und Ausgabegeometrie getrennt mit bestehenden Capturegrenzen und checked Arithmetik. Für Synthetic und Projection müssen beide Geometrien gleich sein. ACCESSIBILITY behält die volle Displayaufnahme; nur sein unbenutzter nachgelagerter Ausgabe-/Packetvertrag wird ausdrücklich auf `max(1,width/4) × max(1,height/4)` gebunden, entsprechend der bereits vorhandenen Viertelgeometrie des Projection-Snapshots. Der enge W-Antrag ist `24 * outputWidth * outputHeight * 4`; daraus entsteht ein v2-Vertrag mit genau diesen Ausgabe-Maximalmaßen/Pixeln und `hybrid_regions=false`. `FrameContract::validate`/`PacketBudget::for_contract` müssen erfolgreich sein. Die echte `frame_memory::admit_session(None, ceiling, W, session_key)`-Rückgabe muss diesen W-Antrag tragen; eine kleinere Teilzusage scheitert vor Ressourcenerzeugung. Kein Verkleinern der tatsächlichen 1280×720-Codecbilder oder Accessibility-Screenshots: deren volle native Quellbytes stehen im Captureplan und werden erforderlichenfalls durch echtes X finanziert. Für einen künftig kleineren Sonderfall, dessen PacketBudget dieses W nicht trägt, wäre ein ausdrücklich berechneter größerer W-Antrag nötig; keiner der jetzigen Fälle darf deshalb ungeprüft Legacy512 verwenden.
3. `bind_contract(Some(contract))`; `HostOutputPlan::new(HostContract::Versioned(contract), outputSize, Tiles|OpenH264)` entsprechend dem anschließenden `begin`; `channel.capture.attach`; `channel.capture.plan(captureSize,kind,&output)`; `admission.authority().approve(plan)`; `channel.capture.activate(grant)`. Erst nach erfolgreichem Abschluss wird die Fixture-ID an Kotlin gegeben. Der vorhandene Plan liefert die genaue Spitze und X; keine eigene Näherungsformel ersetzt seine Phasenrechnung. Diese Quell-/Ausgabetrennung entspricht `CaptureSession::begin` für ACCESSIBILITY und dem bestehenden Fall `batches_capture_task_android_accessibility_fits_source_only_output_before_begin`.
4. Für den Decoderfall entsteht vor dessen Konstruktion eine **separate echte Viewer-SessionAdmission/W-Zusage** aus demselben Pool, mit eigenem Fixture-SourceKey und demselben validierten v2-Vertrag. Host-W wird nicht gleichzeitig als Viewer-W ausgegeben. Unter dieser Viewerzusage wird ein enger lokaler Fixtureplan für die festen expliziten Verbraucher aktiviert: Transport=`PacketBudget.pull_bytes` einmal und NativeSurface=`3 * (w*h + 2*ceil(w/2)*ceil(h/2))` für die angeforderten drei YUV420-Images. Beide Slices sind vor ihren jeweiligen Ablagen gehalten; der Plan erhält keine allgemeine Capture-/Produktstartfunktion. Die native Arena legt den einmaligen Pullbuffer nach Slicezusage an. Die Viewer-W-Zusage deckt den unveränderten verhandelten Decodertransaktionsrahmen ab; Reader-/Pullclaims sind Besitz innerhalb dieses Rahmens, kein zweites Vollbudget über denselben Bytes. Readerbytes werden erst nach `reader.close` freigegeben; Pullablage erst nach Decoderende und Entzug sämtlicher Kotlinansichten. Padding, interne Codecflächen und BufferQueuegesamtgröße werden dadurch nicht als bekannte Gesamtallokation ausgegeben.
5. Kotlin ruft den unveränderten `ScreenCaptureBudget.open(session,w,h,kind)` auf. Seine normalen Claim-/Buffer-/Release-JNI-Funktionen bleiben die einzigen Wege für Capturepixel. Der Native-Fixtureanschluss liefert weder frei wählbare positive Granttoken noch eine Methode zum Abschalten der Zulassung.

Der initiale vollständige Plan muss die erste `begin`-Anforderung abdecken. Spätere Accessibility-Captures können echte neue Generationen eröffnen. Der featuregebundene Beobachtungshaken nach erfolgreichem `screen_capture_begin` registriert nur tatsächlich vorhandene Grants für bekannte Fixture-IDs; er erzeugt und bestätigt keine Grants selbst.

### D3 — Capture-Retirement und tatsächlich beobachteter Besitz

Der Fixtureeintrag hält Host-SessionAdmission und Diagnosealias je tatsächlich eröffneter Capturegeneration ausdrücklich bis zum geordneten Ende. Diese Aliase erzeugen keine neuen Pixel, halten aber die Host-W-Zusage bewusst bis `captureFinish`. Das wird im Bericht nicht als bereits freigegebener W-Betrag dargestellt. Alte Generationen mit `live_bytes=0` können aus der Diagnosehistorie entfernt werden; lebende Generationen bleiben beobachtbar. Die vier Stillanfragen erzeugen keine unbegrenzte Historie. Der Decoderfall hält zusätzlich seinen separaten Viewer-W bis zum bestätigten Decoderende **und** zur tatsächlichen Freigabe seiner festen nativen Pullablage; `captureReleaseViewer` verweigert ein noch gehaltenes Readerobjekt und beendet dann Pullslice, Viewergrant und Viewer-SessionAdmission in dieser Reihenfolge.

`captureRetire` sperrt den Fixtureeintrag für neue Generationen, entfernt seinen Channel still und ruft denselben Capture-Retirementweg auf. Bereits lebende Arenatokens bleiben gültig. `captureState` liest danach echte `CaptureGrant::live_bytes()` für sämtliche noch gehaltenen Generationen. `captureFinish` ist erst zulässig, wenn die Summe null ist, kein Decoderreader-/Viewerbesitz mehr gehalten wird und der Channel beendet ist. Erst dann fallen Diagnosegrants und Host-SessionAdmission synchron. Die Abwesenheit eines `Weak<SessionAdmission>` alleine wäre kein Beleg: die Slices halten intern `SessionInner` unabhängig vom äußeren Admissionobjekt.

Das Kotlin-Cleanup beendet zuerst neue Produktion und entblockiert eigene Warteschlangen, wartet anschließend auf die jeweiligen realen Besitzerenden und ruft zuletzt retire/finish auf. Ausnahmen werden gesammelt, damit ein Encoderfehler nicht Reader-/Projectioncleanup verhindert. Ein abgelaufener Join oder Callbacktimeout wird als Fehler berichtet; die Fixture setzt keine Zähler auf null und gibt keine möglicherweise benutzte Ressource zwangsweise frei. Die Remotesuite beendet nach einem harten Cleanupfehler den isolierten Appprozess vor dem nächsten unabhängigen Testsegment; dessen Ende wird als Prozessabbruch, nicht als bestandener Ressourcenendebeleg, berichtet.

### D4 — Vollständige Migration der bestehenden Screenaufrufer

**Synthetic-Benchmark:** Der bisherige Konfigurationswert `1280×720, 30 fps, 4_000_000 bit/s, high, epoch=1` bleibt. Vor Encoderanlage entsteht `captureOpen(w,h,w,h,SYNTHETIC_SURFACE,heapOffer,false)` und das daraus geöffnete `ScreenCaptureBudget`. Ersatz ist `ScreenEncoder(config,budget,sink)`; anschließend `SyntheticSurfaceSource(encoder.surface,w,h,fps,encoder.retainSurface())`. Der transferierte Surfacealias wird bei fehlschlagender Sourcekonstruktion vom Aufrufer geschlossen. Ressourcen werden ab der ersten erfolgreichen Konstruktion in `try/finally` gehalten. Der Sink wertet Framezahl, Schlüsselbild und SPS-Präfix synchron in seiner geliehenen Ansicht aus; die bisherige 64-Byte-Kopie kann durch skalare Präfix-/NALwerte ersetzt werden, ohne eine Assertion zu entfernen. Messfenster und mindestens 45 Frames bleiben. Der Fixtureblock endet einschließlich Nativefinish vor der vorhandenen `hostPrereqs`-/leere-`hostSessions`-Assertion; ein registrierter Fixturechannel darf dieses bestehende Ergebnis nicht verfälschen.

**Barcode-Decoder:** Identische Capturekonfiguration, aber `decoderReader=true`. Encoder und Syntheticquelle erhalten echte Budgets/Surfacealias. Die unbeschränkte `LinkedBlockingQueue<Triple<ByteArray,...>>` wird durch `ScreenCaptureQueue` ersetzt: höchstens zwei gleichzeitig besessene AUs, einschließlich Produzent, Queue, für `-3` gehaltenem AU und laufendem Pull. Jede Nutzlast wird vor ihrer Kopie durch `budget.tryBuffer(TRANSPORT,length)` zugelassen; die Summe und Rollenlimits bleiben zusätzlich native geprüft. Die Zwei-AU-Grenze stammt aus dem vorhandenen Synthetic-Transportplan, nicht aus einer frei gewählten Queuegröße. Ein Produzent erwirbt vor Allokation einen Platz; `close` beendet begrenztes Warten. Der Sink kopiert ausschließlich innerhalb von `ScreenCaptureBuffer.access`. Negative Sinkrückgaben alleine reichen hier nicht als Fehlerbeleg: der nicht-hybride Encoderpfad ignoriert den Rückgabewert. Deshalb speichert die Queue einen tatsächlichen Fehler für den Test und beendet die Produktion; sie behauptet bei fehlendem Claim keine erfolgreiche Übergabe.

`PictureSource.pull` hält den Besitzer während der vollständigen Kopie ins bereits vorhandene Decoderziel. Zu kleiner Zielpuffer liefert wie bisher `-3`, schreibt die erforderliche Länge und behält denselben AU samt Platz; kein neuer Clone bezahlt eine zweite Ablage. Nicht-Schlüsselbilder vor Start werden mit Besitzerende verworfen. Nach erfolgreicher Kopie oder abschließendem Verwerfen werden Bufferlease und Platz genau einmal geschlossen. Die bisherigen `fullGop=(1L shl 40)+1`, `frameBase=1L shl 42`, 32-Byte-Header, Flags, Decoder-Sessionidentität `1L`, echte YUV-Barcodeauswertung und Präsentationsassertions bleiben. Diese Decoder-Sessionidentität ist das vorhandene synthetische PictureSource-Metadatum; sie wird nicht als Capturegrant-ID verwendet.

Das Readerbudget wird vor `ImageReader(...YUV_420_888,3)` gehalten. Beide tatsächlich erworbenen Images erhalten `use`/`finally`; ihre Planeansichten verlassen den jeweiligen Besitz nicht. Cleanup sperrt zuerst neue Queueübergaben und entblockiert den Produzenten; beendet Source und Decoder; lässt aktive Pullzugriffe auslaufen; schließt alle gehaltenen/queued AUs; beendet den Encoder; schließt den Reader; ruft `captureReleaseDecoderReader`, nach Entzug der Pullansicht `captureReleaseViewer` und danach retire/finish. Constructorfehler vor `decoder.start()` laufen durch dieselbe Besitzerkette. Der bestehende `ScreenDecoder.Listener.onEnded` wird zum CountDownLatch-Endesignal: `ScreenDecoder.run` ruft ihn im `finally` erst nach `releaseCodec`, Hybridcleanup und Entfernung aus der Decoderregistry auf (`ui/screen/ScreenDecoder.kt:118–128`). Das bisherige `stop()` alleine ist lediglich eine Anforderung. Späte MediaCodec-Präsentationscallbacks sind durch `running` und die Codecidentität gesperrt (`ScreenDecoder.kt:354–358`).

Die letzte Bestandslektüre zeigt außerdem `ScreenPullBuffers.prepare()` mit `ByteBuffer.allocateDirect(next.pullBytes)` ohne expliziten Free-Anschluss. Dieser Standardpfad bleibt unverändert. Für genau den Barcodefall erhält `ScreenPullBuffers` einen internen Konstruktor `ScreenPullBuffers(offered:()->ScreenBufferBudget?, fixedBuffer:ByteBuffer)` für eine bereits vorher besessene feste Ablage. `prepare` validiert hier Vertrag und genaue Kapazität, allokiert/vergrößert nicht; `takeFrom` übernimmt keine fremde Vorgängerablage in diesen festen Modus. Ein internes `relinquishFixedBuffer()` sperrt spätere Nutzung und entfernt nach `onEnded` die letzte Kotlinansicht. Der Fixturebesitzer hält den nativen Buffer während des gesamten Decoderlaufs und benutzt den schon vorhandenen Parameter `ScreenDecoder(...,bufferBudget={fixture.viewerBudget},pullBuffers=fixturePulls)`. Nach `onEnded`, Readerende und `relinquishFixedBuffer` beendet `captureReleaseViewer` die tatsächliche native Ablage und erst danach Viewer-W. Weder `ScreenBufferBudget.LEGACY` noch ein nackter Deskriptor ersetzen diese echte Zusage. Das belegt die Codec-/Transport-/Capturegrenze dieses Barcodefalls; es ist ausdrücklich kein GC-Freigabebeleg des unveränderten produktiven `allocateDirect`-Standardpfads. Main bestätigte diesen engen Anschluss am 2026-10-26 ausdrücklich; ein allgemeiner Screenviewer-Umbau ist ausgeschlossen.

**Stillintervall:** Ersatz ist `ScreenStillSource(fixture.session,{width to height}) {buffer,length,generation -> ... }`. Die Geometrie wird unmittelbar vor Open aus dem aktuellen Display bestimmt und bleibt während dieses Falls konsistent; Geometriewechsel ist ein Fehler beziehungsweise verlangt einen eigenen neuen zugelassenen Fixtureblock. Der Sink prüft die echten Dimensionsbytes und exakte Nutzdatenlänge, ruft das bestehende `NativeBridge.screenPushAdmitted(session,generation,buffer,0,length,0L,0x2)` auf und lässt `captureConsumeStill` genau das soeben angenommene Nativepicture abholen. Die vollständige bestehende Signatur lautet `(Long,Long,ByteBuffer,offset:Int,length:Int,ptsUs:Long,flags:Int):Int`. Der Fixtureconsumer liest `channel.next_picture(Duration::ZERO,...)` synchron, prüft Stillflag/Dimensionen/Nutzlastumfang und `AdmittedBytes.belongs_to` gegen den echten Grant der erwarteten Generation und lässt die tatsächlichen `AdmittedBytes` anschließend fallen. `Picture` besitzt keinen eigenen Generationswert; der Plan erfindet keinen solchen Feldbeleg. Ein leerer Channel, ungültige Länge oder abgelehnter Push ist ein Fehler. Die bisherigen vier realen Accessibilityantworten und die Abstandsassertion `>=MIN_INTERVAL_MS` bleiben. `ScreenStillSource.close()` steht im `finally`; nach Callbackabschluss wird auf reale Restclaims gewartet.

**Secure-Still:** Der gemeinsame Helfer wird zu `still(service,budget,width,height):ScreenCaptureBitmap`. Aufruf ist `service.still(budget,width,height) { result -> ... }`. Er besitzt einen synchronisierten Resultatzustand: ein rechtzeitig eingetroffenes Resultat geht genau einmal an den Aufrufer; nach Timeout/Abbruch wird jedes verspätete Resultat im Callback geschlossen. Der Aufrufer verwendet `use { darkShare(it.bitmap,...) }`. `FLAG_SECURE` wird in `finally` entfernt. `dark>0.9` bleibt ein realer Pixelscan; kein Wechsel auf `takeScreenshotOfWindow`, kein Ersetzen durch ein erwartetes Securityfehlerobjekt und keine Testannahme, dass der Callback immer vor dem Timeout kommt. Das Nativefinish wartet zusätzlich auf Hardwareclaimende nach dem Callback, da dessen Signalisierung vor dem Produktions-`finally` liegen kann.

**Projection-Snapshot:** Systemzustimmung, `ScreenHostControl.probe`, der vorhandene `ScreenHostService` und `PROBE_SESSION=-1` bleiben als Permission-/Foregroundpfad. Ein separater positiver `captureOpen` für PROJECTION_TILES finanziert die bisherige Viertelgeometrie, bevor Reader/Display entstehen. Die Fixture verwendet `ScreenProjectionFrames(width:Int,height:Int,budget:ScreenCaptureBudget,submit:(Long,ScreenCaptureBuffer)->Boolean,failed:(String)->Unit)` sowie `captureStill(frame:Long,sink:(ScreenCaptureBuffer)->Boolean)`, statt einen unbudgetierten rowStride-breiten Java-Bitmapklon anzulegen. Der nie erwartete reguläre `submit` meldet einen Fixturefehler; der Still-Sink scannt synchron innerhalb `pixels.access`, signalisiert das echte Ergebnis und liefert erst danach `true`. Der Pixelscan berechnet denselben Dunkelanteil; `dark<0.95` bleibt. Reader und Quelle bleiben echte MediaProjection-Ressourcen. Nach dem vorhandenen Framezeitlimit werden VirtualDisplay, Frameshandler/Images/Reader, Projection und Probezustand geordnet beendet. `ScreenProjectionFrames.close()==true` bestätigt Handlerende; `false` ist kein Freigabesignal. Die genaue Plane-/Rowstride-Normalisierung bleibt die vorhandene produktive Implementierung.

**Nicht veränderte Prüfungen:** `screen_task_android_gesture_tap_lands`, `screen_task_android_text_set`, `screen_task_android_consent_action_is_activity_intent`, `perm_task_android_screen_approval_actions_are_activity_intents` und `screen_task_android_grid_switch` bleiben unter denselben Methodennamen und Assertions registriert. `BatchesCaptureTaskTest.unknownSessionCannotObtainPixelsOrExternalResourceCredit` bleibt als echter negativer JNI-Beleg erhalten. Kein Fall erhält `@Ignore`, `Assume`, Dummycode oder einen neuen Erfolgspfad bei fehlender Systemfreigabe.

Direkt zu lösender Completionpunkt: `SyntheticSurfaceSource.stop()` hat derzeit keinen Rückgabewert nach seinem 1000-ms-Join. Festgelegter Anschluss ist `stop():Boolean` mit tatsächlichem `!worker.isAlive` sowie `awaitStopped(timeoutMs:Long):Boolean`, das ausschließlich denselben Worker joined, vom Worker selbst nicht auf sich wartet und keine Lease schließt. `timeoutMs=0` darf nur vom nachgelagerten Retirementworker für vollständiges Join verwendet werden, nie vom UI. Das Worker-`finally` bleibt der einzige Schließort seines Surfacealias. Ein ruhiger Framezähler beweist kein Workerende.

Bei `stop()==false` bleiben Encoder und Surface in einem nachgelagerten Besitzer bis zum tatsächlichen Workerende. Der enge gemeinsame Helfer `ScreenSyntheticRetirement.kt::retireSynthetic(source:SyntheticSurfaceSource,encoder:ScreenEncoder,onFinished:(Result<Unit>)->Unit)` übernimmt genau dieses Paar: kein Encoderrelease vor `awaitStopped(0)==true`, danach einmaliger Encoderrelease und tatsächliche Abschlussmeldung. Die bestehenden Source-/Encoder-Ablösungen in `ScreenHostControl.kt` verwenden denselben Endevertrag; Feldnullen alleine darf den Altbesitz nicht verlieren. Die Fixture kann währenddessen ihren Timeout melden, hält aber ihr Nativefixturehandle bis zum tatsächlichen Callbackende. Threadstarterfehler oder Encoderreleasefehler bleiben Fehler und behalten ungeklärten Besitz; kein Catch darf Erfolg melden. Main bestätigte diesen eigenen kohärenten Meilenstein ausdrücklich. Er ist ein direkt betroffener Kotlin-Lebensdaueranschluss, kein zusätzlicher Native-Produktzugang.

### D5 — Tatsächlicher Login-ViewerFrame→JNI→HWUI-Beleg

`loginOpen` akzeptiert ausschließlich einen vom produktiven `LoginImagePane.openHandle()` erzeugten Owner. Es verwendet `login_images::bind(owner,session,heapOffer)` und damit genau dessen vorhandene `Arc<ImageAdmission>`. Die Session-ID kommt aus dem bisherigen `login_view`-ID-Zähler und kann keine echte Sitzung überschreiben. Die featuregebundene Kandidatenregistry besitzt maximal einen noch nicht abgeholten ViewerFrame je Fixture. Sie ist ausschließlich vom separaten Debug-JNI-Einstieg erreichbar.

`loginEnqueue` erzeugt deterministische BGRA-Pixel über `CaptureImageBuilder::new_local(size,FrameLimits::LEGACY,&images,ImageRole::ViewerInput,source,generation)`, füllt erst nach erfolgreicher Slicezusage und friert das Bild ein. `ViewerFrame` enthält nichtnull Frame-ID, tatsächliche Geometrie und einen gültigen, ausdrücklich synthetischen Fixture-FrameToken. Keine Java-Pixelarrayübergabe und keine nachträglich angehängte Lease. Ein bereits queued Kandidat wird vor Ersatz tatsächlich fallen gelassen; extern übernommene Raw-/Bitmapowner behalten ihre Plätze. Neue Kandidaten unterliegen den vorhandenen zwei Rawplätzen.

`login_view::take_image_frame` hat ausschließlich im Fixturebuild einen Lookup für registrierte Fixture-IDs. Er überträgt diesen echten ViewerFrame; für reguläre Sitzungen bleibt der vorhandene `ViewerSession::take_frame`-Pfad unverändert. `login_close`/Ownerclose beenden auch den Fixturekandidaten und die vorhandene Bindung. Der Releasebuild enthält diesen Zweig nicht. Die Einspeisung behauptet keine Maschinen-/Challenge-/Rechteprüfung des Netzwerkeinstiegs; Produkt-Eingaberouten erhalten daraus keine Freigabe.

Neu in der bestehenden `LoginImageTaskTest`-Klasse: `viewer_frame_jni_ndk_bitmap_retires_after_hwui_destroy`. Der Fall verwendet `LoginImagePane()` mit `NativeLoginBitmapCalls`, `loginOpen`, ein kleines mehrzeiliges Bild mit unterschiedlichen Rot-/Grün-/Blauwerten und `pane.take(session)`. Er verifiziert den echten Deskriptor und leiht über `NativeBridge.loginBitmapBorrow` kurz das tatsächliche Bitmap. Einzelpixelprüfung belegt BGRA→RGBA und Zeilenadressierung; `Bitmap.config`, Geometrie, tatsächliche RowBytes und AllocationByteCount werden protokolliert. `loginBitmapEndBorrow` folgt in `finally`; nach Ende dieser Leihe werden keine Pixel mehr gelesen. Der Test darf eine Javaobjektreferenz ausschließlich zur späteren `isRecycled`-Beobachtung behalten.

Danach wird dasselbe `LoginPicture` über vorhandene `LoginBitmapSurface`/`LoginBitmapRenderer` real gerendert und die echte Präsentation abgewartet. Der vorhandene Worker-Gate-Mechanismus wird für den gezielten Endebesitz verwendet: während das Cleanup hinter dem Gate wartet, werden Bild/Renderer/Pane geschlossen und weiterhin gehaltene native Bitmap-/Renderzielbytes sowie `isRecycled=false` beobachtet. Nach Gatefreigabe müssen echter Rendererabbau, Borrowende, native Bitmapretirement, Recycling, GlobalRef-Ende und Renderzielrelease eintreten. `loginState` liest hierfür die tatsächlichen, nach Owner gefilterten `BITMAPS`-/`TARGETS`-Einträge und `ImageAdmission.live_bytes/live_count`; es hält selbst keine Bitmap-/Slicealiasse. Statusdaten enthalten Owner/Generation, Raw-/Bitmap-/Renderzielzählung, Borrowanzahl, Retiring/Locked und gebuchte Bytes, keine simulierten Erfolgssignale. Registrierung nach dem produktiven `fill` erfordert bereits den erfolgreichen NDK-Unlock; beobachtete Pixel plus veröffentlichtes, nicht gesperrtes Objekt bilden den Positivbeleg.

Zusätzlich wird `viewer_frame_jni_texture_detach_keeps_owner_until_renderer_ends` mit echtem Nativeowner und demselben kleinen Aufbau ergänzt. Seine echte SurfaceTexture bleibt nach Detach bis zum Rendererabschluss besessen und ist anschließend freigegeben. Die beiden vorhandenen Fälle mit `Calls` bleiben unverändert als eigene, präzise Worker-/HWUI-Reihenfolgebelege erhalten.

Die minimale Rollen-/Handleabsicherung im echten Anschluss umfasst: falsche Session oder Generation liefert kein Borrow; ein dritter lebender Bitmapkandidat erzeugt keine dritte Bitmap und verbraucht den queued ViewerFrame nicht; nach tatsächlicher Retirementfreigabe kann dieser Kandidat regulär übernommen werden. Dafür eignet sich die zusätzliche Methode `viewer_frame_jni_bitmap_slots_wait_for_real_retirement` in derselben Klasse. Kein Test verringert die zwei Raw-/zwei Bitmap-/zwei Renderzielplätze oder vergrößert sie beim Sessionwechsel.

`loginFinish` entfernt lediglich einen vollständig abgeschlossenen Fixtureeintrag: kein queued ViewerFrame, null Livebytes/Livecount für dessen Pane, kein passender BITMAPS-/TARGETS-Eintrag. Ein verbleibender gesperrter oder quarantänisierter Bitmapowner ist ein echter Fehlerbeleg und bleibt gehalten; Finish löscht ihn nicht künstlich. Plattformfehler bei Bitmapcreate, NDK-Lock oder Unlock werden durch diese positiven Fälle nicht erzwungen. Die Suite muss fehlende reale Fehlerbelege ehrlich ausweisen und darf sie nicht aus einem Fake-Latch ableiten.

Präzisierung des Main vom 2026-10-26: Die produktive Kotlinübernahme erhält `NativeBridge.loginBitmapDiscard(owner:Long,rawDescriptor:String)` als Rollback, wenn nach JNI-Publication JSONparse oder Owner-/LoginPicture-Konstruktion scheitert. Beim gezielten Quellabgleich war dieser neue Symbolanschluss noch nicht vorhanden; er bleibt eine ausdrücklich angekündigte Main-Integrationsabhängigkeit. Der Nachplan ergänzt die vierte echte Methode `viewer_frame_jni_discarded_descriptor_releases_native_owner`: tatsächlichen ViewerFrame einspeisen, echten Rohdeskriptor durch `NativeBridge.loginBitmapTake` erhalten, vor Übernahme reale Bitmap-/Metadatenslices und Registryeintrag beobachten, eine Übernahme ausdrücklich verwerfen und den produktiven `loginBitmapDiscard` mit dem ursprünglichen unveränderten Deskriptor aufrufen. Eine vorherige kurze echte Borrow-/EndBorrow-Paarung kann die Bitmapreferenz für `isRecycled` sichern. Nach Discard müssen Recycling, GlobalRef-Ende und Sliceende tatsächlich beobachtbar sein. Falscher Owner beziehungsweise ein fremder Deskriptor darf den gültigen Besitzer nicht beenden. Der Fall behauptet keinen automatisch erzeugten Kotlin-OOM-/Parserfehler; der automatische Catch-/Rollbackanschluss bleibt zusätzlich am produktiven Übernahmecode zu belegen.

Main ergänzt außerdem `hasWindowFocus` und `Session.focused` vor der produktiven `presented`-Freigabe. Alter **und** neu gestarteter Draw nach Fokusverlust dürfen keine neue gültige Präsentations-/SAS-Grundlage erzeugen. Dieser erwartete Integrationsbeleg wird dem bestehenden Login-UI-/Sessionfall von L8 zugeordnet. Der direkte Bitmapfixture-Einstieg besitzt absichtlich keine reguläre `ViewerSession` und keine Eingaberechte; eine dort ohnehin abgelehnte Eingabe wäre kein Fokusnachweis. Die konkreten neuen UI-/Sessionguardstellen liegen außerhalb der hier gelesenen Fläche und bleiben Main-/Loginautorbeleg. Der JNI-/HWUI-Bitmapfall erhält dafür keinen künstlichen Erfolgsmarker.

### D6 — Remote-Build, Registrierung und Releasegrenze

Main integriert diesen Anschluss in die bereits eine Tasksuite. Der bestehende Weg ist `.github/workflows/review-task.yml` → `android/test-android-task.sh review-build` → vorhandene Debug-/Test-APK-Artefakte mit Kandidatenhash → `native/review-task-device.py`. Dieser Plan führt keinen Aufruf aus und erzeugt keine zweite Workflowdatei oder Testmatrix.

Der derzeit gelesene Reviewbuild verwendet Rust stable, `cargo-ndk` 4.1.2, Plattform 30 und ABI x86_64; Gradle baut `assembleDebug`, `assembleDebugAndroidTest` und die ausgewählten vorhandenen JVMklassen. Das Feature wird nur für diesen Instrumentationskandidaten ausdrücklich als `--features android-instrumentation-fixtures` beim Paket `smart_explorer_android` gesetzt. Das Feature wird nicht pauschal in Dependencydefaults oder Umgebungs-RUSTFLAGS eingetragen. Der historische `android-build`-Einstieg darf nicht versehentlich denselben ausgewählten Instrumentationsfall ohne erforderliches Feature ausführen; seine Aufrufauswahl wird von Main auf denselben expliziten Parameter ausgerichtet, ohne einen zweiten Suiteaufruf hinzuzufügen.

Fixture-SOs werden ausschließlich unter `android/app/src/debug/jniLibs/<abi>/libsmart_explorer_android.so` erzeugt. Der Releasepfad `android/build-release-apk.sh` baut weiterhin ohne Fixturefeature und darf keine Debugablage einsammeln. Um das bestehende gemeinsame `src/main/jniLibs` nicht mit Debugbytes zu vermischen, legt `android/app/build.gradle.kts` die JNI-Source-Sets explizit fest: `main.jniLibs` leer, `debug.jniLibs=src/debug/jniLibs`, `release.jniLibs=src/main/jniLibs`. Damit bleibt der bestehende Releaseausgabeort erhalten und dieselbe Bibliothek wird nicht aus zwei Source-Sets gepackt. Main prüft die betroffenen vorhandenen Developer-/Remote-Aufrufer im selben Integrationsschritt; die Planung behauptet keine bereits passende Konfiguration.

Remoteprüfung am tatsächlich gepackten APK, nicht nur am Cargokommando: Debug-Ziel-APK enthält die separate Bridgeklasse und deren erwartete Fixture-JNI-Symbole; Test-APK benutzt diese Klasse und enthält keine zweite App-SO. Der normale Release-APK enthält weder die Debugklasse noch Symbole mit `Java_app_smartexplorer_android_core_AndroidImageFixtureBridge_`. Symbolprüfung erfolgt über das vorhandene NDK-LLVM-Werkzeug auf der tatsächlich gepackten SO; DEX-/APKprüfung verwendet vorhandene Android-SDK-Werkzeuge. Die Prüfung wird beim einen regulären terminalen Release an dessen ohnehin erzeugten Bytes ausgeführt; kein eigener vollständiger Releasebuild nur zum Ausprobieren. Ein fehlender Symbolprüfer ist ein vorher zu klärender Runneraufbau, kein Anlass für einen zusätzlichen Vollbuild.

Die gelesene Devicekonfiguration ist API 34, `google_apis`, x86_64, Pixel-6-Profil, drei Kerne, 3072 MiB Emulator-RAM und SwiftShader im vorhandenen isolierten Emulator. JDK 17, compileSdk/targetSdk 36 und minSdk 30 stammen aus der vorhandenen Konfiguration. Emulator-RAM ist **kein** Beleg für den tatsächlichen Appheap. Die Suite protokolliert `ScreenFrameLimits.workingBytes()`, beide realen W-Beträge im Barcodefall, Host-X, Planpeak und reale Viewer-Packetgrenzen. Reicht das Angebot nicht, scheitert der Fall vor Codec-/Readeranlage; es gibt keinen stillen Skip und keine Produktoption zum Anheben des Heaps.

Das vorhandene Deviceprogramm ermittelt den Instrumentationrunner über `pm list instrumentation`, installiert die zum Kandidaten gehörenden APKs, setzt isolierte Appdaten und evaluiert die tatsächlich erwarteten Methoden. Main registriert die drei bereits ausgewählten `ScreenTaskTest`-Methoden plus die direkt betroffenen Stillintervall-, Securestill- und Projektionssnapshotmethoden sowie `BatchesCaptureTaskTest` und die bestehende `LoginImageTaskTest`-Klasse einschließlich der vier konkret neuen JNI-Methoden im zentralen Taskmanifest. Die vollständige unbeteiligte Gesten-/Text-/Gridtestsammlung wird nicht zusätzlich ausgelöst. Der Inhalt dieses Taskmanifests war nicht Teil der Lesefläche; seine aktuelle Registrierung ist daher noch vom Main zu prüfen. Keine manuelle Sammlung einzelner Instrumentationsaufrufe wird parallel gestartet.

Die bestehenden Accessibility- und MediaProjection-Systemaktionen werden nur auf dem von der Suite besessenen Emulator ausgeführt. `FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES`, aktivierter `ScreenControlService`, `canTakeScreenshot`, Foreground-MediaProjection-Dienst und Systemzustimmung bleiben echte Voraussetzungen. Fehlende Codecunterstützung, verweigerte Zustimmung oder fehlende Servicebindung sind benannte Fehl-/Umgebungsbelege und keine bestandenen Fälle. Für die zusätzliche JNI-Grenze sind keine Liveprovider-Zugangsdaten erforderlich.

Der Remotejob behält mindestens 30 Minuten Timeout; die vorhandenen Androidjobs liegen bereits darüber. Nach Integration aller Auftragsänderungen führt Main genau dieselbe vollständige Tasksuite aus; nur ein erforderlicher Fix rechtfertigt ihre Wiederholung. Dieser reine Planungsauftrag löst weder Commit noch Push noch Release aus. Für die spätere vollständige native Umsetzung gelten die bereits beauftragte gemeinsame Suite und genau ein terminaler Remote-Release über den vorhandenen Wrapper.

### D7 — Kohärente Meilensteine und Akzeptanzsignale

Die Tabelle ist der endgültige zukünftige Arbeitsplan. Jeder Punkt bleibt bis zur Umsetzung und seinem tatsächlichen Remoteergebnis offen. Die erwarteten Resultate gehören gemeinsam in die eine Tasksuite; sie autorisieren hier keinen Testlauf.

| Meilenstein | Genaue Ergebnisfläche und Abhängigkeit | Erwarteter, tatsächlich beobachtbarer Beleg |
|---|---|---|
| F1 Feature-/Paketgrenze | `native/Cargo.toml`, `native/android-bridge/Cargo.toml`, `native/android-bridge/src/lib.rs`, neue Debugbridge; Main passt `android/app/build.gradle.kts`, `android/test-android-task.sh` und den Releasepackaging-Guard in `android/build-release-apk.sh` an. Grundlage für F2/F5. | Gepackter Instrumentationskandidat besitzt die erwarteten Aufrufe; normale Releasebytes besitzen keine Klasse/Exports. Fehlendes Feature erzeugt eindeutigen Build-/Aufruffehler. |
| F2 Reale native Fixtureadmission | Neue `capture_fixture.rs`, `android_image_fixtures.rs`, `android_image_fixture_jni.rs`; bedingte Module/Reexports; `bridge_channels.rs`; featuregebundener Beginbeobachter in `screen_host.rs`. Nach F1 und Main-/Kernintegration. | Vor Surface/Reader/Buffer existieren echte gebundene W/X-Zusagen. Unbekannte Session, zu kleiner tatsächlicher Heap, falsche Generation und überschrittene Besitzerrolle geben keinen Pixel-/Surfaceclaim. Stillgenerationen bleiben bis zum letzten echten Callback gezählt. |
| F3 Synthetic-Worker/Surfaceende | `SyntheticSurfaceSource.kt`, neuer `ScreenSyntheticRetirement.kt`, direkt betroffene Ablöse-/Teardownstellen in `ScreenHostControl.kt`, Cleanup im neuen `ScreenCaptureFixture.kt`. Nach F2. | Normaler Stop bestätigt wirklich beendeten Worker. Ein verzögertes Ende hält den Surfacealias; Timeout meldet Misserfolg und bewirkt keinen Encoder-/Surfacerelease. Worker-`finally` und nachgelagerter Retirementabschluss beenden echte Besitzer genau einmal. Der konkrete verzögerte Plattformfall ist remote nachzuweisen, nicht aus einer Schlafdauer abzuleiten. |
| F4 Bestehender Encoder-/Decoder-Rundlauf | Beide Codecmethoden in `ScreenTaskTest.kt`, neue `ScreenCaptureQueue.kt`, interner fester Konstruktor/Relinquish in `ui/screen/ScreenPullBuffers.kt`; vorhandener `ScreenDecoder`-Parameter und `onEnded` bleiben wiederverwendet. Nach F2/F3. | Alle bisherigen Benchmark-/SPS-/Frame-/Barcode-/Präsentationsassertions bleiben erfolgreich. Maximal zwei reale AU-Besitzer, gültiger `-3`-Retry, getrennte echte Host-/Viewer-W, Reader-/Pullende vor Viewerfreigabe; leere native Capture-Restclaims. |
| F5 Accessibility und Projection | Stillhelfer und drei bestehende Still-/Snapshotmethoden in `ScreenTaskTest.kt`; `ScreenCaptureFixture.kt`; bestehende Produktkonstruktoren unverändert. Nach F2. | Vier echte Stillantworten mit bisherigen Intervallen, echter schwarzer Secureinhalt, echte nichtschwarze Projectionpixel. Timeout/Spätcallback schließt das tatsächliche Bild, Hardware- und Readerbesitz enden erst nach ihren produktiven `finally`-Pfaden. |
| F6 Login-ViewerFrame/JNI/HWUI | Neue `login_image_fixture.rs`; bedingter Lookup/Close in `login_view.rs`; lesende Ownerzustände in `login_images.rs` und `login_bitmap_owner.rs`; vier neue Methoden in `LoginImageTaskTest.kt`. Nach F1 und übergebener L7/L8-Produktion einschließlich Main-Rollback `loginBitmapDiscard`. | Echte gebuchte BGRAquelle, produktives Bitmapticket, echte NDK-konvertierte Pixel/Stride, tatsächlicher Borrow und HWUI-Draw; verzögertes Close/Detach hält Nativebitmap-/Renderbytes; tatsächliches Ende recycelt und entfernt GlobalRefs/Slices. Dritter Bitmapkandidat wartet auf reale Freigabe. Verworfener Deskriptor endet über den produktiven Rollback. Vorhandene Fake-Native-/echte-HWUI-Fälle bleiben erhalten. |
| F7 Gemeinsame Integration und Ergebnisgrenze | Main: vorhandenes Taskmanifest, `.github/workflows/review-task.yml`, `native/review-task-device.py`, Reportauswertung und terminale Releaseintegrität. Nach F1–F6 sowie sämtlichen weiteren Änderungen des aktiven Gesamtbatches. | Ein identifizierter Kandidat und dessen tatsächliche Suite-/APKbelege; keine entfernten Methoden oder ausgelassenen Fehler; reale externe Lücken werden benannt. Danach genau ein vorhandener kompletter Remote-Release, kein separater Fixture-Release. |

F3 ist ausdrücklich ein eigener Verhaltensmeilenstein. F4 erweitert keinen allgemeinen Screenviewerpfad: nur der vorhandene Barcodefall erhält feste native Pullablage und echte Viewer-W. F6 erweitert keinen authentifizierten Loginzugang: Fixture-IDs existieren ausschließlich im Debugfeature und gelten nicht als gültige reguläre Eingabesitzungen.

### D8 — Noch zu liefernde reale Belege und Grenzen

Noch **nicht** belegt sind Kompilierung der geplanten Signaturen/Featurekombination, Symbol-/DEX-Ausschluss im tatsächlichen Release-APK, laufende reale Codec-/Still-/Projectionbilder nach Migration, echte Ressourcenendebeobachtung im verzögerten Worker-/Callbackfall und der neue tatsächliche Login-JNI-/HWUI-Rundlauf. Es wurde hier keine dieser Ausführungen gestartet.

Vor Implementierung gleicht Main die aktuelle Auslagerung `host_codec_phase_plan.rs`, die beobachtete Android-Heapweitergabe und die vorhandene Taskmanifestregistrierung ab. `host_codec_phase_plan.rs` war beim Read noch nicht vorhanden; dessen künftiger Inhalt ist kein Bestandteil dieses Befunds. Die genaue interne Implementierung von `LoginBitmapRenderer`/`LoginBitmapSurface` wurde in diesem Auftrag nicht nachgelesen: Der Plan stützt sich auf die vorhandenen echten Aufrufe in `LoginImageTaskTest`, `LoginBitmapOwner` sowie den kanonischen L8-Vertrag. Der zuständige Login-/Androidautor bindet die neuen Fälle an seinen übergebenen tatsächlichen Rendererstand; keine hier erfundene private Renderer-API ist vorausgesetzt.

Die positiven JNI-Fälle beweisen keinen erzwungenen OEM-Fehler bei Bitmapcreate, Lock oder Unlock und keinen gesamten GPU-/BufferQueue-Speicherbetrag. Das bestehende native IPC-/PNG-Ergebnis und der neue ViewerFrame→JNI-Beleg sind zwei konkret benannte Grenzen; ihr Nebeneinander ist kein behaupteter authentifizierter Android-Netzwerkdurchlauf. Die Barcodefixture beweist nicht das GC-Ende der produktiven Standard-Pullbuffer. Fehlende Liveprovider-Belege sind kein Releasegate; hier fehlt dafür auch keine Nutzerentscheidung.


## Projektbericht dieser Planungsarbeit

Gelesen wurden das Auftragsmanifest `/tmp/smartexplorer-android-capture-fixture-plan-scope.json` und die eigene Ergebnisdatei. Die folgende Liste nennt vollständig die Repositorydateien der tatsächlich erfolgten Lektüre beziehungsweise gezielten Suche. Ganze Dateien wurden dort gelesen, wo die oben belegten Besitzer-/Aufrufverträge dies benötigten; bei Architektur, Regeln, Buildskripten, Mobile-Reexports und großen Consumerdateien wurden relevante Abschnitte und Symboltreffer gelesen. Der abschließende auf exakt die Manifestpfade begrenzte `rg`-Abgleich erzeugt keinen Volllektürebeleg für die bloß gesuchten Randdateien.

- `.github/workflows/review-task.yml`
- `AGENTS.md`
- `android/app/build.gradle.kts`
- `android/app/src/androidTest/java/app/smartexplorer/android/task/BatchesCaptureTaskTest.kt`
- `android/app/src/androidTest/java/app/smartexplorer/android/task/ScreenTaskTest.kt`
- `android/app/src/androidTest/java/app/smartexplorer/android/task/TaskSupport.kt`
- `android/app/src/main/AndroidManifest.xml`
- `android/app/src/main/java/app/smartexplorer/android/api/ScreenApi.kt`
- `android/app/src/main/java/app/smartexplorer/android/core/NativeBridge.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/GestureMapper.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenCaptureBudget.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenCaptureBuffer.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenCodecLatency.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenConsentActivity.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenControlService.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenEncoder.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenFrameLimits.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenHostControl.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenHostNotifier.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenHostService.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenProjectionFrames.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenProjectionIdentity.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenStillSource.kt`
- `android/app/src/main/java/app/smartexplorer/android/screen/SyntheticSurfaceSource.kt`
- `android/app/src/test/java/app/smartexplorer/android/screen/ScreenCaptureBudgetTest.kt`
- `android/test-android-task.sh`
- `docs/ARCHITEKTUR.md`
- `docs/plaene/2026-10-24-batches/aufnahme-speicher.md`
- `native/Cargo.toml`
- `native/android-bridge/Cargo.toml`
- `native/android-bridge/src/lib.rs`
- `native/android-bridge/src/screen_capture_jni.rs`
- `native/android-bridge/src/screen_jni.rs`
- `native/review-task-device.py`
- `native/src/mobile/mod.rs`
- `native/src/mobile/os/shared/domains/args.rs`
- `native/src/mobile/os/shared/domains/mod.rs`
- `native/src/mobile/os/shared/domains/screen_family.rs`
- `native/src/mobile/os/shared/domains/screen_host.rs`
- `native/src/mobile/os/shared/domains/screen_host_tests.rs`
- `native/src/mobile/os/shared/domains/screen_view.rs`
- `native/src/mobile/os/shared/domains/screen_view_run.rs`
- `native/src/screen/core/capture_admission.rs`
- `native/src/screen/core/capture_source_contract.rs`
- `native/src/screen/core/frame_budget.rs`
- `native/src/screen/core/frame_memory.rs`
- `native/src/screen/core/image_admission.rs`
- `native/src/screen/os/android/batches_capture_task_tests.rs`
- `native/src/screen/os/android/bridge_backend.rs`
- `native/src/screen/os/android/bridge_channels.rs`
- `native/src/screen/os/android/bridge_source.rs`
- `native/src/screen/os/android/bridge_tests.rs`
- `native/src/screen/os/android/capture_admission.rs`
- `native/src/screen/os/android/capture_arena.rs`
- `native/src/screen/os/android/capture_prepared.rs`
- `native/src/screen/os/android/session_memory.rs`
- `android/build-release-apk.sh`
- `docs/plaene/2026-10-24-batches/login-bildspeicher.md`
- `native/src/mobile/os/shared/domains/login_images.rs`
- `native/src/mobile/os/shared/domains/login_view.rs`
- `native/src/daemon/os/shared/batches_login_image_task_tests.rs`
- `native/src/login/os/shared/viewer.rs`
- `native/src/login/os/shared/viewer_images.rs`
- `native/android-bridge/src/login_bitmap_jni.rs`
- `native/android-bridge/src/login_bitmap_owner.rs`
- `native/android-bridge/src/login_bitmap_pixels.rs`
- `android/app/src/androidTest/java/app/smartexplorer/android/task/LoginImageTaskTest.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/login/LoginBitmapOwner.kt`
- `native/src/screen/core/host_codec_adapters.rs`
- `native/src/screen/core/frame_size_caps.rs`
- `native/src/screen/core/capture_image.rs`
- `native/src/screen/os/mod.rs`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenDecoder.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenPullBuffers.kt`

Nur gesucht beziehungsweise als Randverweis betrachtet wurden `GestureMapper.kt`, `ScreenCodecLatency.kt`, `ScreenConsentActivity.kt`, `ScreenHostNotifier.kt`, `ScreenProjectionIdentity.kt`, `ScreenCaptureBudgetTest.kt`, `domains/args.rs`, `domains/screen_family.rs`, `domains/screen_host_tests.rs` und `android/bridge_tests.rs`; aus diesen Randdateien werden keine neuen Verhaltensgarantien abgeleitet. `native/src/screen/core/host_codec_phase_plan.rs` war trotz expliziter Lesefreigabe bei beiden gezielten Abrufen nicht vorhanden. Der zunächst freigegebene Fehlpfad `android/app/src/main/java/app/smartexplorer/android/screen/ScreenDecoder.kt` war ebenfalls nicht vorhanden und wurde nach Rückmeldung des Main durch den tatsächlich importierten `ui/screen/ScreenDecoder.kt`-Pfad ersetzt. Es fand keine Suche außerhalb der freigegebenen Fläche statt.

Einzige geschriebene Datei: `/tmp/smartexplorer-android-capture-fixture-plan.md`. Keine Repositorydatei wurde erstellt oder verändert; keine Kotlin-/Rust-/JNI-/Fixture-/Cargo-/CI-/Dokumentations-/Graphänderung und keine Gitaktion wurde ausgeführt.

Wesentliche Ergebnisse: sämtliche alten Encoder-, Synthetic-, StillSource- und Service-Stillaufrufe sind konkret angeschlossen; der unmittelbar betroffene Projection-Snapshot ist ebenfalls budgetiert geplant. Gewählt ist der bedingte native Fixturezugang mit gemeinsamer echter Admission und fehlenden Releaseexporten. Der Barcodefall besitzt getrennte echte Host-/Viewer-W und einen explizit beendbaren nativen Pullbuffer. Der Synthetic-Stop erhält einen eigenen echten Endevertrag. Die Loginergänzung durchläuft ab einem tatsächlich besessenen ViewerFrame die bestehenden produktiven JNI-/NDK-/Bitmap-/HWUI-Besitzer; vorhandene HWUI-Fakes bleiben mit ihrem tatsächlichen Belegumfang erhalten.

Entscheidungen: regulärer Peer-/Gatewaystart und Projectionprobe ersetzen keine Fixtureadmission; keine separaten Testpools im Appprozess; keine still entfernten Assertions; kein Timeout als Freigabe; keine neue Produkt-Einspeisefunktion im Release. Cargo-/CI-/Taskmanifest-/Releaseintegration verbleibt vollständig bei Main. Die geplante produktive Kotlinänderung betrifft ausschließlich den notwendigen Synthetic-Endevertrag und den internen festen Pullbufferanschluss dieses vorhandenen Barcodefalls.

Offen bleiben ausschließlich die in D8 benannten Integrations- und tatsächlichen Remoteausführungsbelege. Es fehlt keine Nutzerentscheidung. Der Plan ist abgeschlossen; Umsetzung und Ausführung wurden nicht begonnen. Die abschließende statische Textprüfung bestätigte das einzige zulässige Schreibziel, sämtliche verpflichtenden Altaufrufer-/Rollback-/Fokusplaneinträge, die vollständige Benennung aller Lesemanifestpfade und geschlossene Codeblöcke. Sie war keine Build-, Compiler-, Formatter- oder Testausführung.



## Integrationsabgleich Main, 2026-10-26

Die zeitgebundenen Lesebefunde oben bleiben historische Belege des Planers.
Inzwischen existieren `host_codec_phase_plan.rs` und der produktive
`NativeBridge.loginBitmapDiscard`-Anschluss samt Kotlin-Rollback. Der
Fixturautor erhält diese aktuellen Dateien als verbindliche Integrationsfläche.
Die zentrale Registrierung bleibt bei Main; die Änderung an D6 begrenzt den
bestehenden ScreenTaskTest-Anschluss auf direkt betroffene Captureverträge.

## F3-Zusatzbefund und Umsetzungsentscheidung, 2026-10-26

Die Autorlektüre des aktuellen `ScreenEncoder.kt` hat vor der Änderung einen
zweiten Endepfad belegt: Der Drainthread ruft `finishResources()` auch nach einem
spontanen Ausgabefehler auf. Dieser Pfad gab `inputSurface`, Codec und Claims frei,
selbst wenn `SyntheticSurfaceSource` noch einen Surfacealias besaß. Der geordnete
`retireSynthetic`-Aufrufer allein konnte diesen Pfad nicht absichern. Main hat die
Schreibfläche ausdrücklich um `ScreenEncoder.kt` erweitert und diesen Fehler F3
zugeordnet.

Entscheidung vor der Änderung: `retainSurface()` registriert vor Sourceanlage
einen wirklichen Produzentenalias beim Encoder. Dessen Abschlusswrapper beendet
den Alias erst im vorhandenen Worker-`finally`. Der Encoderdrain hält vor jeder
Ressourcenfreigabe bis zu genau diesem Aliasende an und joined anschließend den
tatsächlich schließenden Worker, bis dieser auch sein `finally` verlassen hat;
neue Aliasannahmen werden bei beginnendem Ressourcenabschluss gesperrt. `release()` verweigert einen noch
lebenden Surfaceproduzenten, bevor es EOS/Codecstop anfordert. Die vorhandenen
Projection-/Hybridpfade ohne Synthetic-Alias behalten ihren bisherigen Ablauf.
Ein Join-Timeout oder fehlgeschlagenes Workerende erzeugt keine Freigabe; der
bereits eingeführte Retirementbesitzer hält das tatsächliche Paar. Die Prüfung
bleibt statisch; ein erzwungener spontaner MediaCodec-Fehler und ein verzögertes
Plattformende sind weiterhin fehlende Remote-Laufzeitbelege.

Der Dokumentationskontext dieses Zusatzes: gezielter `git status --short --`
meldete die bereits von Main integrierte Planpfaddatei als untracked; der auf
denselben Planpfad begrenzte letzte Gitlog lieferte keinen Eintrag. Die gezielte
Quell-/Plansuche zeigte beide genannten Endepfade. Es wurde keine Gitmutation,
kein Build, kein Compiler, kein Formatter und kein Test ausgeführt.


## Quellkandidat F2–F6, 2026-10-26

Der delegierte Umsetzungsauftrag liefert Quelltext innerhalb der freigegebenen
Kotlin-/JNI-/Rust-Fläche. Er umfasst keine Build-, Test-, Commit-, Push-,
Graph- oder Releaseausführung. Die F1-/F7-Integration liegt bei Main. Der folgende
Stand beschreibt tatsächliche Quellanschlüsse, keine bestandenen Laufzeitfälle.

| Planpunkt | Vorliegender Quellanschluss | Statischer Kompatibilitätsbeleg |
|---|---|---|
| F2 | `capture_fixture.rs` bucht echte Host-W/X über den Prozesspool, bindet den vollständigen produktiven Ausgabeplan und registriert Channels ohne Start-/Endeevent. Decoderreader/Pullablage haben eine getrennte echte Viewer-W-Zusage. `screen_capture_begin` beobachtet nur erfolgreich erzeugte Generationen. | Bestehende Claim-/Buffer-/Release-JNI-Funktionen bleiben die einzige Captureallokation. Fremde Generationen scheitern an produktiver Grantidentität; Stillconsumer vergleicht tatsächliche Grant-, Source- und Generationsidentität. Retire/Finish zählen echte lebende Slices. |
| F3 | `stop`/`awaitStopped`, `ScreenSyntheticRetirement`, die Hostablösungen und der oben dokumentierte spontane Encoderendepfad halten das echte Source-/Encoderpaar bis zum Workerende. | Synthetic zeichnet unverändert per HardwareCanvas. Ein Timeout, Threadstarterfehler oder unbestätigter Encoderabschluss schließt keine möglicherweise benutzte Surface. Projection-/Hybridabschlussfolge ohne Synthetic-Alias bleibt erhalten. |
| F4 | Beide bestehenden Codecfälle verwenden echte Capturebudgets. Die Barcodequeue hält höchstens zwei reale native AU-Besitzer; `-3` behält denselben Besitzer. Feste native Pullablage wird nach Decoder-`onEnded`, Readerende und Entzug der Kotlinansicht beendet. | `1280×720`, 30 fps, 4 Mbit/s, Highprofil, mindestens 45 Benchmarkframes, erstes Schlüsselbild, Annex-B/SPS, Barcodeanstieg, vollständige GOP-/Frame-IDs und tatsächliche Präsentationsidentität bleiben als Assertions erhalten. Der produktive Standard-Pullbufferpfad bleibt erhalten. |
| F5 | Stillintervall und Securestill verwenden echte budgetierte Screenshots; der Stillconsumer nimmt das tatsächliche native Picture ab. Projection benutzt `ScreenProjectionFrames` mit Readerclaim und synchronem Pixelscan. | Vier Stillantworten, mindestens 333 ms Anfrageabstand, Secure-Dunkelanteil über 0,9 und Projection-Dunkelanteil unter 0,95 bleiben erhalten. Verspätete Screenshotantworten schließen ihren tatsächlichen Bitmapbesitzer; Fixturefinish wartet auf produktive Callback-`finally`-Claims. |
| F6 | Debug-ViewerFrame-Registry mit produktivem Pane/ImageAdmission, echten BGRA-Bildern und produktiver Bitmapticketübernahme. Vier neue `LoginImageTaskTest`-Methoden verbinden JNI, NDK, Borrow, HWUI, Detach, reale Slotretirement und Deskriptorrollback. | Die beiden vorhandenen Fake-Native-/echten-HWUI-Methoden bleiben eigenständig erhalten. Login-IDs stammen aus dem gemeinsamen Zähler; Fixtureframes sind keine regulären ViewerSessions und erhalten keine Eingabe-/SAS-Rechte. Status liest tatsächliche Owner-/Borrow-/Targetregister und ImageAdmission-Zähler. |

Die unberührten Screenmethoden für Gestentap, Textsetzen, Zustimmungs-Activityintents,
Genehmigungs-Activityintents und Grid-/Vollbildwechsel behalten Namen, Bedienung und
Assertions. `BatchesCaptureTaskTest` bleibt unverändert. Die beschriebenen
Desktop-/CLI-/MCP-Bildschirmfähigkeiten aus `docs/oberflaechen/bildschirm.md` erhalten
keine neue Dispatchroute und keine Fixtureeingabe.

Der zusätzliche Kotlin-APIanschluss verwendet denselben bestehenden JNI-Stand
0.22.4. Die Signaturen für explizite Fehlerweitergabe und Laufzeitstrings wurden
am 2026-10-26 gegen die Primärdokumentation für
[`Env::throw_new`](https://docs.rs/jni/latest/jni/struct.Env.html#method.throw_new)
und [`JNIString` 0.22.4](https://docs.rs/jni/0.22.4/jni/strings/struct.JNIString.html)
abgeglichen. Fehler enthalten die betroffene Fixtureoperation und ihren Grund.

Statischer Abschluss: Die zugewiesenen Kotlin-/Rust-Schreibdateien haben
lexikalisch ausgeglichene Klammern; die Rustdateien bleiben unter 500 Zeilen und
50 KiB. Dieses Ergebnis ist weder Typprüfung noch Compiler- oder Laufzeitbeleg.
Ein lokaler ASTparser war nicht installiert; es wurde keiner installiert. Keine
lokale Testsuite, kein Formatter und kein Compiler wurde aufgerufen.

Main hat die Feature-/JNI-Source-Set-/Modul-/Exportintegration gemeldet. Die
Suite-/Manifestregistrierung, Kandidatenintegration, zwei zusätzlich beauftragte
Reviews, Graphaktualisierung, Commit/Push, spätere Remoteprüfung und der gemeinsame
terminale Remote-Release bleiben vollständig bei Main. Die zwölf tatsächlichen
`android_fixture_*`-Exporte stehen in `android_image_fixtures.rs`.

Offen als reale Nachweise bleiben: Kompilierung aller geänderten Signaturen;
gepackte Debug-/Release-Klassen und Symbole; echte Codec-/Still-/Projectionbilder;
der neue ViewerFrame/JNI/NDK/HWUI-Rundlauf; verzögertes Synthetic-Workerende und
erzwungener spontaner MediaCodec-Drainfehler; verspätete Systemcallbacks sowie
OEM-Bitmapcreate-/Lock-/Unlockfehler. Ein ruhender Framezähler oder ein Gate allein
beweist kein Plattformende. Authentifizierter Android-Netzwerkempfang, vollständige
GPU-/BufferQueuebytes und das GC-Ende des unveränderten Standard-Pullbuffers sind
kein Belegumfang dieses Kandidaten. Es wird kein fehlender Laufzeitbeleg als
bestandener Fall oder als Produktfreigabe ausgewiesen.

### Ergänzung des Main: Synthetic-Standard, 2026-10-26

`v0.5.184:native/src/screen/os/android/bridge_backend.rs` belegt 1280×720
als bisherigen Standard bei fehlenden `wanted`- und Displayangaben. Die
aktuelle Prepare/Activate-Integration hatte daraus 0×0 gemacht; Kotlin meldet
für Synthetic genau diese Startmaße zurück. Der Anschluss verwendet deshalb
für ausschließlich `SourceId::Synthetic` wieder den bestehenden
`DEFAULT_SYNTHETIC_SIZE` aus dem gemeinsamen synthetischen Backend. Eine echte
Displayquelle ohne Plattformmeldung bleibt ausstehend. Die benannte C9-Fixture
`batches_capture_task_android_synthetic_without_display_keeps_defined_default`
prüft Standardmaße, weiterhin fehlende Aktivierung und Prepared-Abschluss in
der gemeinsamen Remote-Suite; sie wurde lokal nicht ausgeführt.
