# Medienvorschau

Stand 2026-10-25; P32 ist im Quellstand umgesetzt; Remote-Abnahme und Veröffentlichung sind noch nicht belegt.

| Fähigkeit | Vorhandener Einstieg | P32-Ergebnis |
|---|---|---|
| Android Bilder, Videos und Audio | `android/app/src/main/java/app/smartexplorer/android/ui/viewer/MediaViewer.kt` mit `MediaSet.kt`/`MediaLoader.kt` | Vorhandene Vollbildvorschau und Pager erhalten; Anordnung folgt der tatsächlichen Fensterfläche. |
| Bildzoom | `ImagePage.kt` | Zoom und normierter Bildmittelpunkt über Hoch-/Querformatwechsel erhalten; Pinch, Doppeltipp und Wischen bleiben. |
| Video-/Audiowiedergabe | `PlayerPage.kt` | Wiedergabeposition und Pause über Größen-/Surfacewechsel erhalten; genau eine aktive Playerinstanz. |
| Ausrichtung | Dialog der Medienvorschau | Android-Systemeinstellungen bestimmen Rotation und Rotationssperre. Keine eigene Appauswahl und keine requestedOrientation-Zuweisung. |
| Extern öffnen | Bestehendes Vorschau-Menü | „In App öffnen“ und „Öffnen mit…“ erhalten. |

Dieser Zusatz betrifft ausschließlich die eigene Android-Dateivorschau. Desktop öffnet Medien über seine vorhandenen externen Playerwege; CLI/MCP, Bridge und Netz transportieren Dateien, besitzen jedoch keinen Handy-Orientierungszustand. Es entsteht dafür kein neuer Remoteparameter. Bildschirmfreigaben haben einen separaten [Betrachtervertrag](bildschirm.md).

Bedienaufwand im Quellstand: Anpassung an die Android-Systemausrichtung ohne zusätzlichen Bedienvorgang in der App. Medienwechsel, Zoom und Play/Pause behalten ihre vorhandenen Positionen. Keine zusätzliche Einstellungsseite und kein wiederholtes externes Öffnen. Der tatsächliche Ablauf wird erst in der gemeinsamen Remote-Suite belegt.

`MediaGeometry.kt` begrenzt Bildtranslation an der tatsächlich eingepassten Bildfläche; Zoom und normierter Fokus liegen in `ImagePage.kt` unabhängig vom dekodierten Bitmap. `PlayerPage.kt` hält den Plattform-`MediaPlayer` über Fenster-/Surfacewechsel; `MediaPlaybackState.kt` bindet die austauschbare Videosurface und gibt Player sowie Audiofokus beim Seitenwechsel oder Schließen frei. Eine bewusste Pause wird beim Preparedcallback respektiert. Videos behalten ihr Seitenverhältnis, Audio nutzt im breiten Fenster eine kompakte Zeile; schmale Fenster erhalten die Zeitangabe oberhalb des Sliders. Die gesamte Vorschau berücksichtigt sichere Fensterinsets.

Die am 2026-10-25 präzisierte Nutzervorgabe bestimmt ausschließlich Android-Systemeinstellungen. Der zuvor angenommene Appumschalter und seine Activityanforderung sind entfernt. MainActivity lässt `screenOrientation` ungesetzt; die [Android-Systemrichtlinie](https://developer.android.com/guide/topics/manifest/activity-element#screen) bestimmt die Orientierung (Quelle geprüft am 2026-10-25). Die Anordnung folgt deshalb ausschließlich den echten Fensterconstraints. Mediendateien und EXIF werden nicht verändert; es wurde keine Playerbibliothek ergänzt. MainActivity verarbeitet im Manifest Größen-/Orientierungsänderungen, damit der übergeordnete Vorschauzustand beim Drehen erhalten bleibt.

Beim Activityereignis `ON_STOP` sichert die Vorschau die Position, pausiert und gibt den Audiofokus frei. Nach Rückkehr ist ausdrücklich Play nötig. Ein verspäteter Preparedcallback darf die Wiedergabe im Hintergrund nicht starten. Ein fehlender AudioManager verhindert den Start, ohne einen Nullzugriff auszulösen. Diese Grenzen gehören zur gemeinsamen Remote-Abnahme.

APIgrundlagen, geprüft am 2026-10-25: [MediaPlayer-Zustände und Surfacebindung](https://developer.android.com/reference/android/media/MediaPlayer), [Surfacelebenszyklus](https://developer.android.com/reference/android/view/SurfaceHolder.Callback), [kleine UIzustände speichern](https://developer.android.com/develop/ui/compose/state-saving).
