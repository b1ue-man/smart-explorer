# Native Compilerkorrekturen im bestehenden RV1-Fixloop

Stand 2026-10-26. Kandidat `d95dc404e834edfe7be0e1c96be162b36865efa9`,
[Remote-Lauf 40354605251](https://github.com/b1ue-man/smart-explorer/actions/runs/40354605251).
Alle Jobs sind beendet. Vorbereitung erfolgreich; Linux, Windows und Android
scheitern bei der Produktbibliothek. Die Android-Gerätestufe ist übersprungen.
Der vorherige Noq-proto-E0451 ist beseitigt: Linux und Windows liefern das
kompilierte Vendorartefakt, Android erreicht ebenfalls die Produktquellen.
Das bestätigt diese Konstruktorgrenze, nicht die vollständige Suite.

## Belegter Bestand und Zuschnitt

Main hat sämtliche Compilerdiagnosen der beiden nativen JSONL-Artefakte pro
Desktopplattform und das vollständige Android-Buildlog ausgewertet. Gleiche
native Meldungen mit gleicher Primärstelle sind zu C01–C58 zusammengefasst;
die Androidmeldungen sind A01–A19. Diese Kennungen bezeichnen Diagnosen,
keine behauptete Zahl unabhängiger Funktionsfehler. Folgefehler bleiben bei
derselben verantwortlichen Integrationsgrenze.

Die Archive wurden an Kandidat und GitHub-SHA256 gebunden geprüft. Der
Rootgraph und die konkreten gemeldeten Quellstellen belegen die untenstehenden
Verbindungen. Der laufende Gesamtplan bleibt unverändert. Keine neue Funktion,
kein zusätzlicher Testeinstieg und kein vorgezogener Release werden eingeführt.

## Konkrete Korrekturmeilensteine

| Verantwortlicher bestehender Autor | Begrenztes Ergebnis und betroffene Grenze | Erhaltener Vertrag und erwartetes Signal |
|---|---|---|
| `fix_capture_restart` | Login-CLI/MCP, Loginbild, gemeinsame Login-/Familyviewerregistrierung, Rendererfixtures und Windows-Pipegruppen; alleiniger Schreiber von `app/screen_modules.rs` | Die tatsächlichen Framebesitzer, app-internen Viewer-/Rendererzugänge und Gruppenprüfung bleiben erhalten. Betroffene Quellen kompilieren; vorhandene Login-/Rendererfälle behalten ihre Aussagen. Eigener Bericht: `fix-windows-capture.md`. |
| `fix_x11_family` | Öffentliche x11rb-Streamgrenze samt X11-Capture-/Cursorkonsumenten | Dieselbe Speicherzulassung bleibt wirksam. Keine Umgehung des Admissionstreams. Die zugehörigen bestehenden X11-Fälle bleiben ausgewählt. Eigener Bericht: `fix-x11-family.md`. |
| `fix_vault_retry` | Doppelte Recoveryregistrierung, Mount-/Drive-IPC, Retained-Startgrenze, Snapshotfehler und Android-Rename | Genau ein wirksamer Recoverydialog; sichern, entfernen, importieren, reaktivieren und geschützte Wiederholung bleiben erreichbar. Rename ohne Überschreiben bleibt atomar und panikfrei. Eigener Bericht: `fix-vault-retry.md`. |
| `fix_device_navigation` | Gerätezugriff über GUI/CLI/Android, App-/TCP-Autorisierung, Desktopziele und bestehende Metadaten-/Capabilityfixtures | Rechtequelle, Widerruf, Deadline, Identität, Icons und vollständige Capabilitymenge bleiben erhalten. Kein Feld wird zur Beseitigung einer Diagnose aus dem Produktvertrag entfernt. Eigener Bericht: `fix-device-navigation.md`. |
| `fix_transfer_compat` | Windows-Publikationsbeleg, CLI-Spool, Android-Dateiidentität, WebDAV-Antwort und GDrive-Scanfixture | Publikationsschutz, Literalpfade, optische Guards, Ressourcenabschluss und ehrliche Fehlerbehandlung bleiben wirksam; DeniedRoots wird nicht als gelungener Scan verschwiegen. Eigener Bericht: `fix-regression-transfer.md`. |
| `fix_screen_budget` | Retrybedienung, ViewerHidden-Zustand und Statusanzeige, vorhandene Snapshotfixture | Retry ruft weiterhin die Retryfunktion auf; versteckte Viewer behalten die vorgesehenen Zustandsübergänge; Snapshotquelle verwendet echte Capturezulassung. Eigener Bericht: `fix-regression-screenbudget.md`. |

Jeder dieser Autoren erstellt selbst die fortgeschriebene Funktionskarte,
den aus Primärquellen und Bestand begründeten Fixplan, die Änderung und seine
eigene Anti-Regressionsreview. Die zusätzlichen notwendigen Lesewege bleiben
an die betroffene Grenze gebunden. Bei einer fehlenden konkreten Datei erteilt
Main die Scopeerweiterung; kein Autor erweitert eigenständig seine Fläche.
Die sechs Schreibflächen sind überschneidungsfrei. Der Recoveryautor hat in
`app/mod.rs` ausschließlich die doppelte Recoveryregistrierung entfernt und
die Datei danach ausdrücklich freigegeben. Main hat diesen Hunk gesichert
und die Datei anschließend allein dem Captureautor übertragen. Dieser
verantwortet C02–C06 zusammen mit seiner bereits zugewiesenen Viewergrenze
vollständig selbst: Login- und Familymodule werden einmal unter
`screen_modules` registriert, vorhandene Rootnamen bleiben durch Re-Exports
beziehungsweise private Aliase bestehen. Rendererhilfen, Shared.resume und
WindowVisibility behalten damit ihren privaten Besitzer und denselben Typ.
`daemon/mod.rs` gehört dem Geräteautor, `app/screen_modules.rs` dem Captureautor.
Weitere gemeinsame Zugänge werden über Main abgestimmt, bevor ein Konsument
auf eine neue interne Grenze festgelegt wird.

Die Arbeitsweise-Auswahl ist nach belegter Fehlersicht durchgeführt: Grad 8
für Capture, Recovery und Geräte; Grad 7 für X11, Transfer und Bildschirmzustand.
Die vorgegebenen bestehenden Codex-Threads laufen mit `gpt-6.1-sol`/`max` als
Startparametern. Der ausdrückliche Max-Effort ersetzt die Auswahlvorschläge
high beziehungsweise medium. Antwortseitige Modellmetadaten und gemessener
Creditverbrauch stehen nicht zur Verfügung.

## Integration und Nachweis

Main integriert die abgeschlossenen Meilensteine und ihre Berichte, prüft
Quellsyntax, konkrete Vertragsübergänge und Modulgrößen statisch und hält den
Rootgraph anschließend vollständig aktuell. Die bisherigen Fallnamen und
Abnahmeerwartungen bleiben im selben Suiteeinstieg. Die Korrekturen werden
gemeinsam gepusht und ausschließlich durch denselben Remote-Fixloop bestätigt.
Kein lokaler Build, Compiler, Formatter oder Test wird ausgeführt. Ein neuer
Remote-Lauf beginnt erst nach Abschluss aller sechs Korrekturen. Der terminale
vollständige Release bleibt nach der erfolgreichen gemeinsamen Auswertung der
letzte Schritt des Gesamtbatches.

## Integrierter Quellstand

Alle sechs Autoren haben ihren vollständigen Fixzyklus abgeschlossen. Main hat
die geänderten Quellen und die jeweils fortgeschriebene Funktionskarte samt
Autorenreview gelesen. Die statischen Syntax-, Whitespace- und Größenprüfungen
melden keine Fehler; sie ersetzen keine Typ- oder Funktionsbestätigung.

| Korrekturgrenze | Separater Commit |
|---|---|
| X11-Streamvertrag | `e841a579` |
| Geräte-, Rechte- und Desktopverträge | `5070f5ad` |
| Transfer-, WebDAV-, Spool- und GDriveverträge | `34d8c0d3` |
| Recovery-, Mount- und Retryverträge | `1d9ba2db` |
| Gemeinsame Viewerregistrierung, Loginbild und Windows-Pipe | `b47507ec` |
| Versteckte Sitzung, Retrybedienung und Snapshotzulassung | `2143299f` |

Die Recoveryentfernung in `app/mod.rs` gehört zum Recoverycommit; die danach
übergebene Viewerregistrierung gehört zum Capturecommit. Die Moduldefinitionen
bleiben eindeutig, ihre bisherigen Appzugänge bleiben erhalten. Die versteckte
Bildschirmsitzung bleibt auch im Freigabeindikator sichtbar und beendbar.

Das gemeinsame Manifest ergänzt ausschließlich die von den Autoren benannten
bestehenden Integrationsfälle. `R20-Compiler-Integrationen` bindet die erhaltenen
Rechteformate, Geräte-/Fenstermetadaten, Zielidentität, Hidden-/Resumeanzeige und
GDrive-Leerseiten an konkrete erwartete Ergebnisse. Die vorhandenen Login-,
X11-, Transfer-, Recovery- und Snapshotgruppen bleiben unverändert ausgewählt.
Kein Fall ist durch diese Registrierung bereits ausgeführt. Der vollständige
Rootgraph wird vor dem gemeinsamen Push aktualisiert; danach folgt ausschließlich
derselbe Remote-Suiteeinstieg für den neuen exakten Kandidaten.

### Vorbereitung des integrierten Kandidaten

Lauf `40366764400` für `531230bc5344494ec01b307f3424e3120ad0c29c`
hat ausschließlich Rustformatierung in Loginbildfixture, Recoveryrename und
lokaler Dateiidentität geliefert. Die Lockdatei ist unverändert, die
Größenprüfung meldet keinen Befund. Main hat Kandidat, Archiv- und Patchhash
geprüft und den Patch unverändert übernommen. Der statische Vergleich erhält
sämtliche syntaktischen Blatttoken bis auf das zusätzliche abschließende
Argumentkomma; Syntax und Modulgrößen bleiben gültig. Keine Produktfunktion
oder Fixtureaussage wurde geändert. Die Compiler- und Gerätestufen dieses
Versuchs wurden übersprungen. Der folgende Kandidat bleibt im selben Fixloop.

## Fortsetzung nach Lauf 40368746815

Der Kandidat `b233e5336a0ee177e19382a7adf592d297dcf202` hat die
Vorbereitung bestanden. Alle Jobs sind beendet. Windows und Linux melden
weitere Compilerdiagnosen; Android erreicht die separate JNI-Brücke und
scheitert dort an zwei Boolean-Argumenten. Die Gerätestufe ist übersprungen.
Main hat alle nativen JSONL-Diagnosen und das vollständige Android-Buildlog
ausgewertet sowie sämtliche Archive an Kandidat und SHA256 gebunden geprüft.
Nach Normalisierung der Plattformpfade ergeben sich D01–D21; die vier
JNI-Diagnosen sind J01–J04. Diese Kennungen zählen keine unabhängigen
Funktionsfehler. Frühere Diagnosekennungen bleiben historische Belege.

### Bestand, Quellen und konkrete Fortsetzung

Der aktuelle vollständige Graph und die tatsächlichen Moduldefinitionen
belegen fehlende Imports in ausgegliederten GUI-/Renderer-/Fixturemodulen.
Die X11-Diagnosen betreffen eine geliehene Propertyantwort und den Fehlertyp
einer Servertransaktion. Die Android-Zustandsfixture ist außerdem mit ihrer
Kotlin-Schlüsselliste, Textzuordnung und Ressource verkettet; diese Verbindung
gehört vollständig zum bisherigen Zustandsautor.

Zusätzlich geprüft am 2026-10-26:

- [windows 0.58.0, Shellquelle](https://raw.githubusercontent.com/microsoft/windows-rs/0.58.0/crates/libs/windows/src/Windows/Win32/UI/Shell/mod.rs): `ShellExecuteExW` und `SHELLEXECUTEINFOW` benötigen `Win32_System_Registry`. Das Feature fehlt im bestehenden `windows`-Abschnitt; der vorhandene Shellstart muss einschließlich PIDL, Prozessbezug und Abschluss erhalten bleiben.
- [windows-sys 0.59.0, Tastatur-/Mausquelle](https://raw.githubusercontent.com/microsoft/windows-rs/0.59.0/crates/libs/sys/src/Windows/Win32/UI/Input/KeyboardAndMouse/mod.rs): `IsWindowEnabled` gehört zu diesem Namespace. Das zugehörige Feature ist bereits aktiviert. Die Enabled-Prüfung bleibt Teil der Fensterkontrolle.
- [jni-sys 0.4.1, jboolean](https://docs.rs/jni-sys/0.4.1/jni_sys/type.jboolean.html): Der Alias bezeichnet `bool`. Die Brücke bindet `jni = 0.22.4`; Kotlin übergibt Booleanwerte und die nativen Capturefunktionen erwarten bool. Buffer-/Decoderbesitz darf bei der Typkorrektur seine Bedeutung nicht ändern.

| Bestehender Fixautor | Begrenzte Korrekturfläche | Konkretes erhaltenes Ergebnis |
|---|---|---|
| `fix_device_navigation` | D01–D03, D07–D12, D14–D16, D21: Desktopaktionen/-menü, Kontenaktionen, Routenanzeige, Execdeadlinefixture und erforderliches Windows-Feature | Ordnerkonfliktbehandlung, Appoberfläche, Kontoverwaltung, Routendiagnose, Execdeadline und derselbe Shellstart bleiben über ihre vorhandenen Zugänge wirksam. Nur dieser Autor darf das benötigte Manifestfeature ergänzen. |
| `fix_vault_retry` | D04–D05: Kapazitätsanzeige und optische Statusanzeige | Dieselben vorhandenen Kapazitäts-, Unbekannt-, Wartemodus- und Leserwerte erscheinen über die bestehende egui-Instanz. Keine neue GUI-Abhängigkeit. |
| `fix_x11_family` | D13, D18–D20: Linux-Sichtbarkeitsproperty und native Windows-/X11-Fensteradapter | Unbekannte Sichtbarkeit bleibt konservativ; Enabled-/Fokusprüfung, eindeutige Fensterbindung und Serverfreigabe bleiben unverändert wirksam. |
| `fix_capture_restart` | D06, J01–J04: YUV-Renderereigentümer und zwei JNI-Captureeinstiege | Derselbe private Rendererbesitzer und dieselben Kotlin-Booleanwerte bestimmen Bild-, Decoder- und Bufferzulassung ohne zusätzliche Pixelkopie oder vorzeitige Freigabe. |
| `fix_screen_budget` | D17: vollständiger Android-Zustandsvertrag aus Rustfixture, Schlüsselliste, Kotlintextzuordnung und Ressource | `viewer_hidden` hat auf Android denselben verständlichen Pausentext wie auf Desktop; sämtliche bisherigen Zustands-/Fehler-/End-/Blocktexte bleiben erhalten. |

Die konkrete Dateifreigabe je Autor ist überschneidungsfrei und enthält nur
seine Fehlerstellen, ihre tatsächlichen Definitionen und angeschlossenen
Konsumenten. Jeder Autor besitzt erneut seine vollständige Funktionskarte,
seinen konkreten Fixplan, die Änderung und die anschließende eigene
Anti-Regressionsreview. Die Quellenfragen sind vor dem jeweiligen Edit zu
klären. Main übernimmt ausschließlich gemeinsame Registrierung, Integration,
Graph, Commits, Push und denselben Remote-Suite-/Releaseablauf. Die erhaltenen
Fixtureaussagen dürfen nicht zur Beseitigung einer Diagnose entfernt werden.
Die Erwartung jeder Zeile wird durch die vorhandenen zugehörigen Fälle in
derselben Remote-Suite bestätigt; zusätzlich nötige bestehende Fallnamen
melden die Autoren gesammelt an Main. Kein lokaler Lauf ist autorisiert.

### Integrierte D/J-Korrekturen

Alle fünf Autoren haben Fehlerliste, vollständige betroffene Verknüpfungskarte,
vorherigen Fixplan, Umsetzung und anschließende eigene Anti-Regressionsreview
abgeschlossen. Main hat die jeweiligen vollständigen Diffs und Berichte gelesen;
statisches Rust-/Kotlin-/TOML-/XMLparsing, Whitespace und Modulgrößen melden
keinen Befund. Diese Rückmeldung ist kein Compiler- oder Funktionsnachweis.

| Korrekturgrenze | Separater Commit |
|---|---|
| Kapazitäts- und optische Leseranzeige | `7a1f0931` |
| YUV-Besitzer und beide JNI-Booleanübergaben | `f9306ea9` |
| Vollständige Android-Hidden-Textkette | `31560a85` |
| X11-Sichtbarkeit/-Bindung und Windows-Enabled-API | `cd0be5a9` |
| Desktopaktionen, Konten, Routendiagnose, Execfrist und Windows-Shellfeature | `f86bcb63` |

Der Geräteautor hat außerdem die unmittelbar verbundene Kontenfixture korrigiert:
Sie prüft weiterhin Einrichtungsstatus und Version, öffnet dafür jedoch den
tatsächlich vorhandenen Diagnosebereich. Alle übrigen Assertions bleiben erhalten.
Das zusätzliche Windows-API-Feature verändert weder Crate- noch Paketversion.

`R20-Compiler-Integrationen` nimmt die sechs vorhandenen Fälle
`screen_task_android_screen_keys_file_current`,
`screen_task_android_strings_match_desktop_sources`,
`handshake_io_enforces_one_absolute_deadline_for_a_stalled_peer`,
`perm_task_gui_accounts_page_states`,
`perm_task_gui_setup_without_pkexec_shows_sudo_command` und
`desktop_task_desktop_ops_rename_trash_copy` auf. Die vorhandene JVM-Klasse
`ScreenTextsTest` wird einschließlich ihres Detailtextfalls ausgewählt. Alle
vorherigen Auswahlen bleiben erhalten; es gibt keinen zusätzlichen Suiteeinstieg.

Nach dem vollständigen Rootgraphrefresh und gemeinsamen Push wird ausschließlich
derselbe Remote-Einstieg für den exakten neuen Kandidaten wiederholt. Compiler-,
Funktions- und Veröffentlichungserfolg sind zu diesem Zeitpunkt nicht bestätigt.

### Formatvorbereitung des D/J-Kandidaten

Lauf `40379623936` für `5609ce9f834e0a8e32b1f076415b586ae427da68`
fordert ausschließlich eine Zeilenzusammenfassung der fünf Argumente in
`native/android-bridge/src/screen_capture_jni.rs`. Main hat Archiv, Kandidat
und Patchhash geprüft und den Remote-Patch unverändert übernommen. Der
statische Tree-sitter-Vergleich erhält jedes syntaktische Blatttoken exakt;
die Datei ist syntaktisch gültig und hat 143 Zeilen. Lockdatei und Modulgrößen
melden keinen Befund. Keine Compiler- oder Funktionsstufe dieses Versuchs
wurde ausgeführt. Es folgt weiterhin ausschließlich derselbe Remote-Einstieg.

## Android-Kotlinkorrekturen des Kandidaten 0425aa46

Stand 2026-10-26, Remote-Lauf
[`40381452096`](https://github.com/b1ue-man/smart-explorer/actions/runs/40381452096).
Die Vorbereitung ist erfolgreich. Android hat die native Bibliothek einschließlich
JNI-Brücke erfolgreich kompiliert; ihre LOAD-Segmente erfüllen die im Einstieg
verlangte 16-KiB-Ausrichtung. Danach scheitert `:app:compileDebugKotlin`.
Windows und Linux laufen beim Festhalten dieses Befunds noch. Es gibt damit
keinen vollständigen Compiler-, Funktions- oder Veröffentlichungsnachweis.

### Bestandsbefund und erster Korrekturplan

Das kandidatgeprüfte Android-Artefakt `12678538449` hat den SHA-256
`115e29d16f796903e46e4d51a7be74c1381dd157b64b46f318f10e4b8bca6408`.
Main hat alle elf Kotlin-Diagnosen K01–K11 und ihre konkreten Quellstellen
ausgewertet. Sie betreffen vier Grenzen:

- K01 und K03–K07: `ScreenApi.hostPrereqs` und `FamilyWindowMenu` greifen
  auf Serialisierungsnamen statt der tatsächlichen Kotlin-Eigenschaften von
  `ScreenIconReply` beziehungsweise `ScreenIconRef` zu. Die gemeinsamen Typen
  heißen im Code `pngBase64` und `appId`; `@SerialName` erhält die Drahtnamen
  `png_base64` und `app_id`. K07 ist nach den identischen importierten Typen
  voraussichtlich ein Folgefehler; diese Einordnung bleibt bis zum Compilerlauf
  eine ausdrücklich gekennzeichnete Annahme.
- K08/K09: `ScreenAppIcon` importiert ein im eingebundenen
  `material-icons-core` nicht vorhandenes `Icons.Outlined.Apps`.
  `ui/common/Icons.kt` und die vorhandenen VectorDrawable-Ressourcen belegen
  den bestehenden lokalen Symbolweg.
- K02: `ScreenHostControl.attachSurface` verspricht `VirtualDisplay`, während
  `MediaProjection.createVirtualDisplay` auch `null` liefern darf. Die beiden
  Capture-Aufrufer besitzen bereits Abschlusswege für temporäre Frames und
  Encoder. Der erste Projektionsstart und die spätere Konfiguration müssen
  beide einen Erzeugungsfehler korrekt melden.
- K10/K11: `ScreenSession` übergibt das Budget als nachgestelltes Lambda,
  obwohl der letzte Konstruktorparameter von `ScreenPullBuffers` inzwischen
  der optionale feste `ByteBuffer` ist. Die direkte Konsumentensuche zeigt
  denselben Fehler in zwei Aufrufen von `BatchesScreenBudgetTest`; diese
  zusätzlichen Quellbefunde sind noch keine ausgeführten JVM-Diagnosen.

Der Korrekturplan erhält die gemeinsamen Modelle, Drahtnamen, realen
Iconquellen, Identitäts- und Hashprüfung, Capturezulassung, Projektionsidentität
und Pufferbesitzer. Er ändert weder Protokollversion noch Bibliotheksversion.

### Primärquellen und geschlossene Planlücken

Quellenabruf am 2026-10-26:

- [kotlinx.serialization 1.11.0, `SerialName`](https://github.com/Kotlin/kotlinx.serialization/blob/v1.11.0/core/commonMain/src/kotlinx/serialization/Annotations.kt):
  Die Annotation bestimmt den serialisierten Namen. Der Zugriff im Kotlin-Code
  verwendet weiterhin die deklarierte Eigenschaft. Der Projektpin ist 1.11.0;
  die aktuelle API-Webseite zeigt bereits 1.12.0 und ersetzt diesen Versionsbeleg
  deshalb nicht.
- [Kotlin, nachgestellte Lambdas](https://kotlinlang.org/docs/lambdas.html#passing-trailing-lambdas):
  Ein Lambda außerhalb der Argumentklammern setzt einen Funktionsparameter an
  letzter Stelle voraus. Die explizite Zuordnung zu `offered` erhält den festen
  Pufferzugang der JNI-Gerätefixture.
- [Android, `MediaProjection.createVirtualDisplay`](https://developer.android.com/reference/android/media/projection/MediaProjection#createVirtualDisplay(java.lang.String,int,int,int,int,android.view.Surface,android.hardware.display.VirtualDisplay.Callback,android.os.Handler)):
  Eine fehlgeschlagene Erzeugung kann `null` zurückgeben. Registrierung,
  Lebensdauer und Wiederverwendung einer vorhandenen Anzeige bleiben an
  dieselbe Projektion gebunden. Ein fehlendes Display ist kein erfolgreicher
  Capturestart.
- [Android, Compose-Symbole](https://developer.android.com/develop/ui/compose/graphics/images/material)
  und [Google, vorhandenes Apps-Vektorsymbol](https://github.com/google/material-design-icons/blob/master/src/navigation/apps/materialiconsoutlined/24px.svg):
  Der vorhandene lokale VectorDrawable-/`SeIcon`-Weg kann den neutralen
  Ersatz bewahren, ohne eine zusätzliche umfangreiche Iconabhängigkeit
  einzuführen. Das originale 24-px-SVG wurde tatsächlich abgerufen.

Die zweite Lückenprüfung verfolgt alle `ScreenPullBuffers`-Aufrufe einschließlich
`ScreenDecoder` und `ScreenCaptureFixture`, beide `ScreenAppIcon`-Aufrufer,
die gemeinsamen Iconmodelle und den nativen `iconPngBase64`-Empfänger.
`FamilyWindow.app_id` ist eine eigenständige gültige Eigenschaft und gehört
nicht zu einer globalen Umbenennung. Die lokalen Symbolnamen enthalten bisher
kein Apps-Symbol; der neutrale Ersatz darf als kleine eigene Vektorressource
denselben vorhandenen Darstellungsweg nutzen.

### Detaillierte Integrationsmeilensteine

| Begrenztes Ergebnis | Betroffene Schreibfläche | Erhaltener Vertrag und erwartetes Signal |
|---|---|---|
| Gemeinsame Android-Iconmodelle korrekt konsumieren und neutralen Ersatz erhalten | `api/ScreenApi.kt`, `ui/screen/FamilyScreen.kt`, `ui/screen/ScreenIcons.kt`, gegebenenfalls `res/drawable/ic_apps.xml` | Hostvoraussetzungen liefern weiterhin `iconPngBase64`; serialisierte Referenzen behalten `app_id` und `png_base64`. Familienmenü und Fensterauswahl behalten aktuelle Peer-/Appidentität, Rechte, Abbruch, Hash-/Größenprüfung und den zugänglichen neutralen Ersatz. Kotlin- und Ressourcenstufe desselben Remote-Einstiegs bestätigen die tatsächlichen Typen. |
| Capturestart und Viewerbudget an ihre vorhandenen API-Typen binden | `screen/ScreenHostControl.kt`, `ui/screen/ScreenSession.kt`, `ui/screen/BatchesScreenBudgetTest.kt` unter `src/test` | Kein erfolgreicher Start ohne Display; ein Fehler erreicht den vorhandenen Fehler- und Abschlussweg ohne Verlust von Pause, Fortsetzung oder Wiederverwendung. Budgetannahme und dieselbe Allocation über Decoderwechsel bleiben erhalten; feste JNI-Puffer bleiben unterstützt. Vorhandene Budget-, Ownership- und Gerätecapturefälle bleiben im selben Einstieg ausgewählt. |

Diese Tabelle ist Mains Integrationszuschnitt. Der jeweils beauftragte bestehende
Fixautor verantwortet weiterhin selbst Funktionsauflistung, vollständige direkte
und indirekte Verknüpfungskarte, konkreten Fixplan, Änderung und anschließende
Anti-Regressionsreview. Die endgültige Zuweisung berücksichtigt zusätzlich die
noch ausstehenden Windows-/Linuxbefunde. Während deren Builds läuft kein
Fixagent an. Nach vollständiger Korrektur folgt ausschließlich derselbe
Remote-Suiteeinstieg und danach der eine terminale vollständige Remote-Release.

## Linux-Auswertung desselben Laufs: L01 und L02

Beim Abruf um 19:54 UTC ist Linux beendet; Windows läuft weiterhin. Das
kandidatgeprüfte Linux-Artefakt `12681652291` hat den SHA-256
`b3aa1cea1d362661b9914726cb6f7a2962f0ab52212ada59b2812bd2daf08c61`.
Bibliotheks-Prüfbinärdatei, `se` und die betroffene `egui_glow`-Prüfbinärdatei
sind erfolgreich kompiliert. Der vorhandene GL-Fehlerledgerfall liefert seinen
vorgesehenen kontrollierten Nachweis; ein physischer Grafiktreiber ist damit
nicht geprüft. Die Hauptauswahl wird vor der Funktionsausführung abgebrochen.

### L01: veraltete Pflichtauswahl

Der Runner verlangt noch `batches_task_b6_first_worker_snapshot_`.
Commit `4e9f4202` hat den zugehörigen Fall beim Umbau auf vorab gebundene
Capturebesitzer durch
`batches_task_b6_worker_snapshot_requires_final_binding_and_prevents_later_growth`
ersetzt. Der aktuelle Fall steht in
`native/src/screen/os/shared/host_worker_main_tests.rs` und in der tatsächlichen
Linux-Binärliste. Er erhält Snapshot, anschließende Liveproduktion, feste
Bild-/Speichergrenzen, Ablehnung späterer Vergrößerung und Ablehnung eines
ungebundenen Legacyrequests; zusätzlich lehnt er Capture vor finaler Bindung ab.

Mains Korrekturmeilenstein betrifft allein `native/review-task-manifest.json`
und dessen Dokumentation: Die Pflichtauswahl wird auf den tatsächlich
vorhandenen Nachfolger gebunden. Der allgemeine B6-Auswahlbereich enthält ihn
bereits. Keine Assertion und kein Produktverhalten werden zur Beseitigung des
Auswahlfehlers entfernt. Der statische Abgleich aller Linux-Pflichtpräfixe mit
der echten Binärliste findet ausschließlich diese veraltete Kennung.

### L02: Fehler beim Abschluss der realen P27-Messung

Die encoderfreie 10-Mbit/s-Referenz wurde mit echten Relaybeobachtungen
gespeichert. Die anschließende Bildpipelineszene beendet den Lauf in
`Tasks::finish` mit `wire ACK stream closed`; es fehlen ihr Rohdatenartefakt
und der abschließende Leistungsbeleg. Daraus folgt kein Leistungsurteil über
P27 und noch kein nachgewiesener Produktfehler.

Die konkrete Grenze liegt in `batches_rate_reference_transport.rs::Tasks`
und `batches_rate_reference_task_tests.rs::screen`: Der Abschluss bricht
miteinander verbundene Tokio-Aufgaben nacheinander ab und akzeptiert beim
Join ausschließlich `JoinError::is_cancelled`. Ein so geschlossener
ACK-Sender kann jedoch den noch laufenden Empfänger wecken. Dass genau diese
Abbruchreihenfolge den beobachteten Fehler ausgelöst hat, ist bis zur
Autorenanalyse eine Annahme; ein bereits während der Messung entstandener
Fehler darf keinesfalls pauschal verschwiegen werden.

Primärquellen, abgerufen am 2026-10-26:
[Tokio 1.52.3, JoinHandle](https://github.com/tokio-rs/tokio/blob/tokio-1.52.3/tokio/src/runtime/task/join.rs)
und [Tokio 1.52.3, select](https://github.com/tokio-rs/tokio/blob/tokio-1.52.3/tokio/src/macros/select.rs).
Die lokale Registryquelle stimmt mit dem Lockpin überein. `abort` verlangt
keinen einheitlichen Cancelled-Rückgabewert, wenn die Aufgabe bereits fertig
ist. Ein gedroppter JoinHandle wartet nicht auf seine Aufgabe. Die
zweite Lückenprüfung muss deshalb tatsächliches Messversagen und ausdrücklich
eingeleiteten Abschluss unterscheiden und alle eigenen Aufgaben beenden.

Der begrenzte Autorenmeilenstein erhält alle drei Netzprofile, reale
Pipeline-/Wire-/ACK-Pfade, Zeitfenster, Rohdaten und unveränderte V7-Orakel.
Seine Schreibfläche umfasst die beiden genannten Fixturedateien und den
eigenen Bericht `fix-review-p27-abnahme.md`; nötige kleine Lebensdauerhilfen
bleiben im selben Fixturemodul. Erwartetes Signal ist ein sauber beendeter
Messlauf, der frühe Transport-/Pipelinefehler weiterhin als Fehler meldet
und das echte Leistungsergebnis ausgibt. Keine großzügigere Leistungsschwelle,
kein Ersatz durch simulierte Werte und kein zusätzlicher Suiteeinstieg.
Die endgültige Autorzuweisung erfolgt nach Ende der verbleibenden Builds.

## Abschließende Windows-Auswertung und verbindlicher Zuschnitt

Beim Abruf um 20:24 UTC sind sämtliche Jobs von `40381452096` beendet.
Windows hat Bibliotheks-Prüfbinärdatei, `se` und die betroffene
`egui_glow`-Prüfbinärdatei ebenfalls erfolgreich kompiliert. Das Windows-Artefakt
`12682562310` ist an Kandidat und SHA-256
`96f22a7e5ab866782c9b72f591c862c106833ab042bfef05c53e94cfd8a337d7`
gebunden geprüft.

W01 ist eine falsche Plattformzuordnung im gemeinsamen Manifest:
`screen_task_android_screen_keys_file_current` und
`screen_task_android_strings_match_desktop_sources` liegen unter `mobile`.
`native/src/lib.rs` kompiliert dieses Modul für Android sowie Unix-Prüfläufe,
nicht für Windows. Beide Fälle sind in der tatsächlichen Linux-Binärliste
vorhanden. Der vollständige Abgleich der Windows-Pflichtauswahl findet außerdem
den ebenfalls mobilen Marker `batches_login_image_task_tests::android_`.
Der bereits belegte veraltete Snapshotmarker L01 betrifft beide Plattformen.

Mains Manifestkorrektur verlegt die beiden Textintegrationen und den mobilen
Marker in die vorhandenen Linux-Auswahlen. Der Android-Textnachweis erhält
einen ausdrücklich Linux-/JVM-bezogenen R20-Abschnitt; die gemeinsamen
R20-Grenzen bleiben für Windows und Linux verpflichtend. Es entfällt kein
Nachweis und keine Assertion. Die bestätigten Binärlisten liefern den statischen
Auswahlbeleg; die Funktionsausführung bleibt im selben Remote-Einstieg offen.

Verbindlicher Zuschnitt nach vollständiger Fehlersicht: Die Android-Icon-
Konsumgrenze K01/K03–K09 und die Capture-/Puffergrenze K02/K10/K11 einschließlich
der zwei verbundenen JVM-Aufrufe bleiben zwei überschneidungsfreie Ergebnisse.
L02 bleibt ein eigenes zusammenhängendes Messlebensdauer-Ergebnis. Für jedes
Ergebnis ist ein einzelner Autor für Fehlerfunktion, vollständige Verknüpfungskarte,
konkreten Plan, Fix und anschließende Anti-Regressionsreview verantwortlich.
Mains gemeinsames Manifest folgt erst nach den vollständigen Quellkorrekturen.

Der vollständige statische Vergleich umfasst zusätzlich alle native Marker
und jede Ressourcenfallzuordnung mit den tatsächlichen Defaultplattformen
des Runners. Dadurch ist dieselbe Plattformabweichung auch in `R9-L7-Tickets`
belegt: Seine fünf `android_*`-Fälle liegen sämtlich unter
`mobile::domains::login_images`. Dieser vorhandene Nachweis wird ausdrücklich
Linux zugeordnet. Der Abschnitt selbst enthält keine Gerätefälle; der
zusätzliche JNI-Nachweis bleibt von dieser Zuordnung unberührt. Weder
Fallnamen noch Prüfaussagen entfallen.

## Abgeschlossene K/L02-Quellkorrekturen und Auswahlbindung

Alle drei Autoren haben ihren eigenen vollständigen Analyse-, Plan-, Fix- und
Nachreviewzyklus beendet. Die Integrationscommits sind `00c97f88` (K01/K03–K09),
`69f86c46` (K02/K10/K11 und die verbundenen Budgetfixtureaufrufe) sowie
`578357a8` (L02). Main hat die vollständigen neuen Berichtsabschnitte und Diffs
gelesen; statische Kotlin-/Rust-Syntax, XML und Whitespace sind ohne Befund.
Der Iconsersatz erhält die bisherigen serialisierten Namen. Capturefehler
erreichen beim Erststart und bei Rekonfiguration denselben Epochfehlerweg;
beide temporären Besitzer erhalten ihren Abschlussversuch. Displayreuse,
Projektionsidentität und fester JNI-Puffer bleiben erhalten.

L02 hält zuerst sämtliche direkten Messfutures an, bevor ihre Ressourcen
freigegeben werden. Bereite echte Rückgaben und Panics bleiben sichtbar;
alle eigenen Handles werden vor der Rückgabe gejoint. Der genaue Zeitpunkt
des bisherigen ACK-Fehlers bleibt aus dem alten Log unbekannt. Produktregler,
Netzprofile, Messkörper und Leistungsorakel bleiben unverändert. Die neuen
kleinen Abschlussfälle sind Linux zugeordnet und werden innerhalb desselben
Suiteeinstiegs ausgeführt; sie ersetzen keinen P27-Leistungsnachweis.

Mains L01/W01-Korrektur ist vollständig im gemeinsamen Manifest umgesetzt:
Snapshotnachfolger, mobile Pflichtpräfixe, beide Androidtextintegrationen und
`R9-L7-Tickets` sind ihrer tatsächlichen Plattform zugeordnet.
`R20-Androidtext-Vertrag` erhält die beiden bisherigen Rust- und JVM-Fälle;
die verbleibenden gemeinsamen R20-Grenzen gelten weiter für Windows und Linux.
`R21-P27-Messabschluss` enthält sämtliche vom L02-Autor benannten Fälle.
Alle bisherigen Ressourcenfallnamen, Androidklassen, P27-Profile, Qualitäts-
und GL-Ledgerzuordnungen bleiben erhalten.

Der statische vollständige Auswahlvergleich verwendet die gespeicherten
Binärlisten von `40381452096` und kennzeichnet die neuen Abschlussfälle
getrennt als nur im Quelltext vorhanden. Pflichtpräfixe, genaue Integrationen,
Meilensteinmarker und Ressourcenfälle stimmen damit für beide Plattformen.
Es wurde keine Binärdatei und kein Fall lokal ausgeführt. Typ-, Funktions-
und Leistungsbestätigung bleiben bis zur Auswertung desselben Remote-Einstiegs
offen. Main aktualisiert vorher den vollständigen Rootgraph und pusht den
gemeinsamen Kandidaten; die Veröffentlichung bleibt der eine terminale Schritt.

### Formatvorbereitung des K/L02-Kandidaten

Lauf `40400605922` für `4a42b936a6be21cf04ee11e786e081282a45949e`
ist ausschließlich an der Formatierung des `finish().await.expect(...)`-
Aufrufs in der Screenmessfixture gestoppt. Der kandidaten- und hashgebundene
Remote-Patch wird unverändert übernommen. Sämtliche syntaktischen Blatttoken
sind identisch; die Datei bleibt mit 355 Zeilen und 15.233 Bytes innerhalb
der Modulgrenzen. Abhängigkeiten unverändert, keine Größenverletzung.
Compiler- und Funktionsstufen sind übersprungen. Kein lokaler Formatter,
Compiler oder Fall wurde ausgeführt; der nächste Kandidat verwendet denselben
Remote-Suiteeinstieg nach Graphrefresh und Push.
