# M14: Android-UI-Fixtures A01–A03

## Erstplan aus dem Auftrag

Der begrenzte Auftrag vom 2026-10-26 betrifft ausschließlich die
vorhandenen Navigations-, Loginbitmap- und Mediengeometrie-Fixtures.
Arbeitsstand ist `33257f82`; die tatsächliche Remoteevidenz gehört zum
laut Auftrag quellidentischen Kandidaten
`1ffbf4b0bc90b2479947680abcd0a5941cce00f8`. Alle Remote-Arbeitsstufen sind
laut Main beendet. Die aktuelle Scope-Datei und der wörtliche vollständige
Nutzervertrag wurden gelesen; sämtliche früheren Schreibfreigaben sind
für diesen Auftrag ersetzt.

Die beauftragten Ergebnisse sind gültige Compose-API-Nutzung, eine
isolierte Android-Application für die Bitmapbesitzerprüfung und korrekte
geometrische Zahlengleichheit. Die vorhandenen Fallnamen und vollständigen
Funktionsassertions bleiben erhalten. Produktionsstart, tatsächliche
Navigation, Login-/Präsentationsrechte, Bitmapbesitz und Medienbedienung
werden nicht für eine Fixture abgeschwächt. Die Systemrotation von Android
bleibt allein maßgeblich; es entsteht keine App-Rotationspolitik.

Die vollständig gelesene Android-Fehlerdatei belegt den frühen
WorkManager-Initialisierungsfehler in allen vier Loginbitmapfällen und
die beiden fehlschlagenden Offsetvergleiche. Mains abgeschlossener Plan
ordnet zusätzlich den ungültigen Top-Level-Import der Navigationsfixture
A01 zu. Die drei Befunde sind zunächst Fixturefehler; daraus wird kein
Produktfehler abgeleitet.

Vorläufige Planungstiefe ist 2: Die vorhandenen Fixtures berühren echte
Activity-/Intentnavigation, Android-Canvas-/Bitmapbesitz und die
Koordinatentransformation nach Systemrotation. Zu klären sind die genaue
Compose-Member-API der BOM-Version 1.11.4, der Applicationstart unter
Robolectric 4.14.1 und die konkrete Gleichheits-/Clampsemantik. Die
mitgelieferten Primärquellen und tatsächlichen Definitionen bestimmen
die Änderungen. Eine nur vom Nutzer zu liefernde Angabe ist nicht erkennbar.

Schreibbar sind ausschließlich die drei explizit zugewiesenen Fixtures
und dieser neue Bericht. Ein zusätzlich belegter Produktionsfehler wäre
eine konkrete Scopefrage an Main. Lokale Builds, Compiler, Tests,
Formatter, Server, Installationen, Unteragenten, Gitmutationen,
Graphupdates und Releasearbeit sind verboten. Main bestätigt das
Ergebnis ausschließlich über denselben gemeinsamen Remoteeinstieg.

Im Erstplan als nächster Schritt festgelegt: die eigene vollständige
Definition-/Aufruferkarte, danach der konkrete Erhaltungsplan vor den
Fixtureänderungen.

## Bestand und belegte Fehlerfunktionen (Plan B)

Der eigene Quellenabgleich bestätigt drei Fehler in den vorhandenen
Fixtures. Ein zusätzlicher Produktionsfehler ist damit nicht belegt.
`android-failures.json` wurde einschließlich sämtlicher sechs vollständigen
Stacktraces gelesen. A01 steht im abgeschlossenen Main-Plan und ist am
aktuellen Import sowie an der versionierten Memberdefinition nachvollzogen;
die Fehlerdatei selbst enthält für A01 keinen Geräte-Laufzeitbefund.

| Befund | Tatsächlich blockierte Aussage | Belegte Ursache |
|---|---|---|
| A01 | Bereits geöffnete Android-Einstellungen nehmen Programme-, Bildschirm-, Raster- und Anfrageziele sichtbar an, verlassen Diagnoseunterseiten und spielen erledigte Ziele nicht erneut ab. | `BatchesDeviceNavigationTaskTest` importiert `androidx.compose.ui.test.assertDoesNotExist`, obwohl die vorhandenen Aufrufe eine Memberfunktion von `SemanticsNodeInteraction` verwenden. Der Import verhindert die Instrumentationkompilierung. |
| A02 | Ein Softwarezeichen-Borrow und eine Viewreferenz halten ihre Bitmap bis zum tatsächlichen Abschluss; verspätete Präsentationscallbacks erteilen nach Release, Fokuswechsel und Schließen keine Eingabefreigabe. | Robolectric startet die Manifestklasse `SmartExplorerApp` vor dem Prüfkörper. Deren `onCreate` erreicht über `BackgroundController.onAppStart` und `apply` den im Fixtureprozess nicht initialisierten WorkManager. Alle vier Besitzerfälle scheitern an diesem gemeinsamen Startpfad. |
| A03 | Bildfokus über Hoch-/Querformatgeometrie, Nullversatz bei Zoom 1 und Panninggrenzen des tatsächlich eingepassten Bildes stimmen. | Der bisherige Objektvergleich prüft den gepackten `Offset`-Wert. `MediaGeometry.clamp` kann bei einer Nullgrenze durch `-x` oder `-y` negatives Null zurückgeben. Die identische geometrische Koordinate besitzt dann ein abweichendes Float-Vorzeichenbit. |

Die Zeilen 19 und 29 der Medienfixture erklären die beiden tatsächlich
beobachteten Assertionfehler. Die übrigen zwei Offset-Objektvergleiche
prüfen denselben geometrischen Vertrag und werden ebenfalls auf den
Koordinatenvergleich umgestellt. Ihre Eingaben und erwarteten Werte bleiben
identisch. Die bereits vorhandene Rundlauftoleranz für den Fokus bleibt
`0.0001f`.

## Direkte und indirekte Verknüpfungskarte

Die Karte verfolgt die drei geänderten Fixturegrenzen einschließlich der
unverändert angebundenen Produktionsstellen innerhalb der freigegebenen
Lesefläche. Untersuchte Aufrufstellen an der JNI-, FilesApi- und
Bildschirm-/Appdialoggrenze werden als Grenzen benannt; daraus wird kein
neuer Nachweis über deren native Implementierung abgeleitet.

| Bereich | Direkte Kette | Indirekte Verträge und Erhaltungsgrenze |
|---|---|---|
| Navigationsharness | `BatchesDeviceNavigationTaskTest.root` → echte `MainActivity.setContent` → `DeviceSettingsRoot`; `deliver` → `AppNav.intentFor` → vorhandene Activity → `MainActivity.handleIntent` → `AppNav.fromIntent` → `AppNav.send`. | `AndroidManifest.xml` bestimmt `singleTop`; der Intent behält `NEW_TASK`, `CLEAR_TOP` und `SINGLE_TOP`. Installationstoken aus privaten Preferences, Direct-/Room-Endpunktprüfung, Activityidentität und Zustand `RESUMED` bleiben geprüft. Die untrusted-Tokenfixture bleibt unverändert. |
| Navigationsbesitz | `AppNav` hält SharedFlow, Queue und neuesten Pendingrequest; `MainShell` wählt den Tab. `DeviceSettingsRoot` räumt nur für `deviceSettingsAreaFor` die eigene Log-/Über-Unterseite ab. `SettingsScreen` wählt den Gerätebereich. | Die vier Geräteanfragen bleiben `OpenApps`, `OpenScreen`, `ShowScreenGrid` und `ShowShareRequests`. `AppNav.consume` verwendet `compareAndSet`, damit ein neuerer Request erhalten bleibt. Gleichartige wiederholte explizite Requests erreichen die Flowkollektoren. `OpenLocation` bleibt im isolierten Root unbearbeitet und erhält die vorhandene Diagnoseansicht. |
| Sichtbares Navigationsziel | `SettingsScreen` → `ShareScreen`; dort `OpenApps` → `ShareDialog.Apps`, `OpenScreen` → geparstes `ScreenTarget` und `screenView`, Raster → `screenGrid`, Freigabeanfragen → vorhandener Status und Scrollziel. `ShareScreen` → `ShareDialogHost` → `AppsDialog`; Bildschirm-/Rasterzustand → `ScreenViewScreen` beziehungsweise `ScreenTilesScreen`. | Erst das Ziel konsumiert den Request. Die beiden Vorfahren konsumieren ihn nicht. Dialogschluss setzt ausschließlich den jeweiligen Dialog-/Viewzustand zurück. Die Fixture prüft sichtbare Ziele, deren Entfernung, den Gerätebereich und ausbleibende Wiederholung nach Bereichswechsel. Die Implementierungen der drei ausgehenden Dialog-/Viewfunktionen sind keine Änderungsfläche von M14. |
| Applicationgrenze | Robolectric → `@Config.application` oder Manifest-Application; Produktmanifest → `SmartExplorerApp.onCreate` → Preferences, Benachrichtigungen, Wakekeeper, Core, Taskkeeper, Exechost, Hintergrundcontroller und optionaler Widgetfeed. `BackgroundController.onAppStart` → `apply` → `scheduleWork` → `WorkManager.getInstance`. | Nur die Besitzer-JVM-Fixture bekommt die einfache `android.app.Application`. Die Navigations-Instrumentation behält die echte Activity und den echten Produktstart. Die vorhandenen Beispiele `DriveTaskProviderTest` und `BatchesScreenStateTest` verwenden dieselbe Isolation. Produktinitialisierung und Hintergrundverhalten werden nicht angepasst. |
| Bitmapbesitz | `LoginBitmapOwnerTest.Calls` → reales Android-`Bitmap`/`Canvas` unter `GraphicsMode.NATIVE`; `LoginPicture` → `LoginBitmapOwner` → `retain`, `release`, `borrow`; letzter Release → gemeinsamer Pane-Worker → `retire`; `LoginDrawBorrow.close` → `endBorrow`. | Die simulierte JNI-Grenze der Fixture kontrolliert Retire und Borrow; sie ersetzt keine Canvasrasterung. Deskriptor, Owner-/Picture-/Generationsidentität, einmaliger Abschluss, Workerbarriere, roter Ausgabepixel und spätere Recyclingsignale bleiben unverändert. Der Softwarepfad darf weiterhin kein Renderziel buchen. |
| Produktiver Loginrenderpfad | `ShareDialogHost.LoginSetup` → `LoginTargetDialog` → geprüfter Maschinenpin/Zielbenutzer → `LoginViewScreen` → Pane und `LoginSession`; State-/Pictureflows → `LoginCanvas`. Softwarecanvas → `LoginDrawBorrow.draw`/`close`; Hardwarefläche → `LoginBitmapSurface` → `LoginBitmapRenderer`. | Der Renderer hält Retain/Borrow bis zum bestätigten öffentlichen Abbau; ein Hardwarebild wird erst nach Submission, Producercommit und Consumerupdate bestätigt. `LoginCanvas` prüft Identität, Präsentationsstamp, lokale Generation, vollständige Sichtbarkeit und Fokus. Größe, Fokusverlust und Detach lösen Release aus. Keiner dieser Verträge wird von der Applicationisolation verändert. |
| Loginpräsentation und Eingabe | `LoginSession.presentationStamp`, `presented`, `focus`, `release`, `close` → lokale Epoche, Präsentationsepoche, Drawntoken und `input_ready`; echte Starts → `LoginApi` → `Core.call`/`request` sowie binäre JNI-Aufrufe der Pane. | Der Sessionstart wird in den vier Besitzerfällen nicht aufgerufen. Ihre Session-ID bleibt `0L`; `LoginApi.releaseFast` und `closeFast` rufen JNI nur bei positiver ID. Deshalb benötigt dieser begrenzte Prüfkörper keinen produktiven Corestart. Die Queue bleibt begrenzt, Fokusbestätigung und Tokenbindung bleiben Pflicht, Close beendet Scope und Queue; kein Eingaberetry wird ergänzt. |
| Mediengeometrie | `BatchesMediaStateTest` → `MediaGeometry.fit`, `offset`, `focus`, `clamp` → Compose-`Offset` → `packFloats`/`toRawBits`; neue Fixturehilfe → JUnit-Floatvergleich mit Delta null für x und y. | Hochformat → Querformat → Hochformat prüft denselben Fokus bei Zoom 4; beide Zoom-1-Versätze müssen geometrisch null bleiben. Das 1600×400-Bild wird auf 400×100 eingepasst; Pan bleibt bei Zoom 2 auf x = ±200 und y = 0 begrenzt. Eine von null verschiedene Koordinatenabweichung bleibt ein Fehler. |
| Medienoberfläche und gespeicherter Zustand | `MediaSet` bestimmt Gruppe, Reihenfolge und Start; `MediaViewer` hält Pager/Seitenzustand → `ImagePage` → pfadgebundenes `rememberSaveable` für Zoom/Fokus → aktuelle Layoutmaße → `MediaGeometry` → Grafiktransformation. Video/Audio → `PlayerPage` → `MediaPlaybackState`. | Das System bestimmt die Rotation. Manifest-Konfigurationsbehandlung, Bildfokus, Pager, pfadgebundene Wiedergabeposition und Wiedergabeabsicht bleiben erhalten. Der Player beachtet Sichtbarkeit und Audiofokus; Hintergrundrückkehr bleibt bis zum expliziten Play pausiert. Surfacewechsel und spätes Detach behalten ihren tatsächlichen Besitzer. |
| Medienbezug und Abbruch | `MediaViewer` → `loadMedia`; lokal `FilesApi.open`, bei Remote-/Archivquelle `FilesApi.fetch` und `awaitTask` → registrierte lokale Kopie. | Abbruch führt im bestehenden `NonCancellable`-Abschluss zu `cancelTask`, danach `clearFinishedTasks`. Lokale/Remote-Ortsidentität, Retry, Formatfehler und externes Öffnen bleiben erhalten. Kein Loader-, Dateipfad- oder Persistenzformat wird geändert. |
| Gemeinsamer Nachweis | `.github/workflows/review-task.yml` → vorhandener Review-Task-Einstieg → `android/test-android-task.sh review-build` → manifestausgewählte JVM-Klassen und Instrumentation-APK; vorhandene Geräteauswahl enthält die Navigationsklasse. | Main besitzt Manifest, CI und Ausführung. `BatchesMediaWakeTaskTest` enthält bereits den Systemrotations-/Wiedergabefall. `LoginImageTaskTest` enthält die zusätzlichen Hardware-/JNI-Besitzerfälle; diese werden nicht durch die isolierte Besitzer-JVM-Fixture ersetzt. M14 eröffnet keinen separaten Testlauf. |

## Quellenrecherche und Entscheidungsplan (Plan C)

Quellenstand und eigener Abgleich: 2026-10-26. `libs.versions.toml` bindet
Compose-BOM `2026.06.01`, Kotlin `2.4.20` und JUnit `4.13.2`;
`app/build.gradle.kts` bindet Robolectric `4.14.1`, echte Androidressourcen
für JVM-Fixtures und die vorhandene Compose-Testabhängigkeit.

1. Die [offizielle BOM](https://dl.google.com/dl/android/maven2/androidx/compose/compose-bom/2026.06.01/compose-bom-2026.06.01.pom)
   bestätigt `ui-test`, `ui-geometry` und `ui-util` jeweils als `1.11.4`.
   Die gelesene Paketquelle `SemanticsNodeInteraction.kt` definiert
   `fun assertDoesNotExist()` innerhalb der Klasse. Die vorhandenen
   Methodenaufrufe bleiben gültig; ausschließlich der Top-Level-Import
   entfällt. Ein APIwechsel oder ein Ersatz der Existenzassertions wäre
   unbegründet.
2. [Robolectric Config 4.14.1](https://github.com/robolectric/robolectric/blob/robolectric-4.14.1/annotations/src/main/java/org/robolectric/annotation/Config.java)
   definiert `application()` als klassenbezogenen Ersatz mit Vorrang vor
   der Manifestklasse. Gewählt ist derselbe einfache Applicationtyp wie
   in den beiden gelesenen Projektfixtures. Eine globale Testkonfiguration,
   eine zusätzliche WorkManagerinitialisierung oder eine Änderung am
   Produktstart würde die isolierte Besitzerfrage unnötig erweitern.
   SDK 34 und nativer Grafikmodus bleiben bestehen.
3. Die offiziellen [ui-geometry-Quellen 1.11.4](https://dl.google.com/dl/android/maven2/androidx/compose/ui/ui-geometry/1.11.4/ui-geometry-1.11.4-sources.jar)
   und [ui-util-Quellen 1.11.4](https://dl.google.com/dl/android/maven2/androidx/compose/ui/ui-util/1.11.4/ui-util-1.11.4-sources.jar)
   belegen den gepackten Long und die rohen Floatbits. Die Nullgrenzen sind
   am tatsächlichen `MediaGeometry.clamp` nachvollzogen. Gewählt ist ein
   privater Helper, der beide Komponenten prüft; keine Produktionsnormalisierung,
   Rundung oder neue Fehlertoleranz.
4. Die [offizielle JUnit-Quelle r4.13.2](https://github.com/junit-team/junit4/blob/r4.13.2/src/main/java/org/junit/Assert.java)
   wurde zusätzlich direkt abgerufen. Der Floatoverload nimmt Nachricht,
   erwarteten Wert, tatsächlichen Wert und Delta. `floatIsDifferent`
   akzeptiert gleiche Werte oder eine Differenz innerhalb des Delta.
   Delta `0f` akzeptiert damit beide Nullvorzeichen. Gegen die endlichen
   erwarteten Koordinaten scheitern sowohl von null verschiedene
   Abweichungen als auch NaN oder Unendlich. Die Fehlermeldung benennt
   die betroffene Achse. Es entsteht keine neue Bibliotheksabhängigkeit.

Der zweite Lückenabgleich vor der Umsetzung hat die tatsächlichen
Settings-/Shareverbraucher und den exakten JUnitoverload ergänzt. Main
hat ausschließlich diese vier zusätzlichen Quellpfade und die versionierte
JUnit-Primärquelle lesbar freigegeben. Es bleibt kein fehlender
Quellpfad für die drei geplanten Fixturekorrekturen offen. Der native
Rootgraph liefert für diese Kotlin-Fixturegrenze keinen AST-Beleg;
Graphlektüre außerhalb des Scopes und Graphupdates sind nicht erfolgt.

## Konkreter Fix- und Erhaltungsplan vor Umsetzung (Plan D)

| Schritt | Änderung | Erhaltungs- und Abnahmesignal |
|---|---|---|
| M14.1 Navigation | In `BatchesDeviceNavigationTaskTest.kt` genau den falschen `assertDoesNotExist`-Import entfernen. | Alle fünf Fallnamen, sämtliche Requests, Bereichs-/Detailwechsel, reale Activityzustellung, Tokenabwehr, sichtbaren Assertions und Nichtwiederholungsprüfungen bleiben identisch. Die Instrumentation wird über dieselbe Remote-Suite wieder erreichbar. |
| M14.2 Loginbitmap | Nur `@Config` in `LoginBitmapOwnerTest.kt` um `application = android.app.Application::class` ergänzen. | Alle vier Prüfkörper und der Calls-Fake bleiben identisch; SDK und `GraphicsMode.NATIVE` bleiben erhalten. Die Prüfkörper erreichen die reale Bitmaprasterung und die unveränderten Besitzer-/Präsentationsaussagen ohne Produkt-Appstart. |
| M14.3 Mediengeometrie | Alle vier Offset-Objektvergleiche in `BatchesMediaStateTest.kt` durch einen privaten komponentenweisen Helper mit JUnit-Delta `0f` ersetzen. | Beide Fallnamen, sämtliche Geometrieeingaben, ±200/Nullgrenzen, Bildgröße und Fokus-Rundlauftoleranz bleiben erhalten. Vorzeichenverschiedene Null ist gleich; jede geometrisch abweichende Koordinate bleibt fehlerhaft. |
| M14.4 Eigene anschließende Review | Nach allen drei Änderungen eigenen Diff, Kotlin-Syntaxbaum, Fallnamen, Prüfkörpererhalt, Quellenverträge und Bericht gegen diesen Plan prüfen. | Ausschließlich die drei Fixtures und dieser Bericht sind von diesem Autor geändert. Statische Prüfung wird ausdrücklich nicht als Laufzeitpass ausgegeben. Main erhält die unveränderten konkreten Fallnamen für denselben Remote-Einstieg. |

Die Reihenfolge M14.1–M14.3 ist fachlich unabhängig; M14.4 hängt von allen
drei Änderungen ab. Es werden keine neuen Prüffälle oder Produktionsfunktionen
hinzugefügt. Die tatsächlich benötigte Laufzeitbestätigung bleibt gemeinsam
bei Main. Diese vier Planpunkte stehen vor der ersten Fixtureänderung.

## Umsetzung und eigene anschließende Anti-Regressionsreview

M14.1, M14.2 und M14.3 sind quellseitig umgesetzt. Danach hat derselbe
Autor M14.4 anhand des vollständigen eigenen Produktdiffs, der zuvor
gelesenen Definitionen und der versionierten Primärquellen durchgeführt.
Es gab keine Übergabe eines Teils dieses Fünfteilzyklus an einen Reviewer
oder einen weiteren Autor.

| Prüffrage | Eigener anschließender Befund |
|---|---|
| Bleibt die tatsächliche Navigation erhalten? | Der gesamte Dateiinhalt der Navigationsfixture stimmt nach Abzug genau des einen Imports mit `1ffbf4b0` und `33257f82` überein. Alle authentifizierten Intents, Direct-/Room-Endpunkte, fünf Fallnamen, vorhandenen Log-/Überzustände, Geräteziele, Pendingprüfungen, Schließaktionen und Nichtwiederholungsassertions sind identisch. Die abgewiesene Tokenzustellung und der unbearbeitete fremde `OpenLocation`-Request bleiben ausdrücklich enthalten. |
| Verliert die Bitmapfixture ihren Gegenstand durch die Isolation? | Nach Abzug genau des einen `@Config`-Parameters ist der gesamte Dateiinhalt identisch. SDK 34, `GraphicsMode.NATIVE`, echte Bitmaps, echte Canvasrasterung mit rotem Ausgabepixel, Workerbarrieren, Retain-/Borrow-/Retireprüfung und alle Präsentations-/Fokussperren bleiben bestehen. Die einfache Application beseitigt ausschließlich den unbeauftragten Produktstart vor diesen Prüfkörpern. |
| Werden echte Geometriefehler toleriert? | Alle vier ursprünglichen Koordinatenpaare werden weiterhin geprüft. Der Helper nutzt die tatsächlich vorhandenen Floatoverloads mit `0f` für beide Achsen. Es gibt kein Runden, kein positives Epsilon und keinen Verzicht auf eine Achse. Die vorhandenen Größen- und Fokusassertions bleiben bytegleich. Die Produktion behält ihre geometrisch korrekte Clampsemantik einschließlich signierter Null. |
| Bleiben Systemrotation und Medienbedienung erhalten? | Keine Manifest-, Activity-, Viewer-, Bildseiten-, Player-, Loader- oder Playbackdatei ist von diesem Autor geändert. Die untersuchten Wege behalten Systemorientierung, pfadgebundenen Zoom/Fokus und Wiedergabezustand, Pager, pausierte Hintergrundrückkehr, Surfacebesitz, Downloadabbruch und externes Öffnen. Der unveränderte vorhandene Gerätefall kontrolliert Systemrotation und `SCREEN_ORIENTATION_UNSPECIFIED`; die Geometrie-JVM-Fixture wird weiterhin nicht als physischer Sensor-/Rotationsnachweis ausgegeben. |
| Bleiben Rechte, Identität und Ressourcenabschluss erhalten? | Navigationsinstallationstoken, Geräte-/Room-Identität, Requestbesitz, Loginpin/Zielbenutzer, lokale und native Präsentationsepoche, Fokus-/Eingabesperren, Bitmapabschluss und Medienabbruch liegen vollständig außerhalb des Schreibdiffs. Ihre aktuellen Anschlüsse sind in der Karte nachvollzogen. Es wurde kein Produktionszweig für eine Fixture abgeschwächt. |
| Stimmen Syntax und begrenzter Änderungsumfang? | Der vorhandene Tree-sitter-Kotlinparser parste alle drei geänderten Dateien ohne Fehlerknoten. Ein statischer Ganzdateivergleich rekonstruierte genau die geplanten Deltas und bestätigte zusätzlich die Quellidentität der beiden Ausgangskandidaten für diese Dateien. Die annotierten Fallnamen stimmen vollständig überein. `git diff --check` meldete für die drei Fixtures und diesen Bericht keinen Befund. |

Diese Autorprüfung führt keine Fixture aus. Sie startet weder Gradle noch
JUnit, Robolectric, Android, einen Compiler oder einen Formatter. Der
zunächst versuchte freigegebene Werkzeughinweis
`graphify-out/.graphify_python` existiert in der Arbeitskopie nicht;
der vorhandene Parser wurde direkt aus seiner bestehenden Umgebung
`/root/.local/share/uv/tools/graphifyy/bin/python3` geladen. Es wurde
nichts installiert und kein Graph erzeugt oder verändert.

Die geänderten Kotlinquellen umfassen 157 Zeilen/6686 Bytes für Navigation,
94 Zeilen/4472 Bytes für Bitmapbesitz und 37 Zeilen/1597 Bytes für
Mediengeometrie. Die vorhandene Formatierung bleibt erhalten; der neue
Helper ist normal mehrzeilig formatiert. Es gibt keine native
Quelländerung, keine Versionsänderung und keine neue Abhängigkeit.

## Konkrete vorhandene Fälle und Registrierung für Main

Die manifestausgewählten JVM-Klassen enthalten bereits
`app.smartexplorer.android.ui.login.LoginBitmapOwnerTest` und
`app.smartexplorer.android.ui.viewer.BatchesMediaStateTest`. Die vorhandene
Geräteauswahl enthält bereits
`app.smartexplorer.android.task.BatchesDeviceNavigationTaskTest`.
Für diese Korrektur ist keine zusätzliche Klasse und kein neuer Fall zu
registrieren. Main behält die vorhandene Auswahl im selben Remote-Einstieg.

| Vollständige Klasse | Unveränderte Methodennamen |
|---|---|
| `app.smartexplorer.android.task.BatchesDeviceNavigationTaskTest` | `batches_android_apps_request_from_existing_appearance_opens_once` |
| `app.smartexplorer.android.task.BatchesDeviceNavigationTaskTest` | `batches_android_apps_request_leaves_log_and_about_details` |
| `app.smartexplorer.android.task.BatchesDeviceNavigationTaskTest` | `batches_android_untrusted_device_intent_preserves_the_existing_detail` |
| `app.smartexplorer.android.task.BatchesDeviceNavigationTaskTest` | `batches_android_screen_and_grid_requests_leave_log_and_about_details` |
| `app.smartexplorer.android.task.BatchesDeviceNavigationTaskTest` | `batches_android_share_requests_leave_details_and_unrelated_requests_keep_them` |
| `app.smartexplorer.android.ui.login.LoginBitmapOwnerTest` | `software_raster_borrow_survives_state_owner_exchange_until_draw_returns` |
| `app.smartexplorer.android.ui.login.LoginBitmapOwnerTest` | `view_reference_keeps_source_after_latest_state_replacement` |
| `app.smartexplorer.android.ui.login.LoginBitmapOwnerTest` | `delayed_presented_callback_cannot_reauthorize_after_release_focus_change_or_close` |
| `app.smartexplorer.android.ui.login.LoginBitmapOwnerTest` | `a_new_draw_after_focus_loss_cannot_create_a_presentation_proof` |
| `app.smartexplorer.android.ui.viewer.BatchesMediaStateTest` | `batches_android_rotation_keeps_image_focus_in_image_coordinates` |
| `app.smartexplorer.android.ui.viewer.BatchesMediaStateTest` | `batches_android_letterbox_pan_is_clamped_to_the_fitted_image` |

Die Gerätefälle des vorhandenen `BatchesMediaWakeTaskTest` sowie die
Hardware-/JNI-Fälle des vorhandenen `LoginImageTaskTest` wurden als
Nachweisgrenzen gelesen. Diese begrenzte Fixturekorrektur fügt dafür
keine gesonderte Ausführung oder zusätzliche Abnahmekampagne hinzu.

## Vollständige Dateiauskunft für diesen Fixzyklus

Alle folgenden Repositorypfade beziehen sich ausschließlich auf
`/tmp/smartexplorer-review-fixes-ee75e91b`. Bei verknüpften Produktionsdateien
wurden die oben benannten relevanten Definitionen, Aufrufer oder
Konfigurationsabschnitte gelesen; diese Angabe beansprucht keine
vollständige semantische Review jeder Zeile dieser Produktionsdateien.

Vorgaben, Pläne, Nachweis und Suiteanbindung:

- `/root/.codex/AGENTS.md`
- `/tmp/smartexplorer-f-android_ui_fixtures-scope.json`, einschließlich der freigegebenen Ergänzung
- `/tmp/smartexplorer-final-review-user-contract.txt`
- `AGENTS.md`
- `docs/ARCHITEKTUR.md`, Android-Einstieg, Medienrotation, Datenweg und gemeinsame Suite
- `docs/plaene/2026-10-24-batches/vertraege.md`, auftragsbezogene Recherche
- `docs/plaene/2026-10-24-batches/rv1-funktionsplan.md`, Android-Erstbefund und M14
- `docs/plaene/2026-10-24-batches/rv1-funktionsbefunde.json`, gezielte Kennungssuche; kein zusätzlicher A01-Laufzeitbeleg daraus abgeleitet
- `docs/plaene/2026-10-24-batches/fix-device-navigation.md`, vorhandene Anschlussreferenzen
- `docs/plaene/2026-10-24-batches/fix-f-android-ui-fixtures.md`
- `.github/workflows/review-task.yml`
- `native/review-task-manifest.json`
- `android/test-android-task.sh`
- `android/app/build.gradle.kts`
- `android/gradle/libs.versions.toml`
- `/tmp/smartexplorer-rv1-40406246032/android-failures.json`
- `/tmp/smartexplorer-rv1-40406246032/compose-bom.pom`
- `/tmp/smartexplorer-rv1-40406246032/primary-android/Config.java`
- `/tmp/smartexplorer-rv1-40406246032/primary-android/SemanticsNodeInteraction.kt`
- `/tmp/smartexplorer-rv1-40406246032/primary-android/Offset.kt`
- `/tmp/smartexplorer-rv1-40406246032/primary-android/InlineClassHelper.kt`
- Versionsgebundene externe JUnitquelle: `https://github.com/junit-team/junit4/blob/r4.13.2/src/main/java/org/junit/Assert.java`, direkt über `raw.githubusercontent.com` gelesen
- Arbeitsweise-Anweisungen einschließlich Plan-, Code-/Bestandsrecherche-, Primärquellen-, Autorprüfungs- und Abschlussregeln über das lokale MCP geladen

Android-Einstieg, Navigation und unveränderte Verbraucher:

- `android/app/src/main/AndroidManifest.xml`
- `android/app/src/main/java/app/smartexplorer/android/MainActivity.kt`
- `android/app/src/main/java/app/smartexplorer/android/SmartExplorerApp.kt`
- `android/app/src/main/java/app/smartexplorer/android/core/Core.kt`
- `android/app/src/main/java/app/smartexplorer/android/service/BackgroundController.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/AppNav.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/AppRoot.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/settings/SettingsNavigation.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/settings/SettingsScreen.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/share/ShareScreen.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/share/ShareDialogHost.kt`

Login- und Medienbesitz:

- `android/app/src/main/java/app/smartexplorer/android/api/LoginApi.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/login/LoginBitmapOwner.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/login/LoginBitmapRenderer.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/login/LoginBitmapSurface.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/login/LoginInput.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/login/LoginSession.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/login/LoginTargetDialog.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/login/LoginViewScreen.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/viewer/ImagePage.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/viewer/MediaGeometry.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/viewer/MediaLoader.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/viewer/MediaPlaybackState.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/viewer/MediaSet.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/viewer/MediaViewer.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/viewer/PlayerPage.kt`

Fixtures und vorhandene Isolationsbeispiele:

- `android/app/src/androidTest/java/app/smartexplorer/android/task/BatchesDeviceNavigationTaskTest.kt`
- `android/app/src/androidTest/java/app/smartexplorer/android/task/BatchesMediaWakeTaskTest.kt`
- `android/app/src/androidTest/java/app/smartexplorer/android/task/LoginImageTaskTest.kt`
- `android/app/src/test/java/app/smartexplorer/android/ui/login/LoginBitmapOwnerTest.kt`
- `android/app/src/test/java/app/smartexplorer/android/ui/viewer/BatchesMediaStateTest.kt`
- `android/app/src/test/java/app/smartexplorer/android/ui/screen/BatchesScreenStateTest.kt`
- `android/app/src/test/java/app/smartexplorer/android/system/documents/DriveTaskProviderTest.kt`

Geändert wurden ausschließlich diese drei bestehenden Dateien:

- `android/app/src/androidTest/java/app/smartexplorer/android/task/BatchesDeviceNavigationTaskTest.kt`
- `android/app/src/test/java/app/smartexplorer/android/ui/login/LoginBitmapOwnerTest.kt`
- `android/app/src/test/java/app/smartexplorer/android/ui/viewer/BatchesMediaStateTest.kt`

Neu erstellt wurde ausschließlich dieser Bericht:

- `docs/plaene/2026-10-24-batches/fix-f-android-ui-fixtures.md`

## Entscheidungen und verbleibende Grenzen

Die eigene vollständige Fehlerauflistung, die Karte, der vor der Änderung
geschriebene Erhaltungsplan, die Umsetzung und die nachfolgende eigene
statische Anti-Regressionsreview sind abgeschlossen. Es bleibt kein
Scopebedarf und keine unbeantwortete APIfrage für M14 offen. Es gibt
keinen belegten zusätzlichen Produktdefekt aus dieser Untersuchung.

Die Kompilierung der Instrumentation, der tatsächliche Eintritt in alle
Loginbitmap-Prüfkörper, deren reale Canvas-/Ressourcenresultate sowie
die Medien- und Navigations-Laufzeitergebnisse des korrigierten Kandidaten
sind noch nicht bestätigt. Physische Sensorrotation ist ebenfalls kein
Ergebnis dieses statischen Fixzyklus. Main besitzt die Bestätigung im
bereits vorhandenen gemeinsamen Remote-Einstieg und anschließend die
Integration und den terminalen Release des Gesamtbatches. Dieser Bericht
behauptet weder einen Remote-Pass noch eine erfolgte Auslieferung.
