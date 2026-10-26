# Geräte- und Appnavigation: G7, G8, G9

Stand: 2026-10-26. Zugewiesener Fixautor: `/root/fix_device_navigation`.
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
