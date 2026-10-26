# RA-01: Bildschirmwachhaltung der Android-Medienvorschau

Stand: 2026-10-26. Eigener Fixzyklus des beauftragten Android-Fixautors.
Quellbasis: Reviewkandidat `ee75e91bad6b28ef9206a5b2b2857cbea75c4e5b`;
vorgegebene Releasevergleichsbasis `a814f263096fe69d3746e840b5f3fad3fd7e88a0`.
Dieser Bericht behauptet keinen neuen Release und keinen ausgeführten Laufzeitnachweis.

## Auftrag und Erstplan aus dem Prompt

Quelle: vollständiger Nutzervertrag in
`/tmp/smartexplorer-final-review-user-contract.txt`, delegierter RA-01-Auftrag und
`/tmp/smartexplorer-fix-android-media-scope.json`.

Der Nutzer verlangt für jeden Regressionsfix dieselbe Autorschaft für
Funktionsauflistung, vollständige direkte und indirekte Funktionskarte, konkreten
Fixplan, Umsetzung und anschließende Anti-Regressionsreview. Für P32 gilt
wörtlich „nach einstellungen des nutzers im system“ und „also nicht den
einstellungen in der app, in androud“.

Beauftragtes Ergebnis: Sichtbar laufende Videos der eingebauten Android-Vorschau
verhindern den Bildschirmtimeout. Pause, Hintergrund, Audio, Bilder,
Surfacewechsel, Medienwechsel, Wiedergabeposition und Android-Systemrotation
behalten ihre vereinbarten Zustände. Eigene Folgerung: Die Wachhaltung muss an
laufende Videowiedergabe und deren tatsächlichen Surfacebesitz gebunden sein;
ein zusätzlicher App-Rotationsschalter folgt daraus nicht.

Der Erstplan umfasst die Bestandsaufnahme dieser Zustandskette, den Abgleich mit
aktuellen Android-Primärverträgen, den begründeten Fixplan, den begrenzten
Quellfix, vorbereitete Fälle für Mains gemeinsame Remote-Suite und eine eigene
anschließende statische Prüfung derselben Kette. Vor der Bestandslektüre waren
die genaue Wake-Ursache und die vorhandenen Testanschlüsse unbekannt. Eine nur
vom Nutzer zu liefernde Angabe war nicht erforderlich.

Grenzen: Produktänderungen ausschließlich in der freigegebenen isolierten
Arbeitskopie und der exklusiven Schreibfläche. Main hat die Medienoberflächendoku
zusätzlich für den passenden Wake-Vertrag freigegeben. Keine Unteragenten, Builds,
Compiler, Formatter, Tests, Server, Installationen, Commits, Pushes,
Graphupdates, Verpackung oder Releases. Main besitzt gemeinsame
Suiteregistrierung, Remoteausführung und Abschlussrelease.

## Erste Bestandsbefunde am Kandidaten vor dem Fix und Planstufe 1

Der Dokumentationskontext wurde vor dieser Datei geprüft: `git status --short`
zeigte laufende Änderungen der gleichzeitig beauftragten Autoren außerhalb der
Android-Medienschreibfläche; `git log -1 --oneline` zeigte `ee75e91b`. Gezielte
Suche nach RA-01, P32, MediaPlaybackState, PlayerPage, Rotation und Medien sowie
die nachfolgende konkrete Quelllektüre bestimmen die folgenden Befunde.

- `MediaPlaybackState.kt` erzeugt einen `MediaPlayer`, steuert Audiofokus,
  Start/Pause und Freigabe und bindet den `SurfaceHolder` mit `setDisplay`.
  `setScreenOnWhilePlaying` fehlt.
- `PlayerPage.kt` hält nur für die aktive Seite einen Player. Prepared,
  Audiofokus, ON_START, ON_STOP, Wiedergabeknopf, Completion, Fehler und Dispose
  steuern dessen Zustand. Die Videofläche bindet beziehungsweise löst den
  Displayholder; Audio erzeugt keine Surface.
- `MediaViewer.kt` bestimmt die aktive Playerseite aus `pager.settledPage` und
  behält Medienreihenfolge, lokale/Remote-Ladung und externe Appübergabe.
- P32 in `batches.md` verlangt unveränderte Android-Systemrotation,
  pfadgebundene Position/Pausenabsicht und keine Wiederaufnahme nach ON_STOP
  ohne Play. Der Manifestvertrag erhält MainActivity bei den dort benannten
  Größen-/Orientierungsänderungen.
- `BatchesPresentationTaskTest.kt` besitzt bereits Bild-, Audio- und
  Video-Systemlayoutfälle mit einem kandidatengebunden bereitgestellten MP4.
  `BatchesMediaStateTest.kt` und `MediaSetTest.kt` sichern Geometrie und
  Mediengruppierung. Diese Fälle ersetzen keinen Bildschirm-Wake-Nachweis.

Planstufe 1: Den gehaltenen MediaPlayer und seine austauschbare Surface erhalten.
Den fehlenden Plattformvertrag in der bestehenden Besitzgrenze ergänzen und
seine Start-, Pause-, Fehler-, Completion-, Detach- und Releaseübergänge prüfen.
Keine neue Playerbibliothek, Fenster-Wakelock oder Medienladeänderung ist aus
dem belegten Fehler erforderlich. Vor der Umsetzung bleiben die genaue
Android-Wake-Semantik und ein belastbarer Remote-Gerätefall zu recherchieren.

Der Rootgraph bildet laut `docs/ARCHITEKTUR.md` ausschließlich `native/src` ab.
Main bestätigte am 2026-10-26, dass keine Android-Graphreferenz für diesen Auftrag
existiert. Die freigegebene Kotlin-Quellfläche ist deshalb die verfügbare
Kontrollflussgrundlage; es erfolgt keine Graphneuerstellung.

## Eigene Funktionsauflistung und vollständige Verknüpfungskarte

Die zugewiesene fehlerhafte Endnutzerfunktion ist **RA-01**: Ein lokal oder als
registrierte Remotekopie geöffnetes Video der eingebauten Android-Vorschau darf
während sichtbarer laufender Wiedergabe nicht durch den Bildschirmtimeout
unterbrochen werden. Die Freigabe dieses Fixes verlangt zugleich den Erhalt von
Pause/Play, Seek, Completion, Hintergrundpause, Fokusverlust/-ducking,
Seiten-/Medienwechsel, Surfaceersatz, Fehleranzeige, Schließen, Bildzoom,
Audiowiedergabe und der Android-Systemrotation.

Die folgende Karte reicht vom vorhandenen Dateibedienweg über alle den Wake-Fix
bestimmenden Zustandszweige bis zur Android-Frameworkgrenze. Die Core- und
Extern-Öffnen-Aufrufe bleiben benannte, unveränderte Grenzen; sie besitzen keinen
Wakezustand dieser Vorschau.

| Station und Funktionen | Direkte Verbindung | Indirekter Vertrag und Erhalt |
|---|---|---|
| `AndroidManifest.xml` MainActivity, `MainActivity.onCreate/onStart/onStop`, `AppRoot.MainShell` | MainActivity → AppRoot → FilesScreen; Manifest behandelt orientation, screenSize, smallestScreenSize, screenLayout. | Systemrotation bleibt maßgeblich; keine requestedOrientation-Zuweisung; das Vorschau-Overlay bleibt bei Größenwechsel im selben Besitzer. |
| `BrowserTab.shownEntries`, `FileActions.openEntry`, `mediaSetFor/mediaGroupOf` | Sichtbare Listenreihenfolge und aktivierte eigene Medienvorschau → `FilesViewModel.viewer`. | Bilder/Video bleiben eine Gruppe; Audio bleibt separat; Ordner/ZIP-/Nichtmedienwege und externe Standardöffnung bleiben unverändert. Die originale Entry.location wird nicht verändert. |
| `FilesViewModel.viewer`, `FilesScreen` | Besitzer hält MediaSet; MediaViewer wird angezeigt; Schließen setzt viewer=null. | Entfernen der Komposition gibt den aktiven Player frei. Eine fertig geladene lokale Kopie geht über FilesEffect.Open direkt an die externe App; sonst bleibt FileActions.openFile zuständig. |
| `MediaViewer` Dialog, HorizontalPager, `step`, Ladeeffekt | Entry.location ist Pagerkey; pager.settledPage bestimmt aktiven Player; Bildzoom sperrt Paging; Topbar steuert Schließen und externes Öffnen. | Nur die gesetzte aktive Videoseite besitzt einen Player. Benachbarte Videos bleiben Titelansichten; Seitenwechsel entfernt ihre Effekte. Insets, Reihenfolge, Pfeiltasten und Menü bleiben erhalten. |
| `loadMedia`, `PageLoad.isUsable`, `FilesApi.open/fetch/awaitTask/resultOf/cancelTask/clearFinishedTasks` | Lokaler pfadberechtigter Open oder registrierter Fetch → LocalMedia.path → PlayerPage beziehungsweise ImagePage. | Unveränderte Locator-/Berechtigungsgrenze; ZIP-Fallback nur bei unsupported; Ladefehler/-retry, Taskabbruch und Cleanup bleiben. Wake wird erst beim Playerstart verlangt, nicht während Download oder Ladefehler. |
| `PlayerPage` rememberSaveable, aktive Seitengrenze, DisposableEffect | Medienpfad bindet Position und Wiedergabewunsch; aktiver Effekt erzeugt MediaPlaybackState. | Kein Bundle speichert Player/Bitmap. Deaktivieren, Pfadwechsel und Schließen sichern vorhandene Position und geben Player/Audiofokus frei. |
| Prepared, ON_START, Playknopf | Duration/Videoabmessungen und Seek auf gespeicherte Position; `MediaPlaybackState.start(visible)` verlangt STARTED und erteilten Audiofokus. | Kein Start im unsichtbaren Zustand; verweigerter Fokus bleibt verweigert. Replay nach Ende startet weiter bei null, Größenwechsel setzt nicht zurück. |
| Pauseknopf, ON_STOP, Audiofokus-Listener | `pause` beendet den erfolgreichen Start und gibt Fokus frei; ON_STOP und voller/transienter Fokusverlust löschen wantsPlayback. | Rückkehr verlangt ausdrücklich Play. Ducking und Fokusgewinn verändern nur Lautstärke. Die neue Wachhaltung folgt dem echten Plattform-Start/Pause-Zustand. |
| Completion, Fehlerlistener, Catch, Fehlerkomposition | Completion setzt Ende/Pausenabsicht; Fehler bleibt sichtbar und führt zur idempotenten Playerfreigabe. | Kein dauerhaftes Wake nach Ende, Fehler oder Freigabe. Externe Appübergabe bleibt als Fehlerausweg erreichbar. |
| `SurfaceHolder.Callback.surfaceCreated/surfaceDestroyed`, AndroidView.onRelease | Konkrete Surface bindet den Displayholder; Zerstören/Release löst ausschließlich denselben Holder. | Rotation/Fensterwechsel kann die Surface ersetzen, ohne Player, Position oder Pausenabsicht zu ersetzen. Verspätete alte Rückrufe dürfen keinen neueren Holder lösen. |
| `MediaPlaybackState.display/clearDisplay/release` → Android MediaPlayer/SurfaceHolder | Wachhaltung vor Holderersatz löschen, Display binden, Wachhaltung am gebundenen Holder aktivieren; Release löst vor Playerfreigabe. | Nur der tatsächlich gebundene Videoholder besitzt Wake. Audio besitzt keinen Holder. Pause, Completion und Fehler schalten die Frameworkanforderung ab. |
| `ImagePage`, `MediaGeometry`, `clock`, PlayerPage.Controls/MediaTitle | Bilddekodierung, Zoom/Fokus, Video-Seitenverhältnis, Audioanordnung und schmaler Slider bleiben separat. | Bilder erhalten keine Wachanforderung; der Fix schreibt keine Mediendatei/EXIF und ändert weder Zeitformat noch Eingabegesten. |
| Vorhandene und neue Androidfälle → `native/review-task-manifest.json` | P32 verweist auf MediaState/MediaSet-Units und reale Presentationfälle; Main registriert die ergänzten Wakefälle. | Nur Mains eine gemeinsame Remote-Suite darf diese Fälle ausführen. Manifest/Orchestrierung werden von diesem Autor nicht geändert. |

## Primärrecherche und Entscheidungsplan C

Geprüft am 2026-10-26; Androidprojekt: minSdk 30, compileSdk/targetSdk 36.

1. [MediaPlayer-API](https://developer.android.com/reference/android/media/MediaPlayer#setScreenOnWhilePlaying(boolean)):
   `void setScreenOnWhilePlaying(boolean)` steuert den gebundenen SurfaceHolder;
   `setDisplay(SurfaceHolder?)` erhält den existierenden Videoweg. Kein
   zusätzlicher Low-Level-Wakelock oder neuer Berechtigungsvertrag ist nötig.
2. [AOSP Android 15 MediaPlayer](https://raw.githubusercontent.com/aosp-mirror/platform_frameworks_base/android-15.0.0_r1/media/java/android/media/MediaPlayer.java),
   setDisplay, setScreenOnWhilePlaying, updateSurfaceScreenOn, startImpl, pause,
   release und EventHandler: Displayersatz überschreibt den alten Holder vor
   dem Update; deshalb muss seine Wachanforderung vorher abgeschaltet werden.
   Start/Pause, Completion und Fehler liefern den tatsächlichen Wakezustand.
   Sämtliche Produktaufrufe bleiben wie bisher auf dem UI-/Callbackthread.
3. [AOSP Android 15 VideoView](https://raw.githubusercontent.com/aosp-mirror/platform_frameworks_base/android-15.0.0_r1/core/java/android/widget/VideoView.java),
   openVideo: Die Releasebasis verwendete dessen Wakeaktivierung. Der eigene
   Vergleich von `a814f263:PlayerPage.kt` bestätigt den damaligen VideoView-Weg.
4. [SurfaceHolder](https://developer.android.com/reference/android/view/SurfaceHolder#setKeepScreenOn(boolean))
   und [Bildschirmwachhaltung](https://developer.android.com/develop/background-work/background-tasks/awake/screen-on):
   Eine angezeigte Surface kann die Bildschirmabschaltung verhindern. Eine
   allgemeine Fensteranforderung wäre möglich, besäße hier aber eine größere
   Lebensdauer als der tatsächlich spielende Videoholder.

Gewählte Lösung: Den vorhandenen MediaPlayer, seine Audiofokus-/Sichtbarkeitsprüfung
und die austauschbare Surface erhalten. `display` schaltet die bisherige
Holder-Wachhaltung vor dem Binden ab und aktiviert sie nur bei vorhandenem
Holder. `clearDisplay(holder)` prüft die tatsächliche Besitzeridentität.
`release` verwendet denselben sauberen Detachweg vor dem Endzustand.

Verglichene Wege: Ein pauschales Window-KEEP_SCREEN_ON oder SurfaceView.keepScreenOn
während bloßer aktiver Seitensichtbarkeit würde Pause und Audio eigens dupliziert
modellieren. `setWakeMode` wäre für Bildschirmwachhaltung unnötig und erweitert
die Berechtigungs-/Ressourcengrenze. Eine Rückkehr zu VideoView würde die bereits
belegte P32-Surface-/Positionslösung aufgeben. Eine zusätzliche Playerbibliothek
löst den eng belegten fehlenden Aufruf nicht mit geringerem Zustandsumfang.

## Detaillierter Arbeitsplan D und zweite Recherche

| Meilenstein | Dateien und Abhängigkeit | Konkretes erwartetes Ergebnis | Fortschritt |
|---|---|---|---|
| M1: Plattformwachhaltung erhalten | MediaPlaybackState.kt; vorhandene start/pause/focus-Grenze | Nur laufendes Video mit gebundener Surface fordert Wake; Prepared, Pause, nicht sichtbarer Start, Completion und Release fordern kein Wake. | Im Quellstand umgesetzt; statisch geprüft |
| M2: Surfacebesitz bei Rotation/Abschluss | MediaPlaybackState.kt, PlayerPage.kt; M1 | Holderersatz löscht alten Wake; alte Destroy/Release-Rückrufe lassen den neuen Holder bestehen; Position und Wiedergabeabsicht bleiben. | Im Quellstand umgesetzt; statisch geprüft |
| M3: Medienvertrag dokumentieren | docs/oberflaechen/medien.md; M1/M2 | Video-Wake und seine vollständigen Freigabegrenzen sind ohne neue Ausrichtungswahl beschrieben. | Aktualisiert und gegen Quelle geprüft |
| Verifikation nach vollständiger Umsetzung | BatchesMediaWakeTaskTest.kt, dieser Bericht; M1–M3 | Reale Frameworkholder, Pause/Ende, Dispose, Hintergrund, Audio/Bild, Surfaceersatz und zweimaliger Systemlayoutwechsel erhalten überprüfbare Orakel; Main registriert sie in der einen Suite. | Fälle vorbereitet; durch Main registriert; nicht ausgeführt |
| Eigene anschließende Anti-Regressionsreview | gesamte oben ermittelte Kette; fertiger Fix und Fälle | Jeder Verknüpfungszweig wird gegen Quelle und Diff abgeglichen; Befunde und fehlender Laufzeitnachweis stehen ausdrücklich im Bericht. | Durch denselben Fixautor abgeschlossen; Ergebnisse unten |

Zweite Recherche vor dem Produktedit: Der konkrete Holderwechsel und der
Fehler-/Completionpfad wurden erneut gegen AOSP gelesen; `setDisplay(null)` allein
löscht den alten Wakebesitz nicht. Spätes Detach benötigt deshalb Holderidentität.
Für die neuen Remote-Gerätefälle liefert
[WindowInspector.getGlobalWindowViews](https://developer.android.com/reference/android/view/inspector/WindowInspector#getGlobalWindowViews())
ab API29 die tatsächlich angehängten Fenster des Prozesses, einschließlich des
Vorschaudialogs. Damit kann die reale Surface-Wachanforderung geprüft werden.
[ActivityScenario.moveToState](https://developer.android.com/reference/androidx/test/core/app/ActivityScenario#moveToState(androidx.lifecycle.Lifecycle.State))
erreicht CREATED über ON_STOP; derselbe vorhandene MainActivity-Besitzer kann
anschließend zurückkehren. Die Fälle übernehmen das bestehende MP4-Argument
`batchesVideoPath` mit mindestens acht Sekunden und stellen veränderte
Systemrotationswerte im finally wieder her. Kein zusätzlicher Download, kein
physischer Sensornachweis und keine lokale Testausführung folgen daraus.

Die eigene anschließende Review prüft getrennt: Wakebesitz, Start-/Pausenzustand,
Lebenszyklus/Audiofokus, Surface-/Playerfreigabe, Medien-/Ladeidentität,
Bild-/Audiopfad, Rotation/Position und die Aussagekraft der vorbereiteten Fälle.
Befunde werden als belegter Fehler, offene Grenze oder nicht erforderliche
Verbesserung gekennzeichnet; ein Laufzeitpass wird daraus nicht abgeleitet.

## Umsetzung und eigene anschließende Anti-Regressionsreview

Die Produktänderung beschränkt sich auf `MediaPlaybackState.kt:39` bis zum
Releasepfad und die beiden Detachaufrufe in `PlayerPage.kt:180`/`:185`.
Die neue private Holderreferenz wird beim Binden gesetzt und beim sauberen
Detach gelöscht; sie wird nicht gespeichert oder an ein anderes Medium
weitergereicht. Der Plattformplayer bleibt derselbe. Die APIform bleibt
`setDisplay(SurfaceHolder?)`; ein holderloser Surfacewechsel über `setSurface`
wurde nicht eingeführt.

| Geprüfter Zweig der eigenen Karte | Ergebnis der anschließenden statischen Prüfung |
|---|---|
| Video vorbereitet, Start nicht sichtbar oder Audiofokus verweigert | `start` wurde nicht geändert. Ohne erfolgreichen Plattformstart bleibt dessen Wachzustand aus; ein bloß gebundener Holder erzeugt daher keine laufende Wachanforderung. |
| Sichtbares Video, Play und Replay | `display` aktiviert die vorhandene Plattform-Wakeoption erst am konkreten Holder. Der bestehende erfolgreiche `player.start` aktiviert sie während der Wiedergabe; Replay-/Seeklogik in Controls bleibt unverändert. |
| Pause, Completion, Fehler, ON_STOP | Die vorhandenen Pause-/Completion-/Fehler-/Lifecyclepfade bleiben unverändert und beenden den Start beziehungsweise geben frei. Der neue Releaseweg löscht Wake vor dem Holderverlust. Ein Fehler löst keine neue Start-/Retryabsicht aus. |
| Audiofokusverlust, Ducking, Fokusgewinn | Listener und Lautstärkeregeln sind unverändert: voller/transienter Verlust pausiert und verlangt Play; Ducking/Gewinn ändern ausschließlich Lautstärke. Keine zusätzliche Fokusanforderung wurde eingeführt. |
| Hintergrund und Rückkehr | ON_STOP speichert Position, pausiert und setzt wantsPlayback=false. Neuer Holder beim Rückweg bleibt deshalb ohne laufende Wachanforderung; ON_START überschreibt die Pause nicht. Prepared vor ON_START beziehungsweise nach ON_STOP behält seine vorhandene Sichtbarkeitsprüfung. |
| Surfaceersatz, Größenwechsel, alter Destroy/Release | Wake wird vor dem Austausch vom alten Holder entfernt. `clearDisplay` löst nur bei Referenzgleichheit. Ein Rückruf einer bereits ersetzten Surface kann den neu gebundenen Holder nicht lösen. Keine Position, Dauer, Videoabmessung oder Wiedergabeabsicht wird im Fix verändert. |
| Playerrelease vor oder nach Surfacezerstörung | Release löst zuerst sauber, setzt danach released und entfernt Listener/Audiofokus wie bisher. Wiederholter Release und spätere Displayaufrufe sind wirkungslos. Der Holder wird nicht über die Playerlebensdauer gehalten. |
| Medienseite inaktiv, Bild, Audio, Download und Ladefehler | Die aktive Seitengrenze bleibt unverändert. Audio erzeugt keine Surface; Bilder, Nachbarseiten und Laden erzeugen keinen Playerholder. Entfernte aktive Videoseiten durchlaufen den sauberen Release. Kein Backendschema, kein Locator, keine Berechtigung, kein Download-/Taskabbruch und kein Fehlerretry wurde verändert. |
| Schließen und externe Appübergabe | viewer=null entfernt die Komposition und gibt frei. Der Extern-Öffnen-Vertrag über geladene Kopie beziehungsweise openFile bleibt unverändert; ein daraus folgendes ON_STOP behält seine Pause. Keine Datei, EXIF, Kopie oder Uploadregistrierung wird vom Wakefix verändert. |
| Systemrotation, Bildzoom, Audiolayout, Bedienelemente | Manifest, MainActivity, MediaViewer, ImagePage, MediaGeometry und Controls enthalten keinen Produktdiff. Rotation/Rotationssperre bleiben Android zugeordnet; der Fix führt keine Appauswahl, requestedOrientation-Zuweisung, neue Geste oder neue Einstellung ein. |
| Framework-/Ressourcengrenze | Kein Low-Level-Wakelock, keine neue Berechtigung, keine neue Bibliothek, kein Hintergrunddienst und kein zusätzlicher Thread. Die bestehenden UI-/Callbackthreadzugriffe bleiben. |
| Vorbereitete Gerätefälle und Dokumentation | Die Fälle verwenden echte Player/Views, prüfen den Dialog statt nur des Activityfensters, sichern temporäre Dateien und stellen veränderte Systemrotationswerte wieder her. Eigene kleine Fixture-/Dokumentfehler aus der Autorlektüre wurden vor Übergabe korrigiert. Quellen- und Verhaltensbehauptungen sind gegen den finalen Diff abgeglichen. |

Die anschließende Prüfung fand im endgültigen Produktfix keinen weiteren
belegten funktionellen Defekt. Das ist eine begrenzte statische Autorprüfung
der oben vollständig ermittelten Wake-Funktionskette, keine Behauptung über
ausgeführte Codec-, OEM-, Sensor- oder Timeoutläufe. `git diff --check` für die
getrackten eigenen Änderungen war ohne Ausgabe erfolgreich; die gezielte
Quellsuche bestätigt die beiden neuen Wakeaufrufe, die beiden holdergebundenen
Detachaufrufe und keine neue Orientierungs-/Wakelockstelle in der freigegebenen
Produktfläche. Es wurde kein Compiler, Formatter, Build oder Test gestartet.

## Vorbereitete Fälle und Registrierung durch Main

Klasse: `app.smartexplorer.android.task.BatchesMediaWakeTaskTest`.
Main hat die Klasse im gemeinsamen Instrumentation-Aufruf und sämtliche
folgenden Methoden unter P32 sowie `R10-RA01-Android-Medien` des bestehenden
`native/review-task-manifest.json` registriert. Die gezielte Manifestlektüre
bestätigt diesen Anschluss; dieser Autor hat das Manifest nicht bearbeitet.

| Genaue Methode | Konkretes Orakel |
|---|---|
| `batches_android_video_wake_follows_play_pause_completion_and_close` | Reale Surface im Vorschaudialog: Wake bei Play/Replay; kein Wake bei Pause, Ende und Schließen. Seek und bestehende Steuerung bleiben nutzbar. |
| `batches_android_video_wake_preserves_system_rotation_and_explicit_background_resume` | Zweimaliger Systemlayoutwechsel erhält Activitybesitz, Pause/Seekposition und Videowake; echter ON_STOP gibt Wake frei, Rückkehr bleibt pausiert, ausdrückliches Play aktiviert Wake. Rotationswerte werden wiederhergestellt. |
| `batches_android_media_switch_to_image_audio_or_error_releases_video_wake` | Entferntes Video hinterlässt keinen Wake; geladenes PNG, laufendes WAV und die sichtbare Fehleransicht verlangen keinen Wake. Audio erzeugt keine Videosurface. |
| `batches_android_surface_wake_moves_without_reset_and_ignores_stale_detach` | Ein echter MediaPlayer mit zwei realen SurfaceViews: Prepared und nicht sichtbarer Start bleiben ohne Wake; Start, Holderersatz, verspätetes altes Detach, Pause, Rebind und wiederholte Freigabe erhalten richtigen Wakebesitzer und Playerposition. |

Die neuen Fälle verwenden das bestehende `batchesVideoPath`; die gemeinsame
Konfiguration nennt zwölf Sekunden MP4. WAV und PNG entstehen im isolierten
Testcache und werden entfernt. Vorhandene P32-MediaState-/MediaSet-Units und die
Bildzoom-/Audio-/Video-Presentationfälle bleiben registriert und unverändert.
Es entsteht keine zusätzliche Suite und kein neuer Unitselektor.

**Noch nicht ausgeführt:** Die Gerätemethoden wurden weder gebaut noch gestartet.
Auch nach einem späteren erfolgreichen Remote-Lauf belegen diese Methoden die
realen Framework-`keepScreenOn`-Zustände und ihre Besitzerwechsel. Ein tatsächlich
abgewarteter OEM-Bildschirmtimeout, physische Rotationssensoren,
herstellerspezifische Codecs und ein echtes Remoteproviderkonto sind damit nicht
nachgewiesen. Die unveränderten Loader-/Berechtigungsgrenzen sind statisch
verfolgt; die Wakefälle beginnen bei der vorhandenen lokalen Mediendatei.

## Gelesene und geänderte Dateien, Entscheidungen und Restabhängigkeiten

Gelesen wurden die folgenden Dateien; bei den ausdrücklich benannten großen
Einstiegsdateien waren die medienspezifischen Abschnitte und Suchtreffer die
abgegrenzte Lesefläche:

- Regeln/Vertrag: `AGENTS.md`, `/root/.codex/AGENTS.md`, die beiden geladenen
  Arbeitsweise-SKILL.md, `/tmp/smartexplorer-fix-android-media-scope.json` und
  `/tmp/smartexplorer-final-review-user-contract.txt`.
- Architektur/Plan: `docs/ARCHITEKTUR.md`, P32 aus
  `docs/plaene/2026-10-24-batches/batches.md`, thematische Suche in
  `vertraege.md`, `regressionsreview.md`, `docs/oberflaechen/INDEX.md` und
  `docs/oberflaechen/medien.md`; RA-01 und dessen Quellen-/Grenzkontext aus
  Mains `review-regression-astra.md`.
- Android-Verträge: `android/app/build.gradle.kts` (SDK-/Testkonfiguration),
  `android/app/src/main/AndroidManifest.xml` (MainActivity-/Orientierungsvertrag),
  `MainActivity.kt`, `ui/AppRoot.kt`, `ui/files/FilesScreen.kt` (Viewer und
  Extern-Öffnen), `ui/files/FileActions.kt` (openEntry/openFile),
  `ui/files/FilesViewModel.kt` (viewer/effects/Lebensdauer),
  `ui/files/BrowserTab.kt` (shownEntries) und `api/FilesApi.kt`
  (open/fetch/Taskvertrag). Alle Kotlinpfade stehen unter
  `android/app/src/main/java/app/smartexplorer/android/`.
- Viewer: `MediaPlaybackState.kt`, `PlayerPage.kt`, `MediaViewer.kt`,
  `MediaLoader.kt`, `MediaSet.kt`, `ImagePage.kt` und `MediaGeometry.kt`
  unter demselben Kotlinstamm `ui/viewer/`; zusätzlich PlayerPage der
  vorgegebenen Releasevergleichsbasis per `git show`.
- Fälle/Anschluss: `BatchesMediaStateTest.kt`, `MediaSetTest.kt`,
  `BatchesPresentationTaskTest.kt`, eigener `BatchesMediaWakeTaskTest.kt`
  und `native/review-task-manifest.json` (Androidauswahl, P32 und RA-01-Punkt).

Geändert: `MediaPlaybackState.kt`, `PlayerPage.kt` und
`docs/oberflaechen/medien.md`. Erstellt:
`android/app/src/androidTest/java/app/smartexplorer/android/task/BatchesMediaWakeTaskTest.kt`
und dieser Bericht. `BatchesMediaStateTest.kt` blieb unverändert, weil ein
zusätzlicher reiner Unitfall keinen tatsächlichen Android-Wakebesitz belegen
würde. Keine native Rustdatei wurde bearbeitet; kein Graphupdate ist erforderlich.

Entscheidungen: vorhandene Plattformtechnik erhalten; Wake mit dem konkreten
Surfacebesitz koppeln; verspätete Rückrufe anhand Holderidentität begrenzen;
keine zusätzliche System-/Approtationseinstellung und keine neue Abhängigkeit.
Die nötigen reinen Lesefreigaben für Medienvertrag und Dateieinstiege sowie die
exklusive Dokumentationsschreibfreigabe wurden von Main konkret ergänzt.

Es besteht keine ungelöste Produktabhängigkeit außerhalb der freigegebenen
Fläche. Gemeinsamer Kandidatencommit/Push, Auswertung der einen Remote-Suite
und der eine terminale Release sind ausdrücklich Mains anschließende Arbeit.
Dieser Autor hat nichts davon ausgelöst und stoppt nach der abgeschlossenen
eigenen statischen Lieferung.
