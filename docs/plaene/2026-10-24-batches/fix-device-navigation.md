# Geräte- und Appnavigation: G7, G8, G9

Stand: 2026-10-26. Zugewiesener Fixautor: `/root/fix_device_navigation`.
Die erste Abschlussfassung ist der Stand vor Remoteformatierung; ihre
damalige Größenbewertung wird im Fortsetzungsabschnitt anhand des realen
Remote-Befunds ausdrücklich korrigiert.
Arbeitskopie: `/tmp/smartexplorer-review-fixes-ee75e91b`, ursprünglicher
Reviewkandidat `ee75e91bad6b28ef9206a5b2b2857cbea75c4e5b`.
Die Releasebasis `a814f263096fe69d3746e840b5f3fad3fd7e88a0` / `v0.5.184`
stammt aus dem delegierten Auftrag; dieser Bericht ist keine Releaseprüfung.

## Erstplan aus dem Auftrag

Quelle sind der delegierte Fixauftrag, dessen vollständiger Nutzervertrag
`/tmp/smartexplorer-final-review-user-contract.txt` und dessen Dateizuweisung
`/tmp/smartexplorer-fix-device-navigation-scope.json`.
Beauftragt sind gemeinsam die Fehlerfunktionsliste, die vollständige direkte
und indirekte Verknüpfungskarte, der konkrete Plan mit Regressionserhalt,
die Umsetzung und die eigene anschließende Anti-Regressionsreview für G7–G9.

Das sichtbare Ergebnis ist: Geräteaufträge erreichen aus einer vorhandenen
Android-Einstellungsseite ihren Verbraucher. Beide Desktopflächen zeigen
den Programmpicker und können aus einem bestätigten Startbeleg über den
vorhandenen P4-Weg das bestätigte Fenster öffnen. Der Fix ergänzt die
bestehenden Bedienwege. Er erhält Tokenprüfung, Zielidentitäten, Rechte,
Request-/Node-/Generationsbindung und die einmalige Startausführung.

Vor Bestandslektüre bekannte Grenzen: ausschließlich zugewiesene Dateien
in der Fixkopie ändern; keine Unteragenten, Compiler, Formatter, Builds,
Tests, Server, Installationen, Commits, Pushes, Graphupdates oder Releases.
Main besitzt gemeinsame Remote-Suite, Integration und terminalen Release.
Die fehlenden Angaben betreffen ausschließlich den Bestand: konkrete
Anfrageverbraucher, beide UI-Zyklen, Registrierung und P4-Vertrag. Eine
nur vom Nutzer klärbare Angabe ist aus dem Auftrag nicht erkennbar.
Planungstiefe: gemeinsame Zustands- und Prozessketten beider Plattformen
ermitteln; daran kleine, zusammenhängende Reparaturen ausrichten.

## Erste gesicherte Befunde

Dokumentationskontext: `git status --short` und `git log -1 --oneline`
wurden vor Erstellung dieses Berichts gelesen. HEAD war `e5ade582`;
die Fixkopie enthält gleichzeitig bearbeitete, ausdrücklich getrennte
Schreibflächen. Der Gesamtreview wurde für G7–G9 gelesen.
`docs/ARCHITEKTUR.md` nennt `AppRoot`, `apps`, `remote_window` und die
Desktopflächen als einschlägige Grenzen. Die gezielte Graphabfrage
`graphify query 'desktop_surface_app share_apps_ui install_window_opener family_picker surface_windows layer_shell' --budget 2500`
bestätigt die Navigationsknoten. Graphify meldet Skill 0.9.11 gegenüber
Paket 0.9.17; die Abfrage liefert Orientierung, keinen Laufzeitnachweis.

G7: `AppRoot` wählt nur den Tab. `DeviceSettingsRoot` setzt seine Detailseite
bei Geräteaufträgen nicht zurück. `SettingsScreen` leitet Bildschirm,
Raster und Anfragen zu Geräte weiter, lässt `OpenApps` aber aus.
G8: Der Appfensteröffner ist prozesslokal. Nur `App::new` installiert ihn;
der getrennte Surfaceeinstieg konstruiert kein `App`.
G9: Der gemeinsame Menüeintrag speichert einen Picker im Context; nur
der eframe-Zyklus zeichnet ihn. Die Layerfläche benutzt `show_surface`
ohne den Picker.

## Zweitplan: Bestand und vollständige betroffene Verknüpfungskarte

Die eigene Lektüre bestätigt die drei Befunde. Planungstiefe 2: gekoppelte
bestehende Bedien- und Prozessverträge, keine neue Plattformfähigkeit.

| Fehlerhafte Funktion | Direkte und indirekte Kette; zu erhaltender Vertrag |
| --- | --- |
| G7: Android-Apppicker aus Darstellung; vier Geräteaufträge hinter Fehlerprotokoll/Über | `SeMenu.seMenuRows` → `SeActions.openApps/openScreen/screenGrid` → `SeController.open` → `AppNav.decode/intentFor` → `MainActivity.handleIntent` → Tokenprüfung `AppNav.fromIntent` → `AppNav.send` (`pending` plus Tabereignis) → `MainShell` (`tabFor`, Queueabonnement) → `DeviceSettingsRoot.detail` → `SettingsScreen.area` → `ShareScreen` → `ShareDialog.Apps` / `screenView` / `screenGrid` / Anfrage-Scroll. Nur `ShareScreen` verbraucht mit Compare-and-set; `SelectTab` bleibt ausschließlich Tabwahl. |
| G7: nachgelagerter App- und Bildschirmvertrag | `AppsDialog` löst den unveränderten Direct-/Room-Endpunkt über gespeicherte Geräte auf, hält Katalog/ID/Benutzer/Sitzung, speichert vor explizitem Launch und fragt bei unklarer Antwort ausschließlich denselben Beleg ab. Wiederzusammensetzung startet kein Programm. Die vorhandene `ShareScreen`-Weiterleitung zum P4-Fenster bleibt unverändert. Bildschirmziele werden weiter von `ScreenTargetJson.parse` geprüft; Raster und Anfrage-Scroll behalten ihren Verbraucher und Abbruchvertrag. |
| G8: bestätigtes Appfenster aus separatem Surfaceprozess | `desktop.os.shared.supervisor_surface` startet Prozess → `lib::run_gui` → `desktop::run_if_requested` → `desktop_surface_app::run` → Layerverzweigung oder `SurfaceApp`. Kein `App::new` läuft hier. Der Gerätekontextmenüeintrag speichert mit `share_apps_ui::open_surface` genau einen Picker pro unveränderter `PeerOpenTarget`-Identität. |
| G8: Startbeleg und Fensteröffnung | `share_apps_ui::panel` → `jobs::restore/catalog/launch/receipt` → `store::save` vor Launch → Daemon-Appkommando → authentifizierter `AppsClientHello` samt gebundenem Requesthash → `NodeApps` mit Katalog-, Grant-, Besitzer- und Laufzeitprüfung → dauerhafter Apphostbeleg → `node_apps_window::probe` → überprüfter P4-Pin. `settle` prüft Zielgerät und exakten Request; `confirmed` prüft Benutzer, Sitzung, App-ID und Generation. Der Öffner `family_picker::open_confirmed` fragt `screen_families_for_node` frisch ab und verlangt genau einen aktuellen Eintrag derselben Request-ID, Window-ID und Generation. `open_entry` validiert den Eintrag; `screen_launch::open_view` startet den regulären Viewer mit `expected_node`, Zielbeziehung und P4-Quelle. Das Öffnen ist kein zweiter Appstart. |
| G8: nachgelagerter P4-Besitz | `ScreenTarget` → `StartRequest::ScreenView` → `lib::run_gui` → `run_screen_view` → Familienviewer. Die bestehenden Sitzungs-/Rechte-/Familienverträge bleiben vollständig erhalten; die Reparatur ändert weder die Apphostzulassung noch den P4-Auflösungsalgorithmus oder Capture-/Eingaberechte. Ein Fehler des asynchronen Öffners liegt im ursprünglichen Context und muss dort über `family_picker::notifications` sichtbar sein. |
| G9: sichtbarer Picker auf beiden Desktopbackends | Beide Contextmenüs → derselbe gespeicherte Picker → eframe `SurfaceApp::update` oder Layer `OutputView::run_ui` → `desktop_surface_grid_ui::show_surface`. Die gemeinsame Fläche zeichnet Hintergrund, Kacheln, Auswahl, Umbenennen, Raumflyout und Status. Danach werden Picker und P4-Fehlerfenster gezeichnet. Layercontexts bleiben pro Output getrennt; Repaintqueue, Ping, Framedispatch und Heartbeat bleiben unverändert. |

Grenze der Karte: Sie umfasst die zu reparierenden Einstiege und ihre
betroffenen vorhandenen Verträge. Unveränderte Hostbetriebssystemadapter
werden dadurch nicht als auf realer Hardware geprüft ausgegeben. Die
weiteren Android-Tabs Dateien, Sync und Mehr werden nicht umgebogen.

## Entscheidungsrecherche und konkreter Fixplan

Erster Quellenblock, abgerufen 2026-10-26:

- [Android Tasks und Backstack](https://developer.android.com/guide/components/activities/tasks-and-back-stack): Vorhandene Aufgaben und UIzustände bleiben im Hintergrund erhalten; singleTop liefert einen neuen Intent an die vorhandene Aktivität. Ein künstlicher Activityneustart würde den belegten Fehler nur verdecken und den Zustand verwerfen.
- [Compose Side-effects](https://developer.android.com/develop/ui/compose/side-effects), `LaunchedEffect`: zustandsgebundene Navigation läuft nach Zusammensetzung; neue Schlüssel starten den Effekt neu, Verlassen der Zusammensetzung bricht ihn ab. Vorfahren öffnen den Zielzweig und verbrauchen die Anfrage nicht.
- [egui 0.29.1, Panelquellvertrag](https://raw.githubusercontent.com/emilk/egui/0.29.1/crates/egui/src/containers/panel.rs): Fenster nach den Top-Level-Panels zeichnen. Der gemeinsame Anschluss muss nach Ende von `CentralPanel.show` liegen.

Stufe eins aus dem Bestand: Beide Androidvorfahren müssen dieselben vier
Geräteaufträge erkennen. Der Surfaceprozess braucht denselben P4-Öffner wie
der Explorer. Beide Surfacebackends brauchen denselben Dialoganschluss.

Stufe zwei, verbindliche Meilensteine vor Umsetzung:

1. **G7, bestehende Androidzustände:** In `SettingsNavigation.kt` eine
   gemeinsame reine Gerätebereichsentscheidung für `OpenApps`, `OpenScreen`,
   `ShowScreenGrid`, `ShowShareRequests` ergänzen; in `AppRoot.kt` damit nur
   bei diesen Aufträgen die blockierende Detailseite verlassen und in
   `SettingsScreen.kt` den Gerätebereich wählen. Erwartung: Jeder dieser
   vier Aufträge erreicht aus Darstellung, Fehlerprotokoll und Über seinen
   bestehenden Verbraucher. Unbeteiligte Anfragen erhalten Detail und
   Bereich; Vorfahren verbrauchen keinen Geräteauftrag.
2. **G8, prozesslokaler Anschluss:** Die Standardregistrierung in
   `share_apps_ui.rs` idempotent machen; `init.rs` und
   `desktop_surface_app.rs::run` verwenden dieselbe Registrierung.
   Erwartung: Surfacebackendzweige besitzen den unveränderten
   `family_picker::open_confirmed`-Öffner bereits vor ihrer Konstruktion.
3. **G9 und G8-Fehlerrückmeldung, gemeinsame Darstellung:** Ein schmales
   `desktop_surface_windows.rs` besitzt Picker und P4-Fehlerfenster;
   `show_surface` ruft es nach dem Panel auf. Den bisherigen exklusiven
   Aufruf im eframe-Zyklus entfernen. Erwartung: Beide UI-Zyklen zeigen
   dieselben gespeicherten Picker, Lade-/Leer-/Fehlerzustände und
   asynchrone Öffnungsfehler genau einmal je Context und Frame.
4. **Gemeinsame spätere Abnahme vorbereiten:** Nach allen Produktänderungen
   gezielte native Fälle sowie Android-JVM-/Instrumentierungsfälle in der
   zugewiesenen Fläche erstellen. Die vorhandene gemeinsame Suite bleibt
   die einzige Ausführung; Registrierung durch Main melden.
5. **Eigene Anti-Regressionsreview:** Danach jede Kette oben erneut gegen
   den finalen Diff lesen, einschließlich unbeteiligter Navigation,
   Direct-/Room-Zielidentität, Requestverbrauch, idempotenter Registrierung,
   UI-Reihenfolge und vorhandener Einmalstart-/P4-Prüfungen. Statische
   Befunde und offene Laufzeitgrenzen getrennt dokumentieren.

Zweiter Rechercheblock vor Produktänderungen: `family_picker` wurde bis
`screen_launch`, `ScreenTarget` und `lib::run_gui` verfolgt; der Öffner
startet ein eigenes reguläres Viewerfenster und braucht keinen eframe-
Viewport des Surfacehosts. Die von Main ergänzte Lesefläche für beide
Appmodulregister sowie `apps_wire`, `node_apps` und `node_apps_window`
bestätigt die Authentifizierungs-/Beleggrenze. Deshalb kein eigener
Surfacefensterresolver und keine Änderung von Startbelegen oder Grants.

Alternativen: Eine Activityneuanlage verwirft UIzustand und löst die
Weiterleitung nicht. Getrennte Listen der vier Geräteaufträge würden den
belegten Auslassungsfehler wiederholbar machen. Zwei manuelle Renderaufrufe
lassen weitere Dialoganschlüsse zwischen Layer und eframe auseinanderlaufen.
Gewählt ist deshalb die gemeinsame Weiterleitung mit bestehendem Verbraucher
und eine gemeinsame Zeichnungsfunktion nach dem Panel. Neue Abhängigkeiten,
Persistenzmigrationen, Endpunktformate und Produktionsflags sind unnötig.

Bedienung: Der bestehende Menüpunkt bleibt an derselben Stelle. Nach
seiner Auswahl erscheint unmittelbar der vorhandene Picker mit seinen
realen Lade-, Leer- und Fehlerzuständen; manuelle Rücknavigation und
Bereichssuche entfallen. Öffnen und Schließen des Pickers bleiben die
bestehenden Aktionen. Programmstart und Fensteröffnung behalten ihre
gesonderte ausdrückliche Nutzerhandlung. Diese Beschreibung ist der
geplante Bedienweg; es wurde keine Interaktionszeit gemessen.

## Umgesetzter Stand

Die Meilensteine 1–3 sind im Quelltext umgesetzt. `deviceSettingsAreaFor`
enthält genau die vier Geräteauftragsarten; beide Androidvorfahren benutzen
dieselbe Entscheidung. Die wartende Anfrage öffnet den Zielzweig auch bei
kalter Zusammensetzung. Zusätzlich beobachten beide Vorfahren die vorhandenen
expliziten `AppNav.requests`, wenn deren Anfrage noch der aktuellen wartenden
Anfrage entspricht. Damit funktioniert eine erneute ausdrückliche Anfrage
auch, wenn ihr unveränderter Vorgänger noch wartet. Alte gepufferte Ereignisse
können keine neuere abweichende Anfrage überschreiben. Die Vorfahren setzen
nur Detail beziehungsweise Bereich, nie Dialog oder Startauftrag und nie
`consume`.

Dieser zusätzliche Fall wurde bei der Prüfung der bestehenden `AppNav`-
StateFlow-/SharedFlow-Verträge erkannt und in Meilenstein 1 aufgenommen.
[StateFlow, Gleichheitszusammenfassung](https://kotlinlang.org/api/kotlinx.coroutines/kotlinx-coroutines-core/kotlinx.coroutines.flow/-state-flow/),
abgerufen 2026-10-26, dokumentiert das Zusammenfassen gleicher Werte.
Der vorhandene Ereigniskanal bleibt deshalb für wiederholte ausdrückliche
Navigation maßgeblich; das neue Routing verändert dessen Speicherung nicht.

`install_window_opener()` installiert mit `OnceLock::get_or_init` immer
denselben bestehenden P4-Resolver. `App::new` und der Surfaceprozess rufen
diese Funktion auf; im Surfaceprozess liegt der Anschluss nach der
unveränderten Update-Stopp-Wache und vor Headless-/Layer-/eframe-Verzweigung.
Die gültigen Appfenster werden weiterhin ausschließlich von
`family_picker::open_confirmed` aufgelöst.

`desktop_surface_grid_ui::show_surface` zeichnet nach seinem Panel den
schmalen Unterbaustein `desktop_surface_windows`. Der Unterbaustein besitzt
nur Picker und `family_picker::notifications`. Seine Registrierung liegt
direkt beim gemeinsamen Flächenmodul; `app/mod.rs` bleibt im finalen Diff
unverändert. Der frühere zusätzliche eframe-Aufruf wurde entfernt. Der
Layerzyklus muss nicht gesondert angepasst werden: Sein vorhandener Aufruf
derselben `show_surface`-Funktion erreicht den neuen Anschluss.

## Eigene anschließende Anti-Regressionsreview

Die abschließende Prüfung wurde von demselben Fixautor nach Umsetzung und
Vorbereitung der Abnahmefälle statisch am finalen Produktdiff und den oben
ermittelten Verbrauchern durchgeführt. Es wurde kein Reviewteil delegiert.

| Prüffrage | Eigener Befund und Beleg |
| --- | --- |
| Erreichen Geräteaufträge den sichtbaren Androidverbraucher? | `DeviceSettingsRoot` verlässt ausschließlich bei den vier Gerätearten seine Detailseite; `SettingsScreen` wählt dann den Gerätebereich. `ShareScreen` setzt weiterhin Appdialog, Bildschirmansicht, Raster oder Anfrage-Scroll und verbraucht selbst. Die Wege aus frischer Aktivität und vorhandenem ThisDevice-/Devices-/Rooms-Bereich bleiben anschließbar. |
| Bleiben unbeteiligte Navigation und Einstellungen erhalten? | `SelectTab`, Dateien, Übertragungen, Synckonflikte, Updates, Hintergrund, Tresorvergleich und Launchereinstellungen liefern `null` aus der neuen Bereichsentscheidung. Beide Vorfahren ändern ihren Zustand dafür nicht. `SettingsFocus`, Bereichsauswahl und vorhandene Rückknöpfe bleiben unverändert. `DeviceSettingsRoot` ist nur für die Instrumentierung `internal` sichtbar; es entsteht keine exportierte Androidkomponente. |
| Bleiben Queue und Einmalverbrauch erhalten? | `AppNav.send`, Token, Encode/Decode, `flushQueued`, `tabFor` und Compare-and-set in `consume` sind unverändert. Die neuen Vorfahren konsumieren nicht. Ereignisse mit inzwischen abweichender aktueller Anfrage werden ignoriert. Erneutes Zeichnen und die gemeinsame Öffnerregistrierung rufen keinen Launch auf. |
| Bleiben Direct-/Roomidentität und Rechte erhalten? | Die unveränderte Zielbeziehung wird bis zum gespeicherten Picker/Startbeleg weitergegeben. Kein Locator wird neu interpretiert oder lokalisiert. Keine Änderung an Appkatalog, Benutzer-/Sitzungsbindung, AppLaunch-, ScreenView- oder ScreenControl-Rechten, Hostzustimmung oder Authentifizierung. |
| Bleibt P5 → P4 gegen falsche Fenster gebunden? | `settle`, `confirmed`, `resolve_confirmed`, `open_entry`, `ScreenTarget` und der Vieweranschluss wurden erneut gelesen. Requestbody und bestätigter Node bleiben erhalten. Die frische P4-Liste verlangt genau Request-ID, Window-ID und Generation; `ViewerSession::open` übergibt `expected_node` an seinen `StreamRequest`. Es wird kein PID-/Titel-Fallback ergänzt. |
| Bleiben Abbruch und Hostprogramm erhalten? | `family_viewer` hält reguläres Programmschließen und lokales Trennen getrennt; ein folgender Speicherdialog kann weiterhin sichtbar bleiben. `family_viewer_session`/`family_viewer_events` behalten Inputfreigabe, Layoutwechsel, Fokus- und Rechteentzug. Keine Änderung an Viewerbesitz oder Exitpfaden. |
| Sind beide Desktopzyklen vollständig angeschlossen? | eframe und Layer rufen weiterhin dieselbe gemeinsame Fläche auf. Der Picker wird dort einmal nach dem Panel gezeichnet; der zusätzliche eframe-Aufruf ist entfernt. Contextlokaler Zustand erhält die Trennung zwischen Outputs und Direct-/Roomzielen. Die Repaintcallback-/Ping-/Heartbeatkette der Layerfläche bleibt unverändert. |
| Werden Fehler sichtbar? | Der asynchrone P4-Resolver schreibt in den ursprünglichen Context. Derselbe gemeinsame Anschluss zeichnet jetzt auch dessen Fehlerfenster. Lade-, Leer-, Startstatus- und Zugriffsfehler des Pickers bleiben die bestehenden realen Zustände. |
| Bleiben Dateiaufträge, Backup und Konflikte unberührt? | Das Produktdiff ändert keine `SurfaceActions`, Dateiauswahl, Clipboard-, Rename-, Transfer-, Backup-, Konflikt- oder Retryimplementierung. Die neue Dialogzeichnung liegt nach der vorhandenen Fläche. Native Fixtures benutzen einen reinen Recorder ohne Dateiausführung und ohne Systemhintergrundpoll. |
| Sind Modul- und Größenregeln erhalten? | Der neue Dialogbaustein ist ein privates Untermodul der gemeinsamen Fläche; Testfälle sind ein privates Testuntermodul des Pickers. Alle hier geänderten/neuen Rustdateien bleiben unter 500 Zeilen und 50 KiB; der größte betroffene bestehende Quelltext ist `init.rs` mit 489 Zeilen. Keine neue Abhängigkeit und keine Produktionsplattformverzweigung. |

Eigenkorrekturen während der Review: Der Testhost überschreibt ausdrücklich
den Systemhintergrundpoll, damit die GUI-Fälle keine Produktdesktopdaten
lesen. Die Dialogregistrierung wurde unter dem gemeinsamen Flächenmodul
gehalten, statt das allgemeine Appregister um einen Desktopbaustein zu
erweitern. Die Assertions benennen nur die tatsächlich beobachtete
Fixturegrenze; ein Recorder für Dateiaufträge wird nicht als Nachweis eines
realen Programmstarts ausgegeben.

**Ergebnis der statischen Autorprüfung:** Kein weiterer belegter Fehler im
ermittelten und geänderten Navigationsweg blieb offen. `git diff --check`
für die zugewiesenen geänderten Bestandsdateien meldete keinen Fehler.
Das ist ein Text-/Whitespacebefund, kein Compile- oder Laufzeitpass.

## Vorbereitete Fälle und Registrierung durch Main

Diese Fälle wurden ausschließlich geschrieben und gelesen. Keiner wurde
lokal ausgeführt. Sie gehören in Mains bestehende einzige Remote-Suite.

Native Registrierung bereits in `share_apps_ui.rs` unter
`share_apps_ui::device_navigation_task_tests`; die Namen beginnen mit dem
vorhandenen gemeinsamen Auswahlpräfix `batches_task_`:

| Exakter Fallname | Beabsichtigtes Ergebnis und Grenze |
| --- | --- |
| `batches_task_device_navigation_surface_draws_picker_and_p4_errors` | Wiederholte Standardregistrierung, deduplizierter Picker, sichtbarer Öffnenknopf und P4-Fehler im tatsächlichen gemeinsamen `show_surface`-Zeichenweg. Gespeicherter Request bleibt erhalten; keine Surface-Dateiaktion. Reine egui-Shapes, kein Daemon, echter Hoststart oder Compositor. |
| `batches_task_device_navigation_surface_keeps_direct_room_and_output_contexts_separate` | Beide Zielbeziehungen bleiben getrennte Picker; zwei Contexts teilen ihren Pickerzustand nicht. Gemeinsame egui-Darstellung, kein physischer Mehrmonitorlauf. |
| `batches_task_device_navigation_confirmed_receipt_keeps_identity_and_rejects_invalid_bindings` | Direct und Room behalten Request/Node/Generation; fehlender Node, nicht bestätigte Startzustände und App-/Benutzer-/Sitzungs-/Fenster-/Generationsfehler erzeugen keinen bestätigten Öffnerauftrag. Keine Behauptung eines Netz-/P4-Livestarts. |

Die JVM-Klasse
`app.smartexplorer.android.ui.settings.DeviceNavigationRegressionTest`
muss Main in der bestehenden Android-JVMauswahl registrieren:

- `batches_android_device_requests_share_the_same_settings_destination`
- `batches_android_non_device_requests_do_not_override_settings_navigation`

Die Instrumentklasse
`app.smartexplorer.android.task.BatchesDeviceNavigationTaskTest`
muss Main in der bestehenden Geräteauswahl registrieren:

- `batches_android_apps_request_from_existing_appearance_opens_once`
- `batches_android_apps_request_leaves_log_and_about_details`
- `batches_android_untrusted_device_intent_preserves_the_existing_detail`
- `batches_android_screen_and_grid_requests_leave_log_and_about_details`
- `batches_android_share_requests_leave_details_and_unrelated_requests_keep_them`

Die Instrumentierung benutzt die bestehende `MainActivity`, deren echtes
`onNewIntent`, echte Composeeinstellungsseiten und die bereits vorhandenen
Schließ-Tags von Bildschirm und Raster. Ein gültiger Navtoken stammt aus
`AppNav.intentFor`; der Ablehnungsfall hat ausdrücklich einen falschen Token.
Keine Anmeldedaten oder realen Gegenstellen werden benötigt. Die Fälle
prüfen die Navigation und den einmaligen Dialogverbrauch; sie klicken
keinen Hostprogrammstart. Apps werden über lokale, nicht eingerichtete
Fixturebeziehungen geöffnet. Der eigentliche Katalog kann daher seinen
realen Leer-/Fehlerzustand zeigen. Der Anfragefall verlangt den vom lokalen
Kern geladenen Sharestatus und dessen tatsächlichen Anfrageverbrauch.
`DeviceSettingsRoot` ersetzt im Test den Inhalt derselben Hauptaktivität,
damit die vorhandenen Einstellungszustände direkt bedient werden können;
MainShells Tabwechsel wird zusätzlich vom JVMvertrag mit `AppNav.tabFor`
und von der statischen vollständigen Kette abgesichert.

Offene Laufzeitgrenzen: Kompilierung, der gemeinsame Remote-Suitelauf,
reale Android-Launcher-IPC-/OEMzustände, ein physischer Wayland-/X11-/Windows-
Surfaceprozess und der Live-P5→P4-Fensterstart wurden hier nicht ausgeführt.
Die vorbereiteten egui-Fälle prüfen den gemeinsamen Produktzeichenweg;
sie beweisen keinen echten Layercompositor. Das ist keine Releaseblockade.

## Gelesene Dateien und Quellabschnitte

Alle relativen Pfade beziehen sich auf die oben genannte Fixkopie. Der
Gesamtreview wurde aus dem im Auftrag benannten Main-Arbeitsbaum gelesen;
keine Datei in diesem Main-Arbeitsbaum wurde geändert. Wo eine Datei groß
ist, wurden ihre konkret benannten Funktionsabschnitte gelesen, nicht eine
unbeteiligte Gesamtauditierung behauptet.

Auftrag und Regeln:

- `/tmp/smartexplorer-final-review-user-contract.txt`
- `/tmp/smartexplorer-fix-device-navigation-scope.json`, einschließlich der von Main ergänzten konkreten Leserechte
- `/root/.codex/AGENTS.md`
- `/root/.codex/skills/arbeitsweise/SKILL.md` und die gelieferten MCP-Anweisungen
- `/root/.codex/skills/graphify/SKILL.md`, vorhandene Graphabfrage
- `AGENTS.md`, einschlägige Projektvorgaben
- `docs/ARCHITEKTUR.md`, Android-/Desktop-/Appfenster-/Bildschirmdatenwege
- `docs/oberflaechen/INDEX.md`
- `docs/oberflaechen/fernzugriff.md`
- `docs/oberflaechen/desktop-launcher.md`, Surface-/Launcherzeilen und Abschnitt „Laufender Gesamtkandidat“
- `docs/oberflaechen/bildschirm.md`, gezielte Bildschirm-/Fenster-/Appverträge
- `docs/plaene/2026-10-24-batches/batches.md`, P4/P5 und P5c
- `docs/plaene/2026-10-24-batches/vertraege.md`, V6/P4 und Abnahmegrenzen
- `/home/CodeProjects/smartexplorer/smart-explorer/docs/plaene/2026-10-24-batches/review-gesamt.md`, G7–G9 mit Quellen

Android:

- `android/app/src/main/AndroidManifest.xml`, gezielte Activity-/Intentkonfiguration
- `android/app/src/main/java/app/smartexplorer/android/MainActivity.kt`, Activity-/Intentannahme
- `android/app/src/main/java/app/smartexplorer/android/ui/AppNav.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/AppRoot.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/settings/SettingsNavigation.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/settings/SettingsScreen.kt`, Bereich/Fokus und sichtbare Links
- `android/app/src/main/java/app/smartexplorer/android/ui/settings/ErrorLogScreen.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/settings/AboutScreen.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/share/ShareScreen.kt`, Requestverbrauch, Bereichsfilter und Dialoge
- `android/app/src/main/java/app/smartexplorer/android/ui/share/ShareDialogHost.kt`, App-/P4-Anschlüsse
- `android/app/src/main/java/app/smartexplorer/android/ui/share/AppsDialog.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/share/FamilyTargetsDialog.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenTilesScreen.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenViewBars.kt`
- `android/app/src/main/java/app/smartexplorer/android/launcher/se/SeActions.kt`
- `android/app/src/main/java/app/smartexplorer/android/launcher/se/SeMenu.kt`
- `android/app/src/main/java/app/smartexplorer/android/launcher/se/SeController.kt`, tokengebundener Intentstart
- `android/app/src/androidTest/java/app/smartexplorer/android/task/BatchesPresentationTaskTest.kt`, vorhandener Activity-/Composeharness
- die unten genannten neu erstellten Androidfall-Dateien

Native Dateien und Graph:

- `graphify-out/graph.json`, über die dokumentierte gezielte Abfrage
- `native/Cargo.toml`, vorhandene egui-/eframe-Version und Sprachkonfiguration
- `native/review-task-manifest.json`, vorhandene Auswahlpräfixe sowie P4-/P5-Zuordnung
- `native/src/lib.rs`, Getrenntprozess- und Viewerstartverteilung
- `native/src/app/mod.rs`, Modulanschlüsse
- `native/src/app/desktop_modules.rs`
- `native/src/app/screen_modules.rs`, Viewer-/Zielmodulanschlüsse
- `native/src/app/core/init.rs`, `App::new`
- `native/src/app/core/desktop_surface_app.rs`
- `native/src/app/core/desktop_surface_grid_ui.rs`
- `native/src/app/core/desktop_surface_menu_ui.rs`, Aktivierung und Gerätemenü
- `native/src/app/core/desktop_surface_actions.rs`, SurfaceActions und vorhandene Handlungsverträge
- `native/src/app/core/desktop_surface_view_model.rs`, Outbox, Zustand und Heartbeat
- `native/src/app/core/desktop_surface_host.rs`, Hostvertrag und Hintergrundpoll
- `native/src/app/core/desktop_surface_wallpaper_ui.rs`, Hintergrundbesitz und Ladeeinstieg
- `native/src/app/core/desktop_surface_task_tests.rs`, vorhandener egui-Harness
- `native/src/app/core/desktop_surface_ui_tests.rs`, Modell-/Konfigurationsfixtures
- `native/src/app/core/desktop_surface_settings_ui_tests.rs`, gezielte Harnesssuche
- `native/src/app/core/share_apps_ui.rs`
- `native/src/app/core/share_apps_jobs.rs`
- `native/src/app/core/share_apps_store.rs`
- `native/src/app/core/family_picker.rs`
- `native/src/app/core/family_viewer.rs`
- `native/src/app/core/family_viewer_session.rs`
- `native/src/app/core/family_viewer_events.rs`
- `native/src/app/core/family_device_ui.rs`, gezielte Geräteanschlusssuche
- `native/src/app/core/screen_launch.rs`
- `native/src/app/core/screen_target.rs`
- `native/src/app/core/screen_session.rs`, Nodeweitergabe in `ViewerSession::open`
- `native/src/app/core/screen_session_backend.rs`
- `native/src/app/core/screen_viewer_tests.rs`, Frameharness
- `native/src/app/core/screen_viewer_misc_tests.rs`, Textshapes-Auswertung
- `native/src/apps/core/model.rs`
- `native/src/share/core/apps_wire.rs`
- `native/src/share/core/node_apps.rs`, authentifizierter Eingang, Katalog und Start-/Receiptzulassung
- `native/src/share/core/node_apps_window.rs`
- `native/src/desktop/core/config.rs`, Hintergrund- und Gruppenvertrag
- `native/src/desktop/os/shared/supervisor_surface.rs`, getrennter Surfaceprozess
- `native/src/desktop/os/linux_os/layer_shell/view.rs`
- die unten genannten neu erstellten nativen Dateien

## Erstellte und geänderte Dateien

Geändert:

- `android/app/src/main/java/app/smartexplorer/android/ui/AppRoot.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/settings/SettingsNavigation.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/settings/SettingsScreen.kt`
- `native/src/app/core/desktop_surface_app.rs`
- `native/src/app/core/desktop_surface_grid_ui.rs`
- `native/src/app/core/init.rs`
- `native/src/app/core/share_apps_ui.rs`

Neu erstellt:

- `android/app/src/androidTest/java/app/smartexplorer/android/task/BatchesDeviceNavigationTaskTest.kt`
- `android/app/src/test/java/app/smartexplorer/android/ui/settings/DeviceNavigationRegressionTest.kt`
- `native/src/app/core/batches_device_navigation_task_tests.rs`
- `native/src/app/core/desktop_surface_windows.rs`
- `docs/plaene/2026-10-24-batches/fix-device-navigation.md`

## Entscheidungen und Übergabe außerhalb der Schreibfläche

Alle zugewiesenen Produktfixes und die eigene statische Anti-Regressionsreview
sind innerhalb der Fläche abgeschlossen. Es bleibt keine technische
Scopeabhängigkeit für diesen Produktdiff offen. Main hat die zusätzlich
benötigten konkreten Leserechte erteilt.

Main besitzt die Registrierung der genannten Androidklassen im bestehenden
gemeinsamen Suitepfad, den gemeinsamen Graphrefresh nach Integration,
Commit/Push, den Remote-Suitelauf und den terminalen Release. Main hat die
Nachführung von `docs/oberflaechen/fernzugriff.md` und
`docs/oberflaechen/desktop-launcher.md` ausdrücklich übernommen; diese beiden
Dateien gehören nicht zur exklusiven Schreibfläche dieses Fixautors.
Die anschließende Lektüre des gemeinsamen Manifests bestätigt alle hier
benannten Fälle unter `R12-G7-G8-G9-Geraetenavigation`, einschließlich
des untrusted-Intentfalls und beider Androidklassen. Main meldet die
eingetragene Dokumentationsnachführung für die vier Androidanfragen und
den gemeinsamen Flächen-/P4-Anschluss. Damit verbleibt hierfür kein
offener Registrierungs- oder Dokumentationsauftrag an einen späteren
Fixautor.
Kein Build, Compiler, Formatter, Test, Server, Paket, Commit, Push,
Graphupdate oder Release wurde durch diesen Fixautor gestartet.

## Fortsetzung: reale Modulgrößen aus Remote-Vorbereitung 40332971455

### Erstplan aus der Fortsetzung

Der delegierte Auftrag vom 2026-10-26 setzt denselben Autorzyklus fort:
Der Remoteversuch `40332971455` meldete nach normaler Rustformatierung
Größenverletzungen vor jeder Kompilierung. Der übernommene Stand ist
`10988167`; der Arbeitsbaum war bei Fortsetzung sauber. Die aktuelle
Scope-Datei benennt ausschließlich elf Zielmodule und deren erlaubte
Extraktionsdateien. Das frühere, statisch begrenzte Größenurteil wird durch
diesen realen Formatierungsbefund korrigiert: Normale Formatierung ist für
die Größenabnahme maßgeblich, nicht die zuvor vorhandene kompakte Schreibweise.

Beauftragtes Ergebnis: Die elf Verantwortlichkeiten fachlich auf Module
unter 500 Zeilen und 50 KiB verteilen. Geräte-/Appbedienung, UI, Rechte,
Consent, Wire-/Persistenzformate, Zustandsbesitz, Abbruch und exakte
Testfunktionsnamen bleiben erhalten. Keine Zeilenverdichtung und keine
Gateausnahme. Main besitzt Git, Graph und dieselbe spätere Remote-Suite.
Lokale Formatter, Builds, Tests, Hostaufrufe und Unteragenten sind verboten.

Vorläufiger Plan: Für jedes Ziel die vorhandenen Funktionen, Datenbesitzer,
Modulanschlüsse und Testregistrierungen lesen; die zugelassenen fachlichen
Trennstellen festlegen; unveränderte Funktionskörper in zusammenhängende
Unterbausteine verschieben; Rustsichtbarkeit und Wiretypen kontrollieren;
den gesamten betroffenen Aufrufweg danach selbst statisch gegenlesen.
Noch unbekannt sind die tatsächlichen Trennstellen und die benötigte
Sichtbarkeit; diese Angaben sind aus der Lesefläche klärbar. Eine nur vom
Nutzer klärbare Angabe ist nicht erkennbar.

| Zielmodul | Normal formatierte Ausgangszeilen |
| --- | ---: |
| `app/core/analytics_access.rs` | 552 |
| `app/core/share_access_accounts_ui.rs` | 500 |
| `app/core/share_device_page_ui.rs` | 538 |
| `apps/os/android/bridge.rs` | 508 |
| `apps/os/host.rs` | 879 |
| `daemon/os/shared/discovery_pairing_ipc.rs` | 515 |
| `daemon/os/shared/ipc_protocol.rs` | 501 |
| `screen/os/linux_os/app_icons.rs` | 585 |
| `screen/os/windows/app_icons.rs` | 554 |
| `share/core/access_runtime_tests.rs` | 525 |
| `share/core/node_apps.rs` | 977 |

Die Pfade der Tabelle liegen unter `native/src/`. Quelle sind die gelesenen
Dateigrößen am übernommenen Commit; Befundquelle des Auftrags sind außerdem
`/tmp/smartexplorer-rv1-40332971455/format.json` und das gefilterte
`/tmp/smartexplorer-rv1-size-inventory.json`. Kein Compilerjob dieses
Remoteversuchs lief; ein Laufzeiturteil wird daraus nicht abgeleitet.

### Bestand, zusätzliche Verknüpfungskarte und Strukturplan

Die zusätzliche Graphabfrage zu `AppHost`, `node_apps`, Discovery,
Analysezugriff, Konten-/Geräteseite und Icons ordnet die vorhandenen Knoten
zu. Die tatsächlichen Funktionsblöcke wurden anschließend in den elf
zugewiesenen Dateien gelesen. Der Graph ist weiterhin eine Navigationshilfe;
seine Zeilenstände ersetzen nicht die übernommenen formatierten Quellen.

| Bisherige Verantwortung und betroffene Kette | Fachliche Extraktion und Erhalt |
| --- | --- |
| `analytics_access`: Analyseergebnis → DeniedRoots/Teilscan → Rechteanfrage → erneute Ergebnisauswertung → Problemliste/Kopierbericht | `analytics_access_issues.rs` besitzt Berichtsformat und Problemlistendarstellung; `analytics_access_tests.rs` enthält denselben bisherigen `tests`-Modulinhalt. Generation, Consentgeneration, Worker und Pendingreceiver bleiben in `AnalyticsAccess`; die Appmethoden bleiben im Ursprung. Reexports erhalten `issue_report`/`issues_ui`. |
| `share_access_accounts_ui`: Befehletab → Kontextzustand → Kontenliste/OS-Bewertung → ausdrückliche Adminaktion → Ergebnis → Neuladen | `share_access_accounts_actions.rs` besitzt den vorhandenen Sharedzustand, Workerstart und Aktionsausführung. Die Form-, Konto- und Ergebnisdarstellung bleibt im Ursprung; `ui_accounts` wird mit unveränderter Sichtbarkeit reexportiert. Context-ID, Arc/Mutex, Busyprüfung, Elevation und Repaint bleiben gleich. |
| `share_device_page_ui`: Kontakt-/Grant-/Pinnedgerät → Verbindung/Vertrauen → Bildschirm → Appkatalog/Familien/Login/TCP/Routen → Rechte-/Freigabeänderung | `share_device_connection_ui.rs` übernimmt Verbindungs- und Bildschirmabschnitt. Die ursprünglichen Kontakt-/Grant-/Pinnedseiten bleiben Eigentümer von Geräteauswahl, Profiles, Persistenz und den nachgelagerten App-/P4-Anschlüssen. |
| Androidbridge: JNI-Katalog/Dispatch → noncegebundener Zustand → E4-Hold → einmaliger JNI-Claim → OS-Dispatch → lokale Result-/Consentfortsetzung | `bridge_admission.rs` besitzt ausschließlich Admissionregistry, Hold/Drop und Permit-/Claimprüfung. Die vorhandenen Bridgequeues, Sinks, Wartebedingungen und Katalog-/Launch-/Iconantworten bleiben im Ursprung. Die Zulassungstypen/-funktionen werden unverändert reexportiert; keine zweite Registry. |
| AppHost: authentifizierter Peer-/Requestbesitzer → frischer Katalog/Benutzer/Sitzung → Admissionguard → dauerhaftes Claimjournal → OS-Dispatch → Dispatch-/Fensterbeleg → Wiederabfrage/P4 | `host_catalog.rs` besitzt Katalog/Icon und Faktenvalidierung; `host_launch.rs` Startbeleg, Einmalclaim, Consentfortsetzung und Dispatch; `host_lifecycle.rs` Belegabfrage, lokale Antworten und P4-Korrelation. `AppHost` bleibt derselbe Typ mit denselben Feldern, Journalen und Mutexen; seine Methoden bleiben über denselben Typ erreichbar. |
| Discovery: Stream/Beobachter → gepufferter Terminalbeleg → exakter Exchangebesitzer → Start/Wait/Cancel → Verbindungsende → Ownerdrop | Nur die vorhandenen Tests gehen nach `discovery_pairing_ipc_tests.rs`; das registrierte Modul heißt weiterhin `tests`. Beobachter, Terminalpuffer, Verbindungswache, Wait- und Cancelwege bleiben unverändert zusammen. |
| Daemon-IPC: bestehender Request/Response → Mountfähigkeiten/Disposition → VFS-Fakten und Rückabbildung | `ipc_protocol_mount_capabilities.rs` enthält genau Mount-DTOs und deren vorhandene Konvertierungen. Gleich sichtbare Reexports erhalten die Aufrufer; derive-/serde-Attribute, Variantennamen, Felder, Defaults und Fähigkeitssemantik bleiben unverändert. |
| Linuxicons: installierter Desktopkatalog → eindeutige Desktop-ID → Theme/SVG/Raster → gebundene Referenz; X11-WM_CLASS/PID/NET_WM_ICON als kontrollierter Rückfall | `app_icon_catalog.rs` besitzt begrenztes Kataloglesen und eindeutige Zuordnung; `app_icon_netwm.rs` X11-Rückfall und Fensteridentität. Theme-/SVG-/Hash-/Antwortlogik bleibt im Ursprung. Keine neue Pfadauflösung, Dateiausführung oder Exec-Auswertung. |
| Windowsicons: HWND/Prozess/AUMID → installierter Shellkatalog → Icon/Shellbitmap → begrenzte Rasterantwort | `app_icons_shell.rs` übernimmt den vorhandenen Shellbitmapauszug einschließlich HBITMAP-/DC-Abschluss und Alphaaufbereitung. Die übrigen Hostidentitäts-/Katalog-/Iconpfade bleiben zusammen; COM-Initialisierung und Threadbesitz werden nicht verschoben. |
| Access-Runtimeabnahme: Grant-/Identitäts-/Konten-/Deadlineänderung → bestehende Zulassungsregistry → selektiver Rechteentzug und Ressourcenerhalt | `access_runtime_lifecycle_tests.rs` übernimmt die vorhandenen Lebenszyklusprüfkörper. Die ursprünglichen Testfunktionen bleiben als dünne Registrierungsfunktionen am ursprünglichen Modulpfad und rufen genau ihren verschobenen Körper auf. Kein Testname und keine Auswahl ändert sich. |
| NodeApps: authentifizierter Stream/Principal → Katalog-/Icon-/Launch-/Receiptkommando → E4-Bestätigung/Zulassung → AppHost → P4-Pin und lokale Androidfortsetzung | `node_apps_client.rs` hält Streamhandshake und Bestätigungswarten; `node_apps_host.rs` Kommandodispatch, sichtbaren Katalog, Requestfakten und Appzulassung; `node_apps_platform.rs` lokale Result-/Consentcallbacks. `NodeApps` behält Configuration, Pending-AccessHandles, Watchset, Slots und Ownerkey. Lokale Grant-/Konfigurationsmethoden und P4-Windowadapter bleiben am Ursprung angeschlossen. |

Entscheidungsrecherche, abgerufen 2026-10-26:
[Rustsichtbarkeit](https://doc.rust-lang.org/reference/visibility-and-privacy.html)
erlaubt Nachkommen Zugriff auf private Elternfelder; beschränkte Sichtbarkeit
und Reexports müssen die bisherige Vorfahrenreichweite erhalten.
[Inhärente Implementierungen](https://doc.rust-lang.org/reference/items/implementations.html)
erlauben mehrere impl-Blöcke desselben Cratetyps; der Aufrufpfad bleibt am
Typ. Deshalb verbleiben Datenbesitzer im Ursprung. Neu nötige Geschwister-
hilfen sind nur zum ursprünglichen Verantwortungsmodul sichtbar. Eine neue
öffentliche API oder ein anderer Registrybesitzer ist nicht erforderlich.

Verbindlicher Ablauf vor Quelländerungen:

1. Die vorgenannten fachlichen Blöcke ohne Verhaltensänderung in genau die
   erlaubten Extraktionspfade verschieben. Bereits formatierte Zeilen
   übernehmen; nur die für neue Modulrahmen nötige Einrückung ändern.
2. Vorhandene Aufrufnamen durch Reexports oder dieselben inhärenten Methoden
   erhalten. Relative `super`-Pfade um genau die neue Elternebene korrigieren;
   eingeschränkte Sichtbarkeit auf denselben bisherigen Vorfahren binden.
3. Die unveränderten Zustandsfelder, Locks, Guards, Wire-/Serdeattribute,
   Testnamen und Rückgabepfade gegen den übernommenen Commit vergleichen.
   Erforderliches Ergebnis: Keine doppelte Registry, kein zusätzlicher
   Launch, keine geänderte Fehler-/Abbruchreihenfolge, keine verlorene
   Testregistrierung.
4. Quellen statisch auf Größen, Modulpfade und unveränderte Funktionskörper
   prüfen. Die tatsächliche Format-/Kompilier-/Suitebestätigung gehört
   ausschließlich zur späteren Wiederholung desselben Remoteeinstiegs.
5. Derselbe Autor liest anschließend die betroffene gesamte Kette erneut,
   dokumentiert seine Anti-Regressionsbefunde und meldet die endgültigen
   Dateien sowie präzise verbleibende Laufzeitgrenzen an Main.

Zweite Lückenprüfung: Die drei besonders gekoppelten Flächen wurden vor
der Extraktion zusätzlich geprüft: AppHost-Guards laufen weiterhin durch
denselben `dispatch`; NodeApps-Pendingbesitz bleibt bis lokaler Fortsetzung
am selben Node; Windows-DC/HBITMAP und Android-AdmissionHold behalten ihren
Abschlussort innerhalb desselben Funktionskörpers. Ein `include!` als reine
Größenumgehung, neue Schematypen, geänderte Tests oder eine Gateausnahme sind
ausgeschlossen. Die zugelassenen Extraktionspfade reichen nach dieser
Bestandslektüre aus.

### Umgesetzte Modulstruktur und endgültige Dateien

Die geplante Aufteilung ist umgesetzt. Jedes der elf Ursprungsmodule und
jede der 17 erlaubten Extraktionsdateien bleibt im geschriebenen, aus der
Remoteformatierung übernommenen normalen Layout unter 500 Zeilen und
50 KiB. Es wurden keine Zeilen zusammengezogen. Die erneute Ausführung des
Remoteformatters bleibt der verbindliche abschließende Größenbeleg.

| Geändertes Ursprungsmodul unter `native/src/` | Zeilen nach Extraktion |
| --- | ---: |
| `app/core/analytics_access.rs` | 453 |
| `app/core/share_access_accounts_ui.rs` | 397 |
| `app/core/share_device_page_ui.rs` | 417 |
| `apps/os/android/bridge.rs` | 420 |
| `apps/os/host.rs` | 266 |
| `daemon/os/shared/discovery_pairing_ipc.rs` | 451 |
| `daemon/os/shared/ipc_protocol.rs` | 407 |
| `screen/os/linux_os/app_icons.rs` | 357 |
| `screen/os/windows/app_icons.rs` | 465 |
| `share/core/access_runtime_tests.rs` | 389 |
| `share/core/node_apps.rs` | 249 |

| Neu erstelltes fachliches Untermodul unter `native/src/` | Zeilen |
| --- | ---: |
| `app/core/analytics_access_tests.rs` | 49 |
| `app/core/analytics_access_issues.rs` | 58 |
| `app/core/share_access_accounts_actions.rs` | 110 |
| `app/core/share_device_connection_ui.rs` | 128 |
| `apps/os/android/bridge_admission.rs` | 107 |
| `apps/os/host_catalog.rs` | 127 |
| `apps/os/host_launch.rs` | 282 |
| `apps/os/host_lifecycle.rs` | 226 |
| `daemon/os/shared/discovery_pairing_ipc_tests.rs` | 65 |
| `daemon/os/shared/ipc_protocol_mount_capabilities.rs` | 102 |
| `screen/os/linux_os/app_icon_catalog.rs` | 148 |
| `screen/os/linux_os/app_icon_netwm.rs` | 100 |
| `screen/os/windows/app_icons_shell.rs` | 96 |
| `share/core/access_runtime_lifecycle_tests.rs` | 157 |
| `share/core/node_apps_client.rs` | 183 |
| `share/core/node_apps_host.rs` | 412 |
| `share/core/node_apps_platform.rs` | 154 |

Zusätzlich geändert wurde ausschließlich dieser eigene Bericht. Es wurde
kein fremder Bericht, Suiteeintrag oder zentraler Orchestrierungspfad
geändert. Die frühere G7/G8/G9-Produktkorrektur bleibt erhalten.

Der einzige zusätzlich gekapselte Ausdruck ist die bereits vorhandene
Deadlineabfrage im Android-Dispatchtimeout: `admission::claimed_until(id)`
liest unter derselben Registrymutex denselben `Option<Instant>` und gibt
denselben Poisonfehler zurück. So bleibt die Registry privat beim neuen
Admissionbesitzer. Das ist keine neue Zulassungsentscheidung und verändert
weder Frist, Claim, Guardhaltung noch Wiederholungsverhalten.

### Eigene Anti-Regressionsreview der Strukturkorrektur

Derselbe Autor hat nach der Extraktion die vollständige zusätzliche Karte
oben erneut am finalen Quellstand geprüft. Neben dem manuellen Gegenlesen
wurde der bereits installierte Rust-Tree-sitter-Parser aus der vorhandenen
Graphify-Umgebung benutzt. Er liest und parst Quelltext; es wurden damit
keine Produktfunktionen, Tests, Formatter, Compiler oder Hosts gestartet.

Vergleichsbasis war je Ziel die konkrete Datei aus `10988167`. Für den
Funktionskörpervergleich wurden ausschließlich Kommentare, Formatwhitespace
und die gezielt hinzugefügte relative Elternebene normalisiert. Strings,
Operationen, Fehlertexte und Kontrollfluss blieben Bestandteil des Vergleichs.
Alle ursprünglichen Funktionskörper sind im Ursprungsmodul oder zugeordneten
Untermodul unverändert vorhanden; die einzelne Androiddeadline-Kapselung
wurde separat direkt gegen den ursprünglichen gelockten Ausdruck geprüft.
Alle ursprünglichen Structs, Enums, Typaliase und Konstanten stimmen bis
auf die erforderliche Modul-Sichtbarkeit überein. Die ursprünglichen
Testregistrierungsnamen sind ebenfalls erhalten.

Die erste statische Syntaxlektüre erkannte einen eigenen Extraktionsfehler:
Eine für `DesktopIcon` bestimmte Feldsichtbarkeit war auch an den Parameter
`desktop_for::id` geraten. Dieser reine Extraktionsfehler wurde vor Übergabe
korrigiert; die anschließend vollständig wiederholte statische Parser-
und Körperprüfung meldete keinen Syntaxfehler und keine unaufgelöste
Körperabweichung. Ein zusätzliches Leerzeilenproblem am Ende der Kontenseite
wurde ebenfalls vor dem abschließenden `git diff --check` entfernt.

| Erhaltungsvertrag | Eigener abschließender Befund |
| --- | --- |
| Bestehende Aufrufer/Sichtbarkeit | `ui_accounts`, Analysebericht/-UI, Mount-DTOs, Android-Admissiontypen/-funktionen und Linuxfensteridentität werden unter ihren bisherigen Namen reexportiert. `pub(in super::super)` erhält beim Umzug ursprünglich zum Elternmodul sichtbarer DTOs und Funktionen genau denselben Vorfahren. AppHost- und ShareIrohNode-Methoden bleiben inhärente Methoden desselben Typs. Private modulübergreifende Helfer sind nur am bisherigen Verantwortungsmodul sichtbar. |
| Analyseabbruch und Zugriffsentscheidung | `AnalyticsAccess` behält Generation, Consentgeneration, Pendingreceiver, DeniedRoots und Partialworker. Ausgelagert sind nur Problemdarstellung und vorhandene Tests. Erneute Rechteanfrage, lokale/remote Unterscheidung und Teilscancancellation bleiben in denselben Appmethoden. |
| Konten-/Gerätebedienung | Labels, Context-ID, Kontoauswahl, ausdrückliche Adminaktionen, Busyprüfung und Ergebnis-/Reloadfolge sind unverändert. Kontakt-/Grant-/Pinnedseiten behalten ihre Profiles und Persistenz. Verbindung, Vertrauen, Dateifreigabe, Bildschirm, Appkatalog, Fensterfamilie, Login, TCP und Routen bleiben an denselben sichtbaren Bedienwegen. |
| Androidzulassung | Genau eine Admissionregistry liegt nun beim Admissionmodul. Request-ID, deadline, valid-Closure und claimed_until bleiben dieselben Daten. Hold/Drop, einmaliger Claim und Warten auf den tatsächlichen Aufruf bleiben unverändert. Die neue Deadlinefunktion ist ausschließlich eine typisierte Lesefassade auf diese Registry. |
| AppHost-Consent und Einmalstart | Device, Platform, Journal, Serialmutex, Ownershiproot und Correlationsmutex bleiben im ursprünglichen AppHost. Journalbind/Claim, Consentfortsetzung, Admissionguard, Deadline und Dispatch bleiben innerhalb derselben Funktionskörper und Reihenfolge; die Methodenauslagerung erzeugt keinen neuen Besitzer oder Start. |
| AppHost-Beleg/P4 | Requestbody-, Katalog-, App-, Benutzer-, Sitzungs- und Fensterbindung bleiben unverändert. Dispatchantworten können weiterhin keine P4-Aktivierung erfinden. Receipt, Correlation, WindowPending und NoteWindow ändern weder Wireformat noch Journalinhalt. |
| Discovery | Produktionscode, Observerbesitz, Terminalpuffer, Listen-/Exchangewarten, Cancel und Verbindungswache sind unverändert. Die vorhandenen Tests behalten das Modul `tests` und damit ihre bisherigen vollqualifizierten Pfade. |
| IPC-/Persistenzformate | Mountvariantennamen, Feldnamen, serde-/derive-Attribute, Defaults und VFS-Konvertierungen sind unverändert. Apphost-PeerRequestBinding und die gespeicherten Apprequest-/Belegtypen wurden nicht geändert. Kein Schema oder Protokoll wird angehoben oder migriert. |
| Linuxicons | Catalogbudget, Tombstones, Uniqueness, literal ID/Classbindung, Dateigrenzen, Theme-/SVG-/Rasterlimits und Hashvergleich bleiben erhalten. WM_CLASS/PID/NET_WM_ICON-Abfrage erhält durch den Umzug nur die korrekte zusätzliche Elternebene; kein ungeprüfter Pfad oder Exec wird ergänzt. |
| Windowsicons | COM-Apartmentinitialisierung bleibt am bisherigen Caller. Shellbitmapoperation, Pixel-/Alphaberechnung, DC-/HBITMAP-Freigabe und sämtliche Fehlerpfade sind identisch. FFI und Betriebssystemimporte bleiben ausschließlich im Windowsadapter. |
| NodeApps-Rechte und Ressourcenbesitz | Configuration, Pendingkontexte mit AccessHandle, Windowwatchset, Semaphore und Ownerkey bleiben am ursprünglichen Node. Handshake/Principalprüfung, Katalogfilter, Ask-Ticket, Policyrevision, Verbindungsabbruch, Deadline, Accessregistry und Guardübergabe bleiben in denselben Körpern. Genau ein Platformnode-Weakbesitzer und dieselben lokalen Callbackwege bleiben vorhanden. |
| Testregistrierung | Analyse-/Discoverymodule behalten ihren Namen `tests`; AppHost behält `batches_launch_task_tests`. Die vier ausgelagerten Access-Lebenszykluskörper werden von den ursprünglichen registrierten Testfunktionen aufgerufen. Kein Testsuffix, vollqualifizierter Registrierungsort, Filter oder Suitekommando wird ersetzt. |
| Strukturgrenzen | Alle zugewiesenen Quellen liegen im aktuellen Layout unter 500 Zeilen und 50 KiB; die größte ist der vorhandene Windowsiconadapter mit 465 Zeilen. Es gibt keine Zeilenverdichtung, keinen include!-Ausweichpfad, keine Gateausnahme, keine neue Abhängigkeit und keine neue Plattformverzweigung im Kern. |

`git diff --check` für die elf geänderten Ursprungsmodule war abschließend
fehlerfrei. Eine zusätzliche statische Textlektüre prüfte auch die neuen
Dateien auf Whitespace, Konfliktmarker und beide Größenlimits. Die Syntax-,
Körper- und Datendefinitionsvergleiche sind ausdrücklich keine Typprüfung,
kein Linking und kein Laufzeitpass. Rustsichtbarkeit wurde am Quelltext
und am Rustreferenzvertrag geprüft; ihre Compilerbestätigung bleibt offen.

### Exakte bestehende Fallnamen und Registrierung

Es wurde keine zusätzliche Suite und kein zusätzlicher Testfall angelegt.
Diese bereits vorhandenen Registrierungen bleiben exakt erhalten:

Analyse:

- `analytics_access_task_remote_access_never_requests_local_privileges`
- `analytics_access_task_report_lists_each_retained_path_and_omissions`
- `analytics_access_task_new_scan_resets_prompt_without_canceling_consented_launch`

Discovery:

- `batches_task_p29_gui_drain_preserves_exact_owned_terminal_result`
- `batches_task_p29_observer_overflow_fails_closed_and_dead_observers_expire`
- `batches_task_p29_connection_end_cancels_even_before_start_acknowledgement`

Access-Runtime:

- `p18d_targeted_recheck_preserves_unrelated_app_user_port_and_old_rights`
- `p18d_persisted_widening_keeps_lease_narrowing_revokes_before_publication`
- `p18d_session_user_and_relation_changes_end_only_bound_operations`
- `p18d_admission_holds_auth_through_registration_and_resource_publication`
- `p18d_poisoned_auth_denies_without_panic_or_resource_publication`
- `p18d_deadline_wakes_without_screen_and_later_admission_wakes_idle_timer`
- `p18d_account_and_earlier_deadline_changes_revoke_only_affected_leases`
- `p18d_failed_persisted_identity_check_keeps_live_authority_unchanged`
- `p18d_room_login_is_denied_even_with_an_explicit_matching_room_grant`

Der vorhandene G7/G8/G9-Eintrag `R12-G7-G8-G9-Geraetenavigation` und seine
im ersten Berichtsteil benannten Fälle werden durch die Strukturkorrektur
nicht verändert. Main benötigt für diese Extraktion keine neuen
Testnamen oder Auswahlpräfixe. Die neuen Rustdateien müssen wie die
Ursprünge am bestehenden Remoteformat-/Größen-/Kompilierweg teilnehmen.

### Zusätzliche gelesene Quellen, Entscheidungen und Grenzen

Zusätzlich zur früheren Dateiliste wurden für diese Fortsetzung die elf
Ursprungsmodule der Größentabelle, alle 17 neuen Untermoduldateien, die
aktualisierte Scope-Datei, der eigene Bericht und die beiden genannten
Remote-Befunddateien gelesen. In `native/src/apps/os/mod.rs`,
`native/src/apps/os/android/mod.rs` und `native/src/apps/mod.rs` wurden die
vorhandenen Plattform- und AppHost-Exports geprüft. Die Graphabfrage und
die beiden oben verlinkten Rustreferenzen ergänzen diese Quellen.
Die Ursprungskörper und Definitionen wurden jeweils aus demselben
`10988167` gelesen; fremde Repositoryflächen wurden nicht erkundet.

Die fachlichen Extraktionspfade reichten aus; es bleibt kein konkreter
Scopebedarf für die Umsetzung offen. Die zusammenhängende Appkette und
die eigenen ursprünglichen G7/G8/G9-Fixes bleiben unverändert anschließbar.
Erledigt sind Strukturplan, Verknüpfungskarte, Quellaufteilung und eigene
anschließende statische Anti-Regressionsreview durch denselben Autor.

Main besitzt weiterhin den gemeinsamen Graphrefresh, Commit/Push,
die erneute Format-/Größen-/Compilerbestätigung und die Wiederholung des
bestehenden einen Remote-Suiteeinstiegs. Reale OS-, Consent-, Netzwerk-,
Geräte- und UIausführung bleiben hier ungeprüft. Dieser Autor hat weder
lokal formatiert, kompiliert, gebaut oder getestet noch Hosts aufgerufen,
Unteragenten gestartet, Gitmutationen, Graphupdates oder Releaseaktionen
ausgeführt. Die Fortsetzung behauptet keinen erfolgreichen Remote-Suitelauf.

## Fortsetzung: Compilerdiagnosen von Lauf 40354605251

### Erstplan und vollständiger übernommener Befund

Stand dieser Fortsetzung ist `d95dc404e834edfe7be0e1c96be162b36865efa9`.
Die vollständige zugewiesene native und Android-Diagnoseliste aus
`/tmp/smartexplorer-rv1-40354605251/device-navigation-errors.json` wurde
gelesen; der Remoteversuch ist laut Auftrag vollständig beendet. Die neue
Scope-Datei ersetzt die vorige Strukturfläche. Main besitzt insbesondere
den gemeinsamen Remoteeinstieg, Graph, Git und Integration. Dieser Autor
ändert nur die erneut ausdrücklich zugewiesenen Dateien.

Beauftragtes Ergebnis: GUI, CLI und Android lesen Geräterechte über dieselbe
vorhandene reale IPCgrenze; Appstart und TCP erhalten Autorisierung,
Widerruf, Deadline und echte Besitzer. Desktopziele, App-/Fensterfelder und
alle sieben Capabilities bleiben im Produkt und in bestehenden Fixtures
vollständig erhalten. Keine Prüfung oder vorhandene Fixtureerwartung darf
entfallen. Geplant sind zunächst kanonische Definitionen und Verbraucher,
danach ein konkreter Plan und die minimalen verbundenen Korrekturen mit
eigener anschließender statischer Anti-Regressionsreview.

| Zugewiesene Diagnosen | Fehlerhafte Funktion und notwendige Grenze |
| --- | --- |
| C13, C14, C15, A11 | GUI-/CLI-/Android-Geräterechte rufen die private Implementierung `daemon::access_ipc` direkt auf. Ein schmaler bestehender Funktionszugang muss an der Daemonfassade sichtbar sein; keine Modulfreigabe oder lokale Ersatzberechnung. |
| C22, A12 | Der TCP-Antwortfuture leiht ein temporäres `Opened`-Objekt über dessen Lebensdauer hinaus. Antwortbesitz muss über dieselbe widerrufs- und timeoutgebundene Sendefolge erhalten bleiben. |
| C23, A13 | Die App-Receiptprüfung übergibt einen fehlerhaft typisierten temporären Authmutexguard. Die tatsächliche gelockte Autorität muss bis zur abschließenden Principalprüfung bestehen bleiben. |
| C33 | Die Completionfixture initialisiert `AccessGrantChange` ohne das vorhandene Constraintsfeld. Ihre bisherigen Werte und Assertions bleiben erhalten. |
| C37, C38, C39, C44, C47, C48, C49, C50, C51, C52 | Bestehende CLI-/Daemon-/Windowlist-/Wirefixtures fehlen die hinzugekommenen App-ID-/Iconfelder. Die bisherigen Namen, Geometrien, escaping-/Bytebudget- und Auswahlbelege bleiben erhalten; fehlende Metadaten werden ausdrücklich repräsentiert. |
| C45 | Der Desktop-ScreenTarget-Konstruktor nennt `expected_node`, der Typ enthält dieses Feld aber nicht. Desktop- und Appziel müssen den vorgesehenen optionalen Pinvertrag vollständig erhalten. |
| C53, C56 | Persistenzfixture und Completionhilfe behandeln noch vier statt sieben Capabilities. Der vollständige kanonische Capabilityumfang und die alten Grantbedeutungen bleiben erhalten; die Auswahl wird nicht auf alte Capabilities begrenzt. |
| A08, A09, A10 | Android-Node und -Dispatcher beziehen die TCP-ALPN über einen Pfad, der die Konstante nicht exportiert. Beide müssen denselben vorhandenen kanonischen Protokollbezeichner verwenden. |

Noch aus dem Bestand zu klären sind die genaue Fassadensichtbarkeit,
Future-/Guardgrenzen, optionalen Metadaten-/Pinattribute und Legacy-
Capabilityerwartungen. Eine nur vom Nutzer zu liefernde Angabe ist nicht
erkennbar. Lokale Compiler, Builds, Formatter, Tests, Server, Installationen,
Gitmutationen, Graphupdates und Unteragenten bleiben ausdrücklich verboten.

### Zweitplan: belegter Bestand und Verknüpfungen

Die Planungstiefe ist 2: gekoppelte Reparaturen innerhalb vorhandener
Verträge. Die Ursachen sind durch die Remote-Diagnosen und die folgenden
Definitionen belegt; neue Funktionen, Bibliotheken oder Bedienwege sind
nicht erforderlich.

| Einstieg und direkt betroffene Funktion | Direkte und indirekte bestehende Kette; zu erhaltende Grenze |
| --- | --- |
| `device_rights_ui::ui` | Geräte-/Schlüsselgebundener GUI-Snapshot → eigener Hintergrundthread → `daemon`-Fassade → vorhandenes `access_ipc::device_access_rights` → `AccessIpcRequest::DeviceRights` in authentifiziertem `IpcRequest::Access` → laufender Daemon → `service.device_access_view` → typisierter `DeviceAccessView` → derselbe Snapshot mit Aktualisierungs-/Fehleranzeige. |
| `grants_device_view::show` | Profilselektion nach gepinnter Geräteidentität → genau ein `(device_id, public_key)` → bestehendes `ensure_worker_ready` → dieselbe Daemon-Fassade/IPCgrenze → unveränderte JSON- und Beziehungszeilen. |
| mobiles `share_access` | Gespeicherter Kontakt mit bestätigter Geräte-ID und öffentlichem Schlüssel oder explizite Geräte-/Schlüsselargumente → dieselbe Daemon-Fassade/IPCgrenze → bisherige Fehlerübersetzung und Zuordnung jeder Beziehung zu ihrem konkreten Quellenschlüssel. |
| `ShareIrohNode::start_with_power`, `spawn_accept_loop` und `dispatch_connection` | Kanonische `tcp_forward::TCP_ALPN`-Bytes → Endpointanzeige → bisherige Known-/Unknown-Handshakeslots und Authstrafen → Dispatcher → `handle_tcp_connection`. Der falsche Konstantenpfad wird ersetzt; kein Protokollwert und kein Zulassungsschritt ändert sich. |
| `handle_tcp_connection` / `serve_tcp` | Transport-Closeguard, Incomingtracking und Tunnelslot → aktive Freigabe und Pfadpolicy → gemeinsame aktive Handshakefrist → Relationproof mit gelockter Autorität → gebundener Principal → Zielzulassung → `registration`, `access`, Zieladresse und erster autorisierter Socket → `Opened`-Antwort unter Widerruf und 20-Sekunden-Frist → unveränderte streamweise Tunnel-ID-Prüfung, TTL, Cancel, Socketbesitz und abschließende Registrierung-/Zugriffsfreigabe. |
| `apps_execute` / `Receipt` | Aktive Freigabe → konfigurierter Host → `bind_peer_request` mit konkretem `owner_key` und unverändertem Request → Fensterprobe im Blockingjob → erneute gelockte `principal_grant`-Prüfung → derselbe Receipt. `owner_key` umfasst Beziehungstyp, Beziehungs-ID, Geräte-ID, öffentlichen Schlüssel, Fingerprint und Node-ID. Kein Guard überquert eine Awaitgrenze. |
| `apps_execute` / `Launch`, `run_app` | Frischer Katalog und Sitzung → `access_request` → bestehende Bestätigung → konkrete Request-/Ownerbindung → cancelgebundener Blockingjob → Accessregistry oder ursprüngliche noch gültige Consentzulassung → Authorityguard samt Deadline → Hostlaunch/Consentfortsetzung → Fensterprobe/-wache. Dieser Pfad bleibt bei der Receiptkorrektur unverändert. |
| Desktop-`ScreenTarget::new`, `for_tile`, `to_arg` | `open_target::device_screen` verweigert Identitätskonflikte → konkreter Directkontakt oder Raum mit bekannter Geräte-ID → bisherige Quell-/Controlwerte → `StartRequest::ScreenView` mit Ziel-JSON → Prozessparameter nach `--screen-view` → App-`ScreenTarget::parse`. Der Appzieltyp besitzt bereits `expected_node: Option<String>` mit Default und Auslassung bei `None`. Derselbe optionale Pin gehört auch zum Desktoptyp; gewöhnliche Desktopziele behalten `None` und ihre bisherigen JSONfelder. |
| `WindowEntry` / `AppEntry` und Fixturehelfer | Produktdefinitionen besitzen optionale `app_id`-/`icon`-Metadaten mit Default und Auslassung bei `None` → Listenbegrenzung und Validierung → Wireframes/CLI-/Daemon-Stillansicht. `IconRef` bindet App-ID, SHA-256 und Maße; seine eigene Validierung und aktuelle autorisierte Listenzuordnung bleiben unberührt. Die älteren Fixtures repräsentieren ausdrücklich fehlende Metadaten und behalten deshalb ihre bisherigen serialisierten Größen und JSONschlüssel. |
| `decisions`, Legacy-/Unknown-Wirefixtures | Vollständige kanonische Reihenfolge `ExecCommands`, `ExecTerminal`, `ScreenView`, `ScreenControl`, `AppLaunch`, `LoginAccess`, `TcpForward` → sieben Entscheidungen. Die ersten vier Erwartungen bleiben erhalten, die drei Kontextcapabilities bleiben bei alten Grants `Off`. Unbekannte Wireobjekte bleiben vollständig erhalten und schalten alle sieben Capabilities aus. |
| `capability_name`, `access_state_label`, `approval_candidates_from` | Exhaustive Benennung derselben sieben Capabilities → bestehende Zustandslabels und Approvalcompletion. Bestehende englische Namen bleiben erhalten; Appstart, Loginzugriff und TCP erhalten explizite Namen. Es entsteht weder ein Wildcardfall noch ein kleinerer Capabilityumfang. |

Die bestehende IPCfunktion lädt weiterhin Token und Adresse des laufenden
Daemons, verwendet dessen vorhandene Verbindungs-/Antwortfristen und
akzeptiert ausschließlich die passende typisierte Antwort. GUI und Android
erhalten keinen neuen Workerstart; die CLI behält ihren vorhandenen Start.
Das Implementierungsmodul `access_ipc` bleibt privat.

Zusätzlicher belegter Fixturefehler: `access_grant_wire::ACCESS_VERSION`
ist inzwischen 2, `parse` unterstützt ausdrücklich 1 und 2. Der bisherige
Unbekannt-Versionsfall verwendet aber ebenfalls 2. Er muss eine tatsächlich
unbekannte Version verwenden; sämtliche Ablehnungs-, Speicher- und
Wiederaktivierungsassertions bleiben bestehen. Die separate unlesbare
Version-1-Fixture und die unbekannten Schlüssel/Felder bleiben erhalten.

Für die abschließende Detailkarte wurden die direkt referenzierten
Definitionen `access_authorize.rs` und `node_tcp_target.rs` als konkrete
zusätzliche Lesefläche bei Main angefordert und nach dessen Freigabe
vollständig gelesen. Die zusätzliche Desktopkette wurde nach expliziter
Freigabe in `open_target.rs`, `protocol_tests.rs` und
`screen_viewer_misc_tests.rs` nachgelesen. Änderungen benötigen diese
Definitionen und Fixtures nicht. `tiles_tests.rs` wurde ebenfalls gelesen;
die zunächst vermutete ScreenTarget-Fixture befindet sich dort nicht.

### Entscheidungsrecherche und endgültiger Arbeitsplan

Primärquellen, am 2026-10-26 gelesen: Die Rust-Referenz beschreibt den
gezielten Reexport durch ein privates Implementierungsmodul
([Visibility and privacy](https://doc.rust-lang.org/reference/visibility-and-privacy.html#re-exporting-and-visibility)).
Der offizielle Fehlerindex empfiehlt bei E0716 einen benannten lokalen Wert
mit Lebensdauer bis zum Blockende
([E0716](https://doc.rust-lang.org/error_codes/E0716.html)). `MutexGuard`
hält den Mutex bis zum Verlassen seines Scopes und stellt den geschützten
Wert über `Deref` bereit
([MutexGuard](https://doc.rust-lang.org/std/sync/struct.MutexGuard.html)).
Das Projekt verwendet Edition 2021; diese Reparaturen benötigen keine neue
Rust-, Tokio- oder Serde-API. Die genaue Rustversion des kommenden
Remotelaufs ist hier nicht bekannt.

Gewählt sind ein zusätzlicher `pub(crate)`-Funktionsreexport, ein lokal
besessenes `Opened`-Objekt und ein ausdrücklich begrenzter Authguardblock.
Eine Freigabe des ganzen IPCmoduls würde die Implementierungsfläche
unnötig erweitern. Eine lokale Rechteberechnung würde den laufenden
Authoritybesitz umgehen. Ein Authzustandsklon würde die erneute Prüfung
vom tatsächlichen Mutexbesitz lösen. Ein Entfernen des Widerrufsbranches,
Timeouts oder der Principalprüfung ist mit dem Auftrag unvereinbar.

| Punkt | Dateien und konkrete Änderung | Abhängigkeit und prüfbares Ergebnis |
| --- | --- | --- |
| D1 | `daemon/mod.rs`: bestehenden Funktionsreexport ergänzen; `device_rights_ui.rs`, `grants_device_view.rs`, `share_access.rs`: ausschließlich den Aufrufpfad auf die Fassade umstellen. | Vorhandene IPCfunktion und ihre Consumer sind gelesen. Alle drei Pfade erreichen dieselbe reale Requestfunktion; keine neue Berechnung, Startlogik oder Autorität. |
| D2 | `node_tcp.rs`: `TcpHelloReply::Opened` vor dem Sendefuture lokal binden. `node_apps_host.rs`: Guard innerhalb eines Blocks erwerben, `principal_grant` prüfen, nur das boolesche Ergebnis nach außen geben. `node.rs`, `node_accept.rs`: dieselbe `tcp_forward::TCP_ALPN` an allen drei Fehlerstellen verwenden. | D1 ist unabhängig. Widerruf, Fristen, Resourceguards, Ownerbindung und Dispatchgrenzen bleiben textlich bzw. strukturell unverändert. |
| D3 | `desktop/core/screen_target.rs`: optionalen Pin mit denselben Serdeattributen wie im Appziel ergänzen. `completions_access.rs`: alle drei fehlenden Capabilityfälle explizit benennen. | Kanonische Typen sind gelesen. Bisherige Desktop-JSONform bei `None` bleibt; sieben Enumfälle bleiben unterstützt. |
| D4 | `completions_access_tests.rs`: `constraints: None`. CLI-/Daemon-Still-, Windowlist- und Wirefixtures: fehlende optionale Felder ausdrücklich `None`. `access_grant_wire_tests.rs`: vollständige Arraylänge, drei zusätzliche `Off`-Erwartungen pro bisherigem Array, tatsächlich unbekannte Formatversion. | D3 liefert die vollständige Metadatengrenze. Alle bisherigen Fallnamen, Assertions, Namen, Geometrien, Escape-/Budgetfälle und Legacybytes bleiben erhalten. |
| D5 | Gesamter zugewiesener Diff und eigener Bericht: statische Syntax-/Textprüfung, vollständige Fallnamen-/Assertionserhaltung, Größenprüfung und eigene Anti-Regressionsreview der gesamten Karte. | Alle Produktänderungen sind fertig. Keine lokale Produkt- oder Testausführung. Main bestätigt die Korrektur ausschließlich durch denselben gemeinsamen Remote-Suiteeinstieg. |

Die zweite Lückenprüfung bestätigt für D1 bis D4 die vorhandenen
Funktionssignaturen, Serdeattribute, sieben Enumvarianten und die
Wireversionsgrenze. Neue Testnamen, Persistenzformate oder Protokollbytes
sind nicht erforderlich. Der bestehende Funktionsname erhält den nötigen
Fassadenreexport. Die angeforderten Autorisierungs-/Zielzulassungsdefinitionen
und Desktopanschlüsse sind für D5 inzwischen ausdrücklich lesbar und
geprüft; es bleibt keine konkrete Quellenlücke offen.

### Umsetzung und eigene anschließende Anti-Regressionsreview

D1 bis D4 sind umgesetzt. Der gesamte eigene Diff wurde anschließend
gegen `d95dc404` geprüft. Die unabhängige anschließende Autorprüfung
verfolgte jede Tabellenkette erneut mit vier Fragen: Bleiben die reale
Autorität und konkrete Identität erhalten? Bleiben Deadline, Widerruf,
Abbruch und Resourcebesitz erhalten? Bleiben Wire-/Persistenzbedeutungen
und sichtbare Ergebnisse erhalten? Bleiben sämtliche bisherigen
Fixturebelege bestehen? D5 ist damit als statische Autorarbeit erledigt;
die Remoteausführung bleibt bei Main.

| Prüfkette | Eigener Befund nach der Umsetzung |
| --- | --- |
| Geräte-IPC auf GUI, CLI und Android | Genau der bisherige typisierte Aufruf wird über `daemon::device_access_rights` reexportiert. Token, Adresse, 2-Sekunden-Verbindungsfrist, 20-Sekunden-Streamfrist, reale Daemon-Serviceabfrage und Fehlerpfade sind unverändert. Die GUI behält ihren Geräte-/Schlüsselsnapshot und ihre Thread-/Repaintzustände; die CLI behält ihre eindeutige Identitätsselektion und ihren Workerstart; Android behält Kontaktprüfung, bestätigte Geräte-/Schlüsselwerte, Quellenschlüssel und Fehlerübersetzung. |
| Authentifizierter TCP-Einstieg | Alle drei fehlerhaften Verweise verwenden die vorhandene Konstante aus `tcp_forward`, deren Bytes unverändert `smart-explorer/share-tcp/1` sind. Endpoint-ALPN, Strafenzuordnung und Dispatcher stimmen dadurch überein. Known-/Unknown-Admission, Permitabgabe und gemeinsame Route-/Handshakegrenzen bleiben unverändert. |
| TCP-Ziel und Antwortbesitz | `admit_tcp_target` friert hostaufgelöste Kandidaten ein. Jeder konkrete Ziel-IP-/Port-/Richtungsrequest prüft dieselbe bewiesene Beziehung, erhält seine eigene Bestätigung und Livelease, öffnet den gebundenen Socket und registriert den Tunnel unter erneuter Leaseprüfung. `serve_tcp` übernimmt dieselben vier Besitzer. Nur die `Opened`-Antwort erhält einen benannten lokalen Wert; der Sendefuture bleibt im unveränderten priorisierten Widerrufs-/20-Sekunden-`select!`. TTL, Cancel, Stream-ID, autorisierte Folgesockets, Kindtaskabschluss und Reihenfolge Socket → Registrierung → Access bleiben unverändert. |
| App-Receipt und Appstart | Die Receiptabfrage prüft nach der Fensterprobe unter dem tatsächlichen Mutexguard weiterhin `principal_grant`. Diese Funktion verlangt genaue Geräte-ID, öffentlichen Schlüssel, Fingerprint und Node-ID sowie eine aktive akzeptierte Directbeziehung mit passender Lookup-ID oder einen beigetretenen Raum mit zugelassenem Mitglied. Der Guard endet vor der Rückgabe und überquert keine Awaitgrenze. `bind_peer_request`, Ownerhash, Launchbestätigung, ursprünglicher Consent-/Accessbesitz, cancelgebundener Job, Deadline und Fensterwache sind unverändert. |
| Desktop-ScreenTarget | `expected_node` ist optional und hat dieselben Default-/Auslassungsattribute wie der Viewer. Der vorhandene Konstruktorwert `None` bleibt erhalten. `desktop_task_screen_target_json` verlangt unveränderte Direct-/Room-Bytes und Tileverhalten; `screen_task_viewer_target_parse` liest genau diese Desktopausgabe. In den freigegebenen Verbrauchern gibt es keinen zusätzlich anzupassenden vollständigen Structinitialisierer. Identitätskonflikte und unbekannte Geräte bleiben vor dem Viewerstart abgefangen. |
| App-/Fenstermetadaten | Die Produktfelder, IconRefprüfung, App-/Hashbindung und Budgets sind unverändert. CLI-`list`, Daemon-`listed`, Windowlist-`app`/`window`/`long_app`, Wire-`host_frames`/`app_entry` und die Escape-Fixture ergänzen ausschließlich die bislang fehlenden `None`-Felder. Dadurch fehlen Metadaten ausdrücklich; vorhandene Titel, Appnamen, PIDs, Monitore, Fokus-/Minimizedwerte, vollständige Texte, Limits und JSONschlüssel bleiben erhalten. |
| Sieben Capabilities und persistierte Grants | `decisions` verwendet `Capability::ALL` ohne Filter und leitet die Arraylänge daraus ab. Jede bisherige Viererwartung ist unverändert das Präfix der neuen Siebenerwartung; AppLaunch, LoginAccess und TcpForward sind bei alten Grants jeweils ausdrücklich `Off`. Alle alten Bytevergleiche, Konten, Programmrestriktionen, Revisionen, Ablaufwerte, alten Leser und Roundtrips bleiben erhalten. `capability_name` benennt alle sieben Varianten ohne Wildcard. |
| Unbekannte Formatversion | Das ursprüngliche Testobjekt verwendete eine inzwischen unterstützte Version und konnte den beabsichtigten Ablehnungsnachweis deshalb nicht mehr erbringen. Es verwendet nun `ACCESS_VERSION + 1`, derzeit 3. Alle bisherigen Aus-/Speicher-/Wiederaktivierungsassertions sowie unlesbare Version 1 und unbekannte Schlüssel/Felder bleiben erhalten. Die Testkorrektur ändert weder Parser noch Produktionsformat. |

Die statische Parserprüfung mit dem bereits installierten Tree-sitter-
Rustparser liest ausschließlich die zugewiesenen Quelldateien und deren
gleichen Pfade aus `d95dc404`. Sie meldet für alle bearbeiteten Rustdateien
eine vollständige Syntaxstruktur ohne Fehlerknoten. Ein zusätzlicher
Quellvergleich bestätigt identische Funktionsnamen und ausschließlich die
in D1 bis D4 benannten Reparaturen. Für sämtliche Fixturedateien ergibt das
Entfernen genau der hinzugefügten optionalen Felder wieder den ursprünglichen
Quelltext. Bei den Access-Wirefixtures sind ausschließlich Arraylänge,
die drei ergänzten `Off`-Werte und die tatsächlich unbekannte Version
verschieden; jede bisherige Assertion bleibt bestehen. Diese Prüfung
führt weder Rustcode noch Tests, einen Compiler oder einen Formatter aus.

Ein weiterer Produktfehler wurde bei dieser eigenen Review nicht belegt.
Laufzeitverhalten und vollständige Typprüfung sind dadurch nicht bestätigt.
Die ursprünglichen G7/G8/G9-Dateien und ihre Android-/P4-Navigationszustände
erhalten durch diese Fortsetzung keine Änderung.

### Geänderte Dateien und Größen

Es wurden keine Dateien neu angelegt. Geändert wurden der bestehende
Bericht und genau die folgenden zugewiesenen Rustdateien. Die Größen sind
der statische aktuelle Quellstand mit normaler Zeilenaufteilung; die
Remoteformatierung ist hier nicht ausgeführt worden.

| Datei unter `native/src/` | Zeilen | Bytes |
| --- | ---: | ---: |
| `app/core/device_rights_ui.rs` | 285 | 10891 |
| `cli/completions_access.rs` | 232 | 8773 |
| `cli/completions_access_tests.rs` | 225 | 8051 |
| `cli/screen/still_tests.rs` | 201 | 6779 |
| `cli/share/grants_device_view.rs` | 69 | 2590 |
| `daemon/mod.rs` | 408 | 14565 |
| `daemon/os/shared/screen_ipc_still_tests.rs` | 281 | 9257 |
| `desktop/core/screen_target.rs` | 54 | 2185 |
| `mobile/os/shared/domains/share_access.rs` | 487 | 20129 |
| `screen/core/window_list_tests.rs` | 127 | 4568 |
| `screen/core/wire_tests.rs` | 497 | 17863 |
| `share/core/access_grant_wire_tests.rs` | 345 | 12698 |
| `share/core/node.rs` | 471 | 18006 |
| `share/core/node_accept.rs` | 186 | 9220 |
| `share/core/node_apps_host.rs` | 414 | 17229 |
| `share/core/node_tcp.rs` | 349 | 13486 |

Alle geänderten Rustdateien liegen unter 500 Zeilen und unter 50 KiB.
`wire_tests.rs` erhält die fünf notwendigen Metadatenfeldzeilen und bleibt
bei 497 Zeilen; keine Zeilenverdichtung und keine Gateausnahme wurden
eingeführt. `app/mod.rs`, `app/screen_modules.rs`, das gemeinsame Manifest,
fremde Berichte und sämtliche nicht zugewiesenen Produktdateien wurden von
diesem Autor nicht geändert.

### Erhaltene Fälle und erforderliche Registrierung bei Main

Es gibt keine neuen Testfunktionen oder umbenannten Fälle. Für die direkte
Bestätigung der aktuellen Reparaturen sollen im bestehenden einen
Remoteeinstieg folgende vorhandene Fälle ausgewählt bleiben beziehungsweise
von Main ergänzt werden:

- `perm_task_completions_access`
- `screen_task_cli_windows_json_shape`
- `screen_task_ipc_still_client_lists_windows_and_ends_the_session`
- `screen_task_window_list_keeps_more_than_4096_entries`
- `screen_task_window_list_frame_bytes_bound_the_list`
- `screen_task_window_list_keeps_whole_texts_and_all_limit_notes`
- `screen_task_window_list_drops_oversized_entry_and_refuses_bad_ids`
- `screen_task_wire_roundtrip_all_tags`
- `screen_task_frame_limits_reject_oversize`
- `perm_task_port_full_shell_0_5_180`
- `perm_task_port_program_list_0_5_182`
- `perm_task_port_run_as_root_worker`
- `perm_task_port_disabled_keeps_settings`
- `perm_task_write_legacy_form_bytes_unchanged`
- `perm_task_old_reader_sees_new_grant_off`
- `perm_task_unknown_access_version_fail_closed`
- `desktop_task_screen_target_json`
- `screen_task_viewer_target_parse`

Die zwei genannten Wirefälle sind bereits in `native.integrations` des
gelesenen gemeinsamen Manifests registriert. Die übrigen konkreten Namen
wurden Main zur Ergänzung desselben Remoteeinstiegs gemeldet. Der bestehende
G7/G8/G9-Eintrag `R12-G7-G8-G9-Geraetenavigation`, seine drei nativen Fälle,
zwei JVM-Methoden und fünf Instrumentierungsmethoden bleiben erhalten.

Die folgenden ebenfalls in den bearbeiteten Fixturedateien vorhandenen
Fälle bleiben unverändert erhalten; ihre bloße Dateinachbarschaft ist kein
zusätzlicher Ausführungsauftrag:

- `completions_access_as_reads_the_worker_accounts`
- `screen_task_cli_snapshot_refuses_existing_file`
- `screen_task_cli_snapshot_output_shapes`
- `screen_task_cli_windows_and_snapshot_parse`
- `screen_task_ipc_still_client_names_refusals_by_code`
- `screen_task_ipc_still_client_gives_up_waiting_for_the_user`
- `screen_task_ipc_still_client_ends_a_silent_host_after_the_read_limit`
- `screen_task_ipc_still_client_assembles_the_snapshot_stream`
- `screen_task_ipc_still_client_reports_a_paused_source`

### Gelesene Dateien, Entscheidungen und verbleibende Grenzen

Neben allen Dateien der Änderungstabelle und diesem Bericht wurden für
diese Fortsetzung die vollständige zugewiesene Remote-Diagnosedatei und
die laufend ausdrücklich ergänzte Scope-Datei gelesen. Die wörtlichen
Nutzeranforderungen, Projektanweisungen und die vorhandenen Architektur-
und Oberflächenbefunde dieses eigenen Fixzyklus bleiben Grundlage.

Zusätzlich wurden folgende konkrete Quellen vollständig oder an den
genannten Definitionen, Aufrufern und Registrierungen gelesen:

- `native/Cargo.toml`: Edition und vorhandene Tokio-/Serdeabhängigkeiten.
- `native/review-task-manifest.json`: native Auswahl und bestehender R12-Eintrag.
- `native/src/app/core/screen_target.rs`: Viewerziel, optionaler Pin, Parser, Geräteprüfung.
- `native/src/app/core/screen_viewer_misc_tests.rs`: `screen_task_viewer_target_parse` und Gerätewechselprüfung.
- `native/src/app/desktop_modules.rs`, `native/src/app/mod.rs`, `native/src/app/screen_modules.rs`: vorhandene Modul-/Screenzielregistrierung.
- `native/src/daemon/os/shared/access_ipc.rs`: vorhandener Clientaufruf, reale DeviceRights-Abfrage, Token-/Frist-/Antwortgrenze.
- `native/src/desktop/core/mod.rs`, `native/src/desktop/mod.rs`: bestehende Modul-/Testregistrierungen.
- `native/src/desktop/core/protocol_tests.rs`: exakte Direct-/Room-JSONbytes und Tile-/Viewerzielvertrag.
- `native/src/desktop/core/tiles_tests.rs`: vorhandene Tileprojektion; keine ScreenTarget-Fixture.
- `native/src/desktop/os/shared/open_target.rs`: konkreter Geräte-/Prozesseinstieg und Identitätskonfliktablehnung.
- `native/src/screen/core/app_icon.rs`, `native/src/screen/core/window_list.rs`: vollständige Felddefinitionen, optionale Serialisierung und vorhandene Validierung.
- `native/src/share/access_modules.rs`, `native/src/share/mod.rs`: bestehende Modulgrenzen und kanonische TCP-Modulpfade.
- `native/src/share/core/access_authorize.rs`: vollständige konkrete Principal-, Capability- und Kontextrequestprüfung.
- `native/src/share/core/access_grant.rs`, `native/src/share/core/access_grant_wire.rs`: kanonische sieben Capabilities, Constraintsfeld und Legacy-/Versionsgrenze.
- `native/src/share/core/node_apps.rs`: Hostkonfiguration, vorhandene Host-Untermodulgrenze und vollständiger Ownerhash.
- `native/src/share/core/node_tcp_target.rs`: vollständige Zielauflösung, Bestätigung, Zulassung und reale Besitzer.
- `native/src/share/core/route_protocol.rs`: aktive Timeoutgrenze des bestehenden Sendefutures.
- `native/src/share/core/tcp_forward.rs`, `native/src/share/core/tcp_wire.rs`: kanonische ALPN, vorhandene Grenzen, Antworttyp und geliehener Sendefuture.
- `docs/ARCHITEKTUR.md`, `docs/oberflaechen/bildschirm.md`, `docs/oberflaechen/desktop-launcher.md`: gezielte Anschluss-/Begriffssuche auf Grundlage der früheren eigenen Lektüre.

Die Entscheidungen sind gezielter Funktionsreexport, echte gelockte
Autorisierung, besessene TCP-Antwort, optionaler kompatibler Desktop-Pin,
vollständige Metadateninitialisierer und sieben explizite Capabilities.
Es bleibt keine konkret fehlende Scopeabhängigkeit oder erforderliche
Produktänderung außerhalb dieser Schreibfläche offen. Die eigene
Auflistung, direkte/indirekte Karte, Planung, Umsetzung und anschließende
Anti-Regressionsreview wurden durch denselben Autor abgeschlossen.

Offen bleiben die von Main zu übernehmende Registrierung der konkret
genannten vorhandenen Fälle und deren Bestätigung über denselben
gemeinsamen Remote-Suiteeinstieg. Vollständige Typprüfung, Android-
Kompilierung, Remoteformatierung und reale Geräte-/Netzwerk-/Consent- und
Desktopausführung sind in dieser statischen Autorarbeit nicht geprüft.
Main besitzt weiterhin Integration, Graphrefresh, Commit/Push und den
terminalen gemeinsamen Release. Dieser Autor hat keine lokalen Builds,
Compiler, Formatter, Tests, Server, Installationen, Hostaufrufe,
Gitmutationen, Graphupdates oder Unteragenten ausgeführt und behauptet
keinen bestandenen Remotelauf.

## Fortsetzung: Diagnosen D01–D21 aus Lauf 40368746815

### Erstplan aus dem neuen Auftrag

Der neue begrenzte Auftrag gilt am Kandidaten
`b233e5336a0ee177e19382a7adf592d297dcf202`. Lauf 40368746815 ist laut
Auftrag vollständig beendet. Die aktuelle Scope-Datei ersetzt die vorige
Schreibfläche. Dieser Autor verantwortet D01–D03, D07–D12, D14–D16 und
D21 vollständig: Funktionsauflistung, direkte/indirekte Karte, vorherigen
Fixplan, Umsetzung und eigene anschließende Anti-Regressionsreview.
Der wörtliche Nutzervertrag ist erneut vollständig gelesen.

Zu erhalten sind Desktopordner-/Appaktionen, Kontenverwaltung,
Routendiagnose, die vorhandene Execdeadlinefixture und derselbe Windows-
Shellstart mit PIDL, Deadline, Prozesskorrelation und Cleanup. Änderungen
sind ausschließlich in den fünf zugewiesenen Rustdateien, dem bestehenden
Bericht und `Cargo.toml` für das erforderliche Windows-Feature erlaubt.
`Cargo.lock`, Paketversion, gemeinsame Suite und fremde Dateien
bleiben außerhalb dieser Schreibfläche. Main besitzt den praktischen
Nachweis im selben Remoteeinstieg sowie Graph, Git und Release.

Die vollständige zugewiesene Diagnoseliste wurde einschließlich aller
gerenderten Meldungen, Vorschläge und Plattformzuordnungen ausgewertet:

| Diagnosen | Belegter Fehler vor der Bestandsklärung |
| --- | --- |
| D01, D02 | `desktop_surface_actions.rs` benennt `folder_conflict_ui::Admission` über einen nicht vorhandenen Elternmodulpfad, sowohl im Queuefeld als auch beim Anlegen der Admission. |
| D03 | `desktop_surface_menu_ui.rs` erreicht den vorhandenen Apppicker wegen eines falschen Elternmodulpfads nicht. |
| D07 | `share_access_accounts_actions.rs` nennt `AccountChoices`, ohne den vorhandenen Typ im Scope zu haben. |
| D08–D12 | `share_routes_ui.rs` nennt `egui::Ui`, `RichText` und `CollapsingHeader`, ohne den im Projekt verwendeten egui-Zugang zu importieren. |
| D14–D16 | Der vorhandene Windows-Shellstart kann `SHELLEXECUTEINFOW` und `ShellExecuteExW` nicht sehen. Laut Auftrag weist die mitgelieferte Windows-0.58-Primärquelle auf das fehlende Feature `Win32_System_Registry`. Der vorgeschlagene Wechsel zu `ShellExecuteW` ist ohne Prüfung des bestehenden PIDL-/Prozessvertrags nicht zulässig. |
| D21 | Die vorhandene Execdeadlinefixture nennt das vorhandene `io_deadline`-Modul ohne gültigen Import. |

Der Export nennt keine Android-Diagnose. Das ist kein Beleg einer
bestandenen Androidprüfung. Vorläufige Planungstiefe ist 2: bestehende
gekoppelte GUI-/Daemon-/Plattformverträge müssen erhalten werden.
Die genaue Modulhierarchie, Typbesitz, konkrete Featurebedingung und
betroffenen Fixtureaussagen sind noch durch die zugewiesene Lektüre zu
klären. Eine nur vom Nutzer zu liefernde Angabe ist nicht erkennbar.
Nächster Schritt sind diese vorhandenen Definitionen und tatsächlichen
Aufrufer; danach folgt der konkrete regressionsschützende Plan vor jeder
Produktänderung. Lokale Ausführung und Unteragenten bleiben verboten.

### Zweitplan: Bestand, Fehlerfunktionen und vollständige Anschlusskarte

Die belegte Planungstiefe bleibt 2. `app/mod.rs` deklariert
`desktop_modules` als eigenes Untermodul und reexportiert dessen
Desktopmodule. `folder_conflict_ui` und `share_apps_ui` gehören unmittelbar
zu `app`; `super` eines Desktop-Untermoduls bezeichnet dagegen
`desktop_modules`. Die gesuchten Funktionen existieren bereits mit
passender Sichtbarkeit. Eine neue API oder Modulsichtbarkeit ist nicht
erforderlich.

Die Definition von `AccountChoices` ist der gemeinsame typisierte Vertrag
in `share/core/exec_helper_protocol.rs`. Der Kontenrenderer importiert ihn
bereits; sein extrahierter Actionsbaustein importiert bislang nur `page`,
`AccountsPage` und `PageAction`. `share_routes_ui` benötigt denselben
`eframe::egui`-Zugang wie die vorhandenen GUIbausteine. `io_deadline` ist
bereits unter `share` registriert und bietet den exakt von der Fixture
verlangten absoluten `run_until`-Aufruf.

| Fehlerhafte Funktion oder betroffene Grenze | Direkte und indirekte bestehende Kette; Erhaltungsanforderung |
| --- | --- |
| D01/D02: `SystemActions::folders`, `SurfaceActions::run` | `desktop_surface_menu_ui::paste` oder vorhandener Transferauftrag → konkreter Benutzerdesktop aus `ViewModel::folder(false)` und Zwischenablagepfade → `DesktopOp::Transfer` mit Copy-/Movewert → `desktop_ops::transfer_job` → validierter lokaler, wiederherstellbarer Treejob → `folder_conflict_ui::Admission::new` → eigene FIFO des Flächenprozesses. Die Typreferenz und der Konstruktor müssen dieselbe existierende Admission meinen. |
| D01/D02: `take_notices`, `run_transfer_job` | Vorderste Admission → `Admission::show`/FolderPreviewScan → Auswahl KeepBoth, Merge oder Skip pro Quellordner → ausschließlich `Ok(Some(job))` startet den bestehenden Transferthread → `desktop_ops::run_transfer_job` → gemeinsamer `transfer::run_view` → Fortschritt/Issues → Noticekanal und Repaint. Abbruch und vollständiges Überspringen liefern `Ok(None)`; Fehler werden angezeigt. Die eigene Cutzwischenablage wird erst nach erfolgreichem Threadstart und passendem Roots-/Moveauftrag verbraucht. |
| Unveränderte Desktopaktionen neben D01/D02 | `activate`, Datei-/Flächenmenü und `SurfaceActions::open/run` → unveränderte OpenRequest-/DesktopOpvarianten. Umbenennen prüft Einzelname und nutzt lokales NoReplace; neuer Ordner nutzt exklusiven freien Namen; Papierkorbfehler bleiben Fehler und führen zu keiner dauerhaften Löschung. Auswahl, Monitorlayout, Heartbeat, native Dateiablage, Copy und Cut bleiben unverändert. |
| D03: `desktop_surface_menu_ui::device_menu` | Konkretes DeviceTile aus dem ViewModel, einschließlich Raumgerät → „Programme auf diesem Gerät …“ → `app::share_apps_ui::open_surface` mit demselben egui-Kontext, `PeerOpenTarget` und Gerätenamen → kontextgebundene Liste offener Picker, nach exaktem Ziel dedupliziert → `surface_windows` → gerätebezogenes `panel`. Die Änderung darf keinen Appstart und keinen Ersatzprozess auslösen. |
| Indirekter D03-Appauftrag | `panel` → gespeicherten Request der konkreten Beziehung wiederherstellen → authentifizierten Katalog abfragen → neuen vollständigen Request aus Katalog-, App-, Benutzer-, Sitzungs- und Request-ID bilden → privat und synchron vor dem Versenden speichern → genau ein `AppsCommand::Launch` → danach ausschließlich derselbe Receipt bei Antwortverlust/Refresh. `settle` prüft Geräte-ID und vollständigen Request. Der bestätigte Node-/Window-/Generationbeleg geht über den vorhandenen installierten P4-Öffner; ein gespeicherter Request allein ist kein Fensterbeleg. |
| D07: `share_access_accounts_actions::read` | „Dieses Gerät“ → vorhandenes `share_exec_ui::ui_host_accounts` → AccountsPage im selben egui-Kontext → Hintergrundabfrage `daemon::access_accounts` → laufender Daemon mit Token/AccountList → `exec_account_choices` des tatsächlichen Workers → kanonischer `AccountChoices { helper, choices }` → dieselbe Page und Repaint. Ohne erreichbaren Worker bleibt der bisherige lokale Fallback erhalten. |
| Indirekter D07-Administrationspfad | `ui_accounts` → vorhandener Renderer `page` → Refresh, CopyCommand oder ausdrückliche Applyaktion → `run` mit Busyguard → `exec_accounts_apply` und `PkexecGraphical` → Done oder NeedsAdministrator → bestehende Anzeige/CopyCommand → erneutes Einlesen. Setup, Allow, Disallow, Update und Remove behalten Bestätigung, konkrete Kontowerte, Fehler und Refresh. Ein Typimport darf keine Kontorechte erweitern. |
| D08–D12: `share_routes_ui::device`, `show_peer` | Bereits gepollter `ShareRouteSnapshot` des Aufrufers → Anzeige von Snapshotfehler/IPCkürzung → Auswahl ausschließlich nach Geräte-ID und öffentlichem Schlüssel → Verbindungen mit Zustand, Grund und Remotegrenze → eigene Detailheader nach Peer und Connection-ID → rückwärts dargestellter Paarverlauf mit bestehendem Messhinweis. `service_routes` projiziert dieselbe gepinnte Identität aus Monitor-/Historienfakten; der Renderer liest nur und startet keine Messung oder Verbindung. |
| D14–D16: Windows-`Platform::dispatch` | Installierter AppsFolder-Katalog mit Shell-Parsingname und Fingerprint → HostPlatformtarget und unveränderter LaunchRequest → COM-STA → aktuelles Shellitem mit erneutem Fingerprint → tatsächliche interaktive Benutzer-/Logonsitzung → Unlockprüfung → Shell-PIDL → unmittelbare Grantdeadlineprüfung → `SHELLEXECUTEINFOW` mit festem Verb, IDLIST/NOCLOSEPROCESS/NOASYNC/NO_UI → derselbe `ShellExecuteExW` → PIDLfreigabe und ggf. Prozesskorrelation → Handleabschluss → bestehender Ergebnisstatus. |
| Indirekte Windows-Prozess-/P4-Kette | Nur bei erfolgreichem Shellaufruf und gültigem hProcess → `window_correlation::record` → tatsächlicher Nutzer-/Logonabgleich, Prozess-ID, Sitzung und Erstellzeit → dupliziertes Handle mit Dropabschluss → Request-/Dispatchgebundener gemeinsamer Handoff → `Correlations::take_started/confirm` → genaues noch lebendes Prozessfenster, Benutzerprüfung und Originalpintransfer. Fehlendes hProcess erzeugt keinen erfundenen Fensterbeleg; Ausgang bleibt zunächst `StartedWindowPending`. |
| D21: `handshake_io_enforces_one_absolute_deadline_for_a_stalled_peer` | Bestehende Execserver-Fixture → bereits vorhandenes `share::io_deadline::run_until` → `tokio::time::timeout_at` auf derselben absoluten Instant → unveränderter Timeouttyp und unveränderter Meldungstext. Produktions-Exec verwendet seine bestehende aktive Routen-/Heartbeatzeit und unveränderte Grantdeadline; die Fixture wird nicht auf einen anderen Uhrvertrag umgebogen. |

Die zugeschnittene Graphabfrage fand die zugehörigen Typ- und Dateiknoten.
Die anschließenden konkreten Pfadabfragen zeigen die Verbindung
`desktop_surface_actions → desktop_ops → run_transfer_job → TransferJob →
requires_preview → folder_conflict_ui` sowie `share_access_accounts_actions →
lock → AccountsPage → AccountChoices`. Die AST-Referenzkanten ersetzen
keine Modulauflösung; die fehlerhaften Elternpfade wurden deshalb gegen die
tatsächlichen Deklarationen gelesen. Der aktuelle Rootgraph wurde nur
gelesen. Das Werkzeug meldet Skill 0.9.11 gegenüber Paket 0.9.17; alle drei
Leseabfragen waren erfolgreich. Eine Installation ist hier nicht erlaubt.

### Zusätzlicher eigener Fixturebefund

In derselben Kontenkette ist ein weiterer konkreter Fehler belegt:
`perm_task_gui_accounts_page_states` erwartet bei installiertem Dienst
„Kontodienst eingerichtet (Version 0.5.183).“. `service_block` zeigt bereits
die Statuszeile „Kontodienst eingerichtet.“ und die Version im vorhandenen
aufklappbaren Abschnitt „Diagnose des Kontodiensts“. Die alte Assertion
verfehlt die aktuelle Darstellung. Main hat die genaue Fixture zur
Bearbeitung und ihren bestehenden Harness zur Lektüre freigegeben.

Gewählte Erhaltung: zuerst die sichtbare Statuszeile prüfen, danach den
vorhandenen Diagnoseheader mit dem bestehenden echten egui-Klickharness
öffnen und die Version prüfen. Ein Klick auf die Diagnose muss weiterhin
keine Administrationsaktion erzeugen. Sämtliche bisherigen Setup-/Update-/
Entzugs-/Remove-/Sitzungswarnungs- und Fallbackbelege bleiben erhalten.
Das Produktlayout und die Versionsangabe werden nicht verändert.

### Entscheidungsrecherche und endgültiger Fixplan

Die lokale mitgelieferte Windows-0.58-Primärquelle enthält unmittelbar vor
`ShellExecuteExW` die Bedingung `#[cfg(feature = "Win32_System_Registry")]`
(Zeilen 4588–4592). Beide architekturabhängigen `SHELLEXECUTEINFOW`-
Definitionen und deren Defaultimplementierungen tragen dieselbe Bedingung
(ab Zeilen 36140 und 36193). Der vorhandene Highlevelvertrag in
`Cargo.toml` ist `windows = "0.58"`, `Cargo.lock` enthält dazu Windows
0.58.0. Nur `windows-sys` aktiviert Registry bereits; dessen getrennte
Features schalten die Highlevel-`windows`-API nicht frei.

Am 2026-10-26 gelesene Microsoft-Primärdokumentation bestätigt den
PIDL-/Prozesshandlevertrag: `SEE_MASK_IDLIST` bindet die Shell-ID-Liste,
`SEE_MASK_NOCLOSEPROCESS` ermöglicht das zurückgegebene Prozesshandle, und
der Aufrufer muss dieses schließen. Nicht jeder erfolgreiche Shellstart
liefert ein Prozesshandle
([SHELLEXECUTEINFOW](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ns-shellapi-shellexecuteinfow)).
Der Shellaufruf verwendet die In-/Outstruktur und benötigt den passenden
COMkontext
([ShellExecuteExW](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shellexecuteexw)).
Die versuchten docs.rs-Aufrufe für Windows 0.58 waren über das Webwerkzeug
nicht erreichbar; die exakte Featurebedingung wurde aus der vollständigen
lokal mitgelieferten Version gelesen.

Entschieden sind kanonische gezielte Imports und genau ein Featureeintrag
im bestehenden Windows-Targetblock. Ein neuer egui-Crateeintrag würde die
bereits verwendete eframe-API unnötig duplizieren. Neue Moduldeklarationen
oder breite Sichtbarkeitsänderungen sind nicht nötig. Der vorgeschlagene
`ShellExecuteW`-Ersatz bietet den benötigten PIDL-/hProcess-In-/Outvertrag
nicht und würde die bestehende Korrelation verlieren. Der bestehende
absolute Deadlinehelper und die bisherigen Fixtureaussagen werden erhalten.

| Planpunkt | Konkrete Änderung und Abhängigkeit | Prüfbares erwartetes Ergebnis |
| --- | --- | --- |
| E1 | `desktop_surface_actions.rs`: `crate::app::folder_conflict_ui` importieren und beide falschen Elternreferenzen auf den importierten Modulnamen beziehen. `desktop_surface_menu_ui.rs`: `crate::app::share_apps_ui` importieren und bestehenden Pickeraufruf darauf beziehen. Tatsächliche Modulregister, Admission und Picker sind gelesen. | Bestehende Transferqueue und derselbe Apppicker werden erreichbar; kein veränderter Funktionskörper über die Namensauflösung hinaus. |
| E2 | `share_access_accounts_actions.rs`: kanonischen `exec_helper_protocol::AccountChoices` importieren. `share_routes_ui.rs`: bestehenden `eframe::egui`-Zugang importieren. Definitionen und vollständige Consumerkörper sind gelesen. | Bestehender Worker-/Fallbackaccountwert und alle fünf Administrationsaktionen bleiben erhalten; Routendiagnose behält konkrete Identitätsauswahl, Text, Zustand und Detail-IDs. |
| E3 | `Cargo.toml`: ausschließlich `Win32_System_Registry` im vorhandenen `windows`-0.58-Featureblock ergänzen, mit kurzem konkretem Kommentar zum Shellstart. Primärquelle und unveränderter kompletter Windowsadapter sind gelesen. | Bestehende Shell-Ex-Typen/Funktion werden unter derselben Bibliotheksversion verfügbar; PIDL, Frist, Sitzungsbindung, Prozesskorrelation und Cleanup bleiben exakt erhalten. Kein Lock-/Versionswechsel. |
| E4 | `exec_server_tests.rs`: vorhandenes `crate::share::io_deadline` importieren. Zusätzlich freigegebene `share_access_accounts_ui_tests.rs`: vorhandene Status-/Versionsassertion auf Statuszeile plus Diagnoseklick/-version abbilden. Helper und Harness sind gelesen. | Derselbe absolute Deadlinefall bleibt unverändert. Der vorhandene Kontenfall belegt Status und zugängliche Version sowie alle bisherigen Aktionen; keine Fallumbenennung oder ersatzlose Assertionentfernung. |
| E5 | Gesamten eigenen Diff anschließend gegen b233e533 lesen, kanonische Module und zusätzliche Fixture erneut prüfen, statische Rustsyntax-/TOML-/Body-/Größenkontrolle und eigene Anti-Regressionsreview dokumentieren. | Ausschließlich E1–E4 unterscheiden sich; Produktkörper und alle verbleibenden Assertions sind erhalten, native Dateien unter 500 Zeilen/50 KiB. Die tatsächliche Bestätigung erfolgt durch Main im selben Remoteeinstieg. |

Die zweite Lückenprüfung hat kanonischen Typ, Modulsichtbarkeit,
vollständige Windowsfeaturebedingung, beide Deadlineverträge und den
tatsächlichen Klickharness geklärt. Alle benötigten zusätzlichen Lesepfade
und die eine zusätzliche Fixture-Schreibdatei sind ausdrücklich gewährt.
Vor der Umsetzung bleibt keine entscheidungsrelevante Scopefrage offen.

### Umsetzung und eigene anschließende Anti-Regressionsreview

E1 bis E4 sind umgesetzt. E5 wurde danach durch denselben Autor als
erneute Prüfung der gesamten oben ermittelten Anschlusskarte ausgeführt.
Die vollständigen eigenen Quelländerungen wurden gegen den Kandidaten
b233e533 gelesen. Die zusätzlichen integrierten Korrekturen von Main
berühren die exklusive Schreibfläche dieses Autors nicht.

| Prüffrage | Befund nach der Umsetzung |
| --- | --- |
| Erreichen Einfügen und Transfer dieselbe Admission? | Das Queuefeld und `Admission::new` beziehen beide das vorhandene `crate::app::folder_conflict_ui`. Der Kontrakt ist unverändert: Preview vor Jobausführung, FIFO im Flächenprozess, explizite Ordnerwahl, Cancel/Skip ohne Start und sichtbare Fehler. Die komplette Ausführungsfolge, Copy-/Moveentscheidung und Cutverbrauchsbedingung unterscheiden sich ausschließlich in der korrigierten Namensauflösung. |
| Bleiben bestehende Datei-/Geräte-/Raum-/Desktopaktionen bestehen? | Alle Funktionsnamen, Menütexte, aktiven/gesperrten Aktionen, ausgewählten Elemente, konkreten Zielparameter, Monitor-/Heartbeataufrufe und DesktopOp-/OpenRequestvarianten sind erhalten. NoReplace, Namensprüfung, Wiederherstellbarkeit und Papierkorbfehlerpfad des verbundenen Backends sind unverändert. |
| Öffnet die Programmhandlung denselben gerätegebundenen Picker? | `device_menu` ruft den vorhandenen `app::share_apps_ui::open_surface` mit exakt demselben Kontext, Ziel und Namen auf. Direct und Room behalten ihre konkrete Beziehung; Deduplizierung, gespeicherter Auftrag, einmaliger Launch, Receipt-only-Retry, Node-/Request-/Window-/Generationprüfung und bestehender P4-Öffner bleiben unverändert. Der Pfadwechsel fügt keinen Hoststart und keinen neuen Prozess hinzu. |
| Bleiben Kontenquelle und Administratorrechte erhalten? | Der importierte `AccountChoices` ist genau der bereits vom Renderer und Daemon verwendete Typ. Workerabfrage, Token, Fristen und lokaler Fallback bleiben erhalten. Die fünf Aktionen Setup, Allow, Disallow, Update und Remove behalten Busyguard, ausdrückliche Benutzeraktion, `PkexecGraphical`, Ergebnis-/Fehleranzeige und erneutes Lesen. Keine Kontoberechtigung oder Sichtbarkeit wird erweitert. |
| Belegt die angepasste Kontenfixture weiterhin Zustand und Version? | Der existierende Fall prüft die tatsächliche Statuszeile, öffnet die sichtbare „Diagnose des Kontodiensts“, erwartet dabei keine PageAction und prüft danach dieselbe Fixtureversion 0.5.183. Die übrigen Assertions für Verfügbarkeit, Sitzungshinweis, Setup, Update, Entzug, Remove und abgelehnte Anmeldung sind unverändert. Die separate Administrator-Fallback-/Kopierfixture bleibt unverändert. |
| Bleibt die Routendiagnose eine konkrete lesende Ansicht? | Ein Import liefert dieselben vorhandenen egui-Typen. Der vollständige Renderer bleibt bytegleich: Selektion nach Geräte-ID und öffentlichem Schlüssel, Fehler-/Truncatedanzeige, alle Verbindungen, Remotegrenze ohne Doppelanzeige, bestehende Detail-IDs und unveränderter Messverlauf. Keine Verbindungs- oder Messaktion wurde hinzugefügt. |
| Bleibt der Windows-Shellstart derselbe? | Nur die benötigte Featurefreigabe im bestehenden Windows-0.58-Block ist ergänzt. Der komplette Shelladapter ist bytegleich, einschließlich Shellitem-/Fingerprintprüfung, tatsächlicher interaktiver Sitzung, Unlockbedingung, PIDL, unmittelbarer Grantdeadline, festem Verb, Masken, `ShellExecuteExW`, PIDLfreigabe und Handleabschluss. Katalog, Sessionadapter und Prozesskorrelation sind ebenfalls unverändert. |
| Bleiben tatsächliche Prozessbesitzer und Fensterbelege erhalten? | Die Erfolgskorrelation erfolgt weiterhin vor dem Schließen des Originalhandles. Das duplizierte Handle behält seinen eigenen Dropabschluss; Sitzungs-/Nutzer-/Erstellzeitbindung, lebender Prozessbeleg, konkreter Request/Dispatch und P4-Originalpin bleiben erhalten. Fehlender Prozess- oder Fensterbeleg wird nicht durch eine PID-/Namenssuche ersetzt. |
| Bleibt die absolute Execdeadlinefixture erhalten? | Ausschließlich der vorhandene Helperimport ist ergänzt. Der gesamte Fallkörper einschließlich 20-ms-Instant, pending-Future, Timeoutkind und exaktem Text „stalled handshake“ ist bytegleich. Produktive aktive Routenzeit, Heartbeatbudget, Cancel-/Acklogik und Grantdeadline sind unverändert. |
| Bleiben Bibliotheks-/Speicherverträge erhalten? | Der TOMLvergleich zeigt ausschließlich ein zusätzliches Feature bei `target.cfg(windows).dependencies.windows`. Crateversion, Paketversion, alle bisherigen Features und sämtliche übrigen Manifestwerte sind gleich. `Cargo.lock` ist bytegleich. Request-/Receiptmodelle, private Requestdateien und Protokollformen ändern sich nicht. |

Die statische Syntaxprüfung verwendet den bereits installierten
Tree-sitter-Rustparser; für alle bearbeiteten Rustdateien gibt es keine
Fehlerknoten, und sämtliche Funktionsnamen bleiben erhalten. Ein
vollständiger Quellvergleich ergibt nach Rücknahme genau der geplanten
Import-/Pfadänderungen wieder b233e533. Für die Kontenfixture ist
ausschließlich der ausdrücklich geplante Ersatz der einen alten
Inlineassertion durch Status-, Diagnoseklick- und Versionsnachweis
verschieden; jede übrige Zeile bleibt erhalten. Die Execfixture unterscheidet
sich ausschließlich im Import.

Der statische TOMLparser bestätigt das zusätzliche Feature im richtigen
Target und derselben Highlevelbibliothek; sämtliche verbleibenden
Manifestwerte sind identisch. Die gelesenen verbundenen Quellen für
Admission, Picker, Requestjobs/-speicher, Kontenrenderer und Harness,
Routenprojektion, Execserver/Deadlinehelper sowie Windowsplattform,
Katalog, Sitzung und beide Korrelationsstufen sind bytegleich zu b233e533.
`git diff --check` hat im eigenen Diff keinen Befund geliefert.
Keiner dieser Nachweise führt Rustcode, Produktaktionen oder Tests aus.

Die eigene abschließende Review belegt keinen weiteren Fehler in der
ermittelten Kette. Syntaxstruktur und Quellidentität beweisen weder
vollständige Typprüfung noch reale Shell-/Rechte-/Geräte-/UIausführung.
Die erneute praktische Bestätigung gehört weiterhin Main.

### Dateiübersicht und Größen

Es wurden keine Dateien neu angelegt. Geändert wurden ausschließlich
der bestehende Bericht, der bestehende Windowsfeatureblock in
`native/Cargo.toml` und die folgenden explizit freigegebenen Rustdateien:

| Datei unter `native/src/` | Zeilen | Bytes |
| --- | ---: | ---: |
| `app/core/desktop_surface_actions.rs` | 298 | 10272 |
| `app/core/desktop_surface_menu_ui.rs` | 425 | 14343 |
| `app/core/share_access_accounts_actions.rs` | 111 | 4058 |
| `app/core/share_access_accounts_ui_tests.rs` | 201 | 7392 |
| `app/core/share_routes_ui.rs` | 75 | 2713 |
| `share/core/exec_server_tests.rs` | 242 | 7513 |

Alle bearbeiteten Rustmodule bleiben unter 500 Zeilen und unter 50 KiB
mit üblicher Zeilenaufteilung. Es gibt keine Verdichtung oder Gateausnahme.
Die ergänzte Schreibfreigabe betrifft ausschließlich die bereits vorhandene
Kontenfixture; kein zusätzlicher Produkt-Schreibpfad wurde benötigt.

### Vorhandene Fälle und Registrierung durch Main

Die konkreten bestehenden Fälle für den gleichen Remoteeinstieg wurden
Main gemeldet:

- `handshake_io_enforces_one_absolute_deadline_for_a_stalled_peer`
- `perm_task_gui_accounts_page_states`
- `perm_task_gui_setup_without_pkexec_shows_sudo_command`
- `desktop_task_desktop_ops_rename_trash_copy`

Sie bestätigen die zugewiesene Deadlinefixture, die Kontenansicht samt
Status-/Versions-/Administrationsbelegen, den bestehenden fehlenden
Administratorfrontend-Fallback und die verbundenen Desktopdateiaktionen.
Es gibt keine neuen Testnamen oder umbenannten Fälle. Die übrigen
Execserverfälle wurden weder verändert noch als zusätzliche manuelle
Testkampagne angefordert.

Der vorhandene R12-Anschluss bleibt im gelesenen Manifest registriert:

- `batches_task_device_navigation_surface_draws_picker_and_p4_errors`
- `batches_task_device_navigation_surface_keeps_direct_room_and_output_contexts_separate`
- `batches_task_device_navigation_confirmed_receipt_keeps_identity_and_rejects_invalid_bindings`

Der Windowsfeaturefix benötigt außerdem die ohnehin vorhandene
Windowskompilierung desselben Remoteeinstiegs. Ein bestandener Live-
Shellstart, tatsächlicher Kontenwechsel oder beobachtetes Fernfenster wird
daraus nicht abgeleitet. Für die reine Importkorrektur der Routendiagnose
wurde kein neuer künstlicher Test geschrieben; unveränderter Renderer
und die bestehende Remote-Typprüfung bilden hier die konkrete Grenze.

### Vollständige Leseübersicht dieser Fortsetzung

Gelesen wurden sämtliche geänderten Dateien der obigen Übersicht,
`native/Cargo.toml` und der vorhandene Bericht. Zusätzlich wurden folgende
konkrete Pfade vollständig oder an den ausdrücklich betroffenen
Definitionen, Aufrufern und Registrierungen gelesen:

- `/tmp/smartexplorer-final-review-user-contract.txt`, die aktuelle Scope-Datei und `/tmp/smartexplorer-rv1-40368746815/device-navigation-errors.json`.
- `/tmp/smartexplorer-rv1-40368746815/primary/windows-0.58-shell.rs`: exakte versionsgebundene cfg-/Typ-/Funktionsdefinitionen.
- `native/Cargo.lock`: gebundene Windowsversion und unveränderte Datei.
- `native/review-task-manifest.json`: bestehender R12-Anschluss und konkreter Registrierungsbedarf.
- `native/src/app/mod.rs`, `native/src/app/desktop_modules.rs`, `native/src/app/core/prelude.rs`: tatsächliche App-/Desktopmodulgrenzen und vorhandener egui-Zugang.
- `native/src/app/core/desktop_surface_view_model.rs`: konkrete Tile-/Dateiauswahl, Benutzerdesktop, Layout, Heartbeat und Nachrichten.
- `native/src/app/core/folder_conflict_ui.rs`: vollständige Admission, Vorschau, Entscheidungen und Abbruch-/Freigabevertrag.
- `native/src/app/core/share_apps_ui.rs`, `native/src/app/core/share_apps_jobs.rs`, `native/src/app/core/share_apps_store.rs`: vollständiger gerätegebundener Picker, Requestjobs, Receipt-/P4-Bindung und private Wiederherstellung.
- `native/src/app/core/share_exec_ui.rs`, `native/src/app/core/share_access_accounts_ui.rs`: vorhandener Konteneinstieg, vollständiger Renderer, Accounttyp und fünf Administrationsaktionen.
- `native/src/app/core/share_access_ui_tests.rs`: vollständiger bereits vorhandener Klick-/Captureharness.
- `native/src/daemon/os/shared/access_ipc.rs`: existierender AccountList-Client/-Server, Token und Fristen.
- `native/src/desktop/os/mod.rs`, `native/src/desktop/os/shared/desktop_ops.rs`, `native/src/desktop/os/shared/desktop_ops_tests.rs`: kanonische Registrierung, vollständiger lokaler Job-/Dateiaktionspfad und bestehende Fixture.
- `native/src/share/mod.rs`, `native/src/share/core/exec_helper_protocol.rs`, `native/src/share/core/service_routes.rs`: tatsächliche Modul-/Typexporte, kanonischer Accounttyp und vollständige Routenprojektion/-darstellungstexte.
- `native/src/share/core/exec_server.rs`, `native/src/share/core/io_deadline.rs`, `native/src/share/core/route_protocol.rs`: Fixtureeinbindung, absoluter Helper und bestehende aktive Produktionsdeadline.
- `native/src/apps/mod.rs`, `native/src/apps/core/model.rs`, `native/src/apps/os/shared/platform.rs`: vorhandene Exporte, Requests, Session-/Windowfelder, Ergebnistypen und HostPlatformvertrag.
- `native/src/apps/os/windows/mod.rs`, `native/src/apps/os/windows/catalog.rs`, `native/src/apps/os/windows/launch.rs`, `native/src/apps/os/windows/session.rs`, `native/src/apps/os/windows/window_correlation.rs`: vollständige betroffene Windowsadapter und tatsächlicher Start-/Besitzweg.
- `native/src/apps/os/shared/window_correlation.rs`: vollständiger Request-/Dispatchhandoff, Lebendigkeit und Originalfenster-/Pinvertrag.
- `docs/ARCHITEKTUR.md`, `docs/oberflaechen/desktop-launcher.md`, `docs/oberflaechen/fernzugriff.md`: betroffene aktuelle Einstiege und Oberflächenverträge.
- `graphify-out/graph.json`: ausdrücklich freigegebene gezielte Leseabfrage und die beiden beschriebenen Pfadabfragen.
- `/root/.codex/skills/graphify/SKILL.md` und dessen `references/query.md`: vorhandene Leseabfrage, keine Extraktion oder Installation.

Die angewandten zentralen Arbeitsanweisungen dieses laufenden Auftrags
wurden wiederverwendet. Die zusätzlichen Pfade wurden jeweils vor der
Lektüre konkret bei Main angefordert und explizit freigegeben; eine
allgemeine Repositoryerkundung wurde nicht ausgeführt.

### Entscheidungen, Abweichung und verbleibende Grenzen

Die vollständige eigene Funktionsauflistung, Anschlusskarte, vorherige
Planung, Umsetzung und anschließende Anti-Regressionsreview sind erledigt.
Der einzige zusätzliche Funktionsbefund ist die veraltete Kontenfixture;
sie wurde nach konkreter Scopefreigabe mit erhaltenem Zustands- und
Versionsnachweis korrigiert. Alle zugewiesenen D-Diagnosen haben eine
konkrete Quellkorrektur. Es bleibt keine erforderliche Produktänderung
oder konkrete Scopeabhängigkeit offen.

Main besitzt die Registrierung der gemeldeten vorhandenen Fälle und die
praktische Bestätigung im gleichen gemeinsamen Remoteeinstieg sowie
Integration, Rootgraphrefresh, Commit/Push und terminalen Release.
Dieser Autor hat keine Unteragenten, lokalen Builds, Compiler, Formatter,
Tests, Produktaktionen, Server, Installationen, Hostaufrufe, Lockauflösung,
Gitmutationen, Graphupdates oder Releases ausgeführt. Die gezielten
Graphabfragen und statischen Syntax-/TOML-/Quellvergleiche sind keine
Produkt- oder Testausführung. Reale Windows-Shell-/Konten-/Geräte- und
Desktopausführung bleibt hier ungeprüft; ein erfolgreicher Remotelauf
wird nicht behauptet.

## Fortsetzung: Android-Iconmodelle K01 und K03–K09

### Erstplan aus dem Auftrag

Dieser begrenzte Fixzyklus beginnt am Kandidaten
`0425aa4608d7e27eb95ee0a37e3bc13dca2ce8bf` nach dem laut Main vollständig
beendeten Lauf 40381452096. Der erneute wörtliche Nutzervertrag und die
aktuelle Scope-Datei wurden vollständig gelesen. Die aktuelle Phase
ersetzt die vorige D/J-Schreibfläche vollständig.

Ergebnisauftrag ist der korrekte Konsum der gemeinsamen Android-
Iconmodelle in Hostvoraussetzungen, Familienmenü und Fensterauswahl.
Reale Programmsymbole und ein zugänglicher neutraler Ersatz bleiben
erhalten. Drahtnamen, Peer-/Appidentität, Rechte, Hash-/Größenprüfung,
Abbruch und die vorhandenen Bedienwege dürfen sich nicht ändern.
Schreibbar sind ausschließlich `ScreenApi.kt`, `FamilyScreen.kt`,
`ScreenIcons.kt`, `ic_apps.xml` und dieser vorhandene Bericht.

Die vollständige Kotlin-Diagnosedatei nennt diese eigenen Fehler:

| Diagnose | Fehler vor der Bestandsklärung |
| --- | --- |
| K01 | Der Hostvoraussetzungsaufruf liest `png_base64` von einem `ScreenIconReply`, der dieses Kotlinfeld nicht besitzt. |
| K03–K06 | Das Familienmenü liest `app_id` von `ScreenIconRef`; die fehlerhafte Eigenschaft erzeugt Folgefehler der Typinferenz. |
| K07 | Der Iconloader des Familienmenüs sieht trotz gleicher Kurztypenamen inkompatible Request-/Replytypen. Die konkreten Typdefinitionen und Imports sind vor dem Fix zu vergleichen. |
| K08, K09 | Der neutrale Ersatz in `ScreenIcons.kt` verwendet das nicht verfügbare `Icons.Outlined.Apps`. Die vorhandene Iconinfrastruktur und die bereitgestellte Material-Vektorquelle bestimmen den Ersatz. |

K02 sowie K10/K11 gehören zum gleichzeitig arbeitenden Budgetautor.
`ScreenHostControl.kt`, `ScreenSession.kt` und `BatchesScreenBudgetTest.kt`
werden von diesem Autor nicht geändert. Main besitzt die gemeinsame
Fallregistrierung, Remoteausführung, Integration, Git und Release.
Lokale Builds, Compiler, Formatter, Tests, Server, Installationen,
lange Prozesse, Graphupdates und Unteragenten bleiben ausgeschlossen.

Vorläufige Planungstiefe ist 2: gekoppelte vorhandene Kotlin-/Wiremodelle
und ihre beiden UIverbraucher. Noch zu klären sind die kanonischen
Eigenschafts-/Serialnamen, die tatsächliche Request-/Replyidentität,
Bitmapvalidierung und Abbruchfolge sowie der zugängliche vorhandene
Iconersatz. Eine nur vom Nutzer zu liefernde Angabe ist nicht erkennbar.
Nächster Schritt ist die konkrete Definition-/Aufruferlektüre und danach
der eigene regressionsschützende Plan vor den Produktänderungen.

### Zweitplan und vollständige direkte/indirekte Verknüpfungskarte

Die gelesenen Definitionen bestätigen Planungstiefe 2. Alle Iconreferenzen,
Targets, Requests und Replies stammen bereits aus
`app.smartexplorer.android.screen.AppIcons.kt`. `ScreenIconRef.appId`
behält `@SerialName("app_id")`, `ScreenIconReply.pngBase64` behält
`@SerialName("png_base64")`. Es gibt an den betroffenen Funktionen keinen
zweiten Request- oder Replytyp. Insbesondere importieren `ScreenSession`
und `ScreenIcons` dieselben Klassen; `iconFetch` und `fetch` verwenden
denselben suspend-Funktionstyp. Annahme aus diesem Quellbefund: K07 und
K04–K06 sind Folgefehler des fehlerhaften `app_id`-Zugriffs. Die endgültige
Bestätigung dieser Einordnung benötigt den erneuten Kotlincompilerlauf.

| Betroffene Funktion und Grenze | Vollständige Kette der tatsächlich berührten Fähigkeit |
| --- | --- |
| K01: `ScreenApi.prereqs` | `ScreenHostControl.reportPrereqs` → vorhandene Accessibility-/Usagevoraussetzungen und `RunningApps.list` → PackageManager-/Fenster-/Usageidentität → `AppIcons.reference` mit tatsächlichen installierten Symbolbytes → `ScreenAppEntry.icon` → `AppIcons.contentFor` prüft dieselbe vollständige Referenz → `ScreenApi.prereqs` schreibt unverändert `icon` und `iconPngBase64` → `Core.request("screen.hostPrereqs")` → native Hostregistrierung. Nur der Kotlin-Eigenschaftszugriff ist falsch. |
| Indirekte lokale Iconregistrierung | `screen_host.rs` übernimmt Package-/Labelwerte, verlangt `icon.app_id == package` und das Base64budget → `android_app_icons::replace_registered` → `IconReply::bytes_for` prüft Referenz, Hash und Dimensionen → begrenzter PNGdecoder → aktuelle begrenzte Registry. Ersetzen entfernt nicht mehr sichtbare Pakete. Ein unbrauchbares Symbol verhindert weder Hostregistrierung noch Projektion. |
| K03–K07: `FamilyWindowMenu` | Bestätigte `FamilyListEntry` → `FamilyApi.validate(entry)` im FamilySession-Konstruktor → `FamilySession.open` mit derselben ScreenSession → aktueller `FamilyState.snapshot` → tatsächlich gebundenes Rootmitglied → Icon-App-ID als `ScreenIconTarget.app` oder unveränderter Window-Fallback → nur bei Iconversion 1 dieselbe Referenz → `ScreenAppIcon` und der bereits gemeinsame `iconFetch`-Callback. Familien-/Mitgliedsidentität, Generation, Menüstatus und Steuerrechte werden nicht umgeformt. |
| Indirekter Familien-/Viewerabruf | `ScreenSessionState.iconFetch` bindet die reale laufende Session → `ScreenBackend.icon` → `CoreScreenBackend` → `ScreenApi.viewIcon` mit demselben gemeinsamen serialisierten Request → native Session-Iconstate → begrenzte eindeutige Pendinganfrage und `ViewerFrame::IconRequest` → korrelierte, validierte Antwort → dieselben gemeinsamen Kotlinmodelle. Leeres Ergebnis und Fehler bleiben neutraler Ersatz. |
| K08/K09: `ScreenAppIcon` und Fensterauswahl | Familienmenü sowie `ScreenViewBars.WindowsMenu` bilden konkrete Window-/Appzeilen mit Ziel und Referenz → `ScreenAppIcon(peer, target, reference, fetch)` → rekey nach Peer, Target und vollständiger Referenz → begrenzter Cache oder vorhandener Abruf → validiertes Bitmap als echtes Symbol. Nur der fehlende Ersatzglyph wird auf die vorhandene lokale `SeIcon`-/VectorDrawableinfrastruktur umgestellt. |
| Validierung, Abbruch und Besitz | `ScreenIconContent.fetch` verwendet dieselbe Semaphore → `accept` verlangt Peer, identische Referenz, Version, App-ID, Hashform, Pixel-/Bytegrenzen, statisches PNG, SHA-256 und tatsächliche Decoderdimensionen → Cache nach Peer/App/Hash und Löschung überholter Referenzen. CancellationException wird erneut geworfen; Schlüsselwechsel startet frischen Composezustand. Sessionclose vergisst den Peer und beendet seinen bisherigen Collector-/Scopeweg. |
| Bedienung und Eingabegrenze | `FamilyScreen` besitzt dieselbe konkrete FamilySession; Dispose/Zurück trennen sie. `FamilyWindowMenu` meldet geöffnete lokale UI an `ScreenViewScreen`, sodass Menütaps nicht an die entfernte Eingabe gehen. Focus, Resize, Close, Refresh, Keyboard und Disconnect behalten dieselben Bedingungen und Aufrufe. Die gewöhnliche Fensterauswahl behält Systemauswahl-/Waylandgrenzen und Quellwechsel. |

Die native Session-Iconstate übernimmt ausschließlich den bestehenden
Sessionkontext. Sie bindet die Antwort an den vollständigen Request,
prüft die Antwort und hält Cache/Pendingbestand begrenzt; fehlgeschlagener
Send oder Ablauf entfernen den Pendingeintrag. Die dortige Referenz-/
Hashprüfung sowie die lokale Androidregistry werden nicht geändert.

Die vorhandenen Appkatalog-/Startverbraucher `AppCatalog`, `AppLaunch`
und `AppsApi` verwenden dieselben Iconmodelle bereits mit `appId` und
`pngBase64`. `FamilyWindow.app_id` und `FamilyMember.app_id` sind dagegen
eigenständige gültige Kotlin-Eigenschaften ihrer bestehenden Wiremodelle.
Eine globale Umbenennung oder Änderung dieser Modelle ist ausdrücklich
keine Lösung.

### Entscheidungsrecherche und endgültiger Fixplan

Das Projekt bindet Kotlin 2.4.20, kotlinx.serialization 1.11.0 und den
bestehenden Compose-BOM 2026.06.01. Die am 2026-10-26 selbst gelesene
versionsgebundene Primärquelle bestätigt, dass `SerialName` den Namen im
SerialDescriptor und damit im JSON ändert
([Annotations.kt, Version 1.11.0](https://raw.githubusercontent.com/Kotlin/kotlinx.serialization/v1.11.0/core/commonMain/src/kotlinx/serialization/Annotations.kt)).
Die aktuelle APIseite zeigt bereits Version 1.12.0; sie wurde nicht als
Versionsbeleg für die Projektbindung verwendet. Der deklarierte Kotlin-
Eigenschaftsname bleibt der Zugriff im Quellcode.

Die Android-Primärdokumentation beschreibt den vorhandenen Weg
`Icon(painterResource(...), contentDescription = ...)` und dessen
Materialfarbe/-größe
([Compose Icons](https://developer.android.com/develop/ui/compose/graphics/images/material)).
`ui/common/Icons.kt::SeIcon` verwendet genau diesen Weg bereits mit
`LocalContentColor`. Die lokale `ic_tab.xml` belegt die Ressourcen-
und Lizenzkonvention. Das von Main tatsächlich beschaffte 24-px-Apps-SVG
ist die Originalgeometrie des neutralen Ersatzes
([Google Material Icons Outlined](https://github.com/google/material-design-icons/blob/master/src/navigation/apps/materialiconsoutlined/24px.svg)).

Gewählt sind zwei gezielte Kotlin-Eigenschaftskorrekturen und eine kleine
lokale VectorDrawable-Ressource über `SeIcon`. Eine Umbenennung der
gemeinsamen Modelle würde funktionierende Verbraucher und Drahtverträge
unnötig berühren. Neue Adapter, doppelte Requesttypen oder Casts würden
die identischen Typen verdecken. Eine zusätzliche große Materialicon-
Abhängigkeit ist für die vorhandene Ressourcendarstellung nicht nötig.
Reale Bitmapanzeige, Validierung und Coroutines bleiben unverändert.

| Punkt | Konkrete Änderung | Erhalt und prüfbares Ergebnis |
| --- | --- | --- |
| F1 | In `ScreenApi.prereqs` ausschließlich `content.png_base64` auf `content.pngBase64` korrigieren. | Weiterhin derselbe über `contentFor` gebundene Inhalt unter dem unveränderten JSONfeld `iconPngBase64`; gemeinsamer Serializer und native Prüfung bleiben erhalten. |
| F2 | In `FamilyWindowMenu` ausschließlich `root?.icon?.app_id` auf `root?.icon?.appId` korrigieren. | Derselbe aktuelle App-/Windowtarget, Iconversionscheck und Callbacktyp; kein Cast oder neuer Loader, keine Änderung von Menü-/Steuerbedingungen. K07 bleibt bis zum Remotecompiler als Folgefehlerannahme gekennzeichnet. |
| F3 | In `ScreenIcons.kt` fehlende Material-Apps-Imports/-Nutzung durch `R.drawable.ic_apps` und den vorhandenen `SeIcon` ersetzen. `ic_apps.xml` mit exakter sichtbarer SVGgeometrie, 24-dp-Größe und Quellen-/Lizenzkommentar ergänzen. | Echter Bitmapzweig bleibt bytegleich; neutraler Appsersatz erhält denselben Modifier, Materialtint und exakt dieselbe zugängliche Beschreibung „Programmsymbol nicht verfügbar“. Keine Gradleänderung. |
| F4 | Eigenen vollständigen Diff nach der Umsetzung gegen 0425aa46 lesen, zusätzliche Quellen-/Kotlin-/XMLkontrolle und eigene Anti-Regressionsreview der gesamten Tabelle ausführen und dokumentieren. | Nur F1–F3 unterscheiden sich; Models, Wireliteralnamen, Validierung, Rechte, Abbruch, Cache und alle Bedienaktionen bleiben erhalten. Reale Kotlin-/Ressourcen-/UIbestätigung erfolgt ausschließlich durch Main im gleichen Remoteeinstieg. |

Die zweite Lückenprüfung bestätigt den vollständigen gemeinsamen
Request-/Replytyp, die kanonischen Eigenschaften, die native Registry-
und Sessionprüfung sowie den vorhandenen Ressourcenweg. Die beiden
zusätzlichen nativen Registry-/Fixturedateien wurden vor ihrer Lektüre
konkret bei Main angefordert und ausdrücklich freigegeben. Die freigegebene
Produkt-Schreibfläche reicht aus; keine Scopeabhängigkeit bleibt offen.

### Umsetzung und eigene anschließende Anti-Regressionsreview

F1 bis F3 sind umgesetzt. Anschließend wurde F4 durch denselben Autor
ausgeführt: vollständiger eigener Diff gegen 0425aa46, erneute Prüfung
der gesamten oben dokumentierten Kette und statische Syntax-/XML-/
Quellidentitätskontrolle. Der Reviewumfang folgt den tatsächlichen
Eigenschafts-, Ressourcen-, Rechte- und Besitzgrenzen.

| Eigene Prüffrage | Befund nach der Umsetzung |
| --- | --- |
| Behält die Hostregistrierung reale installierte Symbole? | `ScreenApi.prereqs` liest jetzt die existierende Eigenschaft `pngBase64`. `AppIcons.contentFor` erhält dieselbe Referenz, und der JSONschlüssel bleibt exakt `iconPngBase64`. RunningApps, PackageManagerquelle, Referenzprüfung, Hashbildung, Iconbudget und native Ersetzung der aktuellen Pakete sind unverändert. |
| Bleiben sämtliche Drahtnamen und Modelle erhalten? | `AppIcons.kt` und alle gemeinsamen Datenklassen sind bytegleich. `app_id` und `png_base64` bleiben Serialnamen; gültige gleichnamige FamilyWindow-/FamilyMember-Eigenschaften bleiben bestehen. Kein Serializer, keine Protokollversion und keine Bibliotheksbindung wurde geändert. |
| Behält das Familienmenü dieselbe konkrete App-/Fensterbindung? | Der korrigierte `appId`-Zugriff verwendet weiterhin ausschließlich die Iconreferenz des aktuellen Rootmitglieds. Derselbe Appfactory-Aufruf, derselbe Window-Fallback, derselbe Iconversionscheck und derselbe Fetchcallback bleiben bestehen. Es wurde kein Cast, Adapter oder paralleler Modelltyp eingeführt. Die Einordnung K07 als Inferenzfolgefehler ist weiterhin eine noch remote zu bestätigende Annahme. |
| Bleiben echte Bitmaps und ein zugänglicher neutraler Ersatz sichtbar vorgesehen? | Der vollständige Bitmapzweig von `ScreenAppIcon` ist bytegleich. Nur der nicht verfügbare Materialglyph ist durch die lokale Appsressource über das vorhandene `SeIcon` ersetzt. Derselbe Modifier bleibt erhalten; `SeIcon` nutzt den bestehenden Materialtint. Die neutrale Beschreibung bleibt exakt „Programmsymbol nicht verfügbar“. Die echte Grafik bleibt dekorativ neben Menü-/Zeilentext. |
| Ist die neue Vektorressource die beauftragte Geometrie? | XML ist syntaktisch gültig. Größe ist 24 × 24 dp, Viewport 24 × 24, und der einzige sichtbare Pfad ist exakt der sichtbare Pfad des bereitgestellten Original-SVG. Weißes Fill wird vom vorhandenen Iconpainter getönt; der unsichtbare SVGhintergrundpfad erzeugt keine zusätzliche Geometrie. Quellen-/Apache-2.0-Kommentar folgt der bestehenden Ressourcenkonvention. |
| Bleiben Hash-/Größenprüfung und Cachebindung erhalten? | Der komplette `ScreenIconContent` ist unverändert: Peer und vollständige Referenz, Version, App-ID, SHA-256-Form/Inhalt, Byte-/Pixelgrenze, PNGsignatur/IHDR, APNG-Abweisung, Decoderdimensionen und reale Bitmapallokation werden weiter geprüft. Cachekeys und Entfernung überholter Appreferenzen bleiben identisch. Native Referenz-, Sessionantwort- und Registryvalidierung sind ebenfalls bytegleich. |
| Bleiben Abbruch und Ressourcenbesitz erhalten? | Fetchsemaphore, `produceState`, dessen Rekey, CancellationException-Weitergabe und Default-Dispatcherdecodierung sind unverändert. Die Session vergisst ihren bisherigen Peercache beim Close; FamilySession beendet ihren bestehenden Screen-/Coroutineweg. Für die Iconkorrektur wurde kein Capture-, Decoder-, Puffer- oder Sessionbesitzer ergänzt oder entfernt. |
| Bleiben Rechte und Bedienwege erhalten? | FamilyScreen, FamilySession, FamilyApi, WindowsMenu und ScreenViewScreen behalten ihre bisherigen Ziel-/Generations-/Operationsbedingungen. Lokales Menü blockiert weiterhin entfernte Eingabe. Steuerung anfordern/abgeben, Focus, Keyboard, Resize, Close, Refresh, Diagnose, Disconnect und gewöhnliche Quellwahl behalten ihre Aufrufe. Systemauswahl und Waylandbeschränkungen der Fensterauswahl bleiben unverändert. |
| Bleibt die gleichzeitige Budgetarbeit getrennt? | Dieser Autor änderte weder ScreenHostControl noch ScreenSession oder BatchesScreenBudgetTest. Ihre benötigten Icon-Aufrufer-/Typstellen wurden ausschließlich gelesen. Die eigene ScreenApi-Änderung betrifft genau einen Eigenschaftszugriff im bestehenden Icon-JSONblock. |

Der vorhandene Tree-sitter-Kotlinparser liest alle drei bearbeiteten
Kotlindateien ohne Syntaxfehlerknoten. Die Liste ihrer Funktionsnamen
bleibt gleich. Der vollständige Quellvergleich ergibt nach Rücknahme
genau der geplanten Eigenschafts-/Import-/Ersatzaufrufänderungen wieder
0425aa46. Damit sind sämtliche übrigen Zeilen einschließlich der
Validierung, Coroutines, Menuaktionen, Requestfelder und Literalnamen
erhalten.

Der XMLparser bestätigt Namespace, Attribute und Pfadstruktur der neuen
Ressource; ihr sichtbarer Pfad wurde direkt mit dem gelieferten SVG
verglichen. Der Androidnamespace ist der bereits bestehende Namespace
`app.smartexplorer.android`. Gemeinsame Modelle, Registry, Validatoren,
Appkatalog-/Startverbraucher, FamilyApi/FamilySession, Fensterauswahl,
Core sowie Gradledateien sind bytegleich zum Ausgangskandidaten. Der
fertige eigene Diff hat bei `git diff --check` keinen Whitespacebefund.
Diese Kontrollen führen weder Kotlincode noch Androidressourcenbau oder
Tests aus.

Ein weiterer Fehler wurde bei der eigenen abschließenden Prüfung der
betroffenen Kette nicht belegt. Der Compiler muss insbesondere K04–K07
nach der korrigierten Eigenschaftsinferenz noch bestätigen. Ein geparster
Quelltext oder XMLpfad beweist keine tatsächliche Bitmap-/TalkBackanzeige.

### Geänderte und neue Dateien

Geändert wurden die drei bestehenden Kotlinverbraucher und dieser
bestehende Bericht. Neu angelegt wurde ausschließlich die ausdrücklich
freigegebene VectorDrawable-Ressource.

| Datei unter `android/app/src/main/` | Art | Zeilen | Bytes |
| --- | --- | ---: | ---: |
| `java/app/smartexplorer/android/api/ScreenApi.kt` | geändert | 441 | 17493 |
| `java/app/smartexplorer/android/ui/screen/FamilyScreen.kt` | geändert | 86 | 6159 |
| `java/app/smartexplorer/android/ui/screen/ScreenIcons.kt` | geändert | 112 | 5816 |
| `res/drawable/ic_apps.xml` | neu | 12 | 564 |

Es wurden keine Rustdateien geändert. Es gibt keine Größenregel-Ausnahme,
neue Abhängigkeit, Versionsänderung oder Zeilenverdichtung.

### Vorhandene Fälle und Anschluss durch Main

Es gibt keine neuen oder umbenannten Testfälle. Die vorhandenen drei
Iconvertragsfälle bleiben unverändert:

- `batches_task_b6_icon_authorizes_only_unique_current_identity`
- `batches_task_b6_icon_reply_binds_app_hash_dimensions_and_decoded_pixels`
- `batches_task_b6_icon_rejects_oversized_ihdr_before_decoding`

Das gelesene gemeinsame Manifest wählt diese Fälle über `batches_` aus;
sie sind nicht ausgeschlossen. Für diese Fortsetzung ist keine neue
Fallregistrierung notwendig. Kotlin- und Androidressourcenstufe desselben
Remoteeinstiegs müssen zusätzlich die korrigierten Eigenschaften,
gemeinsamen Callbacktypen und die neue Drawable-ID bestätigen. Diese
nativen Iconfälle ersetzen keinen Android-UI- oder TalkBacknachweis.

### Vollständige Leseübersicht dieser Fortsetzung

Gelesen wurden sämtliche vier Produktdateien der obigen Übersicht,
der bestehende Bericht, der vollständige wörtliche Nutzervertrag, die
aktuelle Scope-Datei und die vollständige Kotlin-Diagnosedatei. Die neue
Ressource wurde nach ihrer Erstellung statisch gelesen; ihr Ausgangspfad
war vorher nicht vorhanden. Zusätzlich wurden folgende Quellen
vollständig oder an den konkret betroffenen Definitionen und Aufrufern
gelesen:

- `docs/plaene/2026-10-24-batches/rv1-compilerplan.md`: Android-Kotlinkorrekturen, Primärquellen und abgegrenzte Integrationsmeilensteine.
- `docs/ARCHITEKTUR.md`, `docs/oberflaechen/bildschirm.md`: konkrete Android-Screen-/Iconanschlüsse.
- `android/app/build.gradle.kts`, `android/gradle/libs.versions.toml`: Namespace und vorhandene Kotlin-/Serialization-/Composebindung.
- `android/app/src/main/java/app/smartexplorer/android/core/Core.kt`: bestehende Jsonkonfiguration und Request-/Serializergrenze.
- `android/app/src/main/java/app/smartexplorer/android/screen/AppIcons.kt`: vollständige gemeinsame Modelle, PackageManagerquelle und publizierte Inhalte.
- `android/app/src/main/java/app/smartexplorer/android/screen/RunningApps.kt`: vollständige Sichtbarkeits-/Usagequelle, Packagebindung und aktueller Registerbestand.
- `android/app/src/main/java/app/smartexplorer/android/screen/ScreenHostControl.kt`: ausschließlich vorhandener Iconvoraussetzungs-/RunningApps-Aufruf.
- `android/app/src/main/java/app/smartexplorer/android/api/AppsApi.kt`, `android/app/src/main/java/app/smartexplorer/android/screen/AppCatalog.kt`, `android/app/src/main/java/app/smartexplorer/android/screen/AppLaunch.kt`: bestehende korrekte gemeinsame Iconkonsumenten.
- `android/app/src/main/java/app/smartexplorer/android/api/FamilyApi.kt`, `android/app/src/main/java/app/smartexplorer/android/ui/screen/FamilySession.kt`: vollständige Familymodelle, Identität, Sessionbesitz und Operationen.
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenSession.kt`: gemeinsame Icon-Request-/Replyimports, Callback, Backend, Iconversion und Abschlussweg; keine Änderung.
- `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenViewBars.kt`, `android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenViewScreen.kt`: tatsächliche Iconaufrufer, Fensterauswahl und lokale Menü-/Eingabegrenze.
- `android/app/src/main/java/app/smartexplorer/android/ui/common/Icons.kt`, `android/app/src/main/res/drawable/ic_tab.xml`: vollständiger vorhandener SeIcon-/VectorDrawableweg und Ressourcen-/Lizenzkonvention.
- `/tmp/smartexplorer-rv1-40381452096/material-apps-24px.svg`: tatsächlich bereitgestellter Originalpfad des neutralen Ersatzes.
- `native/src/mobile/os/shared/domains/screen_host.rs`, `native/src/mobile/os/shared/domains/screen_icons.rs`: konkrete native Hostregistrierung und vollständige sessiongebundene Anfrage-/Antwortgrenze.
- `native/src/screen/core/app_icon.rs`, `native/src/screen/os/android/app_icons.rs`: gemeinsame native Referenz-/PNGprüfung und vollständige lokale Androidregistry.
- `native/src/screen/core/batches_icon_task_tests.rs`, `native/review-task-manifest.json`: vollständige vorhandene Iconvertragsfälle und konkrete Auswahl.

Die zentralen Arbeitsanweisungen wurden für diese Fortsetzung geladen
beziehungsweise als unverändert wiederverwendet. Die eigene Recherche
und Entscheidungen stehen mit den konkreten offiziellen URLs oben.

### Entscheidungen und verbleibende Grenzen

Der eigene vollständige Fünfteiler ist abgeschlossen. Die Umsetzung
verwendet die tatsächlichen Kotlin-Eigenschaften und denselben lokalen
Iconpfad; Modelle, Wireformat, Rechte, Validierung und Besitzer bleiben
erhalten. Es bleibt keine konkrete Scopeabhängigkeit oder weitere
erforderliche Produktkorrektur dieser Zuweisung offen.

Main übernimmt die praktische Bestätigung im selben Remoteeinstieg sowie
Integration, Manifest, CI, Git und Release. K07 bleibt bis dahin als
Inferenzfolgefehlerannahme gekennzeichnet. Reale Hostregistrierung,
Iconübertragung, Geräteanzeige, Ressourcenauflösung und TalkBackausgabe
wurden hier nicht ausgeführt. Kein lokaler Build, Compiler, Formatter,
Test, Server, Installation, langer Prozess, Graphupdate oder Unteragent
wurde gestartet; ein bestandener Remotelauf wird nicht behauptet.
